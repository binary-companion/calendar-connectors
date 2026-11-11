//! Port of `Fixtures.swift`: the sample feed and the helpers every ICalSubscription test shares, plus the test
//! doubles the Swift tests declare privately (`StubInteraction`, and the transports that throw).

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use calendar_core::change_monitor::ChangeMonitor;
use calendar_core::connection::Connection;
use calendar_core::connection::{AuthorizationInteraction, CredentialField, OAuthRedirectSession};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::{HttpRequest, HttpResponse, HttpTransport};
use calendar_core::source_types::CalendarChange;
use calendar_core::source_types::SourceError;
use calendar_core::time::{DateInterval, Now, Sleeper, Zone};
use calendar_core::url_string::UrlString;
use jiff::Timestamp;

use crate::kind::ICalSubscriptionKind;
use crate::source::{LinkProvider, SourceSettings};

pub(super) const PRIVATE_PATH: &str = "PRIVATE-PATH-0123";

pub(super) fn pacific() -> Zone {
    Zone::named("America/Los_Angeles").unwrap()
}

pub(super) fn feed_url() -> UrlString {
    UrlString::parse(&format!(
        "https://www.example.test/events/ical/42/{PRIVATE_PATH}/going"
    ))
    .unwrap()
}

/// `pt(2026, 9, 1, 10, 0)`: a Pacific wall-clock time as an instant (Swift's `pt(_:_:_:_:_:)`).
pub(super) fn pt(y: i16, mo: i8, d: i8, h: i8, mi: i8) -> Timestamp {
    jiff::civil::date(y, mo, d)
        .at(h, mi, 0, 0)
        .to_zoned(pacific().as_jiff().clone())
        .unwrap()
        .timestamp()
}

pub(super) fn september() -> DateInterval {
    DateInterval::new(pt(2026, 9, 1, 0, 0), pt(2026, 10, 1, 0, 0))
}

const LA_TIME_ZONE: &[&str] = &[
    "BEGIN:VTIMEZONE",
    "TZID:America/Los_Angeles",
    "X-LIC-LOCATION:America/Los_Angeles",
    "BEGIN:DAYLIGHT",
    "TZOFFSETFROM:-0800",
    "TZOFFSETTO:-0700",
    "TZNAME:PDT",
    "DTSTART:19700308T020000",
    "RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU",
    "END:DAYLIGHT",
    "BEGIN:STANDARD",
    "TZOFFSETFROM:-0700",
    "TZOFFSETTO:-0800",
    "TZNAME:PST",
    "DTSTART:19701101T020000",
    "RRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU",
    "END:STANDARD",
    "END:VTIMEZONE",
];

/// A feed shaped like a Meetup "going" feed: one calendar name, one `VTIMEZONE`, and the given events.
pub(super) fn feed_ics(events: &[Vec<String>], header: &[&str]) -> String {
    let mut lines: Vec<String> = vec![
        "BEGIN:VCALENDAR".into(),
        "VERSION:2.0".into(),
        "PRODID:-//Example//Feed 1.0//EN".into(),
        "METHOD:PUBLISH".into(),
    ];
    lines.extend(header.iter().map(|line| line.to_string()));
    lines.extend(LA_TIME_ZONE.iter().map(|line| line.to_string()));
    for event in events {
        lines.extend(event.iter().cloned());
    }
    lines.push("END:VCALENDAR".into());
    lines.join("\r\n") + "\r\n"
}

/// Swift's default header argument, `["X-WR-CALNAME:My Meetups"]`.
pub(super) const DEFAULT_HEADER: &[&str] = &["X-WR-CALNAME:My Meetups"];

pub(super) fn feed(events: &[Vec<String>]) -> String {
    feed_ics(events, DEFAULT_HEADER)
}

pub(super) fn vevent(
    uid: Option<&str>,
    title: &str,
    start: &str,
    end: &str,
    extra: &[&str],
) -> Vec<String> {
    let mut lines = vec!["BEGIN:VEVENT".to_string()];
    if let Some(uid) = uid {
        lines.push(format!("UID:{uid}"));
    }
    lines.push("DTSTAMP:20260901T000000Z".into());
    lines.push(format!("SUMMARY:{title}"));
    lines.push(format!("DTSTART;TZID=America/Los_Angeles:{start}"));
    lines.push(format!("DTEND;TZID=America/Los_Angeles:{end}"));
    lines.extend(extra.iter().map(|line| line.to_string()));
    lines.push("END:VEVENT".into());
    lines
}

pub(super) fn board_games() -> Vec<String> {
    vevent(
        Some("event_board@example.test"),
        "Board games night",
        "20260910T190000",
        "20260910T210000",
        &["URL:https://www.example.test/events/1/", "LOCATION:Cafe"],
    )
}

pub(super) fn weekly_walk() -> Vec<String> {
    vevent(
        Some("event_walk@example.test"),
        "Weekly walk",
        "20260901T100000",
        "20260901T110000",
        &["RRULE:FREQ=WEEKLY;BYDAY=TU"],
    )
}

/// Two events in September: the single one on the 10th and a weekly Tuesday walk (five instances), six in all.
pub(super) fn sample_feed() -> String {
    feed(&[board_games(), weekly_walk()])
}

pub(super) fn ok(text: &str) -> HttpResponse {
    HttpResponse::new(200, BTreeMap::new(), text.as_bytes().to_vec())
}

pub(super) fn ok_with(text: &str, headers: &[(&str, &str)]) -> HttpResponse {
    HttpResponse::new(
        200,
        headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect(),
        text.as_bytes().to_vec(),
    )
}

pub(super) fn status(code: i64) -> HttpResponse {
    HttpResponse::new(code, BTreeMap::new(), Vec::new())
}

pub(super) fn status_with(code: i64, headers: &[(&str, &str)]) -> HttpResponse {
    HttpResponse::new(
        code,
        headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect(),
        Vec::new(),
    )
}

pub(super) fn redirect(code: i64, location: &str) -> HttpResponse {
    status_with(code, &[("Location", location)])
}

/// A sleeper that returns at once (Swift's `sleep: { _ in }`).
pub(super) fn immediate_sleeper() -> Sleeper {
    Arc::new(|_| Box::pin(async { Ok(()) }))
}

/// The link provider of Swift's `link: { feedURL }`.
pub(super) fn fixed_link(url: UrlString) -> LinkProvider {
    Arc::new(move || {
        let url = url.clone();
        Box::pin(async move { Ok(url) })
    })
}

/// `ICalSubscriptionKind(transport:)` with Swift's defaults where the library has none: the test clock, an
/// immediate sleeper and UTC as the default zone.
pub(super) fn kind(transport: Arc<dyn HttpTransport>) -> ICalSubscriptionKind {
    ICalSubscriptionKind::new(
        transport,
        calendar_test_support::fake_transport::TestNow::default().provider(),
        immediate_sleeper(),
        Zone::utc(),
    )
}

/// Answers `prompt_credentials` with fixed values and records the fields it was shown.
pub(super) struct StubInteraction {
    state: Mutex<StubState>,
}

struct StubState {
    answers: VecDeque<BTreeMap<String, String>>,
    fields: Vec<Vec<CredentialField>>,
}

impl StubInteraction {
    pub(super) fn new(answers: Vec<BTreeMap<String, String>>) -> StubInteraction {
        StubInteraction {
            state: Mutex::new(StubState {
                answers: answers.into(),
                fields: Vec::new(),
            }),
        }
    }

    /// One answer: `{"link": text}`.
    pub(super) fn link(text: &str) -> StubInteraction {
        StubInteraction::new(vec![BTreeMap::from([(
            "link".to_string(),
            text.to_string(),
        )])])
    }

    /// No answers: the prompt is cancelled.
    pub(super) fn cancelled() -> StubInteraction {
        StubInteraction::new(Vec::new())
    }

    pub(super) fn shown(&self) -> Vec<Vec<CredentialField>> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .fields
            .clone()
    }
}

#[async_trait]
impl AuthorizationInteraction for StubInteraction {
    async fn begin_oauth_redirect(&self) -> Outcome<Box<dyn OAuthRedirectSession>> {
        Err(Failure::Source(SourceError::InvalidResponse(
            "not an OAuth kind".to_string(),
        )))
    }

    async fn prompt_credentials(
        &self,
        requested: &[CredentialField],
    ) -> Outcome<BTreeMap<String, String>> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.fields.push(requested.to_vec());
        state.answers.pop_front().ok_or(Failure::Cancelled)
    }
}

/// Swift's private `ThrowingTransport`: fails every request with the given failure.
pub(super) struct ThrowingTransport {
    pub(super) failure: Failure,
}

#[async_trait]
impl HttpTransport for ThrowingTransport {
    async fn send(&self, _request: HttpRequest) -> Outcome<HttpResponse> {
        Err(self.failure.clone())
    }
}

/// `.eventsChanged(calendarIDs: ["feed"])`.
pub(super) fn feed_changed() -> CalendarChange {
    CalendarChange::EventsChanged {
        calendar_ids: Some(std::collections::BTreeSet::from(["feed".to_string()])),
    }
}

/// The source Swift's tests build: connection `c1` ("x (www.example.test)", host `www.example.test`), the link
/// `feed_url()`, a monitor with a 900 second interval and an immediate sleeper, and a 900 second maximum age. Tests
/// assign the fields they vary.
pub(super) fn source_settings(
    transport: Arc<dyn HttpTransport>,
    now: Now,
    default_zone: Zone,
) -> SourceSettings {
    SourceSettings::new(
        Connection::new("icalsub", "c1", "x (www.example.test)")
            .with_config(&[("host", "www.example.test")]),
        fixed_link(feed_url()),
        transport,
        ChangeMonitor::new(900.0, 900.0, immediate_sleeper()),
        900.0,
        now,
        default_zone,
    )
}
