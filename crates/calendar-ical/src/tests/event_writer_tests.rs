//! Port of `EventWriterTests.swift`.

use std::collections::BTreeSet;

use calendar_core::all_day::{AllDay, CalendarDate};
use calendar_core::civil;
use calendar_core::model::{
    AttendeeRole, Availability, Reminder, ReminderAnchor, ReminderTrigger, ReminderType,
    ResponseStatus, StructuredLocation, Visibility,
};
use calendar_core::recurrence::rule::RecurrenceRule;
use calendar_core::time::{DateInterval, Zone};
use calendar_core::write::event_draft::{AttendeeDraft, EventDraft, EventTiming};
use calendar_core::write::event_patch::{AttendeeChanges, FieldUpdate};
use calendar_core::write::types::{EventField, WriteError};
use calendar_test_support::hand_patch;
use jiff::Timestamp;

use super::fixtures::{WEEKLY_SERIES, at, context_named, la, la_time, resource};
use crate::alarm_mapper::AlarmMapper;
use crate::attendee_mapper::AttendeeMapper;
use crate::component::ICalComponent;
use crate::event_reader::EventReader;
use crate::event_resource::EventResource;
use crate::event_writer::EventWriter;
use crate::ical_values::ICalValues;

fn now() -> Timestamp {
    at(1_790_000_000)
}

fn draft(timing: Option<EventTiming>) -> EventDraft {
    let timing = timing.unwrap_or_else(|| {
        EventTiming::new(
            la_time!(2026, 9, 10, 9),
            la_time!(2026, 9, 10, 10),
            Some(la()),
            false,
        )
    });
    let mut draft = EventDraft::new("Design review", timing);
    draft.notes = Some("Bring, notes; please".to_string());
    draft.location = Some("Room 2".to_string());
    draft.availability = Availability::Free;
    draft.visibility = Visibility::PrivateEvent;
    draft.reminders = Some(vec![Reminder::before(10, ReminderType::Display, None)]);
    draft.recurrence = Some(RecurrenceRule::from_rrule("FREQ=WEEKLY;COUNT=3", None).unwrap());
    draft
}

fn read_back(vevent: ICalComponent) -> Vec<calendar_core::model::CalendarEvent> {
    let resource =
        EventWriter::resource_for(vevent, &[la()], la_time!(2026, 9, 1), la_time!(2027, 9, 1))
            .unwrap();
    let parsed = EventResource::from_data(&resource.serialized()).unwrap();
    EventReader::events_in(
        &parsed,
        &DateInterval::new(la_time!(2026, 9, 1, 0), la_time!(2026, 10, 1, 0)),
        &context_named("new.ics", &["mailto:me@icloud.test"]),
    )
}

fn reminders_unsupported() -> WriteError {
    WriteError::Unsupported {
        fields: [EventField::Reminders].into_iter().collect(),
    }
}

fn value<'a>(vevent: &'a ICalComponent, name: &str) -> Option<&'a str> {
    vevent.property(name).map(|p| p.value.as_str())
}

#[test]
fn draft_round_trips_through_the_reader() {
    let events = read_back(EventWriter::vevent_from(&draft(None), "new-uid", now(), None).unwrap());
    assert_eq!(events.len(), 3);
    let first = &events[0];
    assert_eq!(first.uid.as_deref(), Some("new-uid"));
    assert_eq!(first.title, "Design review");
    assert_eq!(first.notes.as_deref(), Some("Bring, notes; please"));
    assert_eq!(first.location.as_deref(), Some("Room 2"));
    assert!(first.start == la_time!(2026, 9, 10, 9) && first.end == la_time!(2026, 9, 10, 10));
    assert_eq!(first.time_zone.identifier(), "America/Los_Angeles");
    assert_eq!(first.availability, Some(Availability::Free));
    assert_eq!(first.visibility, Some(Visibility::PrivateEvent));
    assert_eq!(
        first.reminders,
        Some(vec![Reminder {
            is_calendar_default: Some(false),
            ..Reminder::new(ReminderTrigger::Relative {
                offset: -600.0,
                to: ReminderAnchor::Start
            })
        }])
    );
}

#[test]
fn writes_tzid_and_vtimezone() {
    let vevent = EventWriter::vevent_from(&draft(None), "u", now(), None).unwrap();
    assert_eq!(
        vevent.property("DTSTART").unwrap().parameter("TZID"),
        Some("America/Los_Angeles")
    );
    assert_eq!(value(&vevent, "DTSTART"), Some("20260910T090000"));
    assert_eq!(
        value(&vevent, "DTSTAMP"),
        Some(ICalValues::utc_text(now()).as_str())
    );
    assert_eq!(value(&vevent, "SEQUENCE"), Some("0"));
    let resource =
        EventWriter::resource_for(vevent, &[la()], la_time!(2026, 9, 1), la_time!(2027, 9, 1))
            .unwrap();
    assert_eq!(
        resource.calendar.components_named("VTIMEZONE")[0]
            .property("TZID")
            .map(|p| p.value.as_str()),
        Some("America/Los_Angeles")
    );
    assert_eq!(
        value(&resource.calendar, "PRODID"),
        Some(EventWriter::PRODUCT_ID)
    );
    assert_eq!(value(&resource.calendar, "VERSION"), Some("2.0"));
}

#[test]
fn utc_and_zoneless_drafts_use_z() {
    let timing = EventTiming::new(
        la_time!(2026, 9, 10, 9),
        la_time!(2026, 9, 10, 10),
        None,
        false,
    );
    let vevent = EventWriter::vevent_from(&draft(Some(timing)), "u", now(), None).unwrap();
    assert_eq!(value(&vevent, "DTSTART"), Some("20260910T160000Z"));
    assert_eq!(vevent.property("DTSTART").unwrap().parameter("TZID"), None);
}

#[test]
fn all_day_draft_uses_value_date() {
    let start = AllDay::start_of_day(&CalendarDate::new(2026, 9, 10), &la()).unwrap();
    let end = AllDay::start_of_day(&CalendarDate::new(2026, 9, 12), &la()).unwrap();
    let vevent = EventWriter::vevent_from(
        &draft(Some(EventTiming::new(start, end, Some(la()), true))),
        "u",
        now(),
        None,
    )
    .unwrap();
    assert_eq!(
        vevent.property("DTSTART").unwrap().parameter("VALUE"),
        Some("DATE")
    );
    assert_eq!(value(&vevent, "DTSTART"), Some("20260910"));
    assert_eq!(value(&vevent, "DTEND"), Some("20260912"));
    assert_eq!(value(&vevent, "RRULE"), Some("FREQ=WEEKLY;COUNT=3"));
}

#[test]
fn reminders_nil_and_empty_write_no_alarm_and_unsupported_ones_throw() {
    let mut no_reminders = draft(None);
    no_reminders.reminders = None;
    assert!(
        EventWriter::vevent_from(&no_reminders, "u", now(), None)
            .unwrap()
            .components_named("VALARM")
            .is_empty()
    );
    no_reminders.reminders = Some(vec![]);
    assert!(
        EventWriter::vevent_from(&no_reminders, "u", now(), None)
            .unwrap()
            .components_named("VALARM")
            .is_empty()
    );
    assert_eq!(
        AlarmMapper::alarms_from(&[
            Reminder::with_minutes_before(5),
            Reminder::before(5, ReminderType::Email { address: None }, None),
        ]),
        Err(reminders_unsupported())
    );
    assert_eq!(
        AlarmMapper::alarms_from(&[Reminder::new(ReminderTrigger::Location(
            StructuredLocation {
                title: Some("x".to_string()),
                ..StructuredLocation::default()
            },
            calendar_core::model::ReminderProximity::Enter,
        ))]),
        Err(reminders_unsupported())
    );
}

#[test]
fn alarms_round_trip() {
    let reminders = vec![
        Reminder::new(ReminderTrigger::Relative {
            offset: -600.0,
            to: ReminderAnchor::Start,
        }),
        Reminder {
            kind: ReminderType::Audio {
                sound_name: Some("Chord".to_string()),
            },
            ..Reminder::new(ReminderTrigger::Relative {
                offset: 0.0,
                to: ReminderAnchor::End,
            })
        },
        Reminder {
            repeat_count: 2,
            repeat_interval: Some(300.0),
            ..Reminder::new(ReminderTrigger::Absolute(at(1_790_521_200)))
        },
    ];
    let alarms = AlarmMapper::alarms_from(&reminders).unwrap();
    let back = AlarmMapper::reminders_from(&alarms);
    assert!(Reminder::same_set(&back, &reminders));
}

#[test]
fn attendees_write_with_organizer() {
    let mut with_guests = draft(None);
    with_guests.attendees = vec![
        AttendeeDraft::new("ann@example.test", Some("Ann"), AttendeeRole::Required),
        AttendeeDraft::new("room@example.test", None, AttendeeRole::Resource),
    ];
    let vevent =
        EventWriter::vevent_from(&with_guests, "u", now(), Some("mailto:me@icloud.test")).unwrap();
    assert_eq!(value(&vevent, "ORGANIZER"), Some("mailto:me@icloud.test"));
    let attendees = vevent.properties_named("ATTENDEE");
    let values: Vec<&str> = attendees.iter().map(|a| a.value.as_str()).collect();
    assert_eq!(
        values,
        vec![
            "mailto:me@icloud.test",
            "mailto:ann@example.test",
            "mailto:room@example.test"
        ]
    );
    assert_eq!(attendees[0].parameter("PARTSTAT"), Some("ACCEPTED"));
    assert!(
        attendees[1].parameter("PARTSTAT") == Some("NEEDS-ACTION")
            && attendees[1].parameter("RSVP") == Some("TRUE")
            && attendees[1].parameter("CN") == Some("Ann")
    );
    assert_eq!(attendees[2].parameter("CUTYPE"), Some("RESOURCE"));
}

#[test]
fn set_response_changes_only_the_accounts_entry() {
    let mut vevent = resource(WEEKLY_SERIES).master().unwrap().clone();
    let me: BTreeSet<String> = ["mailto:me@icloud.test".to_string()].into_iter().collect();
    assert!(AttendeeMapper::set_response(
        ResponseStatus::Declined,
        &mut vevent,
        &me
    ));
    let entries = vevent.properties_named("ATTENDEE");
    assert!(
        entries[0].parameter("PARTSTAT") == Some("DECLINED")
            && entries[0].parameter("RSVP").is_none()
    );
    assert_eq!(entries[1].parameter("PARTSTAT"), Some("TENTATIVE"));
    let someone: BTreeSet<String> = ["mailto:someone@else.test".to_string()]
        .into_iter()
        .collect();
    assert!(!AttendeeMapper::set_response(
        ResponseStatus::Accepted,
        &mut vevent,
        &someone
    ));
}

#[test]
fn patch_changes_only_touched_properties_and_keeps_unknown_ones() {
    let mut vevent = resource(WEEKLY_SERIES).master().unwrap().clone();
    let before = vevent.clone();
    let patch = hand_patch(|p| {
        p.title = Some("Renamed".to_string());
        p.location = FieldUpdate::Clear;
    });
    EventWriter::apply(&patch, &mut vevent, now(), None, None).unwrap();
    assert_eq!(
        vevent.property("SUMMARY").map(|p| p.text()).as_deref(),
        Some("Renamed")
    );
    assert!(vevent.property("LOCATION").is_none());
    assert_eq!(
        vevent.property("DESCRIPTION"),
        before.property("DESCRIPTION")
    );
    assert!(
        vevent
            .property("X-APPLE-TRAVEL-ADVISORY-BEHAVIOR")
            .is_some()
    );
    assert_eq!(value(&vevent, "SEQUENCE"), Some("2")); // content only: no bump
    assert_eq!(
        value(&vevent, "LAST-MODIFIED"),
        Some(ICalValues::utc_text(now()).as_str())
    );
}

#[test]
fn timing_and_attendee_changes_bump_the_sequence() {
    let mut vevent = resource(WEEKLY_SERIES).master().unwrap().clone();
    let timing = EventTiming::new(
        la_time!(2026, 9, 1, 11),
        la_time!(2026, 9, 1, 12),
        Some(la()),
        false,
    );
    let patch = hand_patch(|p| {
        p.timing = Some(timing);
        p.attendees = Some(AttendeeChanges::new(
            vec![AttendeeDraft::new(
                "new@example.test",
                None,
                AttendeeRole::Required,
            )],
            vec!["ann@example.test".to_string()],
        ));
    });
    EventWriter::apply(
        &patch,
        &mut vevent,
        now(),
        Some("mailto:me@icloud.test"),
        None,
    )
    .unwrap();
    assert_eq!(value(&vevent, "SEQUENCE"), Some("3"));
    assert_eq!(value(&vevent, "DTSTART"), Some("20260901T110000"));
    assert!(vevent.property("DURATION").is_none());
    let addresses: Vec<&str> = vevent
        .properties_named("ATTENDEE")
        .iter()
        .map(|a| a.value.as_str())
        .collect::<Vec<_>>();
    let addresses: Vec<String> = addresses.iter().map(|s| s.to_string()).collect();
    assert!(addresses.contains(&"mailto:new@example.test".to_string()));
    assert!(
        !addresses
            .iter()
            .any(|a| a.to_lowercase() == "mailto:ann@example.test")
    );
}

#[test]
fn clearing_reminders_is_unsupported() {
    let mut vevent = resource(WEEKLY_SERIES).master().unwrap().clone();
    let patch = hand_patch(|p| p.reminders = FieldUpdate::Clear);
    assert_eq!(
        EventWriter::apply(&patch, &mut vevent, now(), None, None),
        Err(reminders_unsupported())
    );
}

#[test]
fn recurrence_patch_replaces_and_clears() {
    let mut vevent = resource(WEEKLY_SERIES).master().unwrap().clone();
    let set = hand_patch(|p| {
        p.recurrence =
            FieldUpdate::Set(RecurrenceRule::from_rrule("FREQ=DAILY;COUNT=2", None).unwrap());
    });
    EventWriter::apply(&set, &mut vevent, now(), None, None).unwrap();
    let rules: Vec<&str> = vevent
        .properties_named("RRULE")
        .iter()
        .map(|p| p.value.as_str())
        .collect::<Vec<_>>();
    assert_eq!(rules, vec!["FREQ=DAILY;COUNT=2"]);
    let clear = hand_patch(|p| p.recurrence = FieldUpdate::Clear);
    EventWriter::apply(&clear, &mut vevent, now(), None, None).unwrap();
    assert!(
        vevent.property("RRULE").is_none()
            && vevent.property("EXDATE").is_none()
            && vevent.property("RDATE").is_none()
    );
}

#[test]
fn an_all_day_recurrence_until_is_written_in_the_calendar_zone() {
    let berlin = Zone::named("Europe/Berlin").unwrap();
    let mut vevent = resource(
        "BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Test//EN
BEGIN:VEVENT
UID:ALLDAY
DTSTAMP:20260301T000000Z
DTSTART;VALUE=DATE:20260305
DTEND;VALUE=DATE:20260306
RRULE:FREQ=DAILY
END:VEVENT
END:VCALENDAR",
    )
    .master()
    .unwrap()
    .clone();
    // Read the way EventReader reads a date UNTIL: in the calendar's zone.
    let rule = RecurrenceRule::from_rrule("FREQ=DAILY;UNTIL=20260310", Some(&berlin)).unwrap();
    let patch = hand_patch(|p| p.recurrence = FieldUpdate::Set(rule));
    EventWriter::apply(&patch, &mut vevent, now(), None, Some(&berlin)).unwrap();
    assert_eq!(value(&vevent, "RRULE"), Some("FREQ=DAILY;UNTIL=20260310"));
    let _ = civil::components;
}
