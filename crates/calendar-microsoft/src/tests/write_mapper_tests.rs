//! Port of `WriteMapperTests.swift`.

use calendar_core::all_day::{AllDay, CalendarDate};
use calendar_core::model::Availability;
use calendar_core::model::{
    AttendeeRole, Reminder, ReminderAnchor, ReminderTrigger, ResponseStatus, Visibility,
};
use calendar_core::recurrence::rule::{
    Frequency, RecurrenceRule, RuleEnd, Weekday, WeekdayOccurrence,
};
use calendar_core::time::{Zone, add_seconds};
use calendar_core::windows_time_zones::WindowsTimeZones;
use calendar_core::write::event_draft::{AttendeeDraft, EventDraft, EventTiming};
use calendar_core::write::event_patch::{AttendeeChanges, EventPatch, FieldUpdate};
use calendar_core::write::types::{ConferenceChange, ConferenceRequest, EventField, WriteError};
use calendar_test_support::hand_patch;
use jiff::Timestamp;
use serde_json::{Map, Value, json};

use crate::write_mapper::{GraphWriteMapper, RecurrenceAnchor};

fn la() -> Zone {
    Zone::named("America/Los_Angeles").unwrap()
}

/// 2026-09-25 10:00 in Los Angeles.
fn ten_am() -> Timestamp {
    Timestamp::from_second(1_790_355_600).unwrap()
}

fn timing(zone: Option<Zone>) -> EventTiming {
    EventTiming::new(ten_am(), add_seconds(ten_am(), 30.0 * 60.0), zone, false)
}

fn dict(value: Option<&Value>) -> Map<String, Value> {
    value
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn unsupported(field: EventField) -> WriteError {
    WriteError::Unsupported {
        fields: [field].into(),
    }
}

#[test]
fn create_body_writes_a_timed_event_in_the_zones_windows_name() {
    let body =
        GraphWriteMapper::create_body(&EventDraft::new("Standup", timing(Some(la())))).unwrap();
    assert!(body["subject"] == "Standup" && body["isAllDay"] == false);
    assert!(
        dict(body.get("start"))["dateTime"] == "2026-09-25T10:00:00"
            && dict(body.get("start"))["timeZone"] == "Pacific Standard Time"
    );
    assert_eq!(dict(body.get("end"))["dateTime"], "2026-09-25T10:30:00");
    assert!(body["showAs"] == "busy" && body["sensitivity"] == "normal");
    for key in ["body", "location", "attendees", "recurrence"] {
        assert!(!body.contains_key(key), "{key}");
    }
}

#[test]
fn create_body_carries_every_field() {
    let mut draft = EventDraft::new("Review", timing(Some(la())));
    draft.notes = Some("Agenda".to_string());
    draft.location = Some("Room 1".to_string());
    draft.availability = Availability::Free;
    draft.visibility = Visibility::PrivateEvent;
    draft.reminders = Some(vec![Reminder::with_minutes_before(15)]);
    draft.attendees = vec![
        AttendeeDraft::new("A@x.com", Some("Ann"), AttendeeRole::Required),
        AttendeeDraft::new("r@x.com", None, AttendeeRole::Resource),
    ];
    draft.conference = ConferenceRequest::Generate;
    draft.recurrence = Some(RecurrenceRule {
        weekdays: vec![WeekdayOccurrence::new(Weekday::Friday, None)],
        end: RuleEnd::Count(4),
        ..RecurrenceRule::new(Frequency::Weekly)
    });
    let body = GraphWriteMapper::create_body(&draft).unwrap();
    assert!(
        dict(body.get("body"))["contentType"] == "text"
            && dict(body.get("body"))["content"] == "Agenda"
    );
    assert_eq!(dict(body.get("location"))["displayName"], "Room 1");
    assert!(body["showAs"] == "free" && body["sensitivity"] == "private");
    assert!(body["isReminderOn"] == true && body["reminderMinutesBeforeStart"] == 15);
    let attendees = body["attendees"].as_array().unwrap();
    assert!(
        attendees.len() == 2
            && attendees[0]["type"] == "required"
            && attendees[1]["type"] == "resource"
    );
    assert!(
        attendees[0]["emailAddress"]["address"] == "a@x.com"
            && attendees[0]["emailAddress"]["name"] == "Ann"
    );
    assert!(body["isOnlineMeeting"] == true && body["onlineMeetingProvider"] == "teamsForBusiness");
    let range = dict(dict(body.get("recurrence")).get("range"));
    assert!(
        range["type"] == "numbered"
            && range["numberOfOccurrences"] == 4
            && range["startDate"] == "2026-09-25"
    );
}

#[test]
fn a_zone_without_a_windows_name_is_written_as_the_same_instants_in_utc() {
    let odd = Zone::named("Etc/GMT+5").unwrap();
    assert_eq!(WindowsTimeZones::windows_name_for(&odd), None);
    let body = GraphWriteMapper::create_body(&EventDraft::new("x", timing(Some(odd)))).unwrap();
    assert!(
        dict(body.get("start"))["timeZone"] == "UTC"
            && dict(body.get("start"))["dateTime"] == "2026-09-25T17:00:00"
    );
}

#[test]
fn an_all_day_event_is_written_as_midnight_to_midnight() {
    let start = AllDay::start_of_day(&CalendarDate::new(2026, 9, 25), &la()).unwrap();
    let end = AllDay::start_of_day(&CalendarDate::new(2026, 9, 27), &la()).unwrap();
    let body = GraphWriteMapper::create_body(&EventDraft::new(
        "Trip",
        EventTiming::new(start, end, Some(la()), true),
    ))
    .unwrap();
    assert_eq!(body["isAllDay"], true);
    assert!(
        dict(body.get("start"))["dateTime"] == "2026-09-25T00:00:00"
            && dict(body.get("end"))["dateTime"] == "2026-09-27T00:00:00"
    );
    assert_eq!(dict(body.get("start"))["timeZone"], "Pacific Standard Time");
}

#[test]
fn create_refuses_what_graph_cannot_store() {
    let repeating = Reminder {
        repeat_count: 2,
        repeat_interval: Some(60.0),
        ..Reminder::new(ReminderTrigger::Relative {
            offset: -600.0,
            to: ReminderAnchor::Start,
        })
    };
    let from_end = Reminder::new(ReminderTrigger::Relative {
        offset: 0.0,
        to: ReminderAnchor::End,
    });
    for reminders in [
        vec![
            Reminder::with_minutes_before(5),
            Reminder::with_minutes_before(10),
        ],
        vec![repeating],
        vec![from_end],
    ] {
        let mut draft = EventDraft::new("x", timing(Some(la())));
        draft.reminders = Some(reminders);
        assert_eq!(
            GraphWriteMapper::create_body(&draft),
            Err(unsupported(EventField::Reminders))
        );
    }
    let mut draft = EventDraft::new("x", timing(Some(la())));
    draft.reminders = Some(Vec::new());
    let body = GraphWriteMapper::create_body(&draft).unwrap();
    assert_eq!(body["isReminderOn"], false);
}

#[test]
fn an_empty_patch_produces_an_empty_body() {
    assert!(
        GraphWriteMapper::patch_body(
            &EventPatch::default(),
            &[],
            &RecurrenceAnchor::placeholder()
        )
        .unwrap()
        .is_empty()
    );
}

#[test]
fn patch_body_carries_only_what_changed() {
    let patch = hand_patch(|p| {
        p.title = Some("New".to_string());
        p.notes = FieldUpdate::Clear;
        p.location = FieldUpdate::Set("Room 2".to_string());
        p.availability = Some(Availability::Tentative);
        p.visibility = Some(Visibility::Confidential);
        p.conference = Some(ConferenceChange::Remove);
    });
    let body = GraphWriteMapper::patch_body(&patch, &[], &RecurrenceAnchor::placeholder()).unwrap();
    let keys: Vec<&str> = body.keys().map(String::as_str).collect();
    let mut expected = vec![
        "subject",
        "body",
        "location",
        "showAs",
        "sensitivity",
        "isOnlineMeeting",
    ];
    expected.sort_unstable();
    assert_eq!(keys, expected);
    assert!(
        dict(body.get("body"))["content"] == ""
            && dict(body.get("location"))["displayName"] == "Room 2"
    );
    assert!(
        body["showAs"] == "tentative"
            && body["sensitivity"] == "confidential"
            && body["isOnlineMeeting"] == false
    );
}

#[test]
fn patch_refuses_removing_reminders_or_a_recurrence() {
    let anchor = RecurrenceAnchor::placeholder();
    let reminders = hand_patch(|p| {
        p.reminders = FieldUpdate::Clear;
    });
    assert_eq!(
        GraphWriteMapper::patch_body(&reminders, &[], &anchor),
        Err(unsupported(EventField::Reminders))
    );
    let recurrence = hand_patch(|p| {
        p.recurrence = FieldUpdate::Clear;
    });
    assert_eq!(
        GraphWriteMapper::patch_body(&recurrence, &[], &anchor),
        Err(unsupported(EventField::Recurrence))
    );
}

#[test]
fn attendee_changes_start_from_the_current_list_and_keep_everyone_else() {
    let current: Vec<Map<String, Value>> = [
        json!({"type": "required", "status": {"response": "accepted"}, "emailAddress": {"name": "Me", "address": "Me@X.com"}}),
        json!({"type": "optional", "status": {"response": "declined"}, "emailAddress": {"name": "Opt", "address": "opt@x.com"}}),
    ]
    .into_iter()
    .map(|value| value.as_object().cloned().unwrap())
    .collect();
    let changes = AttendeeChanges::new(
        vec![
            AttendeeDraft::new("new@x.com", None, AttendeeRole::Required),
            AttendeeDraft::new("opt@x.com", Some("Optional"), AttendeeRole::Required),
        ],
        vec!["ME@x.com".to_string()],
    );
    let list = GraphWriteMapper::attendee_list(&current, &changes);
    assert_eq!(list.len(), 2);
    assert!(
        dict(list[0].get("emailAddress"))["address"] == "opt@x.com"
            && list[0]["type"] == "required"
    );
    assert_eq!(dict(list[0].get("emailAddress"))["name"], "Optional");
    assert_eq!(dict(list[1].get("emailAddress"))["address"], "new@x.com");
    // Responses stay on the server.
    assert!(list.iter().all(|item| !item.contains_key("status")));
}

#[test]
fn respond_action_names_the_graph_action() {
    assert_eq!(
        GraphWriteMapper::respond_action(ResponseStatus::Accepted),
        Ok("accept")
    );
    assert_eq!(
        GraphWriteMapper::respond_action(ResponseStatus::Tentative),
        Ok("tentativelyAccept")
    );
    assert_eq!(
        GraphWriteMapper::respond_action(ResponseStatus::Declined),
        Ok("decline")
    );
    assert_eq!(
        GraphWriteMapper::respond_action(ResponseStatus::NeedsAction),
        Err(WriteError::Invalid(
            "a response must be accepted, tentative or declined".to_string()
        ))
    );
}
