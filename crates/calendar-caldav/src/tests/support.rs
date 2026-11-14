//! Port of `FakeCalDAVServer.swift` and `Harness.swift`: an in-memory CalDAV server behind `HttpTransport`, the
//! iCalendar builders the tests share, and the harness that builds a source against it.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use calendar_core::change_monitor::ChangeMonitor;
use calendar_core::connection::{
    AuthorizationInteraction, Connection, CredentialField, CredentialStore,
    InMemoryCredentialStore, InMemorySyncStateStore, OAuthRedirectSession,
};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::{HttpRequest, HttpResponse, HttpTransport};
use calendar_core::model::CalendarProvider;
use calendar_core::source_types::SourceError;
use calendar_core::time::{DateInterval, Sleeper, Zone};
use calendar_core::url_string::UrlString;
use calendar_test_support::fake_transport::TestNow;
use jiff::Timestamp;

use crate::account_config::CalDavAccountConfig;
use crate::dav_url::DavUrl;
use crate::dav_xml::{Dav, DavXml};
use crate::source::{CalDavCalendarSource, CalDavSourceSettings};
use crate::webdav_client::{CredentialProvider, WebDavClient, WebDavCredentials};
use crate::xml_tree::XmlTree;

// MARK: The fake server

#[derive(Clone, Debug)]
pub(crate) struct Resource {
    pub body: String,
    pub etag: String,
    pub revision: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct Collection {
    pub display_name: String,
    pub color: Option<String>,
    /// `None`: the server does not say (all components).
    pub components: Option<Vec<String>>,
    pub privileges: Vec<String>,
    pub time_zone_id: Option<String>,
    pub subscribed: bool,
    pub resources: BTreeMap<String, Resource>,
    pub removed: BTreeMap<String, i64>,
    pub revision: i64,
}

impl Collection {
    /// Swift's memberwise `Collection(displayName:)` with every other member at its default.
    pub(crate) fn new(display_name: &str) -> Collection {
        Collection {
            display_name: display_name.to_string(),
            color: Some("#1BADF8FF".to_string()),
            components: Some(vec!["VEVENT".to_string()]),
            privileges: vec!["read".to_string(), "write".to_string()],
            time_zone_id: Some("America/Los_Angeles".to_string()),
            subscribed: false,
            resources: BTreeMap::new(),
            removed: BTreeMap::new(),
            revision: 1,
        }
    }
}

struct Failing {
    method: String,
    path_contains: String,
    status: i64,
    remaining: i64,
    skip: i64,
}

pub(crate) struct FakeState {
    pub username: String,
    pub password: String,
    pub user_addresses: Vec<String>,
    pub auto_schedule: bool,
    pub supports_sync: bool,
    pub sends_etag_on_put: bool,
    pub default_calendar: Option<String>,
    /// Where `/.well-known/caldav` redirects; `None` answers 404.
    pub well_known_location: Option<String>,
    pub collections: BTreeMap<String, Collection>,
    revision: i64,
    oldest_valid_token: i64,
    failures: Vec<Failing>,
    log: Vec<HttpRequest>,
}

pub(crate) struct FakeCalDavServer {
    state: Mutex<FakeState>,
}

pub(crate) const PRINCIPAL_PATH: &str = "/123/principal/";
pub(crate) const HOME_PATH: &str = "/123/calendars/";
pub(crate) const INBOX_PATH: &str = "/123/inbox/";

pub(crate) fn fake_server_url() -> UrlString {
    UrlString::parse("https://caldav.icloud.com").unwrap()
}

pub(crate) fn fake_home_url() -> UrlString {
    UrlString::parse("https://caldav.icloud.com/123/calendars/").unwrap()
}

fn crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

impl FakeCalDavServer {
    pub(crate) fn new() -> FakeCalDavServer {
        let mut collections = BTreeMap::new();
        collections.insert("home".to_string(), Collection::new("Home"));
        FakeCalDavServer {
            state: Mutex::new(FakeState {
                username: "me@icloud.test".to_string(),
                password: "app-pass-1234".to_string(),
                user_addresses: vec![
                    "mailto:me@icloud.test".to_string(),
                    "urn:uuid:11111111-2222-3333-4444-555555555555".to_string(),
                ],
                auto_schedule: true,
                supports_sync: true,
                sends_etag_on_put: true,
                default_calendar: Some("home".to_string()),
                well_known_location: Some("/".to_string()),
                collections,
                revision: 1,
                oldest_valid_token: 0,
                failures: Vec::new(),
                log: Vec::new(),
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn configure(&self, change: impl FnOnce(&mut FakeState)) {
        change(&mut self.lock());
    }

    // MARK: Test helpers

    /// A change made by someone else (another client).
    pub(crate) fn store(&self, calendar: &str, name: &str, ics: &str) {
        let mut state = self.lock();
        state.revision += 1;
        let revision = state.revision;
        let collection = state
            .collections
            .entry(calendar.to_string())
            .or_insert_with(|| Collection::new(calendar));
        collection.resources.insert(
            name.to_string(),
            Resource {
                body: crlf(ics),
                etag: format!("\"e{revision}\""),
                revision,
            },
        );
        collection.removed.remove(name);
        collection.revision = revision;
    }

    pub(crate) fn remove(&self, calendar: &str, name: &str) {
        let mut state = self.lock();
        Self::remove_locked(&mut state, calendar, name);
    }

    fn remove_locked(state: &mut FakeState, calendar: &str, name: &str) {
        state.revision += 1;
        let revision = state.revision;
        if let Some(collection) = state.collections.get_mut(calendar) {
            collection.resources.remove(name);
            collection.removed.insert(name.to_string(), revision);
            collection.revision = revision;
        }
    }

    pub(crate) fn body(&self, calendar: &str, name: &str) -> Option<String> {
        self.lock()
            .collections
            .get(calendar)?
            .resources
            .get(name)
            .map(|r| r.body.clone())
    }

    pub(crate) fn etag(&self, calendar: &str, name: &str) -> Option<String> {
        self.lock()
            .collections
            .get(calendar)?
            .resources
            .get(name)
            .map(|r| r.etag.clone())
    }

    pub(crate) fn names(&self, calendar: &str) -> Vec<String> {
        self.lock()
            .collections
            .get(calendar)
            .map(|c| c.resources.keys().cloned().collect())
            .unwrap_or_default()
    }

    pub(crate) fn log(&self) -> Vec<HttpRequest> {
        self.lock().log.clone()
    }

    pub(crate) fn requests(&self, method: &str) -> Vec<HttpRequest> {
        self.lock()
            .log
            .iter()
            .filter(|r| r.method == method)
            .cloned()
            .collect()
    }

    pub(crate) fn clear_log(&self) {
        self.lock().log.clear();
    }

    /// The next request with this method whose path contains `path_contains` answers `status`.
    pub(crate) fn fail(&self, method: &str, path_contains: &str, status: i64) {
        self.fail_n(method, path_contains, status, 1, 0);
    }

    /// After letting `after` matching requests through, the next `times` requests with this method whose path
    /// contains `path_contains` answer `status`.
    pub(crate) fn fail_n(
        &self,
        method: &str,
        path_contains: &str,
        status: i64,
        times: i64,
        after: i64,
    ) {
        self.lock().failures.push(Failing {
            method: method.to_string(),
            path_contains: path_contains.to_string(),
            status,
            remaining: times,
            skip: after,
        });
    }

    /// Every sync token issued so far stops being accepted.
    pub(crate) fn expire_sync_tokens(&self) {
        let mut state = self.lock();
        state.revision += 1;
        state.oldest_valid_token = state.revision;
        let revision = state.revision;
        for collection in state.collections.values_mut() {
            collection.revision = revision;
        }
    }

    // MARK: Bodies

    fn token(revision: i64) -> String {
        format!("https://fake.test/sync/{revision}")
    }

    fn href(calendar: &str, name: Option<&str>) -> String {
        format!("{HOME_PATH}{calendar}/{}", name.unwrap_or(""))
    }

    fn response(href: &str, props: &[String]) -> String {
        format!(
            "<d:response><d:href>{href}</d:href><d:propstat><d:prop>{}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>",
            props.join("")
        )
    }

    fn multistatus(responses: &[String], sync_token: Option<String>) -> HttpResponse {
        let token = sync_token
            .map(|t| format!("<d:sync-token>{t}</d:sync-token>"))
            .unwrap_or_default();
        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><d:multistatus xmlns:d=\"DAV:\" xmlns:c=\"{}\" xmlns:cs=\"{}\" xmlns:a=\"{}\">{}{}</d:multistatus>",
            Dav::CALDAV,
            Dav::CALENDAR_SERVER,
            Dav::APPLE,
            responses.join(""),
            token
        );
        HttpResponse::new(
            207,
            BTreeMap::from([("Content-Type".to_string(), "application/xml".to_string())]),
            xml.into_bytes(),
        )
    }

    fn home_responses(state: &FakeState) -> Vec<String> {
        let mut responses = vec![
            Self::response(
                HOME_PATH,
                &["<d:resourcetype><d:collection/></d:resourcetype>".to_string()],
            ),
            Self::response(
                INBOX_PATH,
                &[
                    "<d:resourcetype><d:collection/><c:schedule-inbox/></d:resourcetype>"
                        .to_string(),
                ],
            ),
            Self::response(
                &format!("{HOME_PATH}outbox/"),
                &[
                    "<d:resourcetype><d:collection/><c:schedule-outbox/></d:resourcetype>"
                        .to_string(),
                ],
            ),
            Self::response(
                &format!("{HOME_PATH}notification/"),
                &[
                    "<d:resourcetype><d:collection/><cs:notification/></d:resourcetype>"
                        .to_string(),
                ],
            ),
        ];
        for name in state.collections.keys() {
            responses.push(Self::collection_response(state, name));
        }
        responses
    }

    fn collection_response(state: &FakeState, name: &str) -> String {
        let c = &state.collections[name];
        let mut props = vec![
            format!(
                "<d:resourcetype><d:collection/><c:calendar/>{}</d:resourcetype>",
                if c.subscribed { "<cs:subscribed/>" } else { "" }
            ),
            format!(
                "<d:displayname>{}</d:displayname>",
                DavXml::escape(&c.display_name)
            ),
            format!("<cs:getctag>ctag-{}</cs:getctag>", c.revision),
            format!(
                "<d:current-user-privilege-set>{}</d:current-user-privilege-set>",
                c.privileges
                    .iter()
                    .map(|p| format!("<d:privilege><d:{p}/></d:privilege>"))
                    .collect::<String>()
            ),
        ];
        if let Some(color) = &c.color {
            props.push(format!("<a:calendar-color>{color}</a:calendar-color>"));
        }
        if let Some(components) = &c.components {
            props.push(format!(
                "<c:supported-calendar-component-set>{}</c:supported-calendar-component-set>",
                components
                    .iter()
                    .map(|n| format!("<c:comp name=\"{n}\"/>"))
                    .collect::<String>()
            ));
        }
        if state.supports_sync {
            props.push(format!(
                "<d:sync-token>{}</d:sync-token>",
                Self::token(c.revision)
            ));
        }
        if let Some(zone) = &c.time_zone_id {
            let vcalendar = format!(
                "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTIMEZONE\r\nTZID:{zone}\r\nEND:VTIMEZONE\r\nEND:VCALENDAR\r\n"
            );
            props.push(format!(
                "<c:calendar-timezone>{}</c:calendar-timezone>",
                DavXml::escape(&vcalendar)
            ));
        }
        Self::response(&Self::href(name, None), &props)
    }

    fn report(state: &FakeState, calendar: &str, request: &HttpRequest) -> HttpResponse {
        let Some(root) = request
            .body
            .as_deref()
            .and_then(|body| XmlTree::parse(body).ok())
        else {
            return HttpResponse::new(400, BTreeMap::new(), Vec::new());
        };
        let c = &state.collections[calendar];
        if root.name == "sync-collection" {
            let invalid = HttpResponse::new(
                403,
                BTreeMap::new(),
                b"<d:error xmlns:d=\"DAV:\"><d:valid-sync-token/></d:error>".to_vec(),
            );
            if !state.supports_sync {
                return HttpResponse::new(403, BTreeMap::new(), Vec::new());
            }
            let mut since = 0;
            let text = root
                .child(Dav::DAV, "sync-token")
                .map(|t| t.trimmed_text().to_string())
                .unwrap_or_default();
            if !text.is_empty() {
                let value = text
                    .split('/')
                    .rfind(|p| !p.is_empty())
                    .and_then(|p| p.parse::<i64>().ok());
                match value {
                    Some(value) if value >= state.oldest_valid_token && value <= state.revision => {
                        since = value;
                    }
                    _ => return invalid,
                }
            }
            let mut responses: Vec<String> = c
                .resources
                .iter()
                .filter(|(_, r)| r.revision > since)
                .map(|(name, r)| {
                    Self::response(
                        &Self::href(calendar, Some(name)),
                        &[format!(
                            "<d:getetag>{}</d:getetag>",
                            DavXml::escape(&r.etag)
                        )],
                    )
                })
                .collect();
            responses.extend(c.removed.iter().filter(|(_, rev)| **rev > since).map(|(name, _)| {
                format!(
                    "<d:response><d:href>{}</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response>",
                    Self::href(calendar, Some(name))
                )
            }));
            return Self::multistatus(&responses, Some(Self::token(c.revision)));
        }
        let uid = root
            .first(Dav::CALDAV, "text-match")
            .map(|t| t.trimmed_text().to_string());
        let responses: Vec<String> = c
            .resources
            .iter()
            .filter(|(_, r)| match &uid {
                None => true,
                Some(uid) => r
                    .body
                    .split("\r\n")
                    .any(|line| line == format!("UID:{uid}")),
            })
            .map(|(name, r)| {
                Self::response(
                    &Self::href(calendar, Some(name)),
                    &[
                        format!("<d:getetag>{}</d:getetag>", DavXml::escape(&r.etag)),
                        format!(
                            "<c:calendar-data>{}</c:calendar-data>",
                            DavXml::escape(&r.body)
                        ),
                    ],
                )
            })
            .collect();
        Self::multistatus(&responses, None)
    }

    fn handle(&self, request: &HttpRequest) -> HttpResponse {
        let mut state = self.lock();
        state.log.push(request.clone());
        let status = |code: i64| HttpResponse::new(code, BTreeMap::new(), Vec::new());
        let expected = WebDavClient::basic_authorization(&WebDavCredentials {
            username: state.username.clone(),
            password: state.password.clone(),
        });
        if request.headers.get("Authorization") != Some(&expected) {
            return status(401);
        }
        let raw = DavUrl::raw_decoded_path(&request.url);
        let path = if raw.is_empty() { "/".to_string() } else { raw };
        let method = request.method.as_str();
        if let Some(index) = state
            .failures
            .iter()
            .position(|f| f.method == method && path.contains(&f.path_contains) && f.remaining > 0)
        {
            if state.failures[index].skip > 0 {
                state.failures[index].skip -= 1;
            } else {
                state.failures[index].remaining -= 1;
                return status(state.failures[index].status);
            }
        }
        if path.ends_with("/.well-known/caldav") {
            return match &state.well_known_location {
                None => status(404),
                Some(location) => HttpResponse::new(
                    301,
                    BTreeMap::from([("Location".to_string(), location.clone())]),
                    Vec::new(),
                ),
            };
        }
        if method == "OPTIONS" {
            let schedule = if state.auto_schedule {
                ", calendar-auto-schedule"
            } else {
                ""
            };
            return HttpResponse::new(
                200,
                BTreeMap::from([(
                    "DAV".to_string(),
                    format!("1, 2, 3, access-control, calendar-access{schedule}"),
                )]),
                Vec::new(),
            );
        }
        match (method, path.as_str()) {
            ("PROPFIND", "/") => {
                return Self::multistatus(
                    &[Self::response(
                        "/",
                        &[format!(
                            "<d:current-user-principal><d:href>{PRINCIPAL_PATH}</d:href></d:current-user-principal>"
                        )],
                    )],
                    None,
                );
            }
            ("PROPFIND", PRINCIPAL_PATH) => {
                let addresses: String = state
                    .user_addresses
                    .iter()
                    .map(|a| format!("<d:href>{}</d:href>", DavXml::escape(a)))
                    .collect();
                return Self::multistatus(
                    &[Self::response(
                        PRINCIPAL_PATH,
                        &[
                            format!(
                                "<c:calendar-home-set><d:href>{HOME_PATH}</d:href></c:calendar-home-set>"
                            ),
                            format!(
                                "<c:calendar-user-address-set>{addresses}</c:calendar-user-address-set>"
                            ),
                            format!(
                                "<c:schedule-inbox-URL><d:href>{INBOX_PATH}</d:href></c:schedule-inbox-URL>"
                            ),
                        ],
                    )],
                    None,
                );
            }
            ("PROPFIND", INBOX_PATH) => {
                let props: Vec<String> = state
                    .default_calendar
                    .iter()
                    .map(|d| {
                        format!(
                            "<c:schedule-default-calendar-URL><d:href>{HOME_PATH}{d}/</d:href></c:schedule-default-calendar-URL>"
                        )
                    })
                    .collect();
                return Self::multistatus(&[Self::response(INBOX_PATH, &props)], None);
            }
            ("PROPFIND", HOME_PATH) => {
                return Self::multistatus(&Self::home_responses(&state), None);
            }
            _ => {}
        }
        let Some(rest) = path.strip_prefix(HOME_PATH) else {
            return status(404);
        };
        let parts: Vec<String> = rest
            .split('/')
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect();
        let Some(calendar) = parts.first().cloned() else {
            return status(404);
        };
        if !state.collections.contains_key(&calendar) {
            return status(404);
        }
        if parts.len() == 1 {
            return match method {
                "REPORT" => Self::report(&state, &calendar, request),
                "PROPFIND" => {
                    Self::multistatus(&[Self::collection_response(&state, &calendar)], None)
                }
                _ => status(405),
            };
        }
        let name = parts[1].clone();
        let existing = state.collections[&calendar].resources.get(&name).cloned();
        let writable = state.collections[&calendar]
            .privileges
            .iter()
            .any(|p| p == "write");
        match method {
            "GET" => {
                let Some(existing) = existing else {
                    return status(404);
                };
                HttpResponse::new(
                    200,
                    BTreeMap::from([
                        ("ETag".to_string(), existing.etag),
                        ("Content-Type".to_string(), "text/calendar".to_string()),
                    ]),
                    existing.body.into_bytes(),
                )
            }
            "PUT" => {
                if !writable {
                    return status(403);
                }
                if request.headers.get("If-None-Match").map(String::as_str) == Some("*")
                    && existing.is_some()
                {
                    return status(412);
                }
                if let Some(matching) = request.headers.get("If-Match")
                    && Some(matching) != existing.as_ref().map(|e| &e.etag)
                {
                    return status(412);
                }
                state.revision += 1;
                let revision = state.revision;
                let etag = format!("\"e{revision}\"");
                let body =
                    String::from_utf8_lossy(request.body.as_deref().unwrap_or(&[])).into_owned();
                let sends = state.sends_etag_on_put;
                let collection = state.collections.get_mut(&calendar).unwrap();
                collection.resources.insert(
                    name.clone(),
                    Resource {
                        body,
                        etag: etag.clone(),
                        revision,
                    },
                );
                collection.removed.remove(&name);
                collection.revision = revision;
                HttpResponse::new(
                    if existing.is_none() { 201 } else { 204 },
                    if sends {
                        BTreeMap::from([("ETag".to_string(), etag)])
                    } else {
                        BTreeMap::new()
                    },
                    Vec::new(),
                )
            }
            "DELETE" => {
                if !writable {
                    return status(403);
                }
                let Some(existing) = existing else {
                    return status(404);
                };
                if let Some(matching) = request.headers.get("If-Match")
                    && *matching != existing.etag
                {
                    return status(412);
                }
                Self::remove_locked(&mut state, &calendar, &name);
                status(204)
            }
            _ => status(405),
        }
    }
}

#[async_trait]
impl HttpTransport for FakeCalDavServer {
    async fn send(&self, request: HttpRequest) -> Outcome<HttpResponse> {
        Ok(self.handle(&request))
    }
}

/// Hands out `uuid-1`, `uuid-2`, ... so tests know the names and UIDs a write will use.
#[derive(Default)]
pub(crate) struct UuidSequence {
    value: Mutex<i64>,
}

impl UuidSequence {
    pub(crate) fn next(&self) -> String {
        let mut value = self.value.lock().unwrap_or_else(PoisonError::into_inner);
        *value += 1;
        format!("uuid-{}", *value)
    }
}

// MARK: Harness

pub(crate) fn pacific() -> Zone {
    Zone::named("America/Los_Angeles").unwrap()
}

/// Calendars without `calendar-timezone` fall back to this zone in the harness (a different one, so tests can tell).
pub(crate) fn harness_default_zone() -> Zone {
    Zone::named("Europe/Berlin").unwrap()
}

/// A Pacific wall-clock time as an instant (Swift's `pt(_:_:_:_:_:)`).
pub(crate) fn pt(y: i64, mo: i64, d: i64, h: i64, mi: i64) -> Timestamp {
    calendar_core::civil::date_from(y, mo, d, h, mi, 0, &pacific()).unwrap()
}

/// `pt(y, mo, d)` with Swift's default hour of 10.
pub(crate) fn pt10(y: i64, mo: i64, d: i64) -> Timestamp {
    pt(y, mo, d, 10, 0)
}

pub(crate) fn september() -> DateInterval {
    DateInterval::new(pt(2026, 9, 1, 0, 0), pt(2026, 10, 1, 0, 0))
}

fn calendar_file(events: &[Vec<String>]) -> String {
    let mut lines: Vec<String> = vec![
        "BEGIN:VCALENDAR".into(),
        "VERSION:2.0".into(),
        "PRODID:-//Apple Inc.//iCloud//EN".into(),
    ];
    for event in events {
        lines.extend(event.iter().cloned());
    }
    lines.push("END:VCALENDAR".into());
    lines.join("\r\n") + "\r\n"
}

fn strings(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|l| l.to_string()).collect()
}

/// A single timed event in Los Angeles on 2026-09-10 09:00-10:00, with an Apple property the model does not cover.
pub(crate) fn single_ics() -> String {
    single_ics_with("single-uid", "Dentist", &[])
}

pub(crate) fn single_ics_with(uid: &str, title: &str, attendees: &[&str]) -> String {
    let mut lines = strings(&[
        "BEGIN:VEVENT",
        &format!("UID:{uid}"),
        "DTSTAMP:20260901T000000Z",
        "SEQUENCE:0",
        &format!("SUMMARY:{title}"),
        "DTSTART;TZID=America/Los_Angeles:20260910T090000",
        "DTEND;TZID=America/Los_Angeles:20260910T100000",
    ]);
    lines.extend(strings(attendees));
    lines.extend(strings(&[
        "X-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC",
        "END:VEVENT",
    ]));
    calendar_file(&[lines])
}

/// A weekly Tuesday 10:00-10:30 Los Angeles series from 2026-09-01: the 8th excluded, the 15th moved to Wednesday the
/// 16th at 14:00. With an organizer, the account is invited (NEEDS-ACTION); pass no organizer for no attendees.
pub(crate) fn weekly_ics() -> String {
    weekly_ics_with(
        "weekly-uid",
        "FREQ=WEEKLY;BYDAY=TU",
        Some("mailto:boss@example.test"),
    )
}

pub(crate) fn weekly_ics_by(organizer: Option<&str>) -> String {
    weekly_ics_with("weekly-uid", "FREQ=WEEKLY;BYDAY=TU", organizer)
}

pub(crate) fn weekly_ics_with(uid: &str, rule: &str, organizer: Option<&str>) -> String {
    let people: Vec<String> = match organizer {
        Some(o) => vec![
            format!("ORGANIZER;CN=Boss:{o}"),
            format!("ATTENDEE;CN=Boss;PARTSTAT=ACCEPTED:{o}"),
            "ATTENDEE;CN=Me;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:me@icloud.test".to_string(),
        ],
        None => Vec::new(),
    };
    let mut master = strings(&[
        "BEGIN:VEVENT",
        &format!("UID:{uid}"),
        "DTSTAMP:20260901T000000Z",
        "SEQUENCE:0",
        "SUMMARY:Team sync",
        "DTSTART;TZID=America/Los_Angeles:20260901T100000",
        "DTEND;TZID=America/Los_Angeles:20260901T103000",
        &format!("RRULE:{rule}"),
        "EXDATE;TZID=America/Los_Angeles:20260908T100000",
    ]);
    master.extend(people.clone());
    master.extend(strings(&[
        "X-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC",
        "END:VEVENT",
    ]));
    let mut moved = strings(&[
        "BEGIN:VEVENT",
        &format!("UID:{uid}"),
        "DTSTAMP:20260901T000000Z",
        "RECURRENCE-ID;TZID=America/Los_Angeles:20260915T100000",
        "SUMMARY:Team sync (moved)",
        "DTSTART;TZID=America/Los_Angeles:20260916T140000",
        "DTEND;TZID=America/Los_Angeles:20260916T143000",
    ]);
    moved.extend(people);
    moved.push("END:VEVENT".to_string());
    calendar_file(&[master, moved])
}

pub(crate) const DEFAULT_ADDRESSES: [&str; 2] = [
    "mailto:me@icloud.test",
    "urn:uuid:11111111-2222-3333-4444-555555555555",
];

pub(crate) struct CalDavHarness {
    pub server: Arc<FakeCalDavServer>,
    pub credentials: Arc<InMemoryCredentialStore>,
    pub sync: Arc<InMemorySyncStateStore>,
    pub uuids: Arc<UuidSequence>,
    pub now: TestNow,
}

/// A sleeper that returns at once (Swift's `{ _ in }`). The polling tests need no real time.
pub(crate) fn instant_sleeper() -> Sleeper {
    Arc::new(|_seconds| Box::pin(async { Ok(()) }))
}

impl CalDavHarness {
    pub(crate) fn new() -> CalDavHarness {
        CalDavHarness {
            server: Arc::new(FakeCalDavServer::new()),
            credentials: Arc::new(InMemoryCredentialStore::new()),
            sync: Arc::new(InMemorySyncStateStore::new()),
            uuids: Arc::new(UuidSequence::default()),
            now: TestNow::new(Timestamp::from_second(1_790_000_000).unwrap()),
        }
    }

    pub(crate) fn account(
        &self,
        auto_schedule: bool,
        user_addresses: &[&str],
    ) -> CalDavAccountConfig {
        CalDavAccountConfig::new(
            fake_server_url(),
            "me@icloud.test",
            UrlString::parse("https://caldav.icloud.com/123/principal/").unwrap(),
            fake_home_url(),
            user_addresses.iter().map(|a| a.to_string()).collect(),
            auto_schedule,
        )
    }

    pub(crate) async fn source(&self) -> Arc<CalDavCalendarSource> {
        self.source_with(true, &DEFAULT_ADDRESSES, None).await
    }

    pub(crate) async fn source_via(
        &self,
        transport: Arc<dyn HttpTransport>,
    ) -> Arc<CalDavCalendarSource> {
        self.source_with(true, &DEFAULT_ADDRESSES, Some(transport))
            .await
    }

    pub(crate) async fn source_with(
        &self,
        auto_schedule: bool,
        user_addresses: &[&str],
        transport: Option<Arc<dyn HttpTransport>>,
    ) -> Arc<CalDavCalendarSource> {
        self.credentials
            .set_secrets(
                BTreeMap::from([
                    ("username".to_string(), "me@icloud.test".to_string()),
                    ("password".to_string(), "app-pass-1234".to_string()),
                ]),
                "c1",
            )
            .await
            .unwrap();
        let account = self.account(auto_schedule, user_addresses);
        let mut connection = Connection::new("icloud", "c1", "me@icloud.test");
        connection.config = account.config();
        let store = self.credentials.clone();
        let credentials: CredentialProvider = Arc::new(move || {
            let store = store.clone();
            Box::pin(async move {
                let secrets = store.secrets("c1").await?.unwrap_or_default();
                Ok(WebDavCredentials {
                    username: secrets.get("username").cloned().unwrap_or_default(),
                    password: secrets.get("password").cloned().unwrap_or_default(),
                })
            })
        });
        let transport: Arc<dyn HttpTransport> = transport.unwrap_or_else(|| self.server.clone());
        let client = WebDavClient::new(transport, "icloud.com", credentials);
        let uuids = self.uuids.clone();
        let mut settings = CalDavSourceSettings::new(
            connection,
            account,
            client,
            CalendarProvider::icloud(),
            self.sync.clone(),
            ChangeMonitor::new(60.0, 900.0, instant_sleeper()),
            self.now.provider(),
            harness_default_zone(),
        );
        settings.make_uuid = Arc::new(move || uuids.next());
        CalDavCalendarSource::new(settings)
    }
}

/// Answers `prompt_credentials` with fixed values and records the fields it was shown.
pub(crate) struct StubInteraction {
    answers: Mutex<Vec<BTreeMap<String, String>>>,
}

impl StubInteraction {
    pub(crate) fn new(answers: Vec<BTreeMap<String, String>>) -> StubInteraction {
        StubInteraction {
            answers: Mutex::new(answers),
        }
    }

    pub(crate) fn one(pairs: &[(&str, &str)]) -> StubInteraction {
        StubInteraction::new(vec![
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        ])
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
        _fields: &[CredentialField],
    ) -> Outcome<BTreeMap<String, String>> {
        let mut answers = self.answers.lock().unwrap_or_else(PoisonError::into_inner);
        if answers.is_empty() {
            return Err(Failure::Cancelled);
        }
        Ok(answers.remove(0))
    }
}
