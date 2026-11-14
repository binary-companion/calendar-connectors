//! Port of `ModelTests.swift`. `aDeclaredFieldThatIsNilIsReportedAndAnUndeclaredNilIsFine` uses
//! `ProvidedFieldsConformance`, which lives in `calendar-test-support`; its Rust test is in that crate
//! (`crates/calendar-test-support/src/tests/provided_fields_tests.rs`) because a dev-dependency from this crate on
//! `calendar-test-support` would link two copies of `calendar-core`.

use std::collections::BTreeSet;

use jiff::Timestamp;

use super::support::{at, zone};
use crate::all_day::{AllDay, CalendarDate};
use crate::model::{
    AccessRole, Attendee, AttendeeRole, Availability, CalendarDescriptor, CalendarEvent,
    CalendarKind, CalendarPermissions, CalendarProvider, CalendarService, Participation, Reminder,
    ReminderAnchor, ReminderProximity, ReminderTrigger, ReminderType, ResponseStatus, SeriesInfo,
    StructuredLocation,
};
use crate::source_types::{SourceCapabilities, SyncKind};
use crate::time::add_seconds;

fn event(event_id: &str, calendar_id: &str, start: i64, end: i64) -> CalendarEvent {
    CalendarEvent::new(event_id, calendar_id, "T", at(start), at(end))
}

#[test]
fn event_id_is_unique_across_calendars() {
    let a = event("e1", "work", 0, 60);
    let b = event("e1", "home", 0, 60);
    assert_eq!(a.id(), "work/e1");
    assert_ne!(a.id(), b.id());
}

#[test]
fn descriptor_normalizes_color() {
    fn hex(raw: Option<&str>) -> Option<String> {
        CalendarDescriptor::new("c", "C", CalendarService::google())
            .with_color_hex(raw)
            .color_hex()
            .map(str::to_string)
    }
    assert_eq!(hex(Some("#9fe1e7")).as_deref(), Some("#9FE1E7"));
    assert_eq!(hex(Some("9fe1e7")).as_deref(), Some("#9FE1E7"));
    assert_eq!(hex(Some("#abc")).as_deref(), Some("#AABBCC"));
    assert_eq!(hex(Some("nope")), None);
    assert_eq!(hex(None), None);
}

fn attendee_with_email(email: &str) -> Attendee {
    Attendee::new(
        None,
        Some(email),
        AttendeeRole::Required,
        ResponseStatus::NeedsAction,
        false,
        false,
    )
}

#[test]
fn attendee_normalizes_email() {
    assert_eq!(
        attendee_with_email("  Ann@Example.COM ").email(),
        Some("ann@example.com")
    );
    assert_eq!(attendee_with_email("   ").email(), None);
}

#[test]
fn capabilities_default_to_read_only_and_no_sync() {
    let c = SourceCapabilities::default();
    assert!(
        !c.can_write
            && !c.can_edit_attendees
            && !c.can_respond_to_invite
            && c.provided_fields.is_empty()
            && !c.supports_push
    );
    assert_eq!(c.sync_kind, SyncKind::None);
}

#[test]
fn descriptors_are_standard_calendars_unless_told_otherwise() {
    let descriptor = || CalendarDescriptor::new("c", "C", CalendarService::google());
    assert_eq!(descriptor().kind, CalendarKind::Standard);
    let mut birthdays = descriptor();
    birthdays.kind = CalendarKind::Birthdays;
    assert_eq!(birthdays.kind, CalendarKind::Birthdays);
    let mut subscribed = descriptor();
    subscribed.kind = CalendarKind::Subscribed;
    assert_eq!(subscribed.kind, CalendarKind::Subscribed);
}

// Provided fields: `aDeclaredFieldThatIsNilIsReportedAndAnUndeclaredNilIsFine` is pending (Task 6).

#[test]
fn series_and_participation_accessors_separate_unknown_from_none() {
    let mut event = event("e", "c", 0, 60);
    assert!(
        event.series.is_none()
            && event.series_id().is_none()
            && event.participation.is_none()
            && event.my_response().is_none()
    );
    event.series = Some(SeriesInfo::NotRecurring);
    event.participation = Some(Participation::NotInvited);
    assert!(
        event.series_id().is_none()
            && event.my_response().is_none()
            && event.series == Some(SeriesInfo::NotRecurring)
    );
    event.series = Some(SeriesInfo::Occurrence {
        series_id: "s".to_string(),
        original_start: Some(at(30)),
    });
    event.participation = Some(Participation::Invited(ResponseStatus::Tentative));
    assert!(
        event.series_id() == Some("s")
            && event.original_start() == Some(at(30))
            && event.my_response() == Some(ResponseStatus::Tentative)
    );
}

// Availability, permissions and calendar identity (Issues 6 and 7).

fn set(values: &[Availability]) -> BTreeSet<Availability> {
    values.iter().copied().collect()
}

#[test]
fn closest_availability_follows_the_fallback_lists() {
    use Availability::{Busy, Free, Tentative, Unavailable};
    let all = set(&[Busy, Free, Tentative, Unavailable]);
    for value in &all {
        assert_eq!(value.closest_in(&all), Some(*value));
    }
    assert_eq!(Tentative.closest_in(&set(&[Busy, Free])), Some(Busy));
    assert_eq!(Unavailable.closest_in(&set(&[Busy, Free])), Some(Busy));
    assert_eq!(Tentative.closest_in(&set(&[Free])), Some(Free));
    assert_eq!(Busy.closest_in(&set(&[Free])), Some(Free));
    assert_eq!(Free.closest_in(&set(&[Busy])), Some(Busy));
    assert_eq!(Busy.closest_in(&set(&[Tentative])), None); // no fallback in the list
    assert_eq!(Busy.closest_in(&set(&[])), None);
}

#[test]
fn permissions_summarize_into_an_access_role() {
    let permissions = |can_view_details, can_edit, can_share| CalendarPermissions {
        can_view_details,
        can_edit,
        can_share,
        can_view_private: None,
    };
    assert_eq!(
        permissions(false, false, None).access_role(),
        Some(AccessRole::FreeBusyReader)
    );
    assert_eq!(
        CalendarPermissions {
            can_view_details: false,
            ..CalendarPermissions::default()
        }
        .access_role(),
        Some(AccessRole::FreeBusyReader)
    );
    assert_eq!(
        permissions(true, false, Some(true)).access_role(),
        Some(AccessRole::Reader)
    );
    assert_eq!(
        permissions(true, true, Some(true)).access_role(),
        Some(AccessRole::Owner)
    );
    assert_eq!(
        permissions(true, true, Some(false)).access_role(),
        Some(AccessRole::Writer)
    );
    assert_eq!(permissions(true, true, None).access_role(), None); // EventKit, writable
}

#[test]
fn service_and_provider_are_string_backed_and_open() {
    assert!(
        CalendarService::event_kit().raw_value() == "eventkit"
            && CalendarService::new("eventkit") == CalendarService::event_kit()
    );
    assert_ne!(
        CalendarProvider::new("example.org"),
        CalendarProvider::cal_dav()
    );
    assert_eq!(CalendarProvider::icloud().raw_value(), "icloud");
}

// `draftAndPatchReportOnlyAvailabilityAndVisibilityAdjustments` is pending (Task 5).

// Reminders (Issue 4).

#[test]
fn a_reminder_fires_relative_to_the_start_or_end_or_at_a_date() {
    let (start, end) = (at(10_000), at(13_600));
    assert_eq!(
        Reminder::with_minutes_before(10).fire_date(start, end),
        Some(add_seconds(start, -600.0))
    );
    assert_eq!(
        Reminder::new(ReminderTrigger::Relative {
            offset: -300.0,
            to: ReminderAnchor::End
        })
        .fire_date(start, end),
        Some(add_seconds(end, -300.0))
    );
    let fixed = at(5_000);
    assert_eq!(
        Reminder::new(ReminderTrigger::Absolute(fixed)).fire_date(start, end),
        Some(fixed)
    );
    let location = StructuredLocation {
        title: Some("Home".to_string()),
        ..StructuredLocation::default()
    };
    assert_eq!(
        Reminder::new(ReminderTrigger::Location(
            location,
            ReminderProximity::Enter
        ))
        .fire_date(start, end),
        None
    );
}

#[test]
fn an_all_day_reminder_fires_exactly_its_offset_before_the_canonical_midnight_across_a_daylight_saving_change()
 {
    // 2026-03-08 is 23 hours long in New York; the offset is a duration, not a wall-clock time.
    let zone = zone("America/New_York");
    let (start, end) = AllDay::canonical(
        &CalendarDate::new(2026, 3, 8),
        &CalendarDate::new(2026, 3, 9),
        &zone,
    )
    .unwrap();
    let reminder = Reminder::before(9 * 60, ReminderType::Display, None);
    assert_eq!(
        reminder.fire_date(start, end),
        Some(add_seconds(start, -9.0 * 3600.0))
    );
}

#[test]
fn minutes_before_is_nil_unless_the_trigger_is_relative_to_the_start() {
    assert_eq!(Reminder::with_minutes_before(15).minutes_before(), Some(15));
    assert_eq!(
        Reminder::new(ReminderTrigger::Relative {
            offset: -60.0,
            to: ReminderAnchor::End
        })
        .minutes_before(),
        None
    );
    assert_eq!(
        Reminder::new(ReminderTrigger::Absolute(Timestamp::UNIX_EPOCH)).minutes_before(),
        None
    );
    assert_eq!(
        Reminder::new(ReminderTrigger::Location(
            StructuredLocation::default(),
            ReminderProximity::Leave
        ))
        .minutes_before(),
        None
    );
}

#[test]
fn reminder_lists_are_compared_ignoring_order_and_the_calendar_default_flag() {
    let a = vec![
        Reminder::before(10, ReminderType::Display, Some(true)),
        Reminder::before(60, ReminderType::Display, None),
    ];
    let b = vec![
        Reminder::before(60, ReminderType::Display, Some(false)),
        Reminder::before(10, ReminderType::Display, None),
    ];
    assert!(Reminder::same_set(&a, &b));
    // A changed alert type differs.
    let changed = vec![
        Reminder::before(10, ReminderType::Display, None),
        Reminder::before(60, ReminderType::Email { address: None }, None),
    ];
    assert!(!Reminder::same_set(&a, &changed));
}

#[test]
fn cal_dav_service_raw_value() {
    assert_eq!(CalendarService::cal_dav().raw_value(), "caldav");
}

#[test]
fn raw_value_enums_round_trip_through_their_swift_raw_strings() {
    use crate::model::{ConferenceOrigin, ConferenceProvider, EventKind, UidScope, Visibility};
    assert_eq!(ConferenceProvider::GoToMeeting.as_str(), "goToMeeting");
    assert_eq!(ConferenceOrigin::EventUrl.as_str(), "eventURL");
    assert_eq!(
        UidScope::from_raw_value("provider"),
        Some(UidScope::Provider)
    );
    assert_eq!(Visibility::PublicEvent.as_str(), "publicEvent");
    assert_eq!(
        EventKind::from_raw_value("focusTime"),
        Some(EventKind::FocusTime)
    );
    assert_eq!(EventKind::from_raw_value("FocusTime"), None);
    for value in ConferenceProvider::ALL {
        assert_eq!(
            ConferenceProvider::from_raw_value(value.as_str()),
            Some(*value)
        );
    }
}

#[test]
fn draft_and_patch_report_only_availability_and_visibility_adjustments() {
    use crate::model::Visibility;
    use crate::write::event_draft::{EventDraft, EventTiming};
    use crate::write::types::EventField;
    let mut stored = event("e", "c", 0, 60);
    stored.availability = Some(Availability::Busy);
    stored.visibility = Some(Visibility::Default);
    let draft = EventDraft {
        availability: Availability::Tentative,
        visibility: Visibility::Default,
        ..EventDraft::new("T", EventTiming::new(stored.start, stored.end, None, false))
    };
    let set = |list: &[EventField]| list.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(
        draft.adjustments_compared_to(&stored),
        set(&[EventField::Availability])
    );
    let patch = |availability, visibility| {
        super::support::hand_patch(|p| {
            p.availability = availability;
            p.visibility = visibility;
        })
    };
    assert_eq!(
        patch(
            Some(Availability::Tentative),
            Some(Visibility::Confidential)
        )
        .adjustments_compared_to(&stored),
        set(&[EventField::Availability, EventField::Visibility])
    );
    assert!(
        patch(Some(Availability::Busy), None)
            .adjustments_compared_to(&stored)
            .is_empty()
    );
    stored.availability = None; // unknown is never an adjustment
    assert!(draft.adjustments_compared_to(&stored).is_empty());
}
