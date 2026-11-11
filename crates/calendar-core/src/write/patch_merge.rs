//! Port of `Write/PatchMerge.swift`.

use std::future::Future;

use crate::failure::Outcome;
use crate::model::{AttendeeRole, CalendarEvent};
use crate::write::event_patch::EventPatch;
use crate::write::types::{EventField, WriteError};

/// Field-level conflict handling shared by every connector that has a version to compare (Google etag, EventKit
/// modification date). A stale version is not an error by itself: it is a conflict only when someone changed a
/// field this patch touches.
pub enum PatchMerge {}

/// The outcome of one write attempt.
pub enum Attempt<Result> {
    Done(Result),
    /// The provider rejected the write because the version is out of date.
    Stale,
}

impl PatchMerge {
    /// Swift's default for `apply`'s `maxAttempts`.
    pub const DEFAULT_MAX_ATTEMPTS: i64 = 3;

    /// The touched fields whose value in `current` differs from `patch.base`. A patch without a base cannot be
    /// compared, so every touched field is a conflict. Attendees are compared only for the emails the patch adds or
    /// removes (a concurrent change to someone else is not a conflict). Recurrence cannot be read back, so a patch
    /// that touches it always conflicts on a stale version.
    pub fn conflicts(
        patch: &EventPatch,
        current: &CalendarEvent,
    ) -> std::collections::BTreeSet<EventField> {
        let Some(base) = patch.base() else {
            return patch.touched_fields();
        };
        let mut found = std::collections::BTreeSet::new();
        for field in patch.touched_fields() {
            let conflicted = match field {
                EventField::Title => current.title != base.title,
                EventField::Notes => {
                    current.notes.as_deref().unwrap_or("") != base.notes.as_deref().unwrap_or("")
                }
                EventField::Location => {
                    current.location.as_deref().unwrap_or("")
                        != base.location.as_deref().unwrap_or("")
                }
                EventField::Timing => {
                    !(current.start == base.start
                        && current.end == base.end
                        && current.time_zone.identifier() == base.time_zone.identifier()
                        && current.is_all_day == base.is_all_day)
                }
                EventField::Availability => {
                    PatchMerge::differs(&current.availability, &base.availability)
                }
                EventField::Visibility => {
                    PatchMerge::differs(&current.visibility, &base.visibility)
                }
                EventField::Reminders => match (&current.reminders, &base.reminders) {
                    (Some(c), Some(b)) => !crate::model::Reminder::same_set(c, b),
                    _ => false,
                },
                EventField::Attendees => PatchMerge::attendees_differ(patch, base, current),
                EventField::Recurrence => true,
                EventField::Conference => {
                    PatchMerge::structured_urls(current) != PatchMerge::structured_urls(base)
                }
            };
            if conflicted {
                found.insert(field);
            }
        }
        found
    }

    /// A value the source does not say (`None`) can neither conflict nor be conflicted with.
    fn differs<Value: PartialEq>(current: &Option<Value>, base: &Option<Value>) -> bool {
        match (current, base) {
            (Some(current), Some(base)) => current != base,
            _ => false,
        }
    }

    /// The provider's own conference links; links found in the notes change whenever the notes do, which is not a
    /// conference change.
    fn structured_urls(event: &CalendarEvent) -> Vec<&crate::url_string::UrlString> {
        event
            .conferences
            .iter()
            .filter(|c| c.origin == crate::model::ConferenceOrigin::Structured)
            .map(|c| &c.url)
            .collect()
    }

    /// An email conflicts when its role changed between `base` and `current` and `current` does not already hold
    /// what the patch wants (an added attendee with the requested role, or one that is already gone).
    fn attendees_differ(patch: &EventPatch, base: &CalendarEvent, current: &CalendarEvent) -> bool {
        let Some(changes) = &patch.attendees else {
            return false;
        };
        fn role(email: &str, event: &CalendarEvent) -> Option<AttendeeRole> {
            event
                .attendees
                .iter()
                .find(|a| a.email() == Some(email))
                .map(|a| a.role)
        }
        let changed = |email: &str, wanted: Option<AttendeeRole>| {
            let now = role(email, current);
            now != role(email, base) && now != wanted
        };
        changes.remove.iter().any(|email| changed(email, None))
            || changes
                .add
                .iter()
                .any(|draft| changed(draft.email(), Some(draft.role)))
    }

    /// Runs `write` with `version`. When it reports `Stale`, fetches the current event and judges the patch against
    /// it: a real overlap fails with `Conflict`; otherwise the write is retried on the fresh version, and a further
    /// stale result is judged the same way. After `max_attempts` writes it fails with
    /// `Conflict { fields: patch.touched_fields() }` (the event is being edited concurrently). It fails closed the
    /// same way when a retry is due but the fresh event carries no version although the write was versioned:
    /// retrying without one would be an unconditional write that could overwrite an edit made after the fetch. A
    /// write that started without a version has nothing to lose and retries as before.
    ///
    /// Swift checks `Task.checkCancellation()` before each write. A Rust future has no such check: the caller
    /// cancels by dropping it, which stops the loop at its next `.await`.
    pub async fn apply<R, Fetch, FetchFuture, Write, WriteFuture>(
        patch: &EventPatch,
        version: Option<String>,
        max_attempts: i64,
        mut fetch_current: Fetch,
        mut write: Write,
    ) -> Outcome<R>
    where
        Fetch: FnMut() -> FetchFuture,
        FetchFuture: Future<Output = Outcome<CalendarEvent>>,
        Write: FnMut(Option<String>) -> WriteFuture,
        WriteFuture: Future<Output = Outcome<Attempt<R>>>,
    {
        let mut version = version;
        // At least one write, whatever the caller passed.
        for _ in 0..max_attempts.max(1) {
            match write(version.clone()).await? {
                Attempt::Done(result) => return Ok(result),
                Attempt::Stale => {
                    let current = fetch_current().await?;
                    let overlapping = PatchMerge::conflicts(patch, &current);
                    if !overlapping.is_empty() {
                        return Err(WriteError::Conflict {
                            fields: overlapping,
                        }
                        .into());
                    }
                    if version.is_some() && current.version.is_none() {
                        return Err(WriteError::Conflict {
                            fields: patch.touched_fields(),
                        }
                        .into());
                    }
                    version = current.version;
                }
            }
        }
        Err(WriteError::Conflict {
            fields: patch.touched_fields(),
        }
        .into())
    }
}
