//! Port of `Write/WriteTypes.swift`. Only the two types that `SourceCapabilities` names are here so far; the rest
//! of the file (`NotifyPolicy`, `ConferenceRequest`, `WriteError`, ...) arrives with the write model.

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
