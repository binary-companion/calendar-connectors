//! Port of `Write/WriteTypes.swift`. Only the two types that `SourceCapabilities` names are here so far; the rest
//! of the file (`NotifyPolicy`, `ConferenceRequest`, ...) arrives with the write model. `WriteError` is here
//! already because `RecurrenceRule::validate` throws it.

use std::collections::BTreeSet;

use crate::model::CalendarEvent;
use crate::raw_enum::raw_enum;

raw_enum! {
    /// A part of an event that a write can touch. Used for validation, conflict reports and capabilities.
    pub enum EventField {
        Title = "title", Notes = "notes", Location = "location", Timing = "timing",
        Availability = "availability", Visibility = "visibility", Reminders = "reminders",
        Attendees = "attendees", Recurrence = "recurrence", Conference = "conference"
    }
}

/// Which occurrences of a recurring event a write applies to. Ignored for an event that is not part of a series.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RecurrenceScope {
    ThisInstance,
    ThisAndFollowing,
    AllInSeries,
}

impl RecurrenceScope {
    /// Every case, in declaration order (Swift `CaseIterable`).
    pub const ALL: &'static [RecurrenceScope] = &[
        RecurrenceScope::ThisInstance,
        RecurrenceScope::ThisAndFollowing,
        RecurrenceScope::AllInSeries,
    ];
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum WriteError {
    /// The connector cannot write these fields (or the operation; RSVP is reported as `.attendees`).
    #[error("unsupported fields {fields:?}")]
    Unsupported { fields: BTreeSet<EventField> },
    /// Someone else changed a field this patch touches.
    #[error("conflicting fields {fields:?}")]
    Conflict { fields: BTreeSet<EventField> },
    /// The event or calendar is gone.
    #[error("not found")]
    NotFound,
    /// A read-only calendar, or no permission on this event.
    #[error("forbidden")]
    Forbidden(Option<String>),
    /// Malformed input: end before start, RSVP of `.needsAction`, a bad recurrence rule, ...
    #[error("invalid: {0}")]
    Invalid(String),
    /// A create found an event with the draft's `uid` already on the target calendar; nothing was written. The payload
    /// is the calendar's stored copy, so the caller can decide whether to update it.
    #[error("already exists")]
    AlreadyExists(Box<CalendarEvent>),
    /// A multi-step write stopped half way (Google `.thisAndFollowing`: series truncated, new series not created).
    #[error("partial write: {0}")]
    Partial(String),
}
