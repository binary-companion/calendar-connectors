//! Port of `MapperTests.swift`.

use calendar_core::civil;
use calendar_core::model::{
    AccessRole, Availability, CalendarDescriptor, CalendarEvent, CalendarKind, CalendarPermissions,
    CalendarProvider, CalendarService, ConferenceOrigin, ConferenceProvider, EventKind,
    EventStatus, Participation, Reminder, ReminderType, ResponseStatus, SeriesInfo, Visibility,
};
use calendar_core::source_types::{ProvidedField, SourceCapabilities};
use calendar_core::time::{Zone, seconds_between};
use calendar_test_support::all_day_conformance::AllDayConformance;
use calendar_test_support::provided_fields_conformance::ProvidedFieldsConformance;

use super::support::instant;
use crate::dtos::{GoogleCalendarListEntryDTO, GoogleEventDTO};
use crate::event_mapper::GoogleEventMapper;

fn tokyo() -> Zone {
    Zone::named("Asia/Tokyo").unwrap()
}

fn all_rights() -> CalendarPermissions {
    CalendarPermissions {
        can_view_details: true,
        can_edit: true,
        can_share: Some(true),
        can_view_private: Some(true),
    }
}

fn calendar() -> CalendarDescriptor {
    let mut calendar = CalendarDescriptor::new("cal1", "Work", CalendarService::google());
    calendar.permissions = all_rights();
    calendar.is_default = Some(true);
    calendar.time_zone = Some(tokyo());
    calendar
}

fn event(json: &str) -> GoogleEventDTO {
    serde_json::from_str(json).unwrap()
}

fn map(json: &str) -> Option<CalendarEvent> {
    GoogleEventMapper::map(&event(json), &calendar(), None)
}

fn entry(json: &str) -> GoogleCalendarListEntryDTO {
    serde_json::from_str(json).unwrap()
}

#[test]
fn maps_a_timed_event() {
    let e = map(r#"
    {"id":"e1","iCalUID":"uid@google.com","status":"confirmed","summary":"Standup","description":"notes","location":"Room 1",
     "htmlLink":"https://www.google.com/calendar/event?eid=x","etag":"\"123\"",
     "start":{"dateTime":"2026-09-21T10:00:00-07:00","timeZone":"America/Los_Angeles"},
     "end":{"dateTime":"2026-09-21T10:30:00-07:00","timeZone":"America/Los_Angeles"}}
    "#)
    .unwrap();
    assert!(e.event_id == "e1" && e.calendar_id == "cal1" && e.id() == "cal1/e1");
    assert_eq!(e.uid.as_deref(), Some("uid@google.com"));
    assert!(
        e.title == "Standup"
            && e.notes.as_deref() == Some("notes")
            && e.location.as_deref() == Some("Room 1")
    );
    assert!(
        e.start == instant("2026-09-21T10:00:00-07:00")
            && e.end == instant("2026-09-21T10:30:00-07:00")
    );
    assert_eq!(e.time_zone.identifier(), "America/Los_Angeles");
    assert!(!e.is_all_day && e.status == EventStatus::Confirmed);
    assert!(e.availability == Some(Availability::Busy) && e.kind == Some(EventKind::Standard));
    assert_eq!(
        e.url.as_ref().map(|u| u.as_str()),
        Some("https://www.google.com/calendar/event?eid=x")
    );
    assert_eq!(e.version.as_deref(), Some("\"123\""));
    assert!(e.my_response().is_none() && e.attendees.is_empty());
}

#[test]
fn all_day_events_use_midnight_in_the_calendar_zone_with_exclusive_end() {
    let e = map(
        r#"{"id":"a","summary":"Off","start":{"date":"2026-09-20"},"end":{"date":"2026-09-22"}}"#,
    )
    .unwrap();
    assert!(e.is_all_day);
    assert!(AllDayConformance::violations(&e).is_empty());
    assert_eq!(e.time_zone.identifier(), "Asia/Tokyo");
    assert_eq!(
        e.start,
        civil::date_from(2026, 9, 20, 0, 0, 0, &tokyo()).unwrap()
    );
    assert_eq!(
        e.end,
        civil::date_from(2026, 9, 22, 0, 0, 0, &tokyo()).unwrap()
    );
}

#[test]
fn all_day_falls_back_to_utc_when_the_calendar_has_no_zone() {
    let bare = CalendarDescriptor::new("c", "C", CalendarService::google());
    let dto = event(r#"{"id":"a","start":{"date":"2026-09-20"},"end":{"date":"2026-09-21"}}"#);
    let e = GoogleEventMapper::map(&dto, &bare, None).unwrap();
    assert!(e.time_zone.identifier() == "UTC" || e.time_zone.identifier() == "GMT");
    assert_eq!(e.start, instant("2026-09-20T00:00:00Z"));
    assert!(AllDayConformance::violations(&e).is_empty());
}

#[test]
fn a_timed_event_without_its_own_zone_takes_the_calendars_zone_then_utc() {
    let no_zone = r#"{"id":"t","start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"}}"#;
    // The fixture calendar's zone.
    assert_eq!(map(no_zone).unwrap().time_zone.identifier(), "Asia/Tokyo");
    let bare = CalendarDescriptor::new("c", "C", CalendarService::google());
    let e = GoogleEventMapper::map(&event(no_zone), &bare, None).unwrap();
    assert!(e.time_zone.identifier() == "UTC" || e.time_zone.identifier() == "GMT");
    let own = r#"{"id":"t","start":{"dateTime":"2026-09-21T10:00:00-07:00","timeZone":"America/Los_Angeles"},"end":{"dateTime":"2026-09-21T11:00:00-07:00","timeZone":"America/Los_Angeles"}}"#;
    assert_eq!(
        map(own).unwrap().time_zone.identifier(),
        "America/Los_Angeles"
    );
}

#[test]
fn all_day_in_tokyo_is_canonical() {
    let e = map(
        r#"{"id":"a","summary":"Off","start":{"date":"2026-09-18"},"end":{"date":"2026-09-19"}}"#,
    )
    .unwrap();
    assert_eq!(e.start, instant("2026-09-17T15:00:00Z"));
    assert_eq!(e.end, instant("2026-09-18T15:00:00Z"));
    assert_eq!(e.time_zone.identifier(), "Asia/Tokyo");
    assert!(AllDayConformance::violations(&e).is_empty());
}

#[test]
fn cancelled_events_are_dropped() {
    assert!(map(r#"{"id":"x","status":"cancelled","start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"}}"#).is_none());
}

#[test]
fn maps_attendees_organizer_and_my_response() {
    let e = map(r#"
    {"id":"m","summary":"Sync","start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"},
     "organizer":{"email":"Boss@x.com","displayName":"Boss"},
     "attendees":[{"email":"Boss@x.com","responseStatus":"accepted","organizer":true},
                  {"email":"me@x.com","responseStatus":"declined","self":true},
                  {"email":"opt@x.com","optional":true,"responseStatus":"tentative"},
                  {"email":"room@x.com","resource":true}]}
    "#)
    .unwrap();
    assert_eq!(e.attendees.len(), 4);
    let organizer = e.organizer.as_ref().unwrap();
    assert!(organizer.email() == Some("boss@x.com") && organizer.is_organizer);
    assert_eq!(e.my_response(), Some(ResponseStatus::Declined));
    assert!(e.attendees[1].is_self);
    assert!(
        e.attendees[2].role == calendar_core::model::AttendeeRole::Optional
            && e.attendees[2].response == ResponseStatus::Tentative
    );
    assert!(
        e.attendees[3].role == calendar_core::model::AttendeeRole::Resource
            && e.attendees[3].response == ResponseStatus::NeedsAction
    );
}

#[test]
fn maps_conference_from_entry_points_then_hangout_link() {
    let a = map(r#"
    {"id":"c1","start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"},
     "conferenceData":{"entryPoints":[{"entryPointType":"phone","uri":"tel:+1"},{"entryPointType":"video","uri":"https://meet.google.com/abc-defg-hij"}],
       "conferenceSolution":{"key":{"type":"hangoutsMeet"},"name":"Google Meet"}}}
    "#)
    .unwrap();
    assert_eq!(
        a.conference().map(|c| c.url.as_str()),
        Some("https://meet.google.com/abc-defg-hij")
    );
    assert_eq!(
        a.conference().map(|c| c.provider),
        Some(ConferenceProvider::Meet)
    );
    let b = map(r#"
    {"id":"c2","start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"},
     "conferenceData":{"entryPoints":[{"entryPointType":"video","uri":"https://acme.zoom.us/j/123"}],"conferenceSolution":{"key":{"type":"addOn"},"name":"Zoom Meeting"}}}
    "#)
    .unwrap();
    assert_eq!(
        b.conference().map(|c| c.provider),
        Some(ConferenceProvider::Zoom)
    );
    let c = map(r#"{"id":"c3","hangoutLink":"https://meet.google.com/zzz","start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"}}"#).unwrap();
    assert_eq!(
        c.conference().map(|c| c.provider),
        Some(ConferenceProvider::Meet)
    );
}

#[test]
fn a_teams_link_only_in_the_description_is_found() {
    // An invite imported from ICS carries its link in the description, not in `conferenceData`.
    let e = map(r#"
    {"id":"c4","description":"Join https://teams.microsoft.com/l/meetup-join/19%3ameeting_x/0","start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"}}
    "#)
    .unwrap();
    assert_eq!(
        e.conferences.iter().map(|c| c.provider).collect::<Vec<_>>(),
        [ConferenceProvider::Teams]
    );
    assert_eq!(e.conferences[0].origin, ConferenceOrigin::Notes);
}

#[test]
fn structured_links_come_before_ones_found_in_the_description() {
    let e = map(r#"
    {"id":"c5","description":"Backup: https://acme.zoom.us/j/9","hangoutLink":"https://meet.google.com/aaa-bbbb-ccc",
     "start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"},
     "htmlLink":"https://www.google.com/calendar/event?eid=abc"}
    "#)
    .unwrap();
    assert_eq!(
        e.conferences.iter().map(|c| c.provider).collect::<Vec<_>>(),
        [ConferenceProvider::Meet, ConferenceProvider::Zoom]
    );
    assert_eq!(
        e.conferences.iter().map(|c| c.origin).collect::<Vec<_>>(),
        [ConferenceOrigin::Structured, ConferenceOrigin::Notes]
    );
}

#[test]
fn every_video_entry_point_is_kept() {
    let e = map(r#"
    {"id":"c6","start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"},
     "conferenceData":{"entryPoints":[{"entryPointType":"video","uri":"https://acme.zoom.us/j/1"},{"entryPointType":"video","uri":"https://meet.google.com/aaa-bbbb-ccc"}]}}
    "#)
    .unwrap();
    assert_eq!(
        e.conferences.iter().map(|c| c.provider).collect::<Vec<_>>(),
        [ConferenceProvider::Zoom, ConferenceProvider::Meet]
    );
}

#[test]
fn maps_kind_availability_visibility_reminders_and_series() {
    let e = map(r#"
    {"id":"r_20260921","recurringEventId":"r","eventType":"focusTime","transparency":"transparent","visibility":"private",
     "originalStartTime":{"dateTime":"2026-09-21T10:00:00Z"},
     "reminders":{"useDefault":false,"overrides":[{"method":"popup","minutes":10},{"method":"email","minutes":60}]},
     "start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"}}
    "#)
    .unwrap();
    assert!(
        e.kind == Some(EventKind::FocusTime)
            && e.availability == Some(Availability::Free)
            && e.visibility == Some(Visibility::PrivateEvent)
    );
    assert!(
        e.series_id() == Some("r") && e.original_start() == Some(instant("2026-09-21T10:00:00Z"))
    );
    assert_eq!(
        e.reminders,
        Some(vec![
            Reminder::before(10, ReminderType::Display, Some(false)),
            Reminder::before(60, ReminderType::Email { address: None }, Some(false)),
        ])
    );
}

#[test]
fn tolerates_fractional_seconds_and_missing_title() {
    let e = map(r#"{"id":"f","start":{"dateTime":"2026-09-21T10:00:00.500Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"}}"#).unwrap();
    assert_eq!(e.title, "(No title)");
    assert_eq!(e.start, instant("2026-09-21T10:00:00.5Z"));
}

#[test]
fn events_without_usable_times_are_dropped() {
    assert!(
        map(r#"{"id":"bad","start":{"dateTime":"garbage"},"end":{"dateTime":"garbage"}}"#)
            .is_none()
    );
    assert!(map(r#"{"id":"bad2"}"#).is_none());
}

#[test]
fn maps_calendar_list_entries() {
    let dto = entry(
        r##"
    {"id":"me@x.com","summary":"me@x.com","summaryOverride":"Personal","backgroundColor":"#9fe1e7","accessRole":"owner","primary":true,"timeZone":"Asia/Tokyo"}
    "##,
    );
    let d = GoogleEventMapper::descriptor(&dto, Some("me@x.com")).unwrap();
    assert!(d.id == "me@x.com" && d.title == "Personal" && d.color_hex() == Some("#9FE1E7"));
    assert!(
        d.access_role() == Some(AccessRole::Owner)
            && d.is_default == Some(true)
            && d.time_zone.as_ref().map(|z| z.identifier()) == Some("Asia/Tokyo")
            && d.account_name.as_deref() == Some("me@x.com")
    );
    let hidden = entry(r#"{"id":"h","hidden":true}"#);
    assert!(GoogleEventMapper::descriptor(&hidden, None).is_none());
    let deleted = entry(r#"{"id":"d","deleted":true}"#);
    assert!(GoogleEventMapper::descriptor(&deleted, None).is_none());
}

#[test]
fn google_birthday_and_holiday_calendars_are_not_standard() {
    fn kind(id: &str) -> CalendarKind {
        let dto = entry(&format!(r#"{{"id":"{id}","summary":"x"}}"#));
        GoogleEventMapper::descriptor(&dto, None).unwrap().kind
    }
    assert_eq!(
        kind("addressbook#contacts@group.v.calendar.google.com"),
        CalendarKind::Birthdays
    );
    assert_eq!(
        kind("en.usa#holiday@group.v.calendar.google.com"),
        CalendarKind::Subscribed
    );
    assert_eq!(kind("me@x.com"), CalendarKind::Standard);
    assert_eq!(
        kind("abc123@group.calendar.google.com"),
        CalendarKind::Standard
    );
}

// Provided fields (Issue 2).

fn with_defaults() -> CalendarDescriptor {
    let mut calendar = CalendarDescriptor::new("cal1", "Work", CalendarService::google());
    calendar.permissions = all_rights();
    calendar.time_zone = Some(tokyo());
    calendar.default_reminders = Some(vec![
        Reminder::with_minutes_before(30),
        Reminder::with_minutes_before(5),
    ]);
    calendar
}

const TIMES: &str =
    r#""start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"}"#;

#[test]
fn use_default_reminders_resolve_to_the_calendars_defaults() {
    let dto = event(&format!(
        r#"{{"id":"d","reminders":{{"useDefault":true}},{TIMES}}}"#
    ));
    assert_eq!(
        GoogleEventMapper::map(&dto, &with_defaults(), None)
            .unwrap()
            .reminders,
        Some(vec![
            Reminder::before(30, ReminderType::Display, Some(true)),
            Reminder::before(5, ReminderType::Display, Some(true)),
        ])
    );
    let none = event(&format!(
        r#"{{"id":"n","reminders":{{"useDefault":false}},{TIMES}}}"#
    ));
    assert_eq!(
        GoogleEventMapper::map(&none, &with_defaults(), None)
            .unwrap()
            .reminders,
        Some(Vec::new())
    );
}

#[test]
fn a_recurring_instance_is_an_occurrence_and_a_single_event_is_not_recurring() {
    let instance = map(&format!(
        r#"{{"id":"i","recurringEventId":"master","originalStartTime":{{"dateTime":"2026-09-21T10:00:00Z"}},{TIMES}}}"#
    ))
    .unwrap();
    assert_eq!(
        instance.series,
        Some(SeriesInfo::Occurrence {
            series_id: "master".to_string(),
            original_start: Some(instant("2026-09-21T10:00:00Z")),
        })
    );
    let single = map(&format!(r#"{{"id":"s",{TIMES}}}"#)).unwrap();
    assert_eq!(single.series, Some(SeriesInfo::NotRecurring));
}

#[test]
fn participation_separates_invited_from_not_invited() {
    let self_attendee = map(&format!(
        r#"{{"id":"a","attendees":[{{"email":"me@x.com","self":true,"responseStatus":"tentative"}}],{TIMES}}}"#
    ))
    .unwrap();
    assert_eq!(
        self_attendee.participation,
        Some(Participation::Invited(ResponseStatus::Tentative))
    );
    let organizer_only = map(&format!(
        r#"{{"id":"b","organizer":{{"email":"me@x.com","self":true}},{TIMES}}}"#
    ))
    .unwrap();
    assert_eq!(
        organizer_only.participation,
        Some(Participation::Invited(ResponseStatus::Accepted))
    );
    let someone_elses = map(&format!(
        r#"{{"id":"c","organizer":{{"email":"boss@x.com"}},"attendees":[{{"email":"boss@x.com"}}],{TIMES}}}"#
    ))
    .unwrap();
    assert_eq!(someone_elses.participation, Some(Participation::NotInvited));
}

#[test]
fn every_field_google_declares_is_present() {
    let capabilities = SourceCapabilities {
        provided_fields: [
            ProvidedField::Kind,
            ProvidedField::Visibility,
            ProvidedField::Availability,
            ProvidedField::Reminders,
            ProvidedField::Series,
            ProvidedField::Participation,
            ProvidedField::StructuredConference,
            ProvidedField::Version,
        ]
        .into(),
        ..SourceCapabilities::default()
    };
    let minimal = map(&format!(r#"{{"id":"m","etag":"\"1\"",{TIMES}}}"#)).unwrap();
    assert!(ProvidedFieldsConformance::event_violations(&minimal, &capabilities).is_empty());
    let dto = event(&format!(
        r#"{{"id":"r","etag":"\"2\"","reminders":{{"useDefault":true}},{TIMES}}}"#
    ));
    let mapped = GoogleEventMapper::map(&dto, &with_defaults(), None).unwrap();
    assert!(ProvidedFieldsConformance::event_violations(&mapped, &capabilities).is_empty());
}

#[test]
fn calendar_list_entries_carry_their_default_reminders() {
    let dto = entry(r#"{"id":"c","defaultReminders":[{"method":"popup","minutes":10}]}"#);
    assert_eq!(
        GoogleEventMapper::descriptor(&dto, None)
            .unwrap()
            .default_reminders,
        Some(vec![Reminder::before(
            10,
            ReminderType::Display,
            Some(true)
        )])
    );
}

// Calendar identity and permissions (Issues 6 and 7).

fn descriptor(json: &str) -> CalendarDescriptor {
    GoogleEventMapper::descriptor(&entry(json), Some("me@x.com")).unwrap()
}

fn rights(details: bool, edit: bool, share: bool, private: bool) -> CalendarPermissions {
    CalendarPermissions {
        can_view_details: details,
        can_edit: edit,
        can_share: Some(share),
        can_view_private: Some(private),
    }
}

#[test]
fn google_roles_map_to_permissions() {
    assert_eq!(
        descriptor(r#"{"id":"a","accessRole":"owner"}"#).permissions,
        rights(true, true, true, true)
    );
    assert_eq!(
        descriptor(r#"{"id":"a","accessRole":"writer"}"#).permissions,
        rights(true, true, false, true)
    );
    assert_eq!(
        descriptor(r#"{"id":"a","accessRole":"reader"}"#).permissions,
        rights(true, false, false, false)
    );
    assert_eq!(
        descriptor(r#"{"id":"a","accessRole":"freeBusyReader"}"#).permissions,
        rights(false, false, false, false)
    );
    assert_eq!(
        descriptor(r#"{"id":"a","accessRole":"owner"}"#).access_role(),
        Some(AccessRole::Owner)
    );
}

#[test]
fn google_calendars_carry_service_provider_default_and_availabilities() {
    let primary =
        descriptor(r#"{"id":"me@x.com","primary":true,"accessRole":"owner","timeZone":"UTC"}"#);
    assert!(
        primary.service == CalendarService::google()
            && primary.provider == Some(CalendarProvider::google())
            && primary.is_default == Some(true)
    );
    assert_eq!(
        primary.supported_availabilities,
        Some([Availability::Busy, Availability::Free].into())
    );
    assert_eq!(
        descriptor(r#"{"id":"b","accessRole":"reader"}"#).is_default,
        Some(false)
    );
    let holidays =
        descriptor(r#"{"id":"en.usa#holiday@group.v.calendar.google.com","accessRole":"reader"}"#);
    assert_eq!(holidays.provider, Some(CalendarProvider::subscription()));
}

#[test]
fn every_calendar_field_google_declares_is_present() {
    let capabilities = SourceCapabilities {
        provided_fields: [
            ProvidedField::IsDefault,
            ProvidedField::CalendarTimeZone,
            ProvidedField::DefaultReminders,
            ProvidedField::Provider,
            ProvidedField::SupportedAvailabilities,
            ProvidedField::PermissionDetails,
        ]
        .into(),
        ..SourceCapabilities::default()
    };
    let d = descriptor(r#"{"id":"me@x.com","primary":true,"accessRole":"owner","timeZone":"UTC"}"#);
    assert!(ProvidedFieldsConformance::calendar_violations(&d, &capabilities).is_empty());
    let bare = CalendarDescriptor::new("c", "C", CalendarService::event_kit());
    assert_eq!(
        ProvidedFieldsConformance::calendar_violations(&bare, &capabilities).len(),
        6
    );
}

#[test]
fn events_carry_last_modified_and_created_dates() {
    let e = map(&format!(
        r#"{{"id":"u","updated":"2026-09-20T08:00:00.123Z","created":"2026-09-01T00:00:00Z",{TIMES}}}"#
    ))
    .unwrap();
    assert_eq!(e.created, Some(instant("2026-09-01T00:00:00Z")));
    let modified = e.last_modified.unwrap();
    assert!((seconds_between(instant("2026-09-20T08:00:00Z"), modified) - 0.123).abs() < 0.001);
    assert!(
        map(&format!(r#"{{"id":"v",{TIMES}}}"#))
            .unwrap()
            .last_modified
            .is_none()
    );
}

#[test]
fn a_reminder_method_other_than_popup_or_email_is_kept() {
    let e = map(&format!(
        r#"{{"id":"m","reminders":{{"useDefault":false,"overrides":[{{"method":"sms","minutes":5}}]}},{TIMES}}}"#
    ))
    .unwrap();
    assert_eq!(
        e.reminders,
        Some(vec![Reminder::before(
            5,
            ReminderType::Other("sms".to_string()),
            Some(false)
        )])
    );
}
