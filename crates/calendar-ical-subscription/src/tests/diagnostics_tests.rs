//! Port of `DiagnosticsTests.swift`. TestNow starts at 2027-01-15 00:00 Pacific, so the window is Jan 14 to Jan 22.
//! The diagnostics carry counts and fixed tokens only: never the link, never text from the feed.

use std::sync::Arc;

use calendar_core::connection::{ConnectorKind, InMemoryCredentialStore};
use calendar_core::diagnostic_log::{
    DiagnosticEvent, DiagnosticFieldValue, DiagnosticLevel, DiagnosticLog, RingBufferDiagnosticLog,
};
use calendar_core::failure::Failure;
use calendar_core::http::{HttpResponse, HttpTransport};
use calendar_core::source_types::{CalendarSource, PollingCalendarSource, SourceError};
use calendar_core::time::DateInterval;
use calendar_test_support::collecting_diagnostic_log::CollectingDiagnosticLog;
use calendar_test_support::fake_transport::{FakeTransport, TestNow};

use super::fixtures::{
    PRIVATE_PATH, StubInteraction, ThrowingTransport, board_games, feed, feed_url, kind, ok,
    ok_with, pacific, pt, redirect, sample_feed, september, source_settings, status, vevent,
};
use crate::feed_fetcher::FeedFetcher;
use crate::retention_window::RetentionWindow;
use crate::source::ICalSubscriptionSource;

fn window() -> RetentionWindow {
    RetentionWindow::new(1, 7)
}

fn everything() -> DateInterval {
    DateInterval::new(pt(2026, 8, 1, 0, 0), pt(2027, 12, 1, 0, 0))
}

fn make_source(
    transport: Arc<dyn HttpTransport>,
    log: &Arc<CollectingDiagnosticLog>,
    now: &TestNow,
    retention: Option<RetentionWindow>,
) -> Arc<ICalSubscriptionSource> {
    let mut settings = source_settings(transport, now.provider(), pacific());
    settings.retention = retention;
    settings.diagnostics = log.clone();
    ICalSubscriptionSource::new(settings)
}

fn ics(events: &[Vec<String>], etag: Option<&str>) -> HttpResponse {
    let text = feed(events);
    match etag {
        Some(etag) => ok_with(&text, &[("ETag", etag)]),
        None => ok(&text),
    }
}

fn serve(responses: Vec<HttpResponse>) -> Arc<FakeTransport> {
    let transport = FakeTransport::new();
    transport.route(PRIVATE_PATH, responses);
    Arc::new(transport)
}

fn one_off(title: &str, start: &str, end: &str) -> Vec<String> {
    vevent(
        Some(&format!("{}@example.test", title.replace(' ', "-"))),
        title,
        start,
        end,
        &[],
    )
}

fn int(event: Option<&DiagnosticEvent>, name: &str) -> Option<i64> {
    match event?.field(name)?.value {
        DiagnosticFieldValue::Int(value) => Some(value),
        _ => None,
    }
}

fn boolean(event: Option<&DiagnosticEvent>, name: &str) -> Option<bool> {
    match event?.field(name)?.value {
        DiagnosticFieldValue::Bool(value) => Some(value),
        _ => None,
    }
}

fn string(event: Option<&DiagnosticEvent>, name: &str) -> Option<String> {
    match &event?.field(name)?.value {
        DiagnosticFieldValue::String(value) => Some(value.clone()),
        _ => None,
    }
}

fn far_past() -> Vec<String> {
    one_off("Far past", "20260901T100000", "20260901T110000")
}
fn far_future() -> Vec<String> {
    one_off("Far future", "20270301T100000", "20270301T110000")
}
fn in_window() -> Vec<String> {
    one_off("In window", "20270118T100000", "20270118T110000")
}

fn new_log() -> Arc<CollectingDiagnosticLog> {
    Arc::new(CollectingDiagnosticLog::new())
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_normal_fetch_reports_the_fetch_and_the_parse() {
    let log = new_log();
    let source = make_source(
        serve(vec![ok(&sample_feed())]),
        &log,
        &TestNow::default(),
        None,
    );
    source.events_in(september()).await.unwrap();
    let fetched = log
        .events_named("fetchCompleted")
        .into_iter()
        .next()
        .unwrap();
    assert!(fetched.level == DiagnosticLevel::Info && fetched.category == "icalsub");
    assert!(
        int(Some(&fetched), "status") == Some(200)
            && boolean(Some(&fetched), "conditional") == Some(false)
            && boolean(Some(&fetched), "notModified") == Some(false)
    );
    assert!(
        int(Some(&fetched), "hops") == Some(0)
            && int(Some(&fetched), "bytes") == Some(sample_feed().len() as i64)
            && int(Some(&fetched), "ms").is_some()
    );
    let parsed = log.events_named("feedParsed").into_iter().next().unwrap();
    assert_eq!(parsed.level, DiagnosticLevel::Info);
    assert!(
        int(Some(&parsed), "eventsInFeed") == Some(2)
            && int(Some(&parsed), "groupsKept") == Some(2)
            && int(Some(&parsed), "groupsDropped") == Some(0)
    );
    assert_eq!(boolean(Some(&parsed), "retentionActive"), Some(false));
    assert!(
        log.events_named("syntheticUID").is_empty() && log.events_named("fetchFailed").is_empty()
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_redirect_counts_as_a_hop() {
    let transport = FakeTransport::new();
    transport.route(
        "example.test/events",
        vec![redirect(
            302,
            &format!("https://cdn.example.test/{PRIVATE_PATH}/x"),
        )],
    );
    transport.route("cdn.example.test", vec![ics(&[board_games()], None)]);
    let log = new_log();
    make_source(Arc::new(transport), &log, &TestNow::default(), None)
        .events_in(september())
        .await
        .unwrap();
    assert_eq!(
        int(log.events_named("fetchCompleted").first(), "hops"),
        Some(1)
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_not_modified_answer_is_reported() {
    let log = new_log();
    let source = make_source(
        serve(vec![ics(&[board_games()], Some("\"v1\"")), status(304)]),
        &log,
        &TestNow::default(),
        None,
    );
    source.check_for_changes().await.unwrap();
    source.check_for_changes().await.unwrap();
    let second = log
        .events_named("fetchCompleted")
        .into_iter()
        .last()
        .unwrap();
    assert!(
        int(Some(&second), "status") == Some(304)
            && boolean(Some(&second), "conditional") == Some(true)
            && boolean(Some(&second), "notModified") == Some(true)
    );
    assert_eq!(int(Some(&second), "bytes"), Some(0));
    assert_eq!(log.events_named("feedParsed").len(), 2);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn failures_are_reported_with_a_fixed_reason_and_the_status() {
    for (code, reason) in [
        (404, "authExpired"),
        (500, "server"),
        (418, "invalidResponse"),
    ] {
        let log = new_log();
        let source = make_source(serve(vec![status(code)]), &log, &TestNow::default(), None);
        assert!(source.events_in(september()).await.is_err());
        let failed = log
            .events_named("fetchFailed")
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("{code}"));
        assert!(failed.level == DiagnosticLevel::Warning);
        assert_eq!(string(Some(&failed), "reason").as_deref(), Some(reason));
        assert_eq!(int(Some(&failed), "status"), Some(code));
        assert!(log.events_named("fetchCompleted").is_empty());
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn transport_failures_and_refused_redirects_have_no_status_or_leaked_text() {
    let log = new_log();
    let link = feed_url().as_str().to_string();
    let source = make_source(
        Arc::new(ThrowingTransport {
            failure: Failure::Source(SourceError::Network(format!("failed: {link}"))),
        }),
        &log,
        &TestNow::default(),
        None,
    );
    assert!(source.events_in(september()).await.is_err());
    let failed = log.events_named("fetchFailed").into_iter().next().unwrap();
    assert_eq!(string(Some(&failed), "reason").as_deref(), Some("network"));
    assert!(failed.field("status").is_none());

    let refused = new_log();
    let refusing = make_source(
        serve(vec![redirect(302, "http://example.test/x")]),
        &refused,
        &TestNow::default(),
        None,
    );
    assert!(refusing.events_in(september()).await.is_err());
    assert_eq!(
        string(refused.events_named("fetchFailed").first(), "reason").as_deref(),
        Some("redirectRefused")
    );

    let looped = new_log();
    let looping = make_source(
        serve(vec![redirect(302, &link)]),
        &looped,
        &TestNow::default(),
        None,
    );
    assert!(looping.events_in(september()).await.is_err());
    assert_eq!(
        string(looped.events_named("fetchFailed").first(), "reason").as_deref(),
        Some("tooManyRedirects")
    );

    let big = new_log();
    let body = HttpResponse::new(200, Default::default(), vec![0; FeedFetcher::MAX_BYTES + 1]);
    let large = make_source(serve(vec![body]), &big, &TestNow::default(), None);
    assert!(large.events_in(september()).await.is_err());
    assert_eq!(
        string(big.events_named("fetchFailed").first(), "reason").as_deref(),
        Some("tooLarge")
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_page_that_is_not_a_calendar_is_reported() {
    let log = new_log();
    let source = make_source(
        serve(vec![ok("<html>login</html>")]),
        &log,
        &TestNow::default(),
        None,
    );
    assert!(source.events_in(september()).await.is_err());
    assert_eq!(log.events_named("fetchCompleted").len(), 1);
    let event = log
        .events_named("feedNotCalendar")
        .into_iter()
        .next()
        .unwrap();
    assert!(event.level == DiagnosticLevel::Warning && event.fields.is_empty());
    assert!(log.events_named("feedParsed").is_empty());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_unreadable_rule_is_kept_whatever_the_window_and_reported() {
    let broken = vevent(
        Some("broken@example.test"),
        "Odd series",
        "20260901T100000",
        "20260901T110000",
        &["RRULE:INTERVAL=2"],
    );
    let log = new_log();
    let source = make_source(
        serve(vec![ics(&[broken, far_past(), in_window()], None)]),
        &log,
        &TestNow::default(),
        Some(window()),
    );
    let titles: std::collections::BTreeSet<String> = source
        .events_in(everything())
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.title)
        .collect();
    assert_eq!(
        titles,
        ["Odd series", "In window"]
            .iter()
            .map(|t| t.to_string())
            .collect()
    );
    let reported = log.events_named("rruleUnreadable");
    let event = reported.first().unwrap();
    assert!(event.level == DiagnosticLevel::Notice && reported.len() == 1);
    let uid = event.field("uid").unwrap();
    assert!(
        uid.is_private && uid.value == DiagnosticFieldValue::String("broken@example.test".into())
    );
    let parsed = log.events_named("feedParsed").into_iter().next().unwrap();
    assert!(
        int(Some(&parsed), "groupsKept") == Some(2)
            && int(Some(&parsed), "groupsDropped") == Some(1)
    );
}

fn broken(rule: &str) -> Vec<String> {
    vevent(
        Some("broken@example.test"),
        "Odd series",
        "20260901T100000",
        "20260901T110000",
        &[&format!("RRULE:{rule}")],
    )
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_unreadable_rule_is_reported_once_per_poll_run_and_again_when_edited_or_re_broken() {
    let log = new_log();
    let fixed = vevent(
        Some("broken@example.test"),
        "Odd series",
        "20260901T100000",
        "20260901T110000",
        &["RRULE:FREQ=WEEKLY"],
    );
    let now = TestNow::default();
    let responses = vec![
        ics(&[broken("INTERVAL=2")], Some("\"v1\"")),
        status(304),
        ics(&[broken("INTERVAL=2")], Some("\"v2\"")),
        ics(&[broken("INTERVAL=3")], None),
        ics(&[fixed], None),
        ics(&[broken("INTERVAL=3")], None),
    ];
    let count = responses.len();
    let source = make_source(serve(responses), &log, &now, Some(window()));
    let mut counts = Vec::new();
    for _ in 0..count {
        source.check_for_changes().await.unwrap();
        counts.push(log.events_named("rruleUnreadable").len());
    }
    // first sight, 304 re-parse, same rule again, edited rule, fixed, broken again
    assert_eq!(counts, [1, 1, 1, 2, 2, 3]);
    let parsed = log.events_named("feedParsed");
    assert_eq!(int(parsed.first(), "unreadableRules"), Some(1));
    assert_eq!(int(parsed.get(4), "unreadableRules"), Some(0));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn events_without_a_uid_are_counted() {
    let log = new_log();
    let nameless = vevent(
        None,
        "Nameless one",
        "20260910T100000",
        "20260910T110000",
        &[],
    );
    let other = vevent(
        None,
        "Nameless two",
        "20260911T100000",
        "20260911T110000",
        &[],
    );
    let source = make_source(
        serve(vec![ics(&[nameless, other, board_games()], None)]),
        &log,
        &TestNow::default(),
        None,
    );
    source.events_in(september()).await.unwrap();
    let event = log.events_named("syntheticUID").into_iter().next().unwrap();
    assert!(event.level == DiagnosticLevel::Info && int(Some(&event), "count") == Some(2));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_retention_window_drops_are_counted() {
    let log = new_log();
    let source = make_source(
        serve(vec![ics(&[far_past(), far_future(), in_window()], None)]),
        &log,
        &TestNow::default(),
        Some(window()),
    );
    source.events_in(everything()).await.unwrap();
    let parsed = log.events_named("feedParsed").into_iter().next().unwrap();
    assert!(
        int(Some(&parsed), "eventsInFeed") == Some(3)
            && int(Some(&parsed), "groupsKept") == Some(1)
            && int(Some(&parsed), "groupsDropped") == Some(2)
    );
    assert_eq!(boolean(Some(&parsed), "retentionActive"), Some(true));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_change_is_reported_and_an_outside_edit_is_suppressed() {
    let edited_far = one_off("Far future renamed", "20270301T100000", "20270301T110000");
    let edited_in = one_off("In window renamed", "20270118T100000", "20270118T110000");
    let log = new_log();
    let source = make_source(
        serve(vec![
            ics(&[far_future(), in_window()], None),
            ics(&[edited_far.clone(), in_window()], None),
            ics(&[edited_far, edited_in], None),
        ]),
        &log,
        &TestNow::default(),
        Some(window()),
    );
    source.check_for_changes().await.unwrap();
    assert!(
        log.events_named("changeReported").is_empty()
            && log.events_named("changeSuppressed").is_empty()
    );
    source.check_for_changes().await.unwrap();
    let suppressed = log
        .events_named("changeSuppressed")
        .into_iter()
        .next()
        .unwrap();
    assert!(suppressed.level == DiagnosticLevel::Info);
    assert_eq!(
        string(Some(&suppressed), "reason").as_deref(),
        Some("unchangedRetained")
    );
    assert!(log.events_named("changeReported").is_empty());
    source.check_for_changes().await.unwrap();
    assert_eq!(
        log.events_named("changeReported").first().map(|e| e.level),
        Some(DiagnosticLevel::Info)
    );
    assert_eq!(log.events_named("changeSuppressed").len(), 1);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn sign_in_is_reported_with_a_reason_only() {
    let accepted = new_log();
    let sign_in_kind =
        kind(serve(vec![ics(&[board_games()], None)])).with_diagnostics(accepted.clone());
    sign_in_kind
        .authorize(
            &StubInteraction::link(feed_url().as_str()),
            &InMemoryCredentialStore::new(),
        )
        .await
        .unwrap();
    let accepted_event = accepted
        .events_named("signInAccepted")
        .into_iter()
        .next()
        .unwrap();
    assert!(accepted_event.level == DiagnosticLevel::Info);
    assert!(
        accepted_event
            .fields
            .iter()
            .all(|field| field.name == "reason")
    );

    for (name, link, code, reason) in [
        (
            "revoked",
            feed_url().as_str().to_string(),
            404,
            "authExpired",
        ),
        ("bad link", "nonsense".to_string(), 200, "invalidLink"),
    ] {
        let log = new_log();
        let rejecting = kind(serve(vec![status(code)])).with_diagnostics(log.clone());
        let outcome = rejecting
            .authorize(
                &StubInteraction::link(&link),
                &InMemoryCredentialStore::new(),
            )
            .await;
        assert!(outcome.is_err(), "{name}");
        let rejected = log
            .events_named("signInRejected")
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("{name}"));
        assert!(rejected.level == DiagnosticLevel::Warning);
        assert_eq!(string(Some(&rejected), "reason").as_deref(), Some(reason));
        assert_eq!(rejected.fields.len(), 1);
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn no_diagnostic_ever_contains_the_link_or_an_event_text() {
    let log = new_log();
    let titles = [
        "Board games night",
        "Weekly walk",
        "Far past",
        "Far future",
        "In window",
        "Odd series",
        "Nameless one",
    ];
    let broken = vevent(
        Some("broken@example.test"),
        "Odd series",
        "20260901T100000",
        "20260901T110000",
        &["RRULE:INTERVAL=2"],
    );
    let nameless = vevent(
        None,
        "Nameless one",
        "20260910T100000",
        "20260910T110000",
        &[],
    );
    let link = feed_url().as_str().to_string();
    let feeds: Vec<Vec<HttpResponse>> = vec![
        vec![ok(&sample_feed())],
        vec![ics(
            &[broken, nameless, far_past(), far_future(), in_window()],
            None,
        )],
        vec![HttpResponse::new(
            404,
            Default::default(),
            link.clone().into_bytes(),
        )],
        vec![ok(&format!("<html>{link}</html>"))],
    ];
    for responses in feeds {
        let source = make_source(serve(responses), &log, &TestNow::default(), Some(window()));
        let _ = source.events_in(everything()).await;
        let _ = source.check_for_changes().await;
    }
    let sign_in = kind(serve(vec![ics(&[board_games()], None)])).with_diagnostics(log.clone());
    sign_in
        .authorize(
            &StubInteraction::link(&link),
            &InMemoryCredentialStore::new(),
        )
        .await
        .unwrap();
    assert!(log.events().len() > 10);

    let ring = RingBufferDiagnosticLog::with_default_capacity();
    for event in log.events() {
        ring.log(event);
    }
    let everything_said = format!(
        "{}\n{}\n{:?}",
        log.transcript(),
        ring.render(true),
        log.events()
    );
    let mut secrets = vec![
        PRIVATE_PATH.to_string(),
        link.clone(),
        "www.example.test/events".to_string(),
        "/events/ical".to_string(),
        "ical/42".to_string(),
    ];
    secrets.extend(titles.iter().map(|t| t.to_string()));
    for secret in &secrets {
        assert!(
            !everything_said.contains(secret.as_str()),
            "leaked {secret}"
        );
    }
    for event in log.events() {
        for field in &event.fields {
            if matches!(field.value, DiagnosticFieldValue::String(_)) {
                assert!(
                    field.is_private || field.name == "reason",
                    "{}.{}",
                    event.name,
                    field.name
                );
            }
        }
    }
}
