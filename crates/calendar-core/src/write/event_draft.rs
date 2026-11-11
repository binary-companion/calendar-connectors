//! Port of `Write/EventDraft.swift`.
//!
//! Deviation: `EventTiming::validate` has no "times must be finite" check. A Swift `Date` can hold NaN or an infinity;
//! a `jiff::Timestamp` cannot, so the case cannot arise.

use std::collections::BTreeSet;

use jiff::Timestamp;

use crate::all_day::AllDay;
use crate::model::{
    AttendeeRole, Availability, CalendarEvent, ConferenceOrigin, ConferenceProvider, Reminder,
    ReminderTrigger, ReminderType, UidScope, Visibility,
};
use crate::recurrence::rule::RecurrenceRule;
use crate::source_types::SourceCapabilities;
use crate::time::Zone;
use crate::write::types::{ConferenceRequest, EventField, WriteError};

/// Start, end, zone and all-day as one unit, so a write can never carry half a time range.
/// All-day timings use the library's canonical form (midnight of the first day in `time_zone`, `end` exclusive).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventTiming {
    pub start: Timestamp,
    pub end: Timestamp,
    pub time_zone: Option<Zone>,
    pub is_all_day: bool,
}

impl EventTiming {
    pub fn new(
        start: Timestamp,
        end: Timestamp,
        time_zone: Option<Zone>,
        is_all_day: bool,
    ) -> EventTiming {
        EventTiming {
            start,
            end,
            time_zone,
            is_all_day,
        }
    }

    /// Every write calls this before any request or store mutation.
    pub fn validate(&self) -> Result<(), WriteError> {
        if self.end <= self.start {
            return Err(WriteError::Invalid("end must be after start".to_string()));
        }
        if !self.is_all_day {
            return Ok(());
        }
        let Some(zone) = &self.time_zone else {
            return Err(WriteError::Invalid(
                "an all-day event needs a time zone".to_string(),
            ));
        };
        for instant in [self.start, self.end] {
            let midnight = AllDay::start_of_day(&AllDay::date_of(instant, zone), zone);
            if midnight != Some(instant) {
                return Err(WriteError::Invalid(format!(
                    "all-day times must be midnight in {}",
                    zone.identifier()
                )));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AttendeeDraft {
    pub name: Option<String>,
    /// Trimmed and lowercased. Set only through [`AttendeeDraft::new`] (Swift `private(set)`).
    email: String,
    pub role: AttendeeRole,
}

/// The code points `CharacterSet.whitespacesAndNewlines`, `.controlCharacters` and `",;<>"` contain together, as
/// inclusive ranges, read from Foundation (macOS) by enumerating every scalar. `controlCharacters` is Unicode
/// general categories Cc and Cf, so it covers format characters such as U+200B and U+FEFF.
const FORBIDDEN_EMAIL_SCALARS: &[(u32, u32)] = &[
    (0x0, 0x20),
    (0x2c, 0x2c),
    (0x3b, 0x3c),
    (0x3e, 0x3e),
    (0x7f, 0xa0),
    (0xad, 0xad),
    (0x600, 0x605),
    (0x61c, 0x61c),
    (0x6dd, 0x6dd),
    (0x70f, 0x70f),
    (0x890, 0x891),
    (0x8e2, 0x8e2),
    (0x1680, 0x1680),
    (0x180e, 0x180e),
    (0x2000, 0x200f),
    (0x2028, 0x202f),
    (0x205f, 0x2064),
    (0x2066, 0x206f),
    (0x3000, 0x3000),
    (0xfeff, 0xfeff),
    (0xfff9, 0xfffb),
    (0x110bd, 0x110bd),
    (0x110cd, 0x110cd),
    (0x13430, 0x1343f),
    (0x1bca0, 0x1bca3),
    (0x1d173, 0x1d17a),
    (0xe0001, 0xe0001),
    (0xe0020, 0xe007f),
];

impl AttendeeDraft {
    /// Swift's `init(email:name:role:)`; pass `None` and `AttendeeRole::Required` for the defaults.
    pub fn new(email: &str, name: Option<&str>, role: AttendeeRole) -> AttendeeDraft {
        AttendeeDraft {
            name: name.map(str::to_string),
            email: email.trim().to_lowercase(),
            role,
        }
    }

    pub fn email(&self) -> &str {
        &self.email
    }

    /// One `@` with something on both sides; no whitespace, control characters or `, ; < >` (which would smuggle a
    /// second address or a display-name form into the provider request). Deliberately not a full RFC check; the
    /// provider has the final say.
    pub fn is_valid_email(email: &str) -> bool {
        let parts: Vec<&str> = email.split('@').collect();
        if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
            return false;
        }
        email.chars().all(|c| {
            let scalar = c as u32;
            !FORBIDDEN_EMAIL_SCALARS
                .iter()
                .any(|&(low, high)| low <= scalar && scalar <= high)
        })
    }
}

/// Everything needed to create an event. `EventDraft::copying` is the one lenient path.
#[derive(Clone, Debug, PartialEq)]
pub struct EventDraft {
    pub title: String,
    pub notes: Option<String>,
    pub location: Option<String>,
    pub timing: EventTiming,
    pub availability: Availability,
    pub visibility: Visibility,
    /// `None` = the calendar's default reminders; empty = none.
    pub reminders: Option<Vec<Reminder>>,
    pub attendees: Vec<AttendeeDraft>,
    pub conference: ConferenceRequest,
    pub recurrence: Option<RecurrenceRule>,
    /// An iCalendar UID identifying the meeting. Not content: a create that finds it already on the calendar fails
    /// with `WriteError::AlreadyExists`, and otherwise stores it so later copies are recognized.
    /// `EventDraft::copying` fills it only from a `Global` uid.
    pub uid: Option<String>,
}

impl EventDraft {
    /// Swift's `init(title:timing:)` with every other argument at its default: busy, default visibility, the
    /// calendar's reminders, no attendees, no conference, no recurrence, no uid. Set the public fields (struct update)
    /// for the rest.
    pub fn new(title: impl Into<String>, timing: EventTiming) -> EventDraft {
        EventDraft {
            title: title.into(),
            notes: None,
            location: None,
            timing,
            availability: Availability::Busy,
            visibility: Visibility::Default,
            reminders: None,
            attendees: Vec::new(),
            conference: ConferenceRequest::None,
            recurrence: None,
            uid: None,
        }
    }

    pub fn validate(&self) -> Result<(), WriteError> {
        self.timing.validate()?;
        if let Some(recurrence) = &self.recurrence {
            recurrence.validate()?;
        }
        for attendee in &self.attendees {
            if !AttendeeDraft::is_valid_email(attendee.email()) {
                return Err(WriteError::Invalid(format!(
                    "attendee email is not valid: {}",
                    attendee.email()
                )));
            }
        }
        // A reminder relative to the event must fire at or before the point it counts from.
        if let Some(reminders) = &self.reminders
            && reminders.iter().any(
                |r| matches!(r.trigger, ReminderTrigger::Relative { offset, .. } if offset > 0.0),
            )
        {
            return Err(WriteError::Invalid(
                "reminder minutes must not be negative".to_string(),
            ));
        }
        Ok(())
    }

    /// The fields this draft sets beyond their defaults; connectors check them against `writable_fields`.
    pub fn used_fields(&self) -> BTreeSet<EventField> {
        let mut fields = BTreeSet::from([EventField::Title, EventField::Timing]);
        if self.notes.is_some() {
            fields.insert(EventField::Notes);
        }
        if self.location.is_some() {
            fields.insert(EventField::Location);
        }
        if self.availability != Availability::Busy {
            fields.insert(EventField::Availability);
        }
        if self.visibility != Visibility::Default {
            fields.insert(EventField::Visibility);
        }
        if self.reminders.is_some() {
            fields.insert(EventField::Reminders);
        }
        if !self.attendees.is_empty() {
            fields.insert(EventField::Attendees);
        }
        if self.conference != ConferenceRequest::None {
            fields.insert(EventField::Conference);
        }
        if self.recurrence.is_some() {
            fields.insert(EventField::Recurrence);
        }
        fields
    }

    /// The enum-valued fields a connector maps to the closest value the calendar supports, when the stored copy differs
    /// from what was requested. Other provider normalization (reminder defaults, attendee order, all-day zones, text)
    /// is deliberately not reported: it is visible in the returned event but is not an adjustment the library made.
    /// (Swift `adjustments(comparedTo:)`.)
    pub fn adjustments_compared_to(&self, stored: &CalendarEvent) -> BTreeSet<EventField> {
        let mut changed = BTreeSet::new();
        if let Some(value) = stored.availability
            && value != self.availability
        {
            changed.insert(EventField::Availability);
        }
        if let Some(value) = stored.visibility
            && value != self.visibility
        {
            changed.insert(EventField::Visibility);
        }
        changed
    }

    /// What to write for a copied event's reminders. Unknown (`None`) stays `None`, the calendar's defaults; a real
    /// "none" (`[]`) stays `[]`; a list that is all calendar defaults becomes `None`; otherwise only plain on-screen
    /// reminders relative to the start are copied (the common denominator every writer accepts), and a list that
    /// leaves nothing after that becomes `None` too.
    fn copied_reminders(reminders: Option<&Vec<Reminder>>) -> Option<Vec<Reminder>> {
        let reminders = reminders?;
        if reminders.is_empty() {
            return Some(Vec::new());
        }
        if reminders
            .iter()
            .all(|r| r.is_calendar_default == Some(true))
        {
            return None;
        }
        let copyable: Vec<Reminder> = reminders
            .iter()
            .filter(|r| {
                // `minutes_before` is `None` unless the trigger is relative to the start.
                let Some(minutes) = r.minutes_before() else {
                    return false;
                };
                minutes >= 0 && r.kind == ReminderType::Display && r.repeat_count == 0
            })
            .map(|r| Reminder::before(r.minutes_before().unwrap_or(0), ReminderType::Display, None))
            .collect();
        if copyable.is_empty() {
            None
        } else {
            Some(copyable)
        }
    }

    /// A best-effort copy of `event` for a target with `capabilities`: only fields in `writable_fields` are carried
    /// over. Self and email-less attendees are dropped, an empty reminder list becomes "calendar defaults" (reads
    /// cannot tell the two apart), only a Meet link is re-requested, and recurrence is never copied (reads carry none).
    /// Attendees with an invalid email and negative reminders are dropped, so those two never make `validate()` fail.
    /// Timing is copied as is: a source event with a zero-length or inverted interval still fails `validate()`.
    /// (Swift `init(copying:for:)`.)
    pub fn copying(event: &CalendarEvent, capabilities: &SourceCapabilities) -> EventDraft {
        let writable = &capabilities.writable_fields;
        let can = |field: EventField| writable.contains(&field);
        let reminders = EventDraft::copied_reminders(event.reminders.as_ref());
        let attendees = if can(EventField::Attendees) {
            event
                .attendees
                .iter()
                .filter(|a| !a.is_self)
                .filter_map(|a| {
                    a.email()
                        .map(|email| AttendeeDraft::new(email, a.name.as_deref(), a.role))
                })
                .filter(|a| AttendeeDraft::is_valid_email(a.email()))
                .collect()
        } else {
            Vec::new()
        };
        let requests_meet = can(EventField::Conference)
            && event.conferences.iter().any(|c| {
                c.origin == ConferenceOrigin::Structured && c.provider == ConferenceProvider::Meet
            });
        EventDraft {
            title: event.title.clone(),
            timing: EventTiming::new(
                event.start,
                event.end,
                Some(event.time_zone.clone()),
                event.is_all_day,
            ),
            notes: if can(EventField::Notes) {
                event.notes.clone()
            } else {
                None
            },
            location: if can(EventField::Location) {
                event.location.clone()
            } else {
                None
            },
            availability: if can(EventField::Availability) {
                event.availability.unwrap_or(Availability::Busy)
            } else {
                Availability::Busy
            },
            visibility: if can(EventField::Visibility) {
                event.visibility.unwrap_or(Visibility::Default)
            } else {
                Visibility::Default
            },
            reminders: if can(EventField::Reminders) {
                reminders
            } else {
                None
            },
            attendees,
            conference: if requests_meet {
                ConferenceRequest::Generate
            } else {
                ConferenceRequest::None
            },
            recurrence: None,
            uid: if event.uid_scope == Some(UidScope::Global) {
                event.uid.clone()
            } else {
                None
            },
        }
    }
}
