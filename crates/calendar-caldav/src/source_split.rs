//! Port of `CalDAVCalendarSource+Split.swift`: `.thisAndFollowing` after the first occurrence.
//!
//! Swift runs the lookups and the restore in unstructured tasks, so they still happen when the caller was cancelled.
//! A Rust caller cancels by dropping the future, and nothing can be awaited after that: a future dropped between the
//! two writes leaves the server as the last completed request left it. A cancellation that arrives as an error
//! (`Failure::Cancelled` from the transport) is handled like any other unclear failure, as in Swift.

use calendar_core::failure::{Failure, Outcome};
use calendar_core::model::CalendarEvent;
use calendar_core::source_types::SourceError;
use calendar_core::time::{Zone, add_seconds};
use calendar_core::write::event_patch::{EventPatch, FieldUpdate};
use calendar_core::write::patch_merge::Attempt;
use calendar_core::write::types::{EventField, EventRef, WriteError};
use calendar_ical::{
    EventReader, EventResource, EventWriter, ICalComponent, ICalProperty, SeriesEditor,
};
use jiff::Timestamp;

use crate::source::CalDavCalendarSource;
use crate::source_write::{FetchedResource, PutOutcome, invalid};

/// What became of the write that cuts the original series short.
enum HeadOutcome {
    /// The truncation is on the server under `etag`.
    Stored {
        etag: String,
        resource: EventResource,
    },
    /// 412: the copy read is out of date; nothing was written.
    Stale,
    /// The write failed and a look at the resource shows it is untouched.
    NotApplied(Failure),
    /// The write failed but the truncation is on the server (`etag`): the request's outcome was unclear.
    AppliedDespiteError {
        etag: String,
        resource: EventResource,
        cause: Failure,
    },
    /// The write failed and the resource now holds someone else's edit.
    ChangedByOthers(Failure),
    /// The write failed and the resource could not be looked at.
    Unknown { cause: Failure, lookup: Failure },
}

/// What became of the write that creates the new series.
enum TailOutcome {
    Stored {
        etag: String,
        resource: EventResource,
    },
    /// Definitely not on the server.
    Absent(Failure),
    /// The write failed and the server could not be asked whether it went through.
    Unknown { cause: Failure, lookup: Failure },
}

impl CalDavCalendarSource {
    /// Whether a failed request may nevertheless have been applied: the reply was lost (a 5xx, a broken connection, a
    /// cancellation) or the read-back after a 2xx failed, rather than the request refused. Refusals (`WriteError`, an
    /// authentication or rate-limit answer) changed nothing.
    pub(crate) fn might_have_applied(error: &Failure) -> bool {
        match error {
            Failure::Write(_) => false,
            Failure::Source(source) => match source {
                SourceError::AuthExpired
                | SourceError::RateLimited { .. }
                | SourceError::NeedsPermission
                | SourceError::NotFound => false,
                SourceError::Network(_)
                | SourceError::Server { .. }
                | SourceError::InvalidResponse(_) => true,
            },
            Failure::Cancelled | Failure::Host(_) => true,
        }
    }

    /// `.thisAndFollowing` after the first occurrence: (1) the original resource keeps the occurrences before `slot`
    /// (`If-Match` its ETag), (2) a new resource with a new UID holds the rest with the patch applied, carrying the
    /// later overrides and exclusions. If (2) fails, the original body goes back with `If-Match` on the ETag (1)
    /// returned; a 412 there (someone edited meanwhile) is not overwritten and the result is `WriteError::Partial`. A
    /// write whose outcome is unclear (lost reply, cancellation, failed read-back) is looked up before anything is
    /// restored, so a step that did go through is never duplicated or undone blindly.
    pub(crate) async fn split_series(
        &self,
        current: &FetchedResource,
        reference: &EventRef,
        patch: &EventPatch,
        slot: Timestamp,
        calendar_zone: &Zone,
    ) -> Outcome<Attempt<CalendarEvent>> {
        // An attendee cannot split the organizer's series: the new UID would be unknown to the organizer, whose next
        // update would bring the whole series back next to it (as for `respond`).
        if let Some(master) = current.resource.master() {
            let (_, organizer) =
                calendar_ical::AttendeeMapper::read(master, &self.self_addresses());
            if organizer.is_some_and(|o| !o.is_self) {
                return Err(WriteError::Unsupported {
                    fields: [EventField::Attendees].into(),
                }
                .into());
            }
        }
        let stamp = (self.now)();
        let new_uid = (self.make_uuid)();
        let new_name = format!("{}.ics", (self.make_uuid)());
        let (head, mut tail) =
            SeriesEditor::split(&current.resource, slot, &new_uid, calendar_zone, stamp)?;
        let mut tail_master = tail.master().cloned().ok_or(WriteError::NotFound)?;
        let before = EventReader::timing_of(&tail_master, &tail.resolver(), calendar_zone)
            .ok_or(WriteError::NotFound)?;
        // Carried exceptions and exclusions cannot follow a switch between all-day and timed (as in `edit`).
        if let Some(wanted) = &patch.timing
            && wanted.is_all_day != before.is_all_day
            && (!tail.overrides().is_empty() || tail_master.property("EXDATE").is_some())
        {
            return Err(WriteError::Unsupported {
                fields: [EventField::Timing].into(),
            }
            .into());
        }
        let mut tail_patch = patch.clone();
        let mut delta = 0.0;
        if let Some(wanted) = &patch.timing {
            let (moved, shift) = Self::series_timing(&before, slot, wanted, calendar_zone);
            tail_patch.timing = Some(moved);
            delta = shift;
        }
        EventWriter::apply(
            &tail_patch,
            &mut tail_master,
            stamp,
            self.organizer_address().as_deref(),
            Some(calendar_zone),
        )?;
        tail_master.set(ICalProperty::with_value("SEQUENCE", "0"));
        // The carried exceptions are new components under a new UID: stamp them like the master (SeriesEditor::split
        // moves them across without touching DTSTAMP/LAST-MODIFIED).
        let carried: Vec<ICalComponent> = tail
            .overrides()
            .into_iter()
            .map(|vevent| {
                let mut copy = vevent.clone();
                EventWriter::touch(&mut copy, stamp, false);
                copy
            })
            .collect();
        let mut events = vec![tail_master];
        events.extend(carried);
        tail.set_events(events);
        if delta != 0.0 {
            SeriesEditor::shift(&mut tail, delta, calendar_zone);
        }
        if patch.recurrence != FieldUpdate::Keep {
            SeriesEditor::prune_unmatched(&mut tail, calendar_zone);
        }
        if let Some(timing) = &patch.timing
            && let Some(zone) = &timing.time_zone
        {
            tail.ensure_time_zones(
                std::slice::from_ref(zone),
                timing.start,
                add_seconds(timing.start, 20.0 * 366.0 * 86_400.0),
            );
        }

        let truncated: (String, EventResource) = match self
            .truncate(&head, current, &reference.calendar_id)
            .await
        {
            HeadOutcome::Stale => return Ok(Attempt::Stale),
            HeadOutcome::Stored { etag, resource } => (etag, resource),
            HeadOutcome::NotApplied(error) => return Err(error),
            HeadOutcome::AppliedDespiteError {
                etag,
                resource,
                cause,
            } => {
                // The truncation is on the server although the request reported an error: put the series back and
                // report the error, so the caller may retry from a whole series.
                return Err(self
                    .restore_series(
                        current,
                        (etag, resource),
                        cause,
                        "the series was cut short and the request reported an error",
                    )
                    .await);
            }
            HeadOutcome::ChangedByOthers(cause) => {
                return Err(WriteError::Partial(format!(
                    "cutting the series short failed with an unclear outcome ({cause}) and the series was changed meanwhile; check the calendar"
                ))
                .into());
            }
            HeadOutcome::Unknown { cause, lookup } => {
                return Err(WriteError::Partial(format!(
                    "cutting the series short failed with an unknown outcome ({cause}) and looking at the series failed ({lookup}); check the calendar"
                ))
                .into());
            }
        };

        match self
            .create_tail(&tail, &new_uid, &reference.calendar_id, &new_name)
            .await
        {
            TailOutcome::Stored { etag, resource } => Ok(Attempt::Done(
                self.read_back(&resource, &etag, &reference.calendar_id, &new_name, None)
                    .await?,
            )),
            TailOutcome::Absent(cause) => Err(self
                .restore_series(
                    current,
                    truncated,
                    cause,
                    "the series was cut short but the new series was not created",
                )
                .await),
            TailOutcome::Unknown { cause, lookup } => {
                // Restoring would duplicate the series if the new one exists, and leaving it loses occurrences if it does not.
                Err(WriteError::Partial(format!(
                    "the series was cut short and the outcome of creating the new series is unknown ({cause}; lookup: {lookup}); check the calendar"
                ))
                .into())
            }
        }
    }

    /// PUTs the truncated original under `If-Match`. After a failure that may have been applied, looks at the resource:
    /// the ETag read earlier means nothing was written; our own stamp means the truncation is there.
    async fn truncate(
        &self,
        head: &EventResource,
        current: &FetchedResource,
        calendar_id: &str,
    ) -> HeadOutcome {
        match self
            .put(head, &current.url, Some(&current.etag), false)
            .await
        {
            Ok(PutOutcome::Stale) => HeadOutcome::Stale,
            Ok(PutOutcome::Stored { etag, resource }) => HeadOutcome::Stored { etag, resource },
            Err(error) => {
                if !Self::might_have_applied(&error) {
                    return HeadOutcome::NotApplied(error);
                }
                match self.fetch(calendar_id, &current.name).await {
                    Err(lookup) => HeadOutcome::Unknown {
                        cause: error,
                        lookup,
                    },
                    Ok(fresh) => {
                        if fresh.etag == current.etag {
                            return HeadOutcome::NotApplied(error);
                        }
                        // Ours only if every component is as we wrote it: another client's edit to a kept override
                        // (or a server applying an attendee's reply) must not be mistaken for our truncation and
                        // restored over.
                        if Self::fingerprint(&fresh.resource) == Self::fingerprint(head) {
                            HeadOutcome::AppliedDespiteError {
                                etag: fresh.etag,
                                resource: fresh.resource,
                                cause: error,
                            }
                        } else {
                            HeadOutcome::ChangedByOthers(error)
                        }
                    }
                }
            }
        }
    }

    /// One line per component: its `RECURRENCE-ID` (empty for the master), `DTSTAMP`, `SEQUENCE` and, for the master, `RRULE`.
    fn fingerprint(resource: &EventResource) -> Vec<String> {
        let mut lines: Vec<String> = resource
            .events()
            .into_iter()
            .map(|vevent| {
                ["RECURRENCE-ID", "DTSTAMP", "SEQUENCE", "RRULE"]
                    .iter()
                    .map(|name| {
                        vevent
                            .property(name)
                            .map(|p| p.value.clone())
                            .unwrap_or_default()
                    })
                    .collect::<Vec<_>>()
                    .join("|")
            })
            .collect();
        lines.sort();
        lines
    }

    /// PUTs the new series under `If-None-Match: *`. After a failure that may have been applied, asks the server for the
    /// name (it is ours alone): found means the write went through.
    async fn create_tail(
        &self,
        tail: &EventResource,
        uid: &str,
        calendar_id: &str,
        name: &str,
    ) -> TailOutcome {
        match self
            .put(tail, &self.resource_url(calendar_id, name), None, true)
            .await
        {
            Ok(PutOutcome::Stored { etag, resource }) => TailOutcome::Stored { etag, resource },
            Ok(PutOutcome::Stale) => TailOutcome::Absent(invalid(
                "a resource with the new series' name already exists",
            )),
            Err(error) => {
                if !Self::might_have_applied(&error) {
                    return TailOutcome::Absent(error);
                }
                match self.fetch(calendar_id, name).await {
                    Ok(found) => {
                        if found.resource.uid().as_deref() == Some(uid) {
                            TailOutcome::Stored {
                                etag: found.etag,
                                resource: found.resource,
                            }
                        } else {
                            TailOutcome::Absent(error)
                        }
                    }
                    Err(Failure::Write(WriteError::NotFound)) => TailOutcome::Absent(error),
                    Err(lookup) => TailOutcome::Unknown {
                        cause: error,
                        lookup,
                    },
                }
            }
        }
    }

    /// Puts the original series back over the truncation (`If-Match` the truncation's ETag) and returns `cause` to
    /// throw; a 412 (someone edited the truncated series meanwhile) is not overwritten and any other failure of the
    /// restore is `WriteError::Partial`. The restored master carries a higher `SEQUENCE` than the truncation and a new
    /// `DTSTAMP`: attendees' apps ignore an update that is not newer than the cut-short copy they already hold.
    async fn restore_series(
        &self,
        current: &FetchedResource,
        truncated: (String, EventResource),
        cause: Failure,
        why: &str,
    ) -> Failure {
        let mut restored = current.resource.clone();
        let sequence = truncated
            .1
            .master()
            .and_then(|m| m.property("SEQUENCE"))
            .and_then(|p| p.value.parse::<i64>().ok())
            .unwrap_or(0)
            + 1;
        let stamp = (self.now)();
        let events: Vec<ICalComponent> = restored
            .events()
            .into_iter()
            .map(|vevent| {
                let mut copy = vevent.clone();
                EventWriter::touch(&mut copy, stamp, false);
                if copy.property("RECURRENCE-ID").is_none() {
                    copy.set(ICalProperty::with_value("SEQUENCE", sequence.to_string()));
                }
                copy
            })
            .collect();
        restored.set_events(events);
        match self
            .put(&restored, &current.url, Some(&truncated.0), false)
            .await
        {
            Ok(PutOutcome::Stored { .. }) => cause,
            Ok(PutOutcome::Stale) => WriteError::Partial(format!(
                "{why} ({cause}); it was changed meanwhile, so it was not restored"
            ))
            .into(),
            Err(restore_error) => WriteError::Partial(format!(
                "{why} ({cause}); restoring it failed ({restore_error})"
            ))
            .into(),
        }
    }
}
