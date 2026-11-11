//! Port of `Write/EventPatch.swift`.
//!
//! Renames: Swift `EventPatch.init(from:to:)` is [`EventPatch::diff`]; `applied(to:)` is [`EventPatch::applied_to`];
//! `adjustments(comparedTo:)` is [`EventPatch::adjustments_compared_to`].

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{Attendee, Availability, CalendarEvent, ConferenceOrigin, Reminder, Visibility};
use crate::recurrence::rule::RecurrenceRule;
use crate::write::event_draft::{AttendeeDraft, EventTiming};
use crate::write::types::{ConferenceChange, EventField};

/// A field change where "leave alone" and "remove" are different things.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldUpdate<Value> {
    Keep,
    Set(Value),
    Clear,
}

impl<Value> Default for FieldUpdate<Value> {
    /// Swift's patch initializer defaults every `FieldUpdate` to `.keep`.
    fn default() -> FieldUpdate<Value> {
        FieldUpdate::Keep
    }
}

/// A delta on an event's attendees, never a replacement list, so an edit cannot wipe other people's responses.
/// `add` upserts by email: an existing attendee keeps their response and takes the new name and role.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AttendeeChanges {
    pub add: Vec<AttendeeDraft>,
    /// Normalized (trimmed, lowercased) emails.
    pub remove: Vec<String>,
}

impl AttendeeChanges {
    /// Swift's `init(add:remove:)`: `remove` is normalized.
    pub fn new(add: Vec<AttendeeDraft>, remove: Vec<String>) -> AttendeeChanges {
        AttendeeChanges {
            add,
            remove: remove
                .into_iter()
                .map(|email| email.trim().to_lowercase())
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.add.is_empty() && self.remove.is_empty()
    }
}

/// The changes to make to an existing event. Only these fields are sent, so data the library does not model
/// (attachments, extended properties, colors) is never disturbed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EventPatch {
    pub title: Option<String>,
    pub notes: FieldUpdate<String>,
    pub location: FieldUpdate<String>,
    /// Time is one unit: start, end, zone and all-day together.
    pub timing: Option<EventTiming>,
    pub availability: Option<Availability>,
    pub visibility: Option<Visibility>,
    /// `Clear` = the calendar's default reminders; `Set(vec![])` = none.
    pub reminders: FieldUpdate<Vec<Reminder>>,
    pub attendees: Option<AttendeeChanges>,
    pub recurrence: FieldUpdate<RecurrenceRule>,
    pub conference: Option<ConferenceChange>,
    /// The original the patch was diffed from; what conflicts are judged against. `None` for a hand-built patch,
    /// which conflicts on any stale version. Read it with [`EventPatch::base`] (Swift `private(set)`).
    base: Option<CalendarEvent>,
}

impl EventPatch {
    /// The original the patch was diffed from; `None` for a hand-built patch.
    pub fn base(&self) -> Option<&CalendarEvent> {
        self.base.as_ref()
    }

    /// The fields the stored copy shows differently from what this patch asked for (availability and visibility only;
    /// see `EventDraft::adjustments_compared_to`).
    pub fn adjustments_compared_to(&self, stored: &CalendarEvent) -> BTreeSet<EventField> {
        let mut changed = BTreeSet::new();
        if let (Some(requested), Some(value)) = (self.availability, stored.availability)
            && value != requested
        {
            changed.insert(EventField::Availability);
        }
        if let (Some(requested), Some(value)) = (self.visibility, stored.visibility)
            && value != requested
        {
            changed.insert(EventField::Visibility);
        }
        changed
    }

    pub fn touched_fields(&self) -> BTreeSet<EventField> {
        let mut fields = BTreeSet::new();
        if self.title.is_some() {
            fields.insert(EventField::Title);
        }
        if self.notes != FieldUpdate::Keep {
            fields.insert(EventField::Notes);
        }
        if self.location != FieldUpdate::Keep {
            fields.insert(EventField::Location);
        }
        if self.timing.is_some() {
            fields.insert(EventField::Timing);
        }
        if self.availability.is_some() {
            fields.insert(EventField::Availability);
        }
        if self.visibility.is_some() {
            fields.insert(EventField::Visibility);
        }
        if self.reminders != FieldUpdate::Keep {
            fields.insert(EventField::Reminders);
        }
        if let Some(attendees) = &self.attendees
            && !attendees.is_empty()
        {
            fields.insert(EventField::Attendees);
        }
        if self.recurrence != FieldUpdate::Keep {
            fields.insert(EventField::Recurrence);
        }
        if self.conference.is_some() {
            fields.insert(EventField::Conference);
        }
        fields
    }

    pub fn is_empty(&self) -> bool {
        self.touched_fields().is_empty()
    }

    #[must_use]
    pub fn without_base(&self) -> EventPatch {
        let mut copy = self.clone();
        copy.base = None;
        copy
    }

    /// The minimal patch that turns `original` into `edited`. Compares title, notes, location, timing, availability,
    /// visibility, reminders and attendees (by normalized email; a changed role or name is an upsert), plus the
    /// removal of a conference. Ignores provider-owned fields (ids, uid, calendar, source, organizer, status, kind,
    /// url, version, `series`, `participation`, attendee responses and flags), a changed or added conference
    /// (only `Generate` and `Remove` are writable) and recurrence (reads carry none). (Swift `init(from:to:)`.)
    pub fn diff(original: &CalendarEvent, edited: &CalendarEvent) -> EventPatch {
        let mut patch = EventPatch::default();
        if edited.title != original.title {
            patch.title = Some(edited.title.clone());
        }
        patch.notes = EventPatch::update(original.notes.as_deref(), edited.notes.as_deref());
        patch.location =
            EventPatch::update(original.location.as_deref(), edited.location.as_deref());
        let same_timing = original.start == edited.start
            && original.end == edited.end
            && original.time_zone.identifier() == edited.time_zone.identifier()
            && original.is_all_day == edited.is_all_day;
        if !same_timing {
            patch.timing = Some(EventTiming::new(
                edited.start,
                edited.end,
                Some(edited.time_zone.clone()),
                edited.is_all_day,
            ));
        }
        // A `None` value on the edited copy means "unknown", which is never a change.
        if let Some(value) = edited.availability
            && Some(value) != original.availability
        {
            patch.availability = Some(value);
        }
        if let Some(value) = edited.visibility
            && Some(value) != original.visibility
        {
            patch.visibility = Some(value);
        }
        if let Some(value) = &edited.reminders {
            let empty = Vec::new();
            let before = original.reminders.as_ref().unwrap_or(&empty);
            if !Reminder::same_set(value, before) || original.reminders.is_none() {
                patch.reminders = if !value.is_empty()
                    && value.iter().all(|r| r.is_calendar_default == Some(true))
                {
                    FieldUpdate::Clear
                } else {
                    FieldUpdate::Set(value.clone())
                };
            }
        }
        patch.attendees = EventPatch::attendee_changes(&original.attendees, &edited.attendees);
        let has_structured = |event: &CalendarEvent| {
            event
                .conferences
                .iter()
                .any(|c| c.origin == ConferenceOrigin::Structured)
        };
        if has_structured(original) && !has_structured(edited) {
            patch.conference = Some(ConferenceChange::Remove);
        }
        patch.base = Some(original.clone());
        patch
    }

    fn update(old: Option<&str>, new: Option<&str>) -> FieldUpdate<String> {
        if old == new {
            return FieldUpdate::Keep;
        }
        match new {
            Some(value) => FieldUpdate::Set(value.to_string()),
            None => FieldUpdate::Clear,
        }
    }

    fn attendee_changes(original: &[Attendee], edited: &[Attendee]) -> Option<AttendeeChanges> {
        // Provider data may repeat an email; the first occurrence wins.
        let mut before: BTreeMap<&str, &Attendee> = BTreeMap::new();
        for attendee in original.iter().filter(|a| !a.is_self) {
            if let Some(email) = attendee.email() {
                before.entry(email).or_insert(attendee);
            }
        }
        // The account owner is never added or removed by an edit, even if the copy lost the `is_self` flag.
        let self_emails: BTreeSet<&str> = original
            .iter()
            .filter(|a| a.is_self)
            .filter_map(Attendee::email)
            .collect();
        let mut add = Vec::new();
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for attendee in edited.iter().filter(|a| !a.is_self) {
            let Some(email) = attendee.email() else {
                continue;
            };
            if self_emails.contains(email) || !seen.insert(email) {
                continue;
            }
            // A draft cannot clear a name, so dropping one is not a change (it would touch attendees for nothing).
            if let Some(old) = before.get(email)
                && old.role == attendee.role
                && (attendee.name.is_none() || old.name == attendee.name)
            {
                continue;
            }
            add.push(AttendeeDraft::new(
                email,
                attendee.name.as_deref(),
                attendee.role,
            ));
        }
        let mut remove: Vec<String> = before
            .keys()
            .filter(|email| !seen.contains(**email))
            .map(|email| email.to_string())
            .collect();
        remove.sort();
        let changes = AttendeeChanges::new(add, remove);
        if changes.is_empty() {
            None
        } else {
            Some(changes)
        }
    }

    /// The event with this patch applied. Recurrence and a generated conference are not representable on
    /// `CalendarEvent`; `Remove` clears the conference. Used by the in-memory test source.
    pub fn applied_to(&self, event: &CalendarEvent) -> CalendarEvent {
        let mut e = event.clone();
        if let Some(title) = &self.title {
            e.title = title.clone();
        }
        match &self.notes {
            FieldUpdate::Keep => {}
            FieldUpdate::Set(value) => e.notes = Some(value.clone()),
            FieldUpdate::Clear => e.notes = None,
        }
        match &self.location {
            FieldUpdate::Keep => {}
            FieldUpdate::Set(value) => e.location = Some(value.clone()),
            FieldUpdate::Clear => e.location = None,
        }
        if let Some(timing) = &self.timing {
            e.start = timing.start;
            e.end = timing.end;
            if let Some(zone) = &timing.time_zone {
                e.time_zone = zone.clone();
            }
            e.is_all_day = timing.is_all_day;
        }
        if let Some(availability) = self.availability {
            e.availability = Some(availability);
        }
        if let Some(visibility) = self.visibility {
            e.visibility = Some(visibility);
        }
        match &self.reminders {
            FieldUpdate::Keep => {}
            FieldUpdate::Set(value) => e.reminders = Some(value.clone()),
            FieldUpdate::Clear => e.reminders = None,
        }
        if let Some(attendees) = &self.attendees {
            let removed: BTreeSet<&str> = attendees.remove.iter().map(String::as_str).collect();
            e.attendees
                .retain(|a| !a.email().is_some_and(|email| removed.contains(email)));
            for draft in &attendees.add {
                if let Some(existing) = e
                    .attendees
                    .iter_mut()
                    .find(|a| a.email() == Some(draft.email()))
                {
                    if let Some(name) = &draft.name {
                        existing.name = Some(name.clone());
                    }
                    existing.role = draft.role;
                } else {
                    e.attendees.push(Attendee::new(
                        draft.name.clone(),
                        Some(draft.email()),
                        draft.role,
                        crate::model::ResponseStatus::NeedsAction,
                        false,
                        false,
                    ));
                }
            }
        }
        if self.conference == Some(ConferenceChange::Remove) {
            e.conferences = Vec::new();
        }
        e
    }
}

/// A tracked edit: the event as read plus a working copy. `patch` is what changed, `has_changes` whether anything did.
#[derive(Clone, Debug, PartialEq)]
pub struct EventEdit {
    original: CalendarEvent,
    pub event: CalendarEvent,
}

impl EventEdit {
    pub fn new(original: CalendarEvent) -> EventEdit {
        EventEdit {
            event: original.clone(),
            original,
        }
    }

    /// The event as read (Swift `let original`).
    pub fn original(&self) -> &CalendarEvent {
        &self.original
    }

    pub fn patch(&self) -> EventPatch {
        EventPatch::diff(&self.original, &self.event)
    }

    pub fn has_changes(&self) -> bool {
        !self.patch().is_empty()
    }
}
