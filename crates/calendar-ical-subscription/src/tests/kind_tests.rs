//! Port of `KindTests.swift`. The default-transport test (`theDefaultSessionKeepsNothingOnDisk`) is
//! `n/a: host transport configuration`: the library has no transport, and the settings it pinned are the host's
//! obligation (`docs/host-http-contract.md`).

use std::collections::BTreeMap;
use std::sync::Arc;

use calendar_core::connection::{
    AuthorizationMethod, ConnectorKind, CredentialField, InMemoryCredentialStore,
    InMemorySyncStateStore, Platform,
};
use calendar_core::failure::Failure;
use calendar_core::source_types::SourceError;
use calendar_core::url_string::UrlString;
use calendar_test_support::fake_transport::FakeTransport;

use super::fixtures::{
    PRIVATE_PATH, StubInteraction, board_games, feed_ics, kind, ok, sample_feed, september, status,
};
use calendar_core::connection::Connection;
use calendar_core::connection::CredentialStore;

const WEBCAL: &str = "webcal://www.example.test/events/ical/42/PRIVATE-PATH-0123/going";
const FEED_LINK: &str = "https://www.example.test/events/ical/42/PRIVATE-PATH-0123/going";

fn serving(responses: Vec<calendar_core::http::HttpResponse>) -> Arc<FakeTransport> {
    let transport = FakeTransport::new();
    transport.route(PRIVATE_PATH, responses);
    Arc::new(transport)
}

fn secrets_of(link: &str) -> Option<BTreeMap<String, String>> {
    Some(BTreeMap::from([("link".to_string(), link.to_string())]))
}

#[test]
fn the_kind_describes_itself() {
    let kind = kind(Arc::new(FakeTransport::new()));
    assert!(kind.id() == "icalsub" && kind.display_name() == "iCal link");
    let AuthorizationMethod::Password { fields } = kind.authorization() else {
        panic!("not a password kind");
    };
    assert_eq!(fields, [CredentialField::new("link", "iCal link").secret()]);
    let help = kind
        .as_credential_prompt_help()
        .unwrap()
        .credential_help()
        .unwrap();
    assert!(help.text.contains("Meetup") && help.text.contains("will not update"));
    assert_eq!(
        help.url,
        UrlString::parse("https://www.meetup.com/your-events/")
    );
    assert!(
        kind.supported_platforms().contains(Platform::LINUX)
            && kind.supported_platforms().contains(Platform::MACOS)
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn sign_in_stores_the_link_as_a_secret_and_only_the_host_in_the_config() {
    let transport = serving(vec![ok(&sample_feed())]);
    let store = InMemoryCredentialStore::new();
    let interaction = StubInteraction::link(&format!("  {WEBCAL} "));
    let connection = kind(transport.clone())
        .authorize(&interaction, &store)
        .await
        .unwrap();
    assert!(
        connection.kind_id == "icalsub"
            && connection.display_name == "My Meetups (www.example.test)"
    );
    assert_eq!(
        connection.config,
        BTreeMap::from([("host".to_string(), "www.example.test".to_string())])
    );
    assert_eq!(
        store.secrets(&connection.connection_id).await.unwrap(),
        secrets_of(FEED_LINK)
    );
    assert!(!connection.display_name.contains(PRIVATE_PATH));
    assert!(!connection.config.values().any(|v| v.contains(PRIVATE_PATH)));
    assert_eq!(interaction.shown().len(), 1);
    let request = transport.requests().into_iter().next().unwrap();
    assert_eq!(request.url, UrlString::parse(FEED_LINK).unwrap());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_feed_with_no_name_is_named_by_its_host_and_a_feed_with_no_events_is_valid() {
    let transport = serving(vec![ok(&feed_ics(&[], &[]))]);
    let connection = kind(transport)
        .authorize(
            &StubInteraction::link(WEBCAL),
            &InMemoryCredentialStore::new(),
        )
        .await
        .unwrap();
    assert_eq!(connection.display_name, "www.example.test");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn bad_input_stores_nothing_and_sends_nothing() {
    for text in [
        "http://www.example.test/a.ics",
        "file:///tmp/a.ics",
        "/tmp/a.ics",
        "",
        "https://me:pw@www.example.test/a.ics",
    ] {
        let transport = serving(vec![ok(&sample_feed())]);
        let store = InMemoryCredentialStore::new();
        let error = kind(transport.clone())
            .authorize(&StubInteraction::link(text), &store)
            .await
            .err()
            .unwrap_or_else(|| panic!("expected a failure for {text}"));
        assert!(matches!(error, Failure::Source(_)), "{text}");
        assert!(
            store.is_empty() && transport.requests().is_empty(),
            "{text}"
        );
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_link_that_is_not_a_calendar_stores_nothing() {
    let transport = serving(vec![ok("<html><body>Please sign in</body></html>")]);
    let store = InMemoryCredentialStore::new();
    let error = kind(transport)
        .authorize(&StubInteraction::link(WEBCAL), &store)
        .await
        .err()
        .unwrap();
    assert_eq!(
        error,
        Failure::Source(SourceError::InvalidResponse(
            "that link did not return a calendar".to_string()
        ))
    );
    assert!(store.is_empty());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_revoked_link_at_sign_in_is_auth_expired_and_stores_nothing() {
    let transport = serving(vec![status(404)]);
    let store = InMemoryCredentialStore::new();
    let error = kind(transport)
        .authorize(&StubInteraction::link(WEBCAL), &store)
        .await
        .err()
        .unwrap();
    assert_eq!(error, Failure::Source(SourceError::AuthExpired));
    assert!(store.is_empty());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_cancelled_prompt_stores_nothing() {
    let store = InMemoryCredentialStore::new();
    let error = kind(Arc::new(FakeTransport::new()))
        .authorize(&StubInteraction::cancelled(), &store)
        .await
        .err()
        .unwrap();
    assert_eq!(error, Failure::Cancelled);
    assert!(store.is_empty());
}

async fn first_connection(store: &InMemoryCredentialStore) -> Connection {
    kind(serving(vec![ok(&sample_feed())]))
        .authorize(&StubInteraction::link(WEBCAL), store)
        .await
        .unwrap()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn reauthorize_replaces_the_link_and_keeps_the_connection_id() {
    let store = InMemoryCredentialStore::new();
    let first = first_connection(&store).await;
    let newer = "https://www.example.test/events/ical/42/PRIVATE-PATH-4567/going";
    let transport = FakeTransport::new();
    transport.route("PRIVATE-PATH-4567", vec![ok(&sample_feed())]);
    let updated = kind(Arc::new(transport))
        .reauthorize(&first, &StubInteraction::link(newer), &store)
        .await
        .unwrap();
    assert!(
        updated.connection_id == first.connection_id && updated.display_name == first.display_name
    );
    assert_eq!(
        store.secrets(&first.connection_id).await.unwrap(),
        secrets_of(newer)
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn reauthorize_to_another_host_rebuilds_the_name_for_the_new_host_keeping_the_feed_name() {
    let store = InMemoryCredentialStore::new();
    let first = first_connection(&store).await;
    let transport = FakeTransport::new();
    transport.route(
        "PRIVATE-PATH-4567",
        vec![ok(&feed_ics(&[board_games()], &["X-WR-CALNAME:Renamed"]))],
    );
    let updated = kind(Arc::new(transport))
        .reauthorize(
            &first,
            &StubInteraction::link("https://calendar.example.org/x/PRIVATE-PATH-4567/going"),
            &store,
        )
        .await
        .unwrap();
    assert_eq!(
        updated.config,
        BTreeMap::from([("host".to_string(), "calendar.example.org".to_string())])
    );
    assert_eq!(updated.display_name, "My Meetups (calendar.example.org)");
    assert!(!updated.display_name.contains("PRIVATE-PATH"));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn reauthorize_to_another_host_for_a_nameless_feed_uses_the_new_feed_name_or_the_host() {
    let store = InMemoryCredentialStore::new();
    let nameless = ok(&feed_ics(&[board_games()], &[]));
    let first = kind(serving(vec![nameless.clone()]))
        .authorize(&StubInteraction::link(WEBCAL), &store)
        .await
        .unwrap();
    assert_eq!(first.display_name, "www.example.test");
    let transport = FakeTransport::new();
    transport.route("PRIVATE-PATH-4567", vec![nameless]);
    let updated = kind(Arc::new(transport))
        .reauthorize(
            &first,
            &StubInteraction::link("https://calendar.example.org/x/PRIVATE-PATH-4567/going"),
            &store,
        )
        .await
        .unwrap();
    assert_eq!(updated.display_name, "calendar.example.org");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_failed_reauthorize_keeps_the_old_link() {
    let store = InMemoryCredentialStore::new();
    let first = first_connection(&store).await;
    let transport = FakeTransport::new();
    transport.route("PRIVATE-PATH-4567", vec![status(404)]);
    let error = kind(Arc::new(transport))
        .reauthorize(
            &first,
            &StubInteraction::link(
                "https://www.example.test/events/ical/42/PRIVATE-PATH-4567/going",
            ),
            &store,
        )
        .await
        .err()
        .unwrap();
    assert_eq!(error, Failure::Source(SourceError::AuthExpired));
    assert_eq!(
        store.secrets(&first.connection_id).await.unwrap(),
        secrets_of(FEED_LINK)
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_source_reads_through_the_stored_link() {
    let store = Arc::new(InMemoryCredentialStore::new());
    let transport = serving(vec![ok(&sample_feed())]);
    let kind = kind(transport);
    let connection = kind
        .authorize(&StubInteraction::link(WEBCAL), store.as_ref())
        .await
        .unwrap();
    let source = kind
        .make_source(&connection, store, Arc::new(InMemorySyncStateStore::new()))
        .unwrap();
    assert_eq!(source.id(), format!("icalsub-{}", connection.connection_id));
    assert_eq!(source.events_in(september()).await.unwrap().len(), 6);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_source_with_no_stored_link_needs_signing_in_again() {
    let connection = Connection::new("icalsub", "gone", "x");
    let source = kind(Arc::new(FakeTransport::new()))
        .make_source(
            &connection,
            Arc::new(InMemoryCredentialStore::new()),
            Arc::new(InMemorySyncStateStore::new()),
        )
        .unwrap();
    let error = source.events_in(september()).await.err().unwrap();
    assert_eq!(error, Failure::Source(SourceError::AuthExpired));
}

/// Not a Swift test: the builder methods that replace Swift's initializer defaults.
#[test]
fn the_builder_methods_keep_the_swift_defaults() {
    assert_eq!(kind(Arc::new(FakeTransport::new())).poll_interval(), 900.0);
}
