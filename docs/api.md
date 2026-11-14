# calendar-connectors: Rust API reference

This is the reference for the Rust crates on the `v2` branch (the `2.x` line). The `1.x` line is a Swift package on
the `1.x` branch; it is described by [behaviour-contract-1x.md](behaviour-contract-1x.md) and is not covered here.
For the full item-level reference build the rustdoc: `cargo doc --no-deps --workspace --open`. Every public item
has documentation, and the build fails (`missing_docs = "deny"`) if one loses it.

Contents: 1 What it is. 2 Install. 3 Crates. 4 The shape of an integration. 5 Examples. 6 Errors. 7 Threading and
async. 8 What a host must supply. 9 Differences from the Swift line. 10 Versions and the compatibility promise.
11 Platforms. 12 Not provided.

## 1. What it is

A portable library that reads calendars (and, for Google, Microsoft and CalDAV, writes them) through Google
Calendar, Microsoft Graph, CalDAV (including iCloud) and iCalendar subscription links, behind one event model. It is
the connector layer of [TimeTug](https://github.com/binary-companion/timetug), published so other programs can use it.

It is deliberately small on the outside:

- It has **no HTTP client**, no keychain, no sign-in window, no clock and no filesystem policy. The host supplies
  them through traits (section 8). That is what lets the same crates run on Windows, macOS, iOS, Android and Linux.
- It is `#![forbid(unsafe_code)]` in every crate. Parsers (iCalendar, WebDAV XML, provider JSON) return errors for
  any input and never panic.
- It reads no environment variables and opens no network connection of its own.

## 2. Install

**The crates are not on crates.io**, and nothing in the repository promises that they will be: every crate has
`publish = false`, and the crate `version` fields read `0.0.0`. The version of the library is the **git tag** of the
repository. Until a stable `2.0.0` tag exists, the latest tag is the prerelease `2.0.0-alpha.1`. Depend on a tag, one
line per crate (all crates come from the same repository and the same tag):

```toml
[dependencies]
calendar-core = { git = "https://github.com/binary-companion/calendar-connectors", tag = "2.0.0-alpha.1" }
calendar-ical-subscription = { git = "https://github.com/binary-companion/calendar-connectors", tag = "2.0.0-alpha.1" }
```

(`cargo` resolved and compiled exactly this pair when this page was written.) Use the same tag for every
`calendar-*` crate you depend on. Check the repository's tags or releases page for the newest tag; do not depend on a
branch. `timetug-shared` consumes the library the same way.

Requirements: edition 2024 and Rust 1.99.1 or newer (`rust-version`; the repository pins 1.99.1 in
`rust-toolchain.toml`). There are no cargo features. Dependencies are exact-pinned in the workspace `Cargo.toml`:
`serde`, `serde_json`, `jiff` (time zones; a bundled zone database on Windows, iOS and Android, the system one on
macOS and Linux), `quick-xml`, `sha2`, `base64`, `url`, `uuid`, `getrandom`, `thiserror`, `async-trait` and
`tokio` with only `rt`, `time`, `sync` and `macros`. `cargo-deny` checks the licences (MIT, Apache-2.0, BSD, ISC,
Unicode and Zlib only).

To work on the library itself: `cargo test --workspace` (this also builds and runs the examples below).

## 3. Crates

Add only what you need. Dependencies point downward: `calendar-core` depends on nothing in the repository.

| Crate | What it is for | Main public items | Depends on |
|---|---|---|---|
| `calendar-core` | The model and every seam. Start here. | `model` (`CalendarEvent`, `CalendarDescriptor`, `Attendee`, `Reminder`, `ConferenceInfo`, ...); `source_types` (`CalendarSource`, `PollingCalendarSource`, `SeriesSource`, `SourceError`, `CalendarChange`, `SourceCapabilities`); `connection` (`Connection`, `ConnectorKind`, `ConnectorRegistry`, `CredentialStore`, `SyncStateStore`, `AuthorizationInteraction`, in-memory stores); `http` (`HttpTransport`, `HttpRequest`, `HttpResponse`); `failure` (`Failure`, `Outcome`); `time` (`Zone`, `DateInterval`, `Now`, `Sleeper`); `recurrence` (`RecurrenceRule`, `RecurrenceSet`, `CalendarSeries`); `write` (`EventDraft`, `EventPatch`, `WritableCalendarSource`, `WriteError`); `external_tokens`; `diagnostic_log`; `file_connection_store`, `file_sync_state_store`; `change_monitor` | external crates only |
| `calendar-oauth` | OAuth 2.0 authorization-code flow with PKCE, token refresh, a coalescing access-token cache | `OAuthClient`, `OAuthConfig`, `OAuthTokens`, `AccessTokenProvider`, `ExternalAccessTokenProvider`, `OAuthConnections`, `pkce` | `calendar-core` |
| `calendar-ical` | iCalendar (RFC 5545): parser and serializer, `VEVENT`/`VALARM` mapping, `VTIMEZONE` reading and writing, series editing | `ICalParser`, `ICalSerializer`, `ICalComponent`, `EventReader`, `EventWriter`, `SeriesEditor`, `TimeZoneResolver`, `VTimeZoneWriter` | `calendar-core` |
| `calendar-google` | Google Calendar (REST v3) connector, read and write | `GoogleConnectorKind`, `GoogleOAuthConfig`, `GoogleCalendarSource` | `calendar-core`, `calendar-oauth` |
| `calendar-microsoft` | Microsoft 365 / Outlook / Exchange through Microsoft Graph v1.0, read and write | `MicrosoftConnectorKind`, `MicrosoftOAuthConfig`, `MicrosoftBrokeredConnectorKind`, `MicrosoftCalendarSource` | `calendar-core`, `calendar-oauth` |
| `calendar-caldav` | CalDAV and iCloud, read and write, sync tokens or ctag, series editing | `ICloudConnectorKind`, `CalDavConnectorKind`, `CalDavCalendarSource` | `calendar-core`, `calendar-ical` |
| `calendar-ical-subscription` | One read-only calendar from a subscription link (`webcal://` or `https://`) | `ICalSubscriptionKind`, `ICalSubscriptionSource`, `RetentionWindow` | `calendar-core`, `calendar-ical` |
| `calendar-test-support` | Fakes and conformance checks for tests: a scripted `HttpTransport`, a manual clock, a fake writable source, a diagnostic collector | `fake_transport` (`FakeTransport`, `TestNow`), `fake_writable_source`, `fake_external_token_source`, `collecting_diagnostic_log`, and the `*_conformance` helpers | `calendar-core` |

The authoritative dependency table is `ALLOWED` in `scripts/ci/check-crate-architecture.py`; CI fails if a crate
depends on one that is not listed.

The provider crates export the kind and the source and keep everything else private.

## 4. The shape of an integration

1. **Implement the seams** you need: `HttpTransport` always; `CredentialStore`, `SyncStateStore` (or use the
   in-memory or file-backed ones from `calendar-core`); `AuthorizationInteraction` for sign-in; `DiagnosticLog`
   optionally.
2. **Build a connector kind** for each provider you offer and register it in a `ConnectorRegistry`. A kind is
   configuration plus the seams: `GoogleConnectorKind::new(GoogleOAuthConfig::new(client_id, client_secret),
   transport, now, sleep)`; `MicrosoftConnectorKind::new(MicrosoftOAuthConfig::new(client_id), transport, now,
   sleep)`; `ICloudConnectorKind::new(transport, now, sleep, default_zone)` and `CalDavConnectorKind::new(...)` with
   the same arguments; `ICalSubscriptionKind::new(transport, now, sleep, default_zone)`;
   `MicrosoftBrokeredConnectorKind::new(token_source, transport, now, sleep)` for a host whose operating system
   supplies the Microsoft tokens. Optional builders: `with_poll_interval`, `with_retention`, `with_diagnostics`.
   The OAuth client ids are yours; the library ships none.
3. **Sign in**: `kind.authorize(&interaction, &credentials).await` returns a `Connection`: the non-secret
   description of the account (kind id, a unique connection id, a display name and string settings). Persist it
   (it is `serde`-serializable; `FileConnectionStore` does that for you). Secrets (refresh tokens, passwords, feed
   links) go to your `CredentialStore` and nowhere else. `kind.reauthorize(...)` repairs an account whose sign-in
   expired and keeps its `connection_id`.
4. **Make a source**: `kind.make_source(&connection, credentials, sync_state)` returns an
   `Arc<dyn CalendarSource>`.
5. **Read**: `source.calendars().await` and `source.events_in(DateInterval).await`. Recurring events come back
   already expanded into instances. `source.capabilities()` says whether it can write, which fields it fills and how
   it detects change.
6. **Stay current**: `source.changes()` returns a receiver of `CalendarChange` values (`CalendarsChanged`,
   `EventsChanged { calendar_ids }`, `SourceFailed(error)`); polling sources also expose `as_polling()` for a single
   cheap `check_for_changes()`.
7. **Write** (if `as_writable()` is `Some`): `create`, `update`, `delete` and `respond` on
   `WritableCalendarSource`, with an `EventDraft` or `EventPatch`, a `RecurrenceScope` and a `NotifyPolicy`. A write
   returns the event in the provider's resulting form.

Time is always explicit: instants are `jiff::Timestamp`, zones are `calendar_core::Zone` (a name such as
`Europe/Berlin`, or a fixed offset), and all-day events use the canonical form (midnight of the first day in the
event's zone to midnight after the last day, end exclusive). The library never asks the host for "now"; you pass a
`Now` closure when you build a kind.

## 5. Examples

Every example below is a file in the repository that `cargo test --workspace` compiles **and runs** (each crate has
a `tests/examples.rs` that calls the example's `main`). None uses the network or an account. The listings are
the files as they are.

### 5.1 Read an iCalendar subscription (a whole integration)

This is the complete shape: a host transport, a scripted sign-in, the registry, `authorize`, `make_source`,
`events_in`, and error handling. File: `crates/calendar-ical-subscription/examples/read_feed.rs`.

```rust
//! Signs in to an iCalendar subscription link and reads its events, with a scripted host. This is the whole shape
//! of a connector integration: a host supplies HTTP, credential and sync-state storage and the sign-in interaction,
//! registers a connector kind, and reads events from the source the kind makes.
//!
//! The "network" here is a function that answers with a fixed feed, so the example needs no account and no
//! connection. Run it with `cargo run -p calendar-ical-subscription --example read_feed`.

#![allow(clippy::print_stdout, clippy::expect_used)]

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use calendar_core::connection::{
    AuthorizationInteraction, ConnectorRegistry, CredentialField, InMemoryCredentialStore,
    InMemorySyncStateStore, OAuthRedirectSession,
};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::{HttpRequest, HttpResponse, HttpTransport};
use calendar_core::source_types::SourceError;
use calendar_core::time::{DateInterval, Now, Sleeper, Zone};
use calendar_ical_subscription::ICalSubscriptionKind;
use jiff::Timestamp;

const FEED: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Example//Docs//EN\r\n\
    X-WR-CALNAME:Example meetups\r\nBEGIN:VEVENT\r\nUID:docs-1@example.test\r\nDTSTAMP:20260901T000000Z\r\n\
    DTSTART:20260910T190000Z\r\nDTEND:20260910T210000Z\r\nSUMMARY:Board games night\r\nEND:VEVENT\r\n\
    END:VCALENDAR\r\n";

/// The host's HTTP. A real host wraps its HTTP client here. It must return every status as a response, must not
/// follow redirects for this connector, and must not cache (see `docs/host-http-contract.md`).
struct FixedTransport;

#[async_trait]
impl HttpTransport for FixedTransport {
    async fn send(&self, request: HttpRequest) -> Outcome<HttpResponse> {
        if request.url.host().as_deref() == Some("calendar.example.test") {
            Ok(HttpResponse::new(
                200,
                BTreeMap::new(),
                FEED.as_bytes().to_vec(),
            ))
        } else {
            // A transport failure is a `Network` error; a cancelled request would be `Failure::Cancelled`.
            Err(Failure::Source(SourceError::Network(
                "no route".to_string(),
            )))
        }
    }
}

/// The host's sign-in UI. A real host shows `fields` and returns what the user typed.
struct ScriptedSignIn(&'static str);

#[async_trait]
impl AuthorizationInteraction for ScriptedSignIn {
    async fn begin_oauth_redirect(&self) -> Outcome<Box<dyn OAuthRedirectSession>> {
        Err(Failure::Cancelled) // this kind uses a password-style prompt, not an OAuth redirect
    }

    async fn prompt_credentials(
        &self,
        fields: &[CredentialField],
    ) -> Outcome<BTreeMap<String, String>> {
        let mut values = BTreeMap::new();
        for field in fields {
            if field.key == "link" {
                values.insert(field.key.clone(), self.0.to_string());
            }
        }
        Ok(values)
    }
}

/// Runs the example (public so that the crate's test can call it).
#[tokio::main(flavor = "current_thread")]
pub async fn main() -> Outcome<()> {
    // The library reads no clock and sleeps only through what the host passes.
    let now: Now = Arc::new(|| "2026-09-01T12:00:00Z".parse().expect("a valid instant"));
    let sleep: Sleeper = Arc::new(|_seconds| Box::pin(async { Ok(()) }));
    let kind = ICalSubscriptionKind::new(Arc::new(FixedTransport), now, sleep, Zone::utc());

    // Kinds are registered once; a host looks them up by `Connection::kind_id` later.
    let mut registry = ConnectorRegistry::new();
    registry.register(Arc::new(kind));
    let kind = registry
        .kind(ICalSubscriptionKind::KIND_ID)
        .expect("registered above");

    // Sign in: the secret (the link) goes to the credential store, the `Connection` is non-secret and the host
    // persists it.
    let credentials = Arc::new(InMemoryCredentialStore::new());
    let connection = kind
        .authorize(
            &ScriptedSignIn("webcal://calendar.example.test/feed/PRIVATE-TOKEN.ics"),
            credentials.as_ref(),
        )
        .await?;
    assert_eq!(
        connection.display_name,
        "Example meetups (calendar.example.test)"
    );

    // Make the source and read.
    let source = kind.make_source(
        &connection,
        credentials,
        Arc::new(InMemorySyncStateStore::new()),
    )?;
    let window = DateInterval::new(
        "2026-09-01T00:00:00Z"
            .parse::<Timestamp>()
            .expect("a valid instant"),
        "2026-10-01T00:00:00Z"
            .parse::<Timestamp>()
            .expect("a valid instant"),
    );
    let events = source.events_in(window).await?;
    assert_eq!(events.len(), 1);
    println!("{}: {}", events[0].start, events[0].title);

    // Errors: every fallible call returns `Outcome<T>`, a `Result` whose error is `Failure`. Match on it to decide
    // what to tell the user and whether to retry.
    let unreachable = ScriptedSignIn("https://elsewhere.example.test/feed.ics");
    match kind
        .authorize(&unreachable, &InMemoryCredentialStore::new())
        .await
    {
        Err(Failure::Source(SourceError::Network(_))) => println!("offline: try again later"),
        Err(Failure::Cancelled) => println!("the user cancelled"),
        Err(other) => println!("failed: {other}"),
        Ok(_) => unreachable!("the host has no route to that address"),
    }
    Ok(())
}
```

Run it: `cargo run -p calendar-ical-subscription --example read_feed`.

### 5.2 Expand a recurrence rule

File: `crates/calendar-core/examples/expand_recurrence.rs`. `RecurrenceSet::occurrences` returns the starts of the
occurrences that overlap a window; a series that is old costs the same as a new one because expansion skips whole
periods before the window.

```rust
//! Expands a weekly recurrence rule into the occurrences that fall in a window. No network, no clock.
//!
//! Run it with `cargo run -p calendar-core --example expand_recurrence`.

#![allow(clippy::print_stdout, clippy::expect_used)]

use calendar_core::recurrence::rule::RecurrenceRule;
use calendar_core::recurrence::set::RecurrenceSet;
use calendar_core::time::{DateInterval, Zone};
use jiff::Timestamp;

/// Runs the example (public so that the crate's test can call it).
pub fn main() {
    let zone = Zone::named("Europe/Berlin").expect("a zone every platform knows");
    let rule = RecurrenceRule::from_rrule("FREQ=WEEKLY;BYDAY=TU;COUNT=10", Some(&zone))
        .expect("a well-formed rule");
    let set = RecurrenceSet::new(vec![rule]);

    // The series starts on Tuesday 2026-09-01 09:00 Berlin time (07:00 UTC) and each occurrence lasts an hour.
    let anchor: Timestamp = "2026-09-01T07:00:00Z".parse().expect("a valid instant");
    let window = DateInterval::new(
        "2026-09-01T00:00:00Z".parse().expect("a valid instant"),
        "2026-09-30T00:00:00Z".parse().expect("a valid instant"),
    );

    let expansion = set.occurrences(
        anchor,
        3600.0,
        &zone,
        false,
        &window,
        RecurrenceSet::DEFAULT_OCCURRENCE_LIMIT,
    );
    assert!(!expansion.truncated && !expansion.has_unreadable_rule);
    // Tuesdays 1, 8, 15, 22 and 29 September; the daylight-saving offset is the same all month.
    assert_eq!(expansion.starts.len(), 5);
    for start in &expansion.starts {
        println!("{start}");
    }
}
```

### 5.3 Parse and write iCalendar text

File: `crates/calendar-ical/examples/parse_ics.rs`.

```rust
//! Parses iCalendar text into a component tree, reads two properties and writes the text back.
//!
//! Run it with `cargo run -p calendar-ical --example parse_ics`.

#![allow(clippy::print_stdout, clippy::expect_used)]

use calendar_ical::content_line::{ICalParser, ICalSerializer};

/// Runs the example (public so that the crate's test can call it).
pub fn main() {
    let text = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Example//Docs//EN\r\n\
                BEGIN:VEVENT\r\nUID:docs-1@example.test\r\nDTSTAMP:20260901T000000Z\r\n\
                DTSTART:20260910T190000Z\r\nDTEND:20260910T210000Z\r\nSUMMARY:Board games night\r\n\
                END:VEVENT\r\nEND:VCALENDAR\r\n";

    // A parser returns an error for malformed text, never a panic.
    let calendar = ICalParser::parse_text(text).expect("well-formed iCalendar");
    let events = calendar.components_named("VEVENT");
    assert_eq!(events.len(), 1);
    let summary = events[0].property("SUMMARY").map(|p| p.text());
    assert_eq!(summary.as_deref(), Some("Board games night"));
    println!("{}", summary.unwrap_or_default());

    let round_trip = ICalSerializer::serialize(&calendar);
    assert!(round_trip.contains("SUMMARY:Board games night"));

    let broken = ICalParser::parse_text("BEGIN:VCALENDAR\r\nthis line has no colon\r\n");
    assert!(broken.is_err());
}
```

### 5.4 Register the Google kind

Signing in to Google needs a browser and an account, so this example stops at configuration. File:
`crates/calendar-google/examples/register_kind.rs`.

```rust
//! Builds the Google connector kind and registers it. Sign-in and reading need a real Google account and a host that
//! can open a browser, so this example stops at what a host does at start-up; the sign-in flow is in
//! `docs/api.md`. Run it with `cargo run -p calendar-google --example register_kind`.

#![allow(clippy::print_stdout, clippy::expect_used)]

use std::sync::Arc;

use async_trait::async_trait;
use calendar_core::connection::{AuthorizationMethod, ConnectorRegistry, Platform};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::{HttpRequest, HttpResponse, HttpTransport};
use calendar_core::source_types::SourceError;
use calendar_core::time::{Now, Sleeper};
use calendar_google::{GoogleConnectorKind, GoogleOAuthConfig};

/// A host's transport wraps its HTTP client; this one has no network.
struct OfflineTransport;

#[async_trait]
impl HttpTransport for OfflineTransport {
    async fn send(&self, _request: HttpRequest) -> Outcome<HttpResponse> {
        Err(Failure::Source(SourceError::Network("offline".to_string())))
    }
}

/// Runs the example (public so that the crate's test can call it).
pub fn main() {
    // The OAuth client id and secret come from the host's own configuration, never from this library.
    let config = GoogleOAuthConfig::new("client-id.example", "client-secret.example");
    let now: Now = Arc::new(jiff::Timestamp::now);
    let sleep: Sleeper = Arc::new(|seconds| {
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_secs_f64(seconds)).await;
            Ok(())
        })
    });
    let kind = GoogleConnectorKind::new(config, Arc::new(OfflineTransport), now, sleep)
        .with_poll_interval(120.0);

    let mut registry = ConnectorRegistry::new();
    registry.register(Arc::new(kind));

    // A host offers the kinds that work on its platform.
    let offered = registry.kinds_for(Platform::current());
    println!("{} kind(s) available here", offered.len());
    let google = registry.kind("google").expect("registered above");
    assert!(matches!(google.authorization(), AuthorizationMethod::Oauth));
}
```

### 5.5 Testing your host without a network

`calendar-test-support` has `FakeTransport` (route a URL substring to scripted `HttpResponse` values or errors, then
inspect `requests()`), `TestNow` (a clock you advance) and `FakeWritableSource`. The tests of the provider crates under
`crates/*/src/tests/` are working examples of using them; for example `calendar-ical-subscription/src/tests/kind_tests.rs`
signs in against a scripted feed.

## 6. Errors

Every fallible operation returns `Outcome<T>`, which is `Result<T, Failure>`:

| `Failure` variant | Meaning | What to do |
|---|---|---|
| `Source(SourceError)` | A connector error (below) | Depends on the case |
| `Write(WriteError)` | A write was refused or failed | Show the reason; `Conflict` means someone else changed a field the patch touches |
| `Cancelled` | The operation was cancelled (the user closed sign-in, or your transport reported a cancelled request) | Stop quietly; it is never a network error |
| `Host(String)` | Your own seam (store, interaction) failed and said so as text | Log it, report it |

`SourceError`: `AuthExpired` (sign in again with `reauthorize`; polling stops and a `SourceFailed(AuthExpired)`
change is delivered), `Network(String)` (transport failure; retry later, polling backs off by itself),
`RateLimited { retry_after }` (seconds, when the provider said), `Server { status }`, `InvalidResponse(String)`,
`NeedsPermission` (the account has not granted access) and `NotFound`.

`WriteError`: `Unsupported { fields }` (this source cannot store those fields; nothing was changed), `Conflict
{ fields }` (someone else changed a field the patch touches), `NotFound`, `Forbidden`, `Invalid(String)`,
`AlreadyExists(event)` (the draft's UID is already on the calendar; the stored copy is returned) and
`Partial(String)` (a multi-step write stopped half way).

`Failure` implements `std::error::Error` and `Display` (through `thiserror`). The connectors keep tokens, passwords,
authorization codes and subscription links out of their errors and `Debug` output (see each crate's documentation). Parse functions with their own error types
(`RecurrenceRule::from_rrule` returns `RecurrenceParseError`, `ICalParser` returns `ICalError`) are plain `Result`s.

## 7. Threading and async

- Every trait in the library is `Send + Sync`; the async methods use `#[async_trait]` (boxed futures). Sources,
  stores and the transport are shared as `Arc<dyn ...>` and can be called from any task or thread at the same time.
- The async methods are plain futures and do not require a particular runtime, but the polling stream does:
  `source.changes()` (and `ChangeMonitor::changes`) **spawns a polling task and must be called inside a Tokio
  runtime**; outside one the receiver finishes immediately. Polling stops when you drop the receiver. A
  single-threaded (`current_thread`) runtime is enough, and is what the TimeTug engine uses.
- The library enables only Tokio's `rt`, `time`, `sync` and `macros` features. It never uses `net`, `fs` or
  `process`, and `tokio::time` is called only by the engine, not by these crates: waiting goes through the `Sleeper`
  you pass in (a closure from seconds to a future), so tests can run in zero time and you control the timer.
- Interior state is guarded by `std` mutexes (a poisoned mutex is recovered, not propagated). The access-token cache
  uses one `tokio::sync::Mutex` held across the refresh request on purpose, so concurrent callers share one refresh.
- There are no global variables and no background threads other than the tasks described above.

## 8. What a host must supply

| Seam | Trait or type | Notes |
|---|---|---|
| HTTP | `HttpTransport::send(HttpRequest) -> Outcome<HttpResponse>` | Return every status (4xx and 5xx too) as a response; map a transport failure to `Failure::Source(SourceError::Network(..))` and a cancellation to `Failure::Cancelled`; no cache, no cookies, finite timeouts; do not follow redirects for CalDAV, iCloud and subscriptions (the library checks each hop); never put tokens or links in logs. The complete obligations are in [host-http-contract.md](host-http-contract.md). |
| Secrets | `CredentialStore` | `InMemoryCredentialStore` for tests. A real host uses the operating system's secret store. |
| Sync state | `SyncStateStore` | Per-connection sync tokens. `InMemorySyncStateStore`, `FileSyncStateStore`. |
| Accounts list | `FileConnectionStore` | Optional JSON file store of `Connection` values. |
| Sign-in UI | `AuthorizationInteraction`, `OAuthRedirectSession` | Open the browser and capture the redirect, or prompt for the listed fields. |
| Clock | `Now` = `Arc<dyn Fn() -> Timestamp>` | Passed to each kind. |
| Timer | `Sleeper` | Passed to each kind. |
| Default zone | `Zone` | For kinds that need a zone when the data carries none. |
| Diagnostics | `DiagnosticLog` | Optional; events never contain secrets. |
| System accounts | `ExternalTokenSource` | Only for `MicrosoftBrokeredConnectorKind`. |

## 9. Differences from the Swift line

- **Async.** Swift `async throws` becomes `async fn` returning `Outcome<T>`; Swift actors and `Sendable` become
  `Send + Sync` traits behind `Arc`.
- **Errors.** Swift's `throws` is `Failure`; `CancellationError` is `Failure::Cancelled`, never a network error.
- **Names.** Rust style: `makeSource` is `make_source`, `ConnectionID` is `ConnectionId`.
- **Removed seams.** There is no `SHA256Hashing` (the `sha2` crate is used and a golden vector pins the result) and
  no `URLSessionTransport`: the host owns HTTP.
- **No non-finite times.** `jiff::Timestamp` cannot be NaN or infinite, so `EventTiming::validate` has no finiteness
  check; its other rules are unchanged.
- **Latest instant.** `jiff::Timestamp` ends on 9999-12-30T22:00:00Z; an event in the last two hours of year 9999
  is not representable (the Google mapper drops it).
- **Time zones.** Zones are `jiff` zones; the zone chosen for a `VTIMEZONE` uses a captured Foundation identifier
  list, so the choice is the same on every platform.
- **Persisted data.** `connections.json` and `sync-state.json` keep the 1.x format byte for byte; files written by
  1.x load unchanged (`fixtures/persisted-formats` is frozen and replayed by the tests). A 1.x reader of files written
  by 2.x is not promised.
- Behaviour is meant to be identical: every 1.x Swift test has a Rust counterpart (`docs/rust-port/test-parity.md`
  lists the 12 that are not applicable and why), and golden vectors from the Swift code (`vectors/`) pin recurrence,
  time zone, PKCE and iCalendar edge cases.

## 10. Versions and the compatibility promise

- **Two lines.** `1.x` is the Swift package on the `1.x` branch (and the tags `1.0.0` to `1.2.1`). `2.x` is this Rust
  workspace on `v2`; after the cutover it becomes `main`. The line is chosen by the tag you depend on.
- **Tags.** A version is a git tag `X.Y.Z`; prereleases of the Rust line are `2.0.0-alpha.N`, cut by hand from
  `v2`. Published tags are immutable. Today only `2.0.0-alpha.1` exists. An alpha makes no stability promise.
- **From `2.0.0` on (the rule the repository enforces).** Within a major version a release must drop into an
  existing host with no change to the host: additions are allowed; removing, renaming or retyping a public item,
  changing a value or a meaning, or modifying a persisted-format fixture needs a new major version
  (the pull-request label `semver:major`). Breaking changes to this document count as breaking changes.
- **What is checked automatically today.** The persisted-format fixtures are strictly add-only (CI `compat` job).
  There is **no automatic Rust API diff yet** (for example `cargo semver-checks`); the repository's `AGENTS.md` says
  it will be added with the first stable 2.x tag. Until then the promise on the Rust API is enforced by review.
- **The crate `version` fields are `0.0.0` and unrelated to the tag.** Do not infer compatibility from them.
- **Toolchain.** The pinned toolchain is raised only by a reviewed pull request, together with `timetug-shared`.
  Treat a rise of `rust-version` as something to read in the release notes.

## 11. Platforms

The crates build and pass their tests on macOS, Linux, Windows x64 and Windows ARM64 (all four run in CI on every
pull request). CI also builds release libraries for the other Apple targets (macOS x86_64, iOS device and iOS
simulator). The toolchain file also lists Linux ARM64 and Android (`aarch64-linux-android`) targets; CI for this repository
does not build them, so treat those as untested here.

## 12. Not provided

No HTTP client, no OAuth browser or loopback listener, no credential storage, no UI strings, no push
notifications or webhooks (change detection is polling by sync token), no Apple Calendar (EventKit) connector (that
adapter stays in the Mac app), and no crates.io release. Hosts in other languages should use the engine in
[timetug-shared](https://github.com/binary-companion/timetug-shared), which wraps this library behind a C interface.
