//! The calendar model: calendars, events, attendees, reminders and conference links. Port of `Model.swift`.

use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

use jiff::Timestamp;

use crate::raw_enum::raw_enum;
use crate::time::{Zone, add_seconds};
use crate::url_string::UrlString;

raw_enum! {
    /// What a calendar is, for grouping and defaults: `birthdays` and `subscribed` (holidays, read-only feeds) are
    /// system-style calendars that rarely deserve a takeover.
    pub enum CalendarKind { Standard = "standard", Subscribed = "subscribed", Birthdays = "birthdays" }
}

raw_enum! {
    pub enum AccessRole {
        Owner = "owner", Writer = "writer", Reader = "reader", FreeBusyReader = "freeBusyReader"
    }
}

raw_enum! {
    pub enum EventStatus { Confirmed = "confirmed", Tentative = "tentative", Cancelled = "cancelled" }
}

raw_enum! {
    pub enum Availability {
        Busy = "busy", Free = "free", Tentative = "tentative", Unavailable = "unavailable"
    }
}

impl Availability {
    /// The value itself when the calendar supports it; otherwise the first supported value from its fallback list
    /// (`tentative` and `unavailable` fall back to busy, then free; busy to free; free to busy). `None` when
    /// `supported` is empty (the calendar does not track availability).
    pub fn closest_in(&self, supported: &BTreeSet<Availability>) -> Option<Availability> {
        if supported.contains(self) {
            return Some(*self);
        }
        let fallbacks: &[Availability] = match self {
            Availability::Tentative | Availability::Unavailable => {
                &[Availability::Busy, Availability::Free]
            }
            Availability::Busy => &[Availability::Free],
            Availability::Free => &[Availability::Busy],
        };
        fallbacks.iter().copied().find(|a| supported.contains(a))
    }
}

raw_enum! {
    pub enum Visibility {
        Default = "default", PublicEvent = "publicEvent", PrivateEvent = "privateEvent",
        Confidential = "confidential"
    }
}

raw_enum! {
    pub enum EventKind {
        Standard = "standard", FocusTime = "focusTime", OutOfOffice = "outOfOffice",
        WorkingLocation = "workingLocation", Birthday = "birthday", Other = "other"
    }
}

raw_enum! {
    /// Where an event's `uid` can be compared: `.global` is an iCalendar UID, the same across any source; `.provider`
    /// is a provider-specific id (an Exchange object id) that only equals the ids of events from the same service and
    /// provider. (Swift `UIDScope`.)
    pub enum UidScope { Global = "global", Provider = "provider" }
}

raw_enum! {
    pub enum ResponseStatus {
        Accepted = "accepted", Tentative = "tentative", Declined = "declined", NeedsAction = "needsAction"
    }
}

raw_enum! {
    pub enum AttendeeRole { Required = "required", Optional = "optional", Resource = "resource" }
}

/// Whether an event repeats. A real "no" has its own case, so `None` stays "the source does not say".
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SeriesInfo {
    NotRecurring,
    /// An instance of a series. `original_start` is the instance's slot in the series (it differs from `start` for a
    /// moved instance); `None` when the provider knows the series but not the slot.
    Occurrence {
        series_id: String,
        original_start: Option<Timestamp>,
    },
}

/// The account owner's relation to an event. `NotInvited` is a real answer (an event you are not on).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Participation {
    NotInvited,
    Invited(ResponseStatus),
}

raw_enum! {
    pub enum ConferenceProvider {
        Meet = "meet", Teams = "teams", Zoom = "zoom", Webex = "webex", GoToMeeting = "goToMeeting",
        Whereby = "whereby", Jitsi = "jitsi", Slack = "slack", Other = "other"
    }
}

raw_enum! {
    /// Where a conference link came from, so consumers can decide how far to trust it. (`EventUrl` is Swift's
    /// `eventURL`.)
    pub enum ConferenceOrigin {
        Structured = "structured", Location = "location", Url = "url", Notes = "notes", EventUrl = "eventURL"
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Attendee {
    pub name: Option<String>,
    /// Trimmed and lowercased; never empty. Set only through [`Attendee::new`] (Swift `private(set)`).
    email: Option<String>,
    pub role: AttendeeRole,
    pub response: ResponseStatus,
    pub is_self: bool,
    pub is_organizer: bool,
}

impl Attendee {
    /// Swift's memberwise `init`; the email is trimmed and lowercased, and an empty one becomes `None`.
    pub fn new(
        name: Option<String>,
        email: Option<&str>,
        role: AttendeeRole,
        response: ResponseStatus,
        is_self: bool,
        is_organizer: bool,
    ) -> Attendee {
        let normalized = email.map(|e| e.trim().to_lowercase());
        Attendee {
            name,
            email: normalized.filter(|e| !e.is_empty()),
            role,
            response,
            is_self,
            is_organizer,
        }
    }

    pub fn email(&self) -> Option<&str> {
        self.email.as_deref()
    }
}

impl Default for Attendee {
    /// `Attendee()`: no name or email, a required attendee who has not answered.
    fn default() -> Attendee {
        Attendee::new(
            None,
            None,
            AttendeeRole::Required,
            ResponseStatus::NeedsAction,
            false,
            false,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ConferenceInfo {
    pub url: UrlString,
    pub provider: ConferenceProvider,
    pub origin: ConferenceOrigin,
}

impl ConferenceInfo {
    /// Swift's `init(url:provider:origin:)`; pass `ConferenceOrigin::Structured` for the default.
    pub fn new(
        url: UrlString,
        provider: ConferenceProvider,
        origin: ConferenceOrigin,
    ) -> ConferenceInfo {
        ConferenceInfo {
            url,
            provider,
            origin,
        }
    }

    /// Lowercased host plus path without trailing slashes, plus `?mtid=` for Webex (which keeps the meeting id
    /// in that query item). Two links with the same identity are the same meeting.
    pub fn identity(&self) -> String {
        let host = self
            .url
            .host()
            .map_or_else(String::new, |h| h.to_lowercase());
        let mut path = self.url.path().to_lowercase();
        while path.ends_with('/') {
            path.pop();
        }
        let mut identity = host.clone() + &path;
        if host == "webex.com" || host.ends_with(".webex.com") {
            let meeting_id = self
                .url
                .query_items()
                .into_iter()
                .find(|(name, _)| name.to_lowercase() == "mtid")
                .and_then(|(_, value)| value);
            if let Some(meeting_id) = meeting_id {
                identity.push_str("?mtid=");
                identity.push_str(&meeting_id.to_lowercase());
            }
        }
        identity
    }
}

/// What a reminder counts from when it is relative to the event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReminderAnchor {
    Start,
    End,
}

/// Equal doubles hash alike, and `0.0` and `-0.0` (which compare equal) hash alike, as Swift's `Hashable` does.
fn hash_f64<H: Hasher>(value: f64, state: &mut H) {
    let normalized = if value == 0.0 { 0.0 } else { value };
    normalized.to_bits().hash(state);
}

fn hash_optional_f64<H: Hasher>(value: Option<f64>, state: &mut H) {
    value.is_some().hash(state);
    if let Some(value) = value {
        hash_f64(value, state);
    }
}

/// When a reminder fires. The cases are mutually exclusive, so a reminder cannot be both absolute and location-based.
#[derive(Clone, Debug, PartialEq)]
pub enum ReminderTrigger {
    /// Seconds from the event's start or end; negative means before. EventKit's `relativeOffset`; `End` covers
    /// iCalendar `RELATED=END`, which EventKit cannot express.
    Relative { offset: f64, to: ReminderAnchor },
    /// A fixed moment (EventKit `absoluteDate`).
    Absolute(Timestamp),
    /// Arriving at or leaving a place (EventKit `structuredLocation` and `proximity`).
    Location(StructuredLocation, ReminderProximity),
}

/// Swift's `Hashable` over a `Double` makes `NaN` unequal to itself too; the marker is the same promise.
impl Eq for ReminderTrigger {}

impl Hash for ReminderTrigger {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            ReminderTrigger::Relative { offset, to } => {
                hash_f64(*offset, state);
                to.hash(state);
            }
            ReminderTrigger::Absolute(date) => date.hash(state),
            ReminderTrigger::Location(location, proximity) => {
                location.hash(state);
                proximity.hash(state);
            }
        }
    }
}

/// What the alert does, with its related value. `Other` keeps a provider value the library does not know.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ReminderType {
    #[default]
    Display,
    Audio {
        sound_name: Option<String>,
    },
    /// A `None` address means the account owner (Google).
    Email {
        address: Option<String>,
    },
    Procedure {
        url: Option<UrlString>,
    },
    Other(String),
}

raw_enum! {
    pub enum ReminderProximity { Enter = "enter", Leave = "leave" }
}

/// A place, as plain values (no CoreLocation), so the library stays portable. Used by location reminders only; an
/// event's own location stays the provider's raw text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StructuredLocation {
    pub title: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    /// Metres; `None` means the platform default.
    pub radius: Option<f64>,
}

impl Eq for StructuredLocation {}

impl Hash for StructuredLocation {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.title.hash(state);
        hash_optional_f64(self.latitude, state);
        hash_optional_f64(self.longitude, state);
        hash_optional_f64(self.radius, state);
    }
}

/// One reminder, closely modeled on EventKit's `EKAlarm`. The library reports every reminder in this one shape; the
/// consumer decides which to honor and when to show them. It never filters, merges or schedules them.
#[derive(Clone, Debug, PartialEq)]
pub struct Reminder {
    pub trigger: ReminderTrigger,
    pub kind: ReminderType,
    /// How many more times it fires after the first (iCalendar REPEAT); 0 means once.
    pub repeat_count: i64,
    /// Seconds between repeats (iCalendar DURATION); `None` when it does not repeat.
    pub repeat_interval: Option<f64>,
    /// True when the reminder came from the calendar's defaults (Google `useDefault`); `None` when the source cannot
    /// say. Not part of a reminder's write identity.
    pub is_calendar_default: Option<bool>,
}

impl Eq for Reminder {}

impl Hash for Reminder {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.trigger.hash(state);
        self.kind.hash(state);
        self.repeat_count.hash(state);
        hash_optional_f64(self.repeat_interval, state);
        self.is_calendar_default.hash(state);
    }
}

impl Reminder {
    /// `Reminder(trigger:)` with every other argument at its Swift default: an on-screen alert that fires once.
    /// (`kind` is Swift's `type`.)
    pub fn new(trigger: ReminderTrigger) -> Reminder {
        Reminder {
            trigger,
            kind: ReminderType::Display,
            repeat_count: 0,
            repeat_interval: None,
            is_calendar_default: None,
        }
    }

    /// `Reminder(minutesBefore:)`: relative to the start, a plain on-screen alert: the common case.
    pub fn with_minutes_before(minutes_before: i64) -> Reminder {
        Reminder::new(ReminderTrigger::Relative {
            offset: -(minutes_before as f64 * 60.0),
            to: ReminderAnchor::Start,
        })
    }

    /// `Reminder.before(minutes:type:isCalendarDefault:)`; pass `ReminderType::Display` and `None` for the defaults.
    pub fn before(minutes: i64, kind: ReminderType, is_calendar_default: Option<bool>) -> Reminder {
        Reminder {
            kind,
            is_calendar_default,
            ..Reminder::with_minutes_before(minutes)
        }
    }

    /// Whole minutes before the start; `None` unless the trigger is relative to the start.
    pub fn minutes_before(&self) -> Option<i64> {
        match self.trigger {
            ReminderTrigger::Relative {
                offset,
                to: ReminderAnchor::Start,
            } => Some((-offset / 60.0).round() as i64),
            _ => None,
        }
    }

    /// When it fires for an event running `event_start` to `event_end`. All-day events already use canonical
    /// midnights, so offsets land correctly. `None` for a location trigger, whose timing is up to the host.
    pub fn fire_date(&self, event_start: Timestamp, event_end: Timestamp) -> Option<Timestamp> {
        match &self.trigger {
            ReminderTrigger::Relative { offset, to } => {
                let anchor = if *to == ReminderAnchor::Start {
                    event_start
                } else {
                    event_end
                };
                Some(add_seconds(anchor, *offset))
            }
            ReminderTrigger::Absolute(date) => Some(*date),
            ReminderTrigger::Location(..) => None,
        }
    }

    /// Everything a write can change, without `is_calendar_default` (which differs between a provider read and a
    /// hand-built list and is not something a writer sets).
    pub(crate) fn write_identity(&self) -> String {
        let interval = self
            .repeat_interval
            .map_or_else(|| "-".to_string(), |v| format!("{v:?}"));
        format!(
            "{:?}|{:?}|{}|{}",
            self.trigger, self.kind, self.repeat_count, interval
        )
    }

    /// Whether two lists hold the same reminders, ignoring order and `is_calendar_default` (providers do not promise
    /// to keep reminder order).
    pub fn same_set(a: &[Reminder], b: &[Reminder]) -> bool {
        let sorted = |list: &[Reminder]| {
            let mut identities: Vec<String> = list.iter().map(Reminder::write_identity).collect();
            identities.sort();
            identities
        };
        sorted(a) == sorted(b)
    }
}

macro_rules! string_backed {
    ($(#[$meta:meta])* $name:ident { $($(#[$cmeta:meta])* $constructor:ident = $raw:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            /// `init(rawValue:)`.
            pub fn new(raw_value: impl Into<String>) -> $name {
                $name(raw_value.into())
            }

            pub fn raw_value(&self) -> &str {
                &self.0
            }

            $($(#[$cmeta])* pub fn $constructor() -> $name {
                $name::new($raw)
            })+
        }
    };
}

string_backed! {
    /// The connector a calendar is read through (`ConnectorKind.id`): "eventkit", "google", "microsoft", "caldav",
    /// "icalsub". A string-backed value so a host can add its own without changing the library.
    CalendarService {
        event_kit = "eventkit",
        google = "google",
        microsoft = "microsoft",
        cal_dav = "caldav",
        /// An iCalendar subscription link (Meetup's Add to calendar links, a Google secret address, ...).
        ical_subscription = "icalsub",
    }
}

string_backed! {
    /// Who hosts a calendar, which can differ from the service reading it: a Google calendar read directly has service
    /// and provider Google; read through Apple Calendar its service is EventKit. `None` on a descriptor means the
    /// source cannot tell. A string-backed value so hosts (and a CalDAV connector, which can pass the server's
    /// domain) can add their own.
    CalendarProvider {
        icloud = "icloud",
        google = "google",
        /// Exchange, Outlook.com, Microsoft 365.
        microsoft = "microsoft",
        /// On the device.
        local = "local",
        /// A read-only feed (subscribed or birthday calendar).
        subscription = "subscription",
        /// A CalDAV server whose host is not known.
        cal_dav = "caldav",
    }
}

/// What the account may do on a calendar (Graph-style flags; Google roles and CalDAV privileges map onto them).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CalendarPermissions {
    /// False means free/busy only.
    pub can_view_details: bool,
    pub can_edit: bool,
    /// Can manage who has access (Google's "owner"); `None` when the source does not say.
    pub can_share: Option<bool>,
    pub can_view_private: Option<bool>,
}

impl Default for CalendarPermissions {
    /// `CalendarPermissions()`: can view details, cannot edit, nothing else known.
    fn default() -> CalendarPermissions {
        CalendarPermissions {
            can_view_details: true,
            can_edit: false,
            can_share: None,
            can_view_private: None,
        }
    }
}

impl CalendarPermissions {
    /// A one-word summary: free/busy only, reader, writer or owner; `None` when the source cannot separate a writer
    /// from an owner (EventKit, writable).
    pub fn access_role(&self) -> Option<AccessRole> {
        if !self.can_view_details {
            return Some(AccessRole::FreeBusyReader);
        }
        if !self.can_edit {
            return Some(AccessRole::Reader);
        }
        match self.can_share {
            Some(true) => Some(AccessRole::Owner),
            Some(false) => Some(AccessRole::Writer),
            None => None,
        }
    }
}

/// One calendar within an account.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CalendarDescriptor {
    pub id: String,
    pub title: String,
    /// "#RRGGBB" uppercase, or `None` when unknown or invalid. Set through [`CalendarDescriptor::with_color_hex`].
    color_hex: Option<String>,
    pub permissions: CalendarPermissions,
    /// The calendar where new events go by default in its account; `None` when the source cannot tell.
    pub is_default: Option<bool>,
    pub time_zone: Option<Zone>,
    /// The owning account, e.g. the signed-in email.
    pub account_name: Option<String>,
    pub kind: CalendarKind,
    /// The reminders a new event on this calendar gets when it does not set its own (Google's calendar defaults);
    /// `None` when the source does not say. Events with `useDefault` reminders carry these.
    pub default_reminders: Option<Vec<Reminder>>,
    /// The connector this calendar is read through; always known.
    pub service: CalendarService,
    /// Who hosts the calendar; `None` when the source cannot tell.
    pub provider: Option<CalendarProvider>,
    /// The availability values the calendar accepts; an empty set means it does not track availability, `None` that
    /// the source does not say.
    pub supported_availabilities: Option<BTreeSet<Availability>>,
}

impl CalendarDescriptor {
    /// Swift's `init` with every optional argument at its default; assign the public fields for the rest, and set the
    /// colour through [`CalendarDescriptor::with_color_hex`].
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        service: CalendarService,
    ) -> CalendarDescriptor {
        CalendarDescriptor {
            id: id.into(),
            title: title.into(),
            color_hex: None,
            permissions: CalendarPermissions::default(),
            is_default: None,
            time_zone: None,
            account_name: None,
            kind: CalendarKind::Standard,
            default_reminders: None,
            service,
            provider: None,
            supported_availabilities: None,
        }
    }

    /// Sets the colour as `init(colorHex:)` does: normalized with [`CalendarDescriptor::normalized_hex`].
    pub fn with_color_hex(mut self, raw: Option<&str>) -> CalendarDescriptor {
        self.color_hex = CalendarDescriptor::normalized_hex(raw);
        self
    }

    pub fn color_hex(&self) -> Option<&str> {
        self.color_hex.as_deref()
    }

    /// A read-only summary of `permissions`.
    pub fn access_role(&self) -> Option<AccessRole> {
        self.permissions.access_role()
    }

    /// "#RGB", "#RRGGBB" or "RRGGBB" (any case) to "#RRGGBB" uppercase; `None` for anything else.
    pub fn normalized_hex(raw: Option<&str>) -> Option<String> {
        let trimmed = raw?.trim();
        let digits = trimmed.strip_prefix('#').unwrap_or(trimmed);
        let length = digits.chars().count();
        if (length != 3 && length != 6) || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let upper = digits.to_uppercase();
        let expanded: String = if length == 3 {
            upper.chars().flat_map(|c| [c, c]).collect()
        } else {
            upper
        };
        Some(format!("#{expanded}"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CalendarEvent {
    /// Unique within its calendar (a recurring instance has its own id).
    pub event_id: String,
    /// iCalendar UID (Google `iCalUID`); stable across copies of the same meeting.
    pub uid: Option<String>,
    /// What kind of id `uid` is; `None` when the source does not say (treated as `Provider`).
    pub uid_scope: Option<UidScope>,
    pub calendar_id: String,
    pub title: String,
    pub notes: Option<String>,
    pub location: Option<String>,
    /// All-day: midnight of the first day in `time_zone`, `end` exclusive (midnight after the last day).
    pub start: Timestamp,
    pub end: Timestamp,
    /// The zone the event is shown in; always set. `start` and `end` (instants) are authoritative and this is a
    /// display hint: when the provider gives the event no zone the connector fills its fallback (Google: the
    /// calendar's zone, then UTC; EventKit: the device zone). An all-day event's days are read in this zone, not the
    /// device's.
    pub time_zone: Zone,
    pub is_all_day: bool,
    pub status: EventStatus,
    /// The fields below marked "provided" follow one rule: a field listed in `SourceCapabilities.provided_fields` is
    /// never `None` on that source's events. A field that is not listed may be `None`, and `None` always means "this
    /// source does not say", never "none". A real "none" has its own value (`[]`, `NotRecurring`, `NotInvited`).
    /// Content fields every source supports (`notes`, `location`, `url`, `uid`, `organizer`) use `None` for "empty".
    pub availability: Option<Availability>,
    pub visibility: Option<Visibility>,
    pub kind: Option<EventKind>,
    /// Whether the event repeats and where it sits in its series.
    pub series: Option<SeriesInfo>,
    pub attendees: Vec<Attendee>,
    pub organizer: Option<Attendee>,
    /// Join links, most likely first: the provider's structured links, then links found in the location, url and
    /// notes (see `ConferenceDetector`).
    pub conferences: Vec<ConferenceInfo>,
    /// An empty list means none; `None` means the source does not say.
    pub reminders: Option<Vec<Reminder>>,
    pub url: Option<UrlString>,
    /// An opaque token that changes whenever the event changes; compare for equality only. Detects changes since a
    /// cached copy and is the base of optimistic writes (Google etag, EventKit modification date).
    pub version: Option<String>,
    /// When the provider last changed / first created the event; `None` when it does not say (EventKit may omit them).
    pub last_modified: Option<Timestamp>,
    pub created: Option<Timestamp>,
    /// The account owner's relation to the event, when the provider says.
    pub participation: Option<Participation>,
    /// The source that produced this event (`Connection.sourceID`; "eventkit" for EventKit). Stamped by the source.
    pub source_id: Option<String>,
}

impl CalendarEvent {
    /// Swift's `init` with every optional argument at its default (`timeZone` UTC, `status` confirmed); set the
    /// public fields (struct update) for the rest.
    pub fn new(
        event_id: impl Into<String>,
        calendar_id: impl Into<String>,
        title: impl Into<String>,
        start: Timestamp,
        end: Timestamp,
    ) -> CalendarEvent {
        CalendarEvent {
            event_id: event_id.into(),
            uid: None,
            uid_scope: None,
            calendar_id: calendar_id.into(),
            title: title.into(),
            notes: None,
            location: None,
            start,
            end,
            time_zone: Zone::utc(),
            is_all_day: false,
            status: EventStatus::Confirmed,
            availability: None,
            visibility: None,
            kind: None,
            series: None,
            attendees: Vec::new(),
            organizer: None,
            conferences: Vec::new(),
            reminders: None,
            url: None,
            version: None,
            last_modified: None,
            created: None,
            participation: None,
            source_id: None,
        }
    }

    /// Unique across the calendars of one source.
    pub fn id(&self) -> String {
        format!("{}/{}", self.calendar_id, self.event_id)
    }

    /// The series id when this is an instance of a series; `None` for a single event and when unknown.
    pub fn series_id(&self) -> Option<&str> {
        match &self.series {
            Some(SeriesInfo::Occurrence { series_id, .. }) => Some(series_id),
            _ => None,
        }
    }

    pub fn original_start(&self) -> Option<Timestamp> {
        match &self.series {
            Some(SeriesInfo::Occurrence { original_start, .. }) => *original_start,
            _ => None,
        }
    }

    /// The first (most likely) link.
    pub fn conference(&self) -> Option<&ConferenceInfo> {
        self.conferences.first()
    }

    /// The account owner's own response; `None` both when unknown and when not invited (use `participation` to tell
    /// them apart).
    pub fn my_response(&self) -> Option<ResponseStatus> {
        match self.participation {
            Some(Participation::Invited(response)) => Some(response),
            _ => None,
        }
    }
}
