//! Port of `SourceTests.swift`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::{HttpRequest, HttpResponse, HttpTransport};
use calendar_core::recurrence::rule::{
    Frequency, RecurrenceRule, RuleEnd, Weekday, WeekdayOccurrence,
};
use calendar_core::source_types::{CalendarSource, SeriesSource, SourceError, SyncKind};
use calendar_core::write::types::{EventField, RecurrenceScope};
use calendar_test_support::all_day_conformance::AllDayConformance;
use calendar_test_support::fake_transport::FakeTransport;
use calendar_test_support::provided_fields_conformance::ProvidedFieldsConformance;
use jiff::Timestamp;
use serde_json::json;

use super::support::{
    SourceHarness, calendars_json_ids, day_window, graph_error, graph_event, ok, one_hour_window,
    status,
};

/// Swift's `GraphReadProbe`: delays every `calendarView` request and records how many are in flight.
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
        if !request.url.path().contains("/calendarView") {
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

async fn probed(delay: Duration) -> (SourceHarness, Arc<ReadProbe>, Vec<String>) {
    let ids = twenty_ids();
    let slot: Arc<Mutex<Option<Arc<ReadProbe>>>> = Arc::new(Mutex::new(None));
    let captured = slot.clone();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let h = SourceHarness::wrapped(calendars_json_ids(&refs), move |base| {
        let probe = Arc::new(ReadProbe::new(base, delay));
        *captured.lock().unwrap_or_else(PoisonError::into_inner) = Some(probe.clone());
        probe
    })
    .await;
    let probe = slot.lock().unwrap().clone().unwrap();
    (h, probe, ids)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn cancelling_graph_fetch_does_not_start_queued_calendar_reads() {
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
async fn twenty_graph_calendars_never_have_more_than_four_active_event_reads() {
    let (h, probe, ids) = probed(Duration::from_millis(30)).await;
    for (index, id) in ids.iter().enumerate() {
        h.transport.route(
            &format!("{id}/calendarView"),
            vec![ok(
                json!({"value": [graph_event(&format!("e{index}"), json!({}))]}),
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
            .filter(|r| r.url.path().contains("/calendarView"))
            .count(),
        20
    );
}

#[tokio::test]
async fn calendars_maps_and_reads_the_mailbox_zone() {
    let h = SourceHarness::new().await;
    let calendars = h.source.calendars().await.unwrap();
    assert_eq!(
        calendars.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        ["cal1", "cal2"]
    );
    assert!(
        calendars[0].account_name.as_deref() == Some("me@x.com")
            && calendars[0].color_hex() == Some("#112233")
            && calendars[0].is_default == Some(true)
    );
    assert!(calendars.iter().all(|c| {
        c.time_zone.as_ref().map(|z| z.identifier()) == Some("America/Los_Angeles")
            && c.service == calendar_core::model::CalendarService::microsoft()
    }));
    let request = h
        .transport
        .requests_matching("me/calendars?")
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(
        request.headers.get("Authorization").map(String::as_str),
        Some("Bearer at1")
    );
    assert!(request.url.as_str().contains("$top=100"));
}

#[tokio::test]
async fn a_refused_or_unknown_mailbox_zone_is_utc() {
    let refused = SourceHarness::new().await;
    refused.transport.route(
        "me/mailboxSettings/timeZone",
        vec![graph_error("ErrorAccessDenied", 403)],
    );
    assert!(refused.source.calendars().await.unwrap().iter().all(|c| {
        let zone = c.time_zone.as_ref().unwrap();
        zone.identifier() == "UTC" || zone.offset_seconds(Timestamp::UNIX_EPOCH) == 0
    }));

    let unknown = SourceHarness::with_mailbox_zone("Customized Time Zone").await;
    assert_eq!(
        unknown.source.calendars().await.unwrap()[0]
            .time_zone
            .as_ref()
            .map(|z| z.offset_seconds(Timestamp::UNIX_EPOCH)),
        Some(0)
    );
}

#[tokio::test]
async fn a_failing_mailbox_zone_lookup_is_not_turned_into_utc() {
    let h = SourceHarness::new().await;
    h.transport
        .route("me/mailboxSettings/timeZone", vec![status(json!({}), 500)]);
    assert_eq!(
        h.source.calendars().await,
        Err(Failure::Source(SourceError::Server { status: 500 }))
    );
}

#[tokio::test]
async fn calendars_reports_provided_fields_it_declares() {
    let h = SourceHarness::new().await;
    for calendar in h.source.calendars().await.unwrap() {
        assert!(
            ProvidedFieldsConformance::calendar_violations(&calendar, &h.source.capabilities())
                .is_empty()
        );
    }
    assert!(ProvidedFieldsConformance::source_violations(h.source.as_ref()).is_empty());
}

#[tokio::test]
async fn events_fetches_every_calendar_with_the_window_and_reads_in_the_account_zone() {
    let h = SourceHarness::new().await;
    h.transport.route(
        "cal1/calendarView",
        vec![ok(json!({"value": [graph_event("b", json!({
            "start": {"dateTime": "2026-09-25T12:00:00.0000000", "timeZone": "Pacific Standard Time"},
            "end": {"dateTime": "2026-09-25T13:00:00.0000000", "timeZone": "Pacific Standard Time"}}))]}))],
    );
    h.transport.route(
        "cal2/calendarView",
        vec![ok(json!({"value": [graph_event("a", json!({}))]}))],
    );
    let events = h.source.events_in(day_window()).await.unwrap();
    assert_eq!(
        events
            .iter()
            .map(|e| e.event_id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(
        events
            .iter()
            .map(|e| e.calendar_id.as_str())
            .collect::<Vec<_>>(),
        ["cal2", "cal1"]
    );
    assert!(
        events
            .iter()
            .all(|e| e.source_id.as_deref() == Some("microsoft-c1"))
    );

    let request = h
        .transport
        .requests_matching("cal1/calendarView")
        .into_iter()
        .next()
        .unwrap();
    let url = request.url.as_str();
    assert!(
        url.contains("startDateTime=2026-09-2")
            && url.contains("endDateTime=2026-09-2")
            && url.contains("$top=100")
    );
    assert!(url.contains("$select=id,iCalUId"));
    assert_eq!(
        request.headers.get("Prefer").map(String::as_str),
        Some(
            "IdType=\"ImmutableId\", outlook.timezone=\"Pacific Standard Time\", outlook.body-content-type=\"text\""
        )
    );
}

#[tokio::test]
async fn events_reuses_the_stored_calendar_list_and_zone() {
    let h = SourceHarness::new().await;
    h.transport
        .route("calendarView", vec![ok(json!({"value": []}))]);
    h.source.calendars().await.unwrap();
    h.source.events_in(day_window()).await.unwrap();
    h.source.events_in(day_window()).await.unwrap();
    assert_eq!(h.transport.requests_matching("me/calendars?").len(), 1);
    assert_eq!(h.transport.requests_matching("mailboxSettings").len(), 1);
}

#[tokio::test]
async fn events_skips_a_calendar_that_was_removed_or_lost_access() {
    let h = SourceHarness::new().await;
    h.transport.route(
        "cal1/calendarView",
        vec![graph_error("ErrorItemNotFound", 404)],
    );
    h.transport.route(
        "cal2/calendarView",
        vec![ok(json!({"value": [graph_event("a", json!({}))]}))],
    );
    let ids = |events: Vec<calendar_core::model::CalendarEvent>| -> Vec<String> {
        events.into_iter().map(|e| e.event_id).collect()
    };
    assert_eq!(ids(h.source.events_in(day_window()).await.unwrap()), ["a"]);
    h.transport.route(
        "cal2/calendarView",
        vec![graph_error("ErrorAccessDenied", 403)],
    );
    assert!(h.source.events_in(day_window()).await.unwrap().is_empty());
}

#[tokio::test]
async fn events_drops_cancelled_and_unreadable_items_and_follows_pages() {
    let h = SourceHarness::with_calendars(calendars_json_ids(&["cal1"])).await;
    h.transport.route(
        "cal1/calendarView",
        vec![
            ok(json!({
                "value": [graph_event("a", json!({})), graph_event("gone", json!({"isCancelled": true}))],
                "@odata.nextLink": "https://graph.microsoft.com/v1.0/me/calendars/cal1/calendarView?$skiptoken=2",
            })),
            ok(json!({"value": [{"id": 5}, graph_event("c", json!({}))]})),
        ],
    );
    let events = h.source.events_in(day_window()).await.unwrap();
    assert_eq!(
        events
            .iter()
            .map(|e| e.event_id.as_str())
            .collect::<Vec<_>>(),
        ["a", "c"]
    );
}

#[tokio::test]
async fn mapped_events_meet_the_provided_fields_and_all_day_contracts() {
    let h = SourceHarness::with_calendars(calendars_json_ids(&["cal1"])).await;
    h.transport.route(
        "cal1/calendarView",
        vec![ok(json!({"value": [
            graph_event("timed", json!({})),
            graph_event("day", json!({
                "isAllDay": true,
                "start": {"dateTime": "2026-09-25T00:00:00.0000000", "timeZone": "Pacific Standard Time"},
                "end": {"dateTime": "2026-09-26T00:00:00.0000000", "timeZone": "Pacific Standard Time"}})),
        ]}))],
    );
    for event in h.source.events_in(day_window()).await.unwrap() {
        assert!(
            ProvidedFieldsConformance::event_violations(&event, &h.source.capabilities())
                .is_empty(),
            "{}",
            event.event_id
        );
        assert!(
            AllDayConformance::violations(&event).is_empty(),
            "{}",
            event.event_id
        );
    }
}

#[tokio::test]
async fn an_expired_token_surfaces_as_auth_expired() {
    let h = SourceHarness::new().await;
    h.transport
        .route("me/mailboxSettings/timeZone", vec![status(json!({}), 401)]);
    assert_eq!(
        h.source.calendars().await,
        Err(Failure::Source(SourceError::AuthExpired))
    );
}

#[tokio::test]
async fn capabilities_describe_a_fully_writable_synced_source() {
    let h = SourceHarness::new().await;
    let caps = h.source.capabilities();
    assert!(
        caps.can_write
            && caps.can_edit_attendees
            && caps.can_respond_to_invite
            && caps.sync_kind == SyncKind::Token
            && !caps.supports_push
    );
    assert!(
        caps.writable_fields == EventField::ALL.iter().copied().collect()
            && caps.recurrence_scopes == RecurrenceScope::ALL.iter().copied().collect()
    );
    assert!(!caps.controls_notifications);
    assert_eq!(
        caps.can_edit_attendees,
        caps.writable_fields.contains(&EventField::Attendees)
    );
}

// MARK: Series

#[tokio::test]
async fn series_reads_the_masters_rule_and_anchor() {
    let h = SourceHarness::new().await;
    h.transport.route(
        "cal1/events/master1",
        vec![ok(graph_event(
            "master1",
            json!({
                "type": "seriesMaster",
                "recurrence": {
                    "pattern": {"type": "weekly", "interval": 1, "daysOfWeek": ["friday"], "firstDayOfWeek": "monday"},
                    "range": {"type": "numbered", "startDate": "2026-09-25", "numberOfOccurrences": 8, "recurrenceTimeZone": "Pacific Standard Time"},
                },
            }),
        ))],
    );
    let series = h.source.series("master1", "cal1").await.unwrap();
    assert!(series.series_id == "master1" && series.calendar_id == "cal1" && !series.is_all_day);
    assert!(
        series.start == Timestamp::from_second(1_790_355_600).unwrap()
            && series.time_zone.identifier() == "America/Los_Angeles"
    );
    assert_eq!(
        series.recurrence.rules,
        [RecurrenceRule {
            weekdays: vec![WeekdayOccurrence::new(Weekday::Friday, None)],
            end: RuleEnd::Count(8),
            ..RecurrenceRule::new(Frequency::Weekly)
        }]
    );
    assert!(
        series.recurrence.excluded_dates.is_none()
            && series.recurrence.extra_dates.is_none()
            && series.recurrence.unparsed.is_empty()
    );
}

#[tokio::test]
async fn series_rejects_an_event_that_is_not_a_series_master() {
    let h = SourceHarness::new().await;
    h.transport.route(
        "cal1/events/single",
        vec![ok(graph_event("single", json!({})))],
    );
    h.transport.route(
        "cal1/events/missing",
        vec![graph_error("ErrorItemNotFound", 404)],
    );
    assert_eq!(
        h.source.series("single", "cal1").await.err(),
        Some(Failure::Source(SourceError::NotFound))
    );
    assert_eq!(
        h.source.series("missing", "cal1").await.err(),
        Some(Failure::Source(SourceError::NotFound))
    );
}

#[tokio::test]
async fn series_keeps_an_unknown_pattern_as_an_unparsed_line() {
    let h = SourceHarness::new().await;
    h.transport.route(
        "cal1/events/m",
        vec![ok(graph_event(
            "m",
            json!({"type": "seriesMaster", "recurrence": {"pattern": {"type": "lunar"}, "range": {"type": "noEnd", "startDate": "2026-09-25"}}}),
        ))],
    );
    let series = h.source.series("m", "cal1").await.unwrap();
    assert!(
        series.recurrence.rules.is_empty()
            && series.recurrence.unparsed == ["X-MS-RECURRENCE:lunar"]
    );
}
