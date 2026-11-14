//! Port of `WriteSourceTests.swift`.

use calendar_core::failure::Failure;
use calendar_core::http::HttpRequest;
use calendar_core::model::{
    Attendee, AttendeeRole, Availability, CalendarEvent, ResponseStatus, Visibility,
};
use calendar_core::recurrence::rule::{Frequency, RecurrenceRule, RuleEnd};
use calendar_core::source_types::{CalendarSource, ProvidedField, SourceCapabilities, SourceError};
use calendar_core::time::Zone;
use calendar_core::write::event_draft::{AttendeeDraft, EventDraft, EventTiming};
use calendar_core::write::event_patch::{AttendeeChanges, EventEdit, EventPatch, FieldUpdate};
use calendar_core::write::types::{
    ConferenceRequest, EventField, EventRef, NotifyPolicy, RecurrenceScope, WriteError,
};
use calendar_core::write::writable_source::WritableCalendarSource;
use calendar_test_support::fake_transport::json_response;
use calendar_test_support::hand_patch;
use calendar_test_support::provided_fields_conformance::ProvidedFieldsConformance;
use serde_json::{Value, json};

use super::support::{
    Harness, assert_write_error, bare, body_json, google_error, google_error_with_headers,
    google_event, google_event_full, instant, ok, one_hour_window, text,
};

const CAL: &str = "me@x.com";
const CAL_PATH: &str = "calendars/me%40x.com/events";

fn utc() -> Zone {
    Zone::named("UTC").unwrap()
}

fn timing() -> EventTiming {
    EventTiming::new(
        instant("2026-09-21T10:00:00Z"),
        instant("2026-09-21T10:30:00Z"),
        Some(utc()),
        false,
    )
}

fn base() -> CalendarEvent {
    base_with("Standup", "e1", Vec::new())
}

fn base_with(title: &str, version: &str, attendees: Vec<Attendee>) -> CalendarEvent {
    let mut event = CalendarEvent::new(
        "ev1",
        CAL,
        title,
        instant("2026-09-21T10:00:00Z"),
        instant("2026-09-21T10:30:00Z"),
    );
    event.time_zone = utc();
    event.attendees = attendees;
    event.version = Some(version.to_string());
    event
}

fn eref(event_id: &str, version: Option<&str>) -> EventRef {
    let mut reference = EventRef::new(CAL, event_id);
    reference.version = version.map(str::to_string);
    reference
}

fn occurrence(
    event_id: &str,
    version: &str,
    series_id: &str,
    original_start: Option<&str>,
) -> EventRef {
    let mut reference = eref(event_id, Some(version)).with_series_id(Some(series_id));
    reference.original_start = original_start.map(instant);
    reference
}

fn title_patch(title: &str) -> EventPatch {
    hand_patch(|p| p.title = Some(title.to_string()))
}

fn weekly() -> RecurrenceRule {
    let mut rule = RecurrenceRule::new(Frequency::Weekly);
    rule.end = RuleEnd::Count(4);
    rule
}

fn methods(requests: &[HttpRequest]) -> Vec<String> {
    requests.iter().map(|r| r.method.clone()).collect()
}

fn header(request: &HttpRequest, name: &str) -> Option<String> {
    request.headers.get(name).cloned()
}

fn ids(value: &Value) -> Vec<Option<String>> {
    value
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|a| a["email"].as_str().map(str::to_string))
        .collect()
}

#[tokio::test]
async fn google_declares_its_write_capabilities() {
    let h = Harness::new().await;
    let c = h.source.capabilities();
    assert!(
        c.can_write && c.can_edit_attendees && c.can_respond_to_invite && c.controls_notifications
    );
    assert!(
        c.writable_fields == EventField::ALL.iter().copied().collect()
            && c.recurrence_scopes == RecurrenceScope::ALL.iter().copied().collect()
    );
    assert!(h.source.as_writable().is_some());
}

#[tokio::test]
async fn create_posts_to_the_calendar_and_maps_the_result() {
    let h = Harness::new().await;
    h.transport.route(
        CAL_PATH,
        vec![ok(google_event_full("new1", "e9", "Sync", json!({})))],
    );
    let mut draft = EventDraft::new("Sync", timing());
    draft.attendees = vec![AttendeeDraft::new(
        "bob@x.com",
        None,
        AttendeeRole::Required,
    )];
    draft.conference = ConferenceRequest::Generate;
    let created = h
        .source
        .create(&draft, CAL, NotifyPolicy::All)
        .await
        .unwrap();
    let post = h
        .transport
        .requests_matching(CAL_PATH)
        .last()
        .cloned()
        .unwrap();
    assert!(
        post.method == "POST"
            && post.url.as_str().contains("sendUpdates=all")
            && post.url.as_str().contains("conferenceDataVersion=1")
    );
    assert_eq!(body_json(&post)["summary"], "Sync");
    assert!(
        created.event_id == "new1"
            && created.version.as_deref() == Some("e9")
            && created.source_id.as_deref() == Some("google-c1")
            && created.calendar_id == CAL
    );
}

#[tokio::test]
async fn writes_to_a_read_only_calendar_are_forbidden_before_any_event_request() {
    let h = Harness::new().await;
    assert_write_error(
        h.source
            .create(
                &EventDraft::new("T", timing()),
                "team@group.calendar.google.com",
                NotifyPolicy::None,
            )
            .await,
        WriteError::Forbidden(Some("read-only calendar".to_string())),
    );
    assert_write_error(
        h.source
            .create(&EventDraft::new("T", timing()), "nope", NotifyPolicy::None)
            .await,
        WriteError::NotFound,
    );
    assert!(h.transport.requests_matching("/events").is_empty());
}

#[tokio::test]
async fn create_refuses_an_invalid_draft_before_any_request() {
    let h = Harness::new().await;
    let mut draft = EventDraft::new("T", timing());
    draft.reminders = Some(vec![calendar_core::model::Reminder::with_minutes_before(
        -1,
    )]);
    assert_write_error(
        h.source.create(&draft, CAL, NotifyPolicy::None).await,
        WriteError::Invalid("reminder minutes must not be negative".to_string()),
    );
    assert!(h.transport.requests().is_empty());
    // A rule only Google's mapper enforces is also caught before the calendar lookup.
    draft.reminders = Some(vec![calendar_core::model::Reminder::with_minutes_before(
        40321,
    )]);
    assert_write_error(
        h.source.create(&draft, CAL, NotifyPolicy::None).await,
        WriteError::Invalid("reminder minutes must be between 0 and 40320".to_string()),
    );
    assert!(h.transport.requests().is_empty());
}

#[tokio::test]
async fn update_sends_only_the_changed_fields_with_if_match() {
    let h = Harness::new().await;
    h.transport.route(
        &format!("{CAL_PATH}/ev1"),
        vec![ok(google_event_full("ev1", "e2", "New", json!({})))],
    );
    let updated = h
        .source
        .update(
            &eref("ev1", Some("e1")),
            &title_patch("New"),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let patch = h
        .transport
        .requests_matching(&format!("{CAL_PATH}/ev1"))
        .last()
        .cloned()
        .unwrap();
    assert!(
        patch.method == "PATCH"
            && header(&patch, "If-Match").as_deref() == Some("e1")
            && patch.url.as_str().contains("sendUpdates=none")
    );
    assert_eq!(body_json(&patch), json!({"summary": "New"}));
    assert!(updated.title == "New" && updated.version.as_deref() == Some("e2"));
}

#[tokio::test]
async fn a_stale_version_with_no_overlap_is_retried_on_the_fresh_etag() {
    let h = Harness::new().await;
    let mut edit = EventEdit::new(base());
    edit.event.title = "New".to_string();
    h.transport.route(
        &format!("{CAL_PATH}/ev1"),
        vec![
            google_error("conditionNotMet", "m", 412),
            // Someone else set the location.
            ok(google_event_full(
                "ev1",
                "e2",
                "Standup",
                json!({"location": "Elsewhere"}),
            )),
            ok(google_event_full(
                "ev1",
                "e3",
                "New",
                json!({"location": "Elsewhere"}),
            )),
        ],
    );
    let updated = h
        .source
        .update(
            &EventRef::from_event(&base()),
            &edit.patch(),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let sent = h.transport.requests_matching(&format!("{CAL_PATH}/ev1"));
    assert_eq!(methods(&sent), ["PATCH", "GET", "PATCH"]);
    assert!(
        header(&sent[0], "If-Match").as_deref() == Some("e1")
            && header(&sent[2], "If-Match").as_deref() == Some("e2")
    );
    assert!(updated.title == "New" && updated.location.as_deref() == Some("Elsewhere"));
    // The retry resends only what this edit changed, not the other writer's location.
    assert_eq!(body_json(&sent[2]), json!({"summary": "New"}));
}

#[tokio::test]
async fn a_stale_version_with_an_overlap_is_a_conflict() {
    let h = Harness::new().await;
    let mut edit = EventEdit::new(base());
    edit.event.title = "Mine".to_string();
    h.transport.route(
        &format!("{CAL_PATH}/ev1"),
        vec![
            google_error("conditionNotMet", "m", 412),
            ok(google_event_full("ev1", "e2", "Theirs", json!({}))),
        ],
    );
    assert_write_error(
        h.source
            .update(
                &EventRef::from_event(&base()),
                &edit.patch(),
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
        WriteError::Conflict {
            fields: [EventField::Title].into(),
        },
    );
}

#[tokio::test]
async fn attendee_changes_fetch_then_patch_the_full_array_with_the_fetched_etag() {
    let h = Harness::new().await;
    let attendees = json!([
        {"email": "me@x.com", "self": true, "responseStatus": "accepted"},
        {"email": "bob@x.com", "responseStatus": "accepted"},
    ]);
    h.transport.route(
        &format!("{CAL_PATH}/ev1"),
        vec![
            ok(google_event_full(
                "ev1",
                "e1",
                "Standup",
                json!({"attendees": attendees}),
            )),
            ok(google_event_full("ev1", "e2", "Standup", json!({}))),
        ],
    );
    let patch = hand_patch(|p| {
        p.attendees = Some(AttendeeChanges::new(
            vec![AttendeeDraft::new(
                "dee@x.com",
                None,
                AttendeeRole::Required,
            )],
            vec!["bob@x.com".to_string()],
        ));
    });
    h.source
        .update(
            &eref("ev1", Some("e1")),
            &patch,
            RecurrenceScope::ThisInstance,
            NotifyPolicy::ExternalOnly,
        )
        .await
        .unwrap();
    let sent = h.transport.requests_matching(&format!("{CAL_PATH}/ev1"));
    assert_eq!(methods(&sent), ["GET", "PATCH"]);
    assert!(
        header(&sent[1], "If-Match").as_deref() == Some("e1")
            && sent[1].url.as_str().contains("sendUpdates=externalOnly")
    );
    assert_eq!(
        ids(&body_json(&sent[1])["attendees"]),
        [Some("me@x.com".to_string()), Some("dee@x.com".to_string())]
    );
}

#[tokio::test]
async fn an_attendee_change_conflicts_when_someone_else_changed_the_same_attendee() {
    let h = Harness::new().await;
    let bob = Attendee::new(
        None,
        Some("bob@x.com"),
        AttendeeRole::Required,
        ResponseStatus::NeedsAction,
        false,
        false,
    );
    let mut edit = EventEdit::new(base_with("Standup", "e5", vec![bob]));
    edit.event.attendees[0].role = AttendeeRole::Resource;
    h.transport.route(
        &format!("{CAL_PATH}/ev1"),
        vec![ok(google_event_full(
            "ev1",
            "e6",
            "Standup",
            json!({"attendees": [{"email": "bob@x.com", "optional": true}]}),
        ))],
    );
    assert_write_error(
        h.source
            .update(
                &eref("ev1", Some("e5")),
                &edit.patch(),
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
        WriteError::Conflict {
            fields: [EventField::Attendees].into(),
        },
    );
    assert!(
        h.transport
            .requests_matching(&format!("{CAL_PATH}/ev1"))
            .iter()
            .all(|r| r.method == "GET")
    );
}

#[tokio::test]
async fn an_empty_patch_returns_the_base_without_any_request() {
    let h = Harness::new().await;
    let edit = EventEdit::new(base());
    let result = h
        .source
        .update(
            &EventRef::from_event(&base()),
            &edit.patch(),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    // Only the calendar list used for the access check.
    assert!(result == base() && h.transport.requests().len() == 1);
}

#[tokio::test]
async fn instance_and_series_scopes_target_the_instance_or_the_master() {
    let h = Harness::new().await;
    h.transport.route(
        &format!("{CAL_PATH}/master1"),
        vec![ok(google_event_full("master1", "m2", "All", json!({})))],
    );
    h.transport.route(
        &format!("{CAL_PATH}/master1_20260921T100000Z"),
        vec![ok(google_event_full(
            "master1_20260921T100000Z",
            "i2",
            "One",
            json!({}),
        ))],
    );
    let reference = occurrence(
        "master1_20260921T100000Z",
        "i1",
        "master1",
        Some("2026-09-21T10:00:00Z"),
    );
    h.source
        .update(
            &reference,
            &title_patch("One"),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    h.source
        .update(
            &reference,
            &title_patch("All"),
            RecurrenceScope::AllInSeries,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let instance = h
        .transport
        .requests_matching("master1_2026")
        .last()
        .cloned()
        .unwrap();
    let master = h
        .transport
        .requests_matching(&format!("{CAL_PATH}/master1?"))
        .last()
        .cloned()
        .unwrap();
    assert_eq!(header(&instance, "If-Match").as_deref(), Some("i1"));
    // An instance etag cannot lock the master.
    assert!(header(&master, "If-Match").is_none() && master.method == "PATCH");
}

#[tokio::test]
async fn respond_patches_only_the_self_attendee_with_the_fetched_etag() {
    let h = Harness::new().await;
    let attendees = json!([
        {"email": "me@x.com", "self": true, "responseStatus": "needsAction"},
        {"email": "bob@x.com", "responseStatus": "accepted"},
    ]);
    h.transport.route(
        &format!("{CAL_PATH}/ev1"),
        vec![
            ok(google_event_full(
                "ev1",
                "e4",
                "Standup",
                json!({"attendees": attendees}),
            )),
            ok(google_event_full("ev1", "e5", "Standup", json!({}))),
        ],
    );
    h.source
        .respond(
            &eref("ev1", Some("e1")),
            ResponseStatus::Declined,
            RecurrenceScope::ThisInstance,
            NotifyPolicy::All,
        )
        .await
        .unwrap();
    let patch = h
        .transport
        .requests_matching(&format!("{CAL_PATH}/ev1"))
        .last()
        .cloned()
        .unwrap();
    assert!(
        patch.method == "PATCH"
            && header(&patch, "If-Match").as_deref() == Some("e4")
            && patch.url.as_str().contains("sendUpdates=all")
    );
    let sent = body_json(&patch)["attendees"].clone();
    assert!(sent[0]["responseStatus"] == "declined" && sent[1]["responseStatus"] == "accepted");
    let mut series_ref = eref("ev1", None).with_series_id(Some("m"));
    series_ref.original_start = Some(instant("2026-09-21T10:00:00Z"));
    assert_write_error(
        h.source
            .respond(
                &series_ref,
                ResponseStatus::Accepted,
                RecurrenceScope::ThisAndFollowing,
                NotifyPolicy::None,
            )
            .await,
        WriteError::Unsupported {
            fields: [EventField::Attendees].into(),
        },
    );
}

#[tokio::test]
async fn delete_sends_delete_with_the_notify_policy_and_maps_gone_to_not_found() {
    let h = Harness::new().await;
    h.transport
        .route(&format!("{CAL_PATH}/ev1"), vec![bare(204)]);
    h.source
        .delete(
            &eref("ev1", None),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::All,
        )
        .await
        .unwrap();
    let request = h
        .transport
        .requests_matching(&format!("{CAL_PATH}/ev1"))
        .last()
        .cloned()
        .unwrap();
    assert!(request.method == "DELETE" && request.url.as_str().contains("sendUpdates=all"));
    h.transport.route(
        &format!("{CAL_PATH}/ev2"),
        vec![google_error("deleted", "m", 410)],
    );
    assert_write_error(
        h.source
            .delete(
                &eref("ev2", None),
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await
            .map(|()| 0),
        WriteError::NotFound,
    );
}

#[tokio::test]
async fn a_forbidden_write_is_reported_as_forbidden() {
    let h = Harness::new().await;
    h.transport.route(
        &format!("{CAL_PATH}/ev1"),
        vec![google_error("forbiddenForNonOrganizer", "m", 403)],
    );
    assert_write_error(
        h.source
            .update(
                &eref("ev1", Some("e1")),
                &title_patch("X"),
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
        WriteError::Forbidden(None),
    );
}

// MARK: Edge cases beyond the brief

#[tokio::test]
async fn a_request_that_cannot_be_built_is_refused_before_any_event_request() {
    let h = Harness::new().await;
    // An invalid patch is caught even though the attendee path would otherwise fetch first.
    let patch = hand_patch(|p| {
        p.reminders = FieldUpdate::Set(vec![
            calendar_core::model::Reminder::with_minutes_before(5);
            6
        ]);
        p.attendees = Some(AttendeeChanges::new(
            vec![AttendeeDraft::new(
                "dee@x.com",
                None,
                AttendeeRole::Required,
            )],
            Vec::new(),
        ));
    });
    assert_write_error(
        h.source
            .update(
                &eref("ev1", Some("e1")),
                &patch,
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
        WriteError::Invalid("Google allows at most 5 reminders".to_string()),
    );
    // An empty event id would address the events collection itself.
    assert_write_error(
        h.source
            .delete(
                &eref("", None),
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await
            .map(|()| 0),
        WriteError::Invalid("the event id must not be empty".to_string()),
    );
    assert_write_error(
        h.source
            .update(
                &eref("", Some("e1")),
                &title_patch("X"),
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
        WriteError::Invalid("the event id must not be empty".to_string()),
    );
    assert_write_error(
        h.source
            .respond(
                &eref("ev1", None),
                ResponseStatus::NeedsAction,
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
        WriteError::Invalid("cannot respond with needsAction".to_string()),
    );
    assert!(h.transport.requests_matching("/events").is_empty());
}

#[tokio::test]
async fn an_unreadable_event_response_is_an_invalid_response_not_a_cocoa_error() {
    let h = Harness::new().await;
    h.transport.route(
        &format!("{CAL_PATH}/ev1"),
        vec![
            text("<html>oops</html>", 200),
            json_response(&json!([1, 2, 3]), 200),
        ],
    );
    for _ in 0..2 {
        let result = h
            .source
            .respond(
                &eref("ev1", None),
                ResponseStatus::Accepted,
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await;
        assert_eq!(
            result.unwrap_err(),
            Failure::Source(SourceError::InvalidResponse(
                "google: unreadable event".to_string()
            ))
        );
    }
}

#[tokio::test]
async fn responding_to_a_deleted_event_is_not_found() {
    let h = Harness::new().await;
    h.transport.route(
        &format!("{CAL_PATH}/ev1"),
        vec![ok(google_event("ev1", json!({"status": "cancelled"})))],
    );
    assert_write_error(
        h.source
            .respond(
                &eref("ev1", None),
                ResponseStatus::Accepted,
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
        WriteError::NotFound,
    );
    assert!(
        h.transport
            .requests_matching(&format!("{CAL_PATH}/ev1"))
            .iter()
            .all(|r| r.method == "GET")
    );
}

#[tokio::test]
async fn respond_retries_on_the_fresh_etag_when_the_event_changes_between_fetch_and_patch() {
    let h = Harness::new().await;
    let attendees = json!([{"email": "me@x.com", "self": true, "responseStatus": "needsAction"}]);
    h.transport.route(
        &format!("{CAL_PATH}/ev1"),
        vec![
            ok(google_event_full(
                "ev1",
                "e4",
                "Standup",
                json!({"attendees": attendees}),
            )),
            google_error("conditionNotMet", "m", 412),
            ok(google_event_full(
                "ev1",
                "e5",
                "Standup",
                json!({"attendees": attendees}),
            )),
            ok(google_event_full("ev1", "e6", "Standup", json!({}))),
        ],
    );
    let result = h
        .source
        .respond(
            &eref("ev1", None),
            ResponseStatus::Accepted,
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let sent = h.transport.requests_matching(&format!("{CAL_PATH}/ev1"));
    assert_eq!(methods(&sent), ["GET", "PATCH", "GET", "PATCH"]);
    assert!(
        header(&sent[3], "If-Match").as_deref() == Some("e5")
            && result.version.as_deref() == Some("e6")
    );
}

#[tokio::test]
async fn a_quota_forbidden_write_is_rate_limited_not_a_permission_error() {
    let h = Harness::new().await;
    h.transport.route(
        &format!("{CAL_PATH}/ev1"),
        vec![google_error_with_headers(
            "quotaExceeded",
            403,
            &[("Retry-After", "30")],
        )],
    );
    let error = h
        .source
        .update(
            &eref("ev1", Some("e1")),
            &title_patch("X"),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error,
        Failure::Source(SourceError::RateLimited {
            retry_after: Some(30.0)
        })
    );
    h.transport.route(
        &format!("{CAL_PATH}/ev3"),
        vec![google_error("calendarUsageLimitsExceeded", "m", 403)],
    );
    let error = h
        .source
        .delete(
            &eref("ev3", None),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error,
        Failure::Source(SourceError::RateLimited { retry_after: None })
    );
    // Reads keep their behaviour.
    h.transport.route(
        "calendars/me%40x.com/events",
        vec![google_error("quotaExceeded", "m", 403)],
    );
    let error = h.source.events_in(one_hour_window()).await.unwrap_err();
    assert_eq!(
        error,
        Failure::Source(SourceError::InvalidResponse(
            "HTTP 403: quotaExceeded".to_string()
        ))
    );
}

// MARK: Review fixes

fn master_requests(h: &Harness) -> Vec<HttpRequest> {
    h.transport
        .requests_matching(&format!("{CAL_PATH}/master1"))
        .into_iter()
        .filter(|r| r.url.path().ends_with("/master1"))
        .collect()
}

fn later_occurrence() -> EventRef {
    occurrence(
        "master1_20260921T100000Z",
        "i1",
        "master1",
        Some("2026-09-21T10:00:00Z"),
    )
}

fn master_start() -> Value {
    json!({"dateTime": "2026-09-14T10:00:00Z", "timeZone": "UTC"})
}

fn master_end() -> Value {
    json!({"dateTime": "2026-09-14T10:30:00Z", "timeZone": "UTC"})
}

fn moved() -> EventTiming {
    EventTiming::new(
        instant("2026-09-21T11:00:00Z"),
        instant("2026-09-21T11:30:00Z"),
        Some(utc()),
        false,
    )
}

fn timing_patch(timing: EventTiming) -> EventPatch {
    hand_patch(|p| p.timing = Some(timing))
}

#[tokio::test]
async fn a_series_wide_time_change_from_a_later_occurrence_is_refused_before_any_patch() {
    let h = Harness::new().await;
    h.transport.route(
        &format!("{CAL_PATH}/master1"),
        vec![ok(google_event(
            "master1",
            json!({"start": master_start(), "end": master_end()}),
        ))],
    );
    let unsupported = WriteError::Unsupported {
        fields: [EventField::Timing].into(),
    };
    assert_write_error(
        h.source
            .update(
                &later_occurrence(),
                &timing_patch(moved()),
                RecurrenceScope::AllInSeries,
                NotifyPolicy::None,
            )
            .await,
        unsupported.clone(),
    );
    // Without the occurrence's slot there is nothing to compare, so it is refused too.
    let mut no_slot = later_occurrence();
    no_slot.original_start = None;
    assert_write_error(
        h.source
            .update(
                &no_slot,
                &timing_patch(moved()),
                RecurrenceScope::AllInSeries,
                NotifyPolicy::None,
            )
            .await,
        unsupported,
    );
    assert!(master_requests(&h).iter().all(|r| r.method == "GET"));
}

#[tokio::test]
async fn a_series_wide_time_change_from_the_first_occurrence_is_sent() {
    let h = Harness::new().await;
    h.transport.route(
        &format!("{CAL_PATH}/master1"),
        vec![
            ok(google_event(
                "master1",
                json!({"start": master_start(), "end": master_end()}),
            )),
            ok(google_event_full("master1", "m2", "Standup", json!({}))),
        ],
    );
    let first = occurrence(
        "master1_20260914T100000Z",
        "i1",
        "master1",
        Some("2026-09-14T10:00:00Z"),
    );
    h.source
        .update(
            &first,
            &timing_patch(moved()),
            RecurrenceScope::AllInSeries,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert_eq!(methods(&master_requests(&h)), ["GET", "PATCH"]);
}

#[tokio::test]
async fn a_series_wide_title_change_needs_no_master_fetch() {
    let h = Harness::new().await;
    h.transport.route(
        &format!("{CAL_PATH}/master1"),
        vec![ok(google_event_full("master1", "m2", "All", json!({})))],
    );
    h.source
        .update(
            &later_occurrence(),
            &title_patch("All"),
            RecurrenceScope::AllInSeries,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert_eq!(methods(&master_requests(&h)), ["PATCH"]);
}

#[tokio::test]
async fn a_time_change_to_a_single_instance_or_a_non_recurring_event_is_not_restricted() {
    let h = Harness::new().await;
    h.transport.route(
        &format!("{CAL_PATH}/master1_20260921T100000Z"),
        vec![ok(google_event_full(
            "master1_20260921T100000Z",
            "i2",
            "Standup",
            json!({}),
        ))],
    );
    h.source
        .update(
            &later_occurrence(),
            &timing_patch(moved()),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    h.transport.route(
        &format!("{CAL_PATH}/ev1"),
        vec![ok(google_event_full("ev1", "e2", "Standup", json!({})))],
    );
    h.source
        .update(
            &eref("ev1", Some("e1")),
            &timing_patch(moved()),
            RecurrenceScope::AllInSeries,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert!(
        h.transport
            .requests()
            .iter()
            .find(|r| r.method == "GET" && r.url.path().contains("/events/"))
            .is_none()
    );
}

#[tokio::test]
async fn delete_and_respond_with_series_scope_target_the_master() {
    let h = Harness::new().await;
    let attendees = json!([{"email": "me@x.com", "self": true, "responseStatus": "needsAction"}]);
    h.transport
        .route(&format!("{CAL_PATH}/master1"), vec![bare(204)]);
    h.source
        .delete(
            &later_occurrence(),
            RecurrenceScope::AllInSeries,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert_eq!(methods(&master_requests(&h)), ["DELETE"]);
    h.transport.route(
        &format!("{CAL_PATH}/master1"),
        vec![
            ok(google_event_full(
                "master1",
                "m1",
                "Standup",
                json!({"attendees": attendees}),
            )),
            ok(google_event_full("master1", "m2", "Standup", json!({}))),
        ],
    );
    h.source
        .respond(
            &later_occurrence(),
            ResponseStatus::Tentative,
            RecurrenceScope::AllInSeries,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert_eq!(methods(&master_requests(&h)), ["DELETE", "GET", "PATCH"]);
    assert!(h.transport.requests_matching("master1_2026").is_empty());
}

#[tokio::test]
async fn a_stale_attendee_edit_with_no_overlap_merges_on_the_fresh_etag() {
    let h = Harness::new().await;
    let dee = Attendee::new(
        None,
        Some("dee@x.com"),
        AttendeeRole::Required,
        ResponseStatus::NeedsAction,
        false,
        false,
    );
    let mut edit = EventEdit::new(base_with("Standup", "e1", Vec::new()));
    edit.event.attendees = vec![dee];
    let current = google_event_full(
        "ev1",
        "e2",
        "Standup",
        json!({"attendees": [{"email": "me@x.com", "self": true}]}),
    );
    h.transport.route(
        &format!("{CAL_PATH}/ev1"),
        vec![
            ok(current.clone()),
            ok(current.clone()),
            ok(current),
            ok(google_event_full("ev1", "e3", "Standup", json!({}))),
        ],
    );
    h.source
        .update(
            &EventRef::from_event(&base_with("Standup", "e1", Vec::new())),
            &edit.patch(),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let sent = h.transport.requests_matching(&format!("{CAL_PATH}/ev1"));
    assert_eq!(methods(&sent), ["GET", "GET", "GET", "PATCH"]);
    assert_eq!(header(&sent[3], "If-Match").as_deref(), Some("e2"));
    assert_eq!(
        ids(&body_json(&sent[3])["attendees"]),
        [Some("me@x.com".to_string()), Some("dee@x.com".to_string())]
    );
}

#[tokio::test]
async fn a_precondition_failed_on_the_attendee_patch_is_retried() {
    let h = Harness::new().await;
    let dee = Attendee::new(
        None,
        Some("dee@x.com"),
        AttendeeRole::Required,
        ResponseStatus::NeedsAction,
        false,
        false,
    );
    let mut edit = EventEdit::new(base_with("Standup", "e1", Vec::new()));
    edit.event.attendees = vec![dee];
    let first = google_event_full(
        "ev1",
        "e1",
        "Standup",
        json!({"attendees": [{"email": "me@x.com", "self": true}]}),
    );
    let second = google_event_full(
        "ev1",
        "e2",
        "Standup",
        json!({"attendees": [{"email": "me@x.com", "self": true}]}),
    );
    h.transport.route(
        &format!("{CAL_PATH}/ev1"),
        vec![
            ok(first),
            google_error("conditionNotMet", "m", 412),
            ok(second.clone()),
            ok(second),
            ok(google_event_full("ev1", "e3", "Standup", json!({}))),
        ],
    );
    h.source
        .update(
            &EventRef::from_event(&base_with("Standup", "e1", Vec::new())),
            &edit.patch(),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let sent = h.transport.requests_matching(&format!("{CAL_PATH}/ev1"));
    assert_eq!(methods(&sent), ["GET", "PATCH", "GET", "GET", "PATCH"]);
    assert!(
        header(&sent[1], "If-Match").as_deref() == Some("e1")
            && header(&sent[4], "If-Match").as_deref() == Some("e2")
    );
}

// MARK: Final review: a series master is not a single event

const MASTER_MESSAGE: &str =
    "this is a recurring series; use .allInSeries or read the occurrence first";

/// Creates a recurring event (the reply is the series master) and returns the ref a caller would build from it.
async fn created_series(h: &Harness) -> (CalendarEvent, EventRef) {
    h.transport.route(
        CAL_PATH,
        vec![ok(google_event_full(
            "new1",
            "e9",
            "Standup",
            json!({"recurrence": ["RRULE:FREQ=WEEKLY;COUNT=4"]}),
        ))],
    );
    let mut draft = EventDraft::new("Standup", timing());
    draft.recurrence = Some(weekly());
    let event = h
        .source
        .create(&draft, CAL, NotifyPolicy::None)
        .await
        .unwrap();
    let reference = EventRef::from_event(&event);
    (event, reference)
}

#[tokio::test]
async fn creating_a_recurring_event_returns_the_master_as_a_series() {
    let h = Harness::new().await;
    let (event, reference) = created_series(&h).await;
    assert!(
        event.event_id == "new1"
            && event.series_id() == Some("new1")
            && event.original_start() == Some(event.start)
    );
    assert!(
        reference.series_id() == Some("new1")
            && reference.original_start == Some(instant("2026-09-21T10:00:00Z"))
    );
}

#[tokio::test]
async fn an_event_without_a_recurrence_is_not_a_series_in_write_results() {
    let h = Harness::new().await;
    h.transport.route(
        CAL_PATH,
        vec![
            ok(google_event("new1", json!({}))),
            ok(google_event("new2", json!({"recurrence": []}))),
        ],
    );
    let plain = h
        .source
        .create(&EventDraft::new("T", timing()), CAL, NotifyPolicy::None)
        .await
        .unwrap();
    assert!(plain.series_id().is_none() && plain.original_start().is_none());
    let empty = h
        .source
        .create(&EventDraft::new("T", timing()), CAL, NotifyPolicy::None)
        .await
        .unwrap();
    assert!(empty.series_id().is_none() && empty.original_start().is_none());
}

#[tokio::test]
async fn a_this_instance_write_on_a_series_master_is_refused_before_any_request() {
    let h = Harness::new().await;
    let (_, reference) = created_series(&h).await;
    let before = h.transport.requests().len();
    let invalid = WriteError::Invalid(MASTER_MESSAGE.to_string());
    assert_write_error(
        h.source
            .delete(
                &reference,
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await
            .map(|()| 0),
        invalid.clone(),
    );
    assert_write_error(
        h.source
            .update(
                &reference,
                &title_patch("X"),
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
        invalid.clone(),
    );
    assert_write_error(
        h.source
            .respond(
                &reference,
                ResponseStatus::Accepted,
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
        invalid,
    );
    // No DELETE, PATCH or even a lookup.
    assert_eq!(h.transport.requests().len(), before);
}

#[tokio::test]
async fn series_wide_scopes_still_work_on_a_series_master_ref() {
    let h = Harness::new().await;
    let (_, reference) = created_series(&h).await;
    h.transport.route(
        &format!("{CAL_PATH}/new1"),
        vec![
            ok(google_event_full(
                "new1",
                "e10",
                "All",
                json!({"recurrence": ["RRULE:FREQ=WEEKLY;COUNT=4"]}),
            )),
            bare(204),
        ],
    );
    let updated = h
        .source
        .update(
            &reference,
            &title_patch("All"),
            RecurrenceScope::AllInSeries,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert!(updated.series_id() == Some("new1") && updated.title == "All");
    h.source
        .delete(&reference, RecurrenceScope::AllInSeries, NotifyPolicy::None)
        .await
        .unwrap();
    let sent = h.transport.requests_matching(&format!("{CAL_PATH}/new1"));
    assert_eq!(methods(&sent), ["PATCH", "DELETE"]);
    // The ref's version IS the master's, so the series-wide update keeps the lock (an instance ref sends none).
    assert!(
        header(&sent[0], "If-Match") == reference.version
            && reference.version.as_deref() == Some("e9")
    );
}

#[tokio::test]
async fn this_and_following_at_the_master_start_is_the_whole_series() {
    let h = Harness::new().await;
    let (_, reference) = created_series(&h).await;
    h.transport.route(
        &format!("{CAL_PATH}/new1"),
        vec![
            ok(google_event_full(
                "new1",
                "e9",
                "Standup",
                json!({"recurrence": ["RRULE:FREQ=WEEKLY;COUNT=4"]}),
            )),
            bare(204),
        ],
    );
    h.source
        .delete(
            &reference,
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let sent = h.transport.requests_matching(&format!("{CAL_PATH}/new1"));
    assert_eq!(methods(&sent), ["GET", "DELETE"]);
    // Only the create.
    assert_eq!(
        h.transport
            .requests_matching(&format!("{CAL_PATH}?"))
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        1
    );
}

// MARK: Final review: a recurrence change needs the whole series

#[tokio::test]
async fn a_recurrence_change_on_a_single_occurrence_is_refused_before_any_request() {
    let h = Harness::new().await;
    let single = occurrence(
        "m1_20260921T100000Z",
        "i1",
        "m1",
        Some("2026-09-21T10:00:00Z"),
    );
    for change in [FieldUpdate::Clear, FieldUpdate::Set(weekly())] {
        let patch = hand_patch(|p| p.recurrence = change.clone());
        assert_write_error(
            h.source
                .update(
                    &single,
                    &patch,
                    RecurrenceScope::ThisInstance,
                    NotifyPolicy::None,
                )
                .await,
            WriteError::Unsupported {
                fields: [EventField::Recurrence].into(),
            },
        );
    }
    assert!(h.transport.requests().is_empty());
    // Other scopes and non-series events are not affected by the check.
    h.transport.route(
        &format!("{CAL_PATH}/m1"),
        vec![ok(google_event_full("m1", "m2", "Standup", json!({})))],
    );
    let clear = hand_patch(|p| p.recurrence = FieldUpdate::Clear);
    h.source
        .update(
            &single,
            &clear,
            RecurrenceScope::AllInSeries,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    h.transport.route(
        &format!("{CAL_PATH}/ev1"),
        vec![ok(google_event_full("ev1", "e2", "Standup", json!({})))],
    );
    let set = hand_patch(|p| {
        p.timing = Some(timing());
        p.recurrence = FieldUpdate::Set(weekly());
    });
    h.source
        .update(
            &eref("ev1", Some("e1")),
            &set,
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
}

// Copying a meeting that may already be on the calendar (Issue 11).

fn draft_with_uid(uid: Option<&str>) -> EventDraft {
    let mut draft = EventDraft::new("Sync", timing());
    draft.uid = uid.map(str::to_string);
    draft
}

#[tokio::test]
async fn create_with_a_uid_that_is_already_on_the_calendar_throws_already_exists_and_inserts_nothing()
 {
    let h = Harness::new().await;
    h.transport.route(
        CAL_PATH,
        vec![ok(json!({"items": [google_event_full("there", "e5", "Standup", json!({"iCalUID": "u-1"}))]}))],
    );
    let result = h
        .source
        .create(&draft_with_uid(Some("u-1")), CAL, NotifyPolicy::None)
        .await;
    let Err(Failure::Write(WriteError::AlreadyExists(existing))) = result else {
        panic!("expected alreadyExists");
    };
    assert!(
        existing.event_id == "there"
            && existing.version.as_deref() == Some("e5")
            && existing.calendar_id == CAL
    );
    let requests = h.transport.requests_matching(CAL_PATH);
    assert!(
        requests.len() == 1
            && requests[0].method == "GET"
            && requests[0].url.as_str().contains("iCalUID=u-1")
    );
}

#[tokio::test]
async fn create_with_an_unknown_uid_looks_then_inserts_with_the_ical_uid() {
    let h = Harness::new().await;
    h.transport.route(
        CAL_PATH,
        vec![
            ok(json!({"items": []})),
            ok(google_event("new", json!({"iCalUID": "u-2"}))),
        ],
    );
    let created = h
        .source
        .create(&draft_with_uid(Some("u-2")), CAL, NotifyPolicy::None)
        .await
        .unwrap();
    let requests = h.transport.requests_matching(CAL_PATH);
    assert_eq!(methods(&requests), ["GET", "POST"]);
    assert_eq!(body_json(&requests[1])["iCalUID"], "u-2");
    assert!(created.event_id == "new" && created.uid.as_deref() == Some("u-2"));
}

#[tokio::test]
async fn create_without_a_uid_makes_no_lookup() {
    let h = Harness::new().await;
    h.transport
        .route(CAL_PATH, vec![ok(google_event("new", json!({})))]);
    h.source
        .create(&draft_with_uid(None), CAL, NotifyPolicy::None)
        .await
        .unwrap();
    assert_eq!(methods(&h.transport.requests_matching(CAL_PATH)), ["POST"]);
}

#[tokio::test]
async fn a_cancelled_match_is_ignored() {
    let h = Harness::new().await;
    h.transport.route(
        CAL_PATH,
        vec![
            ok(json!({"items": [google_event("gone", json!({"status": "cancelled", "iCalUID": "u-3"}))]})),
            ok(google_event("new", json!({}))),
        ],
    );
    let created = h
        .source
        .create(&draft_with_uid(Some("u-3")), CAL, NotifyPolicy::None)
        .await
        .unwrap();
    assert_eq!(created.event_id, "new");
}

#[tokio::test]
async fn copying_an_event_kit_shaped_event_creates_a_complete_google_event() {
    let h = Harness::new().await;
    let mut source = CalendarEvent::new(
        "x",
        "ek",
        "Sync",
        instant("2026-09-21T10:00:00Z"),
        instant("2026-09-21T10:30:00Z"),
    );
    source.uid = Some("u-4".to_string());
    source.uid_scope = Some(calendar_core::model::UidScope::Global);
    source.time_zone = utc();
    let draft = EventDraft::copying(&source, &h.source.capabilities());
    assert!(
        draft.uid.as_deref() == Some("u-4")
            && draft.availability == Availability::Busy
            && draft.visibility == Visibility::Default
            && draft.reminders.is_none()
    );
    h.transport.route(
        CAL_PATH,
        vec![
            ok(json!({"items": []})),
            ok(google_event(
                "new",
                json!({"iCalUID": "u-4", "reminders": {"useDefault": true}}),
            )),
        ],
    );
    let created = h
        .source
        .create(&draft, CAL, NotifyPolicy::None)
        .await
        .unwrap();
    let capabilities = SourceCapabilities {
        provided_fields: [
            ProvidedField::Kind,
            ProvidedField::Visibility,
            ProvidedField::Availability,
            ProvidedField::Reminders,
            ProvidedField::Series,
            ProvidedField::Participation,
            ProvidedField::Version,
        ]
        .into(),
        ..SourceCapabilities::default()
    };
    assert!(ProvidedFieldsConformance::event_violations(&created, &capabilities).is_empty());
    // An Exchange-style id is never sent as an iCalendar UID.
    source.uid_scope = Some(calendar_core::model::UidScope::Provider);
    assert!(
        EventDraft::copying(&source, &h.source.capabilities())
            .uid
            .is_none()
    );
}

// Setting a series' rule must not drop its skipped and extra dates.

#[tokio::test]
async fn setting_a_series_rule_keeps_the_masters_exdate_and_rdate_lines() {
    let h = Harness::new().await;
    let master = google_event(
        "master1",
        json!({"start": master_start(), "end": master_end(),
               "recurrence": ["RRULE:FREQ=WEEKLY", "EXDATE;TZID=UTC:20260921T100000", "RDATE;TZID=UTC:20261001T100000"]}),
    );
    h.transport.route(
        &format!("{CAL_PATH}/master1"),
        vec![
            ok(master),
            ok(google_event_full("master1", "m2", "Standup", json!({}))),
        ],
    );
    // The base gives the series' zone.
    let mut change = EventPatch::diff(&base(), &base());
    change.recurrence = FieldUpdate::Set(RecurrenceRule::new(Frequency::Daily));
    h.source
        .update(
            &later_occurrence(),
            &change,
            RecurrenceScope::AllInSeries,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let patch = master_requests(&h)
        .into_iter()
        .find(|r| r.method == "PATCH")
        .unwrap();
    assert_eq!(
        body_json(&patch)["recurrence"],
        json!([
            "RRULE:FREQ=DAILY",
            "EXDATE;TZID=UTC:20260921T100000",
            "RDATE;TZID=UTC:20261001T100000"
        ])
    );
}
