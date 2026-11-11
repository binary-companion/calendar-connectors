//! Port of `SourceTests.swift`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::{HttpRequest, HttpResponse, HttpTransport};
use calendar_core::recurrence::rule::Frequency;
use calendar_core::source_types::ProvidedField;
use calendar_core::source_types::{CalendarSource, SourceError, SyncKind};
use calendar_core::time::{DateInterval, Zone};
use calendar_test_support::fake_transport::FakeTransport;
use calendar_test_support::provided_fields_conformance::ProvidedFieldsConformance;
use jiff::Timestamp;
use serde_json::{Value, json};

use super::support::{
    Harness, event_json, instant, list_json, ok, one_hour_window, status, text, with_headers,
};

fn source_failure(error: SourceError) -> Failure {
    Failure::Source(error)
}

/// Swift's `GoogleReadProbe`: delays every events request and records how many are in flight.
struct ReadProbe {
    base: Arc<FakeTransport>,
    delay: Duration,
    active: AtomicUsize,
    peak: AtomicUsize,
    started: AtomicUsize,
}

impl ReadProbe {
    fn new(base: Arc<FakeTransport>, delay: Duration) -> ReadProbe {
        ReadProbe {
            base,
            delay,
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            started: AtomicUsize::new(0),
        }
    }
}

/// Swift's `defer { active -= 1 }`: also runs when the request is dropped (cancelled).
struct InFlight<'a>(&'a AtomicUsize);

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl HttpTransport for ReadProbe {
    async fn send(&self, request: HttpRequest) -> Outcome<HttpResponse> {
        if !request.url.path().contains("/events") {
            return self.base.send(request).await;
        }
        let now_active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        let _in_flight = InFlight(&self.active);
        self.started.fetch_add(1, Ordering::SeqCst);
        self.peak.fetch_max(now_active, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        self.base.send(request).await
    }
}

fn twenty_ids() -> Vec<String> {
    (0..20).map(|i| format!("cal{i}")).collect()
}

fn as_strs(ids: &[String]) -> Vec<&str> {
    ids.iter().map(String::as_str).collect()
}

async fn probed(delay: Duration) -> (Harness, Arc<ReadProbe>, Vec<String>) {
    let ids = twenty_ids();
    let slot: Arc<Mutex<Option<Arc<ReadProbe>>>> = Arc::new(Mutex::new(None));
    let captured = slot.clone();
    let h = Harness::wrapped(list_json(&as_strs(&ids)), move |base| {
        let probe = Arc::new(ReadProbe::new(base, delay));
        *captured.lock().unwrap_or_else(PoisonError::into_inner) = Some(probe.clone());
        probe
    })
    .await;
    let probe = slot.lock().unwrap().clone().unwrap();
    (h, probe, ids)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn cancelling_google_fetch_does_not_start_queued_calendar_reads() {
    let (h, probe, _) = probed(Duration::from_secs(2)).await;
    let source = h.source.clone();
    let fetch = tokio::spawn(async move { source.events_in(one_hour_window()).await });
    for _ in 0..100 {
        if probe.started.load(Ordering::SeqCst) == 4 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(probe.started.load(Ordering::SeqCst), 4);
    fetch.abort();
    assert!(fetch.await.unwrap_err().is_cancelled());
    assert_eq!(probe.started.load(Ordering::SeqCst), 4);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn twenty_google_calendars_never_have_more_than_four_active_event_reads() {
    let (h, probe, ids) = probed(Duration::from_millis(30)).await;
    for (index, id) in ids.iter().enumerate() {
        h.transport.route(
            &format!("calendars/{id}/events"),
            vec![ok(
                json!({"items": [event_json(&format!("e{index}"), "2026-09-21T10:00:00Z")]}),
            )],
        );
    }
    let events = h.source.events_in(one_hour_window()).await.unwrap();
    assert!(probe.peak.load(Ordering::SeqCst) <= 4);
    let mut expected: Vec<String> = (0..ids.len()).map(|i| format!("e{i}")).collect();
    expected.sort();
    assert_eq!(
        events
            .iter()
            .map(|e| e.event_id.clone())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        h.transport
            .requests()
            .iter()
            .filter(|r| r.url.path().contains("/events"))
            .count(),
        20
    );
}

#[tokio::test]
async fn calendars_maps_and_skips_hidden() {
    let h = Harness::new().await;
    let cals = h.source.calendars().await.unwrap();
    assert_eq!(
        cals.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        ["me@x.com", "team@group.calendar.google.com"]
    );
    assert!(
        cals[0].account_name.as_deref() == Some("me@x.com")
            && cals[0].color_hex() == Some("#112233")
            && cals[0].is_default == Some(true)
    );
    let request = h.transport.requests().into_iter().next().unwrap();
    assert_eq!(
        request.headers.get("Authorization").map(String::as_str),
        Some("Bearer at1")
    );
    assert!(request.url.as_str().contains("showHidden=false"));
}

#[tokio::test]
async fn events_fetches_every_calendar_with_window_and_sorts_by_start() {
    let h = Harness::new().await;
    h.transport.route(
        "calendars/me%40x.com/events",
        vec![ok(
            json!({"items": [event_json("b", "2026-09-21T12:00:00Z")]}),
        )],
    );
    h.transport.route(
        "calendars/team%40group.calendar.google.com/events",
        vec![ok(
            json!({"items": [event_json("a", "2026-09-21T09:00:00Z")]}),
        )],
    );
    let interval = DateInterval::new(
        Timestamp::from_second(1_790_000_000).unwrap(),
        Timestamp::from_second(1_790_086_400).unwrap(),
    );
    let events = h.source.events_in(interval).await.unwrap();
    assert_eq!(
        events
            .iter()
            .map(|e| e.event_id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    let request = h
        .transport
        .requests_matching("calendars/me%40x.com/events")
        .into_iter()
        .next()
        .unwrap();
    let url = request.url.as_str();
    assert!(
        url.contains("singleEvents=true") && url.contains("timeMin=") && url.contains("timeMax=")
    );
    assert!(url.contains("showDeleted=false"));
}

#[tokio::test]
async fn events_follows_paging() {
    let h = Harness::new().await;
    h.transport.route(
        "calendars/team%40group.calendar.google.com/events",
        vec![ok(json!({"items": []}))],
    );
    h.transport.route(
        "calendars/me%40x.com/events",
        vec![ok(
            json!({"items": [event_json("one", "2026-09-21T10:00:00Z")], "nextPageToken": "page2"}),
        )],
    );
    h.transport.route(
        "pageToken=page2",
        vec![ok(
            json!({"items": [event_json("two", "2026-09-21T11:00:00Z")]}),
        )],
    );
    let events = h.source.events_in(one_hour_window()).await.unwrap();
    assert_eq!(
        events
            .iter()
            .map(|e| e.event_id.as_str())
            .collect::<Vec<_>>(),
        ["one", "two"]
    );
}

#[tokio::test]
async fn a_missing_or_forbidden_calendar_is_skipped_not_fatal() {
    let h = Harness::new().await;
    h.transport.route(
        "calendars/me%40x.com/events",
        vec![ok(
            json!({"items": [event_json("ok", "2026-09-21T10:00:00Z")]}),
        )],
    );
    h.transport.route(
        "calendars/team%40group.calendar.google.com/events",
        vec![status(
            json!({"error": {"errors": [{"reason": "forbidden"}]}}),
            403,
        )],
    );
    let events = h.source.events_in(one_hour_window()).await.unwrap();
    assert_eq!(
        events
            .iter()
            .map(|e| e.event_id.as_str())
            .collect::<Vec<_>>(),
        ["ok"]
    );
}

#[tokio::test]
async fn a_401_refreshes_the_token_once_then_succeeds() {
    let h = Harness::with_list(list_json(&["me@x.com"])).await;
    h.transport.route(
        "calendars/me%40x.com/events",
        vec![status(json!({}), 401), ok(json!({"items": []}))],
    );
    h.source.events_in(one_hour_window()).await.unwrap();
    let requests = h.transport.requests_matching("calendars/me%40x.com/events");
    assert_eq!(
        requests
            .iter()
            .map(|r| r.headers.get("Authorization").cloned())
            .collect::<Vec<_>>(),
        [
            Some("Bearer at1".to_string()),
            Some("Bearer at2".to_string())
        ]
    );
}

#[tokio::test]
async fn a_forbidden_reason_that_affects_every_calendar_is_not_swallowed() {
    let h = Harness::with_list(list_json(&["me@x.com"])).await;
    h.transport.route(
        "calendars/me%40x.com/events",
        vec![status(
            json!({"error": {"errors": [{"reason": "accessNotConfigured"}]}}),
            403,
        )],
    );
    assert_eq!(
        h.source.events_in(one_hour_window()).await.unwrap_err(),
        source_failure(SourceError::InvalidResponse(
            "HTTP 403: accessNotConfigured".to_string()
        ))
    );
}

#[tokio::test]
async fn insufficient_permissions_needs_reconsent() {
    let h = Harness::with_list(list_json(&["me@x.com"])).await;
    h.transport.route(
        "calendars/me%40x.com/events",
        vec![status(
            json!({"error": {"errors": [{"reason": "insufficientPermissions"}]}}),
            403,
        )],
    );
    assert_eq!(
        h.source.events_in(one_hour_window()).await.unwrap_err(),
        source_failure(SourceError::AuthExpired)
    );
}

#[tokio::test]
async fn an_unparseable_forbidden_body_is_invalid_response_not_skipped() {
    let h = Harness::with_list(list_json(&["me@x.com"])).await;
    h.transport
        .route("calendars/me%40x.com/events", vec![text("nope", 403)]);
    assert_eq!(
        h.source.events_in(one_hour_window()).await.unwrap_err(),
        source_failure(SourceError::InvalidResponse(
            "HTTP 403: unknown".to_string()
        ))
    );
}

#[tokio::test]
async fn a_repeated_401_is_auth_expired() {
    let h = Harness::new().await;
    h.transport
        .route("calendars/", vec![status(json!({}), 401)]);
    assert_eq!(
        h.source.events_in(one_hour_window()).await.unwrap_err(),
        source_failure(SourceError::AuthExpired)
    );
}

#[tokio::test]
async fn rate_limits_are_retried_with_backoff_then_surfaced() {
    let h = Harness::with_list(list_json(&["me@x.com"])).await;
    h.transport.route(
        "calendars/",
        vec![with_headers(
            json!({"error": {"errors": [{"reason": "rateLimitExceeded"}]}}),
            403,
            &[("Retry-After", "2")],
        )],
    );
    assert_eq!(
        h.source.events_in(one_hour_window()).await.unwrap_err(),
        source_failure(SourceError::RateLimited {
            retry_after: Some(2.0)
        })
    );
    assert_eq!(h.sleeps.durations(), [2.0, 2.0, 2.0]);
}

#[tokio::test]
async fn server_errors_and_malformed_bodies_surface_as_source_errors() {
    let h = Harness::new().await;
    h.transport
        .route("calendars/", vec![status(json!({}), 503)]);
    assert_eq!(
        h.source.events_in(one_hour_window()).await.unwrap_err(),
        source_failure(SourceError::Server { status: 503 })
    );
    let h2 = Harness::new().await;
    h2.transport
        .route("calendars/", vec![text("not json", 200)]);
    assert!(matches!(
        h2.source.events_in(one_hour_window()).await,
        Err(Failure::Source(_))
    ));
}

#[tokio::test]
async fn provider_specific_errors_never_escape_as_google_types() {
    let h = Harness::new().await;
    h.transport
        .route("users/me/calendarList", vec![status(json!({}), 404)]);
    assert!(matches!(
        h.source.calendars().await,
        Err(Failure::Source(_))
    ));
    h.transport
        .route("users/me/calendarList", vec![status(json!({}), 410)]);
    assert!(matches!(
        h.source.as_polling().unwrap().check_for_changes().await,
        Err(Failure::Source(_))
    ));
}

#[tokio::test]
async fn capabilities_describe_a_writable_token_synced_source_without_push() {
    let h = Harness::new().await;
    let c = h.source.capabilities();
    assert!(
        c.can_write
            && c.provided_fields
                .contains(&ProvidedField::StructuredConference)
            && c.sync_kind == SyncKind::Token
            && !c.supports_push
    );
    assert!(h.source.id() == "google-c1" && h.source.display_name() == "me@x.com");
}

#[tokio::test]
async fn events_carry_the_source_id_of_their_connection() {
    let h = Harness::new().await;
    h.transport.route(
        "calendars/team%40group.calendar.google.com/events",
        vec![ok(json!({"items": []}))],
    );
    h.transport.route(
        "calendars/me%40x.com/events",
        vec![ok(
            json!({"items": [event_json("one", "2026-09-21T10:00:00Z")]}),
        )],
    );
    let events = h.source.events_in(one_hour_window()).await.unwrap();
    assert!(
        events.len() == 1
            && events
                .iter()
                .all(|e| e.source_id.as_deref() == Some("google-c1"))
    );
}

#[tokio::test]
async fn an_item_without_an_id_is_dropped_not_fatal() {
    let h = Harness::new().await;
    h.transport.route(
        "calendars/team%40group.calendar.google.com/events",
        vec![ok(json!({"items": []}))],
    );
    h.transport.route(
        "calendars/me%40x.com/events",
        vec![ok(json!({"items": [
            event_json("one", "2026-09-21T10:00:00Z"),
            {"noId": true},
            event_json("two", "2026-09-21T11:00:00Z"),
        ]}))],
    );
    let events = h.source.events_in(one_hour_window()).await.unwrap();
    assert_eq!(
        events
            .iter()
            .map(|e| e.event_id.as_str())
            .collect::<Vec<_>>(),
        ["one", "two"]
    );
}

#[tokio::test]
async fn an_item_with_a_wrongly_typed_field_is_dropped_not_fatal() {
    let h = Harness::new().await;
    let mut bad = event_json("bad", "2026-09-21T09:00:00Z");
    bad["reminders"] = json!({"overrides": [{"minutes": "ten"}]});
    h.transport.route(
        "calendars/team%40group.calendar.google.com/events",
        vec![ok(json!({"items": []}))],
    );
    h.transport.route(
        "calendars/me%40x.com/events",
        vec![ok(
            json!({"items": [bad, event_json("good", "2026-09-21T10:00:00Z")]}),
        )],
    );
    let events = h.source.events_in(one_hour_window()).await.unwrap();
    assert_eq!(
        events
            .iter()
            .map(|e| e.event_id.as_str())
            .collect::<Vec<_>>(),
        ["good"]
    );
}

fn calendar_list_requests(h: &Harness) -> usize {
    h.transport.requests_matching("users/me/calendarList").len()
}

#[tokio::test]
async fn calendars_then_events_makes_one_calendar_list_request() {
    let h = Harness::with_list(list_json(&["me@x.com"])).await;
    h.transport.route(
        "calendars/me%40x.com/events",
        vec![ok(json!({"items": []}))],
    );
    h.source.calendars().await.unwrap();
    h.source.events_in(one_hour_window()).await.unwrap();
    assert_eq!(calendar_list_requests(&h), 1);
}

#[tokio::test]
async fn events_on_a_fresh_source_fetches_the_calendar_list_once() {
    let h = Harness::with_list(list_json(&["me@x.com"])).await;
    h.transport.route(
        "calendars/me%40x.com/events",
        vec![ok(json!({"items": []})), ok(json!({"items": []}))],
    );
    h.source.events_in(one_hour_window()).await.unwrap();
    h.source.events_in(one_hour_window()).await.unwrap();
    assert_eq!(calendar_list_requests(&h), 1);
}

#[tokio::test]
async fn a_poll_stores_the_list_for_events() {
    let h = Harness::with_list(list_json(&["me@x.com"])).await;
    h.transport.route(
        "calendars/me%40x.com/events",
        vec![ok(json!({"nextSyncToken": "t1"})), ok(json!({"items": []}))],
    );
    h.source
        .as_polling()
        .unwrap()
        .check_for_changes()
        .await
        .unwrap();
    assert_eq!(calendar_list_requests(&h), 1);
    h.source.events_in(one_hour_window()).await.unwrap();
    assert_eq!(calendar_list_requests(&h), 1);
}

#[tokio::test]
async fn a_calendar_added_between_polls_appears_after_the_next_calendars_call() {
    let h = Harness::with_list(list_json(&["me@x.com"])).await;
    h.transport.route(
        "calendars/me%40x.com/events",
        vec![ok(json!({"items": []})), ok(json!({"items": []}))],
    );
    h.transport.route(
        "calendars/new%40x.com/events",
        vec![ok(
            json!({"items": [event_json("n", "2026-09-21T10:00:00Z")]}),
        )],
    );
    h.source.events_in(one_hour_window()).await.unwrap();
    h.transport.route(
        "users/me/calendarList",
        vec![ok(list_json(&["me@x.com", "new@x.com"]))],
    );
    h.source.calendars().await.unwrap();
    let events = h.source.events_in(one_hour_window()).await.unwrap();
    assert_eq!(
        events
            .iter()
            .map(|e| e.event_id.as_str())
            .collect::<Vec<_>>(),
        ["n"]
    );
}

// Series (Issue 3).

const MASTER_PATH: &str = "calendars/me%40x.com/events/master1";

fn master_json() -> Value {
    json!({
        "id": "master1", "start": {"dateTime": "2026-09-14T10:00:00-06:00", "timeZone": "America/Denver"},
        "end": {"dateTime": "2026-09-14T10:30:00-06:00", "timeZone": "America/Denver"},
        "recurrence": ["RRULE:FREQ=WEEKLY;BYDAY=MO", "EXDATE;TZID=America/Denver:20260921T100000"],
    })
}

#[tokio::test]
async fn a_master_with_rrule_and_exdate_gives_the_full_series() {
    let h = Harness::with_list(list_json(&["me@x.com"])).await;
    h.transport.route(MASTER_PATH, vec![ok(master_json())]);
    let series = h
        .source
        .as_series()
        .unwrap()
        .series("master1", "me@x.com")
        .await
        .unwrap();
    let denver = Zone::named("America/Denver").unwrap();
    assert!(
        series.series_id == "master1"
            && series.calendar_id == "me@x.com"
            && series.time_zone == denver
            && !series.is_all_day
    );
    assert_eq!(series.start, instant("2026-09-14T16:00:00Z"));
    assert_eq!(
        series
            .recurrence
            .rules
            .iter()
            .map(|r| r.frequency)
            .collect::<Vec<_>>(),
        [Frequency::Weekly]
    );
    assert_eq!(
        series.recurrence.excluded_dates,
        Some(vec![instant("2026-09-21T16:00:00Z")])
    );
}

#[tokio::test]
async fn a_non_recurring_or_cancelled_or_missing_series_is_not_found() {
    let h = Harness::with_list(list_json(&["me@x.com"])).await;
    h.transport.route(
        MASTER_PATH,
        vec![
            ok(json!({"id": "master1", "start": {"dateTime": "2026-09-14T10:00:00Z"}, "end": {"dateTime": "2026-09-14T10:30:00Z"}})),
            ok(json!({"id": "master1", "status": "cancelled", "recurrence": ["RRULE:FREQ=DAILY"]})),
            status(json!({"error": {"errors": [{"reason": "notFound"}]}}), 404),
        ],
    );
    for _ in 0..3 {
        assert_eq!(
            h.source
                .as_series()
                .unwrap()
                .series("master1", "me@x.com")
                .await
                .unwrap_err(),
            source_failure(SourceError::NotFound)
        );
    }
}

#[tokio::test]
async fn google_declares_recurrence_rules_exactly_because_it_is_a_series_source() {
    let h = Harness::new().await;
    assert!(
        h.source
            .capabilities()
            .provided_fields
            .contains(&ProvidedField::RecurrenceRules)
    );
    assert!(ProvidedFieldsConformance::source_violations(h.source.as_ref()).is_empty());
}
