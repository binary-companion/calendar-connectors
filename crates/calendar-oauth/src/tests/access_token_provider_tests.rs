use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use calendar_core::connection::{CredentialStore, InMemoryCredentialStore};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::source_types::SourceError;
use calendar_core::time::add_seconds;
use calendar_test_support::fake_transport::TestNow;

use super::pause;
use crate::access_token_provider::{AccessTokenProvider, RefreshFunction};
use crate::access_token_supplying::AccessTokenSupplying;
use crate::oauth::OAuthTokens;

const KEY: &str = AccessTokenProvider::REFRESH_TOKEN_KEY;

fn secrets(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[derive(Default)]
struct RefresherState {
    calls: usize,
    seen: Vec<String>,
}

/// Swift's `Refresher` actor: counts calls, records the refresh tokens it was given, answers after a pause.
struct Refresher {
    state: Mutex<RefresherState>,
    now: TestNow,
    rotate: bool,
}

impl Refresher {
    fn new(now: &TestNow) -> Arc<Refresher> {
        Refresher::with_rotation(now, false)
    }

    fn with_rotation(now: &TestNow, rotate: bool) -> Arc<Refresher> {
        Arc::new(Refresher {
            state: Mutex::new(RefresherState::default()),
            now: now.clone(),
            rotate,
        })
    }

    fn calls(&self) -> usize {
        self.state.lock().unwrap().calls
    }

    fn seen(&self) -> Vec<String> {
        self.state.lock().unwrap().seen.clone()
    }

    async fn refresh(&self, token: String) -> Outcome<OAuthTokens> {
        {
            let mut state = self.state.lock().unwrap();
            state.calls += 1;
            state.seen.push(token);
        }
        pause(5).await;
        let calls = self.calls();
        Ok(OAuthTokens {
            refresh_token: self.rotate.then(|| format!("rt{calls}")),
            ..OAuthTokens::new(format!("at{calls}"), add_seconds(self.now.date(), 3600.0))
        })
    }
}

fn make_provider(
    now: &TestNow,
    refresher: &Arc<Refresher>,
    store: &Arc<InMemoryCredentialStore>,
    initial: Option<OAuthTokens>,
) -> Arc<AccessTokenProvider> {
    let refresher = refresher.clone();
    let refresh: RefreshFunction = Arc::new(move |token| {
        let refresher = refresher.clone();
        Box::pin(async move { refresher.refresh(token).await })
    });
    Arc::new(AccessTokenProvider::new(
        "c1",
        store.clone(),
        refresh,
        now.provider(),
        initial,
    ))
}

async fn store_with(pairs: &[(&str, &str)]) -> Arc<InMemoryCredentialStore> {
    let store = Arc::new(InMemoryCredentialStore::new());
    store.set_secrets(secrets(pairs), "c1").await.unwrap();
    store
}

fn auth_expired() -> Failure {
    Failure::Source(SourceError::AuthExpired)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn caches_until_near_expiry() {
    let now = TestNow::default();
    let store = store_with(&[(KEY, "rt0")]).await;
    let refresher = Refresher::new(&now);
    let provider = make_provider(&now, &refresher, &store, None);
    assert_eq!(provider.access_token().await.unwrap(), "at1");
    assert_eq!(provider.access_token().await.unwrap(), "at1");
    assert_eq!(refresher.calls(), 1);
    now.advance(3600.0 - 30.0); // inside the 60 s safety margin
    assert_eq!(provider.access_token().await.unwrap(), "at2");
    assert_eq!(refresher.calls(), 2);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn concurrent_callers_share_one_refresh() {
    let now = TestNow::default();
    let store = store_with(&[(KEY, "rt0")]).await;
    let refresher = Refresher::new(&now);
    let provider = make_provider(&now, &refresher, &store, None);
    let handles: Vec<_> = (0..10)
        .map(|_| {
            let provider = provider.clone();
            tokio::spawn(async move { provider.access_token().await })
        })
        .collect();
    let mut tokens = Vec::new();
    for handle in handles {
        tokens.push(handle.await.unwrap().unwrap());
    }
    assert_eq!(tokens.len(), 10);
    assert!(tokens.iter().all(|t| t == "at1"));
    assert_eq!(refresher.calls(), 1);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn rotated_refresh_token_is_persisted() {
    let now = TestNow::default();
    let store = store_with(&[(KEY, "rt0"), ("other", "keep")]).await;
    let refresher = Refresher::with_rotation(&now, true);
    let provider = make_provider(&now, &refresher, &store, None);
    provider.access_token().await.unwrap();
    let stored = store.secrets("c1").await.unwrap().unwrap();
    assert_eq!(stored[KEY], "rt1");
    assert_eq!(stored["other"], "keep");
    assert_eq!(refresher.seen(), vec!["rt0".to_string()]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn missing_refresh_token_is_auth_expired() {
    let now = TestNow::default();
    let store = Arc::new(InMemoryCredentialStore::new());
    let provider = make_provider(&now, &Refresher::new(&now), &store, None);
    assert_eq!(provider.access_token().await, Err(auth_expired()));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn invalidate_forces_a_refresh() {
    let now = TestNow::default();
    let store = store_with(&[(KEY, "rt0")]).await;
    let refresher = Refresher::new(&now);
    let provider = make_provider(&now, &refresher, &store, None);
    provider.access_token().await.unwrap();
    provider.invalidate();
    assert_eq!(provider.access_token().await.unwrap(), "at2");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn provider_accepts_reauthorized_credentials_after_invalidation() {
    let now = TestNow::default();
    let store = store_with(&[(KEY, "old")]).await;
    let refresher = Refresher::new(&now);
    let provider = make_provider(&now, &refresher, &store, None);
    assert_eq!(provider.access_token().await.unwrap(), "at1");
    store
        .set_secrets(secrets(&[(KEY, "new")]), "c1")
        .await
        .unwrap();
    provider.invalidate();
    assert_eq!(provider.access_token().await.unwrap(), "at2");
    assert_eq!(refresher.seen(), vec!["old".to_string(), "new".to_string()]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn initial_tokens_are_used_without_touching_the_store() {
    let now = TestNow::default();
    let refresher = Refresher::new(&now);
    let initial = OAuthTokens::new("seed", add_seconds(now.date(), 3600.0));
    let provider = make_provider(
        &now,
        &refresher,
        &Arc::new(InMemoryCredentialStore::new()),
        Some(initial),
    );
    assert_eq!(provider.access_token().await.unwrap(), "seed");
    assert_eq!(refresher.calls(), 0);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn cancelling_the_first_caller_keeps_single_flight() {
    let now = TestNow::default();
    let store = store_with(&[(KEY, "rt0")]).await;
    let refresher = Refresher::new(&now);
    let provider = make_provider(&now, &refresher, &store, None);
    let first = {
        let provider = provider.clone();
        tokio::spawn(async move { provider.access_token().await })
    };
    for _ in 0..200 {
        if refresher.calls() != 0 {
            break;
        }
        pause(1).await;
    }
    assert_eq!(refresher.calls(), 1);
    first.abort();
    let token = provider.access_token().await.unwrap();
    assert_eq!(token, "at1");
    assert_eq!(refresher.calls(), 1);
    let _ = first.await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn rotation_does_not_overwrite_new_credential_lifecycle_or_unrelated_entries() {
    let now = TestNow::default();
    let store = store_with(&[(KEY, "rt0"), ("other", "keep")]).await;
    let refresh: RefreshFunction = {
        let store = store.clone();
        let now = now.clone();
        Arc::new(move |_| {
            let store = store.clone();
            let now = now.clone();
            Box::pin(async move {
                store
                    .set_secrets(
                        secrets(&[(KEY, "rt0"), ("other", "keep"), ("other2", "x")]),
                        "c1",
                    )
                    .await?;
                Ok(OAuthTokens {
                    refresh_token: Some("rt1".to_string()),
                    ..OAuthTokens::new("at", add_seconds(now.date(), 3600.0))
                })
            })
        })
    };
    let provider = AccessTokenProvider::new("c1", store.clone(), refresh, now.provider(), None);
    assert_eq!(provider.access_token().await, Err(auth_expired()));
    let stored = store.secrets("c1").await.unwrap().unwrap();
    assert_eq!(stored[KEY], "rt0");
    assert_eq!(stored["other"], "keep");
    assert_eq!(stored["other2"], "x");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn removal_during_refresh_does_not_recreate_secrets() {
    let now = TestNow::default();
    let store = store_with(&[(KEY, "original")]).await;
    let refresh: RefreshFunction = {
        let store = store.clone();
        let now = now.clone();
        Arc::new(move |_| {
            let store = store.clone();
            let now = now.clone();
            Box::pin(async move {
                store.remove_secrets("c1").await?;
                Ok(OAuthTokens {
                    refresh_token: Some("rotated".to_string()),
                    ..OAuthTokens::new("old-access", add_seconds(now.date(), 3600.0))
                })
            })
        })
    };
    let provider = AccessTokenProvider::new("c1", store.clone(), refresh, now.provider(), None);
    assert_eq!(provider.access_token().await, Err(auth_expired()));
    assert_eq!(store.secrets("c1").await.unwrap(), None);
    assert_eq!(provider.access_token().await, Err(auth_expired()));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn reauthorization_during_refresh_preserves_new_secrets() {
    let now = TestNow::default();
    let store = store_with(&[(KEY, "original")]).await;
    let refresh: RefreshFunction = {
        let store = store.clone();
        let now = now.clone();
        Arc::new(move |_| {
            let store = store.clone();
            let now = now.clone();
            Box::pin(async move {
                store
                    .set_secrets(secrets(&[(KEY, "new-sign-in")]), "c1")
                    .await?;
                Ok(OAuthTokens {
                    refresh_token: Some("old-rotation".to_string()),
                    ..OAuthTokens::new("old-access", add_seconds(now.date(), 3600.0))
                })
            })
        })
    };
    let provider = AccessTokenProvider::new("c1", store.clone(), refresh, now.provider(), None);
    assert_eq!(provider.access_token().await, Err(auth_expired()));
    assert_eq!(
        store.secrets("c1").await.unwrap().unwrap()[KEY],
        "new-sign-in"
    );
}

/// Not in the Swift tests, pinned by the Swift code: a cached token bound to a credential revision stops being
/// served once the credentials were replaced (removed or re-authorized) behind the provider's back.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_cached_token_is_dropped_when_the_credentials_change_behind_it() {
    let now = TestNow::default();
    let store = store_with(&[(KEY, "rt0")]).await;
    let refresher = Refresher::new(&now);
    let provider = make_provider(&now, &refresher, &store, None);
    assert_eq!(provider.access_token().await.unwrap(), "at1");
    store
        .set_secrets(secrets(&[(KEY, "rt-new")]), "c1")
        .await
        .unwrap();
    assert_eq!(provider.access_token().await, Err(auth_expired()));
    // The binding is cleared, so the next call refreshes with the new credentials.
    assert_eq!(provider.access_token().await.unwrap(), "at2");
    assert_eq!(
        refresher.seen(),
        vec!["rt0".to_string(), "rt-new".to_string()]
    );
}

/// A failed refresh is shared by the callers waiting on it and is not remembered: the next call asks again.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_failed_refresh_is_shared_and_not_cached() {
    let now = TestNow::default();
    let store = store_with(&[(KEY, "rt0")]).await;
    let calls = Arc::new(Mutex::new(0usize));
    let refresh: RefreshFunction = {
        let calls = calls.clone();
        let now = now.clone();
        Arc::new(move |_| {
            let calls = calls.clone();
            let now = now.clone();
            Box::pin(async move {
                let n = {
                    let mut c = calls.lock().unwrap();
                    *c += 1;
                    *c
                };
                pause(3).await;
                if n == 1 {
                    Err(Failure::Source(SourceError::Network("offline".to_string())))
                } else {
                    Ok(OAuthTokens::new("ok", add_seconds(now.date(), 3600.0)))
                }
            })
        })
    };
    let provider = Arc::new(AccessTokenProvider::new(
        "c1",
        store,
        refresh,
        now.provider(),
        None,
    ));
    let a = {
        let p = provider.clone();
        tokio::spawn(async move { p.access_token().await })
    };
    let b = {
        let p = provider.clone();
        tokio::spawn(async move { p.access_token().await })
    };
    let offline = Err(Failure::Source(SourceError::Network("offline".to_string())));
    assert_eq!(a.await.unwrap(), offline);
    assert_eq!(b.await.unwrap(), offline);
    assert_eq!(*calls.lock().unwrap(), 1);
    assert_eq!(provider.access_token().await.unwrap(), "ok");
    assert_eq!(*calls.lock().unwrap(), 2);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_provider_works_through_the_supplying_trait() {
    let now = TestNow::default();
    let store = store_with(&[(KEY, "rt")]).await;
    let supplying: Arc<dyn AccessTokenSupplying> =
        make_provider(&now, &Refresher::new(&now), &store, None);
    assert_eq!(supplying.access_token().await.unwrap(), "at1");
    supplying.invalidate().await;
    assert_eq!(supplying.access_token().await.unwrap(), "at2");
}
