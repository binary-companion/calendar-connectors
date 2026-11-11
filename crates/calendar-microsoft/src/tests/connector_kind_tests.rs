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

use super::support::{calendars_json_ids, instant_sleeper, ok};
use crate::{MicrosoftConnectorKind, MicrosoftOAuthConfig};

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
            Mode::Ok => format!("http://localhost:53211/?code=the-code&state={state}"),
            Mode::Denied => format!("http://localhost:53211/?error=access_denied&state={state}"),
            Mode::WrongState => "http://localhost:53211/?code=x&state=nope".to_string(),
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

fn kind_with(
    transport: &Arc<FakeTransport>,
    redirect_host: Option<&str>,
) -> MicrosoftConnectorKind {
    MicrosoftConnectorKind::new(
        MicrosoftOAuthConfig::new("cid").with_redirect_host(redirect_host),
        transport.clone(),
        TestNow::default().provider(),
        instant_sleeper(),
    )
    .with_poll_interval(60.0)
}

fn kind(transport: &Arc<FakeTransport>) -> MicrosoftConnectorKind {
    kind_with(transport, Some("localhost"))
}

fn default_me() -> Value {
    json!({"mail": "Me@X.com", "userPrincipalName": "me@x.onmicrosoft.com"})
}

fn routes(transport: &FakeTransport, me: Value, refresh: Option<&str>) {
    let mut token = json!({"access_token": "at", "expires_in": 3600});
    if let Some(refresh) = refresh {
        token["refresh_token"] = Value::String(refresh.to_string());
    }
    transport.route(
        "login.microsoftonline.com/common/oauth2/v2.0/token",
        vec![ok(token)],
    );
    transport.route("graph.microsoft.com/v1.0/me", vec![ok(me)]);
}

fn query(url: &UrlString) -> BTreeMap<String, String> {
    url.query_items()
        .into_iter()
        .map(|(name, value)| (name, value.unwrap_or_default()))
        .collect()
}

fn refresh_secret(token: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(
        AccessTokenProvider::REFRESH_TOKEN_KEY.to_string(),
        token.to_string(),
    )])
}

fn existing() -> Connection {
    Connection::new("microsoft", "c1", "me@x.com").with_config(&[("email", "me@x.com")])
}

#[tokio::test]
async fn authorize_runs_pkce_flow_stores_refresh_token_and_returns_a_connection() {
    let transport = Arc::new(FakeTransport::new());
    routes(&transport, default_me(), Some("rt-1"));
    let store = InMemoryCredentialStore::new();
    let interaction = FakeInteraction::new(Mode::Ok);
    let connection = kind(&transport)
        .authorize(&interaction, &store)
        .await
        .unwrap();

    assert!(
        connection.kind_id == "microsoft"
            && connection.display_name == "me@x.com"
            && connection.config.get("email").map(String::as_str) == Some("me@x.com")
    );
    assert_eq!(
        connection.source_id(),
        format!("microsoft-{}", connection.connection_id)
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
    assert!(
        url.host().as_deref() == Some("login.microsoftonline.com")
            && url.path() == "/common/oauth2/v2.0/authorize"
    );
    let q = query(&url);
    assert!(
        q["client_id"] == "cid"
            && q["response_type"] == "code"
            && q["code_challenge_method"] == "S256"
    );
    // The configured host, the session's port.
    assert_eq!(q["redirect_uri"], "http://localhost:53211");
    assert_eq!(q["prompt"], "select_account");
    assert_eq!(
        q["scope"],
        "offline_access User.Read MailboxSettings.Read Calendars.Read Calendars.Read.Shared"
    );
    assert_eq!(interaction.recorder.closes(), 1);

    let exchange = transport
        .requests_matching("oauth2/v2.0/token")
        .into_iter()
        .next()
        .unwrap();
    let body = String::from_utf8_lossy(&exchange.body.unwrap_or_default()).to_string();
    assert!(
        body.contains("redirect_uri=http%3A%2F%2Flocalhost%3A53211")
            && body.contains("code=the-code")
            && body.contains("code_verifier=")
    );
    // A public client has no secret.
    assert!(!body.contains("client_secret"));
}

#[tokio::test]
async fn the_sessions_own_host_is_used_when_no_redirect_host_is_configured() {
    let transport = Arc::new(FakeTransport::new());
    routes(&transport, default_me(), Some("rt-1"));
    let interaction = FakeInteraction::new(Mode::Ok);
    kind_with(&transport, None)
        .authorize(&interaction, &InMemoryCredentialStore::new())
        .await
        .unwrap();
    let url = interaction.recorder.urls().into_iter().next().unwrap();
    assert_eq!(query(&url)["redirect_uri"], "http://127.0.0.1:53211");
}

#[tokio::test]
async fn the_account_is_known_by_mail_else_user_principal_name() {
    let transport = Arc::new(FakeTransport::new());
    routes(
        &transport,
        json!({"userPrincipalName": "Me@Outlook.com"}),
        Some("rt-1"),
    );
    let connection = kind(&transport)
        .authorize(
            &FakeInteraction::new(Mode::Ok),
            &InMemoryCredentialStore::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        connection.config.get("email").map(String::as_str),
        Some("me@outlook.com")
    );

    let none = Arc::new(FakeTransport::new());
    routes(&none, json!({"mail": ""}), Some("rt-1"));
    let result = kind(&none)
        .authorize(
            &FakeInteraction::new(Mode::Ok),
            &InMemoryCredentialStore::new(),
        )
        .await;
    assert_eq!(
        result.unwrap_err(),
        Failure::Source(SourceError::InvalidResponse(
            "the account has no email address".to_string()
        ))
    );
}

#[tokio::test]
async fn authorize_without_a_refresh_token_fails_and_stores_nothing() {
    let transport = Arc::new(FakeTransport::new());
    routes(&transport, default_me(), None);
    let store = InMemoryCredentialStore::new();
    let result = kind(&transport)
        .authorize(&FakeInteraction::new(Mode::Ok), &store)
        .await;
    assert_eq!(
        result.unwrap_err(),
        Failure::Source(SourceError::InvalidResponse(
            "no refresh token returned".to_string()
        ))
    );
    assert!(store.is_empty());
}

#[tokio::test]
async fn a_cancelled_sign_in_throws_cancelled_and_closes_the_session() {
    let transport = Arc::new(FakeTransport::new());
    routes(&transport, default_me(), Some("rt-1"));
    let interaction = FakeInteraction::new(Mode::Denied);
    let result = kind(&transport)
        .authorize(&interaction, &InMemoryCredentialStore::new())
        .await;
    assert_eq!(result.unwrap_err(), Failure::Cancelled);
    assert_eq!(interaction.recorder.closes(), 1);
    assert!(transport.requests_matching("oauth2/v2.0/token").is_empty());
}

#[tokio::test]
async fn a_wrong_state_is_rejected() {
    let transport = Arc::new(FakeTransport::new());
    routes(&transport, default_me(), Some("rt-1"));
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
async fn reauthorize_keeps_the_connection_and_replaces_the_secret() {
    let transport = Arc::new(FakeTransport::new());
    routes(&transport, default_me(), Some("rt-2"));
    let store = InMemoryCredentialStore::new();
    let connection = existing();
    store
        .set_secrets(refresh_secret("old"), "c1")
        .await
        .unwrap();
    let result = kind(&transport)
        .reauthorize(&connection, &FakeInteraction::new(Mode::Ok), &store)
        .await
        .unwrap();
    assert_eq!(result, connection);
    assert_eq!(
        store.secrets("c1").await.unwrap(),
        Some(refresh_secret("rt-2"))
    );
}

#[tokio::test]
async fn reauthorize_as_another_account_fails_and_keeps_the_old_secret() {
    let transport = Arc::new(FakeTransport::new());
    routes(&transport, json!({"mail": "other@x.com"}), Some("rt-2"));
    let store = InMemoryCredentialStore::new();
    store
        .set_secrets(refresh_secret("old"), "c1")
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
        store.secrets("c1").await.unwrap(),
        Some(refresh_secret("old"))
    );
}

#[tokio::test]
async fn make_source_refreshes_tokens_without_a_secret_and_returns_a_microsoft_source() {
    let transport = Arc::new(FakeTransport::new());
    routes(&transport, default_me(), Some("rt-1"));
    transport.route(
        "me/mailboxSettings/timeZone",
        vec![ok(json!({"value": "UTC"}))],
    );
    transport.route("me/calendars?", vec![ok(calendars_json_ids(&["cal1"]))]);
    let store = Arc::new(InMemoryCredentialStore::new());
    store
        .set_secrets(refresh_secret("rt-1"), "c1")
        .await
        .unwrap();
    let source = kind(&transport)
        .make_source(&existing(), store, Arc::new(InMemorySyncStateStore::new()))
        .unwrap();
    assert_eq!(source.id(), "microsoft-c1");
    assert_eq!(
        source
            .calendars()
            .await
            .unwrap()
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        ["cal1"]
    );
    let refresh = transport
        .requests_matching("oauth2/v2.0/token")
        .into_iter()
        .next()
        .unwrap();
    let body = String::from_utf8_lossy(&refresh.body.unwrap_or_default()).to_string();
    assert!(
        body.contains("grant_type=refresh_token")
            && body.contains("client_id=cid")
            && !body.contains("client_secret")
    );
}

#[test]
fn the_kind_describes_itself() {
    let kind = kind(&Arc::new(FakeTransport::new()));
    assert!(kind.id() == "microsoft" && kind.display_name() == "Microsoft");
    assert_eq!(kind.authorization(), AuthorizationMethod::Oauth);
    assert!(kind.supported_platforms().contains(Platform::MACOS));
}
