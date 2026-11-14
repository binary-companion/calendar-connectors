//! Port of `BrokeredConnectorKindTests.swift`.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use calendar_core::connection::{
    AuthorizationInteraction, AuthorizationMethod, Connection, ConnectorKind, CredentialField,
    InMemoryCredentialStore, InMemorySyncStateStore, OAuthRedirectSession, Platform,
};
use calendar_core::external_tokens::{ExternalAccessToken, ExternalAccount, ExternalSignInResult};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::source_types::SourceError;
use calendar_core::time::add_seconds;
use calendar_test_support::fake_external_token_source::{FakeExternalTokenSource, TokenCall};
use calendar_test_support::fake_transport::{FakeTransport, TestNow};
use jiff::Timestamp;
use serde_json::{Value, json};

use super::support::{calendars_json_ids, instant_sleeper, ok, status, text};
use crate::{MicrosoftBrokeredConnectorKind, MicrosoftConnectorKind, MicrosoftOAuthConfig};

fn t0() -> Timestamp {
    Timestamp::from_second(1_800_000_000).unwrap()
}

fn token(text: &str, expires_in: f64) -> ExternalAccessToken {
    ExternalAccessToken::new(text, add_seconds(t0(), expires_in))
}

fn signed_in(
    account_id: &str,
    email: Option<&str>,
    token_text: &str,
) -> Outcome<ExternalSignInResult> {
    Ok(ExternalSignInResult::new(
        ExternalAccount {
            id: account_id.to_string(),
            email: email.map(str::to_string),
            connection_id: None,
        },
        token(token_text, 3600.0),
    ))
}

fn signed_in_default(account_id: &str) -> Outcome<ExternalSignInResult> {
    signed_in(account_id, None, "tok-1")
}

fn scopes() -> Vec<String> {
    MicrosoftBrokeredConnectorKind::scopes()
}

fn kind_at(
    source: &Arc<FakeExternalTokenSource>,
    transport: &Arc<FakeTransport>,
    now: &TestNow,
) -> MicrosoftBrokeredConnectorKind {
    MicrosoftBrokeredConnectorKind::new(
        source.clone(),
        transport.clone(),
        now.provider(),
        instant_sleeper(),
    )
    .with_poll_interval(60.0)
}

fn kind(
    source: &Arc<FakeExternalTokenSource>,
    transport: &Arc<FakeTransport>,
) -> MicrosoftBrokeredConnectorKind {
    kind_at(source, transport, &TestNow::new(t0()))
}

fn me_value() -> Value {
    json!({"mail": "me@example.com", "userPrincipalName": "me@example.onmicrosoft.com"})
}

fn me_route(transport: &FakeTransport, me: Value) {
    transport.route("graph.microsoft.com/v1.0/me", vec![ok(me)]);
}

fn stored() -> Connection {
    Connection::new("microsoft-brokered", "c1", "me@example.com").with_config(&[
        ("email", "me@example.com"),
        ("authMode", "external"),
        ("externalAccountID", "acct-1"),
    ])
}

/// Routes for a source's first sync: the time zone and the calendar list.
fn sync_routes(
    transport: &FakeTransport,
    calendars: Option<Vec<calendar_core::http::HttpResponse>>,
) {
    me_route(transport, me_value());
    transport.route(
        "me/mailboxSettings/timeZone",
        vec![ok(json!({"value": "UTC"}))],
    );
    transport.route(
        "me/calendars?",
        calendars.unwrap_or_else(|| vec![ok(calendars_json_ids(&["cal1"]))]),
    );
}

fn unused() -> InMemoryCredentialStore {
    InMemoryCredentialStore::new()
}

fn new_source() -> Arc<FakeExternalTokenSource> {
    Arc::new(FakeExternalTokenSource::new())
}

fn with_sign_ins(results: Vec<Outcome<ExternalSignInResult>>) -> Arc<FakeExternalTokenSource> {
    Arc::new(FakeExternalTokenSource::with_scripts(Vec::new(), results))
}

fn with_tokens(results: Vec<Outcome<ExternalAccessToken>>) -> Arc<FakeExternalTokenSource> {
    Arc::new(FakeExternalTokenSource::with_scripts(results, Vec::new()))
}

/// The brokered kind never uses the browser interaction.
struct NoInteraction;

#[async_trait]
impl AuthorizationInteraction for NoInteraction {
    async fn begin_oauth_redirect(&self) -> Outcome<Box<dyn OAuthRedirectSession>> {
        panic!("the brokered flow must not open a browser");
    }

    async fn prompt_credentials(
        &self,
        _fields: &[CredentialField],
    ) -> Outcome<BTreeMap<String, String>> {
        panic!("the brokered flow must not prompt for credentials");
    }
}

#[tokio::test]
async fn authorize_signs_in_then_reads_me_and_builds_the_connection() {
    let transport = Arc::new(FakeTransport::new());
    me_route(&transport, me_value());
    let source = with_sign_ins(vec![signed_in("acct-1", None, "tok-signin")]);
    let store = InMemoryCredentialStore::new();
    let connection = kind(&source, &transport)
        .authorize(&NoInteraction, &store)
        .await
        .unwrap();

    assert!(
        connection.kind_id == "microsoft-brokered" && connection.display_name == "me@example.com"
    );
    assert!(uuid::Uuid::parse_str(&connection.connection_id).is_ok());
    assert_eq!(
        connection.source_id(),
        format!("microsoft-brokered-{}", connection.connection_id)
    );
    assert_eq!(
        connection.config,
        BTreeMap::from([
            ("email".to_string(), "me@example.com".to_string()),
            ("authMode".to_string(), "external".to_string()),
            ("externalAccountID".to_string(), "acct-1".to_string()),
        ])
    );
    assert!(store.is_empty());

    let calls = source.sign_in_calls();
    assert!(calls.len() == 1 && calls[0].account.is_none() && calls[0].email.is_none());
    assert_eq!(calls[0].scopes, scopes());
    let me = transport
        .requests_matching("/me")
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(
        me.headers.get("Authorization").map(String::as_str),
        Some("Bearer tok-signin")
    );
    assert!(
        me.url.as_str().contains("select=mail,userPrincipalName")
            || me.url.as_str().contains("select=mail%2CuserPrincipalName")
    );
    // The sign-in token serves /me: no second request.
    assert!(source.token_calls().is_empty());
}

#[tokio::test]
async fn authorize_uses_user_principal_name_when_mail_is_missing() {
    let transport = Arc::new(FakeTransport::new());
    me_route(&transport, json!({"userPrincipalName": "upn@example.com"}));
    let connection = kind(
        &with_sign_ins(vec![signed_in_default("acct-1")]),
        &transport,
    )
    .authorize(&NoInteraction, &unused())
    .await
    .unwrap();
    assert!(
        connection.config.get("email").map(String::as_str) == Some("upn@example.com")
            && connection.display_name == "upn@example.com"
    );

    let none = Arc::new(FakeTransport::new());
    me_route(&none, json!({"mail": ""}));
    let result = kind(&with_sign_ins(vec![signed_in_default("acct-1")]), &none)
        .authorize(&NoInteraction, &unused())
        .await;
    assert_eq!(
        result.unwrap_err(),
        Failure::Source(SourceError::InvalidResponse(
            "the account has no email address".to_string()
        ))
    );
}

#[tokio::test]
async fn authorize_lowercases_the_email() {
    let transport = Arc::new(FakeTransport::new());
    me_route(&transport, json!({"mail": "Me@Example.com"}));
    let connection = kind(
        &with_sign_ins(vec![signed_in_default("acct-1")]),
        &transport,
    )
    .authorize(&NoInteraction, &unused())
    .await
    .unwrap();
    assert!(
        connection.config.get("email").map(String::as_str) == Some("me@example.com")
            && connection.display_name == "me@example.com"
    );
}

#[tokio::test]
async fn authorize_ignores_an_email_the_host_claims() {
    let transport = Arc::new(FakeTransport::new());
    me_route(&transport, me_value());
    let connection = kind(
        &with_sign_ins(vec![signed_in(
            "acct-1",
            Some("other@evil.example"),
            "tok-1",
        )]),
        &transport,
    )
    .authorize(&NoInteraction, &unused())
    .await
    .unwrap();
    assert_eq!(
        connection.config.get("email").map(String::as_str),
        Some("me@example.com")
    );
}

#[tokio::test]
async fn authorize_failure_stores_nothing() {
    let store = InMemoryCredentialStore::new();
    // The sign-in throws.
    let transport = Arc::new(FakeTransport::new());
    me_route(&transport, me_value());
    let failing = with_sign_ins(vec![Err(Failure::Source(SourceError::Network(
        "offline".to_string(),
    )))]);
    let result = kind(&failing, &transport)
        .authorize(&NoInteraction, &store)
        .await;
    assert_eq!(
        result.unwrap_err(),
        Failure::Source(SourceError::Network("offline".to_string()))
    );
    assert!(store.is_empty());
    // The sign-in works, /me answers 500.
    let broken = Arc::new(FakeTransport::new());
    broken.route("graph.microsoft.com/v1.0/me", vec![text("boom", 500)]);
    let result = kind(&with_sign_ins(vec![signed_in_default("acct-1")]), &broken)
        .authorize(&NoInteraction, &store)
        .await;
    assert_eq!(
        result.unwrap_err(),
        Failure::Source(SourceError::Server { status: 500 })
    );
    assert!(store.is_empty());
}

#[tokio::test]
async fn a_dismissed_dialog_is_cancellation_and_other_host_failures_do_not_leak_their_text() {
    let transport = Arc::new(FakeTransport::new());
    me_route(&transport, me_value());
    let result = kind(&with_sign_ins(vec![Err(Failure::Cancelled)]), &transport)
        .authorize(&NoInteraction, &unused())
        .await;
    assert_eq!(result.unwrap_err(), Failure::Cancelled);
    let leaky = with_sign_ins(vec![Err(Failure::Host(
        "boom for secret@example.com".to_string(),
    ))]);
    let result = kind(&leaky, &transport)
        .authorize(&NoInteraction, &unused())
        .await;
    assert_eq!(
        result.unwrap_err(),
        Failure::Source(SourceError::InvalidResponse(
            "the Windows account service failed".to_string()
        ))
    );
}

#[tokio::test]
async fn reauthorize_keeps_connection_id_and_takes_new_external_account_id() {
    let transport = Arc::new(FakeTransport::new());
    me_route(&transport, me_value());
    let source = with_sign_ins(vec![signed_in_default("acct-2")]);
    let result = kind(&source, &transport)
        .reauthorize(&stored(), &NoInteraction, &unused())
        .await
        .unwrap();
    assert!(
        result.connection_id == "c1"
            && result.kind_id == "microsoft-brokered"
            && result.display_name == "me@example.com"
    );
    assert_eq!(
        result.config,
        BTreeMap::from([
            ("email".to_string(), "me@example.com".to_string()),
            ("authMode".to_string(), "external".to_string()),
            ("externalAccountID".to_string(), "acct-2".to_string()),
        ])
    );
    let request = source.sign_in_calls().into_iter().next().unwrap();
    assert_eq!(
        request.account,
        Some(ExternalAccount {
            id: "acct-1".to_string(),
            email: Some("me@example.com".to_string()),
            connection_id: Some("c1".to_string())
        })
    );
    assert!(request.email.as_deref() == Some("me@example.com") && request.scopes == scopes());
}

#[tokio::test]
async fn reauthorize_with_a_different_email_throws_invalid_response() {
    let transport = Arc::new(FakeTransport::new());
    me_route(&transport, json!({"mail": "other@example.com"}));
    let result = kind(
        &with_sign_ins(vec![signed_in_default("acct-9")]),
        &transport,
    )
    .reauthorize(&stored(), &NoInteraction, &unused())
    .await;
    assert_eq!(
        result.unwrap_err(),
        Failure::Source(SourceError::InvalidResponse(
            "signed in as a different account".to_string()
        ))
    );
}

#[tokio::test]
async fn reauthorize_compares_the_email_ignoring_case() {
    let transport = Arc::new(FakeTransport::new());
    me_route(&transport, json!({"mail": "ME@Example.com"}));
    let result = kind(
        &with_sign_ins(vec![signed_in_default("acct-2")]),
        &transport,
    )
    .reauthorize(&stored(), &NoInteraction, &unused())
    .await
    .unwrap();
    assert_eq!(
        result.config.get("externalAccountID").map(String::as_str),
        Some("acct-2")
    );
}

#[tokio::test]
async fn reauthorize_works_when_the_stored_account_id_is_missing() {
    // makeSource asks the person to sign in again for such a connection, so that sign-in has to be possible.
    let incomplete = Connection::new("microsoft-brokered", "c1", "me@example.com")
        .with_config(&[("email", "me@example.com"), ("authMode", "external")]);
    let transport = Arc::new(FakeTransport::new());
    me_route(&transport, me_value());
    let source = with_sign_ins(vec![signed_in_default("acct-3")]);
    let result = kind(&source, &transport)
        .reauthorize(&incomplete, &NoInteraction, &unused())
        .await
        .unwrap();
    assert_eq!(
        result.config.get("externalAccountID").map(String::as_str),
        Some("acct-3")
    );
    let request = source.sign_in_calls().into_iter().next().unwrap();
    assert!(request.account.is_none() && request.email.as_deref() == Some("me@example.com"));
}

#[tokio::test]
async fn reauthorize_without_a_stored_email_compares_with_the_display_name() {
    // The display name is the email by construction; a record that lost its `email` key is still guarded.
    let no_email = Connection::new("microsoft-brokered", "c1", "me@example.com")
        .with_config(&[("authMode", "external"), ("externalAccountID", "acct-1")]);
    let transport = Arc::new(FakeTransport::new());
    me_route(&transport, me_value());
    let result = kind(
        &with_sign_ins(vec![signed_in_default("acct-2")]),
        &transport,
    )
    .reauthorize(&no_email, &NoInteraction, &unused())
    .await
    .unwrap();
    assert!(
        result.config.get("externalAccountID").map(String::as_str) == Some("acct-2")
            && result.config.get("email").map(String::as_str) == Some("me@example.com")
    );

    let other = Arc::new(FakeTransport::new());
    me_route(&other, json!({"mail": "other@example.com"}));
    let result = kind(&with_sign_ins(vec![signed_in_default("acct-2")]), &other)
        .reauthorize(&no_email, &NoInteraction, &unused())
        .await;
    assert_eq!(
        result.unwrap_err(),
        Failure::Source(SourceError::InvalidResponse(
            "signed in as a different account".to_string()
        ))
    );
}

#[test]
fn make_source_without_external_account_id_throws_incomplete_settings() {
    for config in [
        vec![("email", "me@example.com")],
        vec![("email", "me@example.com"), ("externalAccountID", "")],
    ] {
        let connection =
            Connection::new("microsoft-brokered", "c1", "me@example.com").with_config(&config);
        let result = kind(&new_source(), &Arc::new(FakeTransport::new())).make_source(
            &connection,
            Arc::new(InMemoryCredentialStore::new()),
            Arc::new(InMemorySyncStateStore::new()),
        );
        assert_eq!(
            result.err(),
            Some(Failure::Source(SourceError::InvalidResponse(
                "the account settings are incomplete; sign in again".to_string()
            )))
        );
    }
}

fn open(
    kind: &MicrosoftBrokeredConnectorKind,
) -> Arc<dyn calendar_core::source_types::CalendarSource> {
    kind.make_source(
        &stored(),
        Arc::new(unused()),
        Arc::new(InMemorySyncStateStore::new()),
    )
    .unwrap()
}

fn ids(calendars: Vec<calendar_core::model::CalendarDescriptor>) -> Vec<String> {
    calendars.into_iter().map(|c| c.id).collect()
}

#[tokio::test]
async fn make_source_returns_a_microsoft_source_that_asks_the_host_for_tokens() {
    let transport = Arc::new(FakeTransport::new());
    sync_routes(&transport, None);
    let source = with_tokens(vec![Ok(token("tok-silent", 3600.0))]);
    let result = open(&kind(&source, &transport));
    assert_eq!(result.id(), "microsoft-brokered-c1");
    assert_eq!(ids(result.calendars().await.unwrap()), ["cal1"]);
    assert_eq!(
        source.token_calls(),
        [TokenCall::new(
            ExternalAccount {
                id: "acct-1".to_string(),
                email: Some("me@example.com".to_string()),
                connection_id: Some("c1".to_string())
            },
            scopes(),
            false
        )]
    );
    assert!(
        transport.requests_matching("me/calendars").iter().all(|r| r
            .headers
            .get("Authorization")
            .map(String::as_str)
            == Some("Bearer tok-silent"))
    );
}

#[tokio::test]
async fn source_refreshes_silently_near_expiry() {
    let transport = Arc::new(FakeTransport::new());
    sync_routes(&transport, None);
    let now = TestNow::new(t0());
    let source = with_tokens(vec![Ok(token("tok-1", 3600.0)), Ok(token("tok-2", 7200.0))]);
    let result = open(&kind_at(&source, &transport, &now));
    result.calendars().await.unwrap();
    now.advance(3500.0);
    result.calendars().await.unwrap();
    // Still cached 100 s before expiry.
    assert_eq!(source.token_calls().len(), 1);
    // 3541 s.
    now.advance(41.0);
    result.calendars().await.unwrap();
    let calls = source.token_calls();
    assert!(calls.len() == 2 && calls.iter().all(|c| !c.force_refresh));
    assert_eq!(
        transport
            .requests_matching("me/calendars")
            .last()
            .and_then(|r| r.headers.get("Authorization").cloned())
            .as_deref(),
        Some("Bearer tok-2")
    );
}

#[tokio::test]
async fn a_401_invalidates_once_then_force_refreshes_then_auth_expired() {
    let transport = Arc::new(FakeTransport::new());
    sync_routes(
        &transport,
        Some(vec![status(
            json!({"error": {"code": "InvalidAuthenticationToken"}}),
            401,
        )]),
    );
    let source = with_tokens(vec![Ok(token("tok-1", 3600.0)), Ok(token("tok-2", 3600.0))]);
    let result = open(&kind(&source, &transport));
    assert_eq!(
        result.calendars().await,
        Err(Failure::Source(SourceError::AuthExpired))
    );
    assert_eq!(
        source
            .token_calls()
            .iter()
            .map(|c| c.force_refresh)
            .collect::<Vec<_>>(),
        [false, true]
    );
}

#[tokio::test]
async fn a_401_then_a_fresh_token_succeeds() {
    let transport = Arc::new(FakeTransport::new());
    sync_routes(
        &transport,
        Some(vec![
            status(json!({}), 401),
            ok(calendars_json_ids(&["cal1"])),
        ]),
    );
    let source = with_tokens(vec![Ok(token("tok-1", 3600.0)), Ok(token("tok-2", 3600.0))]);
    let result = open(&kind(&source, &transport));
    assert_eq!(ids(result.calendars().await.unwrap()), ["cal1"]);
    assert_eq!(
        source
            .token_calls()
            .iter()
            .map(|c| c.force_refresh)
            .collect::<Vec<_>>(),
        [false, true]
    );
}

#[tokio::test]
async fn host_auth_expired_surfaces_as_auth_expired() {
    let transport = Arc::new(FakeTransport::new());
    sync_routes(&transport, None);
    let source = with_tokens(vec![Err(Failure::Source(SourceError::AuthExpired))]);
    let result = open(&kind(&source, &transport));
    assert_eq!(
        result.calendars().await,
        Err(Failure::Source(SourceError::AuthExpired))
    );
    // No Graph request without a token.
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn network_error_passes_through() {
    let transport = Arc::new(FakeTransport::new());
    sync_routes(&transport, None);
    let source = with_tokens(vec![Err(Failure::Source(SourceError::Network(
        "offline".to_string(),
    )))]);
    let result = open(&kind(&source, &transport));
    assert_eq!(
        result.calendars().await,
        Err(Failure::Source(SourceError::Network("offline".to_string())))
    );
}

#[tokio::test]
async fn other_host_failure_is_invalid_response() {
    let transport = Arc::new(FakeTransport::new());
    sync_routes(&transport, None);
    let source = with_tokens(vec![Err(Failure::Host(
        "no consent for me@example.com".to_string(),
    ))]);
    let result = open(&kind(&source, &transport));
    assert_eq!(
        result.calendars().await,
        Err(Failure::Source(SourceError::InvalidResponse(
            "the Windows account service failed".to_string()
        )))
    );
}

#[test]
fn scopes_are_a_subset_of_the_browser_kinds_scopes_without_offline_access() {
    let brokered = MicrosoftBrokeredConnectorKind::scopes();
    assert!(
        brokered
            .iter()
            .all(|scope| MicrosoftConnectorKind::SCOPES.contains(&scope.as_str()))
    );
    assert!(!brokered.iter().any(|scope| scope == "offline_access"));
    assert_eq!(
        brokered,
        [
            "User.Read",
            "MailboxSettings.Read",
            "Calendars.Read",
            "Calendars.Read.Shared"
        ]
    );
}

#[test]
fn scopes_stay_read_only() {
    assert!(
        MicrosoftBrokeredConnectorKind::scopes()
            .iter()
            .all(|scope| !scope.contains("ReadWrite"))
    );
}

#[test]
fn the_brokered_kind_describes_itself() {
    let kind = kind(&new_source(), &Arc::new(FakeTransport::new()));
    assert!(
        kind.id() == "microsoft-brokered"
            && MicrosoftBrokeredConnectorKind::KIND_ID == "microsoft-brokered"
    );
    assert_eq!(kind.display_name(), "Microsoft");
    assert_eq!(kind.authorization(), AuthorizationMethod::Oauth);
    assert_eq!(
        kind.supported_platforms(),
        Platform::MACOS | Platform::IOS | Platform::LINUX | Platform::WINDOWS
    );
}

#[test]
fn identity_family_is_microsoft_for_both_kinds() {
    let browser: Box<dyn ConnectorKind> = Box::new(MicrosoftConnectorKind::new(
        MicrosoftOAuthConfig::new("cid"),
        Arc::new(FakeTransport::new()),
        TestNow::default().provider(),
        instant_sleeper(),
    ));
    let brokered: Box<dyn ConnectorKind> =
        Box::new(kind(&new_source(), &Arc::new(FakeTransport::new())));
    assert_eq!(
        browser
            .as_identity_family()
            .map(|f| f.identity_family())
            .as_deref(),
        Some("microsoft")
    );
    assert_eq!(
        brokered
            .as_identity_family()
            .map(|f| f.identity_family())
            .as_deref(),
        Some("microsoft")
    );
}
