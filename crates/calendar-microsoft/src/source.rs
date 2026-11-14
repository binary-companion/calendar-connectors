//! Port of `MicrosoftCalendarSource.swift`: one Microsoft account as a polling calendar source. Reads every calendar
//! (four at a time) in the mailbox's time zone, detects changes with Graph's delta links, and (in the write modules)
//! writes events. The Swift `extension MicrosoftCalendarSource` files are the `source_*` modules.

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
use calendar_core::shared_connector_helpers::decode_response;
use calendar_core::source_types::{
    CalendarChange, CalendarSource, PollingCalendarSource, ProvidedField, SeriesSource,
    SourceCapabilities, SyncKind,
};
use calendar_core::time::{DateInterval, Now, Zone};
use calendar_core::windows_time_zones::WindowsTimeZones;
use calendar_core::write::types::{EventField, RecurrenceScope};
use calendar_core::write::writable_source::WritableCalendarSource;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::api_client::{GraphApiClient, GraphApiError, GraphError, query_item};
use crate::dtos::{GraphCalendarDTO, GraphEventDTO, GraphListPage, GraphValueDTO};
use crate::event_mapper::GraphEventMapper;
use crate::graph_time::GraphTime;

/// The account's default time zone, and how a request asks Graph to express times in it.
#[derive(Clone, Debug)]
pub(crate) struct AccountZone {
    /// The name as Graph gave it (a Windows name or an IANA id), sent back in `Prefer: outlook.timezone`.
    pub name: String,
    pub zone: Zone,
}

impl AccountZone {
    pub(crate) fn utc() -> AccountZone {
        AccountZone {
            name: "UTC".to_string(),
            zone: Zone::utc(),
        }
    }

    /// Times come back in the account zone (so an all-day event falls on the date the user sees) and bodies as text.
    pub(crate) fn read_preferences(&self) -> Vec<String> {
        vec![
            format!("outlook.timezone=\"{}\"", self.name),
            "outlook.body-content-type=\"text\"".to_string(),
        ]
    }
}

/// What a source has learned from Graph and reuses: the calendar list and the account zone. Swift's actor
/// `MicrosoftSourceState`; a `std` mutex that no method holds across an `.await`.
#[derive(Default)]
pub(crate) struct MicrosoftSourceState {
    inner: Mutex<StateInner>,
}

#[derive(Default)]
struct StateInner {
    calendars: Option<Vec<CalendarDescriptor>>,
    zone: Option<AccountZone>,
}

impl MicrosoftSourceState {
    fn lock(&self) -> std::sync::MutexGuard<'_, StateInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn calendars(&self) -> Option<Vec<CalendarDescriptor>> {
        self.lock().calendars.clone()
    }

    pub(crate) fn zone(&self) -> Option<AccountZone> {
        self.lock().zone.clone()
    }

    pub(crate) fn store_calendars(&self, calendars: Vec<CalendarDescriptor>) {
        self.lock().calendars = Some(calendars);
    }

    pub(crate) fn store_zone(&self, zone: AccountZone) {
        self.lock().zone = Some(zone);
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

/// One Microsoft account. Built by [`crate::MicrosoftConnectorKind`] or [`crate::MicrosoftBrokeredConnectorKind`];
/// the polling stream (`changes()`) keeps the source alive while a receiver exists, as the Swift stream keeps the
/// class alive.
pub struct MicrosoftCalendarSource {
    pub(crate) connection: Connection,
    pub(crate) api: GraphApiClient,
    pub(crate) sync_state: Arc<dyn SyncStateStore>,
    monitor: ChangeMonitor,
    pub(crate) now: Now,
    pub(crate) state: MicrosoftSourceState,
    /// `changes()` hands the monitor an `Arc` of this very source (Swift passes `self`).
    me: Weak<MicrosoftCalendarSource>,
}

impl MicrosoftCalendarSource {
    pub(crate) fn new(
        connection: Connection,
        api: GraphApiClient,
        sync_state: Arc<dyn SyncStateStore>,
        monitor: ChangeMonitor,
        now: Now,
    ) -> Arc<MicrosoftCalendarSource> {
        Arc::new_cyclic(|me| MicrosoftCalendarSource {
            connection,
            api,
            sync_state,
            monitor,
            now,
            state: MicrosoftSourceState::default(),
            me: me.clone(),
        })
    }

    /// The account's email address (lowercased), as the connection stored it.
    pub(crate) fn account_email(&self) -> Option<String> {
        self.connection
            .config
            .get("email")
            .map(|email| email.to_lowercase())
    }

    // MARK: Account zone and calendars

    /// The mailbox time zone (`MailboxSettings.Read`). A definite "no" (the scope was refused, the zone is unknown to
    /// the table) is UTC and remembered; a failure that may pass (network, server, throttling) is rethrown so the
    /// events are not read in the wrong zone.
    pub(crate) async fn account_zone(&self, refresh: bool) -> Outcome<AccountZone> {
        if !refresh && let Some(known) = self.state.zone() {
            return Ok(known);
        }
        let url = GraphApiClient::url("/me/mailboxSettings/timeZone", &[])?;
        let zone = match self.api.get(&url, &[]).await {
            Ok(data) => {
                let value = decode_response::<GraphValueDTO>(&data)?.value;
                match value {
                    Some(name) => match WindowsTimeZones::time_zone_for(&name) {
                        Some(resolved) => AccountZone {
                            name,
                            zone: resolved,
                        },
                        None => AccountZone::utc(),
                    },
                    None => AccountZone::utc(),
                }
            }
            Err(GraphError::Api(_)) => AccountZone::utc(),
            Err(GraphError::Other(failure)) => return Err(failure),
        };
        self.state.store_zone(zone.clone());
        Ok(zone)
    }

    pub(crate) async fn load_calendars(
        &self,
        refresh_zone: bool,
    ) -> Outcome<Vec<CalendarDescriptor>> {
        let zone = self.account_zone(refresh_zone).await?;
        let mut entries: Vec<GraphCalendarDTO> = Vec::new();
        let first = GraphApiClient::url("/me/calendars", &[query_item("$top", "100")])?;
        self.api
            .pages::<GraphListPage<GraphCalendarDTO>, _>(&first, &[], |page| {
                entries.extend(page.items());
                Ok(())
            })
            .await
            .map_err(GraphError::into_failure)?;
        let account = self.account_email();
        let list: Vec<CalendarDescriptor> = entries
            .iter()
            .map(|entry| GraphEventMapper::descriptor(entry, account.as_deref(), &zone.zone))
            .collect();
        self.state.store_calendars(list.clone());
        Ok(list)
    }

    // MARK: Events

    async fn events_for(
        &self,
        calendar: &CalendarDescriptor,
        interval: &DateInterval,
        zone: &AccountZone,
    ) -> Outcome<Vec<CalendarEvent>> {
        let url = GraphApiClient::url(
            &GraphApiClient::calendar_path(&calendar.id, "/calendarView"),
            &[
                query_item("startDateTime", &GraphTime::instant_text(interval.start)),
                query_item("endDateTime", &GraphTime::instant_text(interval.end)),
                query_item("$top", "100"),
                query_item("$select", GraphEventMapper::EVENT_SELECT),
            ],
        )?;
        let mut events: Vec<CalendarEvent> = Vec::new();
        let account = self.account_email();
        let source_id = self.connection.source_id();
        let walked = self
            .api
            .pages::<GraphListPage<GraphEventDTO>, _>(&url, &zone.read_preferences(), |page| {
                events.extend(page.items().iter().filter_map(|dto| {
                    GraphEventMapper::map(dto, calendar, account.as_deref(), Some(&source_id))
                }));
                Ok(())
            })
            .await;
        match walked {
            Ok(()) => Ok(events),
            // A calendar that was removed or lost access is skipped; anything else is not ours to interpret.
            Err(GraphError::Api(GraphApiError::NotFound | GraphApiError::Forbidden)) => {
                Ok(Vec::new())
            }
            Err(error) => Err(error.into_failure()),
        }
    }
}

#[async_trait]
impl CalendarSource for MicrosoftCalendarSource {
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
                ProvidedField::Provider,
                ProvidedField::SupportedAvailabilities,
                ProvidedField::PermissionDetails,
            ]),
            sync_kind: SyncKind::Token,
            supports_push: false,
            writable_fields: EventField::ALL.iter().copied().collect(),
            controls_notifications: false,
            recurrence_scopes: RecurrenceScope::ALL.iter().copied().collect(),
        }
    }

    /// Always asks Graph (an explicit request) and remembers the answer for `events_in`.
    async fn calendars(&self) -> Outcome<Vec<CalendarDescriptor>> {
        self.load_calendars(true).await
    }

    /// Uses the calendar list the last `calendars()` call or poll stored, so a refresh makes one list request. A
    /// calendar removed since then answers 404 or 403 and is skipped.
    async fn events_in(&self, interval: DateInterval) -> Outcome<Vec<CalendarEvent>> {
        let calendars = match self.state.calendars() {
            Some(stored) => stored,
            None => self.load_calendars(true).await?,
        };
        let zone = self.account_zone(false).await?;
        let jobs: Vec<_> = calendars
            .iter()
            .map(|calendar: &CalendarDescriptor| self.events_for(calendar, &interval, &zone))
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

    fn as_writable(&self) -> Option<&dyn WritableCalendarSource> {
        Some(self)
    }
}
