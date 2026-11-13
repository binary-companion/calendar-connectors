//! Port of `Write/WritableCalendarSource.swift`.

use async_trait::async_trait;

use crate::failure::Outcome;
use crate::model::{CalendarEvent, ResponseStatus};
use crate::source_types::CalendarSource;
use crate::write::event_draft::EventDraft;
use crate::write::event_patch::EventPatch;
use crate::write::types::{EventRef, NotifyPolicy, RecurrenceScope};

/// A source that can also write. Read-only connectors never adopt it; callers check `source.as_writable()`
/// (Swift `source as? WritableCalendarSource`). What a conforming source can write is in `capabilities()`
/// (`writable_fields`, `controls_notifications`, `recurrence_scopes`, `can_edit_attendees`,
/// `can_respond_to_invite`); which calendars are writable is each `CalendarDescriptor::permissions` (`can_edit`).
/// An operation the source cannot perform fails with `WriteError::Unsupported` before changing anything. Writes
/// return the event in the provider's resulting form, including its new `version`.
#[async_trait]
pub trait WritableCalendarSource: CalendarSource {
    /// If the draft has a `uid` and the target calendar already has that event, fails with
    /// `WriteError::AlreadyExists` with the stored copy and writes nothing.
    async fn create(
        &self,
        draft: &EventDraft,
        calendar_id: &str,
        notify: NotifyPolicy,
    ) -> Outcome<CalendarEvent>;

    /// An empty patch writes nothing and returns the patch's base event, or the current event when it has none.
    async fn update(
        &self,
        reference: &EventRef,
        patch: &EventPatch,
        scope: RecurrenceScope,
        notify: NotifyPolicy,
    ) -> Outcome<CalendarEvent>;

    /// Deletes the event or occurrences selected by `scope`. Fails with `WriteError::NotFound` when the event is gone.
    async fn delete(
        &self,
        reference: &EventRef,
        scope: RecurrenceScope,
        notify: NotifyPolicy,
    ) -> Outcome<()>;

    /// `response` must be `Accepted`, `Tentative` or `Declined`.
    async fn respond(
        &self,
        reference: &EventRef,
        response: ResponseStatus,
        scope: RecurrenceScope,
        notify: NotifyPolicy,
    ) -> Outcome<CalendarEvent>;
}
