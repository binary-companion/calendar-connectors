//! Port of `PersistedFormatTests.swift`: the Google connection of `Tests/PersistedFormats`, as 1.0.7 wrote it.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use calendar_core::connection::{
    Connection, ConnectorKind, InMemoryCredentialStore, InMemorySyncStateStore,
};
use calendar_test_support::fake_transport::FakeTransport;

use super::support::instant_sleeper;
use crate::{GoogleConnectorKind, GoogleOAuthConfig};

fn persisted(version: &str, name: &str) -> Vec<u8> {
    let root = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../Tests/PersistedFormats"
    ));
    std::fs::read(root.join(version).join(name)).unwrap()
}

#[test]
fn the_google_connection_of_107_still_opens() {
    let connections: Vec<Connection> =
        serde_json::from_slice(&persisted("1.0.7", "connections.json")).unwrap();
    let connection = connections
        .into_iter()
        .find(|c| c.kind_id == GoogleConnectorKind::KIND_ID)
        .unwrap();
    let now: calendar_core::time::Now = Arc::new(|| jiff::Timestamp::UNIX_EPOCH);
    let kind = GoogleConnectorKind::new(
        GoogleOAuthConfig::new("client", "client-secret"),
        Arc::new(FakeTransport::new()),
        now,
        instant_sleeper(),
    );
    let source = kind
        .make_source(
            &connection,
            Arc::new(InMemoryCredentialStore::new()),
            Arc::new(InMemorySyncStateStore::new()),
        )
        .unwrap();
    assert_eq!(source.id(), "google-0B9C1A7E-6A55-4F0B-9E43-5C1F7D2A9B10");
    let stored: BTreeMap<String, BTreeMap<String, String>> =
        serde_json::from_slice(&persisted("1.0.7", "sync-state.json")).unwrap();
    assert_eq!(
        stored[&connection.connection_id]
            .get(crate::source::GoogleCalendarSource::CALENDAR_SET_SCOPE)
            .map(String::as_str),
        Some("alice@example.com\nfamily0123@group.calendar.google.com")
    );
}
