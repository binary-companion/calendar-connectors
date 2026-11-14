//! Port of `MicrosoftCalendarSource+Write.swift`: create, update, delete and respond. The `.thisAndFollowing` split is
//! `source_split.rs`.
//!
//! Swift wraps every public method in `translated`, which turns the provider's `GraphAPIError` into a `WriteError`.
//! Here the private methods return [`ApiResult`] and the trait methods call [`translate`] once at the end; the two
//! closures handed to `PatchMerge::apply` translate their own errors, because `apply` carries only `Failure`.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;

use async_trait::async_trait;
use calendar_core::all_day::AllDay;
use calendar_core::failure::{Failure, Outcome};
use calendar_core::model::{CalendarDescriptor, CalendarEvent, ResponseStatus};
use calendar_core::shared_connector_helpers::decode_response;
use calendar_core::source_types::{CalendarSource, SourceError};
use calendar_core::time::{Zone, seconds_between};
use calendar_core::write::event_draft::EventDraft;
use calendar_core::write::event_patch::{EventPatch, FieldUpdate};
use calendar_core::write::patch_merge::{Attempt, PatchMerge};
use calendar_core::write::types::{
    EventField, EventRef, NotifyPolicy, RecurrenceScope, WriteError, WriteValidation,
};
use calendar_core::write::writable_source::WritableCalendarSource;
use serde_json::Value;

use crate::api_client::{ApiResult, GraphApiClient, GraphApiError, GraphError, query_item};
use crate::dtos::{GraphEventDTO, GraphListPage};
use crate::event_mapper::GraphEventMapper;
use crate::recurrence_mapper::{GraphRecurrenceMapper, Json};
use crate::source::MicrosoftCalendarSource;
use crate::write_mapper::{GraphWriteMapper, RecurrenceAnchor};

impl From<WriteError> for GraphError {
    fn from(error: WriteError) -> GraphError {
        GraphError::Other(Failure::Write(error))
    }
}

/// Swift interpolates an error into the `Partial` messages with its default description; a provider outcome prints as
/// the case name, anything else as its message.
impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphError::Api(error) => write!(f, "{error}"),
            GraphError::Other(failure) => write!(f, "{failure}"),
        }
    }
}

/// Translates provider outcomes into `WriteError`; `SourceError` and `WriteError` pass through unchanged.
pub(crate) fn translate(error: GraphError) -> Failure {
    match error {
        GraphError::Api(GraphApiError::Gone | GraphApiError::NotFound) => {
            Failure::Write(WriteError::NotFound)
        }
        GraphError::Api(GraphApiError::Forbidden) => Failure::Write(WriteError::Forbidden(None)),
        GraphError::Api(GraphApiError::BadRequest(message)) => {
            Failure::Write(WriteError::Invalid(message))
        }
        // No write sends If-Match, and a create carries no client-chosen id, so neither is expected.
        GraphError::Api(
            unexpected @ (GraphApiError::PreconditionFailed | GraphApiError::Conflict),
        ) => Failure::Source(SourceError::InvalidResponse(format!(
            "microsoft: unexpected {unexpected}"
        ))),
        GraphError::Other(failure) => failure,
    }
}

pub(crate) fn invalid(message: &str) -> GraphError {
    WriteError::Invalid(message.to_string()).into()
}

fn unreadable_event() -> GraphError {
    SourceError::InvalidResponse("microsoft: unreadable event".to_string()).into()
}

pub(crate) fn unsupported(field: EventField) -> GraphError {
    WriteError::Unsupported {
        fields: BTreeSet::from([field]),
    }
    .into()
}

/// Swift's `RawEvent`: the full event resource, as `fetch_raw` returns it.
pub(crate) struct RawEvent {
    pub data: Vec<u8>,
    pub json: Json,
}

impl RawEvent {
    pub(crate) fn change_key(&self) -> Option<&str> {
        self.json.get("changeKey").and_then(Value::as_str)
    }

    /// `json["attendees"] as? [[String: Any]]`, or empty.
    pub(crate) fn attendees(&self) -> Vec<Json> {
        self.json
            .get("attendees")
            .and_then(Value::as_array)
            .and_then(|list| list.iter().map(|a| a.as_object().cloned()).collect())
            .unwrap_or_default()
    }
}

impl MicrosoftCalendarSource {
    /// An empty id would address the events collection itself instead of one event.
    fn require_event_id(reference: &EventRef) -> ApiResult<()> {
        if reference.event_id.is_empty() {
            return Err(invalid("the event id must not be empty"));
        }
        Ok(())
    }

    /// A write result for a series master carries its own id as `series_id` (see `mapped_dto`), so a caller can tell it
    /// from an occurrence. A single-instance write on it would hit the whole series, so it is refused; `AllInSeries`,
    /// and `ThisAndFollowing` at the master's start, work as usual.
    fn require_occurrence(reference: &EventRef, scope: RecurrenceScope) -> ApiResult<()> {
        if scope == RecurrenceScope::ThisInstance
            && let Some(series) = reference.series_id()
            && series == reference.event_id
        {
            return Err(invalid(
                "this is a recurring series; use .allInSeries or read the occurrence first",
            ));
        }
        Ok(())
    }

    /// The calendar, provided it exists and the account can write to it.
    async fn writable_calendar(&self, calendar_id: &str) -> ApiResult<CalendarDescriptor> {
        let Some(calendar) = self
            .load_calendars(false)
            .await?
            .into_iter()
            .find(|calendar| calendar.id == calendar_id)
        else {
            return Err(WriteError::NotFound.into());
        };
        if !calendar.permissions.can_edit {
            return Err(WriteError::Forbidden(Some("read-only calendar".to_string())).into());
        }
        Ok(calendar)
    }

    pub(crate) fn mapped(
        &self,
        data: &[u8],
        calendar: &CalendarDescriptor,
    ) -> Outcome<CalendarEvent> {
        self.mapped_dto(&decode_response::<GraphEventDTO>(data)?, calendar)
    }

    fn mapped_dto(
        &self,
        dto: &GraphEventDTO,
        calendar: &CalendarDescriptor,
    ) -> Outcome<CalendarEvent> {
        if dto.is_cancelled == Some(true) {
            return Err(Failure::Write(WriteError::NotFound));
        }
        GraphEventMapper::map(
            dto,
            calendar,
            self.account_email().as_deref(),
            Some(&self.connection.source_id()),
        )
        .ok_or_else(|| {
            Failure::Source(SourceError::InvalidResponse(
                "microsoft: unreadable event".to_string(),
            ))
        })
    }

    /// The full event resource, so unmodeled fields and the attendee list are present. A cancelled event is
    /// `NotFound`, so it is never patched back to life.
    pub(crate) async fn fetch_raw(&self, calendar_id: &str, event_id: &str) -> ApiResult<RawEvent> {
        let zone = self.account_zone(false).await?;
        let url = GraphApiClient::url(&GraphApiClient::event_path(calendar_id, event_id), &[])?;
        let data = self.api.get(&url, &zone.read_preferences()).await?;
        // A body that is not a JSON object must not escape the library as anything but an invalid response.
        let Some(json) = serde_json::from_slice::<Value>(&data)
            .ok()
            .and_then(|value| value.as_object().cloned())
        else {
            return Err(unreadable_event());
        };
        if json.get("isCancelled") == Some(&Value::Bool(true)) {
            return Err(WriteError::NotFound.into());
        }
        Ok(RawEvent { data, json })
    }

    /// Where a recurrence set by `patch` starts: the patch's own timing, else the event as fetched.
    fn anchor(&self, raw: &RawEvent, patch: &EventPatch, fallback: &Zone) -> RecurrenceAnchor {
        if let Some(timing) = &patch.timing {
            let zone = timing.time_zone.clone().unwrap_or_else(|| fallback.clone());
            return RecurrenceAnchor {
                start: AllDay::date_of(timing.start, &zone),
                zone,
            };
        }
        let resolved = serde_json::from_slice::<GraphEventDTO>(&raw.data)
            .ok()
            .and_then(|dto| GraphEventMapper::resolve(&dto, fallback));
        match resolved {
            Some(times) => RecurrenceAnchor {
                start: AllDay::date_of(times.start, &times.zone),
                zone: times.zone,
            },
            // Swift reads `Date()` here; the injected clock is the same value in production.
            None => RecurrenceAnchor {
                start: AllDay::date_of((self.now)(), fallback),
                zone: fallback.clone(),
            },
        }
    }

    /// Fails with `Unsupported([Timing])` unless `reference` is the first occurrence of the series (see `update`).
    async fn require_first_occurrence(
        &self,
        reference: &EventRef,
        series_id: &str,
    ) -> ApiResult<()> {
        let raw = self.fetch_raw(&reference.calendar_id, series_id).await?;
        let zone = self.account_zone(false).await?;
        let first = serde_json::from_slice::<GraphEventDTO>(&raw.data)
            .ok()
            .and_then(|dto| GraphEventMapper::resolve(&dto, &zone.zone));
        match (first, reference.original_start) {
            (Some(first), Some(slot)) if seconds_between(slot, first.start).abs() < 1.0 => Ok(()),
            _ => Err(unsupported(EventField::Timing)),
        }
    }

    /// The calendar's own copy of the meeting with this iCalendar UID, if any (a series master, else the first
    /// instance or exception). Graph does not let a client set `iCalUId`, so an event this library creates gets its own
    /// UID; this finds copies that arrived with the uid (an invitation), which is the duplicate a caller wants to avoid.
    async fn existing_event(
        &self,
        uid: &str,
        calendar: &CalendarDescriptor,
    ) -> ApiResult<Option<CalendarEvent>> {
        let zone = self.account_zone(false).await?;
        let escaped = uid.replace('\'', "''");
        let url = GraphApiClient::url(
            &GraphApiClient::calendar_path(&calendar.id, "/events"),
            &[
                query_item("$filter", &format!("iCalUId eq '{escaped}'")),
                query_item("$top", "50"),
                query_item("$select", GraphEventMapper::EVENT_SELECT),
            ],
        )?;
        let mut found: Vec<GraphEventDTO> = Vec::new();
        self.api
            .pages::<GraphListPage<GraphEventDTO>, _>(&url, &zone.read_preferences(), |page| {
                found.extend(
                    page.items()
                        .into_iter()
                        .filter(|dto| dto.is_cancelled != Some(true)),
                );
                Ok(())
            })
            .await?;
        let chosen = found
            .iter()
            .find(|dto| dto.kind.as_deref() == Some("seriesMaster"))
            .or_else(|| found.first());
        match chosen {
            Some(dto) => Ok(Some(self.mapped_dto(dto, calendar)?)),
            None => Ok(None),
        }
    }

    // MARK: Operations

    async fn create_event(
        &self,
        draft: &EventDraft,
        calendar_id: &str,
        _notify: NotifyPolicy,
    ) -> ApiResult<CalendarEvent> {
        draft.validate()?;
        WriteValidation::require_writable(&draft.used_fields(), &self.capabilities())?;
        // Build (and so validate) the body before the first request.
        let body = GraphWriteMapper::create_body(draft)?;
        let calendar = self.writable_calendar(calendar_id).await?;
        if let Some(uid) = draft.uid.as_ref().filter(|uid| !uid.is_empty())
            && let Some(existing) = self.existing_event(uid, &calendar).await?
        {
            return Err(WriteError::AlreadyExists(Box::new(existing)).into());
        }
        let zone = self.account_zone(false).await?;
        let url = GraphApiClient::url(&GraphApiClient::calendar_path(calendar_id, "/events"), &[])?;
        let data = self
            .api
            .send(
                "POST",
                &url,
                Some(GraphWriteMapper::data(&body)?),
                &zone.read_preferences(),
                &BTreeMap::new(),
            )
            .await?;
        Ok(self.mapped(&data, &calendar)?)
    }

    /// Callers read expanded instances, so a `timing` in `patch` is the instance's absolute date. Sent to the series
    /// master (`AllInSeries`) it would move the whole series, so a series-wide time change is accepted only when
    /// `reference` is the series' first occurrence (its `original_start` equals the master's start, within a second);
    /// otherwise it fails with `Unsupported([Timing])` before any PATCH. Other fields are unaffected.
    pub(crate) async fn update_event(
        &self,
        reference: &EventRef,
        patch: &EventPatch,
        scope: RecurrenceScope,
        notify: NotifyPolicy,
    ) -> ApiResult<CalendarEvent> {
        WriteValidation::require_writable(&patch.touched_fields(), &self.capabilities())?;
        Self::require_event_id(reference)?;
        Self::require_occurrence(reference, scope)?;
        if scope == RecurrenceScope::ThisInstance
            && reference.series_id().is_some()
            && patch.recurrence != FieldUpdate::Keep
        {
            return Err(unsupported(EventField::Recurrence));
        }
        // Validate the whole patch before the first request; attendees and the anchor are filled in from a fresh fetch.
        GraphWriteMapper::patch_body(patch, &[], &RecurrenceAnchor::placeholder())?;
        let calendar = self.writable_calendar(&reference.calendar_id).await?;
        if patch.is_empty() {
            if let Some(base) = patch.base() {
                return Ok(base.clone());
            }
            let raw = self
                .fetch_raw(&reference.calendar_id, &reference.event_id)
                .await?;
            return Ok(self.mapped(&raw.data, &calendar)?);
        }
        if scope == RecurrenceScope::ThisAndFollowing && reference.series_id().is_some() {
            return match self
                .split_series(reference, &calendar, Some(patch), notify)
                .await?
            {
                Some(event) => Ok(event),
                None => Err(SourceError::InvalidResponse(
                    "microsoft: the split returned no event".to_string(),
                )
                .into()),
            };
        }
        let target = reference.write_target(scope);
        let target_is_master = reference.series_id() == Some(target.id.as_str());
        if scope == RecurrenceScope::AllInSeries
            && patch.timing.is_some()
            && target.id != reference.event_id
        {
            self.require_first_occurrence(reference, &target.id).await?;
        }
        let (this, calendar_ref, target_id) = (self, &calendar, target.id.as_str());
        let result = PatchMerge::apply(
            patch,
            if target.use_version {
                reference.version.clone()
            } else {
                None
            },
            PatchMerge::DEFAULT_MAX_ATTEMPTS,
            move || async move {
                let raw = this
                    .fetch_raw(&reference.calendar_id, target_id)
                    .await
                    .map_err(translate)?;
                this.mapped(&raw.data, calendar_ref)
            },
            move |expected: Option<String>| async move {
                this.patch_once(
                    reference,
                    patch,
                    target_id,
                    target_is_master,
                    calendar_ref,
                    expected,
                )
                .await
                .map_err(translate)
            },
        )
        .await?;
        Ok(result)
    }

    /// One attempt of the PATCH `update_event` merges with `PatchMerge`.
    async fn patch_once(
        &self,
        reference: &EventRef,
        patch: &EventPatch,
        target_id: &str,
        target_is_master: bool,
        calendar: &CalendarDescriptor,
        expected: Option<String>,
    ) -> ApiResult<Attempt<CalendarEvent>> {
        // Graph has no If-Match for events, so the version check is a read just before the write; a change made in
        // between is not caught (the window is one round trip).
        let raw = self.fetch_raw(&reference.calendar_id, target_id).await?;
        if let (Some(expected), Some(key)) = (&expected, raw.change_key())
            && key != expected
        {
            return Ok(Attempt::Stale);
        }
        let zone = self.account_zone(false).await?;
        let mut body = GraphWriteMapper::patch_body(
            patch,
            &raw.attendees(),
            &self.anchor(&raw, patch, &zone.zone),
        )?;
        // Moving a series' first occurrence moves the day its range starts on, so the range moves with it.
        if target_is_master
            && let Some(timing) = &patch.timing
            && !body.contains_key("recurrence")
            && let Some(recurrence) = raw.json.get("recurrence").and_then(Value::as_object)
        {
            let timing_zone = timing
                .time_zone
                .clone()
                .unwrap_or_else(|| zone.zone.clone());
            body.insert(
                "recurrence".to_string(),
                Value::Object(GraphRecurrenceMapper::restarted(
                    recurrence,
                    &AllDay::date_of(timing.start, &timing_zone),
                    None,
                )),
            );
        }
        let url = GraphApiClient::url(
            &GraphApiClient::event_path(&reference.calendar_id, target_id),
            &[],
        )?;
        let data = self
            .api
            .send(
                "PATCH",
                &url,
                Some(GraphWriteMapper::data(&body)?),
                &zone.read_preferences(),
                &BTreeMap::new(),
            )
            .await?;
        Ok(Attempt::Done(self.mapped(&data, calendar)?))
    }

    async fn delete_event(
        &self,
        reference: &EventRef,
        scope: RecurrenceScope,
        notify: NotifyPolicy,
    ) -> ApiResult<()> {
        Self::require_event_id(reference)?;
        Self::require_occurrence(reference, scope)?;
        let calendar = self.writable_calendar(&reference.calendar_id).await?;
        if scope == RecurrenceScope::ThisAndFollowing && reference.series_id().is_some() {
            self.split_series(reference, &calendar, None, notify)
                .await?;
            return Ok(());
        }
        let target = reference.write_target(scope);
        let url = GraphApiClient::url(
            &GraphApiClient::event_path(&reference.calendar_id, &target.id),
            &[],
        )?;
        self.api
            .send("DELETE", &url, None, &[], &BTreeMap::new())
            .await?;
        Ok(())
    }

    /// `notify == None` sends the response without telling the organizer (`sendResponse: false`); any other policy
    /// sends it. This is the one place Graph lets a caller choose.
    async fn respond_event(
        &self,
        reference: &EventRef,
        response: ResponseStatus,
        scope: RecurrenceScope,
        notify: NotifyPolicy,
    ) -> ApiResult<CalendarEvent> {
        // Splitting a series only to change one guest's response is not offered.
        if scope == RecurrenceScope::ThisAndFollowing && reference.series_id().is_some() {
            return Err(unsupported(EventField::Attendees));
        }
        Self::require_event_id(reference)?;
        Self::require_occurrence(reference, scope)?;
        let action = GraphWriteMapper::respond_action(response)?;
        let calendar = self.writable_calendar(&reference.calendar_id).await?;
        let target = reference.write_target(scope);
        let url = GraphApiClient::url(
            &format!(
                "{}/{action}",
                GraphApiClient::event_path(&reference.calendar_id, &target.id)
            ),
            &[],
        )?;
        let mut body = Json::new();
        body.insert(
            "sendResponse".to_string(),
            Value::Bool(notify != NotifyPolicy::None),
        );
        self.api
            .send(
                "POST",
                &url,
                Some(GraphWriteMapper::data(&body)?),
                &[],
                &BTreeMap::new(),
            )
            .await?;
        let raw = self.fetch_raw(&reference.calendar_id, &target.id).await?;
        Ok(self.mapped(&raw.data, &calendar)?)
    }
}

#[async_trait]
impl WritableCalendarSource for MicrosoftCalendarSource {
    async fn create(
        &self,
        draft: &EventDraft,
        calendar_id: &str,
        notify: NotifyPolicy,
    ) -> Outcome<CalendarEvent> {
        self.create_event(draft, calendar_id, notify)
            .await
            .map_err(translate)
    }

    async fn update(
        &self,
        reference: &EventRef,
        patch: &EventPatch,
        scope: RecurrenceScope,
        notify: NotifyPolicy,
    ) -> Outcome<CalendarEvent> {
        self.update_event(reference, patch, scope, notify)
            .await
            .map_err(translate)
    }

    async fn delete(
        &self,
        reference: &EventRef,
        scope: RecurrenceScope,
        notify: NotifyPolicy,
    ) -> Outcome<()> {
        self.delete_event(reference, scope, notify)
            .await
            .map_err(translate)
    }

    async fn respond(
        &self,
        reference: &EventRef,
        response: ResponseStatus,
        scope: RecurrenceScope,
        notify: NotifyPolicy,
    ) -> Outcome<CalendarEvent> {
        self.respond_event(reference, response, scope, notify)
            .await
            .map_err(translate)
    }
}
