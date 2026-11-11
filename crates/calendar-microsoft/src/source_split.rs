//! Port of `MicrosoftCalendarSource+Split.swift`: the `.thisAndFollowing` split of a series.
//!
//! Swift runs the restores in unstructured tasks so they happen even for a cancelled caller. A Rust caller cancels by
//! dropping the future, after which nothing can run; the cancellation that can be observed is the transport's
//! `Failure::Cancelled`, which is handled like any other unclear failure (`might_have_applied`).

use std::collections::BTreeMap;

use calendar_core::all_day::{AllDay, CalendarDate};
use calendar_core::failure::Failure;
use calendar_core::model::{CalendarDescriptor, CalendarEvent};
use calendar_core::shared_connector_helpers::decode_response;
use calendar_core::source_types::SourceError;
use calendar_core::time::{Zone, add_seconds, seconds_between};
use calendar_core::windows_time_zones::WindowsTimeZones;
use calendar_core::write::event_draft::EventTiming;
use calendar_core::write::event_patch::EventPatch;
use calendar_core::write::patch_merge::PatchMerge;
use calendar_core::write::types::{EventRef, NotifyPolicy, RecurrenceScope, WriteError};
use jiff::Timestamp;
use serde_json::Value;

use crate::api_client::{ApiResult, GraphApiClient, GraphApiError, GraphError};
use crate::dtos::GraphEventDTO;
use crate::event_mapper::{GraphEventMapper, Resolved};
use crate::recurrence_mapper::{GraphRecurrenceMapper, Json};
use crate::source::MicrosoftCalendarSource;
use crate::source_write::{RawEvent, invalid};
use crate::write_mapper::{GraphWriteMapper, RecurrenceAnchor};
use calendar_core::write::writable_source::WritableCalendarSource;

impl MicrosoftCalendarSource {
    async fn patch_recurrence(
        &self,
        calendar_id: &str,
        event_id: &str,
        recurrence: &Json,
    ) -> ApiResult<()> {
        let mut body = Json::new();
        body.insert("recurrence".to_string(), Value::Object(recurrence.clone()));
        let url = GraphApiClient::url(&GraphApiClient::event_path(calendar_id, event_id), &[])?;
        self.api
            .send(
                "PATCH",
                &url,
                Some(GraphWriteMapper::data(&body)?),
                &[],
                &BTreeMap::new(),
            )
            .await?;
        Ok(())
    }

    /// Puts the master's original recurrence back after a failed insert. If it fails too the result is
    /// `WriteError::Partial`, which names the insert's error.
    async fn restore_recurrence(
        &self,
        calendar_id: &str,
        master_id: &str,
        original: &Json,
        cause: &GraphError,
    ) -> ApiResult<()> {
        match self.patch_recurrence(calendar_id, master_id, original).await {
            Ok(()) => Ok(()),
            Err(error) => Err(WriteError::Partial(format!(
                "the series was cut off before this occurrence but the new series could not be created ({cause}), and restoring the original recurrence failed ({error})"
            ))
            .into()),
        }
    }

    /// Restores the master after a truncation whose reply was lost; a successful restore leaves the caller to rethrow
    /// the truncation's own error.
    async fn restore_truncation(
        &self,
        calendar_id: &str,
        master_id: &str,
        original: &Json,
        cause: &GraphError,
    ) -> ApiResult<()> {
        match self.patch_recurrence(calendar_id, master_id, original).await {
            Ok(()) => Ok(()),
            Err(error) => Err(WriteError::Partial(format!(
                "cutting the series off before this occurrence failed with an unknown outcome ({cause}), and restoring the original recurrence failed ({error}); check the calendar"
            ))
            .into()),
        }
    }

    /// Whether a failed request may nevertheless have been applied: the reply was lost (a 5xx, a broken connection, a
    /// cancellation) rather than the request refused. A 409 counts too: the insert carries a `transactionId` of ours
    /// alone, so a conflict can only mean an earlier attempt of the same POST went through.
    pub(crate) fn might_have_applied(error: &GraphError) -> bool {
        match error {
            GraphError::Api(api) => *api == GraphApiError::Conflict,
            GraphError::Other(Failure::Write(_)) => false,
            GraphError::Other(Failure::Source(source)) => {
                matches!(source, SourceError::Server { .. } | SourceError::Network(_))
            }
            GraphError::Other(Failure::Cancelled | Failure::Host(_)) => true,
        }
    }

    /// `ThisAndFollowing`: cut the master's recurrence just before the occurrence. Delete (`patch == None`) stops there
    /// and returns `None`. Update also inserts a new series from the occurrence with the patch applied. Splitting at
    /// the first occurrence is the same as `AllInSeries`. `NotifyPolicy` does not apply (Graph decides who is told).
    ///
    /// Failure handling for update: everything that can fail without changing anything happens before the
    /// truncation. The insert carries a `transactionId`, so if its reply is lost it is sent once more and the server
    /// returns the first result instead of a second event. If the insert fails for good, the master's original
    /// recurrence is restored, and if restoring fails too the result is `WriteError::Partial`. Once the insert has
    /// succeeded it is never undone.
    pub(crate) async fn split_series(
        &self,
        reference: &EventRef,
        calendar: &CalendarDescriptor,
        patch: Option<&EventPatch>,
        notify: NotifyPolicy,
    ) -> ApiResult<Option<CalendarEvent>> {
        let (Some(master_id), Some(split)) = (reference.series_id(), reference.original_start)
        else {
            return Err(invalid(
                "this and following needs the occurrence's original start",
            ));
        };
        let master = self.fetch_raw(&reference.calendar_id, master_id).await?;
        let zone = self.account_zone(false).await?;
        let master_dto = decode_response::<GraphEventDTO>(&master.data)?;
        let Some(first) = GraphEventMapper::resolve(&master_dto, &zone.zone) else {
            return Err(
                SourceError::InvalidResponse("microsoft: unreadable series".to_string()).into(),
            );
        };
        if seconds_between(split, first.start).abs() < 1.0 {
            if let Some(patch) = patch {
                return Ok(Some(
                    self.update(reference, patch, RecurrenceScope::AllInSeries, notify)
                        .await?,
                ));
            }
            self.delete(reference, RecurrenceScope::AllInSeries, notify)
                .await?;
            return Ok(None);
        }
        let Some(original) = master
            .json
            .get("recurrence")
            .and_then(Value::as_object)
            .filter(|recurrence| recurrence.contains_key("pattern"))
            .cloned()
        else {
            return Err(invalid("the series has no recurrence rule"));
        };
        let range_zone = original
            .get("range")
            .and_then(|range| range.get("recurrenceTimeZone"))
            .and_then(Value::as_str)
            .and_then(WindowsTimeZones::time_zone_for)
            .unwrap_or_else(|| zone.zone.clone());
        let split_date = AllDay::date_of(split, &range_zone);
        let truncated = GraphRecurrenceMapper::truncated(&original, &split_date);

        let mut new_series: Option<Json> = None;
        if let Some(patch) = patch {
            new_series = Some(
                self.new_series_body(
                    reference,
                    &master,
                    &first,
                    &original,
                    split,
                    &split_date,
                    &range_zone,
                    patch,
                    calendar,
                )
                .await?,
            );
        }
        // Serialize before truncating the master: a local encoding failure must leave the series intact.
        let insert_data = match &new_series {
            Some(body) => Some(GraphWriteMapper::data(body)?),
            None => None,
        };
        if let Err(error) = self
            .patch_recurrence(&reference.calendar_id, master_id, &truncated)
            .await
        {
            // A delete stops here and repeating the truncation is harmless. An update goes on to insert, so a
            // truncation whose reply was lost (it may have been applied) must not stay half done: put the original
            // back, so the caller may retry, and report the original error. Definite failures (a 400, a 403, ...)
            // changed nothing.
            if new_series.is_some() && Self::might_have_applied(&error) {
                self.restore_truncation(&reference.calendar_id, master_id, &original, &error)
                    .await?;
            }
            return Err(error);
        }
        let Some(insert_data) = insert_data else {
            return Ok(None);
        };
        let prefer = zone.read_preferences();
        let insert_url = GraphApiClient::url(
            &GraphApiClient::calendar_path(&reference.calendar_id, "/events"),
            &[],
        )?;
        let created = match self
            .api
            .send(
                "POST",
                &insert_url,
                Some(insert_data.clone()),
                &prefer,
                &BTreeMap::new(),
            )
            .await
        {
            Ok(data) => data,
            Err(error) => {
                if !Self::might_have_applied(&error) {
                    self.restore_recurrence(&reference.calendar_id, master_id, &original, &error)
                        .await?;
                    return Err(error);
                }
                // The reply may have been lost after the insert applied: send it once more. The same
                // `transactionId` makes the server answer with the event it already made; a second event is not
                // created.
                match self
                    .api
                    .send(
                        "POST",
                        &insert_url,
                        Some(insert_data),
                        &prefer,
                        &BTreeMap::new(),
                    )
                    .await
                {
                    Ok(data) => data,
                    Err(retry_error) => {
                        // Neither restoring (would duplicate the series if the insert applied) nor leaving it is
                        // safe to do blindly.
                        return Err(WriteError::Partial(format!(
                            "the series was cut off before this occurrence and the outcome of creating the new series is unknown ({error}; retry: {retry_error}); check the calendar"
                        ))
                        .into());
                    }
                }
            }
        };
        Ok(Some(self.mapped(&created, calendar)?))
    }

    /// The insert body for the series that continues after the split. Everything that can fail without changing
    /// anything (the occurrence, the count of earlier occurrences, validating the patch) is done here, before the
    /// master is touched.
    #[allow(clippy::too_many_arguments)]
    async fn new_series_body(
        &self,
        reference: &EventRef,
        master: &RawEvent,
        first: &Resolved,
        original: &Json,
        split: Timestamp,
        split_date: &CalendarDate,
        range_zone: &Zone,
        patch: &EventPatch,
        calendar: &CalendarDescriptor,
    ) -> ApiResult<Json> {
        let instance = self
            .fetch_raw(&reference.calendar_id, &reference.event_id)
            .await?;
        // The new series is built from the master and starts at the occurrence's original slot, so the occurrence's
        // own copy is read only to judge staleness like any other update.
        if let (Some(version), Some(key)) = (&reference.version, instance.change_key())
            && key != version
        {
            let overlapping = PatchMerge::conflicts(patch, &self.mapped(&instance.data, calendar)?);
            if !overlapping.is_empty() {
                return Err(WriteError::Conflict {
                    fields: overlapping,
                }
                .into());
            }
        }
        let numbered = original
            .get("range")
            .and_then(|range| range.get("type"))
            .and_then(Value::as_str)
            == Some("numbered");
        let remaining = if numbered {
            Some(GraphRecurrenceMapper::remaining_count(
                original, split_date,
            )?)
        } else {
            None
        };
        let mut body = GraphWriteMapper::new_series_base(&master.json);
        let slot = EventTiming::new(
            split,
            add_seconds(split, seconds_between(first.start, first.end)),
            Some(first.zone.clone()),
            first.is_all_day,
        );
        body.extend(GraphWriteMapper::time_fields(&slot)?);
        let start_date = match &patch.timing {
            Some(timing) => AllDay::date_of(
                timing.start,
                &timing
                    .time_zone
                    .clone()
                    .unwrap_or_else(|| range_zone.clone()),
            ),
            None => *split_date,
        };
        body.insert(
            "recurrence".to_string(),
            Value::Object(GraphRecurrenceMapper::restarted(
                original,
                &start_date,
                remaining,
            )),
        );
        // The patch overrides what the master carried (including times, recurrence and the attendee list, which starts
        // from the master's guests).
        let anchor = RecurrenceAnchor {
            start: start_date,
            zone: patch
                .timing
                .as_ref()
                .and_then(|timing| timing.time_zone.clone())
                .unwrap_or_else(|| range_zone.clone()),
        };
        body.extend(GraphWriteMapper::patch_body(
            patch,
            &master.attendees(),
            &anchor,
        )?);
        body.insert(
            "transactionId".to_string(),
            Value::String(uuid::Uuid::new_v4().to_string().to_lowercase()),
        );
        Ok(body)
    }
}
