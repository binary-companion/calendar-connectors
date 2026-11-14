//! Port of `GoogleCalendarSource+Write.swift`: create, update, delete, respond and the `.thisAndFollowing` split.
//!
//! Swift wraps every public method in `translated`, which turns the provider's `GoogleAPIError` into a `WriteError`.
//! Here the private methods return [`ApiResult`] and the trait methods call [`translate`] once at the end; the two
//! closures handed to `PatchMerge::apply` translate their own errors, because `apply` carries only `Failure`.
//! Swift runs the lookups and the restores of the split in unstructured tasks so they happen even for a cancelled
//! caller. A Rust caller cancels by dropping the future, after which nothing can run; the cancellation that can be
//! observed is the transport's `Failure::Cancelled`, which is handled like any other unclear failure.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use async_trait::async_trait;
use calendar_core::failure::{Failure, Outcome};
use calendar_core::model::{CalendarDescriptor, CalendarEvent, ResponseStatus, SeriesInfo};
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
use jiff::Timestamp;
use serde::Deserialize;
use serde_json::Value;

use crate::api_client::{
    ApiResult, GoogleApiClient, GoogleApiError, GoogleError, GoogleRequestMode, QueryItem,
    query_item,
};
use crate::dtos::{GoogleEventDTO, GoogleEventsPageDTO, GoogleTimeDTO};
use crate::event_mapper::{GoogleEventMapper, Resolved};
use crate::source::GoogleCalendarSource;
use crate::write_mapper::{Body, GoogleWriteMapper, Json};

impl From<WriteError> for GoogleError {
    fn from(error: WriteError) -> GoogleError {
        GoogleError::Other(Failure::Write(error))
    }
}

/// Translates provider outcomes into `WriteError`; `SourceError` and `WriteError` pass through unchanged.
pub(crate) fn translate(error: GoogleError) -> Failure {
    match error {
        GoogleError::Api(GoogleApiError::Gone | GoogleApiError::NotFound) => {
            Failure::Write(WriteError::NotFound)
        }
        GoogleError::Api(GoogleApiError::Forbidden) => Failure::Write(WriteError::Forbidden(None)),
        GoogleError::Api(GoogleApiError::BadRequest(message)) => {
            Failure::Write(WriteError::Invalid(message))
        }
        // Unreachable: only writes that carry `If-Match` can get a 412, and each of them handles it itself.
        GoogleError::Api(GoogleApiError::PreconditionFailed) => Failure::Source(
            SourceError::InvalidResponse("google: unexpected 412".to_string()),
        ),
        // Only the split's insert with a client-chosen id can get a 409, and it handles it itself.
        GoogleError::Api(GoogleApiError::Conflict) => Failure::Source(
            SourceError::InvalidResponse("google: unexpected 409".to_string()),
        ),
        GoogleError::Other(failure) => failure,
    }
}

fn invalid(message: &str) -> GoogleError {
    WriteError::Invalid(message.to_string()).into()
}

fn unreadable_event() -> GoogleError {
    SourceError::InvalidResponse("google: unreadable event".to_string()).into()
}

fn unsupported(field: EventField) -> GoogleError {
    WriteError::Unsupported {
        fields: BTreeSet::from([field]),
    }
    .into()
}

/// The full event resource, as `fetch_raw` returns it.
pub(crate) struct RawEvent {
    pub data: Vec<u8>,
    pub json: Json,
}

impl RawEvent {
    fn etag(&self) -> Option<&str> {
        self.json.get("etag").and_then(Value::as_str)
    }

    /// `json["attendees"] as? [[String: Any]]`, or empty.
    fn attendees(&self) -> Vec<Json> {
        self.json
            .get("attendees")
            .and_then(Value::as_array)
            .and_then(|list| list.iter().map(|a| a.as_object().cloned()).collect())
            .unwrap_or_default()
    }

    /// `json["recurrence"] as? [String]`, or empty.
    fn recurrence(&self) -> Vec<String> {
        self.json
            .get("recurrence")
            .and_then(Value::as_array)
            .and_then(|list| {
                list.iter()
                    .map(|line| line.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Swift's `Lookup`.
enum Lookup {
    Found(Vec<u8>),
    /// Definitely not there (404, or already deleted).
    Absent,
    Failed(GoogleError),
}

/// Swift's private `InstancePage`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstancePage {
    items: Option<Vec<InstanceItem>>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstanceItem {
    original_start_time: Option<GoogleTimeDTO>,
}

impl std::fmt::Display for GoogleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GoogleError::Api(error) => write!(f, "{error}"),
            GoogleError::Other(failure) => write!(f, "{failure}"),
        }
    }
}

impl GoogleCalendarSource {
    fn query(notify: NotifyPolicy, conference: bool, attachments: bool) -> Vec<QueryItem> {
        let mut items = vec![query_item(
            "sendUpdates",
            GoogleWriteMapper::send_updates(notify),
        )];
        if conference {
            items.push(query_item("conferenceDataVersion", "1"));
        }
        if attachments {
            items.push(query_item("supportsAttachments", "true"));
        }
        items
    }

    /// An empty id would address the events collection itself instead of one event.
    fn require_event_id(reference: &EventRef) -> ApiResult<()> {
        if reference.event_id.is_empty() {
            return Err(invalid("the event id must not be empty"));
        }
        Ok(())
    }

    /// A write result for a series master carries its own id as `series_id` (see `mapped_dto`), so a caller can tell it
    /// from an occurrence. A single-instance write on it would hit the whole series (Google addresses the master by
    /// that id), so it is refused; `AllInSeries`, and `ThisAndFollowing` at the master's start, work as usual.
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
            .calendars()
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

    fn mapped(&self, data: &[u8], calendar: &CalendarDescriptor) -> Outcome<CalendarEvent> {
        self.mapped_dto(&decode_response::<GoogleEventDTO>(data)?, calendar)
    }

    fn mapped_dto(
        &self,
        dto: &GoogleEventDTO,
        calendar: &CalendarDescriptor,
    ) -> Outcome<CalendarEvent> {
        if dto.status.as_deref() == Some("cancelled") {
            return Err(Failure::Write(WriteError::NotFound));
        }
        let Some(mut event) =
            GoogleEventMapper::map(dto, calendar, Some(&self.connection.source_id()))
        else {
            return Err(Failure::Source(SourceError::InvalidResponse(
                "google: unreadable event".to_string(),
            )));
        };
        // A resource with a `recurrence` is a series master (a create with a rule, a series-wide or split write). Reads
        // expand series and never return one. Mark it as its own series, starting at its own slot, so that a delete
        // with `ThisInstance` cannot be mistaken for a single event and remove the whole series.
        if dto
            .recurrence
            .as_ref()
            .is_some_and(|lines| !lines.is_empty())
        {
            event.series = Some(SeriesInfo::Occurrence {
                series_id: dto.id.clone(),
                original_start: Some(event.start),
            });
        }
        Ok(event)
    }

    /// The full event resource (no `fields` mask), so unmodeled fields and `attendees(self)` are present. A deleted
    /// event (status `cancelled`, still readable by id) is `NotFound`, so it is never patched back to life.
    pub(crate) async fn fetch_raw(&self, calendar_id: &str, event_id: &str) -> ApiResult<RawEvent> {
        let data = self
            .api
            .send(
                "GET",
                &GoogleApiClient::event_path(calendar_id, event_id),
                &[],
                None,
                &BTreeMap::new(),
                GoogleRequestMode::Write,
            )
            .await?;
        let Some(json) = serde_json::from_slice::<Value>(&data)
            .ok()
            .and_then(|value| value.as_object().cloned())
        else {
            return Err(unreadable_event());
        };
        if json.get("status").and_then(Value::as_str) == Some("cancelled") {
            return Err(WriteError::NotFound.into());
        }
        Ok(RawEvent { data, json })
    }

    /// The start of a raw event resource in the calendar's zone (a floating or all-day start reads in that zone). The
    /// one place series code reads a master's start.
    fn start_of(raw: &RawEvent, calendar: &CalendarDescriptor) -> ApiResult<Resolved> {
        let dto = decode_response::<GoogleEventDTO>(&raw.data)?;
        let zone = calendar.time_zone.clone().unwrap_or_else(Zone::utc);
        GoogleEventMapper::resolve(dto.start.as_ref(), &zone).ok_or_else(unreadable_event)
    }

    /// Fails with `Unsupported([Timing])` unless `reference` is the first occurrence of the series (see `update`).
    async fn require_first_occurrence(
        &self,
        reference: &EventRef,
        series_id: &str,
        calendar: &CalendarDescriptor,
    ) -> ApiResult<()> {
        let start = Self::start_of(
            &self.fetch_raw(&reference.calendar_id, series_id).await?,
            calendar,
        )?;
        match reference.original_start {
            Some(slot) if seconds_between(slot, start.date).abs() < 1.0 => Ok(()),
            _ => Err(unsupported(EventField::Timing)),
        }
    }

    /// The calendar's own copy of the meeting with this iCalendar UID, if any (a series master, else the first
    /// instance or exception). Unverified live: Google accepts a caller-supplied `iCalUID` on `events.insert`.
    async fn existing_event(
        &self,
        uid: &str,
        calendar: &CalendarDescriptor,
    ) -> ApiResult<Option<CalendarEvent>> {
        let mut found: Vec<GoogleEventDTO> = Vec::new();
        self.api
            .pages::<GoogleEventsPageDTO, _, _>(
                &GoogleApiClient::calendar_path(&calendar.id, "/events"),
                &[
                    query_item("iCalUID", uid),
                    query_item("showDeleted", "false"),
                    query_item("maxResults", "250"),
                    query_item(
                        "fields",
                        &Self::EVENT_FIELDS.replacen("items(id,", "items(id,recurrence,", 1),
                    ),
                ],
                |page| page.next_page_token.clone(),
                |page| {
                    found.extend(
                        page.items
                            .iter()
                            .flatten()
                            .filter_map(|item| item.value.clone())
                            .filter(|dto| dto.status.as_deref() != Some("cancelled")),
                    );
                    Ok(())
                },
            )
            .await?;
        let master = found
            .iter()
            .find(|dto| {
                dto.recurrence
                    .as_ref()
                    .is_some_and(|lines| !lines.is_empty())
            })
            .or_else(|| found.first());
        match master {
            Some(dto) => Ok(Some(self.mapped_dto(dto, calendar)?)),
            None => Ok(None),
        }
    }

    // MARK: Operations

    async fn create_event(
        &self,
        draft: &EventDraft,
        calendar_id: &str,
        notify: NotifyPolicy,
    ) -> ApiResult<CalendarEvent> {
        draft.validate()?;
        WriteValidation::require_writable(&draft.used_fields(), &self.capabilities())?;
        // Build (and so validate) the body before the first request.
        let body = GoogleWriteMapper::create_body(draft)?;
        let calendar = self.writable_calendar(calendar_id).await?;
        if let Some(uid) = draft.uid.as_ref().filter(|uid| !uid.is_empty())
            && let Some(existing) = self.existing_event(uid, &calendar).await?
        {
            return Err(WriteError::AlreadyExists(Box::new(existing)).into());
        }
        let data = self
            .api
            .send(
                "POST",
                &GoogleApiClient::calendar_path(calendar_id, "/events"),
                &Self::query(notify, body.needs_conference_version, false),
                Some(GoogleWriteMapper::data(&body.json)?),
                &BTreeMap::new(),
                GoogleRequestMode::Write,
            )
            .await?;
        Ok(self.mapped(&data, &calendar)?)
    }

    /// Callers read expanded instances, so a `timing` in `patch` is the instance's absolute date. Sent to the series
    /// master (`AllInSeries`) it would move the whole series, so a series-wide time change is accepted only when
    /// `reference` is the series' first occurrence (its `original_start` equals the master's start, within a second);
    /// otherwise it fails with `Unsupported([Timing])` before any PATCH. Other fields are unaffected.
    async fn update_event(
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
        // Validate the whole patch before the first request; the attendee array is filled in later from a fresh fetch.
        GoogleWriteMapper::patch_body(patch, Some(&[]))?;
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
                    "google: the split returned no event".to_string(),
                )
                .into()),
            };
        }
        let target = reference.write_target(scope);
        if scope == RecurrenceScope::AllInSeries
            && patch.timing.is_some()
            && target.id != reference.event_id
        {
            self.require_first_occurrence(reference, &target.id, &calendar)
                .await?;
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
                this.patch_once(reference, patch, notify, target_id, calendar_ref, expected)
                    .await
                    .map_err(translate)
            },
        )
        .await?;
        Ok(result)
    }

    /// One attempt of the PATCH `update` merges with `PatchMerge`.
    async fn patch_once(
        &self,
        reference: &EventRef,
        patch: &EventPatch,
        notify: NotifyPolicy,
        target_id: &str,
        calendar: &CalendarDescriptor,
        expected: Option<String>,
    ) -> ApiResult<Attempt<CalendarEvent>> {
        let mut if_match = expected.clone();
        let mut attendees: Option<Vec<Json>> = None;
        if patch
            .attendees
            .as_ref()
            .is_some_and(|changes| !changes.is_empty())
        {
            // Google replaces the whole array, so start from a fresh copy and judge staleness against the caller's version.
            let raw = self.fetch_raw(&reference.calendar_id, target_id).await?;
            if let (Some(expected), Some(etag)) = (&expected, raw.etag())
                && etag != expected
            {
                return Ok(Attempt::Stale);
            }
            attendees = Some(raw.attendees());
            if_match = raw.etag().map(str::to_string).or(expected);
        }
        let mut body = GoogleWriteMapper::patch_body(patch, attendees.as_deref())?;
        // Setting a series' rule replaces Google's whole `recurrence` array, which also holds the skipped (EXDATE) and
        // extra (RDATE) dates: keep every line that is not a rule, or cancelled occurrences return.
        if matches!(patch.recurrence, FieldUpdate::Set(_))
            && reference.series_id().is_some()
            && let Some(lines) = body
                .json
                .get("recurrence")
                .and_then(Value::as_array)
                .and_then(|list| {
                    list.iter()
                        .map(|line| line.as_str().map(str::to_string))
                        .collect::<Option<Vec<String>>>()
                })
        {
            let master = self.fetch_raw(&reference.calendar_id, target_id).await?;
            let mut lines = lines;
            lines.extend(
                master
                    .recurrence()
                    .into_iter()
                    .filter(|line| !GoogleWriteMapper::is_rrule(line)),
            );
            body.json.insert(
                "recurrence".to_string(),
                Value::Array(lines.into_iter().map(Value::String).collect()),
            );
        }
        let mut headers = BTreeMap::new();
        if let Some(if_match) = if_match {
            headers.insert("If-Match".to_string(), if_match);
        }
        match self
            .api
            .send(
                "PATCH",
                &GoogleApiClient::event_path(&reference.calendar_id, target_id),
                &Self::query(notify, body.needs_conference_version, false),
                Some(GoogleWriteMapper::data(&body.json)?),
                &headers,
                GoogleRequestMode::Write,
            )
            .await
        {
            Ok(data) => Ok(Attempt::Done(self.mapped(&data, calendar)?)),
            Err(GoogleError::Api(GoogleApiError::PreconditionFailed)) => Ok(Attempt::Stale),
            Err(error) => Err(error),
        }
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
        self.api
            .send(
                "DELETE",
                &GoogleApiClient::event_path(&reference.calendar_id, &target.id),
                &Self::query(notify, false, false),
                None,
                &BTreeMap::new(),
                GoogleRequestMode::Write,
            )
            .await?;
        Ok(())
    }

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
        // Refuse a response Google cannot store before any request (the attendee list here is only a placeholder).
        let placeholder: Json = [("self".to_string(), Value::Bool(true))]
            .into_iter()
            .collect();
        GoogleWriteMapper::respond_attendees(&[placeholder], response)?;
        let calendar = self.writable_calendar(&reference.calendar_id).await?;
        let target = reference.write_target(scope);
        // Only our own response changes, so when the event moves between the fetch and the write, start over on the new etag.
        for _ in 0..3 {
            let raw = self.fetch_raw(&reference.calendar_id, &target.id).await?;
            let attendees = GoogleWriteMapper::respond_attendees(&raw.attendees(), response)?;
            let mut headers = BTreeMap::new();
            if let Some(etag) = raw.etag() {
                headers.insert("If-Match".to_string(), etag.to_string());
            }
            let body: Json = [(
                "attendees".to_string(),
                Value::Array(attendees.into_iter().map(Value::Object).collect()),
            )]
            .into_iter()
            .collect();
            match self
                .api
                .send(
                    "PATCH",
                    &GoogleApiClient::event_path(&reference.calendar_id, &target.id),
                    &Self::query(notify, false, false),
                    Some(GoogleWriteMapper::data(&body)?),
                    &headers,
                    GoogleRequestMode::Write,
                )
                .await
            {
                Ok(data) => return Ok(self.mapped(&data, &calendar)?),
                Err(GoogleError::Api(GoogleApiError::PreconditionFailed)) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(WriteError::Conflict {
            fields: BTreeSet::from([EventField::Attendees]),
        }
        .into())
    }

    // MARK: Splitting a series

    /// Occurrences of the series that start before `split`, including individually deleted ones (they still count
    /// toward a `COUNT` rule).
    async fn prior_instances(
        &self,
        calendar_id: &str,
        master_id: &str,
        split: Timestamp,
        zone: &Zone,
    ) -> ApiResult<i64> {
        let mut count: i64 = 0;
        self.api
            .pages::<InstancePage, _, _>(
                &format!(
                    "{}/instances",
                    GoogleApiClient::event_path(calendar_id, master_id)
                ),
                &[
                    query_item("showDeleted", "true"),
                    query_item("maxResults", "250"),
                    query_item("fields", "nextPageToken,items(originalStartTime)"),
                ],
                |page| page.next_page_token.clone(),
                |page| {
                    for item in page.items.iter().flatten() {
                        if let Some(resolved) =
                            GoogleEventMapper::resolve(item.original_start_time.as_ref(), zone)
                            && resolved.date < split
                        {
                            count += 1;
                        }
                    }
                    Ok(())
                },
            )
            .await?;
        Ok(count)
    }

    async fn patch_recurrence(
        &self,
        calendar_id: &str,
        event_id: &str,
        lines: &[String],
        etag: Option<&str>,
        notify: NotifyPolicy,
    ) -> ApiResult<()> {
        let mut headers = BTreeMap::new();
        if let Some(etag) = etag {
            headers.insert("If-Match".to_string(), etag.to_string());
        }
        let body: Json = [(
            "recurrence".to_string(),
            Value::Array(lines.iter().cloned().map(Value::String).collect()),
        )]
        .into_iter()
        .collect();
        match self
            .api
            .send(
                "PATCH",
                &GoogleApiClient::event_path(calendar_id, event_id),
                &Self::query(notify, false, false),
                Some(GoogleWriteMapper::data(&body)?),
                &headers,
                GoogleRequestMode::Write,
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(GoogleError::Api(GoogleApiError::PreconditionFailed)) => {
                Err(WriteError::Conflict {
                    fields: BTreeSet::from([EventField::Recurrence]),
                }
                .into())
            }
            Err(error) => Err(error),
        }
    }

    /// Puts the master's original rule back after a failed insert. It is unconditional so it cannot lose to a
    /// concurrent edit of another field, and notifies with the caller's policy, like the truncation it reverses. If it
    /// fails too the result is `WriteError::Partial`, which names the insert's error.
    async fn restore_recurrence(
        &self,
        calendar_id: &str,
        master_id: &str,
        original: &[String],
        notify: NotifyPolicy,
        cause: &GoogleError,
    ) -> ApiResult<()> {
        match self
            .patch_recurrence(calendar_id, master_id, original, None, notify)
            .await
        {
            Ok(()) => Ok(()),
            Err(error) => Err(WriteError::Partial(format!(
                "the series was cut off before this occurrence but the new series could not be created ({cause}), and restoring the original rule failed ({error})"
            ))
            .into()),
        }
    }

    /// Restores the master after a truncation whose reply was lost; a successful restore leaves the caller to rethrow
    /// the truncation's own error. Unconditional like `restore_recurrence`; `Partial` when it fails too.
    async fn restore_truncation(
        &self,
        calendar_id: &str,
        master_id: &str,
        original: &[String],
        notify: NotifyPolicy,
        cause: &GoogleError,
    ) -> ApiResult<()> {
        match self
            .patch_recurrence(calendar_id, master_id, original, None, notify)
            .await
        {
            Ok(()) => Ok(()),
            Err(error) => Err(WriteError::Partial(format!(
                "cutting the series off before this occurrence failed with an unknown outcome ({cause}), and restoring the original rule failed ({error}); check the calendar"
            ))
            .into()),
        }
    }

    /// Whether a failed insert may nevertheless have been applied: the reply was lost (a 5xx, a broken connection, a
    /// cancellation) rather than the request refused. A 409 (`GoogleApiError::Conflict`) counts too: the id is ours
    /// alone, so it can only mean an earlier attempt of this same POST went through.
    fn might_have_applied(error: &GoogleError) -> bool {
        match error {
            GoogleError::Api(api) => *api == GoogleApiError::Conflict,
            GoogleError::Other(Failure::Write(_)) => false,
            GoogleError::Other(Failure::Source(source)) => {
                matches!(source, SourceError::Server { .. } | SourceError::Network(_))
            }
            GoogleError::Other(Failure::Cancelled | Failure::Host(_)) => true,
        }
    }

    /// Looks an event up by id. The answer decides whether the master is restored. Only a definite "not there" is
    /// `Absent`; a lookup that itself fails says nothing.
    async fn look_up(&self, calendar_id: &str, event_id: &str) -> Lookup {
        match self.fetch_raw(calendar_id, event_id).await {
            Ok(raw) => Lookup::Found(raw.data),
            Err(GoogleError::Api(GoogleApiError::NotFound))
            | Err(GoogleError::Other(Failure::Write(WriteError::NotFound))) => Lookup::Absent,
            Err(error) => Lookup::Failed(error),
        }
    }

    /// The insert body for the new series. Everything that can fail without changing anything (the occurrence, the
    /// count of earlier occurrences, validating the patch) is done here, before the master is touched.
    #[allow(clippy::too_many_arguments)]
    async fn new_series_body(
        &self,
        reference: &EventRef,
        master_id: &str,
        master: &RawEvent,
        original: &[String],
        split: Timestamp,
        master_zone: &Zone,
        patch: &EventPatch,
        calendar: &CalendarDescriptor,
    ) -> ApiResult<Body> {
        let instance = self
            .fetch_raw(&reference.calendar_id, &reference.event_id)
            .await?;
        // The new series is built from the master and starts at the occurrence's original slot, so the occurrence's own
        // copy is read only to judge staleness like any other update.
        if let (Some(version), Some(etag)) = (&reference.version, instance.etag())
            && etag != version
        {
            let overlapping = PatchMerge::conflicts(patch, &self.mapped(&instance.data, calendar)?);
            if !overlapping.is_empty() {
                return Err(WriteError::Conflict {
                    fields: overlapping,
                }
                .into());
            }
        }
        let mut lines = GoogleWriteMapper::rrule_lines(original);
        if patch.recurrence == FieldUpdate::Keep {
            if let Some(total) = GoogleWriteMapper::count_in(original) {
                let prior = self
                    .prior_instances(
                        &reference.calendar_id,
                        master_id,
                        split,
                        &calendar.time_zone.clone().unwrap_or_else(Zone::utc),
                    )
                    .await?;
                if total - prior < 1 {
                    return Err(invalid("the series has no occurrences left to split off"));
                }
                lines = GoogleWriteMapper::replacing_count(&lines, total - prior);
            }
            lines.extend(GoogleWriteMapper::carried_over(
                original,
                split,
                master_zone,
            ));
        }
        let zone_name = master
            .json
            .get("start")
            .and_then(|start| start.get("timeZone"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let fallback = zone_name
            .or_else(|| {
                calendar
                    .time_zone
                    .as_ref()
                    .map(|z| z.identifier().to_string())
            })
            .unwrap_or_else(|| "UTC".to_string());
        Ok(GoogleWriteMapper::new_series_body(
            &master.json,
            split,
            patch,
            lines,
            Some(&fallback),
        )?)
    }

    /// `ThisAndFollowing`: cut the master's rule just before the occurrence. Delete (`patch == None`) stops there and
    /// returns `None`. Update also inserts a new series from the occurrence with the patch applied. Splitting at the
    /// first occurrence is the same as `AllInSeries`. The truncation and the insert both carry the caller's policy (so
    /// on update guests may get two messages; external guests would otherwise keep the old series running), and so
    /// does a restore of the master's rule, so guests told about the cut hear about its reversal. Pass `None` for
    /// silence.
    ///
    /// Failure handling for update: everything that can fail without changing anything happens before the
    /// truncation. The insert carries an id we chose, so if its reply is lost we can look the event up instead of
    /// guessing. If the insert fails, the master's original rule is restored, and if restoring fails too the result is
    /// `WriteError::Partial`. Once the insert has succeeded it is never undone, even if its reply cannot be read.
    async fn split_series(
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
        let first = Self::start_of(&master, calendar)?;
        if seconds_between(split, first.date).abs() < 1.0 {
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
        let original = master.recurrence();
        if GoogleWriteMapper::rrule_lines(&original).is_empty() {
            return Err(invalid("the series has no recurrence rule"));
        }
        let master_zone = first
            .zone
            .clone()
            .or_else(|| calendar.time_zone.clone())
            .unwrap_or_else(Zone::utc);
        let truncated =
            GoogleWriteMapper::truncated(&original, split, first.is_all_day, Some(&master_zone));

        let mut new_series: Option<Body> = None;
        if let Some(patch) = patch {
            new_series = Some(
                self.new_series_body(
                    reference,
                    master_id,
                    &master,
                    &original,
                    split,
                    &master_zone,
                    patch,
                    calendar,
                )
                .await?,
            );
        }
        if let Err(error) = self
            .patch_recurrence(
                &reference.calendar_id,
                master_id,
                &truncated,
                master.etag(),
                notify,
            )
            .await
        {
            // A delete stops here and repeating the truncation is harmless. An update goes on to insert, so a
            // truncation whose reply was lost (it may have been applied) must not stay half done: it also moved the
            // master's EXDATE and RDATE values, which a retry would then lose. Put the original rule back, so the
            // caller may retry, and report the original error. Definite failures (a 412, a 400, a 403, ...) changed
            // nothing and are not restored.
            if new_series.is_some() && Self::might_have_applied(&error) {
                self.restore_truncation(
                    &reference.calendar_id,
                    master_id,
                    &original,
                    notify,
                    &error,
                )
                .await?;
            }
            return Err(error);
        }
        let Some(new_series) = new_series else {
            return Ok(None);
        };

        let mut json = new_series.json.clone();
        // base32hex: 0-9 a-v
        let new_id = uuid::Uuid::new_v4().simple().to_string().to_lowercase();
        json.insert("id".to_string(), Value::String(new_id.clone()));
        // Without this Google drops the attachments the master carries (the live smoke test must confirm both this and
        // the Meet re-request's `conferenceDataVersion`).
        let insert_query = Self::query(
            notify,
            new_series.needs_conference_version,
            json.contains_key("attachments"),
        );
        let created = match self
            .api
            .send(
                "POST",
                &GoogleApiClient::calendar_path(&reference.calendar_id, "/events"),
                &insert_query,
                Some(GoogleWriteMapper::data(&json)?),
                &BTreeMap::new(),
                GoogleRequestMode::Write,
            )
            .await
        {
            Ok(data) => data,
            Err(error) => {
                if Self::might_have_applied(&error) {
                    match self.look_up(&reference.calendar_id, &new_id).await {
                        Lookup::Found(data) => return Ok(Some(self.mapped(&data, calendar)?)),
                        Lookup::Failed(lookup_error) => {
                            // Neither restoring (would duplicate the series if the insert applied) nor leaving it is
                            // safe to do blindly.
                            return Err(WriteError::Partial(format!(
                                "the series was cut off before this occurrence and the outcome of creating the new series is unknown (insert: {error}; lookup: {lookup_error}); check the calendar"
                            ))
                            .into());
                        }
                        Lookup::Absent => {}
                    }
                }
                self.restore_recurrence(
                    &reference.calendar_id,
                    master_id,
                    &original,
                    notify,
                    &error,
                )
                .await?;
                return Err(error);
            }
        };
        match self.mapped(&created, calendar) {
            Ok(event) => Ok(Some(event)),
            Err(error) => {
                // The insert applied; never undo it. If the reply cannot be read, try the copy stored under our id.
                if let Lookup::Found(data) = self.look_up(&reference.calendar_id, &new_id).await
                    && let Ok(event) = self.mapped(&data, calendar)
                {
                    return Ok(Some(event));
                }
                Err(error.into())
            }
        }
    }
}

#[async_trait]
impl WritableCalendarSource for GoogleCalendarSource {
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
