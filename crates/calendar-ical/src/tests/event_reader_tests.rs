//! Port of `EventReaderTests.swift`.

use std::collections::BTreeSet;

use calendar_core::all_day::{AllDay, CalendarDate};
use calendar_core::model::{
    AttendeeRole, Availability, ConferenceProvider, EventStatus, Participation, Reminder,
    ReminderAnchor, ReminderTrigger, ResponseStatus, SeriesInfo, UidScope, Visibility,
};
use calendar_core::recurrence::rule::Frequency;
use calendar_core::source_types::{ProvidedField, SourceCapabilities};
use calendar_core::time::DateInterval;
use calendar_test_support::all_day_conformance::AllDayConformance;
use calendar_test_support::provided_fields_conformance::ProvidedFieldsConformance;

use super::fixtures::{WEEKLY_SERIES, at, context, context_named, la, la_time, resource};
use crate::event_reader::EventReader;

fn september() -> DateInterval {
    DateInterval::new(la_time!(2026, 9, 1, 0), la_time!(2026, 10, 1, 0))
}

fn occurrence(series_id: &str, original_start: jiff::Timestamp) -> Option<SeriesInfo> {
    Some(SeriesInfo::Occurrence {
        series_id: series_id.to_string(),
        original_start: Some(original_start),
    })
}

fn weekly_events() -> Vec<calendar_core::model::CalendarEvent> {
    EventReader::events_in(&resource(WEEKLY_SERIES), &september(), &context())
}

#[test]
fn weekly_series_expands_with_exdate_and_moved_override() {
    let events = weekly_events();
    let starts: Vec<_> = events.iter().map(|e| e.start).collect();
    assert_eq!(
        starts,
        vec![
            la_time!(2026, 9, 1),
            la_time!(2026, 9, 16, 14),
            la_time!(2026, 9, 22),
            la_time!(2026, 9, 29)
        ]
    );
    let moved = &events[1];
    assert_eq!(moved.title, "Team sync (moved)");
    assert_eq!(moved.series, occurrence("4F2A.ics", la_time!(2026, 9, 15)));
    assert_eq!(moved.event_id, "4F2A.ics#20260915T170000Z");
    assert_eq!(events[0].event_id, "4F2A.ics#20260901T170000Z");
    assert_eq!(events[0].end, la_time!(2026, 9, 1, 10, 30));
}

#[test]
fn maps_every_field() {
    let events = weekly_events();
    let first = &events[0];
    assert_eq!(first.uid.as_deref(), Some("4F2A-UID"));
    assert_eq!(first.uid_scope, Some(UidScope::Global));
    assert_eq!(first.calendar_id, "home");
    assert_eq!(first.source_id.as_deref(), Some("icloud-conn"));
    assert_eq!(first.title, "Team sync");
    assert_eq!(
        first.notes.as_deref(),
        Some("Agenda\nhttps://zoom.us/j/123456789")
    );
    assert_eq!(first.location.as_deref(), Some("Room 4"));
    assert_eq!(first.time_zone.identifier(), "America/Los_Angeles");
    assert!(!first.is_all_day);
    assert_eq!(first.status, EventStatus::Confirmed);
    assert_eq!(first.availability, Some(Availability::Busy));
    assert_eq!(first.visibility, Some(Visibility::PrivateEvent));
    assert_eq!(first.version.as_deref(), Some("\"e1\""));
    assert_eq!(first.last_modified, Some(at(1_788_350_400))); // 2026-09-02T12:00:00Z
    assert_eq!(first.created, Some(at(1_785_585_600))); // 2026-08-01T12:00:00Z
    assert_eq!(
        first.conference().map(|c| c.provider),
        Some(ConferenceProvider::Zoom)
    );
    assert_eq!(
        first.reminders,
        Some(vec![Reminder {
            is_calendar_default: Some(true),
            ..Reminder::new(ReminderTrigger::Relative {
                offset: -900.0,
                to: ReminderAnchor::Start
            })
        }])
    );
}

#[test]
fn attendees_self_and_participation() {
    let events = weekly_events();
    let first = &events[0];
    assert_eq!(
        first.organizer.as_ref().and_then(|o| o.email()),
        Some("me@icloud.test")
    );
    assert_eq!(first.organizer.as_ref().map(|o| o.is_self), Some(true));
    let emails: Vec<Option<&str>> = first.attendees.iter().map(|a| a.email()).collect();
    assert_eq!(
        emails,
        vec![
            Some("me@icloud.test"),
            Some("ann@example.test"),
            Some("room@example.test"),
            None
        ]
    );
    let responses: Vec<_> = first.attendees.iter().map(|a| a.response).collect();
    assert_eq!(
        responses,
        vec![
            ResponseStatus::Accepted,
            ResponseStatus::Tentative,
            ResponseStatus::Accepted,
            ResponseStatus::NeedsAction
        ]
    );
    let roles: Vec<_> = first.attendees.iter().map(|a| a.role).collect();
    assert_eq!(
        roles,
        vec![
            AttendeeRole::Required,
            AttendeeRole::Required,
            AttendeeRole::Resource,
            AttendeeRole::Optional
        ]
    );
    assert!(first.attendees[0].is_self && first.attendees[0].is_organizer);
    assert_eq!(first.attendees[3].name.as_deref(), Some("Bo"));
    assert_eq!(
        first.participation,
        Some(Participation::Invited(ResponseStatus::Accepted))
    );
}

#[test]
fn no_self_addresses_gives_nil_participation() {
    let events = EventReader::events_in(
        &resource(WEEKLY_SERIES),
        &september(),
        &context_named("4F2A.ics", &[]),
    );
    let first = &events[0];
    assert_eq!(first.participation, None);
    assert!(first.attendees.iter().all(|a| !a.is_self));
}

#[test]
fn override_moved_into_window_is_shown_and_its_slot_is_not() {
    // A window holding only Wednesday the 16th: the override is in it; its slot (Tuesday the 15th) is not.
    let window = DateInterval::new(la_time!(2026, 9, 16, 0), la_time!(2026, 9, 17, 0));
    let events = EventReader::events_in(&resource(WEEKLY_SERIES), &window, &context());
    let titles: Vec<&str> = events.iter().map(|e| e.title.as_str()).collect();
    assert_eq!(titles, vec!["Team sync (moved)"]);
    // A window holding only Tuesday the 15th shows nothing: the occurrence moved away.
    let slot = DateInterval::new(la_time!(2026, 9, 15, 0), la_time!(2026, 9, 16, 0));
    assert!(EventReader::events_in(&resource(WEEKLY_SERIES), &slot, &context()).is_empty());
}

#[test]
fn override_without_master_is_shown() {
    let text = "BEGIN:VCALENDAR
VERSION:2.0
BEGIN:VEVENT
UID:only-one
RECURRENCE-ID:20260915T170000Z
DTSTART:20260915T170000Z
DTEND:20260915T180000Z
SUMMARY:Just this one
END:VEVENT
END:VCALENDAR";
    let events = EventReader::events_in(
        &resource(text),
        &september(),
        &context_named("only.ics", &["mailto:me@icloud.test"]),
    );
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].series, occurrence("only.ics", at(1_789_491_600)));
    let identifier = events[0].time_zone.identifier();
    assert!(identifier == "UTC" || identifier == "GMT");
}

#[test]
fn cancelled_override_is_returned_cancelled() {
    let text = WEEKLY_SERIES.replace(
        "SUMMARY:Team sync (moved)",
        "SUMMARY:Team sync (moved)\nSTATUS:CANCELLED",
    );
    let events = EventReader::events_in(&resource(&text), &september(), &context());
    let moved = events
        .iter()
        .find(|e| e.title == "Team sync (moved)")
        .unwrap();
    assert_eq!(moved.status, EventStatus::Cancelled);
}

#[test]
fn single_event_is_not_recurring() {
    let text = "BEGIN:VCALENDAR
VERSION:2.0
BEGIN:VEVENT
UID:single
DTSTART;TZID=America/Los_Angeles:20260910T090000
DURATION:PT45M
SUMMARY:One off
TRANSP:TRANSPARENT
END:VEVENT
END:VCALENDAR";
    let events = EventReader::events_in(
        &resource(text),
        &september(),
        &context_named("single.ics", &["mailto:me@icloud.test"]),
    );
    let event = &events[0];
    assert_eq!(event.event_id, "single.ics");
    assert_eq!(event.series, Some(SeriesInfo::NotRecurring));
    assert_eq!(event.end, la_time!(2026, 9, 10, 9, 45));
    assert_eq!(event.availability, Some(Availability::Free));
    assert_eq!(event.visibility, Some(Visibility::Default));
    assert_eq!(event.reminders, Some(vec![]));
    assert_eq!(event.participation, Some(Participation::NotInvited));
}

#[test]
fn all_day_uses_the_calendar_zone_and_passes_conformance() {
    let text = "BEGIN:VCALENDAR
VERSION:2.0
BEGIN:VEVENT
UID:day
DTSTART;VALUE=DATE:20260910
DTEND;VALUE=DATE:20260912
RRULE:FREQ=WEEKLY;COUNT=2
SUMMARY:Offsite
END:VEVENT
END:VCALENDAR";
    let events = EventReader::events_in(
        &resource(text),
        &september(),
        &context_named("day.ics", &["mailto:me@icloud.test"]),
    );
    assert_eq!(events.len(), 2);
    for event in &events {
        assert!(event.is_all_day);
        assert_eq!(AllDayConformance::violations(event), Vec::<String>::new());
    }
    assert_eq!(
        events[0].start,
        AllDay::start_of_day(&CalendarDate::new(2026, 9, 10), &la()).unwrap()
    );
    assert_eq!(
        events[0].end,
        AllDay::start_of_day(&CalendarDate::new(2026, 9, 12), &la()).unwrap()
    );
    assert_eq!(events[0].event_id, "day.ics#20260910");
}

#[test]
fn unreadable_rule_shows_first_occurrence_and_overrides() {
    let text = WEEKLY_SERIES.replace("RRULE:FREQ=WEEKLY;BYDAY=TU", "RRULE:FREQ=FORTNIGHTLY");
    let events = EventReader::events_in(&resource(&text), &september(), &context());
    let starts: Vec<_> = events.iter().map(|e| e.start).collect();
    assert_eq!(
        starts,
        vec![la_time!(2026, 9, 1), la_time!(2026, 9, 16, 14)]
    );
}

#[test]
fn windows_tzid_is_resolved() {
    let text = WEEKLY_SERIES.replace("TZID=America/Los_Angeles", "TZID=Pacific Standard Time");
    let events = EventReader::events_in(&resource(&text), &september(), &context());
    assert_eq!(events[0].start, la_time!(2026, 9, 1));
}

#[test]
fn master_occurrence_and_series_lookups() {
    let r = resource(WEEKLY_SERIES);
    let master = EventReader::master_event_of(&r, &context()).unwrap();
    assert_eq!(master.event_id, "4F2A.ics");
    assert_eq!(master.series, occurrence("4F2A.ics", la_time!(2026, 9, 1)));
    assert_eq!(
        EventReader::occurrence_in(&r, la_time!(2026, 9, 22), &context())
            .map(|e| e.event_id)
            .as_deref(),
        Some("4F2A.ics#20260922T170000Z")
    );
    assert_eq!(
        EventReader::occurrence_in(&r, la_time!(2026, 9, 15), &context())
            .map(|e| e.title)
            .as_deref(),
        Some("Team sync (moved)")
    );
    assert!(EventReader::occurrence_in(&r, la_time!(2026, 9, 8), &context()).is_none()); // excluded
    assert!(EventReader::occurrence_in(&r, la_time!(2026, 9, 23), &context()).is_none()); // not a slot
    let series = EventReader::series_of(&r, &context()).unwrap();
    assert_eq!(series.series_id, "4F2A.ics");
    assert_eq!(
        series.recurrence.rules.first().map(|rule| rule.frequency),
        Some(Frequency::Weekly)
    );
    assert_eq!(
        series.recurrence.excluded_dates,
        Some(vec![la_time!(2026, 9, 8)])
    );
}

#[test]
fn read_events_meet_the_provided_field_rule() {
    let provided: BTreeSet<ProvidedField> = [
        ProvidedField::Visibility,
        ProvidedField::Availability,
        ProvidedField::Reminders,
        ProvidedField::Series,
        ProvidedField::Participation,
        ProvidedField::Version,
        ProvidedField::UidScope,
    ]
    .into_iter()
    .collect();
    let capabilities = SourceCapabilities {
        provided_fields: provided,
        ..SourceCapabilities::default()
    };
    for event in weekly_events() {
        assert_eq!(
            ProvidedFieldsConformance::event_violations(&event, &capabilities),
            Vec::<String>::new()
        );
    }
}
