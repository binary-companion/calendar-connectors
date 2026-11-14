//! Port of `ChangeDetectionTests.swift`.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use calendar_core::connection::{InMemorySyncStateStore, SyncStateStore};
use calendar_core::failure::Failure;
use calendar_core::source_types::{CalendarChange, CalendarSource, SourceError};
use serde_json::json;

use super::support::{Harness, list_json, ok, status};

const ME: &str = "calendars/me%40x.com/events";
const TEAM: &str = "calendars/team%40group.calendar.google.com/events";

fn bootstrap_routes(h: &Harness) {
    bootstrap_routes_with(h, "m1", "t1");
}

fn bootstrap_routes_with(h: &Harness, me_token: &str, team_token: &str) {
    h.transport.route(
        TEAM,
        vec![ok(json!({"items": [], "nextSyncToken": team_token}))],
    );
    h.transport.route(
        ME,
        vec![ok(json!({"items": [], "nextSyncToken": me_token}))],
    );
}

async fn check(h: &Harness) -> Result<Option<CalendarChange>, Failure> {
    h.source.as_polling().unwrap().check_for_changes().await
}

fn events_changed(ids: &[&str]) -> Option<CalendarChange> {
    Some(CalendarChange::EventsChanged {
        calendar_ids: Some(ids.iter().map(|id| id.to_string()).collect::<BTreeSet<_>>()),
    })
}

async fn token(sync: &InMemorySyncStateStore, scope: &str) -> Option<String> {
    sync.token("c1", scope).await
}

#[tokio::test]
async fn first_call_bootstraps_tokens_and_reports_nothing() {
    let h = Harness::new().await;
    bootstrap_routes(&h);
    assert_eq!(check(&h).await.unwrap(), None);
    assert_eq!(token(&h.sync, "me@x.com").await.as_deref(), Some("m1"));
    assert_eq!(
        token(&h.sync, "team@group.calendar.google.com")
            .await
            .as_deref(),
        Some("t1")
    );
    let request = h
        .transport
        .requests_matching(ME)
        .into_iter()
        .next()
        .unwrap();
    let url = request.url.as_str();
    assert!(url.contains("showDeleted=true") && url.contains("maxResults=2500"));
    assert!(!url.contains("timeMin") && !url.contains("syncToken"));
}

#[tokio::test]
async fn a_retry_after_a_partly_failed_first_call_still_reports_real_changes() {
    // A first call bootstrapped calendar "me" then failed before the calendar-set key was written.
    let sync = Arc::new(InMemorySyncStateStore::new());
    sync.set_token(Some("m1"), "c1", "me@x.com").await;
    let h = Harness::with_sync(list_json(&["me@x.com"]), sync).await;
    h.transport.route(
        "syncToken=m1",
        vec![ok(json!({"items": [{"id": "e1"}], "nextSyncToken": "m2"}))],
    );
    assert_eq!(check(&h).await.unwrap(), events_changed(&["me@x.com"]));
}

#[tokio::test]
async fn bootstrap_walks_every_page_and_takes_the_last_token() {
    let h = Harness::new().await;
    h.transport
        .route(TEAM, vec![ok(json!({"items": [], "nextSyncToken": "t1"}))]);
    h.transport
        .route(ME, vec![ok(json!({"nextPageToken": "page2"}))]);
    h.transport.route(
        "pageToken=page2",
        vec![ok(json!({"nextSyncToken": "m-final"}))],
    );
    assert_eq!(check(&h).await.unwrap(), None);
    assert_eq!(token(&h.sync, "me@x.com").await.as_deref(), Some("m-final"));
}

#[tokio::test]
async fn unchanged_poll_reports_nothing_and_advances_the_token() {
    let h = Harness::new().await;
    bootstrap_routes(&h);
    check(&h).await.unwrap();
    h.transport.route(
        "syncToken=t1",
        vec![ok(json!({"items": [], "nextSyncToken": "t2"}))],
    );
    h.transport.route(
        "syncToken=m1",
        vec![ok(json!({"items": [], "nextSyncToken": "m2"}))],
    );
    assert_eq!(check(&h).await.unwrap(), None);
    assert_eq!(token(&h.sync, "me@x.com").await.as_deref(), Some("m2"));
    let poll = h
        .transport
        .requests_matching("syncToken=m1")
        .into_iter()
        .next()
        .unwrap();
    let url = poll.url.as_str();
    assert!(url.contains("showDeleted=true") && url.contains("maxResults=250"));
    assert!(!url.contains("timeMin") && !url.contains("timeMax"));
}

#[tokio::test]
async fn changed_items_report_events_changed_for_that_calendar() {
    let h = Harness::new().await;
    bootstrap_routes(&h);
    check(&h).await.unwrap();
    h.transport.route(
        "syncToken=t1",
        vec![ok(json!({"items": [], "nextSyncToken": "t2"}))],
    );
    h.transport.route(
        "syncToken=m1",
        vec![ok(json!({"items": [{"id": "e9"}], "nextSyncToken": "m2"}))],
    );
    assert_eq!(check(&h).await.unwrap(), events_changed(&["me@x.com"]));
}

#[tokio::test]
async fn poll_pages_until_the_final_token_even_when_the_first_page_has_changes() {
    let h = Harness::new().await;
    bootstrap_routes(&h);
    check(&h).await.unwrap();
    h.transport.route(
        "syncToken=t1",
        vec![ok(json!({"items": [], "nextSyncToken": "t2"}))],
    );
    h.transport.route(
        "syncToken=m1",
        vec![ok(
            json!({"items": [{"id": "e1"}], "nextPageToken": "page2"}),
        )],
    );
    h.transport.route(
        "pageToken=page2",
        vec![ok(json!({"items": [], "nextSyncToken": "m-final"}))],
    );
    assert_eq!(check(&h).await.unwrap(), events_changed(&["me@x.com"]));
    assert_eq!(token(&h.sync, "me@x.com").await.as_deref(), Some("m-final"));
}

#[tokio::test]
async fn expired_token_rebootstraps_and_reports_changed() {
    let h = Harness::new().await;
    bootstrap_routes(&h);
    check(&h).await.unwrap();
    h.transport.route(
        "syncToken=t1",
        vec![ok(json!({"items": [], "nextSyncToken": "t2"}))],
    );
    // The re-bootstrap (no syncToken in its URL).
    h.transport
        .route(ME, vec![ok(json!({"nextSyncToken": "m-new"}))]);
    // Registered last: wins for the poll.
    h.transport.route(
        "syncToken=m1",
        vec![status(json!({"error": {"code": 410}}), 410)],
    );
    assert_eq!(check(&h).await.unwrap(), events_changed(&["me@x.com"]));
    assert_eq!(token(&h.sync, "me@x.com").await.as_deref(), Some("m-new"));
}

#[tokio::test]
async fn an_added_calendar_reports_calendars_changed_and_is_baselined() {
    let sync = Arc::new(InMemorySyncStateStore::new());
    let first = Harness::with_sync(
        list_json(&["me@x.com", "team@group.calendar.google.com"]),
        sync.clone(),
    )
    .await;
    bootstrap_routes(&first);
    check(&first).await.unwrap();

    let second = Harness::with_sync(
        list_json(&[
            "me@x.com",
            "team@group.calendar.google.com",
            "new@group.calendar.google.com",
        ]),
        sync.clone(),
    )
    .await;
    second.transport.route(
        "syncToken=t1",
        vec![ok(json!({"items": [], "nextSyncToken": "t2"}))],
    );
    second.transport.route(
        "syncToken=m1",
        vec![ok(json!({"items": [], "nextSyncToken": "m2"}))],
    );
    second.transport.route(
        "calendars/new%40group.calendar.google.com/events",
        vec![ok(json!({"nextSyncToken": "n1"}))],
    );
    assert_eq!(
        check(&second).await.unwrap(),
        Some(CalendarChange::CalendarsChanged)
    );
    assert_eq!(
        token(&sync, "new@group.calendar.google.com")
            .await
            .as_deref(),
        Some("n1")
    );
    assert!(
        token(&sync, "_calendars")
            .await
            .is_some_and(|set| set.contains("new@group.calendar.google.com"))
    );
}

#[tokio::test]
async fn a_removed_calendar_reports_calendars_changed_and_clears_its_token() {
    let sync = Arc::new(InMemorySyncStateStore::new());
    let first = Harness::with_sync(
        list_json(&["me@x.com", "team@group.calendar.google.com"]),
        sync.clone(),
    )
    .await;
    bootstrap_routes(&first);
    check(&first).await.unwrap();

    let second = Harness::with_sync(list_json(&["me@x.com"]), sync.clone()).await;
    second.transport.route(
        "syncToken=m1",
        vec![ok(json!({"items": [], "nextSyncToken": "m2"}))],
    );
    assert_eq!(
        check(&second).await.unwrap(),
        Some(CalendarChange::CalendarsChanged)
    );
    assert_eq!(token(&sync, "team@group.calendar.google.com").await, None);
}

#[tokio::test]
async fn a_failure_mid_bootstrap_stores_nothing_and_converges_on_retry() {
    let h = Harness::new().await;
    h.transport
        .route(TEAM, vec![ok(json!({"nextSyncToken": "t1"}))]);
    h.transport.route(
        ME,
        vec![status(json!({}), 503), ok(json!({"nextSyncToken": "m1"}))],
    );
    assert_eq!(
        check(&h).await.unwrap_err(),
        Failure::Source(SourceError::Server { status: 503 })
    );
    assert_eq!(token(&h.sync, "_calendars").await, None);
    assert_eq!(check(&h).await.unwrap(), None);
    assert_eq!(token(&h.sync, "me@x.com").await.as_deref(), Some("m1"));
    assert!(token(&h.sync, "_calendars").await.is_some());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn monitor_emits_changes_from_the_source_stream() {
    let h = Harness::new().await;
    bootstrap_routes(&h);
    h.transport.route(
        "syncToken=t1",
        vec![ok(json!({"items": [], "nextSyncToken": "t2"}))],
    );
    h.transport.route(
        "syncToken=m1",
        vec![ok(json!({"items": [{"id": "x"}], "nextSyncToken": "m2"}))],
    );
    // The first check is the baseline (nil); the second finds the change. The harness monitor only yields.
    let mut stream = h.source.changes();
    let first = tokio::time::timeout(Duration::from_secs(5), stream.recv())
        .await
        .expect("timed out waiting for the monitor to emit a change");
    assert_eq!(first, events_changed(&["me@x.com"]));
}

#[tokio::test]
async fn a_poll_item_that_cannot_be_parsed_still_counts_as_a_change_and_advances_the_token() {
    let h = Harness::new().await;
    bootstrap_routes(&h);
    check(&h).await.unwrap();
    h.transport.route(
        "syncToken=t1",
        vec![ok(json!({"items": [], "nextSyncToken": "t2"}))],
    );
    h.transport.route(
        "syncToken=m1",
        vec![ok(
            json!({"items": [{"noId": true}], "nextSyncToken": "m2"}),
        )],
    );
    assert_eq!(check(&h).await.unwrap(), events_changed(&["me@x.com"]));
    assert_eq!(token(&h.sync, "me@x.com").await.as_deref(), Some("m2"));
}
