//! Port of `CalDAVCalendarSource.swift`: one CalDAV account (iCloud or another server). Reads through
//! `calendar-query`, polls with `sync-collection` (or the ctag), writes whole resources with ETags, and expands
//! recurring events itself. The Swift `extension CalDAVCalendarSource` files are the `source_*` modules.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use async_trait::async_trait;
use calendar_core::change_monitor::ChangeMonitor;
use calendar_core::connection::{Connection, SyncStateStore};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::model::{
    Availability, CalendarDescriptor, CalendarEvent, CalendarKind, CalendarPermissions,
    CalendarProvider, CalendarService,
};
use calendar_core::source_types::{
    CalendarChange, CalendarSource, PollingCalendarSource, ProvidedField, SeriesSource,
    SourceCapabilities, SourceError, SyncKind,
};
use calendar_core::time::{DateInterval, Now, Zone};
use calendar_core::url_string::UrlString;
use calendar_core::write::types::{EventField, RecurrenceScope};
use calendar_core::write::writable_source::WritableCalendarSource;
use calendar_ical::{EventReadContext, EventReader, EventResource, ICalParser, TimeZoneResolver};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::account_config::CalDavAccountConfig;
use crate::dav_url::DavUrl;
use crate::dav_xml::{Dav, DavProperty, DavResponse, DavXml};
use crate::webdav_client::WebDavClient;

#[derive(Clone, Debug)]
pub(crate) struct CalDavCalendarInfo {
    pub descriptor: CalendarDescriptor,
    pub url: UrlString,
    pub ctag: Option<String>,
    pub sync_token: Option<String>,
    /// Floating times and all-day dates on this calendar are read in this zone.
    pub zone: Zone,
}

#[derive(Default)]
struct StateData {
    calendars: Option<Vec<CalDavCalendarInfo>>,
    parsed: BTreeMap<UrlString, (String, EventResource)>,
}

/// What a source has learned and reuses: the calendar list and parsed resources (by URL, while the ETag matches).
/// Swift's actor; a `std` mutex that no method holds across an `.await`.
pub(crate) struct CalDavSourceState {
    data: Mutex<StateData>,
}

impl CalDavSourceState {
    fn new() -> CalDavSourceState {
        CalDavSourceState {
            data: Mutex::new(StateData::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, StateData> {
        self.data.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn calendars(&self) -> Option<Vec<CalDavCalendarInfo>> {
        self.lock().calendars.clone()
    }

    pub(crate) fn store(&self, calendars: Vec<CalDavCalendarInfo>) {
        self.lock().calendars = Some(calendars);
    }

    pub(crate) fn resource(&self, url: &UrlString, etag: &str) -> Option<EventResource> {
        let data = self.lock();
        let (stored, resource) = data.parsed.get(url)?;
        (stored == etag).then(|| resource.clone())
    }

    pub(crate) fn remember(&self, resource: &EventResource, url: &UrlString, etag: &str) {
        self.lock()
            .parsed
            .insert(url.clone(), (etag.to_string(), resource.clone()));
    }

    pub(crate) fn forget(&self, url: &UrlString) {
        self.lock().parsed.remove(url);
    }
}

pub(crate) type UuidMaker = Arc<dyn Fn() -> String + Send + Sync>;

/// Everything Swift's initializer takes. The default zone has no default here (a library that reads no environment has
/// no "current" zone); `make_uuid` defaults to a random uppercase UUID, as Swift's `UUID().uuidString`.
pub(crate) struct CalDavSourceSettings {
    pub connection: Connection,
    pub account: CalDavAccountConfig,
    pub client: WebDavClient,
    pub provider: CalendarProvider,
    pub sync_state: Arc<dyn SyncStateStore>,
    pub monitor: ChangeMonitor,
    pub now: Now,
    pub default_zone: Zone,
    pub make_uuid: UuidMaker,
}

impl CalDavSourceSettings {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        connection: Connection,
        account: CalDavAccountConfig,
        client: WebDavClient,
        provider: CalendarProvider,
        sync_state: Arc<dyn SyncStateStore>,
        monitor: ChangeMonitor,
        now: Now,
        default_zone: Zone,
    ) -> CalDavSourceSettings {
        CalDavSourceSettings {
            connection,
            account,
            client,
            provider,
            sync_state,
            monitor,
            now,
            default_zone,
            make_uuid: Arc::new(|| uuid::Uuid::new_v4().to_string().to_uppercase()),
        }
    }
}

/// One CalDAV account (iCloud or another server): reads through `calendar-query`, polls with `sync-collection` (or the
/// ctag), writes whole resources with ETags, and expands recurring events itself. Built by the connector kinds; the
/// polling stream (`changes()`) keeps the source alive while a receiver exists, as the Swift stream keeps the class
/// alive.
pub struct CalDavCalendarSource {
    pub(crate) connection: Connection,
    pub(crate) account: CalDavAccountConfig,
    pub(crate) client: WebDavClient,
    pub(crate) provider: CalendarProvider,
    pub(crate) sync_state: Arc<dyn SyncStateStore>,
    monitor: ChangeMonitor,
    pub(crate) now: Now,
    pub(crate) default_zone: Zone,
    pub(crate) make_uuid: UuidMaker,
    pub(crate) state: CalDavSourceState,
    /// `changes()` hands the monitor an `Arc` of this very source (Swift passes `self`).
    me: Weak<CalDavCalendarSource>,
}

impl CalDavCalendarSource {
    pub(crate) fn new(settings: CalDavSourceSettings) -> Arc<CalDavCalendarSource> {
        Arc::new_cyclic(|me| CalDavCalendarSource {
            connection: settings.connection,
            account: settings.account,
            client: settings.client,
            provider: settings.provider,
            sync_state: settings.sync_state,
            monitor: settings.monitor,
            now: settings.now,
            default_zone: settings.default_zone,
            make_uuid: settings.make_uuid,
            state: CalDavSourceState::new(),
            me: me.clone(),
        })
    }

    pub(crate) fn source_id(&self) -> String {
        self.connection.source_id()
    }

    /// The addresses that mean "me" on an event: the principal's address set and the principal URL (some servers name
    /// the organizer by it). Empty when the server named no addresses, so nothing is ever marked as self.
    pub(crate) fn self_addresses(&self) -> BTreeSet<String> {
        if self.account.user_addresses.is_empty() {
            return BTreeSet::new();
        }
        let mut all: BTreeSet<String> = self.account.user_addresses.iter().cloned().collect();
        all.insert(self.account.principal_url.as_str().to_lowercase());
        all
    }

    /// The `mailto:` address the account organizes meetings as: the one matching the user name, else the first.
    pub(crate) fn organizer_address(&self) -> Option<String> {
        let mail: Vec<&String> = self
            .account
            .user_addresses
            .iter()
            .filter(|a| a.starts_with("mailto:"))
            .collect();
        let wanted = format!("mailto:{}", self.account.username.to_lowercase());
        mail.iter()
            .find(|a| ***a == wanted)
            .or(mail.first())
            .map(|a| (*a).clone())
    }

    // MARK: URLs and ids

    pub(crate) fn calendar_url(&self, calendar_id: &str) -> UrlString {
        DavUrl::appending_path_component(&self.account.home_url, calendar_id, true)
    }

    pub(crate) fn resource_url(&self, calendar_id: &str, name: &str) -> UrlString {
        DavUrl::appending_path_component(&self.calendar_url(calendar_id), name, false)
    }

    /// The collection path below the home, without its trailing slash; `None` for the home itself or anything outside it.
    pub(crate) fn calendar_id_for(&self, url: &UrlString) -> Option<String> {
        let mut home = DavUrl::path(&self.account.home_url);
        if !home.ends_with('/') {
            home.push('/');
        }
        let path = DavUrl::path(url);
        let rest = path.strip_prefix(&home)?;
        let id = rest.trim_matches('/');
        (!id.is_empty()).then(|| id.to_string())
    }

    // MARK: Calendars

    pub(crate) async fn load_calendars(&self) -> Outcome<Vec<CalDavCalendarInfo>> {
        let (status, answered_by) = self
            .client
            .propfind(
                &self.account.home_url,
                1,
                &[
                    DavProperty::resource_type(),
                    DavProperty::display_name(),
                    DavProperty::supported_components(),
                    DavProperty::privileges(),
                    DavProperty::calendar_time_zone(),
                    DavProperty::calendar_color(),
                    DavProperty::get_ctag(),
                    DavProperty::sync_token(),
                ],
            )
            .await?;
        let default_id = self.default_calendar_id().await?;
        let mut list: Vec<CalDavCalendarInfo> = Vec::new();
        for response in &status.responses {
            let Ok(url) = self.client.resolve(&response.href, &answered_by) else {
                continue;
            };
            let Some(id) = self.calendar_id_for(&url) else {
                continue;
            };
            let Some(info) = self.calendar_from(response, &id, &default_id) else {
                continue;
            };
            list.push(info);
        }
        // Known limitation: Swift orders non-ASCII ids by canonical equivalence, Rust by bytes.
        list.sort_by(|a, b| a.descriptor.id.cmp(&b.descriptor.id));
        self.state.store(list.clone());
        Ok(list)
    }

    pub(crate) fn calendar_from(
        &self,
        response: &DavResponse,
        id: &str,
        default_id: &Option<Option<String>>,
    ) -> Option<CalDavCalendarInfo> {
        let empty = Vec::new();
        let types = response
            .properties
            .get(&DavProperty::resource_type())
            .map_or(&empty, |p| &p.children);
        let has = |namespace: &str, name: &str| {
            types
                .iter()
                .any(|t| t.namespace == namespace && t.name == name)
        };
        if !has(Dav::CALDAV, "calendar")
            || has(Dav::CALDAV, "schedule-inbox")
            || has(Dav::CALDAV, "schedule-outbox")
            || has(Dav::CALENDAR_SERVER, "notification")
        {
            return None;
        }
        if let Some(set) = response
            .properties
            .get(&DavProperty::supported_components())
        {
            let names: Vec<String> = set
                .children_named(Dav::CALDAV, "comp")
                .iter()
                .filter_map(|c| c.attributes.get("name").map(|n| n.to_uppercase()))
                .collect();
            if !names.iter().any(|n| n == "VEVENT") {
                return None;
            }
        }
        // A server that does not report privileges is treated as the owner's own calendar.
        let privileges: Option<BTreeSet<String>> = response
            .properties
            .get(&DavProperty::privileges())
            .map(|set| {
                set.children_named(Dav::DAV, "privilege")
                    .iter()
                    .flat_map(|p| p.children.iter().map(|c| c.name.clone()))
                    .collect()
            });
        let any_of = |names: &[&str]| {
            privileges
                .as_ref()
                .map(|p| names.iter().any(|n| p.contains(*n)))
        };
        let subscribed = has(Dav::CALENDAR_SERVER, "subscribed");
        let can_edit = !subscribed && any_of(&["write", "write-content", "all"]).unwrap_or(true);
        let permissions = CalendarPermissions {
            can_view_details: any_of(&["read", "all"]).unwrap_or(true),
            can_edit,
            can_share: Some(any_of(&["write-acl", "all"]).unwrap_or(false)),
            can_view_private: Some(can_edit),
        };
        let zone = response
            .properties
            .get(&DavProperty::calendar_time_zone())
            .and_then(|p| Self::zone_from_calendar_time_zone(&p.text))
            .unwrap_or_else(|| self.default_zone.clone());
        let display_name = response
            .properties
            .get(&DavProperty::display_name())
            .map(|p| p.trimmed_text().to_string())
            .unwrap_or_default();
        let color: Option<String> = response
            .properties
            .get(&DavProperty::calendar_color())
            .map(|p| p.trimmed_text().chars().take(7).collect());
        let title = if display_name.is_empty() {
            id.split('/')
                .rfind(|p| !p.is_empty())
                .unwrap_or(id)
                .to_string()
        } else {
            display_name
        };
        let mut descriptor = CalendarDescriptor::new(id, title, CalendarService::cal_dav())
            .with_color_hex(color.as_deref());
        descriptor.permissions = permissions;
        descriptor.is_default = default_id.as_ref().map(|d| d.as_deref() == Some(id));
        descriptor.time_zone = Some(zone.clone());
        descriptor.account_name = Some(self.connection.display_name.clone());
        descriptor.kind = if subscribed {
            CalendarKind::Subscribed
        } else {
            CalendarKind::Standard
        };
        descriptor.provider = Some(self.provider.clone());
        descriptor.supported_availabilities =
            Some(BTreeSet::from([Availability::Busy, Availability::Free]));
        Some(CalDavCalendarInfo {
            descriptor,
            url: self.calendar_url(id),
            ctag: response
                .properties
                .get(&DavProperty::get_ctag())
                .map(|p| p.trimmed_text().to_string()),
            sync_token: response
                .properties
                .get(&DavProperty::sync_token())
                .map(|p| p.trimmed_text().to_string()),
            zone,
        })
    }

    /// The `calendar-timezone` value is a `VCALENDAR` holding one `VTIMEZONE`.
    pub(crate) fn zone_from_calendar_time_zone(text: &str) -> Option<Zone> {
        let calendar = ICalParser::parse_text(text).ok()?;
        let tzid = calendar
            .components_named("VTIMEZONE")
            .first()?
            .property("TZID")?
            .value
            .clone();
        TimeZoneResolver::from_calendar(&calendar).zone_for(&tzid)
    }

    /// `Some(Some(id))` for the calendar that receives invitations, `Some(None)` when the server says none of these,
    /// `None` when it does not say (RFC 6638 puts `schedule-default-calendar-URL` on the scheduling inbox; some servers
    /// on the principal). Only an expired sign-in is an error here.
    pub(crate) async fn default_calendar_id(&self) -> Outcome<Option<Option<String>>> {
        match self.look_up_default_calendar().await {
            Err(Failure::Source(SourceError::AuthExpired)) => {
                Err(Failure::Source(SourceError::AuthExpired))
            }
            Err(Failure::Cancelled) => Err(Failure::Cancelled),
            Err(_) => Ok(None),
            Ok(found) => Ok(found),
        }
    }

    async fn look_up_default_calendar(&self) -> Outcome<Option<Option<String>>> {
        let (principal, principal_url) = self
            .client
            .propfind(
                &self.account.principal_url,
                0,
                &[
                    DavProperty::schedule_default_calendar_url(),
                    DavProperty::schedule_inbox_url(),
                ],
            )
            .await?;
        let empty = BTreeMap::new();
        let properties = principal
            .responses
            .first()
            .map_or(&empty, |r| &r.properties);
        let href_of = |properties: &BTreeMap<DavProperty, crate::xml_tree::XmlTree>,
                       property: DavProperty| {
            properties
                .get(&property)
                .and_then(|p| p.child(Dav::DAV, "href"))
                .map(|h| h.trimmed_text().to_string())
        };
        if let Some(href) = href_of(properties, DavProperty::schedule_default_calendar_url()) {
            return Ok(Some(
                self.calendar_id_for(&self.client.resolve(&href, &principal_url)?),
            ));
        }
        let Some(inbox_href) = href_of(properties, DavProperty::schedule_inbox_url()) else {
            return Ok(None);
        };
        let inbox_url = self.client.resolve(&inbox_href, &principal_url)?;
        let (inbox, answered_by) = self
            .client
            .propfind(
                &inbox_url,
                0,
                &[DavProperty::schedule_default_calendar_url()],
            )
            .await?;
        let Some(href) = inbox
            .responses
            .first()
            .and_then(|r| href_of(&r.properties, DavProperty::schedule_default_calendar_url()))
        else {
            return Ok(None);
        };
        Ok(Some(self.calendar_id_for(
            &self.client.resolve(&href, &answered_by)?,
        )))
    }

    pub(crate) async fn calendar_zone(&self, calendar_id: &str) -> Outcome<Zone> {
        let calendars = match self.state.calendars() {
            Some(stored) => stored,
            None => self.load_calendars().await?,
        };
        Ok(calendars
            .iter()
            .find(|c| c.descriptor.id == calendar_id)
            .map_or_else(|| self.default_zone.clone(), |c| c.zone.clone()))
    }

    pub(crate) fn context(
        &self,
        calendar_id: &str,
        zone: &Zone,
        resource_name: &str,
        etag: Option<String>,
    ) -> EventReadContext {
        EventReadContext::new(
            calendar_id,
            resource_name,
            etag,
            Some(self.source_id()),
            zone.clone(),
            self.self_addresses(),
        )
    }

    // MARK: Events

    async fn events_for(
        &self,
        calendar: &CalDavCalendarInfo,
        interval: &DateInterval,
    ) -> Outcome<Vec<CalendarEvent>> {
        let reply = self
            .client
            .report(
                &calendar.url,
                1,
                DavXml::calendar_query_range(interval.start, interval.end),
            )
            .await?;
        match reply.response.status {
            207 => {}
            403 | 404 | 410 => return Ok(Vec::new()),
            other => {
                return Err(Failure::Source(SourceError::InvalidResponse(format!(
                    "calendar-query answered {other}"
                ))));
            }
        }
        let mut events = Vec::new();
        for response in DavXml::multistatus(&reply.response.body)?.responses {
            let Ok(url) = self.client.resolve(&response.href, &reply.url) else {
                continue;
            };
            let Some((resource, etag)) = self.calendar_object(&response, &url) else {
                continue;
            };
            let context = self.context(
                &calendar.descriptor.id,
                &calendar.zone,
                &DavUrl::last_path_component(&url),
                Some(etag),
            );
            events.extend(EventReader::events_in(&resource, interval, &context));
        }
        Ok(events)
    }

    /// The parsed resource in a multistatus response (reused while its ETag is unchanged); `None` when it has no ETag
    /// or cannot be read, so one bad file does not hide the rest.
    pub(crate) fn calendar_object(
        &self,
        response: &DavResponse,
        url: &UrlString,
    ) -> Option<(EventResource, String)> {
        let etag = response
            .properties
            .get(&DavProperty::get_etag())?
            .trimmed_text()
            .to_string();
        if etag.is_empty() {
            return None;
        }
        if let Some(cached) = self.state.resource(url, &etag) {
            return Some((cached, etag));
        }
        let text = &response.properties.get(&DavProperty::calendar_data())?.text;
        let resource = EventResource::from_data(text.as_bytes()).ok()?;
        self.state.remember(&resource, url, &etag);
        Some((resource, etag))
    }
}

#[async_trait]
impl CalendarSource for CalDavCalendarSource {
    fn id(&self) -> String {
        self.source_id()
    }

    fn display_name(&self) -> String {
        self.connection.display_name.clone()
    }

    fn capabilities(&self) -> SourceCapabilities {
        let mut provided: BTreeSet<ProvidedField> = BTreeSet::from([
            ProvidedField::Visibility,
            ProvidedField::Availability,
            ProvidedField::Reminders,
            ProvidedField::Series,
            ProvidedField::Version,
            ProvidedField::UidScope,
            ProvidedField::RecurrenceRules,
            ProvidedField::PermissionDetails,
            ProvidedField::Provider,
            ProvidedField::CalendarTimeZone,
            ProvidedField::SupportedAvailabilities,
        ]);
        if !self.account.user_addresses.is_empty() {
            provided.insert(ProvidedField::Participation);
        }
        let mut writable: BTreeSet<EventField> = BTreeSet::from([
            EventField::Title,
            EventField::Notes,
            EventField::Location,
            EventField::Timing,
            EventField::Availability,
            EventField::Visibility,
            EventField::Reminders,
            EventField::Recurrence,
        ]);
        // Without server scheduling, attendees written would never be invited.
        if self.account.auto_schedule {
            writable.insert(EventField::Attendees);
        }
        SourceCapabilities {
            can_write: true,
            can_edit_attendees: self.account.auto_schedule,
            can_respond_to_invite: self.account.auto_schedule
                && !self.account.user_addresses.is_empty(),
            provided_fields: provided,
            sync_kind: SyncKind::Token,
            supports_push: false,
            writable_fields: writable,
            controls_notifications: false,
            recurrence_scopes: RecurrenceScope::ALL.iter().copied().collect(),
        }
    }

    async fn calendars(&self) -> Outcome<Vec<CalendarDescriptor>> {
        Ok(self
            .load_calendars()
            .await?
            .into_iter()
            .map(|c| c.descriptor)
            .collect())
    }

    /// Uses the calendar list the last `calendars()` call or poll stored. A calendar removed since then (403, 404, 410)
    /// is skipped; one the account can see only as free/busy is not queried. (Swift reads the calendars concurrently;
    /// here one after the other, which gives the same events in the same order.)
    async fn events_in(&self, interval: DateInterval) -> Outcome<Vec<CalendarEvent>> {
        let calendars = match self.state.calendars() {
            Some(stored) => stored,
            None => self.load_calendars().await?,
        };
        let mut all: Vec<CalendarEvent> = Vec::new();
        for calendar in calendars
            .iter()
            .filter(|c| c.descriptor.permissions.can_view_details)
        {
            all.extend(self.events_for(calendar, &interval).await?);
        }
        all.sort_by(|a, b| (a.start, &a.event_id).cmp(&(b.start, &b.event_id)));
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
