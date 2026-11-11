//! Port of `GoogleCalendarSource.swift`: one Google account as a polling calendar source. Reads every visible
//! calendar (four at a time), detects changes with Google's sync tokens, and (in the write modules) writes events.
//! The Swift `extension GoogleCalendarSource` files are the `source_*` modules.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::task::Poll;

use async_trait::async_trait;
use calendar_core::change_monitor::ChangeMonitor;
use calendar_core::connection::{Connection, SyncStateStore};
use calendar_core::failure::Outcome;
use calendar_core::model::{CalendarDescriptor, CalendarEvent};
use calendar_core::shared_connector_helpers::detect_calendar_changes;
use calendar_core::source_types::{
    CalendarChange, CalendarSource, PollingCalendarSource, ProvidedField, SeriesSource,
    SourceCapabilities, SourceError, SyncKind,
};
use calendar_core::time::DateInterval;
use calendar_core::write::types::{EventField, RecurrenceScope};
use serde::Deserialize;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::api_client::{ApiResult, GoogleApiClient, GoogleApiError, GoogleError, query_item};
use crate::dtos::{GoogleEventsPageDTO, LenientItem};
use crate::event_mapper::GoogleEventMapper;

/// The last calendar list this source fetched. `events_in` reads it; `calendars` and each poll refresh it. Swift's
/// actor; a `std` mutex that no method holds across an `.await`.
#[derive(Default)]
pub(crate) struct CalendarListCache {
    list: Mutex<Option<Vec<CalendarDescriptor>>>,
}

impl CalendarListCache {
    pub(crate) fn list(&self) -> Option<Vec<CalendarDescriptor>> {
        self.list
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn store(&self, list: Vec<CalendarDescriptor>) {
        *self.list.lock().unwrap_or_else(PoisonError::into_inner) = Some(list);
    }
}

/// How many calendars `events_in` reads at the same time.
const CONCURRENT_READS: usize = 4;

/// Runs the futures `jobs` yields with at most `limit` in flight, and returns their results in completion order.
/// The first failure ends the run and drops what is still in flight; a job that has not started never starts. This is
/// Swift's task group with a sliding window (`withThrowingTaskGroup` plus `enqueueNext`).
async fn run_bounded<T, F>(jobs: impl IntoIterator<Item = F>, limit: usize) -> Outcome<Vec<T>>
where
    F: Future<Output = Outcome<T>>,
{
    let mut jobs = jobs.into_iter();
    let mut active: Vec<Pin<Box<F>>> = Vec::new();
    let mut done: Vec<T> = Vec::new();
    loop {
        while active.len() < limit {
            match jobs.next() {
                Some(job) => active.push(Box::pin(job)),
                None => break,
            }
        }
        if active.is_empty() {
            return Ok(done);
        }
        let (index, result) = std::future::poll_fn(|cx| {
            for (index, job) in active.iter_mut().enumerate() {
                if let Poll::Ready(result) = job.as_mut().poll(cx) {
                    return Poll::Ready((index, result));
                }
            }
            Poll::Pending
        })
        .await;
        drop(active.swap_remove(index));
        done.push(result?);
    }
}

/// One Google account. Built by [`crate::GoogleConnectorKind`]; the polling stream (`changes()`) keeps the source
/// alive while a receiver exists, as the Swift stream keeps the class alive.
pub struct GoogleCalendarSource {
    pub(crate) connection: Connection,
    pub(crate) api: GoogleApiClient,
    pub(crate) sync_state: Arc<dyn SyncStateStore>,
    monitor: ChangeMonitor,
    pub(crate) calendar_list: CalendarListCache,
    /// `changes()` hands the monitor an `Arc` of this very source (Swift passes `self`).
    me: Weak<GoogleCalendarSource>,
}

impl GoogleCalendarSource {
    pub(crate) const EVENT_FIELDS: &'static str = "nextPageToken,items(id,iCalUID,updated,created,status,summary,description,location,htmlLink,etag,hangoutLink,transparency,visibility,eventType,recurringEventId,start,end,originalStartTime,attendees(email,displayName,responseStatus,optional,resource,organizer,self),organizer(email,displayName,self),conferenceData(entryPoints(entryPointType,uri),conferenceSolution(key(type),name)),reminders(useDefault,overrides(method,minutes)))";

    pub(crate) const CALENDAR_SET_SCOPE: &'static str = "_calendars";

    pub(crate) fn new(
        connection: Connection,
        api: GoogleApiClient,
        sync_state: Arc<dyn SyncStateStore>,
        monitor: ChangeMonitor,
    ) -> Arc<GoogleCalendarSource> {
        Arc::new_cyclic(|me| GoogleCalendarSource {
            connection,
            api,
            sync_state,
            monitor,
            calendar_list: CalendarListCache::default(),
            me: me.clone(),
        })
    }

    async fn events_for(
        &self,
        calendar: &CalendarDescriptor,
        interval: &DateInterval,
    ) -> Outcome<Vec<CalendarEvent>> {
        let query = vec![
            query_item("singleEvents", "true"),
            query_item("timeMin", &GoogleEventMapper::instant_text(interval.start)),
            query_item("timeMax", &GoogleEventMapper::instant_text(interval.end)),
            query_item("maxResults", "250"),
            query_item("showDeleted", "false"),
            query_item("fields", Self::EVENT_FIELDS),
        ];
        let mut events: Vec<CalendarEvent> = Vec::new();
        let source_id = self.connection.source_id();
        let walked = self
            .api
            .pages::<GoogleEventsPageDTO, _, _>(
                &GoogleApiClient::calendar_path(&calendar.id, "/events"),
                &query,
                |page| page.next_page_token.clone(),
                |page| {
                    events.extend(
                        page.items
                            .iter()
                            .flatten()
                            .filter_map(|item| item.value.as_ref())
                            .filter_map(|dto| {
                                GoogleEventMapper::map(dto, calendar, Some(&source_id))
                            }),
                    );
                    Ok(())
                },
            )
            .await;
        match walked {
            Ok(()) => Ok(events),
            // A calendar that was removed or lost access is skipped; anything else is not ours to interpret.
            Err(GoogleError::Api(GoogleApiError::NotFound | GoogleApiError::Forbidden)) => {
                Ok(Vec::new())
            }
            Err(error) => Err(error.into_failure()),
        }
    }

    /// The outcome of one calendar's check: a calendar that was removed or is no longer readable counts as unchanged.
    fn skip_missing(result: ApiResult<bool>) -> Outcome<bool> {
        match result {
            Ok(changed) => Ok(changed),
            Err(GoogleError::Api(GoogleApiError::NotFound | GoogleApiError::Forbidden)) => {
                Ok(false)
            }
            Err(error) => Err(error.into_failure()),
        }
    }

    async fn check_calendar(&self, id: String) -> ApiResult<bool> {
        let connection_id = &self.connection.connection_id;
        if let Some(token) = self.sync_state.token(connection_id, &id).await {
            return self.poll(&id, &token).await;
        }
        self.bootstrap(&id).await?;
        Ok(false)
    }

    /// Lists the whole calendar (no time window; Google forbids combining a sync token with one) only to obtain a
    /// token.
    async fn bootstrap(&self, calendar_id: &str) -> ApiResult<()> {
        let mut token: Option<String> = None;
        self.api
            .pages::<SyncPageDTO, _, _>(
                &GoogleApiClient::calendar_path(calendar_id, "/events"),
                &[
                    query_item("showDeleted", "true"),
                    query_item("maxResults", "2500"),
                    query_item("fields", "nextPageToken,nextSyncToken"),
                ],
                |page| page.next_page_token.clone(),
                |page| {
                    if let Some(next) = &page.next_sync_token {
                        token = Some(next.clone());
                    }
                    Ok(())
                },
            )
            .await?;
        let Some(token) = token else {
            return Err(SourceError::InvalidResponse("no nextSyncToken".to_string()).into());
        };
        self.sync_state
            .set_token(Some(&token), &self.connection.connection_id, calendar_id)
            .await;
        Ok(())
    }

    /// True when anything changed since `token`. Always walks to the final page, because only it carries the new token.
    async fn poll(&self, calendar_id: &str, token: &str) -> ApiResult<bool> {
        let mut any_items = false;
        let mut new_token: Option<String> = None;
        let walked = self
            .api
            .pages::<SyncPageDTO, _, _>(
                &GoogleApiClient::calendar_path(calendar_id, "/events"),
                &[
                    query_item("syncToken", token),
                    query_item("showDeleted", "true"),
                    query_item("maxResults", "250"),
                    query_item("fields", "nextPageToken,nextSyncToken,items(id)"),
                ],
                |page| page.next_page_token.clone(),
                |page| {
                    if page.items.as_ref().is_some_and(|items| !items.is_empty()) {
                        any_items = true;
                    }
                    if let Some(next) = &page.next_sync_token {
                        new_token = Some(next.clone());
                    }
                    Ok(())
                },
            )
            .await;
        match walked {
            Ok(()) => {}
            Err(GoogleError::Api(GoogleApiError::Gone)) => {
                self.sync_state
                    .set_token(None, &self.connection.connection_id, calendar_id)
                    .await;
                self.bootstrap(calendar_id).await?;
                return Ok(true);
            }
            Err(error) => return Err(error),
        }
        let Some(new_token) = new_token else {
            return Err(SourceError::InvalidResponse("no nextSyncToken".to_string()).into());
        };
        self.sync_state
            .set_token(
                Some(&new_token),
                &self.connection.connection_id,
                calendar_id,
            )
            .await;
        Ok(any_items)
    }
}

/// Swift's private `SyncPageDTO`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncPageDTO {
    /// Items are only counted; an item that fails to parse is still a change.
    items: Option<Vec<LenientItem<SyncItem>>>,
    next_page_token: Option<String>,
    next_sync_token: Option<String>,
}

#[derive(Deserialize)]
struct SyncItem {
    #[serde(rename = "id")]
    _id: Option<String>,
}

#[async_trait]
impl CalendarSource for GoogleCalendarSource {
    fn id(&self) -> String {
        self.connection.source_id()
    }

    fn display_name(&self) -> String {
        self.connection.display_name.clone()
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities {
            can_write: true,
            can_edit_attendees: true,
            can_respond_to_invite: true,
            provided_fields: BTreeSet::from([
                ProvidedField::Kind,
                ProvidedField::Visibility,
                ProvidedField::Availability,
                ProvidedField::Reminders,
                ProvidedField::Series,
                ProvidedField::Participation,
                ProvidedField::StructuredConference,
                ProvidedField::Version,
                ProvidedField::LastModified,
                ProvidedField::Created,
                ProvidedField::UidScope,
                ProvidedField::RecurrenceRules,
                ProvidedField::IsDefault,
                ProvidedField::CalendarTimeZone,
                ProvidedField::DefaultReminders,
                ProvidedField::Provider,
                ProvidedField::SupportedAvailabilities,
                ProvidedField::PermissionDetails,
            ]),
            sync_kind: SyncKind::Token,
            supports_push: false,
            writable_fields: EventField::ALL.iter().copied().collect(),
            controls_notifications: true,
            recurrence_scopes: RecurrenceScope::ALL.iter().copied().collect(),
        }
    }

    /// Always asks Google (an explicit request) and remembers the answer for `events_in`.
    async fn calendars(&self) -> Outcome<Vec<CalendarDescriptor>> {
        let account = self.connection.config.get("email").map(String::as_str);
        let list: Vec<CalendarDescriptor> = self
            .api
            .calendar_list()
            .await?
            .iter()
            .filter_map(|entry| GoogleEventMapper::descriptor(entry, account))
            .collect();
        self.calendar_list.store(list.clone());
        Ok(list)
    }

    /// Uses the calendar list the last `calendars()` call or poll stored, so a refresh (which asks for the list and then
    /// the events) makes one `calendarList` request. A calendar removed since then answers 404 or 403 and is skipped.
    async fn events_in(&self, interval: DateInterval) -> Outcome<Vec<CalendarEvent>> {
        let calendars = match self.calendar_list.list() {
            Some(stored) => stored,
            None => self.calendars().await?,
        };
        let jobs: Vec<_> = calendars
            .iter()
            .map(|calendar: &CalendarDescriptor| self.events_for(calendar, &interval))
            .collect();
        let parts = run_bounded(jobs, CONCURRENT_READS).await?;
        let mut all: Vec<CalendarEvent> = parts.into_iter().flatten().collect();
        all.sort_by_cached_key(|event| (event.start, event.id()));
        Ok(all)
    }

    fn changes(&self) -> UnboundedReceiver<CalendarChange> {
        match self.me.upgrade() {
            Some(me) => self.monitor.changes(me),
            None => unbounded_channel().1,
        }
    }

    fn as_polling(&self) -> Option<&dyn PollingCalendarSource> {
        Some(self)
    }

    fn as_series(&self) -> Option<&dyn SeriesSource> {
        Some(self)
    }
}

#[async_trait]
impl PollingCalendarSource for GoogleCalendarSource {
    async fn check_for_changes(&self) -> Outcome<Option<CalendarChange>> {
        let calendar_ids: Vec<String> = self
            .calendars()
            .await?
            .into_iter()
            .map(|calendar| calendar.id)
            .collect();
        let connection_id = self.connection.connection_id.clone();
        detect_calendar_changes(
            &calendar_ids,
            Self::CALENDAR_SET_SCOPE,
            &connection_id,
            self.sync_state.as_ref(),
            |id| async move { Self::skip_missing(self.check_calendar(id).await) },
        )
        .await
    }
}
