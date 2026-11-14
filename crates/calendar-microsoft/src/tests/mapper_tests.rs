//! Port of `MapperTests.swift`.

use std::collections::BTreeSet;

use calendar_core::all_day::{AllDay, CalendarDate};
use calendar_core::model::{
    AccessRole, AttendeeRole, Availability, CalendarDescriptor, CalendarEvent, CalendarService,
    ConferenceOrigin, ConferenceProvider, EventKind, EventStatus, Participation, Reminder,
    ReminderType, ResponseStatus, SeriesInfo, UidScope, Visibility,
};
use calendar_core::time::{Zone, add_seconds};
use calendar_test_support::all_day_conformance::AllDayConformance;
use jiff::Timestamp;
use serde_json::{Value, json};

use super::support::{decode_event, graph_event};
use crate::dtos::{GraphBodyDTO, GraphCalendarDTO};
use crate::event_mapper::GraphEventMapper;

fn la() -> Zone {
    Zone::named("America/Los_Angeles").unwrap()
}

fn la_calendar() -> CalendarDescriptor {
    let mut calendar = CalendarDescriptor::new("cal1", "Calendar", CalendarService::microsoft());
    calendar.time_zone = Some(la());
    calendar
}

fn at(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).unwrap()
}

fn map(extra: Value) -> CalendarEvent {
    GraphEventMapper::map(
        &decode_event(graph_event("ev1", extra)),
        &la_calendar(),
        Some("me@x.com"),
        Some("microsoft-c1"),
    )
    .unwrap()
}

#[test]
fn maps_a_timed_event() {
    let event = map(json!({}));
    assert!(
        event.event_id == "ev1"
            && event.uid.as_deref() == Some("uid-ev1")
            && event.uid_scope == Some(UidScope::Global)
            && event.calendar_id == "cal1"
    );
    assert!(
        event.title == "Standup"
            && event.notes.as_deref() == Some("Agenda")
            && event.location.as_deref() == Some("Room 1")
    );
    assert!(event.start == at(1_790_355_600) && event.end == add_seconds(event.start, 1800.0));
    assert!(
        event.time_zone.identifier() == "America/Los_Angeles"
            && !event.is_all_day
            && event.status == EventStatus::Confirmed
    );
    assert!(
        event.availability == Some(Availability::Busy)
            && event.visibility == Some(Visibility::Default)
            && event.kind == Some(EventKind::Standard)
    );
    assert!(
        event.series == Some(SeriesInfo::NotRecurring)
            && event.version.as_deref() == Some("ck1")
            && event.source_id.as_deref() == Some("microsoft-c1")
    );
    assert!(event.last_modified == Some(at(1_790_251_200)) && event.created.is_some());
    assert_eq!(
        event.url.as_ref().and_then(|url| url.host()).as_deref(),
        Some("outlook.office.com")
    );
    assert_eq!(
        event.reminders,
        Some(vec![Reminder::before(10, ReminderType::Display, None)])
    );
}

#[test]
fn a_timed_event_is_shown_in_the_zone_it_was_scheduled_in() {
    let event = map(json!({"originalStartTimeZone": "W. Europe Standard Time"}));
    assert_eq!(event.time_zone.identifier(), "Europe/Berlin");
    // The instant is unchanged.
    assert_eq!(event.start, at(1_790_355_600));
    let unknown = map(json!({"originalStartTimeZone": "Customized Time Zone"}));
    assert_eq!(unknown.time_zone.identifier(), "America/Los_Angeles");
}

#[test]
fn maps_attendees_organizer_and_participation() {
    let event = map(json!({}));
    assert_eq!(
        event
            .attendees
            .iter()
            .map(|a| a.email())
            .collect::<Vec<_>>(),
        [Some("me@x.com"), Some("opt@x.com"), Some("room@x.com")]
    );
    assert_eq!(
        event.attendees.iter().map(|a| a.role).collect::<Vec<_>>(),
        [
            AttendeeRole::Required,
            AttendeeRole::Optional,
            AttendeeRole::Resource
        ]
    );
    assert_eq!(
        event
            .attendees
            .iter()
            .map(|a| a.response)
            .collect::<Vec<_>>(),
        [
            ResponseStatus::Accepted,
            ResponseStatus::Tentative,
            ResponseStatus::NeedsAction
        ]
    );
    assert_eq!(
        event
            .attendees
            .iter()
            .map(|a| a.is_self)
            .collect::<Vec<_>>(),
        [true, false, false]
    );
    let organizer = event.organizer.as_ref().unwrap();
    assert!(
        organizer.email() == Some("boss@x.com") && organizer.is_organizer && !organizer.is_self
    );
    assert_eq!(
        event.participation,
        Some(Participation::Invited(ResponseStatus::Accepted))
    );
}

#[test]
fn an_event_you_organize_with_no_guests_counts_as_accepted() {
    let event = map(
        json!({"isOrganizer": true, "attendees": [], "organizer": {"emailAddress": {"name": "Me", "address": "me@x.com"}}}),
    );
    assert_eq!(
        event.participation,
        Some(Participation::Invited(ResponseStatus::Accepted))
    );
    assert!(event.organizer.as_ref().unwrap().is_self);
    let stranger = map(json!({"attendees": []}));
    assert_eq!(stranger.participation, Some(Participation::NotInvited));
}

#[test]
fn maps_show_as_and_sensitivity() {
    for (raw, expected) in [
        ("free", Availability::Free),
        ("tentative", Availability::Tentative),
        ("oof", Availability::Unavailable),
        ("workingElsewhere", Availability::Free),
        ("busy", Availability::Busy),
    ] {
        assert_eq!(
            map(json!({"showAs": raw})).availability,
            Some(expected),
            "{raw}"
        );
    }
    for (raw, expected) in [
        ("normal", Visibility::Default),
        ("personal", Visibility::PrivateEvent),
        ("private", Visibility::PrivateEvent),
        ("confidential", Visibility::Confidential),
    ] {
        assert_eq!(
            map(json!({"sensitivity": raw})).visibility,
            Some(expected),
            "{raw}"
        );
    }
}

#[test]
fn an_all_day_event_uses_the_canonical_form_in_the_account_zone() {
    let event = map(json!({
        "isAllDay": true,
        "start": {"dateTime": "2026-09-25T00:00:00.0000000", "timeZone": "Pacific Standard Time"},
        "end": {"dateTime": "2026-09-27T00:00:00.0000000", "timeZone": "Pacific Standard Time"},
    }));
    assert!(event.is_all_day && event.time_zone.identifier() == "America/Los_Angeles");
    assert!(AllDayConformance::violations(&event).is_empty());
    let (first, end_exclusive) = AllDay::dates(event.start, event.end, &event.time_zone);
    assert!(
        first == CalendarDate::new(2026, 9, 25) && end_exclusive == CalendarDate::new(2026, 9, 27)
    );
}

#[test]
fn an_all_day_event_in_utc_keeps_its_dates() {
    let mut utc = CalendarDescriptor::new("cal1", "C", CalendarService::microsoft());
    utc.time_zone = Some(Zone::named("UTC").unwrap());
    let dto = decode_event(graph_event(
        "ev1",
        json!({
            "isAllDay": true, "originalStartTimeZone": "UTC",
            "start": {"dateTime": "2026-09-25T00:00:00.0000000", "timeZone": "UTC"},
            "end": {"dateTime": "2026-09-26T00:00:00.0000000", "timeZone": "UTC"},
        }),
    ));
    let event = GraphEventMapper::map(&dto, &utc, None, None).unwrap();
    assert!(event.start == at(1_790_294_400) && event.end == add_seconds(event.start, 86_400.0));
    assert!(AllDayConformance::violations(&event).is_empty());
}

#[test]
fn a_cancelled_or_unreadable_event_maps_to_nil() {
    assert!(
        GraphEventMapper::map(
            &decode_event(graph_event("ev1", json!({"isCancelled": true}))),
            &la_calendar(),
            None,
            None
        )
        .is_none()
    );
    assert!(
        GraphEventMapper::map(
            &decode_event(graph_event("ev1", json!({"start": {"dateTime": "junk"}}))),
            &la_calendar(),
            None,
            None
        )
        .is_none()
    );
}

#[test]
fn maps_series_instances_masters_and_exceptions() {
    let occurrence = map(
        json!({"type": "occurrence", "seriesMasterId": "master1", "originalStart": "2026-09-25T17:00:00Z"}),
    );
    assert_eq!(
        occurrence.series,
        Some(SeriesInfo::Occurrence {
            series_id: "master1".to_string(),
            original_start: Some(at(1_790_355_600))
        })
    );
    let exception = map(
        json!({"type": "exception", "seriesMasterId": "master1", "originalStart": "2026-09-24T17:00:00Z"}),
    );
    assert!(
        exception.series_id() == Some("master1")
            && exception.original_start() == Some(at(1_790_269_200))
    );
    let master = map(json!({"type": "seriesMaster", "id": "master1"}));
    assert_eq!(
        master.series,
        Some(SeriesInfo::Occurrence {
            series_id: "master1".to_string(),
            original_start: Some(master.start)
        })
    );
}

#[test]
fn finds_the_teams_link_in_structured_data_and_in_the_notes() {
    let structured = map(
        json!({"isOnlineMeeting": true, "onlineMeeting": {"joinUrl": "https://teams.microsoft.com/l/meetup-join/abc"}}),
    );
    assert!(
        structured.conferences.first().map(|c| c.provider) == Some(ConferenceProvider::Teams)
            && structured.conferences.first().map(|c| c.origin)
                == Some(ConferenceOrigin::Structured)
    );
    let in_notes = map(
        json!({"body": {"contentType": "text", "content": "Join https://zoom.us/j/12345 please"}}),
    );
    assert!(
        in_notes.conferences.first().map(|c| c.provider) == Some(ConferenceProvider::Zoom)
            && in_notes.conferences.first().map(|c| c.origin) == Some(ConferenceOrigin::Notes)
    );
    assert!(map(json!({})).conferences.is_empty());
}

#[test]
fn reminders_follow_the_events_flag() {
    assert_eq!(
        map(json!({"isReminderOn": false})).reminders,
        Some(Vec::new())
    );
    assert_eq!(
        map(json!({"isReminderOn": true, "reminderMinutesBeforeStart": 0})).reminders,
        Some(vec![Reminder::before(0, ReminderType::Display, None)])
    );
}

#[test]
fn reduces_an_html_body_to_text() {
    let html = GraphBodyDTO {
        content_type: Some("html".to_string()),
        content: Some(
            "<html><body><p>Hello &amp; welcome</p><br/>Line two&nbsp;here</body></html>"
                .to_string(),
        ),
    };
    assert_eq!(
        GraphEventMapper::plain_text(Some(&html)).as_deref(),
        Some("Hello & welcome\n\nLine two here")
    );
    let blank = GraphBodyDTO {
        content_type: Some("text".to_string()),
        content: Some("  ".to_string()),
    };
    assert_eq!(GraphEventMapper::plain_text(Some(&blank)), None);
    assert_eq!(GraphEventMapper::plain_text(None), None);
}

#[test]
fn maps_a_calendar() {
    let dto: GraphCalendarDTO = serde_json::from_value(json!({
        "id": "cal1", "name": "Work", "hexColor": "#0078d4", "isDefaultCalendar": true, "canEdit": true,
        "canShare": true, "canViewPrivateItems": true, "owner": {"name": "Me", "address": "me@x.com"},
    }))
    .unwrap();
    let calendar = GraphEventMapper::descriptor(&dto, Some("me@x.com"), &la());
    assert!(
        calendar.id == "cal1"
            && calendar.title == "Work"
            && calendar.color_hex() == Some("#0078D4")
            && calendar.is_default == Some(true)
    );
    assert!(
        calendar.service == CalendarService::microsoft()
            && calendar.provider == Some(calendar_core::model::CalendarProvider::microsoft())
            && calendar.account_name.as_deref() == Some("me@x.com")
    );
    assert!(calendar.time_zone == Some(la()) && calendar.access_role() == Some(AccessRole::Owner));
    assert_eq!(
        calendar.supported_availabilities,
        Some(BTreeSet::from([
            Availability::Busy,
            Availability::Free,
            Availability::Tentative,
            Availability::Unavailable
        ]))
    );
}

#[test]
fn a_read_only_calendar_and_missing_flags() {
    let dto: GraphCalendarDTO =
        serde_json::from_value(json!({"id": "c", "canEdit": false})).unwrap();
    let calendar = GraphEventMapper::descriptor(&dto, None, &la());
    assert!(
        calendar.title == "c"
            && !calendar.permissions.can_edit
            && calendar.permissions.can_share == Some(false)
    );
    assert!(
        calendar.access_role() == Some(AccessRole::Reader)
            && calendar.color_hex().is_none()
            && calendar.is_default == Some(false)
    );
}
