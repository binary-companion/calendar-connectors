//! Port of `ICalSubscriptionSource.swift`.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use async_trait::async_trait;
use calendar_core::change_monitor::ChangeMonitor;
use calendar_core::connection::Connection;
use calendar_core::diagnostic_log::{
    DiagnosticField, DiagnosticLevel, DiagnosticLog, NullDiagnosticLog,
};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::HttpTransport;
use calendar_core::model::{
    CalendarDescriptor, CalendarEvent, CalendarKind, CalendarPermissions, CalendarProvider,
    CalendarService,
};
use calendar_core::source_types::{
    CalendarChange, CalendarSource, PollingCalendarSource, ProvidedField, SourceCapabilities,
    SourceError, SourceNotice, SourceNoticeKind, SyncKind,
};
use calendar_core::time::{DateInterval, Now, Zone, seconds_between};
use calendar_core::url_string::UrlString;
use calendar_ical::{EventReadContext, EventReader};
use jiff::Timestamp;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::feed_fetcher::{FeedFetcher, FeedResponse, FeedValidators};
use crate::feed_parser::{FeedParser, ParsedFeed, UnreadableRule};
use crate::retention_window::RetentionWindow;

/// What `check_for_changes` found out about the fetch it just made, relative to what it had reported before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Report {
    Changed,
    Unchanged,
    Stale,
}

/// What a fetch needs to know about the held feed, read together.
pub(crate) struct Snapshot {
    pub body: Option<Arc<Vec<u8>>>,
    pub fingerprint: Option<Arc<Vec<u8>>>,
    pub validators: Option<FeedValidators>,
}

#[derive(Default)]
struct FeedStateData {
    feed: Option<Arc<ParsedFeed>>,
    body: Option<Arc<Vec<u8>>>,
    /// What the retained part of the feed looks like; this, not the raw body, decides whether anything changed.
    fingerprint: Option<Arc<Vec<u8>>>,
    validators: Option<FeedValidators>,
    fetched_at: Option<Timestamp>,
    issued: i64,
    stored_generation: i64,
    /// Newest fetch that found the feed unchanged. Kept apart from `stored_generation` so that a newer "unchanged"
    /// answer does not stop an older fetch that saw genuinely different content from being stored (see `report`).
    unchanged_generation: i64,
    reported_body: Option<Arc<Vec<u8>>>,
    previous_reported_body: Option<Arc<Vec<u8>>>,
    reported_generation: i64,
    reported_unreadable: BTreeSet<UnreadableRule>,
    unreadable_generation: i64,
}

/// What the source has loaded: the parsed feed, the bytes it came from (to spot a change) and what the server said
/// about that version. Swift's actor; here a `std` mutex that no method holds across an `.await` (none awaits).
pub(crate) struct FeedState {
    data: Mutex<FeedStateData>,
}

impl FeedState {
    pub(crate) fn new() -> FeedState {
        FeedState {
            data: Mutex::new(FeedStateData::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, FeedStateData> {
        self.data.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn feed(&self) -> Option<Arc<ParsedFeed>> {
        self.lock().feed.clone()
    }

    pub(crate) fn fingerprint(&self) -> Option<Arc<Vec<u8>>> {
        self.lock().fingerprint.clone()
    }

    pub(crate) fn validators(&self) -> Option<FeedValidators> {
        self.lock().validators.clone()
    }

    pub(crate) fn fetched_at(&self) -> Option<Timestamp> {
        self.lock().fetched_at
    }

    /// The unreadable-rule keys not reported yet; the set then becomes `current`, so a rule that was fixed and breaks
    /// again, or is edited, is new again. A fetch older than one already applied changes nothing, so the set never
    /// moves backwards.
    pub(crate) fn new_unreadable(
        &self,
        current: &[UnreadableRule],
        generation: i64,
    ) -> BTreeSet<UnreadableRule> {
        let mut state = self.lock();
        if generation <= state.unreadable_generation {
            return BTreeSet::new();
        }
        state.unreadable_generation = generation;
        let current: BTreeSet<UnreadableRule> = current.iter().cloned().collect();
        let fresh = current
            .difference(&state.reported_unreadable)
            .cloned()
            .collect();
        state.reported_unreadable = current;
        fresh
    }

    /// Taken when a fetch starts, so a fetch that finishes after a newer one can be told apart.
    pub(crate) fn next_generation(&self) -> i64 {
        let mut state = self.lock();
        state.issued += 1;
        state.issued
    }

    /// Keeps the feed only if no newer fetch has stored one already.
    pub(crate) fn store(
        &self,
        feed: Arc<ParsedFeed>,
        body: Arc<Vec<u8>>,
        fingerprint: Arc<Vec<u8>>,
        validators: FeedValidators,
        generation: i64,
        at: Timestamp,
    ) {
        let mut state = self.lock();
        if generation <= state.stored_generation {
            return;
        }
        state.stored_generation = generation;
        state.feed = Some(feed);
        state.body = Some(body);
        state.fingerprint = Some(fingerprint);
        state.validators = Some(validators);
        state.fetched_at = Some(at);
    }

    /// What `check_for_changes` last saw, independent of what `events_in` loaded. The first report has no earlier one,
    /// so `prior` (the fingerprint held when the check began) stands in for it. A check whose fetch is older than one
    /// already reported says nothing, so overlapping checks report a change once and never report one backwards.
    pub(crate) fn report(
        &self,
        body: &Arc<Vec<u8>>,
        generation: i64,
        prior: Option<&Arc<Vec<u8>>>,
    ) -> Report {
        let mut state = self.lock();
        if generation <= state.reported_generation {
            // An older fetch can still be the first to see a change (the server answered a newer request with the old
            // feed). Report a body that differs from the last two reported ones, once; never move the generation back.
            let Some(reported) = state.reported_body.clone() else {
                return Report::Stale;
            };
            if *body == reported || state.previous_reported_body.as_ref() == Some(body) {
                return Report::Stale;
            }
            state.previous_reported_body = Some(reported);
            state.reported_body = Some(body.clone());
            return Report::Changed;
        }
        state.reported_generation = generation;
        let last = state.reported_body.clone().or_else(|| prior.cloned());
        if let Some(reported) = state.reported_body.clone()
            && reported != *body
        {
            state.previous_reported_body = Some(reported);
        }
        state.reported_body = Some(body.clone());
        match last {
            Some(last) if last != *body => Report::Changed,
            _ => Report::Unchanged,
        }
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        let state = self.lock();
        Snapshot {
            body: state.body.clone(),
            fingerprint: state.fingerprint.clone(),
            validators: state.validators.clone(),
        }
    }

    /// The feed is unchanged: keep what is held and restart its age. A fetch older than the stored feed is ignored:
    /// what it found is no longer news, and its validators would replace newer ones.
    pub(crate) fn unchanged(
        &self,
        validators: Option<FeedValidators>,
        body: Option<Arc<Vec<u8>>>,
        generation: i64,
        at: Timestamp,
    ) {
        let mut state = self.lock();
        if generation <= state.stored_generation.max(state.unchanged_generation) {
            return;
        }
        state.unchanged_generation = generation;
        if let Some(body) = body {
            state.body = Some(body);
        }
        if let Some(validators) = validators
            && validators.has_any()
        {
            state.validators = Some(validators);
        }
        state.fetched_at = Some(at);
    }
}

/// Supplies the feed link on every fetch (Swift `@Sendable () async throws -> URL`).
pub(crate) type LinkProvider =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Outcome<UrlString>> + Send>> + Send + Sync>;

/// Everything Swift's initializer takes. `retention` and `diagnostics` have the Swift defaults (none, and
/// `NullDiagnosticLog`).
pub(crate) struct SourceSettings {
    pub connection: Connection,
    /// Read on every fetch, so a replaced link (after `reauthorize`) is picked up without rebuilding the source.
    pub link: LinkProvider,
    pub transport: Arc<dyn HttpTransport>,
    pub monitor: ChangeMonitor,
    /// Seconds a loaded feed is reused.
    pub max_age: f64,
    pub now: Now,
    pub default_zone: Zone,
    pub retention: Option<RetentionWindow>,
    pub diagnostics: Arc<dyn DiagnosticLog>,
}

impl SourceSettings {
    pub(crate) fn new(
        connection: Connection,
        link: LinkProvider,
        transport: Arc<dyn HttpTransport>,
        monitor: ChangeMonitor,
        max_age: f64,
        now: Now,
        default_zone: Zone,
    ) -> SourceSettings {
        SourceSettings {
            connection,
            link,
            transport,
            monitor,
            max_age,
            now,
            default_zone,
            retention: None,
            diagnostics: Arc::new(NullDiagnosticLog),
        }
    }
}

/// One iCal subscription link: a read-only calendar of whatever the feed holds, re-read when it is older than
/// `max_age`. Built by [`crate::ICalSubscriptionKind::make_source`]; the polling stream (`changes()`) keeps the
/// source alive while a receiver exists, as the Swift stream keeps the class alive.
pub struct ICalSubscriptionSource {
    connection: Connection,
    link: LinkProvider,
    fetcher: FeedFetcher,
    monitor: ChangeMonitor,
    max_age: f64,
    now: Now,
    default_zone: Zone,
    retention: Option<RetentionWindow>,
    diagnostics: Arc<dyn DiagnosticLog>,
    state: FeedState,
    /// `changes()` hands the monitor an `Arc` of this very source (Swift passes `self`).
    me: Weak<ICalSubscriptionSource>,
}

/// The result of one `refresh`.
struct Fetched {
    fingerprint: Arc<Vec<u8>>,
    generation: i64,
    body_changed: bool,
}

impl ICalSubscriptionSource {
    pub(crate) const CALENDAR_ID: &'static str = "feed";

    pub(crate) fn new(settings: SourceSettings) -> Arc<ICalSubscriptionSource> {
        Arc::new_cyclic(|me| ICalSubscriptionSource {
            fetcher: FeedFetcher::new(settings.transport)
                .with_diagnostics(settings.diagnostics.clone(), settings.now.clone()),
            connection: settings.connection,
            link: settings.link,
            monitor: settings.monitor,
            max_age: settings.max_age,
            now: settings.now,
            default_zone: settings.default_zone,
            retention: settings.retention,
            diagnostics: settings.diagnostics,
            state: FeedState::new(),
            me: me.clone(),
        })
    }

    async fn current_feed(&self) -> Outcome<Arc<ParsedFeed>> {
        if let (Some(feed), Some(fetched_at)) = (self.state.feed(), self.state.fetched_at())
            && seconds_between(fetched_at, (self.now)()) < self.max_age
        {
            return Ok(feed);
        }
        self.refresh().await?;
        self.state
            .feed()
            .ok_or_else(|| invalid("the feed has not loaded"))
    }

    /// Fetches the feed (conditionally when the server gave validators), keeps what the retention window allows, and
    /// returns a fingerprint of that with the generation the fetch started under. A 304 re-reads the held body, so an
    /// event that has come into the window since is noticed. A body that is not a calendar fails and leaves the last
    /// good feed in place; a fetch that finishes after a newer one does not replace what that one stored.
    async fn refresh(&self) -> Outcome<Fetched> {
        let url = (self.link)().await?;
        let generation = self.state.next_generation();
        let held = self.state.snapshot();
        let validators = held.validators.filter(FeedValidators::has_any);
        let previous_body = held.body;
        let previous_fingerprint = held.fingerprint;
        let sent = if previous_body.is_none() {
            None
        } else {
            validators.as_ref()
        };
        let data: Arc<Vec<u8>>;
        let mut new_validators: Option<FeedValidators> = None;
        let mut new_body: Option<Arc<Vec<u8>>> = None;
        match self.fetcher.fetch(&url, sent).await? {
            FeedResponse::NotModified => {
                data = previous_body
                    .clone()
                    .ok_or_else(|| invalid("the feed has not loaded"))?;
            }
            FeedResponse::Body(body, received) => {
                data = Arc::new(body);
                new_body = Some(data.clone());
                new_validators = Some(received);
            }
        }
        let at = (self.now)();
        let retention = self
            .retention
            .map(|window| (window.interval_at(at), self.default_zone.clone()));
        let feed =
            FeedParser::parse_logged(&data, retention, self.diagnostics.as_ref(), at, false)?;
        let mut fresh: Vec<UnreadableRule> = self
            .state
            .new_unreadable(&feed.unreadable_rules, generation)
            .into_iter()
            .collect();
        fresh.sort_by(|a, b| (&a.uid, &a.rule_hash).cmp(&(&b.uid, &b.rule_hash)));
        for rule in fresh {
            self.diagnostics.log(rule.event(at));
        }
        let fingerprint = Arc::new(feed.fingerprint());
        if previous_fingerprint.as_ref() == Some(&fingerprint) {
            self.state
                .unchanged(new_validators, new_body.clone(), generation, at);
        } else {
            let kept = self.state.validators().unwrap_or_default();
            self.state.store(
                Arc::new(feed),
                data,
                fingerprint.clone(),
                new_validators.unwrap_or(kept),
                generation,
                at,
            );
        }
        let body_changed = match (&previous_body, &new_body) {
            (Some(previous), Some(new)) => new != previous,
            _ => false,
        };
        Ok(Fetched {
            fingerprint,
            generation,
            body_changed,
        })
    }

    fn record(&self, name: &str, fields: Vec<DiagnosticField>) {
        self.diagnostics
            .record(DiagnosticLevel::Info, "icalsub", name, fields, (self.now)());
    }
}

fn invalid(message: &str) -> Failure {
    Failure::Source(SourceError::InvalidResponse(message.to_string()))
}

#[async_trait]
impl CalendarSource for ICalSubscriptionSource {
    fn id(&self) -> String {
        self.connection.source_id()
    }

    fn display_name(&self) -> String {
        self.connection.display_name.clone()
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities {
            provided_fields: BTreeSet::from([
                ProvidedField::Series,
                ProvidedField::UidScope,
                ProvidedField::Provider,
                ProvidedField::CalendarTimeZone,
            ]),
            sync_kind: SyncKind::Token,
            ..SourceCapabilities::default()
        }
    }

    async fn calendars(&self) -> Outcome<Vec<CalendarDescriptor>> {
        let feed = self.current_feed().await?;
        let mut calendar = CalendarDescriptor::new(
            Self::CALENDAR_ID,
            feed.name
                .clone()
                .unwrap_or_else(|| self.connection.display_name.clone()),
            CalendarService::ical_subscription(),
        )
        .with_color_hex(feed.color_hex.as_deref());
        calendar.permissions = CalendarPermissions {
            can_view_details: true,
            can_edit: false,
            can_share: None,
            can_view_private: None,
        };
        calendar.is_default = Some(false);
        calendar.time_zone = Some(
            feed.time_zone
                .clone()
                .unwrap_or_else(|| self.default_zone.clone()),
        );
        calendar.account_name = Some(self.connection.display_name.clone());
        calendar.kind = CalendarKind::Subscribed;
        calendar.provider = Some(CalendarProvider::subscription());
        Ok(vec![calendar])
    }

    async fn events_in(&self, interval: DateInterval) -> Outcome<Vec<CalendarEvent>> {
        let feed = self.current_feed().await?;
        let zone = feed
            .time_zone
            .clone()
            .unwrap_or_else(|| self.default_zone.clone());
        let mut events: Vec<CalendarEvent> = Vec::new();
        for item in &feed.resources {
            let context = EventReadContext::new(
                Self::CALENDAR_ID,
                item.name.clone(),
                None,
                Some(self.id()),
                zone.clone(),
                BTreeSet::new(),
            );
            events.extend(EventReader::events_in(&item.resource, &interval, &context));
        }
        events.sort_by_cached_key(|event| (event.start, event.id()));
        Ok(events)
    }

    fn changes(&self) -> UnboundedReceiver<CalendarChange> {
        match self.me.upgrade() {
            Some(me) => self.monitor.changes(me),
            None => unbounded_channel().1,
        }
    }

    /// Repeating events in the held feed whose rule could not be read (they show only their first occurrence and
    /// exceptions). Empty when none, and empty before the first load.
    async fn notices(&self) -> Vec<SourceNotice> {
        let count = self
            .state
            .feed()
            .map_or(0, |feed| feed.unreadable_rules.len() as i64);
        if count > 0 {
            vec![SourceNotice::new(
                SourceNoticeKind::UnreadableRecurrence,
                count,
            )]
        } else {
            Vec::new()
        }
    }

    fn as_polling(&self) -> Option<&dyn PollingCalendarSource> {
        Some(self)
    }
}

#[async_trait]
impl PollingCalendarSource for ICalSubscriptionSource {
    /// Re-reads the feed. The first check with nothing loaded yet is the baseline and reports nothing; a feed loaded
    /// earlier by `events_in` counts as the baseline, so a change since then is reported.
    async fn check_for_changes(&self) -> Outcome<Option<CalendarChange>> {
        let prior = self.state.fingerprint();
        let fetched = self.refresh().await?;
        let outcome = self
            .state
            .report(&fetched.fingerprint, fetched.generation, prior.as_ref());
        match outcome {
            Report::Changed => self.record(
                "changeReported",
                vec![DiagnosticField::bool("bodyChanged", fetched.body_changed)],
            ),
            Report::Unchanged if fetched.body_changed => self.record(
                "changeSuppressed",
                vec![DiagnosticField::public_string(
                    "reason",
                    "unchangedRetained",
                )],
            ),
            Report::Stale if fetched.body_changed => self.record(
                "changeSuppressed",
                vec![DiagnosticField::public_string("reason", "staleFetch")],
            ),
            _ => {}
        }
        Ok(
            (outcome == Report::Changed).then(|| CalendarChange::EventsChanged {
                calendar_ids: Some(BTreeSet::from([Self::CALENDAR_ID.to_string()])),
            }),
        )
    }
}
