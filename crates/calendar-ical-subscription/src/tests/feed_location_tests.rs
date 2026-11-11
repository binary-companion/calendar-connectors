//! Port of `FeedLocationTests.swift`.

use calendar_core::source_types::SourceError;

use crate::feed_location::FeedLocation;

fn url(text: &str) -> Result<String, SourceError> {
    FeedLocation::url_from(text).map(|url| url.as_str().to_string())
}

#[test]
fn webcal_links_become_https() {
    assert_eq!(
        url("webcal://www.example.test/events/ical/42/abc/going").unwrap(),
        "https://www.example.test/events/ical/42/abc/going"
    );
    assert_eq!(
        url("WEBCALS://example.test/a.ics").unwrap(),
        "https://example.test/a.ics"
    );
}

#[test]
fn https_links_are_accepted_with_or_without_an_ics_ending() {
    assert_eq!(
        url("https://example.test/feed.ics").unwrap(),
        "https://example.test/feed.ics"
    );
    assert_eq!(
        url("https://example.test/feed?id=7").unwrap(),
        "https://example.test/feed?id=7"
    );
}

#[test]
fn surrounding_whitespace_is_trimmed() {
    assert_eq!(
        url("  https://example.test/feed.ics \n").unwrap(),
        "https://example.test/feed.ics"
    );
}

#[test]
fn plain_http_is_refused_except_for_loopback() {
    assert!(url("http://example.test/feed.ics").is_err());
    assert_eq!(
        url("http://localhost:8008/feed.ics").unwrap(),
        "http://localhost:8008/feed.ics"
    );
    assert_eq!(
        FeedLocation::url_from("http://127.0.0.1:8008/feed.ics")
            .unwrap()
            .host()
            .as_deref(),
        Some("127.0.0.1")
    );
    assert_eq!(
        url("http://[::1]:8008/feed.ics").unwrap(),
        "http://[::1]:8008/feed.ics"
    );
    assert_eq!(
        url("http://[::1]/feed.ics").unwrap(),
        "http://[::1]/feed.ics"
    );
}

#[test]
fn links_with_embedded_credentials_are_refused() {
    assert!(url("https://me:pw@example.test/feed.ics").is_err());
    assert!(url("https://me@example.test/feed.ics").is_err());
}

#[test]
fn files_paths_and_junk_are_refused() {
    for text in [
        "file:///tmp/a.ics",
        "/tmp/a.ics",
        "a.ics",
        "meetup",
        "",
        "   ",
        "ftp://example.test/a.ics",
        "https://",
    ] {
        assert!(url(text).is_err(), "{text}");
    }
}

#[test]
fn refusal_messages_point_at_subscription_links() {
    assert_eq!(
        url("file:///tmp/a.ics"),
        Err(SourceError::InvalidResponse(
            FeedLocation::INVALID_MESSAGE.to_string()
        ))
    );
}
