//! Port of `ExternalAccessTokenProvider.swift`.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use calendar_core::external_tokens::{ExternalAccessToken, ExternalAccount, ExternalTokenSource};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::source_types::SourceError;
use calendar_core::time::{Now, seconds_between};

use crate::access_token_provider::SharedFlight;
use crate::access_token_supplying::AccessTokenSupplying;

#[derive(Default)]
struct State {
    cached: Option<ExternalAccessToken>,
    force_next_request: bool,
    inflight: Option<Arc<SharedFlight<ExternalAccessToken>>>,
    /// Bumped by `invalidate()`: an answer to a request that started before it is returned to its callers but
    /// neither cached nor shared with later callers.
    generation: u64,
}

struct Inner {
    account: ExternalAccount,
    scopes: Vec<String>,
    source: Arc<dyn ExternalTokenSource>,
    now: Now,
    state: Mutex<State>,
}

impl Inner {
    /// Swift actors are re-entrant at every `await`, so no guard from here is ever held across one.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn finish(&self, started: u64, token: Option<&ExternalAccessToken>) {
        let mut state = self.lock();
        if started != state.generation {
            return;
        }
        state.inflight = None;
        if let Some(token) = token
            && seconds_between((self.now)(), token.expires_at)
                > ExternalAccessTokenProvider::SAFETY_MARGIN
        {
            state.cached = Some(token.clone());
        }
    }

    async fn perform_request(&self, force_refresh: bool) -> Outcome<ExternalAccessToken> {
        match self
            .source
            .access_token(&self.account, &self.scopes, force_refresh)
            .await
        {
            Ok(token) => Ok(token),
            Err(error @ Failure::Source(_)) => Err(error),
            Err(Failure::Cancelled) => Err(Failure::Cancelled),
            // Anything else could carry account data in its text; it is dropped.
            Err(_) => Err(Failure::Source(SourceError::InvalidResponse(
                "the Windows account service failed".to_string(),
            ))),
        }
    }
}

/// Hands out a valid access token from a host-supplied `ExternalTokenSource`. Behaves like `AccessTokenProvider`
/// where that applies: a cached token until 60 s before it expires, and concurrent callers share one in-flight
/// request. After `invalidate()` the next request passes `force_refresh: true` (once).
///
/// Errors: `Failure::Source` and `Failure::Cancelled` pass through; anything else becomes
/// `SourceError::InvalidResponse("the Windows account service failed")` without the underlying description, which
/// could carry account data.
pub struct ExternalAccessTokenProvider {
    inner: Arc<Inner>,
}

impl ExternalAccessTokenProvider {
    const SAFETY_MARGIN: f64 = 60.0;

    pub fn new(
        account: ExternalAccount,
        scopes: Vec<String>,
        source: Arc<dyn ExternalTokenSource>,
        now: Now,
    ) -> ExternalAccessTokenProvider {
        ExternalAccessTokenProvider {
            inner: Arc::new(Inner {
                account,
                scopes,
                source,
                now,
                state: Mutex::new(State::default()),
            }),
        }
    }

    pub async fn access_token(&self) -> Outcome<String> {
        let cached = self.inner.lock().cached.clone();
        if let Some(cached) = cached
            && seconds_between((self.inner.now)(), cached.expires_at) > Self::SAFETY_MARGIN
        {
            return Ok(cached.access_token);
        }
        Ok(self.request_shared().await?.access_token)
    }

    pub fn invalidate(&self) {
        let mut state = self.inner.lock();
        state.cached = None;
        state.force_next_request = true;
        state.inflight = None;
        state.generation += 1;
    }

    async fn request_shared(&self) -> Outcome<ExternalAccessToken> {
        let flight = {
            let mut state = self.inner.lock();
            if let Some(inflight) = &state.inflight {
                inflight.clone()
            } else if let Some(fresh) = state.cached.clone()
                && seconds_between((self.inner.now)(), fresh.expires_at) > Self::SAFETY_MARGIN
            {
                // A request finished between the caller's cache check and this lock (Swift's actor makes the two
                // one step); use its answer instead of starting a second request.
                return Ok(fresh);
            } else {
                let force_refresh = state.force_next_request;
                state.force_next_request = false;
                let started = state.generation;
                // The flight clears `inflight` itself when it finishes, so a cancelled first caller cannot let a
                // second request start while this one is still running.
                let inner = self.inner.clone();
                let flight = SharedFlight::new(async move {
                    match inner.perform_request(force_refresh).await {
                        Ok(token) => {
                            inner.finish(started, Some(&token));
                            Ok(token)
                        }
                        Err(error) => {
                            inner.finish(started, None);
                            Err(error)
                        }
                    }
                });
                state.inflight = Some(flight.clone());
                flight
            }
        };
        flight.value().await
    }
}

#[async_trait]
impl AccessTokenSupplying for ExternalAccessTokenProvider {
    async fn access_token(&self) -> Outcome<String> {
        ExternalAccessTokenProvider::access_token(self).await
    }

    async fn invalidate(&self) {
        ExternalAccessTokenProvider::invalidate(self);
    }
}
