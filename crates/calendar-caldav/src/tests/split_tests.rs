//! Port of `SplitTests.swift`.

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::{HttpRequest, HttpResponse, HttpTransport};
use calendar_core::model::CalendarEvent;
use calendar_core::source_types::{CalendarSource, SourceError};
use calendar_core::time::DateInterval;
use calendar_core::write::event_patch::EventPatch;
use calendar_core::write::types::{
    EventField, EventRef, NotifyPolicy, RecurrenceScope, WriteError,
};
use calendar_core::write::writable_source::WritableCalendarSource;
use calendar_ical::EventResource;
use calendar_test_support::hand_patch;
use jiff::Timestamp;

use super::support::{
    CalDavHarness, FakeCalDavServer, pacific, pt, september, single_ics_with, weekly_ics,
    weekly_ics_by, weekly_ics_with,
};
use super::write_tests::expect_write_error;
use crate::source::CalDavCalendarSource;

async fn occurrence(source: &CalDavCalendarSource, day: i64) -> CalendarEvent {
    let slot = pt(2026, 9, day, 10, 0);
    source
        .events_in(september())
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.original_start() == Some(slot))
        .unwrap()
}

fn autumn() -> DateInterval {
    DateInterval::new(pt(2026, 9, 1, 0, 0), pt(2026, 11, 1, 0, 0))
}

fn id(event: &CalendarEvent) -> EventRef {
    EventRef::from_event(event)
}

fn title(t: &str) -> EventPatch {
    hand_patch(|p| p.title = Some(t.to_string()))
}

fn resource(h: &CalDavHarness, name: &str) -> EventResource {
    EventResource::from_data(h.server.body("home", name).unwrap().as_bytes()).unwrap()
}

fn master_value(resource: &EventResource, name: &str) -> Option<String> {
    resource
        .master()
        .and_then(|m| m.property(name))
        .map(|p| p.value.clone())
}

async fn split(
    source: &CalDavCalendarSource,
    event: &CalendarEvent,
    patch: &EventPatch,
) -> Outcome<CalendarEvent> {
    source
        .update(
            &id(event),
            patch,
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
}

async fn titles(source: &CalDavCalendarSource, window: DateInterval) -> Vec<String> {
    source
        .events_in(window)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.title)
        .collect()
}

async fn starts(source: &CalDavCalendarSource, window: DateInterval) -> Vec<Timestamp> {
    source
        .events_in(window)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.start)
        .collect()
}

#[tokio::test]
async fn split_an_until_series_moves_later_exceptions() {
    let h = CalDavHarness::new();
    h.server.store(
        "home",
        "weekly.ics",
        &weekly_ics_with(
            "weekly-uid",
            "FREQ=WEEKLY;BYDAY=TU;UNTIL=20261013T235959Z",
            None,
        ),
    );
    let source = h.source().await;
    let fifteenth = occurrence(&source, 15).await; // the moved occurrence
    let tail = split(&source, &fifteenth, &title("New sync"))
        .await
        .unwrap();
    assert!(
        tail.event_id == "uuid-2.ics"
            && tail.series_id() == Some("uuid-2.ics")
            && tail.uid.as_deref() == Some("uuid-1")
    );
    let head = resource(&h, "weekly.ics");
    assert!(
        master_value(&head, "RRULE")
            .unwrap()
            .contains("UNTIL=20260915T165959Z")
    );
    assert!(head.overrides().is_empty());
    let newer = resource(&h, "uuid-2.ics");
    assert!(
        newer.overrides().len() == 1
            && newer.overrides()[0].property("UID").unwrap().text() == "uuid-1"
    );
    assert_eq!(
        titles(&source, autumn()).await,
        [
            "Team sync",
            "Team sync (moved)",
            "New sync",
            "New sync",
            "New sync",
            "New sync"
        ]
    );
    assert_eq!(
        starts(&source, autumn()).await,
        [
            pt(2026, 9, 1, 10, 0),
            pt(2026, 9, 16, 14, 0),
            pt(2026, 9, 22, 10, 0),
            pt(2026, 9, 29, 10, 0),
            pt(2026, 10, 6, 10, 0),
            pt(2026, 10, 13, 10, 0)
        ]
    );
}

#[tokio::test]
async fn split_a_count_series_keeps_the_total() {
    let h = CalDavHarness::new();
    h.server.store(
        "home",
        "weekly.ics",
        &weekly_ics_with("weekly-uid", "FREQ=WEEKLY;BYDAY=TU;COUNT=6", None),
    );
    let source = h.source().await;
    split(&source, &occurrence(&source, 22).await, &title("Later"))
        .await
        .unwrap();
    assert_eq!(
        master_value(&resource(&h, "weekly.ics"), "RRULE").as_deref(),
        Some("FREQ=WEEKLY;BYDAY=TU;COUNT=3")
    );
    assert_eq!(
        master_value(&resource(&h, "uuid-2.ics"), "RRULE").as_deref(),
        Some("FREQ=WEEKLY;BYDAY=TU;COUNT=3")
    );
    assert_eq!(
        starts(&source, autumn()).await,
        [
            pt(2026, 9, 1, 10, 0),
            pt(2026, 9, 16, 14, 0),
            pt(2026, 9, 22, 10, 0),
            pt(2026, 9, 29, 10, 0),
            pt(2026, 10, 6, 10, 0)
        ]
    );
}

#[tokio::test]
async fn split_with_a_time_change_moves_the_new_series() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let source = h.source().await;
    let later = calendar_core::write::event_draft::EventTiming::new(
        pt(2026, 9, 22, 15, 0),
        pt(2026, 9, 22, 16, 0),
        Some(pacific()),
        false,
    );
    split(
        &source,
        &occurrence(&source, 22).await,
        &hand_patch(|p| p.timing = Some(later)),
    )
    .await
    .unwrap();
    assert_eq!(
        starts(&source, september()).await,
        [
            pt(2026, 9, 1, 10, 0),
            pt(2026, 9, 16, 14, 0),
            pt(2026, 9, 22, 15, 0),
            pt(2026, 9, 29, 15, 0)
        ]
    );
}

#[tokio::test]
async fn split_at_the_first_occurrence_is_all_in_series() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let source = h.source().await;
    let series = split(&source, &occurrence(&source, 1).await, &title("Renamed"))
        .await
        .unwrap();
    assert_eq!(series.event_id, "weekly.ics");
    assert_eq!(h.server.names("home"), ["weekly.ics"]);
    let renamed = source
        .events_in(september())
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.title == "Renamed")
        .count();
    assert_eq!(renamed, 3);
}

#[tokio::test]
async fn a_failed_second_step_restores_the_original() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let source = h.source().await;
    let before = starts(&source, september()).await;
    h.server.fail("PUT", "uuid-2.ics", 507);
    let target = occurrence(&source, 22).await;
    assert_eq!(
        split(&source, &target, &title("x")).await.unwrap_err(),
        Failure::Source(SourceError::Server { status: 507 })
    );
    assert_eq!(starts(&source, september()).await, before);
    assert_eq!(h.server.names("home"), ["weekly.ics"]);
}

#[tokio::test]
async fn a_stale_restore_is_partial() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let source = h.source().await;
    h.server.fail("PUT", "uuid-2.ics", 507);
    // The truncation passes, the restore is stale.
    h.server.fail_n("PUT", "weekly.ics", 412, 1, 1);
    let target = occurrence(&source, 22).await;
    match split(&source, &target, &title("x")).await {
        Err(Failure::Write(WriteError::Partial(message))) => {
            assert!(message.contains("not restored"))
        }
        other => panic!("expected partial, got {other:?}"),
    }
}

// MARK: Beyond the brief: ordering, conditional headers, notify rule, restore under a concurrent edit, re-stamped overrides

/// Lets one edit from "another client" land on the original resource just before the new series is written, then
/// fails that write.
struct MeddlingTransport {
    server: Arc<FakeCalDavServer>,
}

#[async_trait]
impl HttpTransport for MeddlingTransport {
    async fn send(&self, request: HttpRequest) -> Outcome<HttpResponse> {
        if request.method == "PUT" && request.url.path().contains("uuid-2.ics") {
            self.server.store(
                "home",
                "weekly.ics",
                &weekly_ics_by(None).replace("Team sync", "Edited elsewhere"),
            );
            return Ok(HttpResponse::new(507, Default::default(), Vec::new()));
        }
        self.server.send(request).await
    }
}

#[tokio::test]
async fn the_head_is_written_first_under_if_match_and_the_tail_under_if_none_match() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let source = h.source().await;
    let original = h.server.etag("home", "weekly.ics").unwrap();
    let target = occurrence(&source, 22).await;
    h.server.clear_log();
    split(&source, &target, &title("x")).await.unwrap();
    let puts = h.server.requests("PUT");
    assert_eq!(puts.len(), 2);
    assert!(
        puts[0].url.path().ends_with("weekly.ics")
            && puts[0].headers.get("If-Match") == Some(&original)
            && !puts[0].headers.contains_key("If-None-Match")
    );
    assert!(
        puts[1].url.path().ends_with("uuid-2.ics")
            && puts[1].headers.get("If-None-Match").map(String::as_str) == Some("*")
            && !puts[1].headers.contains_key("If-Match")
    );
}

#[tokio::test]
async fn the_restore_goes_back_under_the_truncations_etag() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let source = h.source().await;
    let original = h.server.etag("home", "weekly.ics").unwrap();
    let target = occurrence(&source, 22).await;
    h.server.fail("PUT", "uuid-2.ics", 507);
    h.server.clear_log();
    assert_eq!(
        split(&source, &target, &title("x")).await.unwrap_err(),
        Failure::Source(SourceError::Server { status: 507 })
    );
    let puts = h.server.requests("PUT");
    assert_eq!(puts.len(), 3);
    let restore_match = puts.last().unwrap().headers.get("If-Match").unwrap();
    // The truncation's ETag, not the one read at the start.
    assert_ne!(restore_match, &original);
    let restored = resource(&h, "weekly.ics");
    assert_eq!(
        master_value(&restored, "RRULE").as_deref(),
        Some("FREQ=WEEKLY;BYDAY=TU")
    );
    assert_eq!(restored.overrides().len(), 1);
    // Attendees ignore an update that is not newer than the truncation they already hold: SEQUENCE 0 -> 1 (cut) -> 2 (restore).
    assert_eq!(master_value(&restored, "SEQUENCE").as_deref(), Some("2"));
    assert_ne!(
        master_value(&restored, "DTSTAMP").as_deref(),
        Some("20260901T000000Z")
    );
    assert!(master_value(&restored, "LAST-MODIFIED").is_some());
}

#[tokio::test]
async fn a_restore_is_never_forced_over_an_edit_made_meanwhile() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let source = h
        .source_via(Arc::new(MeddlingTransport {
            server: h.server.clone(),
        }))
        .await;
    let target = occurrence(&source, 22).await;
    match split(&source, &target, &title("x")).await {
        Err(Failure::Write(WriteError::Partial(message))) => {
            assert!(message.contains("not restored"))
        }
        other => panic!("expected partial, got {other:?}"),
    }
    assert!(
        h.server
            .body("home", "weekly.ics")
            .unwrap()
            .contains("Edited elsewhere")
    );
    assert_eq!(h.server.names("home"), ["weekly.ics"]);
}

/// The account organizes the series and invites a guest.
fn organized_by_me_ics() -> String {
    weekly_ics_by(Some("mailto:me@icloud.test")).replace(
        "ATTENDEE;CN=Me;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:me@icloud.test",
        "ATTENDEE;CN=Guest:mailto:guest@example.test",
    )
}

#[tokio::test]
async fn a_split_that_tells_others_is_refused_before_any_request_unless_notify_is_all() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &organized_by_me_ics());
    let source = h.source().await;
    let target = occurrence(&source, 22).await;
    for policy in [NotifyPolicy::None, NotifyPolicy::ExternalOnly] {
        h.server.clear_log();
        expect_write_error(
            WriteError::Unsupported {
                fields: [EventField::Attendees].into(),
            },
            source
                .update(
                    &id(&target),
                    &title("x"),
                    RecurrenceScope::ThisAndFollowing,
                    policy,
                )
                .await,
        );
        assert!(h.server.requests("PUT").is_empty());
    }
    source
        .update(
            &id(&target),
            &title("x"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::All,
        )
        .await
        .unwrap();
    assert_eq!(h.server.names("home"), ["uuid-2.ics", "weekly.ics"]);
}

#[tokio::test]
async fn carried_over_exceptions_are_restamped_under_the_new_uid() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let source = h.source().await;
    split(&source, &occurrence(&source, 15).await, &title("New sync"))
        .await
        .unwrap();
    let tail = resource(&h, "uuid-2.ics");
    let carried = tail.overrides()[0].clone();
    let master = tail.master().unwrap().clone();
    let value = |c: &calendar_ical::ICalComponent, n: &str| c.property(n).map(|p| p.value.clone());
    assert_eq!(value(&carried, "DTSTAMP"), value(&master, "DTSTAMP"));
    assert_ne!(
        value(&carried, "DTSTAMP").as_deref(),
        Some("20260901T000000Z")
    );
    assert_eq!(
        value(&carried, "LAST-MODIFIED"),
        value(&master, "LAST-MODIFIED")
    );
}

// MARK: Fix round 1: unclear failures, all-day guard, organizer guard

type Answer = Outcome<Option<HttpResponse>>;
type Hook = Arc<
    dyn Fn(
            HttpRequest,
            Arc<FakeCalDavServer>,
        ) -> std::pin::Pin<Box<dyn Future<Output = Answer> + Send>>
        + Send
        + Sync,
>;

/// Wraps the fake server; the hook may answer a request itself (or fail) before the server sees it, or call
/// `server.send` and change what comes back. `None` passes the request through.
struct ScriptedTransport {
    server: Arc<FakeCalDavServer>,
    hook: Hook,
}

#[async_trait]
impl HttpTransport for ScriptedTransport {
    async fn send(&self, request: HttpRequest) -> Outcome<HttpResponse> {
        if let Some(answer) = (self.hook)(request.clone(), self.server.clone()).await? {
            return Ok(answer);
        }
        self.server.send(request).await
    }
}

fn scripted<F, Fut>(h: &CalDavHarness, hook: F) -> Arc<ScriptedTransport>
where
    F: Fn(HttpRequest, Arc<FakeCalDavServer>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Answer> + Send + 'static,
{
    Arc::new(ScriptedTransport {
        server: h.server.clone(),
        hook: Arc::new(move |request, server| Box::pin(hook(request, server))),
    })
}

fn is_put(request: &HttpRequest, name: &str) -> bool {
    request.method == "PUT" && request.url.path().ends_with(name)
}

fn is_get(request: &HttpRequest, name: &str) -> bool {
    request.method == "GET" && request.url.path().ends_with(name)
}

/// Counts requests so a hook can act on the n-th one.
#[derive(Default)]
struct Count(Mutex<i64>);

impl Count {
    fn next(&self) -> i64 {
        let mut value = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        *value += 1;
        *value
    }

    fn current(&self) -> i64 {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn network(message: &str) -> Failure {
    Failure::Source(SourceError::Network(message.to_string()))
}

async fn series_starts(source: &CalDavCalendarSource) -> Vec<Timestamp> {
    starts(source, autumn()).await
}

fn unique(starts: &[Timestamp]) -> bool {
    let set: std::collections::BTreeSet<_> = starts.iter().collect();
    set.len() == starts.len()
}

// 1. The tail's outcome is checked before anything is restored.

#[tokio::test]
async fn a_read_back_failure_after_the_new_series_was_stored_does_not_restore_the_head() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    // put() must GET the new resource back.
    h.server.configure(|s| s.sends_etag_on_put = false);
    let source = h.source().await;
    let target = occurrence(&source, 22).await;
    // The read-back fails once.
    h.server.fail("GET", "uuid-2.ics", 500);
    let tail = split(&source, &target, &title("Later")).await.unwrap();
    assert_eq!(tail.event_id, "uuid-2.ics");
    assert_eq!(h.server.names("home"), ["uuid-2.ics", "weekly.ics"]);
    assert!(
        master_value(&resource(&h, "weekly.ics"), "RRULE")
            .unwrap()
            .contains("UNTIL")
    );
    assert!(unique(&series_starts(&source).await));
}

#[tokio::test]
async fn a_lost_reply_to_the_new_series_that_went_through_keeps_the_split() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let transport = scripted(&h, |request, server| async move {
        if !is_put(&request, "uuid-2.ics") {
            return Ok(None);
        }
        // Applied ...
        server.send(request).await?;
        // ... but the reply never arrived.
        Err(network("connection lost"))
    });
    let source = h.source_via(transport).await;
    let target = occurrence(&source, 22).await;
    let tail = split(&source, &target, &title("Later")).await.unwrap();
    assert_eq!(tail.event_id, "uuid-2.ics");
    let all = series_starts(&source).await;
    assert!(unique(&all));
    assert_eq!(all.len(), 8);
}

#[tokio::test]
async fn a_cancelled_caller_whose_new_series_went_through_does_not_get_duplicates() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let transport = scripted(&h, |request, server| async move {
        if !is_put(&request, "uuid-2.ics") {
            return Ok(None);
        }
        server.send(request).await?;
        Err(Failure::Cancelled)
    });
    let source = h.source_via(transport).await;
    let target = occurrence(&source, 22).await;
    let _ = split(&source, &target, &title("Later")).await;
    assert!(unique(&series_starts(&source).await));
    assert_eq!(h.server.names("home"), ["uuid-2.ics", "weekly.ics"]);
}

#[tokio::test]
async fn an_unknown_outcome_of_the_new_series_is_partial_and_not_restored_blindly() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let transport = scripted(&h, |request, server| async move {
        if is_put(&request, "uuid-2.ics") {
            server.send(request).await?;
            return Err(network("connection lost"));
        }
        if is_get(&request, "uuid-2.ics") {
            return Err(network("still down"));
        }
        Ok(None)
    });
    let source = h.source_via(transport).await;
    let target = occurrence(&source, 22).await;
    h.server.clear_log();
    match split(&source, &target, &title("Later")).await {
        Err(Failure::Write(WriteError::Partial(message))) => assert!(message.contains("unknown")),
        other => panic!("expected partial, got {other:?}"),
    }
    // The truncation and the (lost) new series; no restore.
    assert_eq!(h.server.requests("PUT").len(), 2);
}

#[tokio::test]
async fn a_name_clash_on_the_new_series_restores_the_original_and_leaves_the_other_resource_alone()
{
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    h.server.store(
        "home",
        "uuid-2.ics",
        &single_ics_with("someone-elses", "Dentist", &[]),
    );
    let theirs = h.server.body("home", "uuid-2.ics").unwrap();
    let source = h.source().await;
    let mine = |events: Vec<CalendarEvent>| {
        events
            .into_iter()
            .filter(|e| e.uid.as_deref() == Some("weekly-uid"))
            .map(|e| e.start)
            .collect::<Vec<_>>()
    };
    let before = mine(source.events_in(september()).await.unwrap());
    let target = occurrence(&source, 22).await;
    assert_eq!(
        split(&source, &target, &title("x")).await.unwrap_err(),
        Failure::Source(SourceError::InvalidResponse(
            "a resource with the new series' name already exists".to_string()
        ))
    );
    assert_eq!(h.server.body("home", "uuid-2.ics").unwrap(), theirs);
    assert_eq!(mine(source.events_in(september()).await.unwrap()), before);
    assert_eq!(
        master_value(&resource(&h, "weekly.ics"), "RRULE").as_deref(),
        Some("FREQ=WEEKLY;BYDAY=TU")
    );
}

// 2. An unclear failure of the truncation is looked up.

#[tokio::test]
async fn a_read_back_failure_after_the_truncation_restores_the_series() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    h.server.configure(|s| s.sends_etag_on_put = false);
    let puts = Arc::new(Count::default());
    let counter = puts.clone();
    let transport = scripted(&h, move |request, server| {
        let counter = counter.clone();
        async move {
            // After the truncation was stored, the read-back GET (the first GET of the resource after it) fails once.
            if is_put(&request, "weekly.ics") {
                let answer = server.send(request).await?;
                if counter.next() == 1 {
                    server.fail("GET", "weekly.ics", 500);
                }
                return Ok(Some(answer));
            }
            Ok(None)
        }
    });
    let source = h.source_via(transport).await;
    let before = starts(&source, september()).await;
    let target = occurrence(&source, 22).await;
    assert_eq!(
        split(&source, &target, &title("x")).await.unwrap_err(),
        Failure::Source(SourceError::Server { status: 500 })
    );
    assert_eq!(h.server.names("home"), ["weekly.ics"]);
    assert_eq!(starts(&source, september()).await, before);
    let restored = resource(&h, "weekly.ics");
    assert_eq!(
        master_value(&restored, "RRULE").as_deref(),
        Some("FREQ=WEEKLY;BYDAY=TU")
    );
    assert_eq!(master_value(&restored, "SEQUENCE").as_deref(), Some("2"));
}

#[tokio::test]
async fn a_lost_reply_to_the_truncation_that_went_through_is_restored() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let puts = Arc::new(Count::default());
    let counter = puts.clone();
    let transport = scripted(&h, move |request, server| {
        let counter = counter.clone();
        async move {
            if !is_put(&request, "weekly.ics") || counter.next() != 1 {
                return Ok(None);
            }
            server.send(request).await?;
            Err(network("connection lost"))
        }
    });
    let source = h.source_via(transport).await;
    let before = starts(&source, september()).await;
    let target = occurrence(&source, 22).await;
    assert_eq!(
        split(&source, &target, &title("x")).await.unwrap_err(),
        network("connection lost")
    );
    assert_eq!(h.server.names("home"), ["weekly.ics"]);
    assert_eq!(starts(&source, september()).await, before);
}

#[tokio::test]
async fn a_lost_truncation_that_was_never_applied_changes_nothing() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let transport = scripted(&h, |request, _| async move {
        if is_put(&request, "weekly.ics") {
            return Err(network("connection lost"));
        }
        Ok(None)
    });
    let source = h.source_via(transport).await;
    let target = occurrence(&source, 22).await;
    let original = h.server.body("home", "weekly.ics").unwrap();
    h.server.clear_log();
    assert_eq!(
        split(&source, &target, &title("x")).await.unwrap_err(),
        network("connection lost")
    );
    assert_eq!(h.server.body("home", "weekly.ics").unwrap(), original);
    // The hook swallowed the only PUT; nothing was restored.
    assert!(h.server.requests("PUT").is_empty());
}

#[tokio::test]
async fn a_lost_truncation_reply_over_an_edit_by_another_client_is_partial() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let transport = scripted(&h, |request, server| async move {
        if !is_put(&request, "weekly.ics") {
            return Ok(None);
        }
        server.store(
            "home",
            "weekly.ics",
            &weekly_ics_by(None).replace("Team sync", "Edited elsewhere"),
        );
        Err(network("connection lost"))
    });
    let source = h.source_via(transport).await;
    let target = occurrence(&source, 22).await;
    assert!(matches!(
        split(&source, &target, &title("x")).await,
        Err(Failure::Write(WriteError::Partial(_)))
    ));
    assert!(
        h.server
            .body("home", "weekly.ics")
            .unwrap()
            .contains("Edited elsewhere")
    );
}

#[tokio::test]
async fn a_lost_truncation_reply_that_cannot_be_looked_up_is_partial() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let puts = Arc::new(Count::default());
    let counter = puts.clone();
    let transport = scripted(&h, move |request, server| {
        let counter = counter.clone();
        async move {
            if is_put(&request, "weekly.ics") {
                counter.next();
                server.send(request).await?;
                return Err(network("connection lost"));
            }
            // Once the truncation was sent, the resource can no longer be looked at.
            if is_get(&request, "weekly.ics") && counter.current() > 0 {
                return Err(network("still down"));
            }
            Ok(None)
        }
    });
    let source = h.source_via(transport).await;
    let target = occurrence(&source, 22).await;
    match split(&source, &target, &title("x")).await {
        Err(Failure::Write(WriteError::Partial(message))) => assert!(message.contains("unknown")),
        other => panic!("expected partial, got {other:?}"),
    }
}

// 3. No switch between all-day and timed when exceptions would be carried.

#[tokio::test]
async fn a_split_that_switches_to_all_day_is_refused_when_exceptions_would_be_carried() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let source = h.source().await;
    // The series has an exception at the 15th.
    let target = occurrence(&source, 15).await;
    let all_day = calendar_core::write::event_draft::EventTiming::new(
        pt(2026, 9, 15, 0, 0),
        pt(2026, 9, 16, 0, 0),
        Some(pacific()),
        true,
    );
    h.server.clear_log();
    expect_write_error(
        WriteError::Unsupported {
            fields: [EventField::Timing].into(),
        },
        split(&source, &target, &hand_patch(|p| p.timing = Some(all_day))).await,
    );
    assert!(h.server.requests("PUT").is_empty());
}

// 5. Only the organizer (or nobody) splits.

#[tokio::test]
async fn an_attendee_cannot_split_the_organizers_series() {
    let h = CalDavHarness::new();
    // Organized by boss@example.test, the account is invited.
    h.server.store("home", "weekly.ics", &weekly_ics());
    let source = h.source().await;
    let target = occurrence(&source, 22).await;
    h.server.clear_log();
    expect_write_error(
        WriteError::Unsupported {
            fields: [EventField::Attendees].into(),
        },
        source
            .update(
                &id(&target),
                &title("x"),
                RecurrenceScope::ThisAndFollowing,
                NotifyPolicy::All,
            )
            .await,
    );
    assert!(h.server.requests("PUT").is_empty());
    assert_eq!(h.server.names("home"), ["weekly.ics"]);
}

// MARK: Fix round 2: our truncation is recognised on every component

#[tokio::test]
async fn an_edit_to_a_kept_override_after_a_lost_truncation_reply_is_not_restored_over() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let puts = Arc::new(Count::default());
    let counter = puts.clone();
    let transport = scripted(&h, move |request, server| {
        let counter = counter.clone();
        async move {
            if !is_put(&request, "weekly.ics") || counter.next() != 1 {
                return Ok(None);
            }
            let landed = server.send(request).await?;
            assert!(landed.status == 204 || landed.status == 201);
            // Our truncation is on the server; before the caller looks again another client edits the kept override at the 15th.
            let mut body = server.body("home", "weekly.ics").unwrap();
            body = body.replace("Team sync (moved)", "Edited by another client");
            // A real client stamps its edit (only the override carries this DTSTAMP and is followed by its RECURRENCE-ID).
            body = body.replace(
                "DTSTAMP:20260901T000000Z\r\nRECURRENCE-ID",
                "DTSTAMP:20260922T000000Z\r\nRECURRENCE-ID",
            );
            server.store("home", "weekly.ics", &body);
            Err(network("connection lost"))
        }
    });
    let source = h.source_via(transport).await;
    let target = occurrence(&source, 22).await;
    h.server.clear_log();
    assert!(matches!(
        split(&source, &target, &title("x")).await,
        Err(Failure::Write(WriteError::Partial(_)))
    ));
    assert!(
        h.server
            .body("home", "weekly.ics")
            .unwrap()
            .contains("Edited by another client")
    );
    // Only our truncation; nothing was restored over the edit.
    assert_eq!(h.server.requests("PUT").len(), 1);
    assert_eq!(h.server.names("home"), ["weekly.ics"]);
}
