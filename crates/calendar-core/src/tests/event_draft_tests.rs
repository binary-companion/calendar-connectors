//! Port of `EventDraftTests.swift`. `timingRejectsNonFiniteDates` has no Rust counterpart: `jiff::Timestamp` cannot
//! hold NaN or an infinity, so the finiteness check does not exist (parity table: `n/a`).

use jiff::Timestamp;

use super::support::{at, attendee, expect_write_error, fields, iso, named_attendee, zone};
use crate::model::{
    AttendeeRole, Availability, CalendarEvent, ConferenceInfo, ConferenceOrigin,
    ConferenceProvider, Reminder, ReminderAnchor, ReminderProximity, ReminderTrigger, ReminderType,
    SeriesInfo, StructuredLocation, UidScope, Visibility,
};
use crate::recurrence::rule::{Frequency, RecurrenceRule};
use crate::source_types::SourceCapabilities;
use crate::time::Zone;
use crate::url_string::UrlString;
use crate::write::event_draft::{AttendeeDraft, EventDraft, EventTiming};
use crate::write::types::{ConferenceRequest, EventField};

fn utc() -> Zone {
    zone("UTC")
}
fn new_york() -> Zone {
    zone("America/New_York")
}
fn instant(text: &str) -> Timestamp {
    iso(text)
}
fn timed() -> EventTiming {
    EventTiming::new(
        instant("2026-09-21T10:00:00Z"),
        instant("2026-09-21T11:00:00Z"),
        Some(utc()),
        false,
    )
}
fn draft_attendee(email: &str) -> AttendeeDraft {
    AttendeeDraft::new(email, None, AttendeeRole::Required)
}
fn all_fields() -> Vec<EventField> {
    EventField::ALL.to_vec()
}

#[test]
fn timing_rejects_end_before_start_and_non_canonical_all_day() {
    expect_write_error(
        crate::write::types::WriteError::Invalid("end must be after start".into()),
        EventTiming::new(
            instant("2026-09-21T11:00:00Z"),
            instant("2026-09-21T10:00:00Z"),
            Some(utc()),
            false,
        )
        .validate(),
    );
    expect_write_error(
        crate::write::types::WriteError::Invalid("an all-day event needs a time zone".into()),
        EventTiming::new(
            instant("2026-09-21T00:00:00Z"),
            instant("2026-09-22T00:00:00Z"),
            None,
            true,
        )
        .validate(),
    );
    expect_write_error(
        crate::write::types::WriteError::Invalid(
            "all-day times must be midnight in America/New_York".into(),
        ),
        EventTiming::new(
            instant("2026-09-21T00:00:00Z"),
            instant("2026-09-22T00:00:00Z"),
            Some(new_york()),
            true,
        )
        .validate(),
    );
    assert!(
        EventTiming::new(
            instant("2026-09-21T04:00:00Z"),
            instant("2026-09-22T04:00:00Z"),
            Some(new_york()),
            true
        )
        .validate()
        .is_ok()
    );
}

#[test]
fn attendee_draft_normalizes_email() {
    assert_eq!(
        draft_attendee("  Ann@Example.COM ").email(),
        "ann@example.com"
    );
}

#[test]
fn draft_validation_checks_timing_attendees_reminders_and_recurrence() {
    use crate::write::types::WriteError::Invalid;
    let base = EventDraft::new("T", timed());
    assert!(base.validate().is_ok());
    let mut no_at = base.clone();
    no_at.attendees = vec![draft_attendee("nobody")];
    expect_write_error(
        Invalid("attendee email is not valid: nobody".into()),
        no_at.validate(),
    );
    let mut negative = base.clone();
    negative.reminders = Some(vec![Reminder::with_minutes_before(-5)]);
    expect_write_error(
        Invalid("reminder minutes must not be negative".into()),
        negative.validate(),
    );
    let mut bad_rule = base.clone();
    bad_rule.recurrence = Some(RecurrenceRule {
        interval: 0,
        ..RecurrenceRule::new(Frequency::Daily)
    });
    expect_write_error(
        Invalid("recurrence interval must be at least 1".into()),
        bad_rule.validate(),
    );
}

#[test]
fn draft_validation_rejects_malformed_addresses_beyond_a_missing_at() {
    use crate::write::types::WriteError::Invalid;
    for bad in [
        "@",
        "a@",
        "@b.c",
        "a@@b.c",
        "a b@c.d",
        "a@b@c.d",
        "",
        "a@b.c,d@e.f",
        "a@b.c;d@e.f",
        "<a@b.c>",
        "a@b.c\nd@e.f",
        "a@b.c\u{0}",
        "a\u{7}@b.c",
        // Controls and format characters Foundation's `controlCharacters` and `whitespacesAndNewlines` cover.
        "a\u{85}@b.c",
        "a\u{a0}@b.c",
        "a\u{200b}@b.c",
        "a\u{2028}@b.c",
        "a\u{feff}@b.c",
        "a\u{3000}@b.c",
    ] {
        let mut draft = EventDraft::new("T", timed());
        draft.attendees = vec![draft_attendee(bad)];
        expect_write_error(
            Invalid(format!(
                "attendee email is not valid: {}",
                draft_attendee(bad).email()
            )),
            draft.validate(),
        );
    }
    let mut ok = EventDraft::new("T", timed());
    ok.attendees = vec![draft_attendee("a@b.c")];
    assert!(ok.validate().is_ok());
}

#[test]
fn used_fields_lists_what_the_draft_sets_beyond_defaults() {
    assert_eq!(
        EventDraft::new("T", timed()).used_fields(),
        fields(&[EventField::Title, EventField::Timing])
    );
    let mut rich = EventDraft {
        notes: Some("n".into()),
        location: Some("l".into()),
        availability: Availability::Free,
        visibility: Visibility::PrivateEvent,
        reminders: Some(vec![]),
        attendees: vec![draft_attendee("a@b.c")],
        conference: ConferenceRequest::Generate,
        recurrence: Some(RecurrenceRule::new(Frequency::Daily)),
        ..EventDraft::new("T", timed())
    };
    assert_eq!(rich.used_fields(), fields(&all_fields()));
    rich.reminders = None;
    assert!(!rich.used_fields().contains(&EventField::Reminders));
}

fn meet() -> ConferenceInfo {
    ConferenceInfo::new(
        UrlString::parse("https://meet.google.com/abc").unwrap(),
        ConferenceProvider::Meet,
        ConferenceOrigin::Structured,
    )
}

fn source() -> CalendarEvent {
    let mut me = attendee("me@x.com");
    me.is_self = true;
    let mut bob = attendee("Bob@x.com");
    bob.role = AttendeeRole::Optional;
    let no_email = named_attendee("No Email", None);
    CalendarEvent {
        notes: Some("n".into()),
        location: Some("Room".into()),
        time_zone: utc(),
        availability: Some(Availability::Free),
        visibility: Some(Visibility::PrivateEvent),
        series: Some(SeriesInfo::Occurrence {
            series_id: "s".into(),
            original_start: None,
        }),
        attendees: vec![me, bob, no_email],
        conferences: vec![meet()],
        reminders: Some(vec![Reminder::with_minutes_before(10)]),
        ..CalendarEvent::new(
            "e",
            "c",
            "Planning",
            instant("2026-09-21T10:00:00Z"),
            instant("2026-09-21T11:00:00Z"),
        )
    }
}

fn caps(can_edit_attendees: bool, writable: &[EventField]) -> SourceCapabilities {
    SourceCapabilities {
        can_write: true,
        can_edit_attendees,
        writable_fields: fields(writable),
        ..SourceCapabilities::default()
    }
}

#[test]
fn copying_to_a_fully_writable_target_keeps_everything_representable() {
    let draft = EventDraft::copying(&source(), &caps(true, &all_fields()));
    assert!(
        draft.title == "Planning"
            && draft.notes.as_deref() == Some("n")
            && draft.location.as_deref() == Some("Room")
    );
    assert_eq!(
        draft.timing,
        EventTiming::new(
            instant("2026-09-21T10:00:00Z"),
            instant("2026-09-21T11:00:00Z"),
            Some(utc()),
            false
        )
    );
    assert!(
        draft.availability == Availability::Free && draft.visibility == Visibility::PrivateEvent
    );
    assert_eq!(
        draft.reminders,
        Some(vec![Reminder::with_minutes_before(10)])
    );
    // Self and email-less attendees are dropped.
    assert_eq!(
        draft.attendees,
        vec![AttendeeDraft::new(
            "bob@x.com",
            None,
            AttendeeRole::Optional
        )]
    );
    assert_eq!(draft.conference, ConferenceRequest::Generate); // only a Meet link is re-requested
    assert_eq!(draft.recurrence, None);
}

#[test]
fn copying_drops_fields_the_target_cannot_write() {
    let writable = [
        EventField::Title,
        EventField::Notes,
        EventField::Location,
        EventField::Timing,
        EventField::Availability,
        EventField::Reminders,
        EventField::Recurrence,
    ];
    let draft = EventDraft::copying(&source(), &caps(false, &writable));
    assert!(
        draft.attendees.is_empty()
            && draft.visibility == Visibility::Default
            && draft.conference == ConferenceRequest::None
    );
    assert!(draft.notes.as_deref() == Some("n") && draft.availability == Availability::Free);
    assert_eq!(draft.recurrence, None);
}

fn all_writable() -> SourceCapabilities {
    caps(false, &all_fields())
}

fn copied_reminders(reminders: Option<Vec<Reminder>>) -> Option<Vec<Reminder>> {
    let mut event = source();
    event.reminders = reminders;
    EventDraft::copying(&event, &all_writable()).reminders
}

fn default_flagged(minutes: i64) -> Reminder {
    Reminder::before(minutes, ReminderType::Display, Some(true))
}

#[test]
fn copying_keeps_unknown_as_defaults_and_a_real_none_as_none() {
    assert_eq!(copied_reminders(None), None);
    assert_eq!(copied_reminders(Some(vec![])), Some(vec![]));
}

#[test]
fn copying_a_list_that_is_all_calendar_defaults_uses_the_targets_defaults() {
    assert_eq!(
        copied_reminders(Some(vec![default_flagged(10), default_flagged(60)])),
        None
    );
}

#[test]
fn copying_keeps_only_plain_reminders_relative_to_the_start() {
    let location = Reminder::new(ReminderTrigger::Location(
        StructuredLocation {
            title: Some("Home".into()),
            ..StructuredLocation::default()
        },
        ReminderProximity::Enter,
    ));
    let email = Reminder::before(30, ReminderType::Email { address: None }, None);
    let repeating = Reminder {
        repeat_count: 2,
        repeat_interval: Some(60.0),
        ..Reminder::new(ReminderTrigger::Relative {
            offset: -60.0,
            to: ReminderAnchor::Start,
        })
    };
    assert_eq!(
        copied_reminders(Some(vec![
            Reminder::before(10, ReminderType::Display, Some(false)),
            location.clone(),
            email.clone(),
            repeating
        ])),
        Some(vec![Reminder::with_minutes_before(10)])
    );
    assert_eq!(copied_reminders(Some(vec![location, email])), None); // nothing copyable: the target's defaults
}

#[test]
fn copying_to_a_read_only_target_keeps_only_the_required_parts() {
    let draft = EventDraft::copying(&source(), &SourceCapabilities::default());
    assert!(
        draft.title == "Planning"
            && draft.notes.is_none()
            && draft.location.is_none()
            && draft.reminders.is_none()
    );
    assert!(
        draft.availability == Availability::Busy
            && draft.visibility == Visibility::Default
            && draft.attendees.is_empty()
    );
    assert!(draft.conference == ConferenceRequest::None && draft.recurrence.is_none());
}

#[test]
fn copying_does_not_re_request_a_non_meet_conference() {
    let mut event = source();
    event.conferences = vec![ConferenceInfo::new(
        UrlString::parse("https://zoom.us/j/1").unwrap(),
        ConferenceProvider::Zoom,
        ConferenceOrigin::Structured,
    )];
    let draft = EventDraft::copying(&event, &caps(false, &all_fields()));
    assert_eq!(draft.conference, ConferenceRequest::None);
}

#[test]
fn copying_drops_source_data_that_would_fail_validation() {
    let mut event = source();
    event.attendees = vec![
        attendee("nobody"),
        attendee("a@b.c,d@e.f"),
        attendee("ok@x.com"),
    ];
    event.reminders = Some(vec![
        Reminder::with_minutes_before(-5),
        Reminder::with_minutes_before(15),
    ]);
    let capabilities = caps(true, &all_fields());
    let draft = EventDraft::copying(&event, &capabilities);
    assert_eq!(draft.attendees, vec![draft_attendee("ok@x.com")]);
    assert_eq!(
        draft.reminders,
        Some(vec![Reminder::with_minutes_before(15)])
    );
    draft.validate().unwrap();

    event.reminders = Some(vec![Reminder::with_minutes_before(-1)]);
    assert_eq!(EventDraft::copying(&event, &capabilities).reminders, None);
}

#[test]
fn copying_keeps_an_all_day_event_in_canonical_form() {
    let mut event = source();
    event.is_all_day = true;
    event.time_zone = new_york();
    event.start = instant("2026-09-21T04:00:00Z");
    event.end = instant("2026-09-22T04:00:00Z");
    let draft = EventDraft::copying(&event, &SourceCapabilities::default());
    assert!(draft.timing.is_all_day && draft.timing.time_zone == Some(new_york()));
    draft.validate().unwrap();
}

#[test]
fn copying_an_event_with_unknown_fields_uses_the_draft_defaults() {
    let event = CalendarEvent::new("e", "c", "T", at(1000), at(2000));
    let draft = EventDraft::copying(&event, &caps(false, &all_fields()));
    assert!(
        draft.availability == Availability::Busy
            && draft.visibility == Visibility::Default
            && draft.reminders.is_none()
    );
    assert_eq!(
        draft.used_fields(),
        fields(&[EventField::Title, EventField::Timing])
    );
}

#[test]
fn copying_carries_a_uid_only_when_it_is_global() {
    let mut event = CalendarEvent {
        uid: Some("u-1".into()),
        uid_scope: Some(UidScope::Global),
        ..CalendarEvent::new("e", "c", "T", at(1000), at(2000))
    };
    let capabilities = caps(false, &all_fields());
    assert_eq!(
        EventDraft::copying(&event, &capabilities).uid.as_deref(),
        Some("u-1")
    );
    event.uid_scope = Some(UidScope::Provider);
    assert_eq!(EventDraft::copying(&event, &capabilities).uid, None);
    event.uid_scope = None;
    assert_eq!(EventDraft::copying(&event, &capabilities).uid, None);
}
