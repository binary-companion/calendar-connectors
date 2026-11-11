//! Port of `Write/WriteTypes.swift`.

use std::collections::BTreeSet;

use jiff::Timestamp;

use crate::model::CalendarEvent;
use crate::raw_enum::raw_enum;
use crate::source_types::SourceCapabilities;

raw_enum! {
    /// A part of an event that a write can touch. Used for validation, conflict reports and capabilities.
    pub enum EventField {
        Title = "title", Notes = "notes", Location = "location", Timing = "timing",
        Availability = "availability", Visibility = "visibility", Reminders = "reminders",
        Attendees = "attendees", Recurrence = "recurrence", Conference = "conference"
    }
}

/// Whether the provider may email attendees about a write. Callers must choose; there is no default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NotifyPolicy {
    All,
    ExternalOnly,
    None,
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

/// On a draft: whether to ask the provider for a generated conference link.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConferenceRequest {
    None,
    Generate,
}

/// On a patch: generate a link or remove the existing one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConferenceChange {
    Generate,
    Remove,
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

/// What a write needs to find an event. Build one from the event you read.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EventRef {
    pub calendar_id: String,
    pub event_id: String,
    /// The version the caller last saw; used for optimistic locking.
    pub version: Option<String>,
    /// An empty id means "no series" (normalised to `None` by [`EventRef::new`], [`EventRef::with_series_id`] and
    /// [`EventRef::set_series_id`], as Swift's `init` and `didSet` do).
    series_id: Option<String>,
    /// The occurrence's slot in its series (Google `originalStartTime`, EventKit `occurrenceDate`); differs from
    /// `start` for a moved occurrence. It identifies the occurrence when `event_id` is shared and is the split
    /// point for `.thisAndFollowing`.
    pub original_start: Option<Timestamp>,
}

impl EventRef {
    /// Swift's `init(calendarID:eventID:version:seriesID:originalStart:)` with the optional arguments left at nil.
    pub fn new(calendar_id: impl Into<String>, event_id: impl Into<String>) -> EventRef {
        EventRef {
            calendar_id: calendar_id.into(),
            event_id: event_id.into(),
            version: None,
            series_id: None,
            original_start: None,
        }
    }

    /// Swift's `init(_ event:)`.
    pub fn from_event(event: &CalendarEvent) -> EventRef {
        EventRef {
            calendar_id: event.calendar_id.clone(),
            event_id: event.event_id.clone(),
            version: event.version.clone(),
            series_id: event
                .series_id()
                .map(str::to_string)
                .filter(|id| !id.is_empty()),
            original_start: event.original_start(),
        }
    }

    #[must_use]
    pub fn with_series_id(mut self, series_id: Option<&str>) -> EventRef {
        self.set_series_id(series_id.map(str::to_string));
        self
    }

    pub fn series_id(&self) -> Option<&str> {
        self.series_id.as_deref()
    }

    pub fn set_series_id(&mut self, series_id: Option<String>) {
        self.series_id = series_id.filter(|id| !id.is_empty());
    }
}

pub enum WriteValidation {}

impl WriteValidation {
    /// Fails with `Unsupported` naming every field in `fields` that `capabilities.writable_fields` lacks.
    pub fn require_writable(
        fields: &BTreeSet<EventField>,
        capabilities: &SourceCapabilities,
    ) -> Result<(), WriteError> {
        let missing: BTreeSet<EventField> = fields
            .difference(&capabilities.writable_fields)
            .copied()
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(WriteError::Unsupported { fields: missing })
        }
    }
}
