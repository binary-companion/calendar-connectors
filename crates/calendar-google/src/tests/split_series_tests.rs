//! Port of `SplitSeriesTests.swift`.

use std::sync::{Arc, Mutex, PoisonError};

use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::{HttpRequest, HttpResponse};
use calendar_core::model::CalendarEvent;
use calendar_core::recurrence::rule::{Frequency, RecurrenceRule, RuleEnd};
use calendar_core::source_types::SourceError;
use calendar_core::time::Zone;
use calendar_core::write::event_draft::EventTiming;
use calendar_core::write::event_patch::{EventPatch, FieldUpdate};
use calendar_core::write::types::{
    ConferenceChange, EventField, EventRef, NotifyPolicy, RecurrenceScope, WriteError,
};
use calendar_core::write::writable_source::WritableCalendarSource;
use calendar_test_support::hand_patch;
use jiff::Timestamp;
use serde_json::{Map, Value, json};

use super::support::{
    Harness, Intercept, assert_write_error, bare, body_json, calendar_list_json, google_error,
    google_event_full, instant, ok, status,
};
use crate::write_mapper::GoogleWriteMapper;

const CAL: &str = "me@x.com";
const CAL_PATH: &str = "calendars/me%40x.com/events";
/// The third weekly occurrence.
const SPLIT: &str = "2026-09-15T15:00:00Z";

fn tokyo() -> Zone {
    Zone::named("Asia/Tokyo").unwrap()
}

fn utc() -> Zone {
    Zone::named("UTC").unwrap()
}

fn strings(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|line| (*line).to_string()).collect()
}

fn object(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
}

fn master_with(recurrence: &[&str]) -> Value {
    google_event_full(
        "m1",
        "em1",
        "Standup",
        json!({
            "start": {"dateTime": "2026-09-01T15:00:00Z", "timeZone": "UTC"},
            "end": {"dateTime": "2026-09-01T15:30:00Z", "timeZone": "UTC"},
            "description": "notes", "location": "Room", "recurrence": recurrence, "colorId": "5",
            "attendees": [
                {"email": "me@x.com", "self": true, "organizer": true, "responseStatus": "accepted"},
                {"email": "bob@x.com", "responseStatus": "accepted"},
            ],
            "conferenceData": {"conferenceSolution": {"key": {"type": "hangoutsMeet"}}, "conferenceId": "abc"},
            "reminders": {"useDefault": false, "overrides": [{"method": "popup", "minutes": 10}]},
            "htmlLink": "https://x", "iCalUID": "u1", "sequence": 3,
            "attachments": [{"fileUrl": "https://drive.example/notes", "title": "Notes"}],
        }),
    )
}

fn master() -> Value {
    master_with(&["RRULE:FREQ=WEEKLY;COUNT=10"])
}

fn instance_json() -> Value {
    google_event_full(
        "m1_20260915T150000Z",
        "ei3",
        "Standup",
        json!({
            "start": {"dateTime": SPLIT, "timeZone": "UTC"},
            "end": {"dateTime": "2026-09-15T15:30:00Z", "timeZone": "UTC"},
            "recurringEventId": "m1", "originalStartTime": {"dateTime": SPLIT},
        }),
    )
}

fn week_start(week: i64) -> String {
    Timestamp::from_second(instant("2026-09-01T15:00:00Z").as_second() + week * 604_800)
        .unwrap()
        .to_string()
}

fn instances_page() -> Value {
    json!({"items": (0..10).map(|i| json!({"originalStartTime": {"dateTime": week_start(i)}})).collect::<Vec<_>>()})
}

fn reference() -> EventRef {
    let mut reference = EventRef::new(CAL, "m1_20260915T150000Z").with_series_id(Some("m1"));
    reference.version = Some("ei3".to_string());
    reference.original_start = Some(instant(SPLIT));
    reference
}

type Hook = Box<dyn Fn(&HttpRequest) -> Option<Outcome<HttpResponse>> + Send + Sync>;

fn default_post() -> HttpResponse {
    ok(google_event_full("n1", "en1", "Standup", json!({})))
}

async fn harness(master_responses: Vec<HttpResponse>) -> Harness {
    harness_with(master_responses, default_post(), None).await
}

async fn harness_with(
    master_responses: Vec<HttpResponse>,
    post: HttpResponse,
    hook: Option<Hook>,
) -> Harness {
    let h = match hook {
        Some(hook) => {
            Harness::wrapped(calendar_list_json(), move |inner| {
                Arc::new(Intercept { inner, hook })
            })
            .await
        }
        None => Harness::new().await,
    };
    h.transport
        .route(&format!("{CAL_PATH}/m1"), master_responses);
    h.transport
        .route(&format!("{CAL_PATH}/m1_2026"), vec![ok(instance_json())]);
    h.transport.route(
        &format!("{CAL_PATH}/m1/instances"),
        vec![ok(instances_page())],
    );
    h.transport.route(&format!("{CAL_PATH}?"), vec![post]);
    h
}

fn master_writes(h: &Harness) -> Vec<HttpRequest> {
    h.transport.requests_matching(&format!("{CAL_PATH}/m1?"))
}

fn posts(h: &Harness) -> Vec<HttpRequest> {
    h.transport.requests_matching(&format!("{CAL_PATH}?"))
}

/// Remembers the id the insert was sent with, so a lookup hook can insist on it.
type SentId = Arc<Mutex<Option<String>>>;

/// Answers `answer` to a GET of exactly the id the POST carried (and nothing else).
fn lookup(sent: &SentId, answer: HttpResponse) -> Hook {
    let sent = sent.clone();
    Box::new(move |request| {
        let mut slot = sent.lock().unwrap_or_else(PoisonError::into_inner);
        if request.method == "POST" {
            *slot = body_json(request)["id"].as_str().map(str::to_string);
            return None;
        }
        if request.method == "GET"
            && let Some(id) = slot.as_ref()
            && request.url.as_str().ends_with(&format!("/events/{id}"))
        {
            return Some(Ok(answer.clone()));
        }
        None
    })
}

fn partial_message(result: Outcome<CalendarEvent>) -> String {
    match result {
        Err(Failure::Write(WriteError::Partial(message))) => message,
        other => panic!("expected .partial, got {other:?}"),
    }
}

fn location_patch(location: &str) -> EventPatch {
    hand_patch(|p| p.location = FieldUpdate::Set(location.to_string()))
}

fn recurrence_of(request: &HttpRequest) -> Value {
    body_json(request)["recurrence"].clone()
}

fn last_of(requests: Vec<HttpRequest>) -> HttpRequest {
    requests.last().cloned().unwrap()
}

// MARK: Pure helpers

#[test]
fn truncation_cuts_the_rule_just_before_the_split_and_keeps_other_lines() {
    let lines = strings(&[
        "RRULE:FREQ=WEEKLY;COUNT=10;BYDAY=TU",
        "EXDATE:20260908T150000Z",
    ]);
    assert_eq!(
        GoogleWriteMapper::truncated(&lines, instant(SPLIT), false, None),
        strings(&[
            "RRULE:FREQ=WEEKLY;BYDAY=TU;UNTIL=20260915T145959Z",
            "EXDATE:20260908T150000Z"
        ])
    );
    let all_day = GoogleWriteMapper::truncated(
        &strings(&["RRULE:FREQ=DAILY;UNTIL=20261231"]),
        instant("2026-09-14T15:00:00Z"),
        true,
        Some(&tokyo()),
    );
    // Split is Sep 15 in Tokyo; the last kept day is Sep 14.
    assert_eq!(all_day, strings(&["RRULE:FREQ=DAILY;UNTIL=20260914"]));
}

#[test]
fn truncation_moves_exception_and_added_dates_from_the_split_on_to_the_new_series() {
    let lines = strings(&[
        "RRULE:FREQ=WEEKLY;COUNT=10",
        "EXDATE:20260908T150000Z,20260922T150000Z",
        "RDATE:20260910T150000Z,20260917T150000Z",
        "EXDATE;TZID=Asia/Tokyo:20260916T000000",
        "RDATE:20260920T150000Z",
        "EXDATE;VALUE=DATE:20260901",
    ]);
    // Sep 16 00:00 in Tokyo is Sep 15 15:00Z, exactly the split, so that EXDATE moves; a line left with no values is dropped.
    assert_eq!(
        GoogleWriteMapper::truncated(&lines, instant(SPLIT), false, Some(&utc())),
        strings(&[
            "RRULE:FREQ=WEEKLY;UNTIL=20260915T145959Z",
            "EXDATE:20260908T150000Z",
            "RDATE:20260910T150000Z",
            "EXDATE;VALUE=DATE:20260901",
        ])
    );
    assert_eq!(
        GoogleWriteMapper::carried_over(&lines, instant(SPLIT), &utc()),
        strings(&[
            "EXDATE:20260922T150000Z",
            "RDATE:20260917T150000Z",
            "EXDATE;TZID=Asia/Tokyo:20260916T000000",
            "RDATE:20260920T150000Z",
        ])
    );
}

#[test]
fn count_helpers_read_and_replace_count() {
    assert_eq!(
        GoogleWriteMapper::count_in(&strings(&["RRULE:FREQ=WEEKLY;COUNT=10"])),
        Some(10)
    );
    assert_eq!(
        GoogleWriteMapper::count_in(&strings(&["RRULE:FREQ=WEEKLY;UNTIL=20261231"])),
        None
    );
    assert_eq!(
        GoogleWriteMapper::replacing_count(
            &strings(&["RRULE:FREQ=WEEKLY;COUNT=10", "EXDATE:x"]),
            8
        ),
        strings(&["RRULE:FREQ=WEEKLY;COUNT=8", "EXDATE:x"])
    );
    assert_eq!(
        GoogleWriteMapper::rrule_lines(&strings(&["RRULE:FREQ=WEEKLY", "EXDATE:x"])),
        strings(&["RRULE:FREQ=WEEKLY"])
    );
}

#[test]
fn only_exception_dates_from_the_split_on_carry_to_the_new_series() {
    let lines = strings(&[
        "RRULE:FREQ=WEEKLY",
        "EXDATE:20260908T150000Z,20260922T150000Z",
        "EXDATE;TZID=Asia/Tokyo:20260901T000000",
        "EXDATE;VALUE=DATE:20260914,20260930",
        "RDATE:20260920T150000Z",
    ]);
    assert_eq!(
        GoogleWriteMapper::carried_over(&lines, instant(SPLIT), &utc()),
        strings(&[
            "EXDATE:20260922T150000Z",
            "EXDATE;VALUE=DATE:20260930",
            "RDATE:20260920T150000Z"
        ])
    );
}

#[test]
fn the_new_series_body_is_built_from_the_master_without_output_only_fields() {
    let patch = location_patch("Lab");
    let body = GoogleWriteMapper::new_series_body(
        &object(master()),
        instant(SPLIT),
        &patch,
        strings(&["RRULE:FREQ=WEEKLY;COUNT=8"]),
        Some("UTC"),
    )
    .unwrap();
    let json = Value::Object(body.json.clone());
    for key in [
        "id",
        "etag",
        "iCalUID",
        "htmlLink",
        "sequence",
        "recurringEventId",
        "originalStartTime",
    ] {
        assert!(json.get(key).is_none(), "{key} must not be copied");
    }
    assert!(
        json["summary"] == "Standup" && json["description"] == "notes" && json["colorId"] == "5"
    );
    assert!(
        json["location"] == "Lab" && json["recurrence"] == json!(["RRULE:FREQ=WEEKLY;COUNT=8"])
    );
    assert_eq!(json["start"]["dateTime"], SPLIT);
    let attendees = json["attendees"].as_array().unwrap();
    assert!(attendees.iter().all(|a| a.get("responseStatus").is_none()
        && a.get("self").is_none()
        && a.get("organizer").is_none()));
    // The old Meet link is not copied; a new one is requested.
    assert!(json["conferenceData"]["createRequest"].is_object() && body.needs_conference_version);
    assert!(json.get("reminders").is_some());
}

#[test]
fn a_patch_that_removes_the_conference_does_not_request_a_new_one() {
    let patch = hand_patch(|p| p.conference = Some(ConferenceChange::Remove));
    let body = GoogleWriteMapper::new_series_body(
        &object(master()),
        instant(SPLIT),
        &patch,
        strings(&["RRULE:FREQ=WEEKLY"]),
        Some("UTC"),
    )
    .unwrap();
    assert!(body.json.get("conferenceData").is_none());
}

#[test]
fn a_timing_patch_on_the_new_series_carries_no_nulls_inside_start_and_end() {
    let all_day = EventTiming::new(
        instant("2026-09-15T00:00:00Z"),
        instant("2026-09-16T00:00:00Z"),
        Some(utc()),
        true,
    );
    let patch = hand_patch(|p| p.timing = Some(all_day));
    let body = GoogleWriteMapper::new_series_body(
        &object(master()),
        instant(SPLIT),
        &patch,
        strings(&["RRULE:FREQ=WEEKLY"]),
        Some("UTC"),
    )
    .unwrap();
    let start = Value::Object(object(body.json["start"].clone()));
    assert!(
        start["date"] == "2026-09-15"
            && start.get("dateTime").is_none()
            && start.get("timeZone").is_none()
    );
    assert!(!start.as_object().unwrap().values().any(Value::is_null));
}

// MARK: Flows

#[tokio::test]
async fn update_this_and_following_truncates_the_master_then_inserts_the_new_series() {
    let h = harness(vec![ok(master()), ok(master())]).await;
    let event = h
        .source
        .update(
            &reference(),
            &location_patch("Lab"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::All,
        )
        .await
        .unwrap();
    let truncate = last_of(master_writes(&h));
    assert!(
        truncate.method == "PATCH"
            && truncate.headers.get("If-Match").map(String::as_str) == Some("em1")
            && truncate.url.as_str().contains("sendUpdates=all")
    );
    assert_eq!(
        recurrence_of(&truncate),
        json!(["RRULE:FREQ=WEEKLY;UNTIL=20260915T145959Z"])
    );
    let post = last_of(posts(&h));
    assert!(post.method == "POST" && post.url.as_str().contains("sendUpdates=all"));
    // The master has attachments and a Meet link: the insert must be allowed to keep the first and re-request the second.
    assert!(
        post.url.as_str().contains("supportsAttachments=true")
            && post.url.as_str().contains("conferenceDataVersion=1")
    );
    assert_eq!(
        body_json(&post)["attachments"].as_array().map(Vec::len),
        Some(1)
    );
    assert!(
        recurrence_of(&post) == json!(["RRULE:FREQ=WEEKLY;COUNT=8"])
            && body_json(&post)["location"] == "Lab"
    );
    assert_eq!(event.event_id, "n1");
}

#[tokio::test]
async fn every_read_and_check_happens_before_the_master_is_touched() {
    // The occurrence cannot be read, so nothing may have been cut off.
    let h = harness(vec![ok(master()), ok(master())]).await;
    h.transport
        .route(&format!("{CAL_PATH}/m1_2026"), vec![status(json!({}), 404)]);
    assert_write_error(
        h.source
            .update(
                &reference(),
                &location_patch("Lab"),
                RecurrenceScope::ThisAndFollowing,
                NotifyPolicy::None,
            )
            .await,
        WriteError::NotFound,
    );
    assert!(master_writes(&h).is_empty());
    // The earlier occurrences cannot be counted, so nothing may have been cut off either.
    let h2 = harness(vec![ok(master()), ok(master())]).await;
    h2.transport.route(
        &format!("{CAL_PATH}/m1/instances"),
        vec![status(json!({}), 500)],
    );
    let error = h2
        .source
        .update(
            &reference(),
            &location_patch("Lab"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap_err();
    assert_eq!(error, Failure::Source(SourceError::Server { status: 500 }));
    assert!(master_writes(&h2).is_empty());
}

#[tokio::test]
async fn the_new_series_is_inserted_with_a_client_id_so_a_lost_reply_can_be_checked() {
    let h = harness(vec![ok(master()), ok(master())]).await;
    h.source
        .update(
            &reference(),
            &location_patch("Lab"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let post = last_of(posts(&h));
    let id = body_json(&post)["id"].as_str().unwrap().to_string();
    // base32hex, as Google requires.
    assert!(id.len() == 32 && id.chars().all(|c| "0123456789abcdef".contains(c)));
}

fn first_occurrence() -> EventRef {
    let mut first = EventRef::new(CAL, "m1_20260901T150000Z").with_series_id(Some("m1"));
    first.version = Some("i1".to_string());
    first.original_start = Some(instant("2026-09-01T15:00:00Z"));
    first
}

#[tokio::test]
async fn splitting_at_the_first_occurrence_is_the_whole_series() {
    let h = harness(vec![
        ok(master()),
        ok(google_event_full("m1", "em2", "All", json!({}))),
    ])
    .await;
    let patch = hand_patch(|p| p.title = Some("All".to_string()));
    h.source
        .update(
            &first_occurrence(),
            &patch,
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let last = last_of(master_writes(&h));
    assert!(
        last.method == "PATCH"
            && !last.headers.contains_key("If-Match")
            && body_json(&last)["summary"] == "All"
    );
    assert!(posts(&h).is_empty());
}

#[tokio::test]
async fn splitting_at_the_first_occurrence_may_change_the_time_of_the_whole_series() {
    let h = harness(vec![
        ok(master()),
        ok(master()),
        ok(google_event_full("m1", "em2", "All", json!({}))),
    ])
    .await;
    let timing = EventTiming::new(
        instant("2026-09-01T16:00:00Z"),
        instant("2026-09-01T16:30:00Z"),
        Some(utc()),
        false,
    );
    let patch = hand_patch(|p| p.timing = Some(timing));
    h.source
        .update(
            &first_occurrence(),
            &patch,
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let last = last_of(master_writes(&h));
    assert!(
        last.method == "PATCH" && body_json(&last)["start"]["dateTime"] == "2026-09-01T16:00:00Z"
    );
}

const POLICIES: [(NotifyPolicy, &str); 3] = [
    (NotifyPolicy::All, "all"),
    (NotifyPolicy::ExternalOnly, "externalOnly"),
    (NotifyPolicy::None, "none"),
];

#[tokio::test]
async fn the_truncation_and_the_insert_both_carry_the_callers_policy() {
    for (policy, wire) in POLICIES {
        let h = harness(vec![ok(master()), ok(master())]).await;
        h.source
            .update(
                &reference(),
                &location_patch("Lab"),
                RecurrenceScope::ThisAndFollowing,
                policy,
            )
            .await
            .unwrap();
        let truncate = last_of(master_writes(&h));
        assert!(
            truncate
                .url
                .as_str()
                .contains(&format!("sendUpdates={wire}")),
            "{wire}"
        );
        let post = last_of(posts(&h));
        assert!(
            post.url.as_str().contains(&format!("sendUpdates={wire}")),
            "{wire}"
        );
    }
}

#[tokio::test]
async fn a_failed_insert_restores_the_original_rule_with_the_callers_policy() {
    for (policy, wire) in POLICIES {
        let h = harness_with(
            vec![ok(master()), ok(master()), ok(master())],
            google_error("invalid", "bad start", 400),
            None,
        )
        .await;
        assert_write_error(
            h.source
                .update(
                    &reference(),
                    &location_patch("Lab"),
                    RecurrenceScope::ThisAndFollowing,
                    policy,
                )
                .await,
            WriteError::Invalid("bad start".to_string()),
        );
        let writes = master_writes(&h);
        assert!(
            writes.len() == 2
                && writes[0]
                    .url
                    .as_str()
                    .contains(&format!("sendUpdates={wire}")),
            "{wire}"
        );
        let restore = writes.last().unwrap();
        assert!(
            !restore.headers.contains_key("If-Match")
                && restore
                    .url
                    .as_str()
                    .contains(&format!("sendUpdates={wire}")),
            "{wire}"
        );
        assert_eq!(
            recurrence_of(restore),
            json!(["RRULE:FREQ=WEEKLY;COUNT=10"])
        );
    }
}

#[tokio::test]
async fn a_failed_insert_restores_the_original_rule() {
    let h = harness_with(
        vec![ok(master()), ok(master()), ok(master())],
        google_error("invalid", "bad start", 400),
        None,
    )
    .await;
    assert_write_error(
        h.source
            .update(
                &reference(),
                &location_patch("Lab"),
                RecurrenceScope::ThisAndFollowing,
                NotifyPolicy::None,
            )
            .await,
        WriteError::Invalid("bad start".to_string()),
    );
    let restore = last_of(master_writes(&h));
    assert!(
        restore.method == "PATCH"
            && !restore.headers.contains_key("If-Match")
            && restore.url.as_str().contains("sendUpdates=none")
    );
    assert_eq!(
        recurrence_of(&restore),
        json!(["RRULE:FREQ=WEEKLY;COUNT=10"])
    );
}

#[tokio::test]
async fn a_failed_insert_and_a_failed_restore_is_reported_as_partial() {
    let h = harness_with(
        vec![ok(master()), ok(master()), bare(500)],
        google_error("invalid", "bad start", 400),
        None,
    )
    .await;
    let message = partial_message(
        h.source
            .update(
                &reference(),
                &location_patch("Lab"),
                RecurrenceScope::ThisAndFollowing,
                NotifyPolicy::None,
            )
            .await,
    );
    // The insert's own error is not lost.
    assert!(message.contains("bad start"));
}

#[tokio::test]
async fn the_restore_still_runs_when_the_caller_is_cancelled_during_the_insert() {
    // Swift cancels the calling task from inside the transport and the restore runs in an unstructured task. A Rust
    // caller cancels by dropping the future, after which nothing can run, so the cancellation that can be observed is
    // the one the transport reports as an error: the insert fails with `Failure::Cancelled`, which may have been
    // applied, so the lookup finds nothing and the master's rule is restored before the cancellation is returned.
    let hook: Hook =
        Box::new(|request| (request.method == "POST").then_some(Err(Failure::Cancelled)));
    let h = harness_with(
        vec![ok(master()), ok(master()), ok(master())],
        default_post(),
        Some(hook),
    )
    .await;
    let result = h
        .source
        .update(
            &reference(),
            &location_patch("Lab"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await;
    assert_eq!(result.unwrap_err(), Failure::Cancelled);
    let restore = last_of(master_writes(&h));
    assert!(
        restore.method == "PATCH"
            && recurrence_of(&restore) == json!(["RRULE:FREQ=WEEKLY;COUNT=10"])
    );
}

#[tokio::test]
async fn a_lost_insert_reply_that_did_apply_is_not_rolled_back() {
    // The POST answers 500 but the event exists under the id we chose: the split is complete, so keep it.
    let sent: SentId = Arc::default();
    let hook = lookup(
        &sent,
        ok(google_event_full("n1", "en1", "Standup", json!({}))),
    );
    let h = harness_with(
        vec![ok(master()), ok(master()), ok(master())],
        bare(500),
        Some(hook),
    )
    .await;
    let event = h
        .source
        .update(
            &reference(),
            &location_patch("Lab"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert_eq!(event.event_id, "n1");
    let writes = master_writes(&h);
    assert!(
        writes.len() == 1
            && recurrence_of(&writes[0]) == json!(["RRULE:FREQ=WEEKLY;UNTIL=20260915T145959Z"])
    );
}

#[tokio::test]
async fn a_conflict_on_the_insert_means_our_own_request_already_applied() {
    // The id is ours alone, so a 409 (the typed `GoogleApiError::Conflict`) can only be an earlier attempt of this same POST.
    let sent: SentId = Arc::default();
    let hook = lookup(
        &sent,
        ok(google_event_full("n1", "en1", "Standup", json!({}))),
    );
    let h = harness_with(
        vec![ok(master()), ok(master()), ok(master())],
        google_error("duplicate", "exists", 409),
        Some(hook),
    )
    .await;
    let event = h
        .source
        .update(
            &reference(),
            &location_patch("Lab"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert_eq!(event.event_id, "n1");
    assert_eq!(master_writes(&h).len(), 1);
}

#[tokio::test]
async fn a_conflict_on_the_insert_that_finds_nothing_is_rolled_back_and_reported_as_unexpected() {
    let sent: SentId = Arc::default();
    let hook = lookup(&sent, google_error("notFound", "m", 404));
    let h = harness_with(
        vec![ok(master()), ok(master()), ok(master())],
        google_error("duplicate", "exists", 409),
        Some(hook),
    )
    .await;
    let error = h
        .source
        .update(
            &reference(),
            &location_patch("Lab"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error,
        Failure::Source(SourceError::InvalidResponse(
            "google: unexpected 409".to_string()
        ))
    );
    let restore = last_of(master_writes(&h));
    assert_eq!(
        recurrence_of(&restore),
        json!(["RRULE:FREQ=WEEKLY;COUNT=10"])
    );
}

#[tokio::test]
async fn a_lookup_that_fails_after_a_lost_reply_leaves_the_master_alone_and_reports_partial() {
    let sent: SentId = Arc::default();
    let hook = lookup(&sent, bare(503));
    let h = harness_with(
        vec![ok(master()), ok(master()), ok(master())],
        bare(500),
        Some(hook),
    )
    .await;
    let message = partial_message(
        h.source
            .update(
                &reference(),
                &location_patch("Lab"),
                RecurrenceScope::ThisAndFollowing,
                NotifyPolicy::None,
            )
            .await,
    );
    assert!(message.contains("unknown") && message.contains("503") && message.contains("500"));
    // Only the truncation: no restore that could duplicate the series.
    assert_eq!(master_writes(&h).len(), 1);
}

#[tokio::test]
async fn a_lost_insert_reply_that_did_not_apply_is_rolled_back() {
    let h = harness_with(
        vec![ok(master()), ok(master()), ok(master())],
        bare(500),
        None,
    )
    .await;
    let error = h
        .source
        .update(
            &reference(),
            &location_patch("Lab"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap_err();
    assert_eq!(error, Failure::Source(SourceError::Server { status: 500 }));
    let restore = last_of(master_writes(&h));
    assert_eq!(
        recurrence_of(&restore),
        json!(["RRULE:FREQ=WEEKLY;COUNT=10"])
    );
}

#[tokio::test]
async fn an_insert_that_succeeded_is_never_rolled_back_even_if_its_reply_cannot_be_read() {
    let h = harness_with(
        vec![ok(master()), ok(master()), ok(master())],
        ok(json!({})),
        None,
    )
    .await;
    let result = h
        .source
        .update(
            &reference(),
            &location_patch("Lab"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await;
    assert!(matches!(result, Err(Failure::Source(_))));
    // Only the truncation.
    assert_eq!(master_writes(&h).len(), 1);
}

#[tokio::test]
async fn an_unreadable_reply_to_a_successful_insert_is_recovered_by_looking_the_event_up() {
    let sent: SentId = Arc::default();
    let hook = lookup(
        &sent,
        ok(google_event_full("n1", "en1", "Standup", json!({}))),
    );
    let h = harness_with(
        vec![ok(master()), ok(master()), ok(master())],
        ok(json!({})),
        Some(hook),
    )
    .await;
    let event = h
        .source
        .update(
            &reference(),
            &location_patch("Lab"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert_eq!(event.event_id, "n1");
    assert_eq!(master_writes(&h).len(), 1);
}

#[tokio::test]
async fn a_stale_master_is_a_conflict_and_nothing_is_inserted() {
    let h = harness(vec![ok(master()), status(json!({}), 412)]).await;
    assert_write_error(
        h.source
            .update(
                &reference(),
                &location_patch("Lab"),
                RecurrenceScope::ThisAndFollowing,
                NotifyPolicy::None,
            )
            .await,
        WriteError::Conflict {
            fields: [EventField::Recurrence].into(),
        },
    );
    assert_eq!(master_writes(&h).len(), 1);
    assert!(posts(&h).iter().all(|r| r.method != "POST"));
}

// MARK: A lost reply to the truncation

#[tokio::test]
async fn a_truncation_whose_reply_is_lost_is_restored_and_the_original_error_rethrown() {
    // The PATCH may have been applied. Restore the original rule so a retry is safe (it would otherwise lose the EXDATE and
    // RDATE values the first truncation moved), and never insert.
    let h = harness(vec![ok(master()), bare(502), ok(master())]).await;
    let error = h
        .source
        .update(
            &reference(),
            &location_patch("Lab"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::All,
        )
        .await
        .unwrap_err();
    assert_eq!(error, Failure::Source(SourceError::Server { status: 502 }));
    let writes = master_writes(&h);
    assert_eq!(writes.len(), 2);
    assert_eq!(
        recurrence_of(&writes[0]),
        json!(["RRULE:FREQ=WEEKLY;UNTIL=20260915T145959Z"])
    );
    assert!(writes[0].url.as_str().contains("sendUpdates=all"));
    let restore = writes.last().unwrap();
    // Guests told about the truncation hear about its reversal too.
    assert!(
        restore.method == "PATCH"
            && !restore.headers.contains_key("If-Match")
            && restore.url.as_str().contains("sendUpdates=all")
    );
    assert_eq!(
        recurrence_of(restore),
        json!(["RRULE:FREQ=WEEKLY;COUNT=10"])
    );
    assert!(posts(&h).iter().all(|r| r.method != "POST"));
}

#[tokio::test]
async fn a_lost_truncation_reply_and_a_failed_restore_is_reported_as_partial() {
    let h = harness(vec![ok(master()), bare(502), bare(503)]).await;
    let message = partial_message(
        h.source
            .update(
                &reference(),
                &location_patch("Lab"),
                RecurrenceScope::ThisAndFollowing,
                NotifyPolicy::None,
            )
            .await,
    );
    // Both errors are named.
    assert!(message.contains("502") && message.contains("503"));
    assert!(posts(&h).iter().all(|r| r.method != "POST"));
}

#[tokio::test]
async fn the_restore_after_a_lost_truncation_reply_still_runs_when_the_caller_is_cancelled() {
    // See `the_restore_still_runs_when_the_caller_is_cancelled_during_the_insert`: the transport reports the
    // cancellation as an error on the truncating PATCH, and the restore still goes out.
    let hook: Hook = Box::new(|request| {
        let truncating = request.method == "PATCH"
            && request.url.path().ends_with("/m1")
            && recurrence_of(request) != json!(["RRULE:FREQ=WEEKLY;COUNT=10"]);
        truncating.then_some(Err(Failure::Cancelled))
    });
    let h = harness_with(
        vec![ok(master()), bare(502), ok(master())],
        default_post(),
        Some(hook),
    )
    .await;
    let result = h
        .source
        .update(
            &reference(),
            &location_patch("Lab"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await;
    assert_eq!(result.unwrap_err(), Failure::Cancelled);
    let restore = last_of(master_writes(&h));
    assert!(
        restore.method == "PATCH"
            && recurrence_of(&restore) == json!(["RRULE:FREQ=WEEKLY;COUNT=10"])
    );
}

#[tokio::test]
async fn a_definite_truncation_failure_is_not_restored() {
    let refused = harness(vec![ok(master()), google_error("invalid", "bad rule", 400)]).await;
    assert_write_error(
        refused
            .source
            .update(
                &reference(),
                &location_patch("Lab"),
                RecurrenceScope::ThisAndFollowing,
                NotifyPolicy::None,
            )
            .await,
        WriteError::Invalid("bad rule".to_string()),
    );
    assert_eq!(master_writes(&refused).len(), 1);
    let forbidden = harness(vec![ok(master()), google_error("forbidden", "m", 403)]).await;
    assert_write_error(
        forbidden
            .source
            .update(
                &reference(),
                &location_patch("Lab"),
                RecurrenceScope::ThisAndFollowing,
                NotifyPolicy::None,
            )
            .await,
        WriteError::Forbidden(None),
    );
    assert_eq!(master_writes(&forbidden).len(), 1);
    let gone = harness(vec![ok(master()), status(json!({}), 404)]).await;
    assert_write_error(
        gone.source
            .update(
                &reference(),
                &location_patch("Lab"),
                RecurrenceScope::ThisAndFollowing,
                NotifyPolicy::None,
            )
            .await,
        WriteError::NotFound,
    );
    assert_eq!(master_writes(&gone).len(), 1);
}

#[tokio::test]
async fn delete_with_a_lost_truncation_reply_keeps_its_plain_error_and_does_not_restore() {
    // For a delete the truncation is the operation and repeating it is idempotent.
    let h = harness(vec![ok(master()), bare(502)]).await;
    let error = h
        .source
        .delete(
            &reference(),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap_err();
    assert_eq!(error, Failure::Source(SourceError::Server { status: 502 }));
    assert_eq!(master_writes(&h).len(), 1);
}

#[tokio::test]
async fn delete_this_and_following_only_truncates_the_master() {
    let h = harness(vec![ok(master()), ok(master())]).await;
    h.source
        .delete(
            &reference(),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::All,
        )
        .await
        .unwrap();
    let truncate = last_of(master_writes(&h));
    assert!(truncate.method == "PATCH" && truncate.url.as_str().contains("sendUpdates=all"));
    assert_eq!(
        recurrence_of(&truncate),
        json!(["RRULE:FREQ=WEEKLY;UNTIL=20260915T145959Z"])
    );
    assert!(posts(&h).is_empty());
}

#[tokio::test]
async fn delete_this_and_following_at_the_first_occurrence_deletes_the_whole_series() {
    let h = harness(vec![ok(master()), bare(204)]).await;
    h.source
        .delete(
            &first_occurrence(),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert_eq!(last_of(master_writes(&h)).method, "DELETE");
}

#[tokio::test]
async fn a_rule_without_a_count_is_split_without_listing_instances() {
    let h = harness(vec![
        ok(master_with(&["RRULE:FREQ=WEEKLY;UNTIL=20261231T000000Z"])),
        ok(master()),
    ])
    .await;
    h.source
        .update(
            &reference(),
            &location_patch("Lab"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert!(h.transport.requests_matching("/instances").is_empty());
    let post = last_of(posts(&h));
    assert_eq!(
        recurrence_of(&post),
        json!(["RRULE:FREQ=WEEKLY;UNTIL=20261231T000000Z"])
    );
}

#[tokio::test]
async fn exception_and_added_dates_follow_the_side_of_the_split_they_belong_to() {
    let lines = [
        "RRULE:FREQ=WEEKLY;UNTIL=20261231T000000Z",
        "EXDATE:20260908T150000Z,20260922T150000Z",
        "RDATE:20260910T150000Z,20260917T150000Z",
    ];
    let h = harness(vec![ok(master_with(&lines)), ok(master())]).await;
    h.source
        .update(
            &reference(),
            &location_patch("Lab"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let truncate = last_of(master_writes(&h));
    assert_eq!(
        recurrence_of(&truncate),
        json!([
            "RRULE:FREQ=WEEKLY;UNTIL=20260915T145959Z",
            "EXDATE:20260908T150000Z",
            "RDATE:20260910T150000Z"
        ])
    );
    let post = last_of(posts(&h));
    assert_eq!(
        recurrence_of(&post),
        json!([
            "RRULE:FREQ=WEEKLY;UNTIL=20261231T000000Z",
            "EXDATE:20260922T150000Z",
            "RDATE:20260917T150000Z"
        ])
    );
}

#[tokio::test]
async fn delete_this_and_following_removes_later_added_dates_from_the_master() {
    let lines = [
        "RRULE:FREQ=WEEKLY;COUNT=10",
        "RDATE:20260920T150000Z",
        "EXDATE:20260908T150000Z,20260922T150000Z",
    ];
    let h = harness(vec![ok(master_with(&lines)), ok(master())]).await;
    h.source
        .delete(
            &reference(),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let truncate = last_of(master_writes(&h));
    assert_eq!(
        recurrence_of(&truncate),
        json!([
            "RRULE:FREQ=WEEKLY;UNTIL=20260915T145959Z",
            "EXDATE:20260908T150000Z"
        ])
    );
}

#[tokio::test]
async fn a_patch_that_replaces_the_rule_does_not_list_instances() {
    let h = harness(vec![ok(master()), ok(master())]).await;
    let mut daily = RecurrenceRule::new(Frequency::Daily);
    daily.end = RuleEnd::Count(3);
    let timing = EventTiming::new(
        instant(SPLIT),
        instant("2026-09-15T15:30:00Z"),
        Some(utc()),
        false,
    );
    let patch = hand_patch(|p| {
        p.timing = Some(timing);
        p.recurrence = FieldUpdate::Set(daily);
    });
    h.source
        .update(
            &reference(),
            &patch,
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert!(h.transport.requests_matching("/instances").is_empty());
    let post = last_of(posts(&h));
    assert_eq!(recurrence_of(&post), json!(["RRULE:FREQ=DAILY;COUNT=3"]));
}

#[tokio::test]
async fn a_moved_occurrence_is_cut_at_its_original_slot_and_the_new_series_starts_there_too() {
    let h = harness(vec![ok(master()), ok(master())]).await;
    let mut moved = instance_json();
    // This occurrence was moved an hour later and made longer; the rest of the series was not.
    moved["start"] = json!({"dateTime": "2026-09-15T16:00:00Z", "timeZone": "UTC"});
    moved["end"] = json!({"dateTime": "2026-09-15T17:00:00Z", "timeZone": "UTC"});
    h.transport
        .route(&format!("{CAL_PATH}/m1_2026"), vec![ok(moved)]);
    h.source
        .update(
            &reference(),
            &location_patch("Lab"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let truncate = last_of(master_writes(&h));
    assert_eq!(
        recurrence_of(&truncate),
        json!(["RRULE:FREQ=WEEKLY;UNTIL=20260915T145959Z"])
    );
    // The patch does not touch timing, so the following occurrences keep the series' time and length.
    let post = body_json(&last_of(posts(&h)));
    assert!(
        post["start"]["dateTime"] == "2026-09-15T15:00:00Z" && post["start"]["timeZone"] == "UTC"
    );
    assert!(post["end"]["dateTime"] == "2026-09-15T15:30:00Z" && post["end"]["timeZone"] == "UTC");
}

fn all_day_master(end: &str, until: &str) -> Value {
    google_event_full(
        "m1",
        "em1",
        "Standup",
        json!({"start": {"date": "2026-09-01"}, "end": {"date": end}, "recurrence": [format!("RRULE:FREQ=WEEKLY;UNTIL={until}")]}),
    )
}

fn all_day_ref() -> EventRef {
    let mut day = EventRef::new(CAL, "m1_20260915").with_series_id(Some("m1"));
    day.version = Some("ei3".to_string());
    day.original_start = Some(instant("2026-09-15T00:00:00Z"));
    day
}

#[tokio::test]
async fn a_moved_all_day_occurrence_starts_the_new_series_on_its_original_day_with_the_series_length()
 {
    let all_day_master = all_day_master("2026-09-03", "20261231");
    let all_day_instance = google_event_full(
        "m1_20260915",
        "ei3",
        "Standup",
        json!({"start": {"date": "2026-09-16"}, "end": {"date": "2026-09-19"}, "recurringEventId": "m1", "originalStartTime": {"date": "2026-09-15"}}),
    );
    let h = harness(vec![ok(all_day_master.clone()), ok(all_day_master)]).await;
    h.transport
        .route(&format!("{CAL_PATH}/m1_2026"), vec![ok(all_day_instance)]);
    h.source
        .update(
            &all_day_ref(),
            &location_patch("Lab"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let post = body_json(&last_of(posts(&h)));
    assert_eq!(post["start"], json!({"date": "2026-09-15"}));
    // The master spans two days.
    assert_eq!(post["end"], json!({"date": "2026-09-17"}));
}

#[test]
fn a_timing_patch_still_sets_the_new_series_time_whatever_the_slot_was() {
    let timing = EventTiming::new(
        instant("2026-09-15T18:00:00Z"),
        instant("2026-09-15T19:00:00Z"),
        Some(utc()),
        false,
    );
    let patch = hand_patch(|p| p.timing = Some(timing));
    let body = GoogleWriteMapper::new_series_body(
        &object(master()),
        instant(SPLIT),
        &patch,
        strings(&["RRULE:FREQ=WEEKLY"]),
        Some("UTC"),
    )
    .unwrap();
    assert_eq!(body.json["start"]["dateTime"], "2026-09-15T18:00:00Z");
    assert_eq!(body.json["end"]["dateTime"], "2026-09-15T19:00:00Z");
}

#[test]
fn the_new_series_keeps_the_masters_own_zone_name_and_the_fallback_uses_the_wire_name() {
    // The master's zone name carries over to the slot.
    let mut berlin = master();
    berlin["start"] = json!({"dateTime": "2026-09-01T17:00:00+02:00", "timeZone": "Europe/Berlin"});
    berlin["end"] = json!({"dateTime": "2026-09-01T17:30:00+02:00", "timeZone": "Europe/Berlin"});
    let patch = hand_patch(|p| p.title = Some("X".to_string()));
    let carried = GoogleWriteMapper::new_series_body(
        &object(berlin),
        instant(SPLIT),
        &patch,
        strings(&["RRULE:FREQ=WEEKLY"]),
        Some("UTC"),
    )
    .unwrap();
    assert_eq!(carried.json["start"]["timeZone"], "Europe/Berlin");
    // A patch time without a zone gets the fallback, spelled as the wire format wants ("UTC", not Darwin's "GMT").
    let zoneless = EventTiming::new(
        instant("2026-09-15T18:00:00Z"),
        instant("2026-09-15T19:00:00Z"),
        None,
        false,
    );
    let patch = hand_patch(|p| p.timing = Some(zoneless));
    let body = GoogleWriteMapper::new_series_body(
        &object(master()),
        instant(SPLIT),
        &patch,
        strings(&["RRULE:FREQ=WEEKLY"]),
        Some("GMT"),
    )
    .unwrap();
    assert_eq!(body.json["start"]["timeZone"], "UTC");
    assert_eq!(body.json["end"]["timeZone"], "UTC");
}

#[tokio::test]
async fn earlier_occurrences_are_counted_across_pages_including_cancelled_ones() {
    let h = harness(vec![ok(master()), ok(master())]).await;
    let page = |weeks: Vec<i64>, token: Option<&str>, cancelled: &[i64]| -> Value {
        let mut json = json!({"items": weeks.iter().map(|week| {
            let mut item = json!({"originalStartTime": {"dateTime": week_start(*week)}});
            if cancelled.contains(week) {
                item["status"] = json!("cancelled");
            }
            item
        }).collect::<Vec<_>>()});
        if let Some(token) = token {
            json["nextPageToken"] = json!(token);
        }
        json
    };
    h.transport.route(
        &format!("{CAL_PATH}/m1/instances"),
        vec![
            ok(page(vec![0, 1], Some("p2"), &[1])),
            ok(page((2..10).collect(), None, &[])),
        ],
    );
    h.source
        .update(
            &reference(),
            &location_patch("Lab"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let listings = h.transport.requests_matching("/instances");
    assert_eq!(listings.len(), 2);
    let first = listings[0].url.as_str();
    assert!(
        first.contains("showDeleted=true")
            && first.contains("maxResults=250")
            && first.contains("fields=")
    );
    assert!(listings[1].url.as_str().contains("pageToken=p2"));
    let post = last_of(posts(&h));
    // 10 in all, 2 before the split (one of them cancelled).
    assert_eq!(recurrence_of(&post), json!(["RRULE:FREQ=WEEKLY;COUNT=8"]));
}

#[tokio::test]
async fn an_all_day_series_is_cut_at_the_day_before_and_split_on_dates() {
    let all_day_master = all_day_master("2026-09-02", "20261231");
    let all_day_instance = google_event_full(
        "m1_20260915",
        "ei3",
        "Standup",
        json!({"start": {"date": "2026-09-15"}, "end": {"date": "2026-09-16"}, "recurringEventId": "m1", "originalStartTime": {"date": "2026-09-15"}}),
    );
    let h = harness(vec![ok(all_day_master.clone()), ok(all_day_master)]).await;
    h.transport
        .route(&format!("{CAL_PATH}/m1_2026"), vec![ok(all_day_instance)]);
    h.source
        .update(
            &all_day_ref(),
            &location_patch("Lab"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let truncate = last_of(master_writes(&h));
    assert_eq!(
        recurrence_of(&truncate),
        json!(["RRULE:FREQ=WEEKLY;UNTIL=20260914"])
    );
    let post = body_json(&last_of(posts(&h)));
    assert!(
        post["start"] == json!({"date": "2026-09-15"})
            && post["end"] == json!({"date": "2026-09-16"})
    );
    assert_eq!(
        post["recurrence"],
        json!(["RRULE:FREQ=WEEKLY;UNTIL=20261231"])
    );
}

#[tokio::test]
async fn clearing_the_rule_makes_the_rest_of_the_series_a_single_event() {
    let h = harness(vec![ok(master()), ok(master())]).await;
    let patch = hand_patch(|p| p.recurrence = FieldUpdate::Clear);
    h.source
        .update(
            &reference(),
            &patch,
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert!(h.transport.requests_matching("/instances").is_empty());
    let truncate = last_of(master_writes(&h));
    assert_eq!(
        recurrence_of(&truncate),
        json!(["RRULE:FREQ=WEEKLY;UNTIL=20260915T145959Z"])
    );
    let post = last_of(posts(&h));
    assert!(body_json(&post).get("recurrence").is_none());
}

// MARK: Staleness

fn stale_instance(location: Option<&str>, summary: &str) -> Value {
    let mut json = instance_json();
    json["etag"] = json!("ei9");
    json["summary"] = json!(summary);
    if let Some(location) = location {
        json["location"] = json!(location);
    }
    json
}

fn editing(location: &str) -> EventPatch {
    let mut base = CalendarEvent::new(
        "m1_20260915T150000Z",
        CAL,
        "Standup",
        instant(SPLIT),
        instant("2026-09-15T15:30:00Z"),
    );
    base.time_zone = utc();
    base.version = Some("ei3".to_string());
    let mut edited = base.clone();
    edited.location = Some(location.to_string());
    EventPatch::diff(&base, &edited)
}

#[tokio::test]
async fn a_stale_occurrence_that_overlaps_the_patch_is_a_conflict_and_nothing_is_written() {
    let h = harness(vec![ok(master()), ok(master())]).await;
    h.transport.route(
        &format!("{CAL_PATH}/m1_2026"),
        vec![ok(stale_instance(Some("Elsewhere"), "Standup"))],
    );
    assert_write_error(
        h.source
            .update(
                &reference(),
                &editing("Lab"),
                RecurrenceScope::ThisAndFollowing,
                NotifyPolicy::None,
            )
            .await,
        WriteError::Conflict {
            fields: [EventField::Location].into(),
        },
    );
    assert!(master_writes(&h).is_empty());
    assert!(posts(&h).is_empty());
}

#[tokio::test]
async fn a_hand_built_patch_conflicts_on_any_stale_version() {
    let h = harness(vec![ok(master()), ok(master())]).await;
    h.transport.route(
        &format!("{CAL_PATH}/m1_2026"),
        vec![ok(stale_instance(None, "Standup"))],
    );
    assert_write_error(
        h.source
            .update(
                &reference(),
                &location_patch("Lab"),
                RecurrenceScope::ThisAndFollowing,
                NotifyPolicy::None,
            )
            .await,
        WriteError::Conflict {
            fields: [EventField::Location].into(),
        },
    );
    assert!(master_writes(&h).is_empty());
}

#[tokio::test]
async fn a_stale_occurrence_that_only_changed_other_fields_still_splits() {
    let h = harness(vec![ok(master()), ok(master())]).await;
    h.transport.route(
        &format!("{CAL_PATH}/m1_2026"),
        vec![ok(stale_instance(None, "Renamed"))],
    );
    let event = h
        .source
        .update(
            &reference(),
            &editing("Lab"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert_eq!(event.event_id, "n1");
    assert_eq!(master_writes(&h).len(), 1);
}

#[tokio::test]
async fn a_ref_without_a_version_is_not_checked_for_staleness() {
    let h = harness(vec![ok(master()), ok(master())]).await;
    h.transport.route(
        &format!("{CAL_PATH}/m1_2026"),
        vec![ok(stale_instance(Some("Elsewhere"), "Standup"))],
    );
    let mut unversioned = reference();
    unversioned.version = None;
    h.source
        .update(
            &unversioned,
            &location_patch("Lab"),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert_eq!(master_writes(&h).len(), 1);
}

#[tokio::test]
async fn splitting_needs_the_occurrences_original_start() {
    let h = harness(vec![ok(master())]).await;
    let mut no_start = EventRef::new(CAL, "m1_x").with_series_id(Some("m1"));
    no_start.version = Some("i".to_string());
    let patch = hand_patch(|p| p.title = Some("X".to_string()));
    assert_write_error(
        h.source
            .update(
                &no_start,
                &patch,
                RecurrenceScope::ThisAndFollowing,
                NotifyPolicy::None,
            )
            .await,
        WriteError::Invalid("this and following needs the occurrence's original start".to_string()),
    );
}

#[tokio::test]
async fn google_now_accepts_all_three_scopes() {
    use calendar_core::source_types::CalendarSource;
    let h = Harness::new().await;
    assert_eq!(
        h.source.capabilities().recurrence_scopes,
        RecurrenceScope::ALL.iter().copied().collect()
    );
}
