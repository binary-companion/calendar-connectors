# Calendar Connectors: API contract

Status: parts 1 to 10 describe the code on `master` once Phase 3 (write capabilities) has merged; part 11 lists the known gaps.
The library is pre-1.0 and lives in this repository; it is meant to be extracted into its own repository once a
second provider (Microsoft, Phase 4) has proved the API is provider-neutral. Until then, anything here can change
with a matching change to this document.

## 1. What the suite is

A portable Swift library that lets a host application read (and, from Phase 3, write) calendars from several
providers behind one model, plus the small amount of host-specific glue TimeTug needs.

| Package / target | Role | Depends on | Platform notes |
|---|---|---|---|
| `CalendarConnectors` / `CalendarCore` | Model, source and connector protocols, stores, HTTP seam, change polling, all-day helpers | Foundation only | Portable; built on Linux in CI |
| `CalendarConnectors` / `CalendarOAuth` | OAuth 2.0 authorization-code + PKCE client, access-token cache | `CalendarCore` | Portable; no crypto dependency (pure-Swift SHA-256 default) |
| `CalendarConnectors` / `GoogleCalendar` | Google Calendar connector (REST v3) | `CalendarCore`, `CalendarOAuth` | Portable |
| `CalendarConnectors` / `CalendarTestSupport` | Fakes and conformance helpers for connector tests | `CalendarCore` | Test-only product |
| `CalendarApple` | Apple-only adapters: Keychain credentials, loopback and web-auth-session OAuth interaction, CryptoKit hashing | `CalendarCore`, `CalendarOAuth` | macOS |
| `EventKitSource` | Apple Calendar via EventKit as a connector | `CalendarCore` | macOS |
| `CalendarBridge` | Adapts a library source to TimeTug Core's own `CalendarSource` | `CalendarCore`, `TimeTugCore` | TimeTug only |

Layering rule: maximise generic, cross-platform code in `CalendarConnectors`. Anything OS-specific (Keychain,
Network.framework, EventKit, AuthenticationServices) lives in a separate adapter that implements a library
protocol, so another host can supply its own.

Dependency direction: `GoogleCalendar` → `CalendarOAuth` → `CalendarCore`; adapters and the bridge depend on
`CalendarCore`; nothing in `CalendarConnectors` depends on an adapter.

## 2. Conventions every part of the contract relies on

- **Concurrency.** All public types are `Sendable`; sources and stores are safe to call from any task. Swift 6
  language mode (the Apple adapters build in Swift 5 mode because of AppKit/Network types).
- **Errors.** Sources throw `SourceError` (part 3.6) and never leak provider or transport types. Cancellation
  surfaces as `CancellationError`.
- **Time.** Instants are `Date` and authoritative. `timeZone` is always set (non-optional): the zone the event is
  shown in, a display hint for timed events. When the provider gives an event no zone the connector fills its
  fallback: Google the calendar's zone, then UTC; EventKit the device zone. On writes `EventTiming.timeZone` stays
  optional: nil means "no preference" (Google leaves out the zone name so the event takes the calendar's zone;
  EventKit leaves the event's zone nil, a floating time), and a given zone is sent as it is.
- **All-day events (canonical form).** `isAllDay == true` ⇒ `start` is midnight of the
  first day in that zone, `end` is midnight of the day after the last day (exclusive). Days are interpreted in the
  event's own zone, not the device's. Connectors build and read this form only through the `AllDay` helpers, and
  their tests run fixtures through `AllDayConformance.violations`.
- **Identity.** A calendar is identified by `CalendarDescriptor.id` within its source. An event is identified by
  `(calendarID, eventID)` within its source (`CalendarEvent.id == "\(calendarID)/\(eventID)"`). A source is
  identified by `CalendarSource.id`. For network connectors `id == Connection.sourceID == "<kindID>-<connectionID>"`.
  EventKit is the documented exception: its source id is the constant `"eventkit"`, kept so previously stored
  calendar keys stay valid. Hosts always identify a source by `source.id`.
- **Secrets never live in the library.** Non-secret account data is a `Connection`; secrets go through a
  host-supplied `CredentialStore`. OAuth client identifiers are injected by the host from configuration that is
  not committed.
- **Polling, not push.** Change detection uses provider sync tokens on a timer. There are no webhooks or relay
  servers in the contract (the `supportsPush` capability is a seam only).

## 3. `CalendarCore`

### 3.1 Model

```swift
public struct CalendarDescriptor: Hashable, Sendable, Identifiable {
    var id: String; var title: String
    private(set) var colorHex: String?   // "#RRGGBB" uppercase, or nil if unknown/invalid
    var service: CalendarService         // the connector reading it ("eventkit", "google"); always known
    var provider: CalendarProvider?      // who hosts it (.iCloud, .google, .microsoft, .local, .subscription, .calDAV); nil = can't tell
    var permissions: CalendarPermissions // canViewDetails, canEdit, canShare?, canViewPrivate?
    var accessRole: AccessRole? { get }  // read-only summary: owner, writer, reader, freeBusyReader; nil for EventKit writable
    var isDefault: Bool?                 // where new events go in its account; nil = can't tell
    var timeZone: TimeZone?
    var accountName: String?             // the owning account, e.g. the signed-in email
    var kind: CalendarKind               // standard, subscribed, birthdays
    var defaultReminders: [Reminder]?    // the calendar's own default reminders (Google); nil = source does not say
    var supportedAvailabilities: Set<Availability>?   // what the calendar accepts; [] = none tracked; nil = unknown
    static func normalizedHex(_:) -> String?   // "#RGB" | "#RRGGBB" | "RRGGBB" → "#RRGGBB"
}
```

`CalendarEvent` (the one generic event type; TimeTug's own wrapper is `TimeTugCalendarEvent`):

| Field | Type | Contract |
|---|---|---|
| `eventID` | `String` | Unique within its calendar; a recurring instance has its own id (see part 11 for EventKit) |
| `id` | `String` (computed) | `"\(calendarID)/\(eventID)"`, unique within a source |
| `uid` | `String?` | iCalendar UID (Google `iCalUID`); stable across copies of the same meeting |
| `calendarID` | `String` | Owning calendar |
| `title` | `String` | Non-optional; a connector substitutes its own placeholder for an untitled event ("(No title)" today) |
| `notes`, `location` | `String?` | |
| `start`, `end` | `Date` | See the all-day convention; `end` is exclusive |
| `timeZone` | `TimeZone` | Always set; see Time above |
| `isAllDay` | `Bool` | |
| `status` | `EventStatus` | `confirmed`, `tentative`, `cancelled` |
| `availability` | `Availability?` | `busy`, `free`, `tentative`, `unavailable`; nil when the source does not say (EventKit `.notSupported`) |
| `visibility` | `Visibility?` | `default`, `publicEvent`, `privateEvent`, `confidential`; nil when the source does not say |
| `kind` | `EventKind?` | `standard`, `focusTime`, `outOfOffice`, `workingLocation`, `birthday`, `other`; nil when the source does not say |
| `series` | `SeriesInfo?` | `.notRecurring`, or `.occurrence(seriesID:originalStart:)` on an instance of a recurring series (Google `recurringEventId` and `originalStartTime`; EventKit `eventIdentifier` and `occurrenceDate`, for events that recur or are detached occurrences); nil when the source does not say. `seriesID` and `originalStart` are get-only accessors |
| `attendees` | `[Attendee]` | `name?`, `email?` (trimmed, lowercased), `role` (`required/optional/resource`), `response`, `isSelf`, `isOrganizer` |
| `organizer` | `Attendee?` | |
| `conferences` | `[ConferenceInfo]` | Join links, most likely first: `url`, `provider` (`meet/teams/zoom/webex/goToMeeting/whereby/jitsi/slack/other`), `origin` (`structured/location/url/notes/eventURL`), computed `identity`. Filled by every connector through `ConferenceDetector`. `conference` is the first entry (get-only). |
| `reminders` | `[Reminder]?` | `minutesBefore`; `[]` means none, nil means the source does not say. Google `useDefault` resolves to the calendar's `defaultReminders` |
| `url` | `URL?` | Link to the event in the provider's UI |
| `version` | `String?` | Opaque provider version (Google etag, EventKit `lastModifiedDate` as a fractional-epoch string); the base of optimistic writes (part 10) |
| `participation` | `Participation?` | `.notInvited` or `.invited(ResponseStatus)`; nil when the source does not say. `myResponse` is a get-only accessor (nil for both unknown and not invited) |
| `uidScope` | `UIDScope?` | `.global` (an iCalendar UID, comparable across sources), `.provider` (a provider id, comparable only within one service and provider), nil = unknown (treated as `.provider`) |
| `sourceID` | `String?` | The source that produced the event (`Connection.sourceID`; `"eventkit"` for EventKit); lets a host route an event to its account. Does not change `id` |

Reads return recurring events already expanded into instances. No read path returns a recurrence rule.

### 3.2 Sources

```swift
public protocol CalendarSource: Sendable {
    var id: String { get }
    var displayName: String { get }
    var capabilities: SourceCapabilities { get }
    func calendars() async throws -> [CalendarDescriptor]
    /// Events of every visible calendar of the account that overlap `interval`.
    func events(in interval: DateInterval) async throws -> [CalendarEvent]
    func changes() -> AsyncStream<CalendarChange>
}

public protocol PollingCalendarSource: CalendarSource {
    /// One cheap incremental check. Returns the change since the last call, or nil for none.
    /// The first call establishes the baseline and returns nil.
    func checkForChanges() async throws -> CalendarChange?
}
```

Guarantees:

- `events(in:)` returns events sorted by start (then id) for network connectors, excluding calendars the account
  can no longer read. Cancelled events may be present in the model (`status == .cancelled`); consumers such as the
  bridge drop them.
- A source is read-only unless it also conforms to `WritableCalendarSource` (part 10).

### 3.3 Capabilities

```swift
public struct SourceCapabilities: Equatable, Sendable {
    var canWrite: Bool              // by convention true exactly when the source conforms to WritableCalendarSource
    var canEditAttendees: Bool      // true exactly when writableFields contains .attendees (checked by the conformance suite)
    var canRespondToInvite: Bool
    var providedFields: Set<ProvidedField>   // what the source reliably fills on reads; a listed field is never nil
    var syncKind: SyncKind          // .none, .token, .notification
    var supportsPush: Bool
    var writableFields: Set<EventField>          // what create/update can write; empty when read-only
    var controlsNotifications: Bool              // honors NotifyPolicy; false = the server decides
    var recurrenceScopes: Set<RecurrenceScope>   // scopes accepted on a series; empty when read-only
}
```

Capabilities are what a source *can* do, declared per connector; the per-calendar `permissions` say which of its
calendars are writable. The three write fields default to the read-only value. Current values:

| | `canWrite` | `canEditAttendees` | `canRespondToInvite` | `providedFields` | `syncKind` | `writableFields` | `controlsNotifications` | `recurrenceScopes` |
|---|---|---|---|---|---|---|---|---|
| Google | true | true | true | true | `.token` | all | true | all three |
| EventKit | true | false | false | false | `.notification` | title, notes, location, timing, availability, reminders, recurrence | false | all three (subject to the EventKit spike, part 11) |

### 3.4 Changes

```swift
public enum CalendarChange: Equatable, Sendable {
    case calendarsChanged                      // reload calendars AND events; event changes in the same check are not reported separately
    case eventsChanged(calendarIDs: Set<String>?)   // nil = scope unknown
    case sourceFailed(SourceError)             // terminal: the stream finishes after this (e.g. .authExpired)
}
```

`ChangeMonitor` (public struct) turns a `PollingCalendarSource.checkForChanges()` into `changes()`:
default interval 60 s, exponential backoff on failures capped at 900 s, sleeping through an injectable `Sleeper`.
Polling starts when the stream is created and stops when the consuming task is cancelled or the stream finishes;
dropping the stream without cancelling does not stop it. Use one subscriber per source.

### 3.5 Connecting

```swift
public struct Connection: Codable, Hashable, Sendable, Identifiable {
    var kindID: String; var connectionID: ConnectionID   // ConnectionID = String
    var displayName: String; var config: [String: String] // non-secret
    var sourceID: String { "\(kindID)-\(connectionID)" }  // every consumer must use this helper
}

public protocol ConnectorKind: Sendable {                 // registered once per provider
    var id: String { get }; var displayName: String { get }
    var supportedPlatforms: Platform { get }              // OptionSet: macOS, iOS, linux, windows
    var authorization: AuthorizationMethod { get }        // .oauth, .password(fields:), .system
    func authorize(using: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection
    func reauthorize(_: Connection, using: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection
    func makeSource(for: Connection, credentials: any CredentialStore, syncState: any SyncStateStore) throws -> any CalendarSource
}
```

- `authorize` stores secrets only after sign-in fully succeeds, under a fresh `ConnectionID`; a failure leaves
  nothing stored.
- `reauthorize` keeps the `connectionID` (so `sourceID`, calendar keys and sync state stay valid), replaces the
  secrets, and throws `SourceError.invalidResponse` if the user signs in as a different account.
- `ConnectorRegistry` (`register`, `kind(id:)`, `kinds(for: Platform)` sorted by id) is the host's lookup.

Host-supplied seams:

```swift
public protocol CredentialStore: Sendable {   // keyed secrets per connection (Google: ["refresh_token": …])
    func secrets(for:) async throws -> [String: String]?
    func setSecrets(_:for:) async throws
    func removeSecrets(for:) async throws
}
public protocol SyncStateStore: Sendable {    // per-connection, per-scope tokens; absent = full sync
    func token(for:scope:) async -> String?
    func setToken(_:for:scope:) async
    func removeAll(for:) async
}
public protocol AuthorizationInteraction: Sendable {   // the only OS/UI-specific part of sign-in
    func beginOAuthRedirect() async throws -> any OAuthRedirectSession
    func promptCredentials(_ fields: [CredentialField]) async throws -> [String: String]
}
public protocol OAuthRedirectSession: Sendable {
    var redirectURI: URL { get }
    func authorize(at authorizationURL: URL) async throws -> URL   // opens the URL, waits, returns the redirect
    func close() async
}
```

Shipped implementations in `CalendarCore`: `InMemoryCredentialStore`, `InMemorySyncStateStore` (actors),
`FileConnectionStore` (JSON list of `Connection`; a missing file loads as empty, an unreadable file is never
overwritten and throws `FileStoreError.unreadable` on write), `FileSyncStateStore` (one JSON file; unreadable ⇒
"no tokens", so sources re-bootstrap).

### 3.6 Errors

```swift
public enum SourceError: Error, Sendable, Equatable {
    case authExpired                    // re-authorize
    case network(String)
    case rateLimited(retryAfter: TimeInterval?)
    case server(status: Int)
    case invalidResponse(String)
    case needsPermission                // OS/user has not granted access to a local store; never thrown by network connectors
}
```

Provider-specific outcomes (404, 410, per-calendar 403) are handled inside the connector and never escape as
provider types. Write methods additionally throw `WriteError` (part 10); authentication, network, rate-limit and server
failures on a write are still `SourceError`.

### 3.7 HTTP seam

`HTTPTransport.send(_:)` returns any HTTP response including 4xx/5xx and throws `SourceError.network` for
transport failures. `URLSessionTransport` is the default (`FoundationNetworking` on Linux); tests use
`FakeTransport`. `HTTPRequest` and `HTTPResponse` are plain value types (response header names are lowercased).

### 3.8 All-day helpers

`CalendarDate` (`year, month, day`, `adding(days:)`, `Comparable`) and `AllDay` (`startOfDay`, `date(of:in:)`,
`canonical(first:endExclusive:in:)`, `dates(start:end:in:)`, `endExclusive(afterLast:)`) are the only sanctioned way
to convert between provider all-day dates and the canonical instants.

## 4. `CalendarOAuth`

- `OAuthConfig`: authorization and token endpoints, client id, optional client secret (a desktop client's secret is
  not confidential but the token endpoint still requires it), scopes, extra authorization parameters.
- `OAuthClient(config:transport:now:)`: `authorizationURL(redirectURI:state:codeChallenge:)` (always PKCE `S256`),
  `authorizationCode(from:expectedState:)` (throws `AuthorizationError.stateMismatch`, `.cancelled` for
  `access_denied`, `.missingCode`), `exchange(code:verifier:redirectURI:)`, and `refresh(refreshToken:)`, which
  throws `SourceError.authExpired` on `invalid_grant`.
- `AccessTokenProvider` (actor): `accessToken()` returns a cached token or refreshes 60 s before expiry; concurrent
  callers share one in-flight refresh; a rotated refresh token is written back to the `CredentialStore` (re-reading
  first so other keys are not overwritten); `invalidate()` after a 401.
- `PKCE`: verifier generation and `challenge(for:hasher:)`. `SHA256Hashing` is a one-method protocol; the library
  ships `PureSwiftSHA256` (NIST-vector tested, used only on a public verifier) so `CalendarCore` and its dependents
  need no crypto package; hosts may inject `CryptoKitSHA256` (from `CalendarApple`) or their own.

## 5. `GoogleCalendar`

Public surface: `GoogleOAuthConfig(clientID:clientSecret:)`, `GoogleConnectorKind(config:transport:now:sleep:pollInterval:hasher:)`,
and the `CalendarSource` returned by `makeSource` (id `google-<connectionID>`).

- **Kind:** id `google`, display name "Google", all platforms, `.oauth`. Scopes
  `calendar.events` and `calendar.calendarlist.readonly` (the events scope already covers writes, so no
  re-consent is needed; it cannot create calendars). Authorization requests `access_type=offline` and `prompt=consent` so a refresh token is issued.
- **Connection:** `displayName` and `config["email"]` are the primary calendar's id (lowercased); sign-in throws
  `invalidResponse` if no refresh token or no primary calendar comes back.
- **Reads:** `calendarList` (paged, `showHidden=false`, `minAccessRole=freeBusyReader`), then per calendar
  `events.list` with `singleEvents=true`, the requested window, paging and a restricted `fields` mask. All-day `date`
  values are converted to the canonical form. A calendar that returns 404 or a plain `forbidden` 403 is skipped.
- **Change detection:** one sync token per calendar plus a token for the set of calendars. Google forbids combining a
  sync token with a time window, so a token is bootstrapped from one minimal listing. A changed calendar set yields
  `.calendarsChanged`; changed events yield `.eventsChanged(calendarIDs:)`; HTTP 410 drops the token, re-bootstraps
  and reports a change.
- **HTTP behaviour:** 401 refreshes the token once, then `.authExpired`; 403 `insufficientPermissions` is
  `.authExpired`; 429 and rate-limit 403 retry up to three times honoring `Retry-After` (else jittered exponential
  backoff) and then throw `.rateLimited`; 5xx throws `.server`; anything else is `.invalidResponse`.
- **Writes:** `GoogleCalendarSource` conforms to `WritableCalendarSource` (part 10). Create, update (only the changed fields, `If-Match` from `ref.version`), delete and RSVP, with all three recurrence scopes; `.thisAndFollowing` truncates the master's `RRULE` and inserts a new series (two calls; see part 10 for the failure handling). Write requests use a write mode of the client: 412 is a stale version (handled by `PatchMerge`), 400 is `.invalid`, and a 403 with any reason but `insufficientPermissions` is `.forbidden`, except quota reasons, which are `SourceError.rateLimited`. Reads keep their original mapping.
- **Capabilities:** `canWrite`, `canEditAttendees`, `canRespondToInvite`, `providedFields`, `.token` sync, every field writable, `controlsNotifications`, all three scopes.

## 6. `CalendarApple` (macOS adapters)

| Type | Implements | Purpose |
|---|---|---|
| `KeychainCredentialStore(service:)` | `CredentialStore` | Secrets in the macOS Keychain |
| `LoopbackAuthorizationInteraction` | `AuthorizationInteraction` | OAuth through a 127.0.0.1 ephemeral-port listener; `openURL` and an optional `AuthorizationPresenting` are host-supplied; times out after 300 s; errors in `LoopbackError` |
| `WebAuthenticationSessionPresenter` | `AuthorizationPresenting` | Shows the sign-in page in an `ASWebAuthenticationSession` sheet that closes itself |
| `CryptoKitSHA256` | `SHA256Hashing` | Hardware-backed hashing for PKCE |

## 7. `EventKitSource`

- `EventKitSource` (`CalendarSource`, id `"eventkit"`, display name "Apple Calendar"): `requestAccess()` prompts for
  full calendar access; every read requires it and otherwise throws `.needsPermission`. Calendars map
  `allowsContentModifications` to `permissions.canEdit` and EventKit's calendar types to `CalendarKind`.
  All-day events are normalised from EventKit's floating device-local dates to the canonical form in the device zone.
  `changes()` yields `.calendarsChanged` on every `EKEventStoreChanged`. `init(store:)` lets a host or test share an
  `EKEventStore`. Reads fill `version` (`lastModifiedDate`), `sourceID`, and, for events that recur or are detached
  occurrences, `seriesID` (the `eventIdentifier`) and `originalStart` (`occurrenceDate`).
- `EventKitConnectorKind` (`.system` authorization, macOS only): `authorize` requests access and returns the
  synthesised `Connection` (`kindID: "eventkit"`, `connectionID: "this-mac"`); hosts do not persist it, and
  `makeSource` returns the shared source.
- **Writes:** `EventKitSource` conforms to `WritableCalendarSource` (part 10). Create, update and delete with all three scopes (subject to the spike in part 11); `respond`, attendee changes, `visibility` and generated conferences throw `.unsupported`; a read-only calendar is `.forbidden`. Inputs are validated before the access check.
- Capabilities: no conference links, `.notification` sync, `canWrite`, `writableFields` = title, notes, location, timing, availability, reminders, recurrence, no attendee editing or RSVP, no notification control, all three scopes.

## 8. `CalendarBridge` (TimeTug only)

Core defines its own `CalendarSource` (`calendars() -> [CalendarInfo]`, `events(in:) -> [TimeTugCalendarEvent]`,
`changes() -> AsyncStream<Void>`) and never sees library types.

- `ConnectedSource` wraps any library source. It maps calendars to `CalendarInfo`, events to `TimeTugCalendarEvent`
  (dropping cancelled ones), forwards every library change (including `.sourceFailed`) as a `Void` yield, and translates
  `needsPermission` and `authExpired` to Core's `SourceError`. Cancelling the consumer cancels the inner stream.
- `TimeTugCalendarEvent { event, sourceID, conferenceURL, otherAttendeeCount, responseStatus, merge state… }` carries the
  library event as is. Its `id` also appends the start time, because recurring events share a source event id but
  differ in start.

## 9. Rules for implementing a connector

1. Conform to `CalendarSource`; add `PollingCalendarSource` and use `ChangeMonitor` when the provider has a cheap
   incremental check.
2. Throw only `SourceError` (and `WriteError` from write methods); map provider errors inside the module.
3. Emit canonical all-day events and run your mapper fixtures through `AllDayConformance`.
4. Set `id` to `Connection.sourceID` (unless a documented compatibility reason applies) and keep it stable across
   `reauthorize`.
5. Declare `capabilities` honestly; `calendars()` must set `service` and `permissions` per calendar.
6. Keep secrets in the injected `CredentialStore` and persist nothing during a failed `authorize`.
7. Test against `FakeTransport` (network) or pure mappers (local stores); no test may need a real account.
8. To support writing, also conform to `WritableCalendarSource` (part 10), declare `writableFields`, `controlsNotifications` and `recurrenceScopes` honestly, throw `WriteError.unsupported` before changing anything for what you cannot write, and run `WritableSourceConformance` against a scratch calendar or a fake backend.

## 10. Write capabilities (Phase 3)

`docs/superpowers/specs/2026-09-23-calendar-connectors-phase3-design.md` is the design and rationale; this part is the
contract as built. The types live in `CalendarCore` (`Write/`), so they are portable.

**Opt-in.** Writing is an optional, separate protocol. A read-only connector never implements it.

```swift
public protocol WritableCalendarSource: CalendarSource {
    func create(_ draft: EventDraft, in calendarID: String, notify: NotifyPolicy) async throws -> CalendarEvent
    /// An empty patch writes nothing and returns the patch's `base`, or the current event when it has none.
    func update(_ ref: EventRef, _ patch: EventPatch, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent
    func delete(_ ref: EventRef, scope: RecurrenceScope, notify: NotifyPolicy) async throws
    /// `response` must be `.accepted`, `.tentative` or `.declined`.
    func respond(to ref: EventRef, _ response: ResponseStatus, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent
}
```

Writes return the event in the provider's resulting form, including its new `version`. What a write on a series returns
differs by connector, and callers that need instances re-read. Google: `create` with a recurrence, and the series-wide
writes (`.allInSeries`, a `.thisAndFollowing` at the series' first occurrence, and the new series of a split), return the
series master, now with `seriesID` set to its own id and `originalStart` to its start, so `EventRef(returned)` designates
the whole series: a `.thisInstance` update, delete or respond on it throws `WriteError.invalid("this is a recurring series;
use .allInSeries or read the occurrence first")` before any request (a single-instance write would otherwise hit the
whole series). EventKit: the first occurrence. `scope` is ignored for an event that is not part of a series.

**Three levels of "can I write?"**
1. Conformance to `WritableCalendarSource` (checked by callers with `as?`). Invariant: `capabilities.canWrite` is true
   exactly when the source conforms (a convention; `WritableSourceConformance` only checks that it is true on a writable source).
2. `capabilities` (part 3.3): `writableFields` (what create/update can write, including `.recurrence`),
   `controlsNotifications` and `recurrenceScopes`, alongside `canEditAttendees` (true exactly when `.attendees` is
   writable) and `canRespondToInvite`. `writableFields` and `recurrenceScopes` are empty unless `canWrite`, and `canRespondToInvite` implies `canWrite` (conventions upheld by the read-only defaults and unit tests; the conformance checks verify only `canEditAttendees == writableFields.contains(.attendees)`).
3. `CalendarDescriptor.permissions` per calendar (`canEdit == false` is `.forbidden`).

**Unsupported means an error, never a partial write.** A write that touches something the connector cannot represent
throws `WriteError.unsupported`, naming the fields, before any write request or store change. `WriteValidation.requireWritable(_:_:)`
is the shared check of a set of fields against `writableFields`.

```swift
public enum EventField: String, Sendable, Hashable, CaseIterable { case title, notes, location, timing, availability, visibility, reminders, attendees, recurrence, conference }
public enum NotifyPolicy: Sendable, Hashable { case all, externalOnly, none }        // required argument, no default
public enum RecurrenceScope: Sendable, Hashable, CaseIterable { case thisInstance, thisAndFollowing, allInSeries }
public enum ConferenceRequest: Sendable, Hashable { case none, generate }    // on a draft
public enum ConferenceChange: Sendable, Hashable { case generate, remove }   // on a patch
public enum WriteError: Error, Sendable, Equatable {
    case unsupported(fields: Set<EventField>)   // the connector cannot write these (or the operation; RSVP is reported as .attendees)
    case conflict(fields: Set<EventField>)      // someone else changed a field this patch touches
    case notFound                               // the event or calendar is gone
    case forbidden(String?)                     // read-only calendar or no permission on the event
    case invalid(String)                        // malformed input (end before start, needsAction RSVP, bad rule, ...)
    case partial(String)                        // a multi-step write stopped half way (Google .thisAndFollowing)
}   // auth, network, rate-limit and 5xx stay SourceError
public struct EventRef: Hashable, Sendable {
    var calendarID: String; var eventID: String
    var version: String?; var seriesID: String?
    var originalStart: Date?   // the occurrence's slot in its series; identifies it when eventID is shared, and is the .thisAndFollowing split point
    init(calendarID:eventID:version:seriesID:originalStart:)
    init(_ event: CalendarEvent)
}
```

A ref with a `seriesID` but no `originalStart` is handled per connector. Google: only `.thisAndFollowing` needs it (it throws `.invalid`, "this and following needs the occurrence's original start"); `.thisInstance`, `.allInSeries` and `respond` never read it, except that an `.allInSeries` update with a timing change and no `originalStart` is `.unsupported(fields: [.timing])`. EventKit: `.thisInstance` and `.thisAndFollowing` throw `.invalid` (a recurring occurrence is located by its original start); `.allInSeries` starts from the series' first occurrence and needs it only for a timing change (`.unsupported(fields: [.timing])` without it).
`NotifyPolicy` with `controlsNotifications == false` is accepted only when nobody else would be told (EventKit: only for
an event with no other attendees; otherwise `.unsupported(fields: [.attendees])`).

**Models.**
- `EventTiming { start, end, timeZone, isAllDay }`: time is one unit, in the canonical all-day form; `validate()` throws
  `.invalid` for a non-finite time, `end <= start`, or an all-day timing without a zone or off midnight in it.
- `AttendeeDraft(email:name:role:)`: `Hashable`; the email is trimmed and lowercased on init.
- `EventDraft` (create): `title`, `timing`, `notes`, `location`, `availability` (`.busy`), `visibility` (`.default`),
  `reminders` (`nil` = calendar defaults, empty = none), `attendees: [AttendeeDraft]`, `conference: ConferenceRequest`,
  `recurrence: RecurrenceRule?`. `validate()` checks timing, recurrence, attendee emails and non-negative reminders;
  `usedFields` is what the draft sets beyond defaults. `EventDraft(copying: event, for: capabilities)` is the one lenient
  path: it carries over what the target's `writableFields` allow and drops the rest (self and email-less attendees
  are dropped, an empty reminder list becomes `nil`, only a Meet link is re-requested, recurrence is never copied).
- `EventPatch` (update): optional fields where `nil` means keep (`title`, `timing`, `availability`, `visibility`,
  `attendees`, `conference`) and `FieldUpdate<Value>` = `.keep` (default), `.set(value)` or `.clear` where clearing is
  meaningful (`notes`, `location`, `reminders`, `recurrence`; `.clear` on reminders means the calendar defaults). Attendees are a
  delta, `AttendeeChanges(add:remove:)`, never a replacement list; `add` upserts by email (an existing attendee keeps their
  response). `touchedFields` and `isEmpty` describe what it changes. `private(set) var base: CalendarEvent?` is the original the
  patch was diffed from (`nil` for a hand-built patch), which is what lets conflicts be judged per field.
  `EventPatch(from: original, to: edited)` computes the minimal patch and sets `base`. It compares title, notes, location,
  timing, availability, visibility, reminders and attendees (by normalized email), plus removal of `conference`, and ignores
  provider-owned fields, a changed or added conference, recurrence, the account owner (the `isSelf` attendee), and a name
  cleared to `nil` with the role unchanged (an `AttendeeDraft` cannot express clearing a name). `withoutBase()` drops the
  base; `applied(to:)` applies a patch to an event (used by the in-memory test source; `.clear` reminders gives `[]`).
- `EventEdit(original)` with `event` (the working copy), `patch` and `hasChanges` for callers that want tracked edits.
- `RecurrenceRule`: frequency (daily/weekly/monthly/yearly), interval, weekdays with optional ordinal, month days,
  months, and end (`.never`, `.count(n)`, `.until(date)`), with `init(rrule:in:)`, `validate()` and
  `rruleString(allDay:in:)`. Anything outside this RFC 5545 subset (`BYSETPOS`, `BYHOUR`, sub-daily frequencies,
  `COUNT` with `UNTIL`, a daily rule with `BYDAY`, ...) throws `WriteError.unsupported(fields: [.recurrence])`. EXDATE and RDATE are not
  authorable; removing one occurrence is a `delete` with `.thisInstance`.

**Conflicts.** A write sends `If-Match: ref.version` where the provider supports it (Google etag) or compares
`lastModifiedDate` (EventKit). The shared helper in `CalendarCore` does the rest:

```swift
public enum PatchMerge {
    public enum Attempt<Result> { case done(Result), stale }
    public static func conflicts(patch: EventPatch, current: CalendarEvent) -> Set<EventField>
    public static func apply<Result>(patch: EventPatch, version: String?, maxAttempts: Int = 3,
        fetchCurrent: () async throws -> CalendarEvent,
        write: (String?) async throws -> Attempt<Result>) async throws -> Result
}
```

On `.stale` it fetches the current event and compares only the fields the patch touches against the patch's `base`
(timing as a unit; `nil` and `""` equal for notes and location; reminder order ignored; attendees only for the emails the
patch adds or removes, and an attendee someone else already changed to the wanted state is not a conflict). No difference:
the patch is re-applied on the fresh version and re-judged if it goes stale again. A difference: `WriteError.conflict(fields:)`.
`maxAttempts` is clamped to at least 1, cancellation is checked per attempt, and after the last attempt it throws
`.conflict(fields: touchedFields)`. A patch without a `base` (or that touches `recurrence`) conflicts on any stale
version. It fails closed when a retry is due, the write was versioned and the fresh event carries no `version`:
retrying without one would be an unconditional write that could overwrite an edit made after the fetch, so it throws
`.conflict(fields: touchedFields)`. Only a write that started unversioned (no `ref.version`) retries unconditionally.

`delete` ignores `ref.version` on both connectors (no `If-Match`, no modification-date check): it is last-writer-wins,
so deleting an event that someone else edited meanwhile deletes it anyway.

**Provider mapping.**

| Operation | Google | EventKit |
|---|---|---|
| create | `POST …/events?sendUpdates=…` (`conferenceDataVersion=1` for a generated Meet link) | `EKEvent` save |
| update | `PATCH` of changed fields only, `If-Match`; attendee changes fetch the current event and send the full array | mutate and save |
| delete | `DELETE …?sendUpdates=…` | `remove` with span |
| respond | fetch, set own attendee's `responseStatus`, `PATCH` the full array with the fetched etag (a 412 restarts, three tries) | `.unsupported(fields: [.attendees])` |
| `.thisInstance` | instance id (own etag) | `.thisEvent` |
| `.allInSeries` | master id (an instance ref sends no `If-Match`, since its etag differs from the master's; a master ref keeps the lock, its version being the master's) | first occurrence with `.futureEvents`; no version check |
| `.thisAndFollowing` | delete truncates the master's RRULE `UNTIL`; update truncates then inserts a new series (client-chosen id; rollback only after a definite failure; `.partial` if the rollback fails or the outcome is unknown; a patch that sets or clears `recurrence` replaces or omits the new series' rule instead of copying the master's); `respond` is `.unsupported(fields: [.attendees])` | `.futureEvents` |

An `.allInSeries` update that changes timing is refused with `.unsupported(fields: [.timing])` unless `ref` is the
series' first occurrence (the series' start equals `ref.originalStart`), on Google and EventKit: callers read expanded
instances, so an instance's date would otherwise move the whole series' start.

EventKit's declared capabilities: writes yes, `writableFields` = title, notes, location, timing, availability, reminders
and recurrence, all three scopes, RSVP no, attendee editing no, notification control no. It refuses attendee changes,
`respond`, `visibility` and a generated conference with `.unsupported`, and update or delete on a read-only calendar with
`.forbidden` (an empty-patch update returns before that check). Its writes validate input before checking calendar access.

A recurrence change (`.set` or `.clear`) with `.thisInstance` on an occurrence of a series is refused on both
connectors with `.unsupported(fields: [.recurrence])`, before any request or store mutation: a rule belongs to the whole
series (use `.allInSeries`, or `.thisAndFollowing` on Google, which starts a new series with the rule). EventKit, when a
failed save leaves the in-memory event edited, discards it with `rollback()`.

**Known limitations of Google's `.thisAndFollowing` (a split is a truncation plus a new series, not a native operation).**
The new series starts at the occurrence's original slot with the master's length, so a moved occurrence does not move
the following ones; a patch that sets `timing` applies its time to the new series instead. Beyond that:
- Notifications: the truncation, the insert and any restore of the master's rule all carry the caller's `NotifyPolicy`, so on
  an update guests may receive two messages (the cut and the new series). External iTIP guests would otherwise keep the
  old series and also get the new one. A caller who wants silence passes `.none`.
- If the split occurrence was moved and the patch sets no `timing`, it snaps back to its original slot in the new series,
  and its old exception may linger next to it.
- If the truncating `PATCH` fails in a way that may have been applied (a 5xx, a broken connection, a cancellation) on an
  update, the connector restores the master's original rule (unconditionally, with the caller's `sendUpdates` policy, even if the caller was
  cancelled) and rethrows the original error, so a retry is safe; if the restore fails too the result is
  `WriteError.partial` naming both errors. A definite failure (412 as `.conflict([.recurrence])`, 400, 403, 404, auth or rate
  limit) is not restored. A delete keeps the plain error: the truncation is the operation and repeating it is harmless.
- An occurrence after the split point that was cancelled (deleted through the API) is not carried over: only `EXDATE`
  lines are, so the cancelled occurrence may reappear in the new series.
- The carried `EXDATE` lines keep the old time of day, so if the patch changes the series' time they no longer match the
  new series' occurrences.
- A modified exception after the split point stays with the old series. Whether Google still shows it past the old
  series' new `UNTIL` (a possible duplicate next to the new series' own occurrence) is UNVERIFIED; the live smoke test
  prints what the calendar shows (`LIVE split with later exceptions ...`) and the answer has not been recorded yet.

**Testing seams (`CalendarTestSupport`).** `FakeWritableSource` is an in-memory `WritableCalendarSource` (non-recurring
events, `writeCount`, `simulateExternalEdit`). `WritableSourceConformance.violations(of:calendarID:window:)` runs the
behavior every writable source must have (create and read back, update touches only patched fields, empty patch writes
nothing, unwritable fields throw `.unsupported`, delete and delete-again `.notFound`) and returns a list of violations;
it runs against the fake and, live and opt-in, against the real EventKit source. Google is covered by request-shape tests
with `FakeTransport`. Live tests are gated by `TIMETUG_LIVE_EVENTKIT=1` and `TIMETUG_LIVE_GOOGLE=1` and never run in CI.

**Out of scope for Phase 3:** TimeTug write UI, cross-calendar link orchestration (kept in TimeTug's local store, not in
provider metadata; a `metadata` field and capability can be added later without breaking this API), Microsoft, CalDAV.

## 11. Known gaps

- **EventKit identifiers and write behavior (unverified).** EventKit's `eventIdentifier` is, to our knowledge, shared by
  every occurrence of a series (TimeTug's wrapper id already appends the start time for this reason), so the read model's
  per-instance `eventID` promise is not kept for EventKit; writes identify the occurrence by `EventRef.originalStart`
  (`occurrenceDate`). Reads now fill `seriesID`, `originalStart` and `version`. The EventKit spike has not been run: it
  must confirm the shared identifier, the predicate lookup of a moved occurrence, `.futureEvents` on the first
  occurrence, `refresh()` on a deleted event and `lastModifiedDate` granularity. Run
  `TIMETUG_LIVE_EVENTKIT=1 swift test --package-path Packages/EventKitSource --filter eventKit` (see the spec's Risks).
- **Google write behavior beyond the fake transport (unverified).** `supportsAttachments=true` on a split with
  attachments, the Meet re-request on a new series, and the `COUNT` arithmetic (assumes `events.instances?showDeleted=true`
  includes EXDATE'd occurrences) are checked only against the fake transport until the Google smoke test confirms them.
  The same goes for how a split treats exceptions after the split point (see the known limitations in part 10).
- **Reminder defaults.** A provider's "use default reminders" and "no reminders" both read as an empty list.
- **Google OAuth client in release builds.** The client id and secret are injected from git-ignored configuration; CI
  injection for release builds is a separate follow-up.
- **No CalDAV or Microsoft connector yet.** They are the next providers; their `AuthorizationMethod` cases (`.password`,
  `.oauth`) are already in the contract.

## Calendar identity and permissions

- **Service vs provider.** `service` is the connector a calendar is read through; `provider` is who hosts it. A Google calendar read directly has service and provider Google; read through Apple Calendar its service is EventKit and its provider is whatever EventKit reports. EventKit's provider comes from the account **type** only (`.local`, `.exchange` gives Microsoft, `.mobileMe` gives iCloud, `.calDAV` gives CalDAV, subscribed and birthday feeds give subscription); the account title is user-editable and never read. The same calendar may therefore read as CalDAV through EventKit and as Google through a direct connection.
- **`isDefault`.** Google: `primary`. EventKit has one default calendar across all accounts: true for it, false for its siblings in the same account, nil for other accounts.
- **Permissions.** Google roles map onto Graph-style flags (owner: view, edit, share, private; writer: view, edit, private; reader: view; freeBusyReader: none). EventKit: `canViewDetails` true, `canEdit` = `allowsContentModifications`, `canShare` and `canViewPrivate` nil (listed as `.permissionDetails` only by sources that fill them).
- **Availability.** `Availability.closest(in:)` maps a value a calendar cannot store to the nearest one it can (tentative and unavailable to busy then free, busy to free, free to busy). A write returns the provider's stored copy; `EventDraft.adjustments(comparedTo:)` and `EventPatch.adjustments(comparedTo:)` report `.availability` and `.visibility` when the stored value differs. Reminder defaults, attendee order, all-day zones and text are provider normalization, not reported.
- **Event dates.** `lastModified` and `created` (nil when the source does not say); `version` stays the opaque equality token.

- **Copying and duplicates.** `EventDraft.uid` (only a `.global` uid is copied) makes `create` look for that event on the target calendar first; if it exists, `create` throws `WriteError.alreadyExists(storedCopy)` and writes nothing. Google stores the uid as `iCalUID` (unverified live); EventKit cannot set a UID and matches on `calendarItemExternalIdentifier` among the events around the draft's time.
