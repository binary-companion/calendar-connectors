//! Port of `WriteMapperTests.swift`.

use calendar_core::failure::Failure;
use calendar_core::model::{
    AttendeeRole, Availability, CalendarEvent, Reminder, ReminderAnchor, ReminderTrigger,
    ReminderType, ResponseStatus, Visibility,
};
use calendar_core::recurrence::rule::{
    Frequency, RecurrenceRule, RuleEnd, Weekday, WeekdayOccurrence,
};
use calendar_core::time::Zone;
use calendar_core::write::event_draft::{AttendeeDraft, EventDraft, EventTiming};
use calendar_core::write::event_patch::{AttendeeChanges, EventPatch, FieldUpdate};
use calendar_core::write::types::{ConferenceChange, EventField, NotifyPolicy, WriteError};
use calendar_test_support::hand_patch;
use jiff::Timestamp;
use serde_json::{Map, Value, json};

use super::support::{assert_write_error, instant};
use crate::write_mapper::{Body, GoogleWriteMapper};

fn utc() -> Zone {
    Zone::named("UTC").unwrap()
}

fn new_york() -> Zone {
    Zone::named("America/New_York").unwrap()
}

fn timed(zone: Option<Zone>) -> EventTiming {
    EventTiming::new(
        instant("2026-09-21T10:00:00Z"),
        instant("2026-09-21T10:30:00Z"),
        zone,
        false,
    )
}

fn obj(map: &Map<String, Value>) -> Value {
    Value::Object(map.clone())
}

fn body_obj(body: &Body) -> Value {
    obj(&body.json)
}

fn attendee_list(value: &Value) -> Vec<Value> {
    value.as_array().cloned().unwrap_or_default()
}

fn map_of(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
}

fn maps(values: Vec<Value>) -> Vec<Map<String, Value>> {
    values.into_iter().map(map_of).collect()
}

fn emails(values: &[Map<String, Value>]) -> Vec<Option<String>> {
    values
        .iter()
        .map(|m| m.get("email").and_then(Value::as_str).map(str::to_string))
        .collect()
}

// Foundation on Darwin reports the "UTC" zone's identifier as "GMT" while Linux says "UTC"; the wire format is always "UTC".
#[test]
fn create_body_for_a_timed_event() {
    let mut draft = EventDraft::new("Sync", timed(Some(utc())));
    draft.notes = Some("n".to_string());
    draft.location = Some("Room".to_string());
    draft.availability = Availability::Free;
    draft.visibility = Visibility::PrivateEvent;
    draft.reminders = Some(vec![Reminder::with_minutes_before(10)]);
    draft.attendees = vec![
        AttendeeDraft::new("a@x.com", Some("A"), AttendeeRole::Optional),
        AttendeeDraft::new("r@x.com", None, AttendeeRole::Resource),
    ];
    let body = GoogleWriteMapper::create_body(&draft).unwrap();
    let json = body_obj(&body);
    assert!(json["summary"] == "Sync" && json["description"] == "n" && json["location"] == "Room");
    assert!(
        json["start"]["dateTime"] == "2026-09-21T10:00:00Z" && json["start"]["timeZone"] == "UTC"
    );
    assert!(json["end"]["dateTime"] == "2026-09-21T10:30:00Z");
    assert!(json["transparency"] == "transparent" && json["visibility"] == "private");
    assert_eq!(
        json["reminders"],
        json!({"useDefault": false, "overrides": [{"method": "popup", "minutes": 10}]})
    );
    let attendees = attendee_list(&json["attendees"]);
    assert!(
        attendees.len() == 2
            && attendees[0]["optional"] == true
            && attendees[0]["displayName"] == "A"
    );
    assert!(attendees[1]["resource"] == true);
    assert!(
        !body.needs_conference_version
            && json.get("recurrence").is_none()
            && json.get("conferenceData").is_none()
    );
}

#[test]
fn all_day_uses_dates_with_the_exclusive_end() {
    let timing = EventTiming::new(
        instant("2026-09-18T04:00:00Z"),
        instant("2026-09-20T04:00:00Z"),
        Some(new_york()),
        true,
    );
    let json = body_obj(&GoogleWriteMapper::create_body(&EventDraft::new("Trip", timing)).unwrap());
    assert!(
        json["start"] == json!({"date": "2026-09-18"})
            && json["end"] == json!({"date": "2026-09-20"})
    );
}

#[test]
fn nil_reminders_are_omitted_and_generate_requests_a_meet_link() {
    let mut draft = EventDraft::new("Sync", timed(Some(utc())));
    assert!(
        GoogleWriteMapper::create_body(&draft)
            .unwrap()
            .json
            .get("reminders")
            .is_none()
    );
    draft.conference = calendar_core::write::types::ConferenceRequest::Generate;
    let body = GoogleWriteMapper::create_body(&draft).unwrap();
    let json = body_obj(&body);
    let request = &json["conferenceData"]["createRequest"];
    assert!(
        body.needs_conference_version && request["conferenceSolutionKey"]["type"] == "hangoutsMeet"
    );
    assert!(
        request["requestId"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
}

#[test]
fn recurrence_renders_an_rrule_and_needs_a_zone_unless_all_day() {
    let mut rule = RecurrenceRule::new(Frequency::Weekly);
    rule.weekdays = vec![WeekdayOccurrence::new(Weekday::Monday, None)];
    rule.end = RuleEnd::Count(4);
    let mut draft = EventDraft::new("Weekly", timed(Some(utc())));
    draft.recurrence = Some(rule);
    assert_eq!(
        body_obj(&GoogleWriteMapper::create_body(&draft).unwrap())["recurrence"],
        json!(["RRULE:FREQ=WEEKLY;BYDAY=MO;COUNT=4"])
    );
    draft.timing = timed(None);
    assert_write_error(
        GoogleWriteMapper::create_body(&draft),
        WriteError::Invalid("recurring events need a time zone".to_string()),
    );
}

#[test]
fn too_many_reminders_are_rejected() {
    let many: Vec<Reminder> = (1..=6).map(Reminder::with_minutes_before).collect();
    let mut draft = EventDraft::new("T", timed(Some(utc())));
    draft.reminders = Some(many);
    assert_write_error(
        GoogleWriteMapper::create_body(&draft),
        WriteError::Invalid("Google allows at most 5 reminders".to_string()),
    );
}

#[test]
fn patch_body_contains_only_touched_fields() {
    let patch = hand_patch(|p| {
        p.title = Some("New".to_string());
        p.availability = Some(Availability::Busy);
    });
    let body = GoogleWriteMapper::patch_body(&patch, None).unwrap();
    assert!(
        body_obj(&body) == json!({"summary": "New", "transparency": "opaque"})
            && !body.needs_conference_version
    );
    assert!(
        GoogleWriteMapper::patch_body(&EventPatch::default(), None)
            .unwrap()
            .json
            .is_empty()
    );
}

#[test]
fn patch_body_clears_with_null_and_resets_reminders_to_defaults() {
    let patch = hand_patch(|p| {
        p.notes = FieldUpdate::Clear;
        p.location = FieldUpdate::Clear;
        p.reminders = FieldUpdate::Clear;
        p.recurrence = FieldUpdate::Clear;
        p.conference = Some(ConferenceChange::Remove);
    });
    let json = body_obj(&GoogleWriteMapper::patch_body(&patch, None).unwrap());
    assert!(
        json["description"].is_null()
            && json["location"].is_null()
            && json["recurrence"].is_null()
            && json["conferenceData"].is_null()
    );
    for key in ["description", "location", "recurrence", "conferenceData"] {
        assert!(json.get(key).is_some(), "{key} must be present as null");
    }
    assert_eq!(json["reminders"], json!({"useDefault": true}));
}

#[test]
fn patch_body_sends_time_as_a_pair_and_recurrence_uses_the_patch_timing() {
    let mut daily = RecurrenceRule::new(Frequency::Daily);
    daily.end = RuleEnd::Count(3);
    let patch = hand_patch(|p| {
        p.timing = Some(timed(Some(new_york())));
        p.recurrence = FieldUpdate::Set(daily);
    });
    let json = body_obj(&GoogleWriteMapper::patch_body(&patch, None).unwrap());
    assert!(
        json["start"]["timeZone"] == "America/New_York"
            && json["end"]["dateTime"] == "2026-09-21T10:30:00Z"
    );
    assert_eq!(json["recurrence"], json!(["RRULE:FREQ=DAILY;COUNT=3"]));
}

#[test]
fn attendee_changes_merge_into_the_current_array_and_keep_the_rest() {
    let current = maps(vec![
        json!({"email": "me@x.com", "self": true, "responseStatus": "accepted"}),
        json!({"email": "Bob@x.com", "responseStatus": "accepted", "optional": true}),
        json!({"email": "cy@x.com", "responseStatus": "declined"}),
    ]);
    let changes = AttendeeChanges::new(
        vec![
            AttendeeDraft::new("bob@x.com", Some("Bob"), AttendeeRole::Required),
            AttendeeDraft::new("dee@x.com", None, AttendeeRole::Required),
        ],
        vec!["cy@x.com".to_string()],
    );
    let patch = hand_patch(|p| p.attendees = Some(changes));
    let json = body_obj(&GoogleWriteMapper::patch_body(&patch, Some(&current)).unwrap());
    let merged = maps(attendee_list(&json["attendees"]));
    assert_eq!(
        emails(&merged),
        [
            Some("me@x.com".into()),
            Some("Bob@x.com".into()),
            Some("dee@x.com".into())
        ]
    );
    assert!(
        merged[1]["responseStatus"] == "accepted"
            && merged[1].get("optional").is_none()
            && merged[1]["displayName"] == "Bob"
    );
    assert!(merged[0]["self"] == true);
}

#[test]
fn attendee_changes_need_the_current_attendees() {
    let patch = hand_patch(|p| {
        p.attendees = Some(AttendeeChanges::new(
            vec![AttendeeDraft::new("a@b.c", None, AttendeeRole::Required)],
            Vec::new(),
        ));
    });
    assert_write_error(
        GoogleWriteMapper::patch_body(&patch, None),
        WriteError::Invalid("attendee changes need the current attendees".to_string()),
    );
}

#[test]
fn responding_sets_the_self_attendee_only() {
    let current = maps(vec![
        json!({"email": "me@x.com", "self": true, "responseStatus": "needsAction"}),
        json!({"email": "bob@x.com", "responseStatus": "accepted"}),
    ]);
    let updated =
        GoogleWriteMapper::respond_attendees(&current, ResponseStatus::Tentative).unwrap();
    assert!(
        updated[0]["responseStatus"] == "tentative" && updated[1]["responseStatus"] == "accepted"
    );
    assert_write_error(
        GoogleWriteMapper::respond_attendees(&current, ResponseStatus::NeedsAction),
        WriteError::Invalid("cannot respond with needsAction".to_string()),
    );
    assert_write_error(
        GoogleWriteMapper::respond_attendees(
            &maps(vec![json!({"email": "bob@x.com"})]),
            ResponseStatus::Accepted,
        ),
        WriteError::Invalid("you are not an attendee of this event".to_string()),
    );
}

#[test]
fn send_updates_maps_every_policy() {
    assert!(
        GoogleWriteMapper::send_updates(NotifyPolicy::All) == "all"
            && GoogleWriteMapper::send_updates(NotifyPolicy::ExternalOnly) == "externalOnly"
            && GoogleWriteMapper::send_updates(NotifyPolicy::None) == "none"
    );
}

// MARK: Hostile input and Google's limits

#[test]
fn reminder_minutes_must_be_within_google_limits_on_create_and_patch() {
    // Google accepts 0 through 40320 minutes (four weeks); a patch skips `EventDraft.validate`, so the mapper checks.
    let ok = GoogleWriteMapper::reminders_json(&[
        Reminder::with_minutes_before(0),
        Reminder::with_minutes_before(40320),
    ])
    .unwrap();
    assert!(!ok.is_empty());
    let message = "reminder minutes must be between 0 and 40320";
    let patch =
        hand_patch(|p| p.reminders = FieldUpdate::Set(vec![Reminder::with_minutes_before(-1)]));
    assert_write_error(
        GoogleWriteMapper::patch_body(&patch, None),
        WriteError::Invalid(message.to_string()),
    );
    let mut draft = EventDraft::new("T", timed(Some(utc())));
    draft.reminders = Some(vec![Reminder::with_minutes_before(40321)]);
    assert_write_error(
        GoogleWriteMapper::create_body(&draft),
        WriteError::Invalid(message.to_string()),
    );
}

#[test]
fn serialising_non_finite_numbers_throws_instead_of_crashing() {
    // Swift's `JSONSerialization` needs a guard against NaN. A `serde_json::Value` cannot hold one at all
    // (`Number::from_f64` refuses it), so there is no value to reject here; the output rules are what remain.
    assert!(serde_json::Number::from_f64(f64::NAN).is_none());
    let json = map_of(json!({"b": 1, "a": "é"}));
    assert_eq!(
        String::from_utf8(GoogleWriteMapper::data(&json).unwrap()).unwrap(),
        r#"{"a":"é","b":1}"#
    );
}

#[test]
fn times_outside_googles_year_range_are_rejected() {
    // Swift builds dates in the year 11,000; `jiff::Timestamp` stops at year 9999, so the lower bound (year 1) is
    // what can be crossed here: one hour in year 0.
    let before_year_one = Timestamp::from_second(-62_135_596_801 - 3600).unwrap();
    let far = EventTiming::new(
        before_year_one,
        Timestamp::from_second(-62_135_596_801).unwrap(),
        Some(utc()),
        false,
    );
    let message = WriteError::Invalid("times must fall in the years 1 to 9999".to_string());
    assert_write_error(
        GoogleWriteMapper::create_body(&EventDraft::new("T", far.clone())),
        message.clone(),
    );
    let patch = hand_patch(|p| p.timing = Some(far));
    assert_write_error(GoogleWriteMapper::patch_body(&patch, None), message);
}

#[test]
fn a_fixed_offset_zone_is_not_sent_as_a_time_zone_name() {
    // `GMT+0500` is not an IANA name Google accepts; the `dateTime` offset already pins the instant.
    let offset = Zone::named("GMT+0500").unwrap();
    let json = body_obj(
        &GoogleWriteMapper::create_body(&EventDraft::new("T", timed(Some(offset.clone()))))
            .unwrap(),
    );
    assert!(
        json["start"].get("timeZone").is_none()
            && json["start"]["dateTime"] == "2026-09-21T10:00:00Z"
    );
    let mut draft = EventDraft::new("T", timed(Some(offset)));
    draft.recurrence = Some(RecurrenceRule::new(Frequency::Daily));
    assert_write_error(
        GoogleWriteMapper::create_body(&draft),
        WriteError::Invalid("recurring events need a time zone".to_string()),
    );
}

#[test]
fn patching_time_nulls_the_other_form_so_an_all_day_event_can_become_timed_and_back() {
    let to_timed = hand_patch(|p| p.timing = Some(timed(Some(utc()))));
    let json = body_obj(&GoogleWriteMapper::patch_body(&to_timed, None).unwrap());
    assert!(json["start"]["date"].is_null() && json["end"]["date"].is_null());
    assert!(json["start"].get("date").is_some() && json["end"].get("date").is_some());
    let all_day = EventTiming::new(
        instant("2026-09-18T04:00:00Z"),
        instant("2026-09-19T04:00:00Z"),
        Some(new_york()),
        true,
    );
    let to_all_day = hand_patch(|p| p.timing = Some(all_day));
    let json = body_obj(&GoogleWriteMapper::patch_body(&to_all_day, None).unwrap());
    assert!(json["start"]["date"] == "2026-09-18");
    assert!(json["start"].get("dateTime").is_some_and(Value::is_null));
    assert!(json["start"].get("timeZone").is_some_and(Value::is_null));
    // Creating never sends nulls.
    let created = body_obj(
        &GoogleWriteMapper::create_body(&EventDraft::new("T", timed(Some(utc())))).unwrap(),
    );
    assert!(created["start"].get("date").is_none());
}

#[test]
fn a_patch_timing_without_a_zone_does_not_borrow_the_base_zone_for_recurrence() {
    let mut base = CalendarEvent::new(
        "e",
        "c",
        "T",
        instant("2026-09-21T10:00:00Z"),
        instant("2026-09-21T10:30:00Z"),
    );
    base.time_zone = new_york();
    let mut patch = EventPatch::diff(&base, &base);
    // "No preference".
    patch.timing = Some(EventTiming::new(base.start, base.end, None, false));
    patch.recurrence = FieldUpdate::Set(RecurrenceRule::new(Frequency::Daily));
    assert_write_error(
        GoogleWriteMapper::patch_body(&patch, None),
        WriteError::Invalid("recurring events need a time zone".to_string()),
    );
    // With no timing in the patch the base's zone is what the series will use.
    let mut recurrence_only = EventPatch::diff(&base, &base);
    recurrence_only.recurrence = FieldUpdate::Set(RecurrenceRule::new(Frequency::Daily));
    assert_eq!(
        body_obj(&GoogleWriteMapper::patch_body(&recurrence_only, None).unwrap())["recurrence"],
        json!(["RRULE:FREQ=DAILY"])
    );
}

#[test]
fn removing_attendees_ignores_case_even_when_the_list_was_edited_after_construction() {
    let changes = AttendeeChanges {
        remove: vec!["CY@X.com".to_string()],
        ..AttendeeChanges::default()
    };
    let merged = GoogleWriteMapper::merge_attendees(
        &maps(vec![
            json!({"email": "cy@x.com"}),
            json!({"email": "Dee@x.com"}),
        ]),
        &changes,
    );
    assert_eq!(emails(&merged), [Some("Dee@x.com".to_string())]);
}

#[test]
fn a_gmt_or_utc_zone_is_sent_as_utc_on_every_platform() {
    for name in ["UTC", "GMT"] {
        let zone = Zone::named(name).unwrap();
        let (start, end) = GoogleWriteMapper::time_json(&timed(Some(zone)));
        assert!(
            start["timeZone"] == "UTC" && end["timeZone"] == "UTC",
            "{name}"
        );
    }
    // The same zone gates and renders a recurrence, so a series on a GMT zone still gets its rule.
    let mut draft = EventDraft::new("Daily", timed(Some(Zone::named("GMT").unwrap())));
    draft.recurrence = Some(RecurrenceRule::new(Frequency::Daily));
    assert!(
        GoogleWriteMapper::create_body(&draft)
            .unwrap()
            .json
            .get("recurrence")
            .is_some()
    );
}

#[test]
fn a_tentative_availability_is_written_as_opaque_and_reported_as_an_adjustment() {
    let mut draft = EventDraft::new("T", timed(Some(utc())));
    draft.availability = Availability::Tentative;
    assert_eq!(
        body_obj(&GoogleWriteMapper::create_body(&draft).unwrap())["transparency"],
        "opaque"
    );
    let mut stored = CalendarEvent::new("e", "c", "T", draft.timing.start, draft.timing.end);
    stored.availability = Some(Availability::Busy);
    assert_eq!(
        draft.adjustments_compared_to(&stored),
        [EventField::Availability].into()
    );
    let patched = hand_patch(|p| p.availability = Some(Availability::Unavailable));
    assert_eq!(
        body_obj(&GoogleWriteMapper::patch_body(&patched, None).unwrap())["transparency"],
        "opaque"
    );
}

#[test]
fn reminders_become_popup_or_email_and_unsupported_ones_are_refused() {
    let json = GoogleWriteMapper::reminders_json(&[
        Reminder::before(10, ReminderType::Display, None),
        Reminder::before(60, ReminderType::Email { address: None }, None),
    ])
    .unwrap();
    let overrides = attendee_list(&json["overrides"]);
    assert_eq!(
        overrides
            .iter()
            .map(|o| o["method"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["popup", "email"]
    );
    assert_eq!(
        overrides
            .iter()
            .map(|o| o["minutes"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        [10, 60]
    );
    let mut repeating = Reminder::new(ReminderTrigger::Relative {
        offset: -60.0,
        to: ReminderAnchor::Start,
    });
    repeating.repeat_count = 1;
    repeating.repeat_interval = Some(60.0);
    for reminder in [
        Reminder::new(ReminderTrigger::Relative {
            offset: -60.0,
            to: ReminderAnchor::End,
        }),
        Reminder::new(ReminderTrigger::Absolute(Timestamp::UNIX_EPOCH)),
        Reminder::before(5, ReminderType::Audio { sound_name: None }, None),
        Reminder::before(
            5,
            ReminderType::Email {
                address: Some("a@x.test".to_string()),
            },
            None,
        ),
        repeating,
    ] {
        let result = GoogleWriteMapper::reminders_json(std::slice::from_ref(&reminder));
        assert_eq!(
            result.unwrap_err(),
            Failure::Write(WriteError::Unsupported {
                fields: [EventField::Reminders].into()
            }),
            "{reminder:?}"
        );
    }
}
