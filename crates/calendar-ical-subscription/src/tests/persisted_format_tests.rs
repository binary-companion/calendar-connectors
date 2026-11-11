//! Port of `PersistedFormatTests.swift` (ICalSubscriptionTests): the iCal-link connection of
//! `Tests/PersistedFormats`, as 1.0.7 wrote it.

use std::path::Path;
use std::sync::Arc;

use calendar_core::connection::{
    Connection, ConnectorKind, InMemoryCredentialStore, InMemorySyncStateStore,
};
use calendar_test_support::fake_transport::FakeTransport;

use super::fixtures::kind;

const PERSISTED_FORMATS: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../Tests/PersistedFormats");

fn persisted(version: &str, name: &str) -> Vec<u8> {
    std::fs::read(Path::new(PERSISTED_FORMATS).join(version).join(name)).unwrap()
}

#[test]
fn the_ical_link_connection_of_107_still_opens() {
    let connections: Vec<Connection> =
        serde_json::from_slice(&persisted("1.0.7", "connections.json")).unwrap();
    let connection = connections
        .into_iter()
        .find(|connection| connection.kind_id == "icalsub")
        .unwrap();
    assert_eq!(
        connection.config.get("host").map(String::as_str),
        Some("calendar.example.org")
    );
    let source = kind(Arc::new(FakeTransport::new()))
        .make_source(
            &connection,
            Arc::new(InMemoryCredentialStore::new()),
            Arc::new(InMemorySyncStateStore::new()),
        )
        .unwrap();
    assert_eq!(source.id(), "icalsub-C0FFEE00-1234-4567-89AB-CDEF01234567");
}
