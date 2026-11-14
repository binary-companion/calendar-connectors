//! What a source can do and what it reports, and the `CalendarSource` seam. Port of `SourceTypes.swift`.

use std::collections::BTreeSet;

use async_trait::async_trait;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::failure::Outcome;
use crate::model::{CalendarDescriptor, CalendarEvent};
use crate::raw_enum::raw_enum;
use crate::recurrence::set::CalendarSeries;
use crate::time::DateInterval;
use crate::write::types::{EventField, RecurrenceScope};
use crate::write::writable_source::WritableCalendarSource;

/// How a source learns about changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SyncKind {
    /// The source has no way to report changes.
    #[default]
    None,
    /// The source supports incremental checks with a sync token (`PollingCalendarSource`).
    Token,
    /// The platform notifies the source of changes (EventKit).
    Notification,
}

raw_enum! {
    /// A field a source reliably fills on the events (or calendars) it reads. A listed field is never `None`; an
    /// unlisted one may be `None` or partly filled, and `None` means "this source does not say", never "none".
    pub enum ProvidedField {
        // Events
        /// `CalendarEvent::kind` is filled on every event.
        Kind = "kind",
        /// `CalendarEvent::visibility` is filled on every event.
        Visibility = "visibility",
        /// `CalendarEvent::availability` is filled on every event.
        Availability = "availability",
        /// `CalendarEvent::reminders` is filled on every event.
        Reminders = "reminders",
        /// `CalendarEvent::series` is filled on every event.
        Series = "series",
        /// `CalendarEvent::participation` is filled on every event.
        Participation = "participation",
        /// Supplies structured conference data (Google `conferenceData`); says nothing about whether an event has
        /// links.
        StructuredConference = "structuredConference",
        /// `CalendarEvent::version` is filled on every event.
        Version = "version",
        /// `CalendarEvent::last_modified` is filled on every event.
        LastModified = "lastModified",
        /// `CalendarEvent::created` is filled on every event.
        Created = "created",
        /// `CalendarEvent::uid_scope` is filled on every event.
        UidScope = "uidScope",
        /// The source can return a series' recurrence rules (`SeriesSource`); declared exactly when it conforms.
        RecurrenceRules = "recurrenceRules",
        // Calendars
        /// `CalendarDescriptor::is_default` is filled on every calendar.
        IsDefault = "isDefault",
        /// `CalendarDescriptor::time_zone` is filled on every calendar.
        CalendarTimeZone = "calendarTimeZone",
        /// `CalendarDescriptor::default_reminders` is filled on every calendar.
        DefaultReminders = "defaultReminders",
        /// `CalendarDescriptor::provider` is filled on every calendar.
        Provider = "provider",
        /// `CalendarDescriptor::supported_availabilities` is filled on every calendar.
        SupportedAvailabilities = "supportedAvailabilities",
        /// `permissions.can_share` and `can_view_private` are never `None`.
        PermissionDetails = "permissionDetails",
    }
}

/// What a source can do and which fields it reliably fills.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceCapabilities {
    /// True when the source also implements `WritableCalendarSource`.
    pub can_write: bool,
    /// True exactly when `writable_fields` contains `EventField::Attendees`.
    pub can_edit_attendees: bool,
    /// True when `WritableCalendarSource::respond` works on this source.
    pub can_respond_to_invite: bool,
    /// What this source reliably fills on the events it reads (see [`ProvidedField`]). Separate from
    /// `writable_fields`, which is what a write can change; a few names overlap (reminders, availability) with
    /// different meanings.
    pub provided_fields: BTreeSet<ProvidedField>,
    /// How the source detects changes.
    pub sync_kind: SyncKind,
    /// Reserved for push notifications from a server; no connector supports it.
    pub supports_push: bool,
    /// The fields create/update can write; drives validation and `EventDraft(copying:for:)`. Empty when read-only.
    pub writable_fields: BTreeSet<EventField>,
    /// Honors `NotifyPolicy` on create, update and delete (Google: `sendUpdates`); false means the server decides for
    /// those. A connector may still honor the policy for a response to an invite (Microsoft: `sendResponse`).
    pub controls_notifications: bool,
    /// Scopes accepted for update/delete on a recurring series. Empty when read-only.
    pub recurrence_scopes: BTreeSet<RecurrenceScope>,
}

/// What the library reports through `changes()`. Sources fail with `SourceError`; they never leak provider types.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum SourceError {
    /// The refresh token or credentials are no longer valid; the user must sign in again.
    #[error("authorization expired")]
    AuthExpired,
    /// The server could not be reached; the text describes the transport error.
    #[error("network error: {0}")]
    Network(String),
    /// The server asked the caller to slow down.
    #[error("rate limited")]
    RateLimited {
        /// Seconds to wait before retrying, when the server said.
        retry_after: Option<f64>,
    },
    /// The server failed.
    #[error("server error {status}")]
    Server {
        /// The HTTP status code.
        status: i64,
    },
    /// The server's answer could not be understood, or the request was refused in a way that is not covered by another
    /// case.
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    /// The OS or user has not granted access to a local data store (EventKit). Never raised by network connectors.
    #[error("access not granted")]
    NeedsPermission,
    /// The id names nothing the source can find (a series that does not exist, or an event that does not recur).
    #[error("not found")]
    NotFound,
}

/// What a source reports through `changes()`.
#[derive(Clone, Debug, PartialEq)]
pub enum CalendarChange {
    /// The calendar set changed. Consumers must reload the calendar list AND events; event changes detected in the
    /// same check are not reported separately.
    CalendarsChanged,
    /// `None` calendar ids mean the scope is unknown.
    EventsChanged {
        /// Calendars that had changes, or `None` when the scope is unknown.
        calendar_ids: Option<BTreeSet<String>>,
    },
    /// Terminal: the stream finishes after this (e.g. `AuthExpired`) so the app can surface it and re-authorize.
    SourceFailed(SourceError),
}

/// Something about a source worth a quiet hint in a front end (a warning icon), as opposed to a failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceNotice {
    /// What the notice is about.
    pub kind: SourceNoticeKind,
    /// How many items it concerns.
    pub count: i64,
}

/// The kind of a `SourceNotice`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceNoticeKind {
    /// Repeating events whose rule could not be read; `count` is the number of such events (series).
    UnreadableRecurrence,
}

impl SourceNotice {
    /// Creates a notice.
    pub fn new(kind: SourceNoticeKind, count: i64) -> SourceNotice {
        SourceNotice { kind, count }
    }
}

/// A read-only view of one connected account's calendars and events. Obtain it from `ConnectorKind::make_source`.
#[async_trait]
pub trait CalendarSource: Send + Sync {
    /// Unique id of this source; equals `Connection::source_id` for sources made from a connection.
    fn id(&self) -> String;
    /// Name the host shows for this account.
    fn display_name(&self) -> String;
    /// What this source can do and which event fields it fills.
    fn capabilities(&self) -> SourceCapabilities;
    /// The account's calendars, including those the account may only read. Fails with `Failure::Source`.
    async fn calendars(&self) -> Outcome<Vec<CalendarDescriptor>>;
    /// Events of every visible calendar of the account that overlap `interval`.
    async fn events_in(&self, interval: DateInterval) -> Outcome<Vec<CalendarEvent>>;
    /// A new receiver for what the source reports; every call returns its own. Changes are never dropped.
    fn changes(&self) -> UnboundedReceiver<CalendarChange>;
    /// What the source noticed in the data it last loaded. Optional: the default is none.
    async fn notices(&self) -> Vec<SourceNotice> {
        Vec::new()
    }
    /// Swift's `as? PollingCalendarSource`: `Some` when the source can poll.
    fn as_polling(&self) -> Option<&dyn PollingCalendarSource> {
        None
    }
    /// Swift's `as? SeriesSource`: `Some` when the source can return a series' rules.
    fn as_series(&self) -> Option<&dyn SeriesSource> {
        None
    }
    /// Swift's `as? WritableCalendarSource`: `Some` when the source can also write.
    fn as_writable(&self) -> Option<&dyn WritableCalendarSource> {
        None
    }
}

/// A source that can check for changes cheaply with a sync token instead of re-reading everything.
#[async_trait]
pub trait PollingCalendarSource: CalendarSource {
    /// One cheap incremental check (sync token / delta). Returns the change since the last call, or `None` for none.
    /// The first call establishes the baseline and returns `None`.
    async fn check_for_changes(&self) -> Outcome<Option<CalendarChange>>;
}

/// A source that can return a recurring series' rules on demand, so `events_in` stays cheap. Declared through
/// `ProvidedField::RecurrenceRules`; a source declares it exactly when it conforms.
#[async_trait]
pub trait SeriesSource: CalendarSource {
    /// The series with this id (an instance's `SeriesInfo::Occurrence { series_id, .. }`). Fails with
    /// `SourceError::NotFound` for an unknown id or an id that is not a recurring series.
    async fn series(&self, id: &str, calendar_id: &str) -> Outcome<CalendarSeries>;
}
