//! Port of `PersistedFormatTests.swift`: the Microsoft parts of `Tests/PersistedFormats` (the stored delta positions,
//! `"<baseline seconds>|<link>"`, and the calendar-set key, as 1.0.7 wrote them) and the brokered connection of 1.2.0.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use calendar_core::connection::{
    Connection, ConnectorKind, InMemoryCredentialStore, InMemorySyncStateStore,
};
use calendar_core::time::add_seconds;
use calendar_core::url_string::UrlString;
use calendar_test_support::fake_external_token_source::FakeExternalTokenSource;
use calendar_test_support::fake_transport::{FakeTransport, TestNow};
use jiff::Timestamp;
use serde_json::json;

use super::support::{instant_sleeper, ok};
use crate::source::MicrosoftCalendarSource;
use crate::source_sync::DeltaState;
use crate::{MicrosoftBrokeredConnectorKind, MicrosoftConnectorKind, MicrosoftOAuthConfig};

fn persisted(version: &str, name: &str) -> Vec<u8> {
    let root = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../Tests/PersistedFormats"
    ));
    std::fs::read(root.join(version).join(name)).unwrap()
}

#[test]
fn the_microsoft_state_of_107_still_reads() {
    let connections: Vec<Connection> =
        serde_json::from_slice(&persisted("1.0.7", "connections.json")).unwrap();
    let connection = connections
        .into_iter()
        .find(|c| c.kind_id == MicrosoftConnectorKind::KIND_ID)
        .unwrap();
    let kind = MicrosoftConnectorKind::new(
        MicrosoftOAuthConfig::new("client"),
        Arc::new(FakeTransport::new()),
        Arc::new(|| Timestamp::UNIX_EPOCH),
        instant_sleeper(),
    );
    let source = kind
        .make_source(
            &connection,
            Arc::new(InMemoryCredentialStore::new()),
            Arc::new(InMemorySyncStateStore::new()),
        )
        .unwrap();
    assert_eq!(
        source.id(),
        "microsoft-5D2E8F4A-1B3C-4D6E-8F90-A1B2C3D4E5F6"
    );
    let concrete = kind
        .make_microsoft_source(
            &connection,
            Arc::new(InMemoryCredentialStore::new()),
            Arc::new(InMemorySyncStateStore::new()),
        )
        .unwrap();
    assert_eq!(
        concrete.account_email().as_deref(),
        Some("carol@contoso.com")
    );

    let state: BTreeMap<String, BTreeMap<String, String>> =
        serde_json::from_slice(&persisted("1.0.7", "sync-state.json")).unwrap();
    let stored = &state[&connection.connection_id];
    assert_eq!(
        stored
            .get(MicrosoftCalendarSource::CALENDAR_SET_SCOPE)
            .map(String::as_str),
        Some("AAMkAGI2TAAA=\nAAMkAGI2TBBB=")
    );
    assert_eq!(
        DeltaState::parse(&stored["AAMkAGI2TAAA="]),
        Some(DeltaState {
            baseline_date: Timestamp::from_second(1_790_000_000).unwrap(),
            link: UrlString::parse("https://graph.microsoft.com/v1.0/me/calendars/AAMkAGI2TAAA=/calendarView/delta?$deltatoken=R0usmci39OQxqJrxK4").unwrap(),
        })
    );
    assert_eq!(
        DeltaState::parse(&stored["AAMkAGI2TBBB="]),
        Some(DeltaState {
            baseline_date: Timestamp::from_second(1_790_086_400).unwrap(),
            link: UrlString::parse("https://graph.microsoft.com/v1.0/me/calendars/AAMkAGI2TBBB=/calendarView/delta?$skiptoken=X9a2").unwrap(),
        })
    );
}

// Release 1.2.0 added the brokered Microsoft kind: a connection with no stored secret, whose account is the host's.
#[tokio::test]
async fn brokered_connection_from_release_120_decodes() {
    let connections: Vec<Connection> =
        serde_json::from_slice(&persisted("1.2.0", "connections.json")).unwrap();
    assert_eq!(connections.len(), 1);
    let connection = connections.into_iter().next().unwrap();
    assert!(
        connection.kind_id == MicrosoftBrokeredConnectorKind::KIND_ID
            && connection.kind_id == "microsoft-brokered"
    );
    assert_eq!(connection.display_name, "dave@contoso.com");
    assert_eq!(
        connection
            .config
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["authMode", "email", "externalAccountID"]
    );
    assert!(
        connection.config.get("authMode").map(String::as_str) == Some("external")
            && connection.config.get("email").map(String::as_str) == Some("dave@contoso.com")
    );

    // It still opens through its kind, asks the host for tokens with the stored account and needs no credential.
    let now = TestNow::default();
    let source = Arc::new(FakeExternalTokenSource::with_scripts(
        vec![Ok(
            calendar_core::external_tokens::ExternalAccessToken::new(
                "tok",
                add_seconds(now.date(), 3600.0),
            ),
        )],
        Vec::new(),
    ));
    let transport = Arc::new(FakeTransport::new());
    transport.route(
        "me/mailboxSettings/timeZone",
        vec![ok(json!({"value": "UTC"}))],
    );
    transport.route(
        "me/calendars?",
        vec![ok(
            json!({"value": [{"id": "cal1", "name": "Calendar", "canEdit": false}]}),
        )],
    );
    let kind = MicrosoftBrokeredConnectorKind::new(
        source.clone(),
        transport,
        now.provider(),
        instant_sleeper(),
    );
    let store = Arc::new(InMemoryCredentialStore::new());
    let opened = kind
        .make_source(
            &connection,
            store.clone(),
            Arc::new(InMemorySyncStateStore::new()),
        )
        .unwrap();
    assert_eq!(
        opened.id(),
        "microsoft-brokered-7E3A9C21-4B58-4D0F-A6E2-91C8B5D40F37"
    );
    let concrete = kind
        .make_microsoft_source(
            &connection,
            store.clone(),
            Arc::new(InMemorySyncStateStore::new()),
        )
        .unwrap();
    assert_eq!(
        concrete.account_email().as_deref(),
        Some("dave@contoso.com")
    );
    assert_eq!(
        opened
            .calendars()
            .await
            .unwrap()
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        ["cal1"]
    );
    assert_eq!(
        source
            .token_calls()
            .first()
            .map(|call| call.account.id.clone()),
        connection.config.get("externalAccountID").cloned()
    );
    assert!(store.is_empty());
}
