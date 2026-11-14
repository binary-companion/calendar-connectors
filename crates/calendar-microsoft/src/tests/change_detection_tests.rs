//! Port of `ChangeDetectionTests.swift`.

use std::sync::Arc;

use calendar_core::connection::{InMemorySyncStateStore, SyncStateStore};
use calendar_core::http::HttpResponse;
use calendar_core::source_types::{CalendarChange, PollingCalendarSource};
use calendar_core::time::add_seconds;
use calendar_core::url_string::UrlString;
use jiff::Timestamp;
use serde_json::{Map, Value, json};

use super::support::{SourceHarness, calendars_json_ids, graph_error, graph_event, ok};
use crate::graph_time::GraphTime;
use crate::source::MicrosoftCalendarSource;
use crate::source_sync::DeltaState;

const DELTA_BASE: &str = "https://graph.microsoft.com/v1.0/me/calendars/cal1/calendarView/delta";

fn page(items: Vec<Value>, delta: Option<&str>, next: Option<&str>) -> HttpResponse {
    let mut json = Map::new();
    json.insert("value".to_string(), Value::Array(items));
    if let Some(token) = delta {
        json.insert(
            "@odata.deltaLink".to_string(),
            Value::String(format!("{DELTA_BASE}?$deltatoken={token}")),
        );
    }
    if let Some(next) = next {
        json.insert(
            "@odata.nextLink".to_string(),
            Value::String(next.to_string()),
        );
    }
    ok(Value::Object(json))
}

fn delta_page(token: &str) -> HttpResponse {
    page(Vec::new(), Some(token), None)
}

/// The delta position stored for calendar `cal1`.
async fn stored_delta(sync: &InMemorySyncStateStore) -> DeltaState {
    let stored = sync.token("c1", "cal1").await.unwrap();
    DeltaState::parse(&stored).unwrap()
}

async fn one_calendar() -> SourceHarness {
    SourceHarness::with_calendars(calendars_json_ids(&["cal1"])).await
}

fn events_changed(ids: &[&str]) -> Option<CalendarChange> {
    Some(CalendarChange::EventsChanged {
        calendar_ids: Some(ids.iter().map(|id| (*id).to_string()).collect()),
    })
}

#[tokio::test]
async fn the_first_check_takes_a_baseline_and_reports_nothing() {
    let h = one_calendar().await;
    h.transport
        .route("calendarView/delta", vec![delta_page("t0")]);
    assert_eq!(h.source.check_for_changes().await, Ok(None));

    let request = h
        .transport
        .requests_matching("calendarView/delta")
        .into_iter()
        .next()
        .unwrap();
    let url = request.url.as_str();
    assert!(url.contains("startDateTime=") && url.contains("endDateTime="));
    assert!(
        request
            .headers
            .get("Prefer")
            .is_some_and(|prefer| prefer.contains("odata.maxpagesize=200"))
    );
    let state = stored_delta(&h.sync).await;
    assert!(state.baseline_date == h.now.date() && state.link.as_str().ends_with("$deltatoken=t0"));
    assert_eq!(
        h.sync
            .token("c1", MicrosoftCalendarSource::CALENDAR_SET_SCOPE)
            .await
            .as_deref(),
        Some("cal1")
    );
}

#[tokio::test]
async fn the_baseline_window_is_thirty_days_back_and_a_year_ahead() {
    let h = one_calendar().await;
    h.transport
        .route("calendarView/delta", vec![delta_page("t0")]);
    h.source.check_for_changes().await.unwrap();
    let request = h
        .transport
        .requests_matching("calendarView/delta")
        .into_iter()
        .next()
        .unwrap();
    let items = request.url.query_items();
    let value = |name: &str| -> String {
        items
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| v.clone())
            .unwrap()
    };
    assert_eq!(
        value("startDateTime"),
        GraphTime::instant_text(add_seconds(h.now.date(), -30.0 * 86_400.0))
    );
    assert_eq!(
        value("endDateTime"),
        GraphTime::instant_text(add_seconds(h.now.date(), 365.0 * 86_400.0))
    );
}

#[tokio::test]
async fn a_baseline_walks_every_page_to_the_link() {
    let h = one_calendar().await;
    h.transport.route(
        "calendarView/delta",
        vec![
            page(
                vec![graph_event("a", json!({}))],
                None,
                Some(&format!("{DELTA_BASE}?$skiptoken=2")),
            ),
            delta_page("t9"),
        ],
    );
    assert_eq!(h.source.check_for_changes().await, Ok(None));
    let state = stored_delta(&h.sync).await;
    assert!(state.link.as_str().ends_with("$deltatoken=t9"));
}

#[tokio::test]
async fn a_poll_with_nothing_new_reports_nothing_and_keeps_the_baseline_date() {
    let h = one_calendar().await;
    h.transport
        .route("calendarView/delta", vec![delta_page("t0")]);
    h.source.check_for_changes().await.unwrap();
    let baselined = h.now.date();
    h.now.advance(60.0);
    h.transport.route("deltatoken=t0", vec![delta_page("t1")]);
    assert_eq!(h.source.check_for_changes().await, Ok(None));
    let state = stored_delta(&h.sync).await;
    assert!(state.baseline_date == baselined && state.link.as_str().ends_with("$deltatoken=t1"));
}

#[tokio::test]
async fn a_poll_with_changes_reports_the_calendar_including_removals() {
    let h = one_calendar().await;
    h.transport
        .route("calendarView/delta", vec![delta_page("t0")]);
    h.source.check_for_changes().await.unwrap();
    h.transport.route(
        "deltatoken=t0",
        vec![page(
            vec![json!({"id": "x", "@removed": {"reason": "deleted"}})],
            Some("t1"),
            None,
        )],
    );
    assert_eq!(
        h.source.check_for_changes().await,
        Ok(events_changed(&["cal1"]))
    );
    // An item that fails to parse is still a change.
    h.transport.route(
        "deltatoken=t1",
        vec![page(vec![json!({"id": 7})], Some("t2"), None)],
    );
    assert_eq!(
        h.source.check_for_changes().await,
        Ok(events_changed(&["cal1"]))
    );
}

#[tokio::test]
async fn a_poll_follows_pages_and_takes_the_link_from_the_last_one() {
    let h = one_calendar().await;
    h.transport
        .route("calendarView/delta", vec![delta_page("t0")]);
    h.source.check_for_changes().await.unwrap();
    h.transport.route(
        "deltatoken=t0",
        vec![page(
            vec![graph_event("a", json!({}))],
            None,
            Some(&format!("{DELTA_BASE}?$skiptoken=p2")),
        )],
    );
    h.transport.route("skiptoken=p2", vec![delta_page("t1")]);
    assert_eq!(
        h.source.check_for_changes().await,
        Ok(events_changed(&["cal1"]))
    );
    let state = stored_delta(&h.sync).await;
    assert!(state.link.as_str().ends_with("$deltatoken=t1"));
}

#[tokio::test]
async fn a_link_graph_no_longer_accepts_takes_a_new_baseline_and_reports_a_change() {
    let h = one_calendar().await;
    h.transport
        .route("calendarView/delta", vec![delta_page("t0")]);
    h.source.check_for_changes().await.unwrap();
    h.transport
        .route("deltatoken=t0", vec![graph_error("syncStateNotFound", 410)]);
    h.transport.route(
        "calendarView/delta?startDateTime",
        vec![delta_page("fresh")],
    );
    assert_eq!(
        h.source.check_for_changes().await,
        Ok(events_changed(&["cal1"]))
    );
    let state = stored_delta(&h.sync).await;
    assert!(state.link.as_str().ends_with("$deltatoken=fresh"));
}

#[tokio::test]
async fn a_bad_request_that_means_resync_is_treated_as_gone() {
    let h = one_calendar().await;
    h.transport
        .route("calendarView/delta", vec![delta_page("t0")]);
    h.source.check_for_changes().await.unwrap();
    h.transport
        .route("deltatoken=t0", vec![graph_error("ResyncRequired", 400)]);
    h.transport.route(
        "calendarView/delta?startDateTime",
        vec![delta_page("fresh")],
    );
    assert_eq!(
        h.source.check_for_changes().await,
        Ok(events_changed(&["cal1"]))
    );
}

#[tokio::test]
async fn a_window_older_than_two_weeks_is_renewed_after_the_poll_and_reports_a_change() {
    let h = one_calendar().await;
    h.transport
        .route("calendarView/delta", vec![delta_page("t0")]);
    h.source.check_for_changes().await.unwrap();
    h.now.advance(13.0 * 86_400.0);
    h.transport.route("deltatoken=t0", vec![delta_page("t1")]);
    // 13 days: not yet.
    assert_eq!(h.source.check_for_changes().await, Ok(None));
    h.now.advance(2.0 * 86_400.0);
    h.transport.route("deltatoken=t1", vec![delta_page("t2")]);
    h.transport.route(
        "calendarView/delta?startDateTime",
        vec![delta_page("renewed")],
    );
    assert_eq!(
        h.source.check_for_changes().await,
        Ok(events_changed(&["cal1"]))
    );
    let state = stored_delta(&h.sync).await;
    assert!(
        state.baseline_date == h.now.date() && state.link.as_str().ends_with("$deltatoken=renewed")
    );
    // The poll ran before the renewal, so nothing done in between is lost.
    let order: Vec<String> = h
        .transport
        .requests()
        .iter()
        .map(|r| r.url.as_str().to_string())
        .filter(|u| u.contains("calendarView/delta"))
        .collect();
    let last_two = &order[order.len() - 2..];
    assert!(last_two[0].contains("deltatoken=t1") && last_two[1].contains("startDateTime"));
}

#[tokio::test]
async fn a_changed_calendar_set_reports_calendars_changed_and_drops_removed_tokens() {
    let h = SourceHarness::with_calendars(calendars_json_ids(&["cal1", "cal2"])).await;
    h.transport
        .route("calendarView/delta", vec![delta_page("t0")]);
    h.source.check_for_changes().await.unwrap();
    assert!(h.sync.token("c1", "cal2").await.is_some());
    h.transport
        .route("me/calendars?", vec![ok(calendars_json_ids(&["cal1"]))]);
    h.transport.route("deltatoken=t0", vec![delta_page("t1")]);
    assert_eq!(
        h.source.check_for_changes().await,
        Ok(Some(CalendarChange::CalendarsChanged))
    );
    assert!(h.sync.token("c1", "cal2").await.is_none());
}

#[tokio::test]
async fn a_removed_or_forbidden_calendar_is_skipped() {
    let h = one_calendar().await;
    h.transport.route(
        "calendarView/delta",
        vec![graph_error("ErrorItemNotFound", 404)],
    );
    assert_eq!(h.source.check_for_changes().await, Ok(None));
    h.transport.route(
        "calendarView/delta",
        vec![graph_error("ErrorAccessDenied", 403)],
    );
    assert_eq!(h.source.check_for_changes().await, Ok(None));
}

#[tokio::test]
async fn the_delta_position_survives_a_relaunch() {
    let sync = Arc::new(InMemorySyncStateStore::new());
    let first = SourceHarness::with_sync(calendars_json_ids(&["cal1"]), sync.clone()).await;
    first
        .transport
        .route("calendarView/delta", vec![delta_page("t0")]);
    first.source.check_for_changes().await.unwrap();

    let second = SourceHarness::with_sync(calendars_json_ids(&["cal1"]), sync).await;
    second.transport.route(
        "deltatoken=t0",
        vec![page(vec![graph_event("a", json!({}))], Some("t1"), None)],
    );
    assert_eq!(
        second.source.check_for_changes().await,
        Ok(events_changed(&["cal1"]))
    );
}

#[tokio::test]
async fn a_stored_value_that_is_not_a_delta_state_is_baselined_again() {
    let sync = Arc::new(InMemorySyncStateStore::new());
    sync.set_token(Some("garbage"), "c1", "cal1").await;
    let h = SourceHarness::with_sync(calendars_json_ids(&["cal1"]), sync.clone()).await;
    h.transport
        .route("calendarView/delta", vec![delta_page("t0")]);
    assert_eq!(h.source.check_for_changes().await, Ok(None));
    stored_delta(&sync).await;
}

#[test]
fn delta_state_round_trips() {
    let state = DeltaState {
        baseline_date: Timestamp::from_second(1_790_000_000).unwrap(),
        link: UrlString::parse("https://graph.microsoft.com/v1.0/x?$deltatoken=a|b").unwrap(),
    };
    // A `|` inside the link survives: only the first one splits.
    assert_eq!(DeltaState::parse(&state.stored()), Some(state));
    assert!(DeltaState::parse("no-bar").is_none() && DeltaState::parse("abc|https://x").is_none());
}
