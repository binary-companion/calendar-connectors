# Calendar connectors, Phase 1: portable library + read-only Google connector

Status: draft for review. Approved roadmap: this is Phase 1 of 4 (see "Roadmap"). Nothing in `Apps/macOS`, `TimeTugCore` or `EventKitSource` changes in this phase.

## Goal
A portable Swift package, `Packages/CalendarConnectors`, that defines a provider-neutral calendar model and source interface, a standard way to authorize and reconnect a connector, a change-notification mechanism, and one real connector: read-only Google Calendar. TimeTug does not use it yet (Phase 2).

## Non-goals (Phase 1)
- Writing events, RSVP, `EventDraft`/`EventPatch`, `RecurrenceRule` (Phase 3). Reads use Google's `singleEvents=true` expansion, so no read path produces a recurrence rule; instances carry `seriesID` only.
- Any TimeTug integration, Accounts UI, Keychain code, OAuth browser/loopback listener (all Phase 2; they are the host app's part of the auth flow).
- Microsoft, CalDAV (Phase 4 and later). Push/webhooks (a seam only).
- A live Google smoke test. Phase 1 is verified by unit tests against a fake transport; the first live run is in Phase 2 through the app.

## Roadmap
1. Library + read-only Google connector (this spec). 2. Plug into TimeTug (`CalendarEvent` -> `TimeTugCalendarEvent` rename, dynamic sources, Accounts pane, Keychain, OAuth interaction, mapper). 3. Write capabilities (`WritableCalendarSource`, `EventDraft`/`EventPatch`, `RecurrenceRule`, Google writes, EventKit writable). 4. Microsoft connector and plug it into TimeTug.

## Package
`Packages/CalendarConnectors/Package.swift`: swift-tools 6.0, `platforms: [.macOS(.v14)]`, Swift 6 language mode.
- Products/targets: `CalendarCore` (library, no Apple-only imports, must build on Linux), `GoogleCalendar` (library, depends on `CalendarCore`), test targets `CalendarCoreTests` and `GoogleCalendarTests` (Swift Testing).
- One external dependency: `apple/swift-crypto` (`Crypto`) for SHA-256 in PKCE (CryptoKit is Apple-only). Pin exact, like the app's other dependencies.
- The package has no dependency on TimeTug packages, so the compiler enforces the boundary; no separate import check is needed.
- Networking uses an injected `HTTPTransport` protocol; the default implementation wraps `URLSession` (`import FoundationNetworking` under `#if canImport(FoundationNetworking)` for Linux). Tests inject a fake transport (simpler and more portable than `URLProtocol`).
- Time is passed in (`now`) or comes from an injected `Clock`; library logic never calls `Date()`.

## CalendarCore

### Model
```swift
public struct CalendarDescriptor: Hashable, Sendable {   // one calendar within an account
    public var id: String; public var title: String
    public var colorHex: String?          // "#RRGGBB" uppercase
    public var accessRole: AccessRole     // .owner, .writer, .reader, .freeBusyReader
    public var isPrimary: Bool; public var timeZone: TimeZone?
    public var accountName: String?       // e.g. the signed-in email
}
public struct CalendarEvent: Hashable, Sendable, Identifiable {
    public var eventID: String            // unique within its calendar (a recurring instance has its own id)
    public var id: String { "\(calendarID)/\(eventID)" }   // unique across calendars of one source
    public var uid: String?               // iCalendar UID (Google iCalUID); stable across copies of the same meeting
    public var calendarID: String
    public var title: String; public var notes: String?; public var location: String?
    public var start: Date; public var end: Date      // all-day: midnight of the first day in `timeZone`, end EXCLUSIVE (midnight after the last day)
    public var timeZone: TimeZone?; public var isAllDay: Bool   // `timeZone` is always non-nil when `isAllDay`; consumers must interpret an all-day event's days in that zone, not the device's
    public var status: EventStatus        // .confirmed, .tentative, .cancelled
    public var availability: Availability // .busy, .free
    public var visibility: Visibility     // .default, .publicEvent, .privateEvent, .confidential
    public var kind: EventKind            // .standard, .focusTime, .outOfOffice, .workingLocation, .birthday, .other
    public var seriesID: String?          // recurring instance -> series id
    public var originalStart: Date?       // recurring instance -> its original start
    public var attendees: [Attendee]; public var organizer: Attendee?
    public var conference: ConferenceInfo?
    public var reminders: [Reminder]
    public var url: URL?                  // link to the event in the provider's UI
    public var version: String?           // opaque (Google etag); Phase 3 uses it for optimistic writes
    public var myResponse: ResponseStatus? // the account owner's own response, when the provider says
}
public struct Attendee: Hashable, Sendable { name, email (normalized lowercase), role (.required/.optional/.resource), response: ResponseStatus, isSelf, isOrganizer }
public enum ResponseStatus: String, Sendable { case accepted, tentative, declined, needsAction }
public struct ConferenceInfo: Hashable, Sendable { public var url: URL; public var provider: ConferenceProvider /* .meet, .teams, .zoom, .other */ }
public struct Reminder: Hashable, Sendable { public var minutesBefore: Int }
```
`CalendarEvent` is deliberately smaller than what a provider exposes; unmapped provider data is dropped in Phase 1. TimeTug's dedup fields are not here (they belong to `TimeTugCalendarEvent`).

### Source interface
```swift
public struct SourceCapabilities: Equatable, Sendable {
    public var canWrite = false, canEditAttendees = false, canRespondToInvite = false, providesConference = false
    public var syncKind: SyncKind = .none     // .none, .token, .notification
    public var supportsPush = false
}
public enum CalendarChange: Equatable, Sendable {
    case calendarsChanged; case eventsChanged(calendarIDs: Set<String>?)   // nil = unknown scope
    case sourceFailed(SourceError)     // terminal: the stream finishes after this (e.g. .authExpired) so the app can surface it and re-authorize
}
public protocol CalendarSource: Sendable {
    var id: String { get }; var displayName: String { get }; var capabilities: SourceCapabilities { get }
    func calendars() async throws -> [CalendarDescriptor]
    func events(in interval: DateInterval) async throws -> [CalendarEvent]   // all visible calendars of the account
    func changes() -> AsyncStream<CalendarChange>
}
public protocol PollingCalendarSource: CalendarSource {
    /// One cheap incremental check (sync token / delta). Returns the change since the last call, or nil for none. The first call establishes the baseline and returns nil.
    func checkForChanges() async throws -> CalendarChange?
}
public enum SourceError: Error, Sendable, Equatable { case authExpired, network(String), rateLimited(retryAfter: TimeInterval?), server(status: Int), invalidResponse(String) }
```
`SourceError` here is the library's; TimeTug's adapter (Phase 2) maps `.authExpired` to its own status.

### ChangeMonitor
```swift
public struct ChangeMonitor: Sendable {
    public init(interval: Duration = .seconds(60), maxBackoff: Duration = .seconds(900), clock: any Clock<Duration> = ContinuousClock())
    /// Polls `source.checkForChanges()` until the stream is cancelled, yielding whatever change it returns.
    public func changes(polling source: some PollingCalendarSource) -> AsyncStream<CalendarChange>
}
```
Behavior: sleeps `interval` between checks; on a transient error (network, rate limit, server) waits `min(maxBackoff, interval * (1 << min(failures, 10)))` (exponent clamped before multiplying, so the counter can grow without overflow) and yields nothing; `authExpired` yields `.sourceFailed(.authExpired)` and finishes the stream, so the app is told and can re-authorize instead of the connector going quiet; a success resets the backoff. Sleep uses the injected clock, so tests drive it without real time. `changes()` returns a new poller per call, so the app should hold one subscriber per source (two would double the request volume). Polling starts when the stream is created and stops only when the consumer's task is cancelled or the stream finishes; dropping the stream without cancelling does not stop it. A `PushChangeSource` seam is not defined in code in Phase 1 (YAGNI); `SourceCapabilities.supportsPush` and `syncKind` are the documented hooks.

### Connecting
```swift
public struct Platform: OptionSet, Sendable { macOS, iOS, linux, windows; static var current: Platform }
public enum AuthorizationMethod: Sendable { case oauth; case password(fields: [CredentialField]); case system }
public typealias ConnectionID = String
public struct Connection: Codable, Hashable, Sendable, Identifiable {   // non-secret; the app persists it
    public var kindID: String; public var connectionID: ConnectionID; public var displayName: String; public var config: [String: String]
}
public protocol CredentialStore: Sendable {   // a keyed map per connection: Google stores ["refresh_token": ...]; password connectors store their own fields
    func secrets(for: ConnectionID) async throws -> [String: String]?
    func setSecrets(_: [String: String], for: ConnectionID) async throws
    func removeSecrets(for: ConnectionID) async throws
}
public protocol SyncStateStore: Sendable { func token(for: ConnectionID, scope: String) async -> String?; func setToken(_: String?, for: ConnectionID, scope: String) async; func removeAll(for: ConnectionID) async }
public actor InMemoryCredentialStore: CredentialStore { ... }   // tests and short-lived tools; there is no plaintext-file default
public actor InMemorySyncStateStore: SyncStateStore { ... }
public protocol OAuthRedirectSession: Sendable {
    var redirectURI: URL { get }                                    // e.g. http://127.0.0.1:53211
    func authorize(at authorizationURL: URL) async throws -> URL    // opens the browser, waits for the redirect, returns the redirect URL
    func close() async
}
public protocol AuthorizationInteraction: Sendable {
    func beginOAuthRedirect() async throws -> any OAuthRedirectSession
    func promptCredentials(_ fields: [CredentialField]) async throws -> [String: String]   // not used by Google
}
public protocol ConnectorKind: Sendable {
    var id: String { get }; var displayName: String { get }
    var supportedPlatforms: Platform { get }; var authorization: AuthorizationMethod { get }
    func authorize(using: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection
    /// Re-runs sign-in for an existing connection (after `authExpired`): keeps its `connectionID` and `sourceID` so calendar keys and sync state stay valid, replaces the secrets, and throws `invalidResponse` if the user signs in as a different account.
    func reauthorize(_ connection: Connection, using: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection
    func makeSource(for: Connection, credentials: any CredentialStore, syncState: any SyncStateStore) throws -> any CalendarSource
}
public struct ConnectorRegistry: Sendable { register(_:), kind(id:), kinds(for: Platform) }
```
`sourceID` for a connection is `"\(kindID)-\(connectionID)"`, provided by a `Connection.sourceID` helper so every consumer agrees (Phase 2 relies on it for calendar keys).

### OAuth (portable, used by connectors)
- `PKCE`: verifier = 64 random URL-safe characters; challenge = base64url(SHA-256(verifier)), method `S256`. Test with the RFC 7636 appendix B vector.
- `OAuthClient(config: OAuthConfig(authorizationEndpoint, tokenEndpoint, clientID, clientSecret?, scopes, extraAuthParams), transport)`: `authorizationURL(redirectURI:, state:, challenge:)`, `exchange(code:, verifier:, redirectURI:) -> OAuthTokens` (callers must pass the same `redirectURI` as `authorizationURL`; `authorize` reads it once from the session and uses that value for both), `refresh(refreshToken:) -> OAuthTokens`. `OAuthTokens { accessToken, expiresAt (from injected now), refreshToken? }`. `invalid_grant` on refresh throws `SourceError.authExpired`; other OAuth errors throw `.invalidResponse`.
- `AccessTokenProvider` (actor): holds the cached access token, returns it if it has more than 60 s left, else refreshes using the stored refresh token from `CredentialStore`, persisting a rotated refresh token if the response supplies one. Concurrent callers share one in-flight refresh. `invalidate()` forces a refresh (used after a 401).
- Validation: the redirect `state` must match; an `error` query item (e.g. `access_denied`) becomes a cancellation error, not a crash.

## GoogleCalendar
- `GoogleOAuthConfig { clientID, clientSecret }` is injected by the app (values come from git-ignored config; never in this package). `GoogleConnectorKind(config:, transport:, now:)`: id `google`, displayName "Google", supports all platforms, `.oauth`.
- Scopes: `https://www.googleapis.com/auth/calendar.events` (also what Phase 3 writes need) and `https://www.googleapis.com/auth/calendar.calendarlist.readonly`. Request `access_type=offline` and `prompt=consent` so a refresh token is issued.
- `authorize`: begin redirect session, build the PKCE authorization URL with a random `state`, `authorize(at:)`, validate state, exchange the code, call `calendarList` with the new access token and take the `primary` entry's id (the account email) as `displayName`, and only then store the refresh token via `CredentialStore` under a fresh `ConnectionID` (UUID string), so a failed `calendarList` leaves nothing stored. The redirect session is closed in a `defer`. Return `Connection(kindID: "google", connectionID:, displayName: email, config: ["email": email])`. If no refresh token is returned, throw `invalidResponse`. `reauthorize` runs the same flow, requires the primary calendar id to equal `config["email"]`, and overwrites the secrets under the existing `ConnectionID`.
- `GoogleCalendarSource` (id `google-<connectionID>`):
  - `calendars()`: `GET calendar/v3/users/me/calendarList` (paged, `showHidden=false`, `minAccessRole=freeBusyReader`), mapping `summaryOverride ?? summary`, `backgroundColor` (normalized), `accessRole`, `primary`, `timeZone`, `accountName` = email. `selected == false` entries are kept but the app decides visibility (Phase 2).
  - `events(in:)`: for each calendar concurrently, `GET calendars/{id}/events` with `singleEvents=true`, `timeMin`/`timeMax` (RFC 3339), `maxResults=250`, paging via `nextPageToken`, `showDeleted=false`, and a `fields` mask limited to what the mapper reads. A calendar returning 404 or 403 `forbidden` is skipped (removed or lost access) without failing the whole account; other errors propagate.
  - Mapping (pure, unit-tested from JSON fixtures): timed events use `dateTime` with `timeZone`; all-day events use `date` (midnight of the given date in the calendar's zone, else UTC; `timeZone` is set to that zone; end exclusive as Google supplies it; Google's `date` values carry no per-event zone); `status == cancelled` events are dropped; `eventType` maps to `kind`; `transparency == transparent` is `.free`; `attendees[]` map with `self`, `organizer`, `optional`, `resource`, `responseStatus` (and `myResponse` from the `self` attendee; nil when there are no attendees); `conferenceData.entryPoints` with `entryPointType == "video"` gives the conference URL (provider from `conferenceSolution.key.type`), falling back to `hangoutLink`; `reminders.overrides` (when `useDefault` is false) to `Reminder`s; `htmlLink` to `url`; `etag` to `version`; `recurringEventId`/`originalStartTime` to `seriesID`/`originalStart`.
  - Auth failures: a 401 invalidates the access token and retries once, then throws `authExpired`. 403/429 with `rateLimitExceeded`/`userRateLimitExceeded` are retried up to 3 times with jittered backoff (delay honoring `Retry-After`), then throw `rateLimited`. 5xx throws `server`. Network errors throw `network`.
  - `capabilities`: `canWrite` false, `providesConference` true, `syncKind` `.token`, `supportsPush` false.
  - `changes()`: returns `ChangeMonitor().changes(polling: self)`.
  - `checkForChanges()`: first `calendars()`. The calendar set is itself tracked as a token under scope `_calendars` (the sorted ids joined); if it differs from the stored value, store the new value and report `.calendarsChanged` (an added or removed calendar), clearing the tokens of calendars that disappeared. Then per calendar: if `SyncStateStore` has no token, **bootstrap** by listing the whole calendar with `showDeleted=true`, `maxResults=2500` and `fields=nextPageToken,nextSyncToken`, paging (no page cap; the loop honors task cancellation) until `nextSyncToken` is returned, and store it; a bootstrap on a source that already has a baseline (a newly added calendar) reports `.calendarsChanged` via the set change above, and the very first call ever reports nil (baseline). Otherwise list with `syncToken`, `showDeleted=true` (Google requires it whenever `syncToken` is used; use the same value in both requests), `maxResults=250`, `fields=nextPageToken,nextSyncToken,items(id)`, following pages until `nextSyncToken` (only the last page carries it); any item means `.eventsChanged(calendarIDs:)` for that calendar; store the new token. HTTP 410 clears the token, re-bootstraps, and reports `.eventsChanged` for that calendar. The result merges the per-calendar results into one change (calendar ids unioned; `.calendarsChanged` wins). A failure mid-bootstrap stores nothing and simply retries next interval (converges, because there is no cap). Google forbids combining `syncToken` with `timeMin`/`timeMax`, which is why the check is a separate whole-calendar listing rather than the windowed one.
  - `.calendarsChanged` means reload calendars and events (event changes found in the same check are folded into it).
  - The first `checkForChanges` after connecting costs one full (field-minimal) listing per calendar; a calendar with many thousands of events takes several pages of 2500. Each poll also costs one `calendarList` request. Acceptable.
- The Desktop OAuth client's non-secret "client secret" is required by Google's token endpoint for that client type; it is supplied by the app, not embedded here.

## Error handling summary
Library errors are `SourceError` (auth, network, rate limit, server, invalid response). Sources never throw provider-specific types. Cancellation (`CancellationError`) propagates untouched. Parsing a single malformed event drops that event and records nothing else (no logging in the library); a wholly unparseable response throws `invalidResponse`.

## Testing (Swift Testing, TDD: failing test first)
- `CalendarCoreTests`: `Connection` Codable round trip and decoding with unknown extra keys; `Connection.sourceID`; `ConnectorRegistry` filtering by platform; `SyncStateStore.removeAll`; `PKCE` vs RFC 7636 vector and verifier alphabet/length; `OAuthClient` authorization URL and token exchange/refresh via fake transport (including `invalid_grant`, rotated refresh token); `AccessTokenProvider` (secrets read from `CredentialStore` under key `refresh_token`; cache hit, expiry refresh, single in-flight refresh under concurrency, rotation persisted, `invalidate`); `ChangeMonitor` with a manual test clock (interval, backoff doubling and cap, no overflow after thousands of failures, reset on success, `authExpired` yields `.sourceFailed` then finishes, cancellation); `InMemory*` stores.
- `GoogleCalendarTests`: mapping fixtures (timed, all-day, multi-day all-day exclusive end, recurring instance, cancelled dropped, declined by self, no attendees, conference via `conferenceData` and via `hangoutLink`, focus-time and out-of-office kinds, reminders default vs overrides, missing fields); calendar list mapping; paging; per-calendar skip on 404/403; 401 retry then `authExpired`; 429 backoff then `rateLimited`; `authorize` happy path (state check, no-refresh-token error, `access_denied`); `checkForChanges` (bootstrap returns nil, unchanged, changed, 410 re-bootstrap, multi-page with a token only on the last page, `showDeleted=true` on both requests, calendar added or removed reports `.calendarsChanged` and clears stale tokens, failure mid-bootstrap stores nothing and converges on retry); `reauthorize` (same `ConnectionID`, different account rejected); `authorize` failure after token exchange stores nothing; redirect session closed.
- Commands: `swift test --package-path Packages/CalendarConnectors`. CI: extend the existing `core` job with that command and a Linux build of the package in `core-linux` (allowed to fail like today; the goal is to prove portability).

## Docs and repo changes
- New ADR `docs/decisions/0012-calendar-connector-library.md`: decision to build a portable connector library in-repo (extract later), poll-with-sync-tokens with a push seam, `CredentialStore` as our abstraction, no Apple-only OAuth dependencies, connector capabilities per connector, the `CalendarEvent`/`TimeTugCalendarEvent` naming, Google verification caveats (sensitive scope; Testing mode 7-day refresh-token expiry; verification for public release).
- `AGENTS.md`: layout entry for `Packages/CalendarConnectors`, its test command, and the rule "the library imports nothing from TimeTug; TimeTug's EventKit connector lives in TimeTug".
- `docs/architecture.md`: short pointer.

## Risks and open questions
- Google Cloud OAuth client (Desktop) must exist before Phase 2's live testing; the user creates it (Calendar API enabled; add the account as a test user).
- `swift-crypto` adds a first-build network fetch (already true for KeyboardShortcuts/Sparkle).
- `CalendarSource` is also the name of TimeTug's existing protocol. They live in different modules; Phase 2's adapter module-qualifies them. If that proves clumsy, rename TimeTug's in Phase 2.
- Bootstrapping a sync token on very large calendars is the most expensive operation (several 2500-item pages, once per calendar per connection). Persisting the sync tokens across launches (Phase 2) avoids repeating it.
