//! Port of `SplitSeriesTests.swift`.

use calendar_core::failure::Failure;
use calendar_core::http::HttpResponse;
use calendar_core::time::add_seconds;
use calendar_core::write::event_patch::{EventPatch, FieldUpdate};
use calendar_core::write::types::{
    EventField, EventRef, NotifyPolicy, RecurrenceScope, WriteError,
};
use calendar_core::write::writable_source::WritableCalendarSource;
use jiff::Timestamp;
use serde_json::{Map, Value, json};

use super::support::{
    SourceHarness, body_json, expect_write_error, graph_error, graph_error_message, graph_event,
    status,
};
use calendar_test_support::hand_patch;

/// 2026-09-25 10:00 in Los Angeles (a Friday).
fn first_start() -> Timestamp {
    Timestamp::from_second(1_790_355_600).unwrap()
}

/// 2026-10-09 10:00.
fn split_start() -> Timestamp {
    add_seconds(first_start(), 14.0 * 86_400.0)
}

fn recurrence(range: Value) -> Value {
    let mut range = range;
    range["startDate"] = json!("2026-09-25");
    range["recurrenceTimeZone"] = json!("Pacific Standard Time");
    json!({
        "pattern": {"type": "weekly", "interval": 1, "daysOfWeek": ["friday"], "firstDayOfWeek": "monday"},
        "range": range,
    })
}

fn master(range: Value) -> Value {
    graph_event(
        "master1",
        json!({"type": "seriesMaster", "recurrence": recurrence(range)}),
    )
}

fn no_end_master() -> Value {
    master(json!({"type": "noEnd"}))
}

fn occurrence() -> Value {
    graph_event(
        "occ3",
        json!({
            "type": "occurrence", "seriesMasterId": "master1",
            "start": {"dateTime": "2026-10-09T10:00:00.0000000", "timeZone": "Pacific Standard Time"},
            "end": {"dateTime": "2026-10-09T10:30:00.0000000", "timeZone": "Pacific Standard Time"},
        }),
    )
}

fn reference_at(event: &str, start: Option<Timestamp>) -> EventRef {
    let mut reference = EventRef::new("cal1", event).with_series_id(Some("master1"));
    reference.version = Some("ck1".to_string());
    reference.original_start = start;
    reference
}

fn reference() -> EventRef {
    reference_at("occ3", Some(split_start()))
}

fn created_event() -> HttpResponse {
    status(graph_event("new", json!({})), 201)
}

fn routes(h: &SourceHarness, master: Value, insert: Vec<HttpResponse>) {
    // The insert (least specific first).
    h.transport.route("cal1/events", insert);
    h.transport
        .route("cal1/events/master1", vec![super::support::ok(master)]);
    h.transport
        .route("cal1/events/occ3", vec![super::support::ok(occurrence())]);
}

fn default_routes(h: &SourceHarness) {
    routes(h, no_end_master(), vec![created_event()]);
}

fn patches_to_master(h: &SourceHarness) -> Vec<Map<String, Value>> {
    h.transport
        .requests_matching("cal1/events/master1")
        .iter()
        .filter(|request| request.method == "PATCH")
        .map(body_json)
        .collect()
}

fn inserts(h: &SourceHarness) -> Vec<Map<String, Value>> {
    h.transport
        .requests_matching("cal1/events")
        .iter()
        .filter(|request| request.method == "POST")
        .map(body_json)
        .collect()
}

fn range(body: &Map<String, Value>) -> Map<String, Value> {
    body.get("recurrence")
        .and_then(|recurrence| recurrence.get("range"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn title_patch(title: &str) -> EventPatch {
    hand_patch(|p| p.title = Some(title.to_string()))
}

#[tokio::test]
async fn deleting_this_and_following_cuts_the_series_the_day_before() {
    let h = SourceHarness::new().await;
    default_routes(&h);
    h.source
        .delete(
            &reference(),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let patches = patches_to_master(&h);
    assert!(
        patches.len() == 1
            && range(&patches[0])["type"] == "endDate"
            && range(&patches[0])["endDate"] == "2026-10-08"
    );
    assert!(inserts(&h).is_empty());
}

#[tokio::test]
async fn updating_this_and_following_cuts_the_series_and_starts_a_new_one_with_the_patch() {
    let h = SourceHarness::new().await;
    default_routes(&h);
    let event = h
        .source
        .update(
            &reference(),
            &title_patch("Renamed"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert_eq!(event.event_id, "new");
    let patches = patches_to_master(&h);
    assert!(patches.len() == 1 && range(&patches[0])["endDate"] == "2026-10-08");
    let created = inserts(&h);
    assert_eq!(created.len(), 1);
    assert!(created[0]["subject"] == "Renamed" && created[0]["transactionId"].is_string());
    assert!(
        range(&created[0])["type"] == "noEnd" && range(&created[0])["startDate"] == "2026-10-09"
    );
    assert_eq!(created[0]["start"]["dateTime"], "2026-10-09T10:00:00");
    assert!(
        created[0]["attendees"].is_array()
            && !created[0].contains_key("id")
            && !created[0].contains_key("iCalUId")
    );
}

#[tokio::test]
async fn a_numbered_series_keeps_the_occurrences_left_over() {
    let h = SourceHarness::new().await;
    routes(
        &h,
        master(json!({"type": "numbered", "numberOfOccurrences": 8})),
        vec![created_event()],
    );
    h.transport.route(
        "master1/instances",
        vec![super::support::ok(
            json!({"value": [{"id": "i1"}, {"id": "i2"}]}),
        )],
    );
    h.source
        .update(
            &reference(),
            &title_patch("Renamed"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let created = inserts(&h);
    assert!(
        range(&created[0])["type"] == "numbered" && range(&created[0])["numberOfOccurrences"] == 6
    );
    let patches = patches_to_master(&h);
    assert_eq!(range(&patches[0])["type"], "endDate");
}

#[tokio::test]
async fn a_deleted_slot_and_moved_exception_do_not_extend_a_numbered_continuation() {
    let h = SourceHarness::new().await;
    // 2026-10-16 10:00 Los Angeles.
    let fourth = Timestamp::from_second(1_792_170_000).unwrap();
    let fourth_ref = reference_at("occ3", Some(fourth));
    routes(
        &h,
        master(json!({"type": "numbered", "numberOfOccurrences": 5})),
        vec![created_event()],
    );
    // Graph omits deleted slot 2 and reports moved slot 3 after this split.
    h.transport.route(
        "master1/instances",
        vec![super::support::ok(json!({"value": [{"id": "i1"}]}))],
    );
    h.source
        .update(
            &fourth_ref,
            &title_patch("Renamed"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let created = inserts(&h);
    assert_eq!(created.len(), 1);
    assert_eq!(range(&created[0])["numberOfOccurrences"], 2);
    assert!(
        h.transport
            .requests_matching("master1/instances")
            .is_empty()
    );
}

#[tokio::test]
async fn unsupported_numbered_pattern_fails_before_any_write() {
    let h = SourceHarness::new().await;
    let mut unsupported = master(json!({"type": "numbered", "numberOfOccurrences": 5}));
    unsupported["recurrence"] = json!({
        "pattern": {"type": "lunar"},
        "range": {"type": "numbered", "startDate": "2026-09-25", "numberOfOccurrences": 5},
    });
    routes(&h, unsupported, vec![created_event()]);
    expect_write_error(
        WriteError::Unsupported {
            fields: [EventField::Recurrence].into(),
        },
        h.source
            .update(
                &reference(),
                &title_patch("Renamed"),
                RecurrenceScope::ThisAndFollowing,
                NotifyPolicy::None,
            )
            .await,
    );
    assert!(patches_to_master(&h).is_empty());
    assert!(inserts(&h).is_empty());
}

#[tokio::test]
async fn splitting_at_the_first_occurrence_is_the_same_as_writing_the_whole_series() {
    let h = SourceHarness::new().await;
    default_routes(&h);
    let mut first = EventRef::new("cal1", "master1").with_series_id(Some("master1"));
    first.version = Some("ck1".to_string());
    first.original_start = Some(first_start());
    h.source
        .update(
            &first,
            &title_patch("All"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let patches = patches_to_master(&h);
    assert!(
        patches.len() == 1
            && patches[0]["subject"] == "All"
            && !patches[0].contains_key("recurrence")
    );
    assert!(inserts(&h).is_empty());
}

#[tokio::test]
async fn a_definite_insert_failure_puts_the_original_recurrence_back() {
    let h = SourceHarness::new().await;
    routes(
        &h,
        no_end_master(),
        vec![graph_error_message("ErrorInvalidRequest", "no", 400)],
    );
    expect_write_error(
        WriteError::Invalid("no".to_string()),
        h.source
            .update(
                &reference(),
                &title_patch("Renamed"),
                RecurrenceScope::ThisAndFollowing,
                NotifyPolicy::None,
            )
            .await,
    );
    let patches = patches_to_master(&h);
    assert_eq!(patches.len(), 2);
    assert_eq!(range(&patches[0])["type"], "endDate");
    assert!(
        range(&patches[1])["type"] == "noEnd" && range(&patches[1])["startDate"] == "2026-09-25"
    );
    assert_eq!(inserts(&h).len(), 1);
}

#[tokio::test]
async fn a_lost_insert_reply_is_retried_once_with_the_same_transaction_id() {
    let h = SourceHarness::new().await;
    routes(
        &h,
        no_end_master(),
        vec![graph_error("Oops", 500), created_event()],
    );
    let event = h
        .source
        .update(
            &reference(),
            &title_patch("Renamed"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert_eq!(event.event_id, "new");
    let created = inserts(&h);
    assert!(created.len() == 2 && created[0]["transactionId"] == created[1]["transactionId"]);
    assert_eq!(
        patches_to_master(&h).len(),
        1,
        "the master is not restored: the insert may have applied"
    );
}

#[tokio::test]
async fn an_insert_that_keeps_failing_with_an_unknown_outcome_is_reported_as_partial() {
    let h = SourceHarness::new().await;
    routes(&h, no_end_master(), vec![graph_error("Oops", 500)]);
    let result = h
        .source
        .update(
            &reference(),
            &title_patch("Renamed"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await;
    assert!(
        matches!(result, Err(Failure::Write(WriteError::Partial(_)))),
        "{result:?}"
    );
    assert_eq!(patches_to_master(&h).len(), 1);
}

#[tokio::test]
async fn a_failed_restore_after_a_definite_insert_failure_is_reported_as_partial() {
    let h = SourceHarness::new().await;
    routes(
        &h,
        no_end_master(),
        vec![graph_error_message("ErrorInvalidRequest", "no", 400)],
    );
    // The truncation succeeds, the restore that follows the failed insert does not.
    h.transport.route(
        "cal1/events/master1",
        vec![
            super::support::ok(no_end_master()),
            super::support::ok(no_end_master()),
            graph_error("Oops", 400),
        ],
    );
    let result = h
        .source
        .update(
            &reference(),
            &title_patch("Renamed"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await;
    assert!(
        matches!(result, Err(Failure::Write(WriteError::Partial(_)))),
        "{result:?}"
    );
}

#[tokio::test]
async fn nothing_is_written_when_the_patch_is_invalid() {
    let h = SourceHarness::new().await;
    default_routes(&h);
    let patch = hand_patch(|p| {
        p.reminders = FieldUpdate::Clear;
    });
    expect_write_error(
        WriteError::Unsupported {
            fields: [EventField::Reminders].into(),
        },
        h.source
            .update(
                &reference(),
                &patch,
                RecurrenceScope::ThisAndFollowing,
                NotifyPolicy::None,
            )
            .await,
    );
    assert!(patches_to_master(&h).is_empty() && inserts(&h).is_empty());
}

#[tokio::test]
async fn a_split_needs_the_occurrences_original_start() {
    let h = SourceHarness::new().await;
    default_routes(&h);
    let no_slot = reference_at("occ3", None);
    expect_write_error(
        WriteError::Invalid("this and following needs the occurrence's original start".to_string()),
        h.source
            .delete(
                &no_slot,
                RecurrenceScope::ThisAndFollowing,
                NotifyPolicy::None,
            )
            .await,
    );
}
