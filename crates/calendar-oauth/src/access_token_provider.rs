//! Port of `AccessTokenProvider.swift`.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use calendar_core::connection::{ConnectionId, CredentialStore};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::source_types::SourceError;
use calendar_core::time::{Now, seconds_between};
use uuid::Uuid;

use crate::oauth::OAuthTokens;

/// Swift's `RefreshFunction`: given the stored refresh token, performs one refresh request.
pub type RefreshFunction =
    Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = Outcome<OAuthTokens>> + Send>> + Send + Sync>;

type BoxedFlight<T> = Pin<Box<dyn Future<Output = Outcome<T>> + Send>>;

enum Slot<T> {
    Running(BoxedFlight<T>),
    Done(Outcome<T>),
}

/// The Rust form of Swift's stored `Task<T, Error>` that several callers `await`: one request in flight that every
/// caller shares, and that keeps going when the caller driving it is cancelled.
///
/// The request is a future that nobody owns; whichever caller currently waits drives it. Callers queue on the
/// slot's lock, and the lock holder polls the future. When the holder is dropped (its caller was cancelled) the
/// future stays in the slot half-run and the next caller in the queue continues it, so a cancelled first caller can
/// never make a second request start. When it finishes, the answer is stored and every queued caller reads it.
///
/// This uses `tokio::sync::Mutex` for the queue on purpose: the guard is held across an `.await` by design, and it
/// guards no provider state (that is behind a `std` mutex that is never held across an `.await`).
pub(crate) struct SharedFlight<T> {
    slot: tokio::sync::Mutex<Slot<T>>,
}

impl<T: Clone + Send + 'static> SharedFlight<T> {
    pub(crate) fn new(request: impl Future<Output = Outcome<T>> + Send + 'static) -> Arc<Self> {
        Arc::new(SharedFlight {
            slot: tokio::sync::Mutex::new(Slot::Running(Box::pin(request))),
        })
    }

    /// Swift's `try await task.value`.
    pub(crate) async fn value(&self) -> Outcome<T> {
        let mut slot = self.slot.lock().await;
        let result = match &mut *slot {
            Slot::Done(result) => return result.clone(),
            Slot::Running(request) => request.as_mut().await,
        };
        *slot = Slot::Done(result.clone());
        result
    }
}

#[derive(Default)]
struct State {
    cached: Option<OAuthTokens>,
    bound_revision: Option<Uuid>,
    inflight: Option<Arc<SharedFlight<OAuthTokens>>>,
}

struct Inner {
    connection_id: ConnectionId,
    credentials: Arc<dyn CredentialStore>,
    refresh: RefreshFunction,
    now: Now,
    state: Mutex<State>,
}

impl Inner {
    /// Swift actors are re-entrant at every `await`, so no guard from here is ever held across one.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn finish_refresh(&self) {
        self.lock().inflight = None;
    }

    async fn perform_refresh(&self) -> Outcome<OAuthTokens> {
        let Some(snapshot) = self
            .credentials
            .credential_snapshot(&self.connection_id)
            .await?
        else {
            return Err(Failure::Source(SourceError::AuthExpired));
        };
        let Some(refresh_token) = snapshot
            .secrets
            .get(AccessTokenProvider::REFRESH_TOKEN_KEY)
            .cloned()
        else {
            return Err(Failure::Source(SourceError::AuthExpired));
        };
        let tokens = (self.refresh)(refresh_token.clone()).await?;
        // Check and persist as one store operation: a removed or reauthorized account must not be restored
        // by an old in-flight refresh, even when its server returned a rotated token.
        let rotated = tokens.refresh_token.as_deref().unwrap_or(&refresh_token);
        if !self
            .credentials
            .update_refresh_token(rotated, &self.connection_id, snapshot.revision)
            .await?
        {
            return Err(Failure::Source(SourceError::AuthExpired));
        }
        let mut state = self.lock();
        state.bound_revision = Some(snapshot.revision);
        state.cached = Some(tokens.clone());
        Ok(tokens)
    }
}

/// Hands out a valid access token, refreshing with the stored refresh token when it is about to expire.
/// Concurrent callers share one in-flight refresh. A rotated refresh token is written back to the store.
pub struct AccessTokenProvider {
    inner: Arc<Inner>,
}

impl AccessTokenProvider {
    pub const REFRESH_TOKEN_KEY: &'static str = "refresh_token";
    const SAFETY_MARGIN: f64 = 60.0;

    /// `initial` is Swift's `initial:` (default nil).
    pub fn new(
        connection_id: impl Into<ConnectionId>,
        credentials: Arc<dyn CredentialStore>,
        refresh: RefreshFunction,
        now: Now,
        initial: Option<OAuthTokens>,
    ) -> AccessTokenProvider {
        AccessTokenProvider {
            inner: Arc::new(Inner {
                connection_id: connection_id.into(),
                credentials,
                refresh,
                now,
                state: Mutex::new(State {
                    cached: initial,
                    ..State::default()
                }),
            }),
        }
    }

    pub async fn access_token(&self) -> Outcome<String> {
        let (cached, bound_revision) = {
            let state = self.inner.lock();
            (state.cached.clone(), state.bound_revision)
        };
        if let Some(cached) = cached
            && seconds_between((self.inner.now)(), cached.expires_at) > Self::SAFETY_MARGIN
        {
            if let Some(bound_revision) = bound_revision {
                let current = self
                    .inner
                    .credentials
                    .credential_snapshot(&self.inner.connection_id)
                    .await?
                    .map(|snapshot| snapshot.revision);
                if current != Some(bound_revision) {
                    let mut state = self.inner.lock();
                    state.cached = None;
                    state.bound_revision = None;
                    return Err(Failure::Source(SourceError::AuthExpired));
                }
            }
            return Ok(cached.access_token);
        }
        Ok(self.refresh_shared().await?.access_token)
    }

    /// Drops the cached token (call after a 401) so the next `access_token()` refreshes.
    pub fn invalidate(&self) {
        let mut state = self.inner.lock();
        state.cached = None;
        state.bound_revision = None;
    }

    async fn refresh_shared(&self) -> Outcome<OAuthTokens> {
        let flight = {
            let mut state = self.inner.lock();
            if let Some(inflight) = &state.inflight {
                inflight.clone()
            } else {
                // The flight clears `inflight` itself when the refresh finishes, so a cancelled first caller
                // cannot let a second refresh start while this one is still running.
                let inner = self.inner.clone();
                let flight = SharedFlight::new(async move {
                    let result = inner.perform_refresh().await;
                    inner.finish_refresh();
                    result
                });
                state.inflight = Some(flight.clone());
                flight
            }
        };
        flight.value().await
    }
}
