//! Port of `NoticeTests.swift`.

use std::collections::BTreeSet;
use std::sync::Arc;

use calendar_core::source_types::{
    CalendarSource, PollingCalendarSource, SourceNotice, SourceNoticeKind,
};
use calendar_test_support::collecting_diagnostic_log::CollectingDiagnosticLog;
use calendar_test_support::fake_transport::{FakeTransport, TestNow};

use super::fixtures::{
    PRIVATE_PATH, board_games, feed, ok, ok_with, pacific, september, source_settings, status,
    vevent, weekly_walk,
};
use crate::feed_parser::{FeedParser, UnreadableRule};
use crate::source::{FeedState, ICalSubscriptionSource};

fn source(
    transport: &Arc<FakeTransport>,
    log: Option<Arc<CollectingDiagnosticLog>>,
) -> Arc<ICalSubscriptionSource> {
    let mut settings = source_settings(transport.clone(), TestNow::default().provider(), pacific());
    if let Some(log) = log {
        settings.diagnostics = log;
    }
    ICalSubscriptionSource::new(settings)
}

fn series(name: &str, rule: &str) -> Vec<String> {
    vevent(
        Some(&format!("{name}@example.test")),
        name,
        "20260901T100000",
        "20260901T110000",
        &[&format!("RRULE:{rule}")],
    )
}

fn serve(responses: Vec<calendar_core::http::HttpResponse>) -> Arc<FakeTransport> {
    let transport = FakeTransport::new();
    transport.route(PRIVATE_PATH, responses);
    Arc::new(transport)
}

fn unreadable(count: i64) -> Vec<SourceNotice> {
    vec![SourceNotice::new(
        SourceNoticeKind::UnreadableRecurrence,
        count,
    )]
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_clean_feed_has_no_notices() {
    let feed_source = source(
        &serve(vec![ok(&feed(&[board_games(), weekly_walk()]))]),
        None,
    );
    assert!(feed_source.notices().await.is_empty()); // nothing loaded yet
    feed_source.events_in(september()).await.unwrap();
    assert!(feed_source.notices().await.is_empty());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn each_unreadable_series_counts_and_the_notice_clears_when_the_feed_is_fixed() {
    let feed_source = source(
        &serve(vec![
            ok(&feed(&[
                series("a", "INTERVAL=2"),
                series("b", "FREQ=BOGUS"),
                board_games(),
            ])),
            ok(&feed(&[
                series("a", "FREQ=WEEKLY"),
                series("b", "FREQ=DAILY"),
                board_games(),
            ])),
        ]),
        None,
    );
    feed_source.check_for_changes().await.unwrap();
    assert_eq!(feed_source.notices().await, unreadable(2));
    feed_source.check_for_changes().await.unwrap();
    assert!(feed_source.notices().await.is_empty());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_notice_survives_a_not_modified_re_parse_and_the_log_dedupe() {
    let feed_source = source(
        &serve(vec![
            ok_with(&feed(&[series("a", "INTERVAL=2")]), &[("ETag", "\"v1\"")]),
            status(304),
            status(304),
        ]),
        None,
    );
    for _ in 0..3 {
        feed_source.check_for_changes().await.unwrap();
        assert_eq!(feed_source.notices().await, unreadable(1));
    }
}

#[test]
fn an_older_fetch_finishing_late_does_not_move_the_reported_set_back() {
    let state = FeedState::new();
    let rule = UnreadableRule {
        uid: "a".to_string(),
        is_synthetic: false,
        rule_hash: "1".to_string(),
    };
    assert_eq!(
        state.new_unreadable(std::slice::from_ref(&rule), 2),
        BTreeSet::from([rule.clone()])
    );
    assert!(state.new_unreadable(&[], 1).is_empty());
    assert!(state.new_unreadable(&[rule], 3).is_empty());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_made_up_uid_is_never_logged() {
    let log = Arc::new(CollectingDiagnosticLog::new());
    let nameless = vevent(
        None,
        "Secret title",
        "20260901T100000",
        "20260901T110000",
        &["RRULE:INTERVAL=2"],
    );
    source(
        &serve(vec![ok(&feed(std::slice::from_ref(&nameless)))]),
        Some(log.clone()),
    )
    .check_for_changes()
    .await
    .unwrap();
    let events = log.events_named("rruleUnreadable");
    let event = events.first().unwrap();
    assert!(event.field("uid").is_none());
    assert_eq!(
        event.field("syntheticUID").map(|field| field.value.clone()),
        Some(calendar_core::diagnostic_log::DiagnosticFieldValue::Bool(
            true
        ))
    );
    let transcript = log.transcript();
    assert!(!transcript.contains("feed-") && !transcript.contains("Secret title"));
    // The sign-in parse path logs through the parser itself.
    let parser_log = CollectingDiagnosticLog::new();
    FeedParser::parse_logged(
        feed(&[nameless]).as_bytes(),
        None,
        &parser_log,
        jiff::Timestamp::UNIX_EPOCH,
        true,
    )
    .unwrap();
    assert!(
        parser_log
            .events_named("rruleUnreadable")
            .first()
            .unwrap()
            .field("uid")
            .is_none()
    );
}
