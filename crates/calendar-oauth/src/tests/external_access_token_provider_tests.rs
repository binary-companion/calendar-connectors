use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use calendar_core::connection::{CredentialStore, InMemoryCredentialStore};
use calendar_core::external_tokens::{
    ExternalAccessToken, ExternalAccount, ExternalSignInRequest, ExternalSignInResult,
    ExternalTokenSource,
};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::source_types::SourceError;
use calendar_core::time::add_seconds;
use calendar_test_support::fake_transport::TestNow;

use super::pause;
use crate::access_token_provider::{AccessTokenProvider, RefreshFunction};
use crate::access_token_supplying::AccessTokenSupplying;
use crate::external_access_token_provider::ExternalAccessTokenProvider;
use crate::oauth::OAuthTokens;

#[derive(Default)]
struct CountingState {
    force_refresh_flags: Vec<bool>,
    scopes_seen: Vec<Vec<String>>,
    accounts_seen: Vec<ExternalAccount>,
    failure: Option<Failure>,
}

/// Counts silent requests and records each `force_refresh`. Tokens are numbered `ext-1`, `ext-2`, ... and live for
/// `lifetime` seconds on the test clock; `failure` makes every request fail.
struct CountingSource {
    state: Mutex<CountingState>,
    now: TestNow,
    lifetime: f64,
    delay: usize,
}

impl CountingSource {
    fn new(now: &TestNow) -> CountingSource {
        CountingSource::configured(now, 3600.0, 5, None)
    }

    fn configured(
        now: &TestNow,
        lifetime: f64,
        delay: usize,
        failure: Option<Failure>,
    ) -> CountingSource {
        CountingSource {
            state: Mutex::new(CountingState {
                failure,
                ..CountingState::default()
            }),
            now: now.clone(),
            lifetime,
            delay,
        }
    }

    fn failing(now: &TestNow, failure: Failure) -> CountingSource {
        CountingSource::configured(now, 3600.0, 5, Some(failure))
    }

    fn calls(&self) -> usize {
        self.state.lock().unwrap().force_refresh_flags.len()
    }

    fn force_refresh_flags(&self) -> Vec<bool> {
        self.state.lock().unwrap().force_refresh_flags.clone()
    }

    fn set_failure(&self, failure: Option<Failure>) {
        self.state.lock().unwrap().failure = failure;
    }
}

#[async_trait]
impl ExternalTokenSource for CountingSource {
    async fn access_token(
        &self,
        account: &ExternalAccount,
        scopes: &[String],
        force_refresh: bool,
    ) -> Outcome<ExternalAccessToken> {
        let number = {
            let mut state = self.state.lock().unwrap();
            state.force_refresh_flags.push(force_refresh);
            state.scopes_seen.push(scopes.to_vec());
            state.accounts_seen.push(account.clone());
            state.force_refresh_flags.len()
        };
        pause(self.delay).await;
        if let Some(failure) = self.state.lock().unwrap().failure.clone() {
            return Err(failure);
        }
        Ok(ExternalAccessToken::new(
            format!("ext-{number}"),
            add_seconds(self.now.date(), self.lifetime),
        ))
    }

    async fn sign_in(&self, _request: &ExternalSignInRequest) -> Outcome<ExternalSignInResult> {
        Err(Failure::Source(SourceError::InvalidResponse(
            "the silent provider never signs in".to_string(),
        )))
    }
}

fn account() -> ExternalAccount {
    ExternalAccount {
        id: "acct-1".to_string(),
        email: Some("me@example.com".to_string()),
        connection_id: Some("c1".to_string()),
    }
}

fn scopes() -> Vec<String> {
    vec!["User.Read".to_string(), "Calendars.Read".to_string()]
}

fn provider(source: &Arc<CountingSource>, now: &TestNow) -> Arc<ExternalAccessTokenProvider> {
    Arc::new(ExternalAccessTokenProvider::new(
        account(),
        scopes(),
        source.clone(),
        now.provider(),
    ))
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn first_call_requests_a_token_with_force_refresh_false() {
    let now = TestNow::default();
    let source = Arc::new(CountingSource::new(&now));
    let token = provider(&source, &now).access_token().await.unwrap();
    assert_eq!(token, "ext-1");
    assert_eq!(source.force_refresh_flags(), vec![false]);
    assert_eq!(source.state.lock().unwrap().scopes_seen, vec![scopes()]);
    assert_eq!(source.state.lock().unwrap().accounts_seen, vec![account()]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn cached_token_is_served_until_sixty_seconds_before_expiry() {
    let now = TestNow::default();
    let source = Arc::new(CountingSource::new(&now));
    let provider = provider(&source, &now);
    assert_eq!(provider.access_token().await.unwrap(), "ext-1");
    now.advance(3539.0);
    assert_eq!(provider.access_token().await.unwrap(), "ext-1");
    assert_eq!(source.calls(), 1);
    now.advance(2.0); // 3541 s: inside the 60 s safety margin
    assert_eq!(provider.access_token().await.unwrap(), "ext-2");
    assert_eq!(source.calls(), 2);
    assert_eq!(source.force_refresh_flags(), vec![false, false]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn concurrent_callers_share_one_in_flight_request() {
    let now = TestNow::default();
    let source = Arc::new(CountingSource::new(&now));
    let provider = provider(&source, &now);
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
    assert!(tokens.iter().all(|t| t == "ext-1") && tokens.len() == 10);
    assert_eq!(source.calls(), 1);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn invalidate_makes_the_next_request_force_refresh_once() {
    let now = TestNow::default();
    let source = Arc::new(CountingSource::configured(&now, 120.0, 5, None));
    let provider = provider(&source, &now);
    assert_eq!(provider.access_token().await.unwrap(), "ext-1");
    provider.invalidate();
    assert_eq!(provider.access_token().await.unwrap(), "ext-2");
    assert_eq!(provider.access_token().await.unwrap(), "ext-2"); // cached again
    now.advance(100.0);
    assert_eq!(provider.access_token().await.unwrap(), "ext-3"); // expiry refresh: not forced
    assert_eq!(source.force_refresh_flags(), vec![false, true, false]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn invalidate_while_a_request_is_in_flight_does_not_reuse_or_cache_its_answer() {
    let now = TestNow::default();
    let source = Arc::new(CountingSource::configured(&now, 3600.0, 16, None));
    let provider = provider(&source, &now);
    let stale = {
        let provider = provider.clone();
        tokio::spawn(async move { provider.access_token().await })
    };
    pause(4).await;
    provider.invalidate();
    let fresh = provider.access_token().await.unwrap();
    let _ = stale.await.unwrap().unwrap();
    assert_eq!(fresh, "ext-2");
    assert_eq!(source.force_refresh_flags(), vec![false, true]);
    // the answer cached is the post-invalidate one
    assert_eq!(provider.access_token().await.unwrap(), "ext-2");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn host_auth_expired_passes_through() {
    let now = TestNow::default();
    let source = Arc::new(CountingSource::failing(
        &now,
        Failure::Source(SourceError::AuthExpired),
    ));
    assert_eq!(
        provider(&source, &now).access_token().await,
        Err(Failure::Source(SourceError::AuthExpired))
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn host_network_error_passes_through() {
    let now = TestNow::default();
    let offline = Failure::Source(SourceError::Network("offline".to_string()));
    let source = Arc::new(CountingSource::failing(&now, offline.clone()));
    assert_eq!(provider(&source, &now).access_token().await, Err(offline));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_failed_request_is_not_cached_and_the_next_call_asks_again() {
    let now = TestNow::default();
    let offline = Failure::Source(SourceError::Network("offline".to_string()));
    let source = Arc::new(CountingSource::failing(&now, offline.clone()));
    let provider = provider(&source, &now);
    assert_eq!(provider.access_token().await, Err(offline));
    source.set_failure(None);
    assert_eq!(provider.access_token().await.unwrap(), "ext-2");
    assert_eq!(source.calls(), 2);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn cancellation_passes_through() {
    let now = TestNow::default();
    let source = Arc::new(CountingSource::failing(&now, Failure::Cancelled));
    assert_eq!(
        provider(&source, &now).access_token().await,
        Err(Failure::Cancelled)
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn unknown_error_becomes_invalid_response_without_its_message() {
    let now = TestNow::default();
    let source = Arc::new(CountingSource::failing(
        &now,
        Failure::Host("no consent for secret@example.com (acct-1, token-xyz)".to_string()),
    ));
    let result = provider(&source, &now).access_token().await;
    let expected = SourceError::InvalidResponse("the Windows account service failed".to_string());
    assert_eq!(result, Err(Failure::Source(expected.clone())));
    let text = format!("{:?} {expected}", result);
    assert!(!text.contains("secret@example.com") && !text.contains("token-xyz"));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn already_expired_token_from_the_host_is_not_cached_across_calls() {
    let now = TestNow::default();
    let source = Arc::new(CountingSource::configured(&now, -10.0, 5, None));
    let provider = provider(&source, &now);
    assert_eq!(provider.access_token().await.unwrap(), "ext-1"); // one call asks the host once: no loop
    assert_eq!(source.calls(), 1);
    assert_eq!(provider.access_token().await.unwrap(), "ext-2"); // the next call asks again
    assert_eq!(source.calls(), 2);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_existing_provider_adopts_the_supplying_protocol() {
    let now = TestNow::default();
    let store = Arc::new(InMemoryCredentialStore::new());
    store
        .set_secrets(
            [(
                AccessTokenProvider::REFRESH_TOKEN_KEY.to_string(),
                "rt".to_string(),
            )]
            .into(),
            "c1",
        )
        .await
        .unwrap();
    let refresh: RefreshFunction = {
        let now = now.clone();
        Arc::new(move |_| {
            let now = now.clone();
            Box::pin(async move { Ok(OAuthTokens::new("at", add_seconds(now.date(), 3600.0))) })
        })
    };
    let real = AccessTokenProvider::new("c1", store, refresh, now.provider(), None);
    let supplying: &dyn AccessTokenSupplying = &real;
    assert_eq!(supplying.access_token().await.unwrap(), "at");
    supplying.invalidate().await;
}

/// Not in the Swift tests: the external provider is a supplying provider too, and a cancelled first caller does not
/// start a second request.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn cancelling_the_first_caller_keeps_single_flight() {
    let now = TestNow::default();
    let source = Arc::new(CountingSource::new(&now));
    let provider = provider(&source, &now);
    let first = {
        let provider = provider.clone();
        tokio::spawn(async move { provider.access_token().await })
    };
    pause(2).await;
    assert_eq!(source.calls(), 1);
    first.abort();
    let supplying: Arc<dyn AccessTokenSupplying> = provider;
    assert_eq!(supplying.access_token().await.unwrap(), "ext-1");
    assert_eq!(source.calls(), 1);
    let _ = first.await;
}
