//! Port of `LeakTests.swift`. The feed link is a credential (ADR 0018): no error from sign-in, reads or checks may
//! contain it.

use std::sync::Arc;

use calendar_core::connection::ConnectorKind;
use calendar_core::connection::InMemoryCredentialStore;
use calendar_core::failure::Failure;
use calendar_core::http::HttpTransport;
use calendar_core::source_types::{CalendarSource, PollingCalendarSource, SourceError};
use calendar_test_support::fake_transport::{FakeTransport, TestNow};

use super::fixtures::{
    PRIVATE_PATH, StubInteraction, ThrowingTransport, feed_url, fixed_link, kind, pacific,
    september, source_settings,
};
use crate::feed_fetcher::FeedFetcher;
use crate::source::ICalSubscriptionSource;

fn leaks(error: &Failure) -> bool {
    let text = format!("{error} {error:?}");
    text.contains(PRIVATE_PATH) || text.contains("example.test/events")
}

fn answering(response: calendar_core::http::HttpResponse) -> Arc<dyn HttpTransport> {
    let transport = FakeTransport::new();
    transport.route(PRIVATE_PATH, vec![response]);
    Arc::new(transport)
}

fn with_body(code: i64, body: String) -> calendar_core::http::HttpResponse {
    calendar_core::http::HttpResponse::new(code, Default::default(), body.into_bytes())
}

fn throwing(failure: Failure) -> Arc<dyn HttpTransport> {
    Arc::new(ThrowingTransport { failure })
}

/// Swift's `transports()`. The `URLError` that carried the link in Swift is a `Failure::Host` here: the text of a
/// host exception (conventions section 10, item 12).
fn transports() -> Vec<(&'static str, Arc<dyn HttpTransport>)> {
    let link = feed_url().as_str().to_string();
    vec![
        (
            "network message with the link",
            throwing(Failure::Source(SourceError::Network(format!(
                "failed: {link}"
            )))),
        ),
        (
            "network message with the path",
            throwing(Failure::Source(SourceError::Network(format!(
                "no route to /events/ical/42/{PRIVATE_PATH}/going"
            )))),
        ),
        (
            "a host failure that carries the link",
            throwing(Failure::Host(format!("could not reach {link}"))),
        ),
        (
            "404",
            answering(with_body(404, format!("not found {link}"))),
        ),
        ("500", answering(with_body(500, format!("boom {link}")))),
        (
            "200 that is not a calendar",
            answering(with_body(200, format!("<html>{link}</html>"))),
        ),
    ]
}

fn source(transport: Arc<dyn HttpTransport>) -> Arc<ICalSubscriptionSource> {
    let now = TestNow::default();
    let mut settings = source_settings(transport, now.provider(), pacific());
    settings.link = fixed_link(feed_url());
    ICalSubscriptionSource::new(settings)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn no_error_from_sign_in_events_or_checks_ever_contains_the_link() {
    for (name, transport) in transports() {
        let mut errors: Vec<Failure> = Vec::new();
        let store = InMemoryCredentialStore::new();
        if let Err(error) = kind(transport.clone())
            .authorize(&StubInteraction::link(feed_url().as_str()), &store)
            .await
        {
            errors.push(error);
        }
        if let Err(error) = source(transport.clone()).events_in(september()).await {
            errors.push(error);
        }
        if let Err(error) = source(transport.clone()).check_for_changes().await {
            errors.push(error);
        }
        assert_eq!(errors.len(), 3, "{name}: every call should fail");
        for error in &errors {
            assert!(!leaks(error), "{name}: {error}");
        }
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_transport_failure_without_the_link_keeps_its_message() {
    let transport = throwing(Failure::Source(SourceError::Network(
        "The Internet connection appears to be offline.".to_string(),
    )));
    let error = FeedFetcher::new(transport)
        .fetch(&feed_url(), None)
        .await
        .expect_err("expected a failure");
    assert_eq!(
        error,
        Failure::Source(SourceError::Network(
            "The Internet connection appears to be offline.".to_string()
        ))
    );
}

/// Added in the Rust port: a `Failure` other than a `SourceError` (a host exception, a write error) can carry the
/// link in its text too; the fetcher replaces all of them.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn any_other_failure_from_the_transport_is_replaced_by_a_fixed_message() {
    let link = feed_url().as_str().to_string();
    for failure in [
        Failure::Host(format!("boom {link}")),
        Failure::Host(String::new()),
    ] {
        let error = FeedFetcher::new(throwing(failure))
            .fetch(&feed_url(), None)
            .await
            .expect_err("expected a failure");
        assert_eq!(
            error,
            Failure::Source(SourceError::Network(
                "the feed could not be reached".to_string()
            ))
        );
    }
}
