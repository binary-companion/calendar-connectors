//! Port of `ConnectorKindTests.swift`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use calendar_core::connection::{
    AuthorizationInteraction, AuthorizationMethod, Connection, ConnectorKind, CredentialField,
    CredentialStore, InMemoryCredentialStore, InMemorySyncStateStore, OAuthRedirectSession,
    Platform,
};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::source_types::SourceError;
use calendar_core::url_string::UrlString;
use calendar_oauth::{AccessTokenProvider, AuthorizationError};
use calendar_test_support::fake_transport::{FakeTransport, TestNow};
use serde_json::{Value, json};

use super::support::{instant_sleeper, ok, status};
use crate::{GoogleConnectorKind, GoogleOAuthConfig};

#[derive(Clone, Copy)]
enum Mode {
    Ok,
    Denied,
    WrongState,
}

#[derive(Default)]
struct SessionRecorder {
    authorization_urls: Mutex<Vec<UrlString>>,
    close_count: Mutex<usize>,
}

impl SessionRecorder {
    fn urls(&self) -> Vec<UrlString> {
        self.authorization_urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn closes(&self) -> usize {
        *self
            .close_count
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

struct FakeSession {
    recorder: Arc<SessionRecorder>,
    mode: Mode,
}

#[async_trait]
impl OAuthRedirectSession for FakeSession {
    fn redirect_uri(&self) -> UrlString {
        UrlString::parse("http://127.0.0.1:53211").unwrap()
    }

    async fn authorize(&self, authorization_url: &UrlString) -> Outcome<UrlString> {
        self.recorder
            .authorization_urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(authorization_url.clone());
        let state = authorization_url.query_value("state").unwrap();
        let text = match self.mode {
            Mode::Ok => format!("http://127.0.0.1:53211/?code=the-code&state={state}"),
            Mode::Denied => format!("http://127.0.0.1:53211/?error=access_denied&state={state}"),
            Mode::WrongState => "http://127.0.0.1:53211/?code=x&state=nope".to_string(),
        };
        Ok(UrlString::parse(&text).unwrap())
    }

    async fn close(&self) {
        *self
            .recorder
            .close_count
            .lock()
            .unwrap_or_else(PoisonError::into_inner) += 1;
    }
}

struct FakeInteraction {
    recorder: Arc<SessionRecorder>,
    mode: Mode,
}

impl FakeInteraction {
    fn new(mode: Mode) -> FakeInteraction {
        FakeInteraction {
            recorder: Arc::new(SessionRecorder::default()),
            mode,
        }
    }
}

#[async_trait]
impl AuthorizationInteraction for FakeInteraction {
    async fn begin_oauth_redirect(&self) -> Outcome<Box<dyn OAuthRedirectSession>> {
        Ok(Box::new(FakeSession {
            recorder: self.recorder.clone(),
            mode: self.mode,
        }))
    }

    async fn prompt_credentials(
        &self,
        _fields: &[CredentialField],
    ) -> Outcome<BTreeMap<String, String>> {
        Ok(BTreeMap::new())
    }
}

fn kind(transport: &Arc<FakeTransport>) -> GoogleConnectorKind {
    GoogleConnectorKind::new(
        GoogleOAuthConfig::new("cid", "csecret"),
        transport.clone(),
        TestNow::default().provider(),
        instant_sleeper(),
    )
    .with_poll_interval(60.0)
}

fn routes(transport: &FakeTransport, email: &str, refresh: Option<&str>) {
    let mut token = json!({"access_token": "at", "expires_in": 3600});
    if let Some(refresh) = refresh {
        token["refresh_token"] = Value::String(refresh.to_string());
    }
    transport.route("oauth2.googleapis.com/token", vec![ok(token)]);
    transport.route(
        "users/me/calendarList",
        vec![ok(
            json!({"items": [{"id": email, "primary": true, "accessRole": "owner"}]}),
        )],
    );
}

fn refresh_secret(token: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(
        AccessTokenProvider::REFRESH_TOKEN_KEY.to_string(),
        token.to_string(),
    )])
}

fn existing() -> Connection {
    Connection::new("google", "keep-me", "me@x.com").with_config(&[("email", "me@x.com")])
}

#[tokio::test]
async fn authorize_runs_pkce_flow_stores_refresh_token_and_returns_a_connection() {
    let transport = Arc::new(FakeTransport::new());
    routes(&transport, "me@x.com", Some("rt-1"));
    let store = InMemoryCredentialStore::new();
    let interaction = FakeInteraction::new(Mode::Ok);
    let connection = kind(&transport)
        .authorize(&interaction, &store)
        .await
        .unwrap();

    assert!(
        connection.kind_id == "google"
            && connection.display_name == "me@x.com"
            && connection.config.get("email").map(String::as_str) == Some("me@x.com")
    );
    assert_eq!(
        connection.source_id(),
        format!("google-{}", connection.connection_id)
    );
    assert!(uuid::Uuid::parse_str(&connection.connection_id).is_ok());
    let secrets = store
        .secrets(&connection.connection_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        secrets
            .get(AccessTokenProvider::REFRESH_TOKEN_KEY)
            .map(String::as_str),
        Some("rt-1")
    );

    let url = interaction.recorder.urls().into_iter().next().unwrap();
    let q: BTreeMap<String, String> = url
        .query_items()
        .into_iter()
        .map(|(name, value)| (name, value.unwrap_or_default()))
        .collect();
    assert!(q["client_id"] == "cid" && q["redirect_uri"] == "http://127.0.0.1:53211");
    assert!(
        q["access_type"] == "offline"
            && q["prompt"] == "consent"
            && q["code_challenge_method"] == "S256"
    );
    assert_eq!(
        q["scope"],
        "https://www.googleapis.com/auth/calendar.events.readonly https://www.googleapis.com/auth/calendar.calendarlist.readonly"
    );
    assert_eq!(interaction.recorder.closes(), 1);

    let exchange = transport
        .requests_matching("oauth2.googleapis.com/token")
        .into_iter()
        .next()
        .unwrap();
    let body = String::from_utf8_lossy(&exchange.body.unwrap_or_default()).to_string();
    assert!(
        body.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A53211")
            && body.contains("code=the-code")
    );
    assert!(body.contains("client_secret=csecret"));
}

#[tokio::test]
async fn authorize_without_a_refresh_token_fails_and_stores_nothing() {
    let transport = Arc::new(FakeTransport::new());
    routes(&transport, "me@x.com", None);
    let store = InMemoryCredentialStore::new();
    let interaction = FakeInteraction::new(Mode::Ok);
    let result = kind(&transport).authorize(&interaction, &store).await;
    assert!(matches!(result, Err(Failure::Source(_))));
    assert!(store.is_empty());
    assert_eq!(interaction.recorder.closes(), 1);
}

#[tokio::test]
async fn authorize_failure_after_token_exchange_stores_nothing() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(
        "oauth2.googleapis.com/token",
        vec![ok(
            json!({"access_token": "at", "expires_in": 3600, "refresh_token": "rt"}),
        )],
    );
    transport.route("users/me/calendarList", vec![status(json!({}), 500)]);
    let store = InMemoryCredentialStore::new();
    let result = kind(&transport)
        .authorize(&FakeInteraction::new(Mode::Ok), &store)
        .await;
    assert_eq!(
        result.unwrap_err(),
        Failure::Source(SourceError::Server { status: 500 })
    );
    // Nothing is keyed by an id the caller never received: the store is empty for every id we could have used.
    assert!(store.is_empty());
}

#[tokio::test]
async fn user_denying_access_throws_cancelled_and_closes_the_session() {
    let transport = Arc::new(FakeTransport::new());
    routes(&transport, "me@x.com", Some("rt-1"));
    let interaction = FakeInteraction::new(Mode::Denied);
    let result = kind(&transport)
        .authorize(&interaction, &InMemoryCredentialStore::new())
        .await;
    assert_eq!(result.unwrap_err(), Failure::Cancelled);
    assert_eq!(interaction.recorder.closes(), 1);
    assert!(
        transport
            .requests_matching("oauth2.googleapis.com/token")
            .is_empty()
    );
}

#[tokio::test]
async fn a_mismatched_state_is_rejected() {
    let transport = Arc::new(FakeTransport::new());
    routes(&transport, "me@x.com", Some("rt-1"));
    let result = kind(&transport)
        .authorize(
            &FakeInteraction::new(Mode::WrongState),
            &InMemoryCredentialStore::new(),
        )
        .await;
    assert_eq!(
        result.unwrap_err(),
        Failure::Host(AuthorizationError::StateMismatch.to_string())
    );
}

#[tokio::test]
async fn reauthorize_keeps_the_connection_id_and_replaces_secrets() {
    let transport = Arc::new(FakeTransport::new());
    routes(&transport, "me@x.com", Some("rt-2"));
    let store = InMemoryCredentialStore::new();
    store
        .set_secrets(refresh_secret("rt-old"), "keep-me")
        .await
        .unwrap();
    let existing = existing();
    let result = kind(&transport)
        .reauthorize(&existing, &FakeInteraction::new(Mode::Ok), &store)
        .await
        .unwrap();
    assert_eq!(result, existing);
    assert_eq!(
        store.secrets("keep-me").await.unwrap(),
        Some(refresh_secret("rt-2"))
    );
}

#[tokio::test]
async fn reauthorize_as_a_different_account_is_rejected_and_keeps_old_secrets() {
    let transport = Arc::new(FakeTransport::new());
    routes(&transport, "other@x.com", Some("rt-2"));
    let store = InMemoryCredentialStore::new();
    store
        .set_secrets(refresh_secret("rt-old"), "keep-me")
        .await
        .unwrap();
    let result = kind(&transport)
        .reauthorize(&existing(), &FakeInteraction::new(Mode::Ok), &store)
        .await;
    assert_eq!(
        result.unwrap_err(),
        Failure::Source(SourceError::InvalidResponse(
            "signed in as a different account".to_string()
        ))
    );
    assert_eq!(
        store.secrets("keep-me").await.unwrap(),
        Some(refresh_secret("rt-old"))
    );
}

#[tokio::test]
async fn make_source_refreshes_and_lists_calendars_without_prompting() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(
        "oauth2.googleapis.com/token",
        vec![ok(json!({"access_token": "fresh", "expires_in": 3600}))],
    );
    transport.route(
        "users/me/calendarList",
        vec![ok(
            json!({"items": [{"id": "me@x.com", "primary": true, "accessRole": "owner"}]}),
        )],
    );
    let store = Arc::new(InMemoryCredentialStore::new());
    store.set_secrets(refresh_secret("rt"), "c9").await.unwrap();
    let connection =
        Connection::new("google", "c9", "me@x.com").with_config(&[("email", "me@x.com")]);
    let source = kind(&transport)
        .make_source(&connection, store, Arc::new(InMemorySyncStateStore::new()))
        .unwrap();
    assert_eq!(source.id(), "google-c9");
    assert_eq!(
        source
            .calendars()
            .await
            .unwrap()
            .iter()
            .map(|c| c.id.clone())
            .collect::<Vec<_>>(),
        ["me@x.com"]
    );
    assert_eq!(
        transport
            .requests_matching("calendarList")
            .first()
            .and_then(|r| r.headers.get("Authorization").cloned())
            .as_deref(),
        Some("Bearer fresh")
    );
}

#[test]
fn kind_metadata() {
    let k = kind(&Arc::new(FakeTransport::new()));
    assert!(k.id() == "google" && k.display_name() == "Google");
    assert!(
        k.supported_platforms().contains(Platform::MACOS)
            && k.supported_platforms().contains(Platform::LINUX)
    );
    assert!(matches!(k.authorization(), AuthorizationMethod::Oauth));
}

#[tokio::test]
async fn authorize_maps_provider_errors_from_the_calendar_list_to_source_error() {
    let transport = Arc::new(FakeTransport::new());
    routes(&transport, "me@x.com", Some("rt-1"));
    transport.route(
        "users/me/calendarList",
        vec![status(json!({"error": {"code": 404}}), 404)],
    );
    let store = InMemoryCredentialStore::new();
    let result = kind(&transport)
        .authorize(&FakeInteraction::new(Mode::Ok), &store)
        .await;
    assert!(matches!(result, Err(Failure::Source(_))));
    assert!(store.is_empty());
}

#[tokio::test]
async fn reauthorize_maps_provider_errors_from_the_calendar_list_to_source_error() {
    let transport = Arc::new(FakeTransport::new());
    routes(&transport, "me@x.com", Some("rt-2"));
    transport.route(
        "users/me/calendarList",
        vec![status(json!({"error": {"code": 404}}), 404)],
    );
    let store = InMemoryCredentialStore::new();
    store
        .set_secrets(refresh_secret("rt-old"), "keep-me")
        .await
        .unwrap();
    let result = kind(&transport)
        .reauthorize(&existing(), &FakeInteraction::new(Mode::Ok), &store)
        .await;
    assert!(matches!(result, Err(Failure::Source(_))));
    assert_eq!(
        store.secrets("keep-me").await.unwrap(),
        Some(refresh_secret("rt-old"))
    );
}
