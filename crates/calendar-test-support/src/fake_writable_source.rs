//! Port of `CalendarTestSupport/FakeWritableSource.swift`.

use std::collections::{BTreeMap, BTreeSet};
use std::future::ready;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use calendar_core::failure::Outcome;
use calendar_core::model::{
    Attendee, CalendarDescriptor, CalendarEvent, CalendarPermissions, CalendarService,
    ConferenceInfo, ConferenceOrigin, ConferenceProvider, Participation, Reminder, ResponseStatus,
    UidScope,
};
use calendar_core::source_types::{CalendarChange, CalendarSource, SourceCapabilities};
use calendar_core::time::{DateInterval, Zone, add_seconds};
use calendar_core::url_string::UrlString;
use calendar_core::write::event_draft::{EventDraft, EventTiming};
use calendar_core::write::event_patch::{EventPatch, FieldUpdate};
use calendar_core::write::patch_merge::{Attempt, PatchMerge};
use calendar_core::write::types::{
    ConferenceChange, ConferenceRequest, EventField, EventRef, NotifyPolicy, RecurrenceScope,
    WriteError, WriteValidation,
};
use calendar_core::write::writable_source::WritableCalendarSource;
use jiff::Timestamp;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

#[derive(Default)]
struct State {
    /// By `CalendarEvent::id`.
    events: BTreeMap<String, CalendarEvent>,
    counter: i64,
    writes: i64,
}

/// An in-memory `WritableCalendarSource` for tests. Non-recurring events only: the scope is ignored and recurrence is
/// not modelled, so by default `EventField::Recurrence` is not among the writable fields and a draft or patch that
/// sets it fails with `Unsupported { fields: [Recurrence] }`. It never has a self attendee of its own; a test that
/// needs one adds it with [`FakeWritableSource::simulate_external_edit`].
pub struct FakeWritableSource {
    capabilities: SourceCapabilities,
    calendar_ids: BTreeSet<String>,
    state: Mutex<State>,
}

impl Default for FakeWritableSource {
    fn default() -> FakeWritableSource {
        FakeWritableSource::new()
    }
}

impl FakeWritableSource {
    const ID: &'static str = "fake-writable";

    /// Swift's `init` defaults: one calendar `cal`, every field but recurrence writable, RSVP supported.
    pub fn new() -> FakeWritableSource {
        FakeWritableSource::with_options(
            &["cal"],
            FakeWritableSource::default_writable_fields(),
            true,
        )
    }

    /// Swift's default for `writableFields`: every field except recurrence.
    pub fn default_writable_fields() -> BTreeSet<EventField> {
        EventField::ALL
            .iter()
            .copied()
            .filter(|field| *field != EventField::Recurrence)
            .collect()
    }

    /// Swift's `init(calendarIDs:writableFields:canRespond:)`.
    pub fn with_options(
        calendar_ids: &[&str],
        writable_fields: BTreeSet<EventField>,
        can_respond: bool,
    ) -> FakeWritableSource {
        FakeWritableSource {
            calendar_ids: calendar_ids.iter().map(|id| id.to_string()).collect(),
            capabilities: SourceCapabilities {
                can_write: true,
                can_edit_attendees: writable_fields.contains(&EventField::Attendees),
                can_respond_to_invite: can_respond,
                writable_fields,
                controls_notifications: true,
                recurrence_scopes: RecurrenceScope::ALL.iter().copied().collect(),
                ..SourceCapabilities::default()
            },
            state: Mutex::new(State::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }

    /// Number of successful writes; lets tests assert that a call wrote nothing.
    pub fn write_count(&self) -> i64 {
        self.lock().writes
    }

    /// Simulates another writer: applies `mutate` and bumps the version. Returns false (and does nothing) when there is
    /// no such event, so a test that asserts the result notices a mistyped id.
    pub fn simulate_external_edit(
        &self,
        calendar_id: &str,
        event_id: &str,
        mutate: impl FnOnce(&mut CalendarEvent),
    ) -> bool {
        let mut state = self.lock();
        let key = format!("{calendar_id}/{event_id}");
        let Some(mut event) = state.events.get(&key).cloned() else {
            return false;
        };
        mutate(&mut event);
        state.counter += 1;
        event.version = Some(format!("v{}", state.counter));
        state.events.insert(key, event);
        true
    }

    /// The patch's attendee emails, reminders and timing get the same checks as a draft's. Only the patch's values
    /// matter, so they ride on a throwaway draft.
    fn validate_patch(patch: &EventPatch) -> Result<(), WriteError> {
        let placeholder = Timestamp::UNIX_EPOCH;
        let timing = patch.timing.clone().unwrap_or_else(|| {
            EventTiming::new(placeholder, add_seconds(placeholder, 1.0), None, false)
        });
        let reminders: Option<Vec<Reminder>> = match &patch.reminders {
            FieldUpdate::Set(set) => Some(set.clone()),
            _ => None,
        };
        EventDraft {
            reminders,
            attendees: patch
                .attendees
                .as_ref()
                .map(|changes| changes.add.clone())
                .unwrap_or_default(),
            ..EventDraft::new("", timing)
        }
        .validate()
    }

    fn write(
        &self,
        key: &str,
        version: Option<&str>,
        patch: &EventPatch,
    ) -> Outcome<Attempt<CalendarEvent>> {
        let mut state = self.lock();
        let Some(current) = state.events.get(key).cloned() else {
            return Err(WriteError::NotFound.into());
        };
        if let Some(version) = version
            && current.version.as_deref() != Some(version)
        {
            return Ok(Attempt::Stale);
        }
        let mut updated = patch.applied_to(&current);
        if patch.conference == Some(ConferenceChange::Generate) {
            updated.conferences = vec![meet_link(state.counter + 1)];
        }
        state.counter += 1;
        state.writes += 1;
        updated.version = Some(format!("v{}", state.counter));
        state.events.insert(key.to_string(), updated.clone());
        Ok(Attempt::Done(updated))
    }
}

fn meet_link(number: i64) -> ConferenceInfo {
    ConferenceInfo::new(
        UrlString::parse(&format!("https://meet.example/{number}")).unwrap(),
        ConferenceProvider::Meet,
        ConferenceOrigin::Structured,
    )
}

#[async_trait]
impl CalendarSource for FakeWritableSource {
    fn id(&self) -> String {
        FakeWritableSource::ID.to_string()
    }

    fn display_name(&self) -> String {
        "Fake writable".to_string()
    }

    fn capabilities(&self) -> SourceCapabilities {
        self.capabilities.clone()
    }

    async fn calendars(&self) -> Outcome<Vec<CalendarDescriptor>> {
        Ok(self
            .calendar_ids
            .iter()
            .map(|id| {
                let mut calendar =
                    CalendarDescriptor::new(id.clone(), id.clone(), CalendarService::new("fake"));
                calendar.permissions = CalendarPermissions {
                    can_view_details: true,
                    can_edit: true,
                    can_share: Some(true),
                    can_view_private: Some(true),
                };
                calendar
            })
            .collect())
    }

    async fn events_in(&self, interval: DateInterval) -> Outcome<Vec<CalendarEvent>> {
        let mut found: Vec<CalendarEvent> = self
            .lock()
            .events
            .values()
            .filter(|e| e.start < interval.end && e.end > interval.start)
            .cloned()
            .collect();
        found.sort_by_key(|e| (e.start, e.id()));
        Ok(found)
    }

    /// Never reports anything: the stream is finished at once (the sender is dropped).
    fn changes(&self) -> UnboundedReceiver<CalendarChange> {
        let (_sender, receiver) = unbounded_channel();
        receiver
    }

    fn as_writable(&self) -> Option<&dyn WritableCalendarSource> {
        Some(self)
    }
}

#[async_trait]
impl WritableCalendarSource for FakeWritableSource {
    async fn create(
        &self,
        draft: &EventDraft,
        calendar_id: &str,
        _notify: NotifyPolicy,
    ) -> Outcome<CalendarEvent> {
        draft.validate()?;
        WriteValidation::require_writable(&draft.used_fields(), &self.capabilities)?;
        if !self.calendar_ids.contains(calendar_id) {
            return Err(WriteError::NotFound.into());
        }
        let mut state = self.lock();
        if let Some(uid) = &draft.uid
            && let Some(existing) = state
                .events
                .values()
                .find(|e| e.calendar_id == calendar_id && e.uid.as_ref() == Some(uid))
        {
            return Err(WriteError::AlreadyExists(Box::new(existing.clone())).into());
        }
        state.counter += 1;
        state.writes += 1;
        let counter = state.counter;
        let mut event = CalendarEvent {
            uid: draft.uid.clone(),
            uid_scope: if draft.uid.is_none() {
                None
            } else {
                Some(UidScope::Global)
            },
            notes: draft.notes.clone(),
            location: draft.location.clone(),
            time_zone: draft.timing.time_zone.clone().unwrap_or_else(Zone::utc),
            is_all_day: draft.timing.is_all_day,
            availability: Some(draft.availability),
            visibility: Some(draft.visibility),
            attendees: draft
                .attendees
                .iter()
                .map(|a| {
                    Attendee::new(
                        a.name.clone(),
                        Some(a.email()),
                        a.role,
                        ResponseStatus::NeedsAction,
                        false,
                        false,
                    )
                })
                .collect(),
            reminders: draft.reminders.clone(),
            version: Some(format!("v{counter}")),
            source_id: Some(FakeWritableSource::ID.to_string()),
            ..CalendarEvent::new(
                format!("ev{counter}"),
                calendar_id,
                draft.title.clone(),
                draft.timing.start,
                draft.timing.end,
            )
        };
        if draft.conference == ConferenceRequest::Generate {
            event.conferences = vec![meet_link(counter)];
        }
        state.events.insert(event.id(), event.clone());
        Ok(event)
    }

    async fn update(
        &self,
        reference: &EventRef,
        patch: &EventPatch,
        _scope: RecurrenceScope,
        _notify: NotifyPolicy,
    ) -> Outcome<CalendarEvent> {
        WriteValidation::require_writable(&patch.touched_fields(), &self.capabilities)?;
        FakeWritableSource::validate_patch(patch)?;
        let key = format!("{}/{}", reference.calendar_id, reference.event_id);
        let existing = self
            .lock()
            .events
            .get(&key)
            .cloned()
            .ok_or(WriteError::NotFound)?;
        if patch.is_empty() {
            return Ok(patch.base().cloned().unwrap_or(existing));
        }
        PatchMerge::apply(
            patch,
            reference.version.clone(),
            PatchMerge::DEFAULT_MAX_ATTEMPTS,
            || {
                let current = self.lock().events.get(&key).cloned();
                ready(Ok(current.unwrap_or_else(|| existing.clone())))
            },
            |version| ready(self.write(&key, version.as_deref(), patch)),
        )
        .await
    }

    async fn delete(
        &self,
        reference: &EventRef,
        _scope: RecurrenceScope,
        _notify: NotifyPolicy,
    ) -> Outcome<()> {
        let mut state = self.lock();
        let key = format!("{}/{}", reference.calendar_id, reference.event_id);
        if state.events.remove(&key).is_none() {
            return Err(WriteError::NotFound.into());
        }
        state.writes += 1;
        Ok(())
    }

    async fn respond(
        &self,
        reference: &EventRef,
        response: ResponseStatus,
        _scope: RecurrenceScope,
        _notify: NotifyPolicy,
    ) -> Outcome<CalendarEvent> {
        if !self.capabilities.can_respond_to_invite {
            return Err(WriteError::Unsupported {
                fields: BTreeSet::from([EventField::Attendees]),
            }
            .into());
        }
        if response == ResponseStatus::NeedsAction {
            return Err(WriteError::Invalid("cannot respond with needsAction".to_string()).into());
        }
        let key = format!("{}/{}", reference.calendar_id, reference.event_id);
        let mut state = self.lock();
        let Some(mut event) = state.events.get(&key).cloned() else {
            return Err(WriteError::NotFound.into());
        };
        let Some(index) = event.attendees.iter().position(|a| a.is_self) else {
            return Err(
                WriteError::Invalid("you are not an attendee of this event".to_string()).into(),
            );
        };
        event.attendees[index].response = response;
        event.participation = Some(Participation::Invited(response));
        state.counter += 1;
        state.writes += 1;
        event.version = Some(format!("v{}", state.counter));
        state.events.insert(key, event.clone());
        Ok(event)
    }
}
