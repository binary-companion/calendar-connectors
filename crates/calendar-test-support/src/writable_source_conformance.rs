//! Port of `CalendarTestSupport/WritableSourceConformance.swift`.

use calendar_core::failure::Failure;
use calendar_core::model::{AttendeeRole, CalendarEvent};
use calendar_core::time::{DateInterval, Zone, add_seconds};
use calendar_core::write::event_draft::{AttendeeDraft, EventDraft, EventTiming};
use calendar_core::write::event_patch::{AttendeeChanges, EventPatch};
use calendar_core::write::types::{
    EventField, EventRef, NotifyPolicy, RecurrenceScope, WriteError,
};
use calendar_core::write::writable_source::WritableCalendarSource;

use crate::hand_patch;

/// Behaviour every `WritableCalendarSource` must have. Run it against a source and a scratch calendar (the fake
/// here, the real EventKit source in the live tests). An empty result means the source conforms. It cleans up the
/// event it creates, even when a check fails part way.
pub enum WritableSourceConformance {}

impl WritableSourceConformance {
    /// Runs the conformance checks against `source` using the calendar `calendar_id`, placing the test event inside
    /// `window`. Returns a description of each violated expectation; an empty vector means the source conforms. The
    /// event it creates is deleted before returning.
    pub async fn violations(
        source: &dyn WritableCalendarSource,
        calendar_id: &str,
        window: DateInterval,
    ) -> Vec<String> {
        let mut found = Vec::new();
        let caps = source.capabilities();
        if !caps.can_write {
            found.push("canWrite is false on a WritableCalendarSource".to_string());
        }
        if caps.can_edit_attendees != caps.writable_fields.contains(&EventField::Attendees) {
            found.push(
                "canEditAttendees does not match writableFields.contains(.attendees)".to_string(),
            );
        }
        // Only ask for what the source says it can write, so a source with fewer fields is not failed for create.
        let uses_location = caps.writable_fields.contains(&EventField::Location);
        let start = add_seconds(window.start, 3600.0);
        let draft = EventDraft {
            location: if uses_location {
                Some("Room 1".to_string())
            } else {
                None
            },
            ..EventDraft::new(
                "Conformance A",
                EventTiming::new(start, add_seconds(start, 1800.0), Some(Zone::utc()), false),
            )
        };
        let created = match source.create(&draft, calendar_id, NotifyPolicy::None).await {
            Ok(created) => created,
            Err(error) => {
                found.push(format!("create threw {error}"));
                return found;
            }
        };
        // Where the event is now, so a failure part way still deletes it.
        let mut latest = created.clone();
        let mut deleted = false;
        if created.title != "Conformance A" || created.calendar_id != calendar_id {
            found.push("create returned a different title or calendar".to_string());
        }

        if let Err(error) = Self::checks(
            source,
            window,
            &caps.writable_fields,
            uses_location,
            &created,
            &mut latest,
            &mut deleted,
            &mut found,
        )
        .await
        {
            found.push(format!("threw {error}"));
        }
        if !deleted
            && let Err(error) = source
                .delete(
                    &EventRef::from_event(&latest),
                    RecurrenceScope::ThisInstance,
                    NotifyPolicy::None,
                )
                .await
        {
            found.push(format!(
                "cleanup delete failed, the test event may be left behind: {error}"
            ));
        }
        found
    }

    /// The checks that run against the created event; any `Err` is reported by the caller as "threw ...".
    #[allow(clippy::too_many_arguments)]
    async fn checks(
        source: &dyn WritableCalendarSource,
        window: DateInterval,
        writable_fields: &std::collections::BTreeSet<EventField>,
        uses_location: bool,
        created: &CalendarEvent,
        latest: &mut CalendarEvent,
        deleted: &mut bool,
        found: &mut Vec<String>,
    ) -> Result<(), Failure> {
        let scope = RecurrenceScope::ThisInstance;
        let notify = NotifyPolicy::None;
        let listed = source.events_in(window).await?;
        if !listed.iter().any(|e| e.event_id == created.event_id) {
            found.push("the created event is not returned by events(in:)".to_string());
        }
        let rename = hand_patch(|p| p.title = Some("Conformance B".to_string()));
        let renamed = source
            .update(&EventRef::from_event(latest), &rename, scope, notify)
            .await?;
        *latest = renamed.clone();
        if renamed.title != "Conformance B" {
            found.push("update did not change the title".to_string());
        }
        if uses_location && renamed.location.as_deref() != Some("Room 1") {
            found.push("update changed a field the patch did not touch (location)".to_string());
        }
        if renamed.start != created.start
            || renamed.end != created.end
            || renamed.is_all_day != created.is_all_day
        {
            found.push("update changed a field the patch did not touch (timing)".to_string());
        }
        if renamed.version.is_some() && renamed.version == created.version {
            found.push("update did not change the version".to_string());
        }

        let unchanged = source
            .update(
                &EventRef::from_event(&renamed),
                &EventPatch::default(),
                scope,
                notify,
            )
            .await?;
        // The ref for the later delete follows whatever the source last returned, so a version-checking source
        // does not see a stale ref.
        *latest = unchanged.clone();
        if unchanged.title != "Conformance B" {
            found.push("an empty patch changed the event".to_string());
        }
        if renamed.version.is_some() && unchanged.version != renamed.version {
            found.push("an empty patch changed the version".to_string());
        }

        if !writable_fields.contains(&EventField::Attendees) {
            let add_attendee = hand_patch(|p| {
                p.attendees = Some(AttendeeChanges::new(
                    vec![AttendeeDraft::new("a@b.c", None, AttendeeRole::Required)],
                    vec![],
                ));
            });
            match source
                .update(
                    &EventRef::from_event(&renamed),
                    &add_attendee,
                    scope,
                    notify,
                )
                .await
            {
                Ok(_) => found.push("an unwritable field (attendees) was accepted".to_string()),
                Err(Failure::Write(WriteError::Unsupported { .. })) => {}
                Err(error) => found.push(format!(
                    "an unwritable field threw {error} instead of .unsupported"
                )),
            }
        }

        source
            .delete(&EventRef::from_event(latest), scope, notify)
            .await?;
        *deleted = true;
        let remaining = source.events_in(window).await?;
        if remaining.iter().any(|e| e.event_id == created.event_id) {
            found.push("the event is still present after delete".to_string());
        }
        match source
            .delete(&EventRef::from_event(latest), scope, notify)
            .await
        {
            Ok(()) => found.push("deleting a missing event did not throw".to_string()),
            Err(Failure::Write(WriteError::NotFound)) => {}
            Err(error) => found.push(format!(
                "deleting a missing event threw {error} instead of .notFound"
            )),
        }
        Ok(())
    }
}
