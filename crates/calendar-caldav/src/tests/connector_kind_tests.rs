//! Port of `ConnectorKindTests.swift`.

use std::collections::BTreeMap;
use std::sync::Arc;

use calendar_core::connection::{
    AuthorizationMethod, ConnectorKind, CredentialStore, InMemoryCredentialStore,
    InMemorySyncStateStore, Platform,
};
use calendar_core::failure::Failure;
use calendar_core::source_types::SourceError;

use super::support::{
    FakeCalDavServer, StubInteraction, fake_home_url, harness_default_zone, instant_sleeper,
};
use crate::account_config::CalDavAccountConfig;
use crate::connector_kinds::{CalDavAccountSetup, CalDavConnectorKind, ICloudConnectorKind};

const GOOD: [(&str, &str); 2] = [
    ("username", "  me@icloud.test "),
    ("password", "app-pass-1234"),
];

fn now() -> calendar_core::time::Now {
    Arc::new(|| jiff::Timestamp::UNIX_EPOCH)
}

fn icloud(server: &Arc<FakeCalDavServer>) -> ICloudConnectorKind {
    ICloudConnectorKind::new(
        server.clone(),
        now(),
        instant_sleeper(),
        harness_default_zone(),
    )
}

fn caldav(server: &Arc<FakeCalDavServer>) -> CalDavConnectorKind {
    CalDavConnectorKind::new(
        server.clone(),
        now(),
        instant_sleeper(),
        harness_default_zone(),
    )
}

fn secrets(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn is_source_error(result: &Result<impl Sized, Failure>) -> bool {
    matches!(result, Err(Failure::Source(_)))
}

#[test]
fn icloud_kind_describes_itself() {
    let kind = icloud(&Arc::new(FakeCalDavServer::new()));
    assert!(kind.id() == "icloud" && kind.display_name() == "iCloud");
    let AuthorizationMethod::Password { fields } = kind.authorization() else {
        panic!("not a password kind");
    };
    assert_eq!(
        fields.iter().map(|f| f.key.as_str()).collect::<Vec<_>>(),
        ["username", "password"]
    );
    assert_eq!(
        fields.iter().map(|f| f.is_secret).collect::<Vec<_>>(),
        [false, true]
    );
    assert_eq!(
        fields.iter().map(|f| f.label.as_str()).collect::<Vec<_>>(),
        ["Apple ID", "App-specific password"]
    );
    use calendar_core::connection::CredentialPromptHelp;
    assert!(kind.credential_help().unwrap().url.is_some());
    assert!(kind.supported_platforms().contains(Platform::LINUX));
}

#[tokio::test]
async fn icloud_sign_in_stores_config_and_secrets() {
    let server = Arc::new(FakeCalDavServer::new());
    let store = InMemoryCredentialStore::new();
    let connection = icloud(&server)
        .authorize(&StubInteraction::one(&GOOD), &store)
        .await
        .unwrap();
    assert!(connection.kind_id == "icloud" && connection.display_name == "me@icloud.test");
    let account = CalDavAccountConfig::from_config(&connection.config).unwrap();
    assert!(
        account.username == "me@icloud.test"
            && account.home_url == fake_home_url()
            && account.auto_schedule
    );
    assert!(!connection.config.values().any(|v| v.contains("app-pass")));
    assert_eq!(
        store.secrets(&connection.connection_id).await.unwrap(),
        Some(secrets(&[
            ("username", "me@icloud.test"),
            ("password", "app-pass-1234")
        ]))
    );
}

#[tokio::test]
async fn failed_sign_in_stores_nothing() {
    let store = InMemoryCredentialStore::new();
    let result = icloud(&Arc::new(FakeCalDavServer::new()))
        .authorize(
            &StubInteraction::one(&[("username", "me@icloud.test"), ("password", "wrong")]),
            &store,
        )
        .await;
    assert_eq!(
        result.unwrap_err(),
        Failure::Source(SourceError::AuthExpired)
    );
    assert!(store.is_empty());
}

#[tokio::test]
async fn caldav_kind_validates_the_server_address() {
    let text =
        |input: &str| CalDavAccountSetup::server_url_from(input).map(|u| u.as_str().to_string());
    assert_eq!(
        text(" caldav.example.test/dav ").unwrap(),
        "https://caldav.example.test/dav"
    );
    assert_eq!(
        text("http://localhost:8008").unwrap(),
        "http://localhost:8008"
    );
    assert!(is_source_error(&text("http://caldav.example.test")));
    assert!(is_source_error(&text("https://")));
    assert!(is_source_error(&text(
        "https://me:secret@caldav.example.test/dav/"
    )));
    let server = Arc::new(FakeCalDavServer::new());
    let interaction = StubInteraction::one(&[
        ("serverURL", "http://caldav.example.test"),
        ("username", "me"),
        ("password", "p"),
    ]);
    let result = caldav(&server)
        .authorize(&interaction, &InMemoryCredentialStore::new())
        .await;
    assert!(is_source_error(&result));
    assert!(server.log().is_empty());
}

#[tokio::test]
async fn caldav_kind_signs_in_to_the_entered_server() {
    let server = Arc::new(FakeCalDavServer::new());
    server.configure(|s| {
        s.username = "me".to_string();
        s.password = "p".to_string();
    });
    let kind = caldav(&server);
    let AuthorizationMethod::Password { fields } = kind.authorization() else {
        panic!("not a password kind");
    };
    assert_eq!(
        fields.iter().map(|f| f.key.as_str()).collect::<Vec<_>>(),
        ["serverURL", "username", "password"]
    );
    let connection = kind
        .authorize(
            &StubInteraction::one(&[
                ("serverURL", "https://caldav.example.test"),
                ("username", "me"),
                ("password", "p"),
            ]),
            &InMemoryCredentialStore::new(),
        )
        .await
        .unwrap();
    assert!(connection.kind_id == "caldav" && connection.display_name == "me@caldav.example.test");
    assert_eq!(
        CalDavAccountConfig::from_config(&connection.config)
            .unwrap()
            .home_url
            .host()
            .as_deref(),
        Some("caldav.example.test")
    );
}

#[tokio::test]
async fn reauthorize_keeps_the_connection_and_refuses_another_account() {
    let server = Arc::new(FakeCalDavServer::new());
    let store = InMemoryCredentialStore::new();
    let kind = icloud(&server);
    let connection = kind
        .authorize(&StubInteraction::one(&GOOD), &store)
        .await
        .unwrap();
    server.configure(|s| s.password = "new-pass".to_string());
    let again = kind
        .reauthorize(
            &connection,
            &StubInteraction::one(&[("username", "me@icloud.test"), ("password", "new-pass")]),
            &store,
        )
        .await
        .unwrap();
    assert_eq!(again.connection_id, connection.connection_id);
    assert_eq!(
        store
            .secrets(&connection.connection_id)
            .await
            .unwrap()
            .unwrap()["password"],
        "new-pass"
    );

    let mut other = connection.clone();
    other.config.insert(
        "principalURL".to_string(),
        "https://caldav.icloud.com/999/principal/".to_string(),
    );
    let error = kind
        .reauthorize(
            &other,
            &StubInteraction::one(&[("username", "me@icloud.test"), ("password", "new-pass")]),
            &store,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error,
        Failure::Source(SourceError::InvalidResponse(
            "signed in as a different account".to_string()
        ))
    );
}

#[tokio::test]
async fn make_source_reads_secrets_from_the_store() {
    let server = Arc::new(FakeCalDavServer::new());
    let store = Arc::new(InMemoryCredentialStore::new());
    let kind = icloud(&server);
    let connection = kind
        .authorize(&StubInteraction::one(&GOOD), store.as_ref())
        .await
        .unwrap();
    let source = kind
        .make_source(
            &connection,
            store.clone(),
            Arc::new(InMemorySyncStateStore::new()),
        )
        .unwrap();
    assert_eq!(source.id(), format!("icloud-{}", connection.connection_id));
    // The Swift `source is CalDAVCalendarSource`: it polls, writes and returns series.
    assert!(
        source.as_polling().is_some()
            && source.as_writable().is_some()
            && source.as_series().is_some()
    );
    let ids: Vec<String> = source
        .calendars()
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(ids, ["home"]);
    store
        .remove_secrets(&connection.connection_id)
        .await
        .unwrap();
    assert_eq!(
        source.calendars().await.unwrap_err(),
        Failure::Source(SourceError::AuthExpired)
    );
    let mut broken = connection.clone();
    broken.config = BTreeMap::new();
    assert!(is_source_error(&kind.make_source(
        &broken,
        store,
        Arc::new(InMemorySyncStateStore::new())
    )));
}
