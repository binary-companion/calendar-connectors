//! Port of `WriteSourceTests.swift`.

use calendar_core::failure::Failure;
use calendar_core::http::{HttpRequest, HttpResponse};
use calendar_core::model::{AttendeeRole, CalendarDescriptor, CalendarService, ResponseStatus};
use calendar_core::time::{Zone, add_seconds};
use calendar_core::write::event_draft::{AttendeeDraft, EventDraft, EventTiming};
use calendar_core::write::event_patch::{AttendeeChanges, EventPatch, FieldUpdate};
use calendar_core::write::types::{
    EventField, EventRef, NotifyPolicy, RecurrenceScope, WriteError,
};
use calendar_core::write::writable_source::WritableCalendarSource;
use jiff::Timestamp;
use serde_json::json;

use super::support::{
    SourceHarness, body_json, decode_event, expect_write_error, graph_error, graph_error_message,
    graph_event, ok, status,
};
use crate::event_mapper::GraphEventMapper;
use calendar_test_support::hand_patch;

const EVENT_PATH: &str = "cal1/events/ev1";

fn la() -> Zone {
    Zone::named("America/Los_Angeles").unwrap()
}

/// 2026-09-25 10:00 in Los Angeles.
fn ten_am() -> Timestamp {
    Timestamp::from_second(1_790_355_600).unwrap()
}

fn draft(uid: Option<&str>) -> EventDraft {
    let mut draft = EventDraft::new(
        "Standup",
        EventTiming::new(ten_am(), add_seconds(ten_am(), 1800.0), Some(la()), false),
    );
    draft.uid = uid.map(str::to_string);
    draft
}

fn sent(h: &SourceHarness, method: &str, matching: &str) -> Vec<HttpRequest> {
    h.transport
        .requests_matching(matching)
        .into_iter()
        .filter(|request| request.method == method)
        .collect()
}

fn title_patch(title: &str) -> EventPatch {
    hand_patch(|p| p.title = Some(title.to_string()))
}

fn series_ref(event: &str, series: &str, start: Timestamp) -> EventRef {
    let mut reference = EventRef::new("cal1", event).with_series_id(Some(series));
    reference.original_start = Some(start);
    reference
}

fn no_content() -> HttpResponse {
    HttpResponse::new(204, Default::default(), Vec::new())
}

fn accepted() -> HttpResponse {
    HttpResponse::new(202, Default::default(), Vec::new())
}

#[tokio::test]
async fn create_posts_the_body_and_returns_the_stored_event() {
    let h = SourceHarness::new().await;
    h.transport.route(
        "cal1/events",
        vec![status(graph_event("new", json!({})), 201)],
    );
    let event = h
        .source
        .create(&draft(None), "cal1", NotifyPolicy::All)
        .await
        .unwrap();
    assert!(event.event_id == "new" && event.calendar_id == "cal1");
    let posts = sent(&h, "POST", "cal1/events");
    assert!(posts.len() == 1 && body_json(&posts[0])["subject"] == "Standup");
    assert!(
        posts[0]
            .headers
            .get("Prefer")
            .is_some_and(|prefer| prefer.contains("outlook.timezone=\"Pacific Standard Time\""))
    );
}

#[tokio::test]
async fn create_finds_an_existing_copy_by_ical_uid() {
    let h = SourceHarness::new().await;
    h.transport.route(
        "cal1/events",
        vec![ok(
            json!({"value": [graph_event("old", json!({"iCalUId": "uid-1"}))]}),
        )],
    );
    match h
        .source
        .create(&draft(Some("uid-1")), "cal1", NotifyPolicy::None)
        .await
    {
        Err(Failure::Write(WriteError::AlreadyExists(existing))) => {
            assert_eq!(existing.event_id, "old");
        }
        other => panic!("expected alreadyExists, got {other:?}"),
    }
    let filter = h
        .transport
        .requests_matching("$filter")
        .first()
        .map(|request| request.url.as_str().to_string())
        .unwrap_or_default();
    assert!(filter.contains("iCalUId%20eq%20'uid-1'") || filter.contains("iCalUId eq 'uid-1'"));
    assert!(sent(&h, "POST", "cal1/events").is_empty());
}

#[tokio::test]
async fn create_refuses_a_read_only_or_missing_calendar() {
    let h = SourceHarness::new().await;
    expect_write_error(
        WriteError::Forbidden(Some("read-only calendar".to_string())),
        h.source
            .create(&draft(None), "cal2", NotifyPolicy::None)
            .await,
    );
    expect_write_error(
        WriteError::NotFound,
        h.source
            .create(&draft(None), "nope", NotifyPolicy::None)
            .await,
    );
    assert!(sent(&h, "POST", "events").is_empty());
}

#[tokio::test]
async fn create_validates_before_any_request() {
    let h = SourceHarness::new().await;
    let bad = EventDraft::new("x", EventTiming::new(ten_am(), ten_am(), Some(la()), false));
    expect_write_error(
        WriteError::Invalid("end must be after start".to_string()),
        h.source.create(&bad, "cal1", NotifyPolicy::None).await,
    );
}

#[tokio::test]
async fn update_patches_only_the_changed_fields() {
    let h = SourceHarness::new().await;
    h.transport
        .route(EVENT_PATH, vec![ok(graph_event("ev1", json!({})))]);
    let mut reference = EventRef::new("cal1", "ev1");
    reference.version = Some("ck1".to_string());
    h.source
        .update(
            &reference,
            &title_patch("New"),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let patches = sent(&h, "PATCH", EVENT_PATH);
    assert_eq!(patches.len(), 1);
    assert_eq!(
        body_json(&patches[0]).keys().collect::<Vec<_>>(),
        ["subject"]
    );
}

#[tokio::test]
async fn an_empty_patch_sends_nothing() {
    let h = SourceHarness::new().await;
    h.transport
        .route(EVENT_PATH, vec![ok(graph_event("ev1", json!({})))]);
    let event = h
        .source
        .update(
            &EventRef::new("cal1", "ev1"),
            &EventPatch::default(),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert_eq!(event.event_id, "ev1");
    assert!(sent(&h, "PATCH", EVENT_PATH).is_empty());
}

#[tokio::test]
async fn a_stale_version_conflicts_only_on_overlapping_fields() {
    let h = SourceHarness::new().await;
    // Someone changed the title after the caller read version ck0.
    h.transport.route(
        EVENT_PATH,
        vec![ok(graph_event(
            "ev1",
            json!({"subject": "Theirs", "changeKey": "ck9"}),
        ))],
    );
    let mut calendar = CalendarDescriptor::new("cal1", "c", CalendarService::microsoft());
    calendar.time_zone = Some(la());
    let base = GraphEventMapper::map(
        &decode_event(graph_event("ev1", json!({"changeKey": "ck0"}))),
        &calendar,
        Some("me@x.com"),
        Some("s"),
    )
    .unwrap();
    let mut edited = base.clone();
    edited.title = "Mine".to_string();
    let reference = EventRef::from_event(&base);
    expect_write_error(
        WriteError::Conflict {
            fields: [EventField::Title].into(),
        },
        h.source
            .update(
                &reference,
                &EventPatch::diff(&base, &edited),
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
    );
    let mut relocated = base.clone();
    relocated.location = Some("Room 9".to_string());
    h.source
        .update(
            &reference,
            &EventPatch::diff(&base, &relocated),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert_eq!(sent(&h, "PATCH", EVENT_PATH).len(), 1);
}

#[tokio::test]
async fn attendee_edits_resend_the_current_list_with_the_change() {
    let h = SourceHarness::new().await;
    h.transport
        .route(EVENT_PATH, vec![ok(graph_event("ev1", json!({})))]);
    let patch = hand_patch(|p| {
        p.attendees = Some(AttendeeChanges::new(
            vec![AttendeeDraft::new(
                "new@x.com",
                None,
                AttendeeRole::Required,
            )],
            vec!["opt@x.com".to_string()],
        ));
    });
    let mut reference = EventRef::new("cal1", "ev1");
    reference.version = Some("ck1".to_string());
    h.source
        .update(
            &reference,
            &patch,
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let body = body_json(&sent(&h, "PATCH", EVENT_PATH)[0]);
    let addresses: Vec<String> = body["attendees"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|a| a["emailAddress"]["address"].as_str().map(str::to_string))
        .collect();
    assert_eq!(addresses, ["Me@X.com", "room@x.com", "new@x.com"]);
}

#[tokio::test]
async fn a_series_wide_write_targets_the_master_and_a_single_write_on_the_master_is_refused() {
    let h = SourceHarness::new().await;
    h.transport.route(
        "cal1/events/master1",
        vec![ok(graph_event("master1", json!({"type": "seriesMaster"})))],
    );
    let occurrence = series_ref("occ1", "master1", ten_am());
    h.source
        .update(
            &occurrence,
            &title_patch("All"),
            RecurrenceScope::AllInSeries,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let to_master = sent(&h, "PATCH", "cal1/events/master1").len();
    let to_occurrence = sent(&h, "PATCH", "cal1/events/occ1").len();
    assert!(to_master == 1 && to_occurrence == 0);
    let master = EventRef::new("cal1", "master1").with_series_id(Some("master1"));
    expect_write_error(
        WriteError::Invalid(
            "this is a recurring series; use .allInSeries or read the occurrence first".to_string(),
        ),
        h.source
            .update(
                &master,
                &title_patch("x"),
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
    );
}

fn one_hour_timing(start: Timestamp) -> EventTiming {
    EventTiming::new(start, add_seconds(start, 3600.0), Some(la()), false)
}

#[tokio::test]
async fn a_series_wide_time_change_from_a_later_occurrence_is_refused() {
    let h = SourceHarness::new().await;
    h.transport.route(
        "cal1/events/master1",
        vec![ok(graph_event("master1", json!({"type": "seriesMaster"})))],
    );
    let later = series_ref("occ2", "master1", add_seconds(ten_am(), 7.0 * 86_400.0));
    let patch = hand_patch(|p| {
        p.timing = Some(one_hour_timing(ten_am()));
    });
    expect_write_error(
        WriteError::Unsupported {
            fields: [EventField::Timing].into(),
        },
        h.source
            .update(
                &later,
                &patch,
                RecurrenceScope::AllInSeries,
                NotifyPolicy::None,
            )
            .await,
    );
    assert!(sent(&h, "PATCH", "cal1/events").is_empty());
}

#[tokio::test]
async fn moving_the_first_occurrence_resends_the_range_with_the_new_start_date() {
    let h = SourceHarness::new().await;
    let recurrence = json!({
        "pattern": {"type": "weekly", "interval": 1, "daysOfWeek": ["friday"], "firstDayOfWeek": "monday"},
        "range": {"type": "noEnd", "startDate": "2026-09-25", "recurrenceTimeZone": "Pacific Standard Time"},
    });
    h.transport.route(
        "cal1/events/master1",
        vec![ok(graph_event(
            "master1",
            json!({"type": "seriesMaster", "recurrence": recurrence}),
        ))],
    );
    let first = series_ref("master1", "master1", ten_am());
    let next_day = add_seconds(ten_am(), 86_400.0);
    let patch = hand_patch(|p| {
        p.timing = Some(EventTiming::new(
            next_day,
            add_seconds(next_day, 1800.0),
            Some(la()),
            false,
        ));
    });
    h.source
        .update(
            &first,
            &patch,
            RecurrenceScope::AllInSeries,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let body = body_json(&sent(&h, "PATCH", "cal1/events/master1")[0]);
    assert_eq!(body["recurrence"]["range"]["startDate"], "2026-09-26");
}

#[tokio::test]
async fn delete_removes_the_instance_or_the_whole_series() {
    let h = SourceHarness::new().await;
    h.transport.route("cal1/events/", vec![no_content()]);
    let reference = series_ref("occ1", "master1", ten_am());
    h.source
        .delete(
            &reference,
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    h.source
        .delete(&reference, RecurrenceScope::AllInSeries, NotifyPolicy::None)
        .await
        .unwrap();
    let deletes: Vec<String> = sent(&h, "DELETE", "cal1/events/")
        .iter()
        .map(|request| request.url.path())
        .collect();
    assert_eq!(
        deletes,
        [
            "/v1.0/me/calendars/cal1/events/occ1",
            "/v1.0/me/calendars/cal1/events/master1"
        ]
    );
}

#[tokio::test]
async fn respond_honors_the_notify_policy_and_reads_the_event_back() {
    let h = SourceHarness::new().await;
    h.transport.route(
        EVENT_PATH,
        vec![ok(graph_event(
            "ev1",
            json!({"responseStatus": {"response": "accepted"}}),
        ))],
    );
    h.transport
        .route(&format!("{EVENT_PATH}/accept"), vec![accepted()]);
    h.transport
        .route(&format!("{EVENT_PATH}/decline"), vec![accepted()]);
    let reference = EventRef::new("cal1", "ev1");
    let event = h
        .source
        .respond(
            &reference,
            ResponseStatus::Accepted,
            RecurrenceScope::ThisInstance,
            NotifyPolicy::All,
        )
        .await
        .unwrap();
    assert_eq!(event.event_id, "ev1");
    h.source
        .respond(
            &reference,
            ResponseStatus::Declined,
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let accept = &sent(&h, "POST", &format!("{EVENT_PATH}/accept"))[0];
    let decline = &sent(&h, "POST", &format!("{EVENT_PATH}/decline"))[0];
    assert!(
        body_json(accept)["sendResponse"] == true && body_json(decline)["sendResponse"] == false
    );
    expect_write_error(
        WriteError::Invalid("a response must be accepted, tentative or declined".to_string()),
        h.source
            .respond(
                &reference,
                ResponseStatus::NeedsAction,
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
    );
    let series = series_ref("ev1", "master1", ten_am());
    expect_write_error(
        WriteError::Unsupported {
            fields: [EventField::Attendees].into(),
        },
        h.source
            .respond(
                &series,
                ResponseStatus::Accepted,
                RecurrenceScope::ThisAndFollowing,
                NotifyPolicy::None,
            )
            .await,
    );
}

#[tokio::test]
async fn service_errors_become_write_errors() {
    let h = SourceHarness::new().await;
    let reference = EventRef::new("cal1", "ev1");
    h.transport
        .route(EVENT_PATH, vec![graph_error("ErrorItemNotFound", 404)]);
    expect_write_error(
        WriteError::NotFound,
        h.source
            .update(
                &reference,
                &title_patch("x"),
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
    );
    h.transport
        .route(EVENT_PATH, vec![graph_error("ErrorAccessDenied", 403)]);
    expect_write_error(
        WriteError::Forbidden(None),
        h.source
            .delete(
                &reference,
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
    );
    h.transport.route(
        EVENT_PATH,
        vec![graph_error_message("ErrorInvalidRequest", "bad", 400)],
    );
    expect_write_error(
        WriteError::Invalid("bad".to_string()),
        h.source
            .delete(
                &reference,
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
    );
    h.transport.route(
        EVENT_PATH,
        vec![ok(graph_event("ev1", json!({"isCancelled": true})))],
    );
    expect_write_error(
        WriteError::NotFound,
        h.source
            .update(
                &reference,
                &title_patch("x"),
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
    );
}

#[tokio::test]
async fn an_unwritable_field_is_refused_before_any_request() {
    let h = SourceHarness::new().await;
    let patch = hand_patch(|p| {
        p.reminders = FieldUpdate::Clear;
    });
    expect_write_error(
        WriteError::Unsupported {
            fields: [EventField::Reminders].into(),
        },
        h.source
            .update(
                &EventRef::new("cal1", "ev1"),
                &patch,
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
    );
    assert!(h.transport.requests_matching("cal1/events").is_empty());
}
