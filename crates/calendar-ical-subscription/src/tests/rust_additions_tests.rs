//! Not ports: behaviour the Rust port pins that Swift's tests do not (each test says why).

use std::sync::Arc;

use calendar_core::connection::{ConnectorKind, InMemoryCredentialStore};
use calendar_core::source_types::CalendarSource;
use calendar_test_support::fake_transport::{FakeTransport, TestNow};

use super::fixtures::{
    PRIVATE_PATH, StubInteraction, feed, feed_url, kind, ok, pacific, sample_feed, september,
    source_settings, vevent,
};
use crate::feed_fetcher::{FeedFetcher, FeedResponse};
use crate::feed_parser::FeedParser;
use crate::retention_window::RetentionWindow;
use crate::source::ICalSubscriptionSource;

/// `RetentionWindow.init` clamps negatives to 0 and `interval(at:)` counts 24-hour days.
#[test]
fn the_retention_window_clamps_and_counts_24_hour_days() {
    assert_eq!(RetentionWindow::new(-3, -1), RetentionWindow::new(0, 0));
    let now = TestNow::default().date();
    let interval = RetentionWindow::new(1, 7).interval_at(now);
    assert_eq!(
        calendar_core::time::seconds_between(interval.start, now),
        86_400.0
    );
    assert_eq!(
        calendar_core::time::seconds_between(now, interval.end),
        7.0 * 86_400.0
    );
}

/// Event ids join the calendar id, the resource name and `#`; a UID with spaces, `#`, `/` and non-ASCII text must
/// still give ASCII-only ids (so Swift's and Rust's string order of ids cannot differ).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn event_ids_are_ascii_whatever_the_uid_holds() {
    let uids = ["é@x.test", "日本語@x.test", "a b#c/d@x.test", "😀@x.test"];
    let events: Vec<Vec<String>> = uids
        .iter()
        .map(|uid| {
            vevent(
                Some(uid),
                "Same time",
                "20260910T100000",
                "20260910T110000",
                &[],
            )
        })
        .collect();
    let transport = FakeTransport::new();
    transport.route(PRIVATE_PATH, vec![ok(&feed(&events))]);
    let mut settings = source_settings(
        Arc::new(transport),
        TestNow::default().provider(),
        pacific(),
    );
    settings.connection.display_name = "x".to_string();
    let found = ICalSubscriptionSource::new(settings)
        .events_in(september())
        .await
        .unwrap();
    assert_eq!(found.len(), 4);
    assert!(found.iter().all(|event| event.id().is_ascii()));
    let ids: Vec<String> = found.iter().map(|event| event.id()).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "events at the same start are ordered by id");
}

/// The feed link is a credential (ADR 0018): nothing the library hands back, printed with `Debug`, contains it.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn debug_output_never_contains_the_link() {
    let transport = FakeTransport::new();
    transport.route(PRIVATE_PATH, vec![ok(&sample_feed())]);
    let transport = Arc::new(transport);
    let response = FeedFetcher::new(transport.clone())
        .fetch(&feed_url(), None)
        .await
        .unwrap();
    let FeedResponse::Body(data, validators) = &response else {
        panic!("expected a body");
    };
    let parsed = FeedParser::parse(data).unwrap();
    let store = InMemoryCredentialStore::new();
    let connection = kind(transport)
        .authorize(&StubInteraction::link(feed_url().as_str()), &store)
        .await
        .unwrap();
    for text in [
        format!("{validators:?}"),
        format!("{parsed:?}"),
        format!("{connection:?}"),
        format!("{:?}", parsed.fingerprint()),
    ] {
        assert!(!text.contains(PRIVATE_PATH) && !text.contains("example.test/events/ical"));
    }
}
