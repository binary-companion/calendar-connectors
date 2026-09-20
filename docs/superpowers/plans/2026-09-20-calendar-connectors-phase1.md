# Calendar Connectors Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the portable `Packages/CalendarConnectors` Swift package: a provider-neutral calendar model and source interface, OAuth2 PKCE plumbing, a polling change monitor, and a read-only Google Calendar connector. TimeTug does not use it yet.

**Architecture:** `CalendarCore` (pure Swift 6, Linux-buildable) holds the model, protocols, connection/credential abstractions, OAuth helpers and `ChangeMonitor`. `GoogleCalendar` implements `ConnectorKind` and `CalendarSource` over Google's REST API through an injected `HTTPTransport`. `CalendarTestSupport` (not a product) holds fakes shared by both test targets. Sync detection uses per-calendar Google `syncToken`s kept in an injected `SyncStateStore`.

**Tech Stack:** Swift 6.4 toolchain, swift-tools 6.0 (Swift 6 mode), Swift Testing, `apple/swift-crypto` (`Crypto`) for SHA-256. No other dependencies.

Spec: `docs/superpowers/specs/2026-09-20-calendar-connectors-phase1-design.md`. Read it and `AGENTS.md` first.

## Global Constraints

- The package lives at `Packages/CalendarConnectors`, has NO dependency on any TimeTug package, and nothing under `Apps/macOS`, `Packages/TimeTugCore` or `Packages/EventKitSource` changes in this phase (only CI, `AGENTS.md`, `docs/`).
- `CalendarCore` and `GoogleCalendar`: swift-tools 6.0, Swift 6 language mode, `platforms: [.macOS(.v14)]`, no Apple-only imports (no `CryptoKit`, `Security`, `AppKit`, `Network`). Networking through the injected `HTTPTransport`; `import FoundationNetworking` only under `#if canImport(FoundationNetworking)`.
- Library logic never calls `Date()`; `now` is an injected `@Sendable () -> Date` closure (only the default arguments of public initializers may write `{ Date() }`). Sleeping goes through the injected `Sleeper`.
- Every behavior gets a Swift Testing test written first (`import Testing`, `@Test`, `#expect`).
- Library errors are `SourceError`/`AuthorizationError`; provider-specific types never leak out of `GoogleCalendar` (`GoogleAPIError` is internal).
- Google scopes: `https://www.googleapis.com/auth/calendar.events` and `https://www.googleapis.com/auth/calendar.calendarlist.readonly`. OAuth params `access_type=offline`, `prompt=consent`. Loopback redirect (supplied by the host through `OAuthRedirectSession`).
- Google `syncToken` requests always send `showDeleted=true` (Google rejects `false` with a token) and never send `timeMin`/`timeMax`. Bootstrap pages use `maxResults=2500`; incremental pages `maxResults=250`. No page cap.
- All-day events: `start` = midnight of the first day in `timeZone` (non-nil for all-day), `end` exclusive.
- Secrets: `CredentialStore` is a keyed map per connection; Google uses key `refresh_token`. No secret, client id or client secret value is ever committed or logged.
- Commit messages end with `Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>`.
- DeepSeek reviews use the `deepseek-review` skill only; never send secrets; verify findings against the repo.
- Commands run from the repo root. Test command: `swift test --package-path Packages/CalendarConnectors`. First resolve needs network (swift-crypto).

## File Structure

```
Packages/CalendarConnectors/
  Package.swift
  Sources/CalendarCore/
    Model.swift              value types: CalendarEvent, CalendarDescriptor, Attendee, ...
    SourceTypes.swift        SourceError, SourceCapabilities, CalendarChange, CalendarSource, PollingCalendarSource
    Connection.swift         Connection, Platform, CredentialStore, SyncStateStore, in-memory stores, auth protocols, ConnectorKind, ConnectorRegistry
    HTTP.swift               HTTPRequest/HTTPResponse/HTTPTransport/URLSessionTransport
    PKCE.swift               PKCE helpers
    OAuth.swift              OAuthConfig, OAuthTokens, OAuthClient, AuthorizationError
    AccessTokenProvider.swift
    ChangeMonitor.swift      Sleeper + ChangeMonitor
  Sources/CalendarTestSupport/
    FakeTransport.swift      FakeTransport, HTTPResponse.json, TestNow
  Sources/GoogleCalendar/
    GoogleDTOs.swift         Decodable wire types
    GoogleEventMapper.swift  DTO -> CalendarEvent / CalendarDescriptor (pure)
    GoogleAPIClient.swift    authenticated GET, retries, paging, GoogleAPIError
    GoogleCalendarSource.swift  calendars(), events(in:), changes(), checkForChanges()
    GoogleConnectorKind.swift   GoogleOAuthConfig, authorize/reauthorize/makeSource
  Tests/CalendarCoreTests/  ModelTests, ConnectionTests, PKCETests, OAuthTests, AccessTokenProviderTests, ChangeMonitorTests
  Tests/GoogleCalendarTests/ MapperTests, SourceTests, ChangeDetectionTests, ConnectorKindTests
docs/decisions/0012-calendar-connector-library.md
AGENTS.md, docs/architecture.md, .github/workflows/ci.yml   (modified)
```

---

### Task 1: Package scaffold, model and source interfaces

**Files:**
- Create: `Packages/CalendarConnectors/Package.swift`, `Sources/CalendarCore/Model.swift`, `Sources/CalendarCore/SourceTypes.swift`, `Sources/CalendarTestSupport/FakeTransport.swift` (placeholder created in Task 3; for now a one-line file), `Sources/GoogleCalendar/Placeholder.swift`, `Tests/CalendarCoreTests/ModelTests.swift`, `Tests/GoogleCalendarTests/Placeholder.swift`
- Modify: root `.gitignore` only if `Packages/CalendarConnectors/.build` is not already ignored (check with `git check-ignore`).

**Interfaces:**
- Produces: all `Model.swift` and `SourceTypes.swift` types exactly as written below (later tasks use them verbatim).

- [ ] **Step 1: Find and pin swift-crypto**

Run: `git ls-remote --tags https://github.com/apple/swift-crypto.git | awk '{print $2}' | grep -E 'refs/tags/[0-9]+\.[0-9]+\.[0-9]+$' | sort -V | tail -1`
Expected: a tag such as `refs/tags/5.0.0`. Use that version as `SWIFT_CRYPTO_VERSION` below (the example uses `5.0.0`).

- [ ] **Step 2: Create the package manifest and stubs**

`Packages/CalendarConnectors/Package.swift`:
```swift
// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "CalendarConnectors",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "CalendarCore", targets: ["CalendarCore"]),
        .library(name: "GoogleCalendar", targets: ["GoogleCalendar"]),
    ],
    dependencies: [
        .package(url: "https://github.com/apple/swift-crypto.git", exact: "5.0.0"),
    ],
    targets: [
        .target(name: "CalendarCore", dependencies: [.product(name: "Crypto", package: "swift-crypto")]),
        .target(name: "GoogleCalendar", dependencies: ["CalendarCore"]),
        .target(name: "CalendarTestSupport", dependencies: ["CalendarCore"]),
        .testTarget(name: "CalendarCoreTests", dependencies: ["CalendarCore", "CalendarTestSupport"]),
        .testTarget(name: "GoogleCalendarTests", dependencies: ["GoogleCalendar", "CalendarCore", "CalendarTestSupport"]),
    ]
)
```
`Sources/GoogleCalendar/Placeholder.swift`: `import CalendarCore` (one line; deleted in Task 7). `Sources/CalendarTestSupport/FakeTransport.swift`: `import CalendarCore` (replaced in Task 3). `Tests/GoogleCalendarTests/Placeholder.swift`: `import Testing` (deleted in Task 7).

- [ ] **Step 3: Write the failing test**

`Packages/CalendarConnectors/Tests/CalendarCoreTests/ModelTests.swift`:
```swift
import Foundation
import Testing
@testable import CalendarCore

@Test func eventIDIsUniqueAcrossCalendars() {
    let start = Date(timeIntervalSince1970: 0)
    let a = CalendarEvent(eventID: "e1", calendarID: "work", title: "T", start: start, end: start.addingTimeInterval(60))
    let b = CalendarEvent(eventID: "e1", calendarID: "home", title: "T", start: start, end: start.addingTimeInterval(60))
    #expect(a.id == "work/e1")
    #expect(a.id != b.id)
}

@Test func descriptorNormalizesColor() {
    func hex(_ raw: String?) -> String? {
        CalendarDescriptor(id: "c", title: "C", colorHex: raw).colorHex
    }
    #expect(hex("#9fe1e7") == "#9FE1E7")
    #expect(hex("9fe1e7") == "#9FE1E7")
    #expect(hex("#abc") == "#AABBCC")
    #expect(hex("nope") == nil)
    #expect(hex(nil) == nil)
}

@Test func attendeeNormalizesEmail() {
    #expect(Attendee(email: "  Ann@Example.COM ").email == "ann@example.com")
    #expect(Attendee(email: "   ").email == nil)
}

@Test func capabilitiesDefaultToReadOnlyAndNoSync() {
    let c = SourceCapabilities()
    #expect(!c.canWrite && !c.canEditAttendees && !c.canRespondToInvite && !c.providesConference && !c.supportsPush)
    #expect(c.syncKind == .none)
}
```

- [ ] **Step 4: Run test to verify it fails**

Run: `swift test --package-path Packages/CalendarConnectors --filter ModelTests`
Expected: FAIL to compile ("cannot find 'CalendarEvent' in scope"; `CalendarCoreTests` sources reference missing types).

- [ ] **Step 5: Write the model**

`Sources/CalendarCore/Model.swift`:
```swift
import Foundation

public enum AccessRole: String, Sendable { case owner, writer, reader, freeBusyReader }
public enum EventStatus: String, Sendable { case confirmed, tentative, cancelled }
public enum Availability: String, Sendable { case busy, free }
public enum Visibility: String, Sendable { case `default`, publicEvent, privateEvent, confidential }
public enum EventKind: String, Sendable { case standard, focusTime, outOfOffice, workingLocation, birthday, other }
public enum ResponseStatus: String, Sendable { case accepted, tentative, declined, needsAction }
public enum AttendeeRole: String, Sendable { case required, optional, resource }
public enum ConferenceProvider: String, Sendable { case meet, teams, zoom, other }

public struct Attendee: Hashable, Sendable {
    public var name: String?
    public private(set) var email: String?
    public var role: AttendeeRole
    public var response: ResponseStatus
    public var isSelf: Bool
    public var isOrganizer: Bool

    public init(
        name: String? = nil, email: String? = nil, role: AttendeeRole = .required,
        response: ResponseStatus = .needsAction, isSelf: Bool = false, isOrganizer: Bool = false
    ) {
        self.name = name
        let trimmed = email?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        self.email = (trimmed?.isEmpty ?? true) ? nil : trimmed
        self.role = role
        self.response = response
        self.isSelf = isSelf
        self.isOrganizer = isOrganizer
    }
}

public struct ConferenceInfo: Hashable, Sendable {
    public var url: URL
    public var provider: ConferenceProvider
    public init(url: URL, provider: ConferenceProvider) {
        self.url = url
        self.provider = provider
    }
}

public struct Reminder: Hashable, Sendable {
    public var minutesBefore: Int
    public init(minutesBefore: Int) { self.minutesBefore = minutesBefore }
}

/// One calendar within an account.
public struct CalendarDescriptor: Hashable, Sendable, Identifiable {
    public var id: String
    public var title: String
    /// "#RRGGBB" uppercase, or nil when unknown or invalid.
    public private(set) var colorHex: String?
    public var accessRole: AccessRole
    public var isPrimary: Bool
    public var timeZone: TimeZone?
    /// The owning account, e.g. the signed-in email.
    public var accountName: String?

    public init(
        id: String, title: String, colorHex: String? = nil, accessRole: AccessRole = .reader,
        isPrimary: Bool = false, timeZone: TimeZone? = nil, accountName: String? = nil
    ) {
        self.id = id
        self.title = title
        self.colorHex = Self.normalizedHex(colorHex)
        self.accessRole = accessRole
        self.isPrimary = isPrimary
        self.timeZone = timeZone
        self.accountName = accountName
    }

    /// "#RGB", "#RRGGBB" or "RRGGBB" (any case) to "#RRGGBB" uppercase; nil for anything else.
    public static func normalizedHex(_ raw: String?) -> String? {
        guard var s = raw?.trimmingCharacters(in: .whitespacesAndNewlines) else { return nil }
        if s.hasPrefix("#") { s.removeFirst() }
        guard s.count == 3 || s.count == 6, s.allSatisfy(\.isASCII), s.allSatisfy(\.isHexDigit) else { return nil }
        s = s.uppercased()
        if s.count == 3 { s = s.map { "\($0)\($0)" }.joined() }
        return "#" + s
    }
}

public struct CalendarEvent: Hashable, Sendable, Identifiable {
    /// Unique within its calendar (a recurring instance has its own id).
    public var eventID: String
    /// Unique across the calendars of one source.
    public var id: String { "\(calendarID)/\(eventID)" }
    /// iCalendar UID (Google `iCalUID`); stable across copies of the same meeting.
    public var uid: String?
    public var calendarID: String
    public var title: String
    public var notes: String?
    public var location: String?
    /// All-day: midnight of the first day in `timeZone`, `end` exclusive (midnight after the last day).
    public var start: Date
    public var end: Date
    /// Always non-nil when `isAllDay`; interpret an all-day event's days in this zone, not the device's.
    public var timeZone: TimeZone?
    public var isAllDay: Bool
    public var status: EventStatus
    public var availability: Availability
    public var visibility: Visibility
    public var kind: EventKind
    public var seriesID: String?
    public var originalStart: Date?
    public var attendees: [Attendee]
    public var organizer: Attendee?
    public var conference: ConferenceInfo?
    public var reminders: [Reminder]
    public var url: URL?
    /// Opaque provider version (Google etag); Phase 3 uses it for optimistic writes.
    public var version: String?
    /// The account owner's own response, when the provider says.
    public var myResponse: ResponseStatus?

    public init(
        eventID: String, uid: String? = nil, calendarID: String, title: String,
        notes: String? = nil, location: String? = nil, start: Date, end: Date,
        timeZone: TimeZone? = nil, isAllDay: Bool = false, status: EventStatus = .confirmed,
        availability: Availability = .busy, visibility: Visibility = .default, kind: EventKind = .standard,
        seriesID: String? = nil, originalStart: Date? = nil, attendees: [Attendee] = [],
        organizer: Attendee? = nil, conference: ConferenceInfo? = nil, reminders: [Reminder] = [],
        url: URL? = nil, version: String? = nil, myResponse: ResponseStatus? = nil
    ) {
        self.eventID = eventID
        self.uid = uid
        self.calendarID = calendarID
        self.title = title
        self.notes = notes
        self.location = location
        self.start = start
        self.end = end
        self.timeZone = timeZone
        self.isAllDay = isAllDay
        self.status = status
        self.availability = availability
        self.visibility = visibility
        self.kind = kind
        self.seriesID = seriesID
        self.originalStart = originalStart
        self.attendees = attendees
        self.organizer = organizer
        self.conference = conference
        self.reminders = reminders
        self.url = url
        self.version = version
        self.myResponse = myResponse
    }
}
```

`Sources/CalendarCore/SourceTypes.swift`:
```swift
import Foundation

public enum SyncKind: Sendable { case none, token, notification }

public struct SourceCapabilities: Equatable, Sendable {
    public var canWrite: Bool
    public var canEditAttendees: Bool
    public var canRespondToInvite: Bool
    public var providesConference: Bool
    public var syncKind: SyncKind
    public var supportsPush: Bool

    public init(
        canWrite: Bool = false, canEditAttendees: Bool = false, canRespondToInvite: Bool = false,
        providesConference: Bool = false, syncKind: SyncKind = .none, supportsPush: Bool = false
    ) {
        self.canWrite = canWrite
        self.canEditAttendees = canEditAttendees
        self.canRespondToInvite = canRespondToInvite
        self.providesConference = providesConference
        self.syncKind = syncKind
        self.supportsPush = supportsPush
    }
}

/// What the library reports through `changes()`. Sources throw `SourceError`; they never leak provider types.
public enum SourceError: Error, Sendable, Equatable {
    case authExpired
    case network(String)
    case rateLimited(retryAfter: TimeInterval?)
    case server(status: Int)
    case invalidResponse(String)
}

public enum CalendarChange: Equatable, Sendable {
    case calendarsChanged
    /// nil calendar ids mean the scope is unknown.
    case eventsChanged(calendarIDs: Set<String>?)
    /// Terminal: the stream finishes after this (e.g. `.authExpired`) so the app can surface it and re-authorize.
    case sourceFailed(SourceError)
}

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
    /// One cheap incremental check (sync token / delta). Returns the change since the last call, or nil for none.
    /// The first call establishes the baseline and returns nil.
    func checkForChanges() async throws -> CalendarChange?
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `swift test --package-path Packages/CalendarConnectors --filter ModelTests`
Expected: PASS (4 tests). Also run `git check-ignore Packages/CalendarConnectors/.build`; if it prints nothing add `Packages/CalendarConnectors/.build/` to the root `.gitignore`.

- [ ] **Step 7: Commit**

```bash
git add Packages/CalendarConnectors .gitignore
git commit -m "feat(connectors): package scaffold, model and source interfaces

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 2: Connection, platform, stores, registry

**Files:**
- Create: `Sources/CalendarCore/Connection.swift`, `Tests/CalendarCoreTests/ConnectionTests.swift`

**Interfaces:**
- Consumes: `CalendarSource`, `SourceError` (Task 1).
- Produces (used by Tasks 5, 10): `ConnectionID`, `Connection` (`kindID`, `connectionID`, `displayName`, `config`, `sourceID`), `CredentialField`, `AuthorizationMethod`, `Platform`, `CredentialStore` (`secrets(for:)`, `setSecrets(_:for:)`, `removeSecrets(for:)`), `SyncStateStore` (`token(for:scope:)`, `setToken(_:for:scope:)`, `removeAll(for:)`), `InMemoryCredentialStore`, `InMemorySyncStateStore`, `OAuthRedirectSession`, `AuthorizationInteraction`, `ConnectorKind`, `ConnectorRegistry`.

- [ ] **Step 1: Write the failing tests**

`Tests/CalendarCoreTests/ConnectionTests.swift`:
```swift
import Foundation
import Testing
@testable import CalendarCore

@Test func connectionRoundTripsAndDerivesSourceID() throws {
    let c = Connection(kindID: "google", connectionID: "abc", displayName: "me@x.com", config: ["email": "me@x.com"])
    let data = try JSONEncoder().encode(c)
    #expect(try JSONDecoder().decode(Connection.self, from: data) == c)
    #expect(c.sourceID == "google-abc")
    #expect(c.id == "abc")
}

@Test func connectionDecodesWithUnknownKeys() throws {
    let json = #"{"kindID":"google","connectionID":"a","displayName":"d","config":{},"future":1}"#
    let c = try JSONDecoder().decode(Connection.self, from: Data(json.utf8))
    #expect(c.kindID == "google")
}

@Test func inMemoryCredentialStoreStoresAndRemoves() async throws {
    let store = InMemoryCredentialStore()
    #expect(try await store.secrets(for: "c1") == nil)
    try await store.setSecrets(["refresh_token": "r"], for: "c1")
    #expect(try await store.secrets(for: "c1") == ["refresh_token": "r"])
    try await store.removeSecrets(for: "c1")
    #expect(try await store.secrets(for: "c1") == nil)
}

@Test func inMemorySyncStateScopesTokensAndRemovesAll() async {
    let store = InMemorySyncStateStore()
    await store.setToken("t1", for: "c1", scope: "calA")
    await store.setToken("t2", for: "c1", scope: "calB")
    await store.setToken("t3", for: "c2", scope: "calA")
    #expect(await store.token(for: "c1", scope: "calA") == "t1")
    await store.setToken(nil, for: "c1", scope: "calA")
    #expect(await store.token(for: "c1", scope: "calA") == nil)
    await store.removeAll(for: "c1")
    #expect(await store.token(for: "c1", scope: "calB") == nil)
    #expect(await store.token(for: "c2", scope: "calA") == "t3")
}

private struct StubKind: ConnectorKind {
    let id: String
    let supportedPlatforms: Platform
    var displayName: String { id }
    var authorization: AuthorizationMethod { .oauth }
    func authorize(using: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        Connection(kindID: id, connectionID: "x", displayName: "x")
    }
    func reauthorize(_ connection: Connection, using: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection { connection }
    func makeSource(for connection: Connection, credentials: any CredentialStore, syncState: any SyncStateStore) throws -> any CalendarSource {
        throw SourceError.invalidResponse("stub")
    }
}

@Test func registryFiltersByPlatform() {
    var registry = ConnectorRegistry()
    registry.register(StubKind(id: "google", supportedPlatforms: [.macOS, .linux]))
    registry.register(StubKind(id: "eventkit", supportedPlatforms: [.macOS]))
    #expect(registry.kind(id: "google")?.id == "google")
    #expect(registry.kind(id: "missing") == nil)
    #expect(Set(registry.kinds(for: .macOS).map(\.id)) == ["google", "eventkit"])
    #expect(registry.kinds(for: .linux).map(\.id) == ["google"])
}
```

- [ ] **Step 2: Run to verify failure**

Run: `swift test --package-path Packages/CalendarConnectors --filter ConnectionTests`
Expected: FAIL to compile ("cannot find 'Connection' in scope").

- [ ] **Step 3: Implement**

`Sources/CalendarCore/Connection.swift`:
```swift
import Foundation

public typealias ConnectionID = String

/// Non-secret description of a connected account. The host app persists it (JSON, UserDefaults, ...).
public struct Connection: Codable, Hashable, Sendable, Identifiable {
    public var kindID: String
    public var connectionID: ConnectionID
    public var displayName: String
    public var config: [String: String]

    public init(kindID: String, connectionID: ConnectionID, displayName: String, config: [String: String] = [:]) {
        self.kindID = kindID
        self.connectionID = connectionID
        self.displayName = displayName
        self.config = config
    }

    public var id: ConnectionID { connectionID }
    /// The `CalendarSource.id` for this connection. Every consumer must use this helper.
    public var sourceID: String { "\(kindID)-\(connectionID)" }
}

public struct CredentialField: Hashable, Sendable {
    public var key: String
    public var label: String
    public var isSecret: Bool
    public init(key: String, label: String, isSecret: Bool = false) {
        self.key = key
        self.label = label
        self.isSecret = isSecret
    }
}

public enum AuthorizationMethod: Sendable {
    case oauth
    case password(fields: [CredentialField])
    case system
}

public struct Platform: OptionSet, Sendable {
    public let rawValue: Int
    public init(rawValue: Int) { self.rawValue = rawValue }
    public static let macOS = Platform(rawValue: 1 << 0)
    public static let iOS = Platform(rawValue: 1 << 1)
    public static let linux = Platform(rawValue: 1 << 2)
    public static let windows = Platform(rawValue: 1 << 3)

    public static var current: Platform {
        #if os(macOS)
        return .macOS
        #elseif os(iOS)
        return .iOS
        #elseif os(Linux)
        return .linux
        #elseif os(Windows)
        return .windows
        #else
        return []
        #endif
    }
}

/// A keyed map of secrets per connection (Google stores `["refresh_token": ...]`).
/// The host supplies the real store (TimeTug: Keychain). The library ships only an in-memory one.
public protocol CredentialStore: Sendable {
    func secrets(for connectionID: ConnectionID) async throws -> [String: String]?
    func setSecrets(_ secrets: [String: String], for connectionID: ConnectionID) async throws
    func removeSecrets(for connectionID: ConnectionID) async throws
}

public protocol SyncStateStore: Sendable {
    func token(for connectionID: ConnectionID, scope: String) async -> String?
    func setToken(_ token: String?, for connectionID: ConnectionID, scope: String) async
    func removeAll(for connectionID: ConnectionID) async
}

public actor InMemoryCredentialStore: CredentialStore {
    private var storage: [ConnectionID: [String: String]] = [:]
    public init() {}
    public func secrets(for connectionID: ConnectionID) async throws -> [String: String]? { storage[connectionID] }
    public func setSecrets(_ secrets: [String: String], for connectionID: ConnectionID) async throws {
        storage[connectionID] = secrets
    }
    public func removeSecrets(for connectionID: ConnectionID) async throws { storage[connectionID] = nil }
}

public actor InMemorySyncStateStore: SyncStateStore {
    private var storage: [ConnectionID: [String: String]] = [:]
    public init() {}
    public func token(for connectionID: ConnectionID, scope: String) async -> String? { storage[connectionID]?[scope] }
    public func setToken(_ token: String?, for connectionID: ConnectionID, scope: String) async {
        storage[connectionID, default: [:]][scope] = token
    }
    public func removeAll(for connectionID: ConnectionID) async { storage[connectionID] = nil }
}

/// One in-progress OAuth redirect (e.g. a loopback listener) owned by the host app.
public protocol OAuthRedirectSession: Sendable {
    /// The exact redirect URI to put in the authorization URL and the token exchange, e.g. `http://127.0.0.1:53211`.
    var redirectURI: URL { get }
    /// Opens `authorizationURL` for the user, waits for the redirect and returns the redirect URL received.
    func authorize(at authorizationURL: URL) async throws -> URL
    func close() async
}

/// Implemented by the host app: the only OS/UI-specific part of authorization.
public protocol AuthorizationInteraction: Sendable {
    func beginOAuthRedirect() async throws -> any OAuthRedirectSession
    func promptCredentials(_ fields: [CredentialField]) async throws -> [String: String]
}

public protocol ConnectorKind: Sendable {
    var id: String { get }
    var displayName: String { get }
    var supportedPlatforms: Platform { get }
    var authorization: AuthorizationMethod { get }
    func authorize(using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection
    /// Re-runs sign-in for an existing connection (after `authExpired`). Keeps its `connectionID` (so `sourceID`,
    /// calendar keys and sync state stay valid), replaces the secrets, and throws `SourceError.invalidResponse`
    /// if the user signs in as a different account.
    func reauthorize(
        _ connection: Connection, using interaction: any AuthorizationInteraction, credentials: any CredentialStore
    ) async throws -> Connection
    func makeSource(
        for connection: Connection, credentials: any CredentialStore, syncState: any SyncStateStore
    ) throws -> any CalendarSource
}

public struct ConnectorRegistry: Sendable {
    private var kinds: [String: any ConnectorKind] = [:]
    public init() {}
    public mutating func register(_ kind: any ConnectorKind) { kinds[kind.id] = kind }
    public func kind(id: String) -> (any ConnectorKind)? { kinds[id] }
    /// Kinds usable on `platform`, sorted by id.
    public func kinds(for platform: Platform) -> [any ConnectorKind] {
        kinds.values.filter { !$0.supportedPlatforms.isDisjoint(with: platform) }.sorted { $0.id < $1.id }
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `swift test --package-path Packages/CalendarConnectors --filter ConnectionTests`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "feat(connectors): connection, credential/sync stores, auth protocols, registry

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 3: HTTP transport, test fakes, PKCE

**Files:**
- Create: `Sources/CalendarCore/HTTP.swift`, `Sources/CalendarCore/PKCE.swift`, `Tests/CalendarCoreTests/PKCETests.swift`
- Replace: `Sources/CalendarTestSupport/FakeTransport.swift`

**Interfaces:**
- Produces: `HTTPRequest(url:method:headers:body:)`, `HTTPResponse(status:headers:body:)` with `header(_:)`, `HTTPTransport.send(_:)`, `URLSessionTransport`, `FakeTransport` (`route(_:_:)`, `requests`, `requests(matching:)`), `HTTPResponse.json(_:status:headers:)`, `HTTPResponse.text(_:status:)`, `TestNow` (`date`, `advance(_:)`, `provider`), `PKCE.challenge(for:)`, `PKCE.randomString(length:)`.

- [ ] **Step 1: Write the failing PKCE test**

`Tests/CalendarCoreTests/PKCETests.swift`:
```swift
import Foundation
import Testing
@testable import CalendarCore

@Test func pkceChallengeMatchesRFC7636Vector() {
    #expect(PKCE.challenge(for: "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk")
        == "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
}

@Test func pkceRandomStringUsesUnreservedAlphabetAndLength() {
    let s = PKCE.randomString(length: 64)
    #expect(s.count == 64)
    let allowed = Set("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")
    #expect(s.allSatisfy { allowed.contains($0) })
    #expect(PKCE.randomString(length: 64) != s)
}
```

- [ ] **Step 2: Run to verify failure**

Run: `swift test --package-path Packages/CalendarConnectors --filter PKCETests`
Expected: FAIL to compile ("cannot find 'PKCE' in scope").

- [ ] **Step 3: Implement HTTP and PKCE**

`Sources/CalendarCore/HTTP.swift`:
```swift
import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

public struct HTTPRequest: Sendable {
    public var url: URL
    public var method: String
    public var headers: [String: String]
    public var body: Data?
    public init(url: URL, method: String = "GET", headers: [String: String] = [:], body: Data? = nil) {
        self.url = url
        self.method = method
        self.headers = headers
        self.body = body
    }
}

public struct HTTPResponse: Sendable {
    public var status: Int
    /// Header names lowercased.
    public var headers: [String: String]
    public var body: Data
    public init(status: Int, headers: [String: String] = [:], body: Data = Data()) {
        self.status = status
        self.headers = Dictionary(headers.map { ($0.key.lowercased(), $0.value) }, uniquingKeysWith: { _, last in last })
        self.body = body
    }
    public func header(_ name: String) -> String? { headers[name.lowercased()] }
}

public protocol HTTPTransport: Sendable {
    /// Returns any HTTP response (including 4xx/5xx). Throws `SourceError.network` for transport failures.
    func send(_ request: HTTPRequest) async throws -> HTTPResponse
}

public struct URLSessionTransport: HTTPTransport {
    private let session: URLSession
    public init(session: URLSession = .shared) { self.session = session }

    public func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        var urlRequest = URLRequest(url: request.url)
        urlRequest.httpMethod = request.method
        urlRequest.httpBody = request.body
        for (name, value) in request.headers { urlRequest.setValue(value, forHTTPHeaderField: name) }
        do {
            let (data, response) = try await session.data(for: urlRequest)
            guard let http = response as? HTTPURLResponse else { throw SourceError.invalidResponse("not an HTTP response") }
            var headers: [String: String] = [:]
            for (key, value) in http.allHeaderFields { headers["\(key)"] = "\(value)" }
            return HTTPResponse(status: http.statusCode, headers: headers, body: data)
        } catch let error as URLError {
            if error.code == .cancelled { throw CancellationError() }
            throw SourceError.network(error.localizedDescription)
        }
    }
}
```

`Sources/CalendarCore/PKCE.swift`:
```swift
import Crypto
import Foundation

public enum PKCE {
    private static let alphabet = Array("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")

    public static func randomString(length: Int = 64, using generator: inout some RandomNumberGenerator) -> String {
        String((0..<length).map { _ in alphabet.randomElement(using: &generator)! })
    }

    public static func randomString(length: Int = 64) -> String {
        var generator = SystemRandomNumberGenerator()
        return randomString(length: length, using: &generator)
    }

    /// base64url(SHA-256(verifier)) without padding (RFC 7636, method S256).
    public static func challenge(for verifier: String) -> String {
        let digest = SHA256.hash(data: Data(verifier.utf8))
        return Data(digest).base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
}
```

`Sources/CalendarTestSupport/FakeTransport.swift`:
```swift
import CalendarCore
import Foundation

/// Routes requests by URL substring. The MOST RECENTLY registered route whose text is contained in the request URL
/// answers (so register general routes first and specific ones after); it consumes its responses in order and
/// repeats the last one. Unmatched requests get 404.
public actor FakeTransport: HTTPTransport {
    private struct Route {
        let match: String
        var responses: [HTTPResponse]
    }
    private var routes: [Route] = []
    public private(set) var requests: [HTTPRequest] = []

    public init() {}

    public func route(_ match: String, _ responses: [HTTPResponse]) {
        routes.append(Route(match: match, responses: responses))
    }

    public func requests(matching text: String) -> [HTTPRequest] {
        requests.filter { $0.url.absoluteString.contains(text) }
    }

    public func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        requests.append(request)
        let url = request.url.absoluteString
        for index in routes.indices.reversed() where url.contains(routes[index].match) {
            if routes[index].responses.count > 1 { return routes[index].responses.removeFirst() }
            return routes[index].responses[0]
        }
        return HTTPResponse(status: 404, body: Data("no route for \(url)".utf8))
    }
}

extension HTTPResponse {
    public static func json(_ object: Any, status: Int = 200, headers: [String: String] = [:]) -> HTTPResponse {
        let data = (try? JSONSerialization.data(withJSONObject: object)) ?? Data()
        return HTTPResponse(status: status, headers: headers, body: data)
    }
    public static func text(_ string: String, status: Int = 200) -> HTTPResponse {
        HTTPResponse(status: status, body: Data(string.utf8))
    }
}

/// A controllable clock for tests: `provider` is the `@Sendable () -> Date` the library takes.
public final class TestNow: @unchecked Sendable {
    private let lock = NSLock()
    private var value: Date
    public init(_ start: Date = Date(timeIntervalSince1970: 1_800_000_000)) { value = start }
    public var date: Date { lock.withLock { value } }
    public func advance(_ seconds: TimeInterval) { lock.withLock { value = value.addingTimeInterval(seconds) } }
    public var provider: @Sendable () -> Date { { [self] in date } }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `swift test --package-path Packages/CalendarConnectors --filter PKCETests`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "feat(connectors): HTTP transport, PKCE, test fakes

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 4: OAuth client

**Files:**
- Create: `Sources/CalendarCore/OAuth.swift`, `Tests/CalendarCoreTests/OAuthTests.swift`

**Interfaces:**
- Consumes: `HTTPTransport`, `HTTPRequest`, `HTTPResponse`, `SourceError`, `FakeTransport`, `TestNow`.
- Produces: `AuthorizationError` (`.cancelled`, `.stateMismatch`, `.missingCode`), `OAuthConfig(authorizationEndpoint:tokenEndpoint:clientID:clientSecret:scopes:extraAuthParams:)`, `OAuthTokens(accessToken:expiresAt:refreshToken:)`, `OAuthClient(config:transport:now:)` with `authorizationURL(redirectURI:state:codeChallenge:)`, `authorizationCode(from:expectedState:)`, `exchange(code:verifier:redirectURI:)`, `refresh(refreshToken:)`.

- [ ] **Step 1: Write the failing tests**

`Tests/CalendarCoreTests/OAuthTests.swift`:
```swift
import CalendarTestSupport
import Foundation
import Testing
@testable import CalendarCore

private let config = OAuthConfig(
    authorizationEndpoint: URL(string: "https://accounts.example.com/auth")!,
    tokenEndpoint: URL(string: "https://oauth.example.com/token")!,
    clientID: "client-1", clientSecret: "shh",
    scopes: ["scope.a", "scope.b"], extraAuthParams: ["access_type": "offline", "prompt": "consent"])

private func client(_ transport: FakeTransport, now: TestNow = TestNow()) -> OAuthClient {
    OAuthClient(config: config, transport: transport, now: now.provider)
}

private func form(_ request: HTTPRequest) -> [String: String] {
    let body = String(decoding: request.body ?? Data(), as: UTF8.self)
    var out: [String: String] = [:]
    for pair in body.split(separator: "&") {
        let parts = pair.split(separator: "=", maxSplits: 1).map(String.init)
        out[parts[0].removingPercentEncoding ?? parts[0]] = (parts.count > 1 ? parts[1] : "").removingPercentEncoding
    }
    return out
}

@Test func authorizationURLCarriesPKCEScopesAndExtras() throws {
    let url = client(FakeTransport()).authorizationURL(
        redirectURI: URL(string: "http://127.0.0.1:5000")!, state: "st", codeChallenge: "ch")
    let items = Dictionary(uniqueKeysWithValues: URLComponents(url: url, resolvingAgainstBaseURL: false)!.queryItems!.map { ($0.name, $0.value ?? "") })
    #expect(url.host == "accounts.example.com")
    #expect(items["client_id"] == "client-1")
    #expect(items["redirect_uri"] == "http://127.0.0.1:5000")
    #expect(items["response_type"] == "code")
    #expect(items["scope"] == "scope.a scope.b")
    #expect(items["state"] == "st")
    #expect(items["code_challenge"] == "ch")
    #expect(items["code_challenge_method"] == "S256")
    #expect(items["access_type"] == "offline")
    #expect(items["prompt"] == "consent")
}

@Test func authorizationCodeValidatesStateAndErrors() throws {
    let c = client(FakeTransport())
    #expect(try c.authorizationCode(from: URL(string: "http://127.0.0.1:5000/?code=abc&state=st")!, expectedState: "st") == "abc")
    #expect(throws: AuthorizationError.stateMismatch) {
        try c.authorizationCode(from: URL(string: "http://127.0.0.1:5000/?code=abc&state=other")!, expectedState: "st")
    }
    #expect(throws: AuthorizationError.cancelled) {
        try c.authorizationCode(from: URL(string: "http://127.0.0.1:5000/?error=access_denied&state=st")!, expectedState: "st")
    }
    #expect(throws: AuthorizationError.missingCode) {
        try c.authorizationCode(from: URL(string: "http://127.0.0.1:5000/?state=st")!, expectedState: "st")
    }
    #expect(throws: SourceError.invalidResponse("oauth error: server_error")) {
        try c.authorizationCode(from: URL(string: "http://127.0.0.1:5000/?error=server_error&state=st")!, expectedState: "st")
    }
}

@Test func exchangePostsFormAndParsesTokens() async throws {
    let transport = FakeTransport()
    await transport.route("oauth.example.com/token", [.json(["access_token": "at", "expires_in": 3600, "refresh_token": "rt"])])
    let now = TestNow()
    let tokens = try await client(transport, now: now).exchange(
        code: "the+code/1", verifier: "ver", redirectURI: URL(string: "http://127.0.0.1:5000")!)
    #expect(tokens.accessToken == "at")
    #expect(tokens.refreshToken == "rt")
    #expect(tokens.expiresAt == now.date.addingTimeInterval(3600))
    let request = try #require(await transport.requests.first)
    #expect(request.method == "POST")
    #expect(request.headers["Content-Type"] == "application/x-www-form-urlencoded")
    let f = form(request)
    #expect(f["grant_type"] == "authorization_code")
    #expect(f["code"] == "the+code/1")
    #expect(f["code_verifier"] == "ver")
    #expect(f["client_id"] == "client-1")
    #expect(f["client_secret"] == "shh")
    #expect(f["redirect_uri"] == "http://127.0.0.1:5000")
}

@Test func refreshMapsInvalidGrantToAuthExpired() async throws {
    let transport = FakeTransport()
    await transport.route("token", [.json(["error": "invalid_grant"], status: 400)])
    await #expect(throws: SourceError.authExpired) {
        try await client(transport).refresh(refreshToken: "old")
    }
}

@Test func refreshSendsGrantAndKeepsNilRefreshToken() async throws {
    let transport = FakeTransport()
    await transport.route("token", [.json(["access_token": "at2", "expires_in": 100])])
    let tokens = try await client(transport).refresh(refreshToken: "rt")
    #expect(tokens.refreshToken == nil)
    let f = form(try #require(await transport.requests.first))
    #expect(f["grant_type"] == "refresh_token")
    #expect(f["refresh_token"] == "rt")
}

@Test func exchangeOtherOAuthErrorsAreInvalidResponse() async throws {
    let transport = FakeTransport()
    await transport.route("token", [.json(["error": "invalid_grant"], status: 400)])
    await #expect(throws: SourceError.invalidResponse("oauth error: invalid_grant")) {
        try await client(transport).exchange(code: "c", verifier: "v", redirectURI: URL(string: "http://127.0.0.1:1")!)
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `swift test --package-path Packages/CalendarConnectors --filter OAuthTests`
Expected: FAIL to compile ("cannot find 'OAuthConfig' in scope").

- [ ] **Step 3: Implement**

`Sources/CalendarCore/OAuth.swift`:
```swift
import Foundation

public enum AuthorizationError: Error, Sendable, Equatable {
    /// The user declined or closed the sign-in.
    case cancelled
    case stateMismatch
    case missingCode
}

public struct OAuthConfig: Sendable {
    public var authorizationEndpoint: URL
    public var tokenEndpoint: URL
    public var clientID: String
    /// For desktop clients this is not confidential, but the token endpoint still requires it. Supplied by the host app.
    public var clientSecret: String?
    public var scopes: [String]
    public var extraAuthParams: [String: String]

    public init(
        authorizationEndpoint: URL, tokenEndpoint: URL, clientID: String, clientSecret: String? = nil,
        scopes: [String], extraAuthParams: [String: String] = [:]
    ) {
        self.authorizationEndpoint = authorizationEndpoint
        self.tokenEndpoint = tokenEndpoint
        self.clientID = clientID
        self.clientSecret = clientSecret
        self.scopes = scopes
        self.extraAuthParams = extraAuthParams
    }
}

public struct OAuthTokens: Sendable, Equatable {
    public var accessToken: String
    public var expiresAt: Date
    public var refreshToken: String?
    public init(accessToken: String, expiresAt: Date, refreshToken: String? = nil) {
        self.accessToken = accessToken
        self.expiresAt = expiresAt
        self.refreshToken = refreshToken
    }
}

public struct OAuthClient: Sendable {
    private let config: OAuthConfig
    private let transport: any HTTPTransport
    private let now: @Sendable () -> Date

    public init(config: OAuthConfig, transport: any HTTPTransport, now: @escaping @Sendable () -> Date) {
        self.config = config
        self.transport = transport
        self.now = now
    }

    public func authorizationURL(redirectURI: URL, state: String, codeChallenge: String) -> URL {
        var items = [
            URLQueryItem(name: "client_id", value: config.clientID),
            URLQueryItem(name: "redirect_uri", value: redirectURI.absoluteString),
            URLQueryItem(name: "response_type", value: "code"),
            URLQueryItem(name: "scope", value: config.scopes.joined(separator: " ")),
            URLQueryItem(name: "state", value: state),
            URLQueryItem(name: "code_challenge", value: codeChallenge),
            URLQueryItem(name: "code_challenge_method", value: "S256"),
        ]
        for (name, value) in config.extraAuthParams.sorted(by: { $0.key < $1.key }) {
            items.append(URLQueryItem(name: name, value: value))
        }
        var components = URLComponents(url: config.authorizationEndpoint, resolvingAgainstBaseURL: false)!
        components.queryItems = items
        return components.url!
    }

    /// Extracts the authorization code from the redirect URL, validating `state`.
    public func authorizationCode(from redirect: URL, expectedState: String) throws -> String {
        let items = URLComponents(url: redirect, resolvingAgainstBaseURL: false)?.queryItems ?? []
        func value(_ name: String) -> String? { items.first { $0.name == name }?.value }
        if let error = value("error") {
            if error == "access_denied" { throw AuthorizationError.cancelled }
            throw SourceError.invalidResponse("oauth error: \(error)")
        }
        guard value("state") == expectedState else { throw AuthorizationError.stateMismatch }
        guard let code = value("code"), !code.isEmpty else { throw AuthorizationError.missingCode }
        return code
    }

    public func exchange(code: String, verifier: String, redirectURI: URL) async throws -> OAuthTokens {
        try await token(
            [
                "grant_type": "authorization_code", "code": code, "code_verifier": verifier,
                "redirect_uri": redirectURI.absoluteString,
            ], isRefresh: false)
    }

    /// Throws `SourceError.authExpired` when the refresh token is no longer valid (`invalid_grant`).
    public func refresh(refreshToken: String) async throws -> OAuthTokens {
        try await token(["grant_type": "refresh_token", "refresh_token": refreshToken], isRefresh: true)
    }

    private struct TokenResponse: Decodable {
        var accessToken: String?
        var expiresIn: Double?
        var refreshToken: String?
        var error: String?
    }

    private func token(_ params: [String: String], isRefresh: Bool) async throws -> OAuthTokens {
        var fields = params
        fields["client_id"] = config.clientID
        if let secret = config.clientSecret { fields["client_secret"] = secret }
        let body = fields.sorted { $0.key < $1.key }
            .map { "\(Self.encode($0.key))=\(Self.encode($0.value))" }.joined(separator: "&")
        let response = try await transport.send(HTTPRequest(
            url: config.tokenEndpoint, method: "POST",
            headers: ["Content-Type": "application/x-www-form-urlencoded", "Accept": "application/json"],
            body: Data(body.utf8)))
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let parsed = try? decoder.decode(TokenResponse.self, from: response.body)
        guard (200..<300).contains(response.status), let accessToken = parsed?.accessToken else {
            let error = parsed?.error ?? "HTTP \(response.status)"
            if isRefresh, error == "invalid_grant" { throw SourceError.authExpired }
            throw SourceError.invalidResponse("oauth error: \(error)")
        }
        return OAuthTokens(
            accessToken: accessToken,
            expiresAt: now().addingTimeInterval(parsed?.expiresIn ?? 3600),
            refreshToken: parsed?.refreshToken)
    }

    private static let unreserved = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-._~"))
    private static func encode(_ s: String) -> String { s.addingPercentEncoding(withAllowedCharacters: unreserved) ?? s }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `swift test --package-path Packages/CalendarConnectors --filter OAuthTests`
Expected: PASS (6 tests).

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "feat(connectors): OAuth2 PKCE client

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 5: AccessTokenProvider

**Files:**
- Create: `Sources/CalendarCore/AccessTokenProvider.swift`, `Tests/CalendarCoreTests/AccessTokenProviderTests.swift`

**Interfaces:**
- Consumes: `OAuthTokens`, `CredentialStore`, `InMemoryCredentialStore`, `SourceError`, `TestNow`.
- Produces: `actor AccessTokenProvider` with `init(connectionID:credentials:refresh:now:initial:)`, `accessToken() async throws -> String`, `invalidate()`; `static let refreshTokenKey = "refresh_token"`; `typealias RefreshFunction = @Sendable (String) async throws -> OAuthTokens`.

- [ ] **Step 1: Write the failing tests**

`Tests/CalendarCoreTests/AccessTokenProviderTests.swift`:
```swift
import CalendarTestSupport
import Foundation
import Testing
@testable import CalendarCore

private actor Refresher {
    private(set) var calls = 0
    private(set) var seen: [String] = []
    let now: TestNow
    let rotate: Bool
    init(now: TestNow, rotate: Bool = false) { self.now = now; self.rotate = rotate }

    func refresh(_ token: String) async throws -> OAuthTokens {
        calls += 1
        seen.append(token)
        try await Task.sleep(for: .milliseconds(30))
        return OAuthTokens(
            accessToken: "at\(calls)", expiresAt: now.date.addingTimeInterval(3600),
            refreshToken: rotate ? "rt\(calls)" : nil)
    }
}

private func makeProvider(
    now: TestNow, refresher: Refresher, store: InMemoryCredentialStore, initial: OAuthTokens? = nil
) -> AccessTokenProvider {
    AccessTokenProvider(
        connectionID: "c1", credentials: store, refresh: { try await refresher.refresh($0) },
        now: now.provider, initial: initial)
}

@Test func cachesUntilNearExpiry() async throws {
    let now = TestNow()
    let store = InMemoryCredentialStore()
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt0"], for: "c1")
    let refresher = Refresher(now: now)
    let provider = makeProvider(now: now, refresher: refresher, store: store)
    #expect(try await provider.accessToken() == "at1")
    #expect(try await provider.accessToken() == "at1")
    #expect(await refresher.calls == 1)
    now.advance(3600 - 30) // inside the 60 s safety margin
    #expect(try await provider.accessToken() == "at2")
    #expect(await refresher.calls == 2)
}

@Test func concurrentCallersShareOneRefresh() async throws {
    let now = TestNow()
    let store = InMemoryCredentialStore()
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt0"], for: "c1")
    let refresher = Refresher(now: now)
    let provider = makeProvider(now: now, refresher: refresher, store: store)
    let tokens = try await withThrowingTaskGroup(of: String.self) { group in
        for _ in 0..<10 { group.addTask { try await provider.accessToken() } }
        return try await group.reduce(into: [String]()) { $0.append($1) }
    }
    #expect(Set(tokens) == ["at1"])
    #expect(await refresher.calls == 1)
}

@Test func rotatedRefreshTokenIsPersisted() async throws {
    let now = TestNow()
    let store = InMemoryCredentialStore()
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt0", "other": "keep"], for: "c1")
    let refresher = Refresher(now: now, rotate: true)
    let provider = makeProvider(now: now, refresher: refresher, store: store)
    _ = try await provider.accessToken()
    let secrets = try #require(try await store.secrets(for: "c1"))
    #expect(secrets[AccessTokenProvider.refreshTokenKey] == "rt1")
    #expect(secrets["other"] == "keep")
    #expect(await refresher.seen == ["rt0"])
}

@Test func missingRefreshTokenIsAuthExpired() async {
    let now = TestNow()
    let provider = makeProvider(now: now, refresher: Refresher(now: now), store: InMemoryCredentialStore())
    await #expect(throws: SourceError.authExpired) { try await provider.accessToken() }
}

@Test func invalidateForcesARefresh() async throws {
    let now = TestNow()
    let store = InMemoryCredentialStore()
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt0"], for: "c1")
    let refresher = Refresher(now: now)
    let provider = makeProvider(now: now, refresher: refresher, store: store)
    _ = try await provider.accessToken()
    await provider.invalidate()
    #expect(try await provider.accessToken() == "at2")
}

@Test func initialTokensAreUsedWithoutTouchingTheStore() async throws {
    let now = TestNow()
    let refresher = Refresher(now: now)
    let initial = OAuthTokens(accessToken: "seed", expiresAt: now.date.addingTimeInterval(3600))
    let provider = makeProvider(now: now, refresher: refresher, store: InMemoryCredentialStore(), initial: initial)
    #expect(try await provider.accessToken() == "seed")
    #expect(await refresher.calls == 0)
}
```

- [ ] **Step 2: Run to verify failure**

Run: `swift test --package-path Packages/CalendarConnectors --filter AccessTokenProviderTests`
Expected: FAIL to compile ("cannot find 'AccessTokenProvider' in scope").

- [ ] **Step 3: Implement**

`Sources/CalendarCore/AccessTokenProvider.swift`:
```swift
import Foundation

/// Hands out a valid access token, refreshing with the stored refresh token when it is about to expire.
/// Concurrent callers share one in-flight refresh. A rotated refresh token is written back to the store.
public actor AccessTokenProvider {
    public typealias RefreshFunction = @Sendable (String) async throws -> OAuthTokens
    public static let refreshTokenKey = "refresh_token"
    private static let safetyMargin: TimeInterval = 60

    private let connectionID: ConnectionID
    private let credentials: any CredentialStore
    private let refresh: RefreshFunction
    private let now: @Sendable () -> Date
    private var cached: OAuthTokens?
    private var inflight: Task<OAuthTokens, Error>?

    public init(
        connectionID: ConnectionID, credentials: any CredentialStore, refresh: @escaping RefreshFunction,
        now: @escaping @Sendable () -> Date, initial: OAuthTokens? = nil
    ) {
        self.connectionID = connectionID
        self.credentials = credentials
        self.refresh = refresh
        self.now = now
        self.cached = initial
    }

    public func accessToken() async throws -> String {
        if let cached, cached.expiresAt.timeIntervalSince(now()) > Self.safetyMargin { return cached.accessToken }
        return try await refreshShared().accessToken
    }

    /// Drops the cached token (call after a 401) so the next `accessToken()` refreshes.
    public func invalidate() { cached = nil }

    private func refreshShared() async throws -> OAuthTokens {
        if let inflight { return try await inflight.value }
        let task = Task { try await self.performRefresh() }
        inflight = task
        defer { inflight = nil }
        return try await task.value
    }

    private func performRefresh() async throws -> OAuthTokens {
        var secrets = try await credentials.secrets(for: connectionID) ?? [:]
        guard let refreshToken = secrets[Self.refreshTokenKey] else { throw SourceError.authExpired }
        let tokens = try await refresh(refreshToken)
        cached = tokens
        if let rotated = tokens.refreshToken, rotated != refreshToken {
            secrets[Self.refreshTokenKey] = rotated
            try await credentials.setSecrets(secrets, for: connectionID)
        }
        return tokens
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `swift test --package-path Packages/CalendarConnectors --filter AccessTokenProviderTests`
Expected: PASS (6 tests). If the concurrency test is flaky, the failure means single-flight is broken; do not loosen the assertion.

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "feat(connectors): access token provider with single-flight refresh

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 6: ChangeMonitor

**Files:**
- Create: `Sources/CalendarCore/ChangeMonitor.swift`, `Tests/CalendarCoreTests/ChangeMonitorTests.swift`

**Interfaces:**
- Consumes: `PollingCalendarSource`, `CalendarChange`, `SourceError`.
- Produces: `typealias Sleeper = @Sendable (Duration) async throws -> Void`, `let defaultSleeper: Sleeper`, `struct ChangeMonitor(interval:maxBackoff:sleep:)` with `changes(polling:) -> AsyncStream<CalendarChange>`.

- [ ] **Step 1: Write the failing tests**

`Tests/CalendarCoreTests/ChangeMonitorTests.swift`:
```swift
import Foundation
import Testing
@testable import CalendarCore

private actor Script {
    private var results: [Result<CalendarChange?, Error>]
    private(set) var calls = 0
    init(_ results: [Result<CalendarChange?, Error>]) { self.results = results }
    func next() throws -> CalendarChange? {
        calls += 1
        guard !results.isEmpty else { throw CancellationError() }
        return try results.removeFirst().get()
    }
}

private struct ScriptedSource: PollingCalendarSource {
    let script: Script
    var id: String { "scripted" }
    var displayName: String { "Scripted" }
    var capabilities: SourceCapabilities { SourceCapabilities(syncKind: .token) }
    func calendars() async throws -> [CalendarDescriptor] { [] }
    func events(in interval: DateInterval) async throws -> [CalendarEvent] { [] }
    func changes() -> AsyncStream<CalendarChange> { AsyncStream { $0.finish() } }
    func checkForChanges() async throws -> CalendarChange? { try await script.next() }
}

private actor SleepLog {
    private(set) var durations: [Duration] = []
    func record(_ d: Duration) { durations.append(d) }
}

private func run(_ results: [Result<CalendarChange?, Error>], monitor: (@escaping @Sendable (Duration) async -> Void) -> ChangeMonitor)
    async -> (changes: [CalendarChange], sleeps: [Duration])
{
    let log = SleepLog()
    let m = monitor { await log.record($0) }
    var changes: [CalendarChange] = []
    for await change in m.changes(polling: ScriptedSource(script: Script(results))) { changes.append(change) }
    return (changes, await log.durations)
}

private func standard(_ record: @escaping @Sendable (Duration) async -> Void) -> ChangeMonitor {
    ChangeMonitor(interval: .seconds(60), maxBackoff: .seconds(900), sleep: { await record($0) })
}

@Test func yieldsChangesAndSleepsTheIntervalBetweenChecks() async {
    let result = await run([.success(nil), .success(.eventsChanged(calendarIDs: ["a"])), .success(.calendarsChanged)], monitor: standard)
    #expect(result.changes == [.eventsChanged(calendarIDs: ["a"]), .calendarsChanged])
    #expect(result.sleeps == [.seconds(60), .seconds(60), .seconds(60)])
}

@Test func backsOffExponentiallyThenResetsOnSuccess() async {
    let boom = SourceError.network("down")
    let result = await run([.failure(boom), .failure(boom), .success(nil), .success(nil)], monitor: standard)
    #expect(result.sleeps == [.seconds(120), .seconds(240), .seconds(60), .seconds(60)])
}

@Test func backoffIsCappedAndNeverOverflows() async {
    let boom = SourceError.server(status: 503)
    let result = await run(Array(repeating: .failure(boom), count: 5000), monitor: standard)
    #expect(result.sleeps.count == 5000)
    #expect(result.sleeps.max() == .seconds(900))
    #expect(result.sleeps.last == .seconds(900))
}

@Test func authExpiredYieldsSourceFailedAndFinishesWithoutSleeping() async {
    let result = await run([.failure(SourceError.authExpired)], monitor: standard)
    #expect(result.changes == [.sourceFailed(.authExpired)])
    #expect(result.sleeps.isEmpty)
}

@Test func cancellingTheConsumerStopsPolling() async {
    let script = Script(Array(repeating: .success(nil), count: 1_000_000))
    let monitor = ChangeMonitor(interval: .milliseconds(1), maxBackoff: .seconds(1))
    let consumer = Task {
        for await _ in monitor.changes(polling: ScriptedSource(script: script)) {}
    }
    try? await Task.sleep(for: .milliseconds(30))
    consumer.cancel()
    await consumer.value
    let callsAtStop = await script.calls
    try? await Task.sleep(for: .milliseconds(30))
    #expect(await script.calls <= callsAtStop + 1)
}
```

- [ ] **Step 2: Run to verify failure**

Run: `swift test --package-path Packages/CalendarConnectors --filter ChangeMonitorTests`
Expected: FAIL to compile ("cannot find 'ChangeMonitor' in scope").

- [ ] **Step 3: Implement**

`Sources/CalendarCore/ChangeMonitor.swift`:
```swift
import Foundation

public typealias Sleeper = @Sendable (Duration) async throws -> Void
public let defaultSleeper: Sleeper = { try await Task.sleep(for: $0) }

/// Turns a source's cheap `checkForChanges()` into a `changes()` stream by polling.
/// Sleeping goes through `sleep`, so tests drive it without real time. Use one subscriber per source
/// (each `changes(polling:)` call starts its own poller).
public struct ChangeMonitor: Sendable {
    public var interval: Duration
    public var maxBackoff: Duration
    private let sleep: Sleeper

    public init(interval: Duration = .seconds(60), maxBackoff: Duration = .seconds(900), sleep: @escaping Sleeper = defaultSleeper) {
        self.interval = interval
        self.maxBackoff = maxBackoff
        self.sleep = sleep
    }

    public func changes(polling source: some PollingCalendarSource) -> AsyncStream<CalendarChange> {
        let interval = interval, maxBackoff = maxBackoff, sleep = sleep
        return AsyncStream { continuation in
            let task = Task {
                var failures = 0
                while !Task.isCancelled {
                    do {
                        if let change = try await source.checkForChanges() { continuation.yield(change) }
                        failures = 0
                        try await sleep(interval)
                    } catch is CancellationError {
                        break
                    } catch SourceError.authExpired {
                        continuation.yield(.sourceFailed(.authExpired))
                        break
                    } catch {
                        failures += 1
                        // Clamp the exponent before multiplying so a long outage cannot overflow.
                        let delay = min(maxBackoff, interval * (1 << min(failures, 10)))
                        do { try await sleep(delay) } catch { break }
                    }
                }
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `swift test --package-path Packages/CalendarConnectors --filter ChangeMonitorTests`
Expected: PASS (5 tests). If `standard` closure capture warnings appear under Swift 6, keep the `@Sendable` annotations as written.

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "feat(connectors): polling ChangeMonitor with backoff and terminal auth failure

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 7: Google wire types and event/calendar mapping

**Files:**
- Create: `Sources/GoogleCalendar/GoogleDTOs.swift`, `Sources/GoogleCalendar/GoogleEventMapper.swift`, `Tests/GoogleCalendarTests/MapperTests.swift`
- Delete: `Sources/GoogleCalendar/Placeholder.swift`, `Tests/GoogleCalendarTests/Placeholder.swift`

**Interfaces:**
- Consumes: Model types (Task 1).
- Produces (internal to `GoogleCalendar`, used by Tasks 8-10): `GoogleEventDTO`, `GoogleTimeDTO`, `GoogleCalendarListEntryDTO`, `GoogleEventMapper.map(_ dto: GoogleEventDTO, calendar: CalendarDescriptor) -> CalendarEvent?`, `GoogleEventMapper.descriptor(from: GoogleCalendarListEntryDTO, accountName: String?) -> CalendarDescriptor?`.

- [ ] **Step 1: Write the failing tests**

`Tests/GoogleCalendarTests/MapperTests.swift`:
```swift
import CalendarCore
import Foundation
import Testing
@testable import GoogleCalendar

private let tokyo = TimeZone(identifier: "Asia/Tokyo")!
private let calendar = CalendarDescriptor(id: "cal1", title: "Work", accessRole: .owner, isPrimary: true, timeZone: tokyo)

private func event(_ json: String) throws -> GoogleEventDTO {
    try JSONDecoder().decode(GoogleEventDTO.self, from: Data(json.utf8))
}
private func map(_ json: String) throws -> CalendarEvent? { GoogleEventMapper.map(try event(json), calendar: calendar) }
private func instant(_ s: String) -> Date { ISO8601DateFormatter().date(from: s)! }

@Test func mapsATimedEvent() throws {
    let e = try #require(try map("""
    {"id":"e1","iCalUID":"uid@google.com","status":"confirmed","summary":"Standup","description":"notes","location":"Room 1",
     "htmlLink":"https://www.google.com/calendar/event?eid=x","etag":"\\"123\\"",
     "start":{"dateTime":"2026-09-21T10:00:00-07:00","timeZone":"America/Los_Angeles"},
     "end":{"dateTime":"2026-09-21T10:30:00-07:00","timeZone":"America/Los_Angeles"}}
    """))
    #expect(e.eventID == "e1" && e.calendarID == "cal1" && e.id == "cal1/e1")
    #expect(e.uid == "uid@google.com")
    #expect(e.title == "Standup" && e.notes == "notes" && e.location == "Room 1")
    #expect(e.start == instant("2026-09-21T10:00:00-07:00") && e.end == instant("2026-09-21T10:30:00-07:00"))
    #expect(e.timeZone?.identifier == "America/Los_Angeles")
    #expect(!e.isAllDay && e.status == .confirmed && e.availability == .busy && e.kind == .standard)
    #expect(e.url?.absoluteString == "https://www.google.com/calendar/event?eid=x")
    #expect(e.version == "\"123\"")
    #expect(e.myResponse == nil && e.attendees.isEmpty)
}

@Test func allDayEventsUseMidnightInTheCalendarZoneWithExclusiveEnd() throws {
    let e = try #require(try map(#"{"id":"a","summary":"Off","start":{"date":"2026-09-20"},"end":{"date":"2026-09-22"}}"#))
    var cal = Calendar(identifier: .gregorian)
    cal.timeZone = tokyo
    #expect(e.isAllDay)
    #expect(e.timeZone?.identifier == "Asia/Tokyo")
    #expect(e.start == cal.date(from: DateComponents(year: 2026, month: 9, day: 20)))
    #expect(e.end == cal.date(from: DateComponents(year: 2026, month: 9, day: 22)))
}

@Test func allDayFallsBackToUTCWhenTheCalendarHasNoZone() throws {
    let bare = CalendarDescriptor(id: "c", title: "C")
    let dto = try event(#"{"id":"a","start":{"date":"2026-09-20"},"end":{"date":"2026-09-21"}}"#)
    let e = try #require(GoogleEventMapper.map(dto, calendar: bare))
    #expect(e.timeZone?.identifier == "UTC" || e.timeZone?.identifier == "GMT")
    #expect(e.start == instant("2026-09-20T00:00:00Z"))
}

@Test func cancelledEventsAreDropped() throws {
    #expect(try map(#"{"id":"x","status":"cancelled","start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"}}"#) == nil)
}

@Test func mapsAttendeesOrganizerAndMyResponse() throws {
    let e = try #require(try map("""
    {"id":"m","summary":"Sync","start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"},
     "organizer":{"email":"Boss@x.com","displayName":"Boss"},
     "attendees":[{"email":"Boss@x.com","responseStatus":"accepted","organizer":true},
                  {"email":"me@x.com","responseStatus":"declined","self":true},
                  {"email":"opt@x.com","optional":true,"responseStatus":"tentative"},
                  {"email":"room@x.com","resource":true}]}
    """))
    #expect(e.attendees.count == 4)
    #expect(e.organizer?.email == "boss@x.com" && e.organizer?.isOrganizer == true)
    #expect(e.myResponse == .declined)
    #expect(e.attendees[1].isSelf)
    #expect(e.attendees[2].role == .optional && e.attendees[2].response == .tentative)
    #expect(e.attendees[3].role == .resource && e.attendees[3].response == .needsAction)
}

@Test func mapsConferenceFromEntryPointsThenHangoutLink() throws {
    let a = try #require(try map("""
    {"id":"c1","start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"},
     "conferenceData":{"entryPoints":[{"entryPointType":"phone","uri":"tel:+1"},{"entryPointType":"video","uri":"https://meet.google.com/abc-defg-hij"}],
       "conferenceSolution":{"key":{"type":"hangoutsMeet"},"name":"Google Meet"}}}
    """))
    #expect(a.conference?.url.absoluteString == "https://meet.google.com/abc-defg-hij")
    #expect(a.conference?.provider == .meet)
    let b = try #require(try map("""
    {"id":"c2","start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"},
     "conferenceData":{"entryPoints":[{"entryPointType":"video","uri":"https://acme.zoom.us/j/123"}],"conferenceSolution":{"key":{"type":"addOn"},"name":"Zoom Meeting"}}}
    """))
    #expect(b.conference?.provider == .zoom)
    let c = try #require(try map(#"{"id":"c3","hangoutLink":"https://meet.google.com/zzz","start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"}}"#))
    #expect(c.conference?.provider == .meet)
}

@Test func mapsKindAvailabilityVisibilityRemindersAndSeries() throws {
    let e = try #require(try map("""
    {"id":"r_20260921","recurringEventId":"r","eventType":"focusTime","transparency":"transparent","visibility":"private",
     "originalStartTime":{"dateTime":"2026-09-21T10:00:00Z"},
     "reminders":{"useDefault":false,"overrides":[{"method":"popup","minutes":10},{"method":"email","minutes":60}]},
     "start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"}}
    """))
    #expect(e.kind == .focusTime && e.availability == .free && e.visibility == .privateEvent)
    #expect(e.seriesID == "r" && e.originalStart == instant("2026-09-21T10:00:00Z"))
    #expect(e.reminders == [Reminder(minutesBefore: 10), Reminder(minutesBefore: 60)])
}

@Test func toleratesFractionalSecondsAndMissingTitle() throws {
    let e = try #require(try map(#"{"id":"f","start":{"dateTime":"2026-09-21T10:00:00.500Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"}}"#))
    #expect(e.title == "")
    #expect(e.start == instant("2026-09-21T10:00:00Z").addingTimeInterval(0.5))
}

@Test func eventsWithoutUsableTimesAreDropped() throws {
    #expect(try map(#"{"id":"bad","start":{"dateTime":"garbage"},"end":{"dateTime":"garbage"}}"#) == nil)
    #expect(try map(#"{"id":"bad2"}"#) == nil)
}

@Test func mapsCalendarListEntries() throws {
    let dto = try JSONDecoder().decode(GoogleCalendarListEntryDTO.self, from: Data("""
    {"id":"me@x.com","summary":"me@x.com","summaryOverride":"Personal","backgroundColor":"#9fe1e7","accessRole":"owner","primary":true,"timeZone":"Asia/Tokyo"}
    """.utf8))
    let d = try #require(GoogleEventMapper.descriptor(from: dto, accountName: "me@x.com"))
    #expect(d.id == "me@x.com" && d.title == "Personal" && d.colorHex == "#9FE1E7")
    #expect(d.accessRole == .owner && d.isPrimary && d.timeZone?.identifier == "Asia/Tokyo" && d.accountName == "me@x.com")
    let hidden = try JSONDecoder().decode(GoogleCalendarListEntryDTO.self, from: Data(#"{"id":"h","hidden":true}"#.utf8))
    #expect(GoogleEventMapper.descriptor(from: hidden, accountName: nil) == nil)
    let deleted = try JSONDecoder().decode(GoogleCalendarListEntryDTO.self, from: Data(#"{"id":"d","deleted":true}"#.utf8))
    #expect(GoogleEventMapper.descriptor(from: deleted, accountName: nil) == nil)
}
```

- [ ] **Step 2: Run to verify failure**

Run: `git rm -q Packages/CalendarConnectors/Sources/GoogleCalendar/Placeholder.swift Packages/CalendarConnectors/Tests/GoogleCalendarTests/Placeholder.swift && swift test --package-path Packages/CalendarConnectors --filter MapperTests`
Expected: FAIL to compile ("cannot find 'GoogleEventDTO' in scope").

- [ ] **Step 3: Implement**

`Sources/GoogleCalendar/GoogleDTOs.swift`:
```swift
import Foundation

struct GoogleTimeDTO: Decodable {
    var date: String?
    var dateTime: String?
    var timeZone: String?
}

struct GoogleAttendeeDTO: Decodable {
    var email: String?
    var displayName: String?
    var responseStatus: String?
    var optional: Bool?
    var resource: Bool?
    var organizer: Bool?
    var isSelf: Bool?

    enum CodingKeys: String, CodingKey {
        case email, displayName, responseStatus, optional, resource, organizer
        case isSelf = "self"
    }
}

struct GoogleOrganizerDTO: Decodable {
    var email: String?
    var displayName: String?
    var isSelf: Bool?

    enum CodingKeys: String, CodingKey {
        case email, displayName
        case isSelf = "self"
    }
}

struct GoogleConferenceDTO: Decodable {
    struct EntryPoint: Decodable {
        var entryPointType: String?
        var uri: String?
    }
    struct Solution: Decodable {
        struct Key: Decodable { var type: String? }
        var key: Key?
        var name: String?
    }
    var entryPoints: [EntryPoint]?
    var conferenceSolution: Solution?
}

struct GoogleRemindersDTO: Decodable {
    struct Override: Decodable { var minutes: Int? }
    var useDefault: Bool?
    var overrides: [Override]?
}

struct GoogleEventDTO: Decodable {
    var id: String
    var iCalUID: String?
    var status: String?
    var summary: String?
    var description: String?
    var location: String?
    var htmlLink: String?
    var etag: String?
    var hangoutLink: String?
    var transparency: String?
    var visibility: String?
    var eventType: String?
    var recurringEventId: String?
    var start: GoogleTimeDTO?
    var end: GoogleTimeDTO?
    var originalStartTime: GoogleTimeDTO?
    var attendees: [GoogleAttendeeDTO]?
    var organizer: GoogleOrganizerDTO?
    var conferenceData: GoogleConferenceDTO?
    var reminders: GoogleRemindersDTO?
}

struct GoogleEventsPageDTO: Decodable {
    var items: [GoogleEventDTO]?
    var nextPageToken: String?
}

struct GoogleCalendarListEntryDTO: Decodable {
    var id: String
    var summary: String?
    var summaryOverride: String?
    var backgroundColor: String?
    var accessRole: String?
    var primary: Bool?
    var timeZone: String?
    var hidden: Bool?
    var deleted: Bool?
}

struct GoogleCalendarListPageDTO: Decodable {
    var items: [GoogleCalendarListEntryDTO]?
    var nextPageToken: String?
}
```

`Sources/GoogleCalendar/GoogleEventMapper.swift`:
```swift
import CalendarCore
import Foundation

enum GoogleEventMapper {
    static func descriptor(from dto: GoogleCalendarListEntryDTO, accountName: String?) -> CalendarDescriptor? {
        if dto.deleted == true || dto.hidden == true { return nil }
        let role: AccessRole
        switch dto.accessRole {
        case "owner": role = .owner
        case "writer": role = .writer
        case "freeBusyReader": role = .freeBusyReader
        default: role = .reader
        }
        return CalendarDescriptor(
            id: dto.id, title: dto.summaryOverride ?? dto.summary ?? dto.id, colorHex: dto.backgroundColor,
            accessRole: role, isPrimary: dto.primary ?? false,
            timeZone: dto.timeZone.flatMap { TimeZone(identifier: $0) }, accountName: accountName)
    }

    /// Returns nil for cancelled events and for events whose times cannot be understood.
    static func map(_ dto: GoogleEventDTO, calendar: CalendarDescriptor) -> CalendarEvent? {
        if dto.status == "cancelled" { return nil }
        let calendarZone = calendar.timeZone ?? TimeZone(identifier: "UTC")!
        guard let start = resolve(dto.start, calendarZone: calendarZone),
              let end = resolve(dto.end, calendarZone: calendarZone)
        else { return nil }

        let attendees = (dto.attendees ?? []).map(attendee)
        let organizer = dto.organizer.map {
            Attendee(name: $0.displayName, email: $0.email, role: .required, response: .accepted,
                     isSelf: $0.isSelf ?? false, isOrganizer: true)
        }
        return CalendarEvent(
            eventID: dto.id, uid: dto.iCalUID, calendarID: calendar.id, title: dto.summary ?? "",
            notes: dto.description, location: dto.location, start: start.date, end: end.date,
            timeZone: start.zone, isAllDay: start.isAllDay, status: dto.status == "tentative" ? .tentative : .confirmed,
            availability: dto.transparency == "transparent" ? .free : .busy,
            visibility: visibility(dto.visibility), kind: kind(dto.eventType),
            seriesID: dto.recurringEventId,
            originalStart: dto.originalStartTime.flatMap { resolve($0, calendarZone: calendarZone)?.date },
            attendees: attendees, organizer: organizer, conference: conference(dto),
            reminders: reminders(dto.reminders), url: dto.htmlLink.flatMap(URL.init(string:)),
            version: dto.etag, myResponse: attendees.first(where: \.isSelf)?.response)
    }

    private struct Resolved {
        var date: Date
        var zone: TimeZone?
        var isAllDay: Bool
    }

    private static func resolve(_ time: GoogleTimeDTO?, calendarZone: TimeZone) -> Resolved? {
        guard let time else { return nil }
        if let day = time.date {
            let parts = day.split(separator: "-").compactMap { Int($0) }
            var calendar = Calendar(identifier: .gregorian)
            calendar.timeZone = calendarZone
            guard parts.count == 3,
                  let date = calendar.date(from: DateComponents(year: parts[0], month: parts[1], day: parts[2]))
            else { return nil }
            return Resolved(date: date, zone: calendarZone, isAllDay: true)
        }
        guard let text = time.dateTime, let date = parseInstant(text) else { return nil }
        return Resolved(date: date, zone: time.timeZone.flatMap { TimeZone(identifier: $0) }, isAllDay: false)
    }

    static func parseInstant(_ text: String) -> Date? {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        if let date = formatter.date(from: text) { return date }
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter.date(from: text)
    }

    private static func attendee(_ dto: GoogleAttendeeDTO) -> Attendee {
        let role: AttendeeRole = dto.resource == true ? .resource : (dto.optional == true ? .optional : .required)
        let response: ResponseStatus
        switch dto.responseStatus {
        case "accepted": response = .accepted
        case "tentative": response = .tentative
        case "declined": response = .declined
        default: response = .needsAction
        }
        return Attendee(
            name: dto.displayName, email: dto.email, role: role, response: response,
            isSelf: dto.isSelf ?? false, isOrganizer: dto.organizer ?? false)
    }

    private static func visibility(_ raw: String?) -> Visibility {
        switch raw {
        case "public": .publicEvent
        case "private": .privateEvent
        case "confidential": .confidential
        default: .default
        }
    }

    private static func kind(_ raw: String?) -> EventKind {
        switch raw {
        case nil, "default": .standard
        case "focusTime": .focusTime
        case "outOfOffice": .outOfOffice
        case "workingLocation": .workingLocation
        case "birthday": .birthday
        default: .other
        }
    }

    private static func conference(_ dto: GoogleEventDTO) -> ConferenceInfo? {
        if let video = dto.conferenceData?.entryPoints?.first(where: { $0.entryPointType == "video" }),
           let text = video.uri, let url = URL(string: text)
        {
            let solution = dto.conferenceData?.conferenceSolution
            let name = (solution?.name ?? "").lowercased()
            let host = (url.host ?? "").lowercased()
            let provider: ConferenceProvider
            if solution?.key?.type == "hangoutsMeet" || host.contains("meet.google.com") {
                provider = .meet
            } else if name.contains("zoom") || host.contains("zoom.") {
                provider = .zoom
            } else if name.contains("teams") || host.contains("teams.microsoft") {
                provider = .teams
            } else {
                provider = .other
            }
            return ConferenceInfo(url: url, provider: provider)
        }
        if let text = dto.hangoutLink, let url = URL(string: text) {
            return ConferenceInfo(url: url, provider: .meet)
        }
        return nil
    }

    private static func reminders(_ dto: GoogleRemindersDTO?) -> [Reminder] {
        guard let dto, dto.useDefault != true else { return [] }
        return (dto.overrides ?? []).compactMap { $0.minutes.map(Reminder.init(minutesBefore:)) }
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `swift test --package-path Packages/CalendarConnectors --filter MapperTests`
Expected: PASS (10 tests).

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "feat(google): wire types and event/calendar mapping

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 8: Google API client, calendars() and events(in:)

**Files:**
- Create: `Sources/GoogleCalendar/GoogleAPIClient.swift`, `Sources/GoogleCalendar/GoogleCalendarSource.swift`, `Tests/GoogleCalendarTests/SourceTests.swift`

**Interfaces:**
- Consumes: `HTTPTransport`, `AccessTokenProvider`, `Sleeper`, `SyncStateStore`, `ChangeMonitor`, mapper + DTOs.
- Produces: `GoogleAPIError` (`.gone`, `.notFound`, `.forbidden`, `.sourceError`; internal, never thrown out of a public method), `GoogleAPIClient(transport:tokens:sleep:)` with `get(path:query:)`, `pages(_:path:query:next:handle:)`, `calendarList()`, `static func calendarPath(_:)`; `public final class GoogleCalendarSource: PollingCalendarSource` with `init(connection:api:syncState:monitor:)` (internal init), `calendars()`, `events(in:)`. `checkForChanges()` and `changes()` are added in Task 9 (a temporary stub is added here so the type compiles).

- [ ] **Step 1: Write the failing tests**

`Tests/GoogleCalendarTests/SourceTests.swift`:
```swift
import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import GoogleCalendar

// `[String: Any]` is not Sendable, so a Swift 6 global must be marked.
nonisolated(unsafe) let calendarListJSON: [String: Any] = [
    "items": [
        ["id": "me@x.com", "summary": "me@x.com", "primary": true, "accessRole": "owner", "timeZone": "UTC", "backgroundColor": "#112233"],
        ["id": "team@group.calendar.google.com", "summary": "Team", "accessRole": "reader", "timeZone": "UTC"],
        ["id": "hidden", "hidden": true],
    ]
]

/// A calendarList response with these calendar ids (all readers, UTC).
func listJSON(_ ids: [String]) -> [String: Any] {
    ["items": ids.map { ["id": $0, "accessRole": "reader", "timeZone": "UTC"] as [String: Any] }]
}

func eventJSON(_ id: String, start: String, summary: String = "E") -> [String: Any] {
    ["id": id, "summary": summary, "start": ["dateTime": start], "end": ["dateTime": start]]
}

struct Harness {
    let transport = FakeTransport()
    let now = TestNow()
    let sync: InMemorySyncStateStore
    let store = InMemoryCredentialStore()
    let source: GoogleCalendarSource
    let sleeps = SleepRecorder()

    /// `calendarList` is what the account's calendar list returns; pass a shared `sync` to simulate a relaunch.
    init(calendarList: [String: Any] = calendarListJSON, sync: InMemorySyncStateStore = InMemorySyncStateStore()) async throws {
        self.sync = sync
        let connection = Connection(kindID: "google", connectionID: "c1", displayName: "me@x.com", config: ["email": "me@x.com"])
        try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt"], for: "c1")
        let now = self.now
        let transport = self.transport
        let sleeps = self.sleeps
        let provider = AccessTokenProvider(
            connectionID: "c1", credentials: store,
            refresh: { _ in OAuthTokens(accessToken: "at", expiresAt: now.date.addingTimeInterval(3600)) }, now: now.provider)
        let api = GoogleAPIClient(transport: transport, tokens: provider, sleep: { await sleeps.record($0) })
        source = GoogleCalendarSource(connection: connection, api: api, syncState: sync, monitor: ChangeMonitor(sleep: { _ in }))
        await transport.route("users/me/calendarList", [.json(calendarList)])
    }
}

actor SleepRecorder {
    private(set) var durations: [Duration] = []
    func record(_ d: Duration) { durations.append(d) }
}

@Test func calendarsMapsAndSkipsHidden() async throws {
    let h = try await Harness()
    let cals = try await h.source.calendars()
    #expect(cals.map(\.id) == ["me@x.com", "team@group.calendar.google.com"])
    #expect(cals[0].accountName == "me@x.com" && cals[0].colorHex == "#112233" && cals[0].isPrimary)
    let request = try #require(await h.transport.requests.first)
    #expect(request.headers["Authorization"] == "Bearer at")
    #expect(request.url.absoluteString.contains("showHidden=false"))
}

@Test func eventsFetchesEveryCalendarWithWindowAndSortsByStart() async throws {
    let h = try await Harness()
    await h.transport.route("calendars/me%40x.com/events", [.json(["items": [eventJSON("b", start: "2026-09-21T12:00:00Z")]])])
    await h.transport.route("calendars/team%40group.calendar.google.com/events", [.json(["items": [eventJSON("a", start: "2026-09-21T09:00:00Z")]])])
    let interval = DateInterval(start: Date(timeIntervalSince1970: 1_790_000_000), end: Date(timeIntervalSince1970: 1_790_086_400))
    let events = try await h.source.events(in: interval)
    #expect(events.map(\.eventID) == ["a", "b"])
    let request = try #require(await h.transport.requests(matching: "calendars/me%40x.com/events").first)
    let url = request.url.absoluteString
    #expect(url.contains("singleEvents=true") && url.contains("timeMin=") && url.contains("timeMax="))
    #expect(url.contains("showDeleted=false"))
}

@Test func eventsFollowsPaging() async throws {
    let h = try await Harness()
    await h.transport.route("calendars/team%40group.calendar.google.com/events", [.json(["items": []])])
    await h.transport.route("calendars/me%40x.com/events", [.json(["items": [eventJSON("one", start: "2026-09-21T10:00:00Z")], "nextPageToken": "page2"])])
    await h.transport.route("pageToken=page2", [.json(["items": [eventJSON("two", start: "2026-09-21T11:00:00Z")]])])
    let events = try await h.source.events(in: DateInterval(start: .now, duration: 3600))
    #expect(events.map(\.eventID) == ["one", "two"])
}

@Test func aMissingOrForbiddenCalendarIsSkippedNotFatal() async throws {
    let h = try await Harness()
    await h.transport.route("calendars/me%40x.com/events", [.json(["items": [eventJSON("ok", start: "2026-09-21T10:00:00Z")]])])
    await h.transport.route("calendars/team%40group.calendar.google.com/events", [.json(["error": ["errors": [["reason": "forbidden"]]]], status: 403)])
    let events = try await h.source.events(in: DateInterval(start: .now, duration: 3600))
    #expect(events.map(\.eventID) == ["ok"])
}

@Test func a401RefreshesTheTokenOnceThenSucceeds() async throws {
    let h = try await Harness()
    await h.transport.route("calendars/me%40x.com/events", [.json([:], status: 401), .json(["items": []])])
    await h.transport.route("calendars/team%40group.calendar.google.com/events", [.json(["items": []])])
    _ = try await h.source.events(in: DateInterval(start: .now, duration: 3600))
    #expect(await h.transport.requests(matching: "calendars/me%40x.com/events").count == 2)
}

@Test func aRepeated401IsAuthExpired() async throws {
    let h = try await Harness()
    await h.transport.route("calendars/", [.json([:], status: 401)])
    await #expect(throws: SourceError.authExpired) {
        try await h.source.events(in: DateInterval(start: .now, duration: 3600))
    }
}

@Test func rateLimitsAreRetriedWithBackoffThenSurfaced() async throws {
    let h = try await Harness(calendarList: listJSON(["me@x.com"]))
    await h.transport.route("calendars/", [.json(["error": ["errors": [["reason": "rateLimitExceeded"]]]], status: 403, headers: ["Retry-After": "2"])])
    await #expect(throws: SourceError.rateLimited(retryAfter: 2)) {
        try await h.source.events(in: DateInterval(start: .now, duration: 3600))
    }
    #expect(await h.sleeps.durations == [.seconds(2), .seconds(2), .seconds(2)])
}

@Test func serverErrorsAndMalformedBodiesSurfaceAsSourceErrors() async throws {
    let h = try await Harness()
    await h.transport.route("calendars/", [.json([:], status: 503)])
    await #expect(throws: SourceError.server(status: 503)) {
        try await h.source.events(in: DateInterval(start: .now, duration: 3600))
    }
    let h2 = try await Harness()
    await h2.transport.route("calendars/", [.text("not json")])
    await #expect(throws: SourceError.self) {
        try await h2.source.events(in: DateInterval(start: .now, duration: 3600))
    }
}

@Test func providerSpecificErrorsNeverEscapeAsGoogleTypes() async throws {
    let h = try await Harness()
    await h.transport.route("users/me/calendarList", [.json([:], status: 404)])
    await #expect(throws: SourceError.self) { try await h.source.calendars() }
    await h.transport.route("users/me/calendarList", [.json([:], status: 410)])
    await #expect(throws: SourceError.self) { try await h.source.checkForChanges() }
}

@Test func capabilitiesDescribeAReadOnlyTokenSyncedSource() async throws {
    let h = try await Harness()
    let c = h.source.capabilities
    #expect(!c.canWrite && c.providesConference && c.syncKind == .token && !c.supportsPush)
    #expect(h.source.id == "google-c1" && h.source.displayName == "me@x.com")
}
```

- [ ] **Step 2: Run to verify failure**

Run: `swift test --package-path Packages/CalendarConnectors --filter SourceTests`
Expected: FAIL to compile ("cannot find 'GoogleAPIClient' in scope").

- [ ] **Step 3: Implement the API client**

`Sources/GoogleCalendar/GoogleAPIClient.swift`:
```swift
import CalendarCore
import Foundation

/// Provider-specific outcomes that callers handle; never leaves the `GoogleCalendar` module.
enum GoogleAPIError: Error, Equatable {
    case gone       // 410: the sync token is no longer valid
    case notFound   // 404
    case forbidden  // 403 without a rate-limit reason

    /// What a public `CalendarSource` method throws if it cannot handle the case itself.
    var sourceError: SourceError { .invalidResponse("google: \(self)") }
}

struct GoogleAPIClient: Sendable {
    static let base = "https://www.googleapis.com/calendar/v3"
    private static let maxRateLimitRetries = 3

    let transport: any HTTPTransport
    let tokens: AccessTokenProvider
    let sleep: Sleeper

    /// `path` must already be percent-encoded, e.g. `/calendars/me%40x.com/events`.
    static func calendarPath(_ calendarID: String, _ tail: String) -> String {
        let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-._~"))
        return "/calendars/\(calendarID.addingPercentEncoding(withAllowedCharacters: allowed) ?? calendarID)\(tail)"
    }

    func get(path: String, query: [URLQueryItem]) async throws -> Data {
        var rateLimitRetries = 0
        var refreshedAfter401 = false
        while true {
            try Task.checkCancellation()
            let token = try await tokens.accessToken()
            var components = URLComponents(string: Self.base)!
            components.percentEncodedPath += path
            components.queryItems = query
            let response = try await transport.send(HTTPRequest(
                url: components.url!, headers: ["Authorization": "Bearer \(token)", "Accept": "application/json"]))
            switch response.status {
            case 200..<300:
                return response.body
            case 401:
                if refreshedAfter401 { throw SourceError.authExpired }
                refreshedAfter401 = true
                await tokens.invalidate()
            case 410:
                throw GoogleAPIError.gone
            case 404:
                throw GoogleAPIError.notFound
            case 403, 429:
                guard Self.isRateLimit(response) else {
                    if response.status == 403 { throw GoogleAPIError.forbidden }
                    throw SourceError.invalidResponse("HTTP \(response.status)")
                }
                let retryAfter = response.header("retry-after").flatMap(TimeInterval.init)
                rateLimitRetries += 1
                if rateLimitRetries > Self.maxRateLimitRetries { throw SourceError.rateLimited(retryAfter: retryAfter) }
                // Honor Retry-After exactly; otherwise exponential backoff with jitter so several clients de-synchronize.
                let fallback = Double(1 << (rateLimitRetries - 1)) * Double.random(in: 0.5...1.0)
                try await sleep(.seconds(retryAfter ?? fallback))
            case 500...:
                throw SourceError.server(status: response.status)
            default:
                throw SourceError.invalidResponse("HTTP \(response.status)")
            }
        }
    }

    private static func isRateLimit(_ response: HTTPResponse) -> Bool {
        if response.status == 429 { return true }
        let body = String(decoding: response.body, as: UTF8.self)
        return body.contains("rateLimitExceeded") || body.contains("userRateLimitExceeded")
    }

    func decode<T: Decodable>(_ type: T.Type, from data: Data) throws -> T {
        do { return try JSONDecoder().decode(type, from: data) }
        catch { throw SourceError.invalidResponse("could not decode \(T.self)") }
    }

    /// Walks every page, calling `handle` for each, until `next` returns nil. No page cap; honors cancellation.
    func pages<Page: Decodable>(
        _ type: Page.Type, path: String, query: [URLQueryItem],
        next: (Page) -> String?, handle: (Page) throws -> Void
    ) async throws {
        var pageToken: String?
        repeat {
            var pageQuery = query
            if let pageToken { pageQuery.append(URLQueryItem(name: "pageToken", value: pageToken)) }
            let page = try decode(Page.self, from: try await get(path: path, query: pageQuery))
            try handle(page)
            pageToken = next(page)
        } while pageToken != nil
    }

    func calendarList() async throws -> [GoogleCalendarListEntryDTO] {
        var entries: [GoogleCalendarListEntryDTO] = []
        try await pages(
            GoogleCalendarListPageDTO.self, path: "/users/me/calendarList",
            query: [
                URLQueryItem(name: "showHidden", value: "false"),
                URLQueryItem(name: "minAccessRole", value: "freeBusyReader"),
                URLQueryItem(name: "maxResults", value: "250"),
                URLQueryItem(name: "fields", value: "nextPageToken,items(id,summary,summaryOverride,backgroundColor,accessRole,primary,timeZone,hidden,deleted)"),
            ],
            next: { $0.nextPageToken }, handle: { entries += $0.items ?? [] })
        return entries
    }
}
```

`Sources/GoogleCalendar/GoogleCalendarSource.swift`:
```swift
import CalendarCore
import Foundation

public final class GoogleCalendarSource: PollingCalendarSource {
    private static let eventFields =
        "nextPageToken,items(id,iCalUID,status,summary,description,location,htmlLink,etag,hangoutLink,transparency,visibility,eventType,recurringEventId,start,end,originalStartTime,attendees(email,displayName,responseStatus,optional,resource,organizer,self),organizer(email,displayName,self),conferenceData(entryPoints(entryPointType,uri),conferenceSolution(key(type),name)),reminders(useDefault,overrides(minutes)))"

    let connection: Connection
    let api: GoogleAPIClient
    let syncState: any SyncStateStore
    let monitor: ChangeMonitor

    init(connection: Connection, api: GoogleAPIClient, syncState: any SyncStateStore, monitor: ChangeMonitor) {
        self.connection = connection
        self.api = api
        self.syncState = syncState
        self.monitor = monitor
    }

    public var id: String { connection.sourceID }
    public var displayName: String { connection.displayName }
    public var capabilities: SourceCapabilities {
        SourceCapabilities(providesConference: true, syncKind: .token)
    }

    public func calendars() async throws -> [CalendarDescriptor] {
        let account = connection.config["email"]
        do {
            return try await api.calendarList().compactMap { GoogleEventMapper.descriptor(from: $0, accountName: account) }
        } catch let error as GoogleAPIError {
            throw error.sourceError
        }
    }

    public func events(in interval: DateInterval) async throws -> [CalendarEvent] {
        let calendars = try await calendars()
        return try await withThrowingTaskGroup(of: [CalendarEvent].self) { group in
            for calendar in calendars {
                group.addTask { try await self.events(for: calendar, in: interval) }
            }
            var all: [CalendarEvent] = []
            for try await part in group { all += part }
            return all.sorted { ($0.start, $0.id) < ($1.start, $1.id) }
        }
    }

    private func events(for calendar: CalendarDescriptor, in interval: DateInterval) async throws -> [CalendarEvent] {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        let query = [
            URLQueryItem(name: "singleEvents", value: "true"),
            URLQueryItem(name: "timeMin", value: formatter.string(from: interval.start)),
            URLQueryItem(name: "timeMax", value: formatter.string(from: interval.end)),
            URLQueryItem(name: "maxResults", value: "250"),
            URLQueryItem(name: "showDeleted", value: "false"),
            URLQueryItem(name: "fields", value: Self.eventFields),
        ]
        var events: [CalendarEvent] = []
        do {
            try await api.pages(
                GoogleEventsPageDTO.self, path: GoogleAPIClient.calendarPath(calendar.id, "/events"), query: query,
                next: { $0.nextPageToken },
                handle: { events += ($0.items ?? []).compactMap { GoogleEventMapper.map($0, calendar: calendar) } })
        } catch let error as GoogleAPIError {
            // A calendar that was removed or lost access is skipped; anything else is not ours to interpret.
            if error == .notFound || error == .forbidden { return [] }
            throw error.sourceError
        }
        return events
    }

    public func changes() -> AsyncStream<CalendarChange> { monitor.changes(polling: self) }

    public func checkForChanges() async throws -> CalendarChange? { nil } // replaced in Task 9
}
```

- [ ] **Step 4: Run to verify pass**

Run: `swift test --package-path Packages/CalendarConnectors --filter SourceTests`
Expected: PASS (10 tests). Note: `Harness` is a struct holding a `let source` built in `init`; if Swift 6 flags `sleeps`/`now` use before `self` init, copy them into locals first as written.

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "feat(google): API client, calendars() and events(in:)

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 9: Google change detection (`checkForChanges`)

**Files:**
- Modify: `Sources/GoogleCalendar/GoogleCalendarSource.swift` (replace the stub)
- Create: `Tests/GoogleCalendarTests/ChangeDetectionTests.swift`

**Interfaces:**
- Consumes: `Harness(calendarList:sync:)`, `calendarListJSON`, `eventJSON` from `SourceTests.swift` (Task 8, same test module), `SyncStateStore`.
- Produces: `checkForChanges() -> CalendarChange?` per the spec; sync scopes are calendar ids plus the reserved scope `"_calendars"`.

- [ ] **Step 1: Write the failing tests**

`Tests/GoogleCalendarTests/ChangeDetectionTests.swift`:
```swift
import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import GoogleCalendar

private let me = "calendars/me%40x.com/events"
private let team = "calendars/team%40group.calendar.google.com/events"

private func bootstrapRoutes(_ h: Harness, meToken: String = "m1", teamToken: String = "t1") async {
    await h.transport.route(team, [.json(["items": [], "nextSyncToken": teamToken])])
    await h.transport.route(me, [.json(["items": [], "nextSyncToken": meToken])])
}

@Test func firstCallBootstrapsTokensAndReportsNothing() async throws {
    let h = try await Harness()
    await bootstrapRoutes(h)
    #expect(try await h.source.checkForChanges() == nil)
    #expect(await h.sync.token(for: "c1", scope: "me@x.com") == "m1")
    #expect(await h.sync.token(for: "c1", scope: "team@group.calendar.google.com") == "t1")
    let request = try #require(await h.transport.requests(matching: me).first)
    let url = request.url.absoluteString
    #expect(url.contains("showDeleted=true") && url.contains("maxResults=2500"))
    #expect(!url.contains("timeMin") && !url.contains("syncToken"))
}

@Test func bootstrapWalksEveryPageAndTakesTheLastToken() async throws {
    let h = try await Harness()
    await h.transport.route(team, [.json(["items": [], "nextSyncToken": "t1"])])
    await h.transport.route(me, [.json(["nextPageToken": "page2"])])
    await h.transport.route("pageToken=page2", [.json(["nextSyncToken": "m-final"])])
    #expect(try await h.source.checkForChanges() == nil)
    #expect(await h.sync.token(for: "c1", scope: "me@x.com") == "m-final")
}

@Test func unchangedPollReportsNothingAndAdvancesTheToken() async throws {
    let h = try await Harness()
    await bootstrapRoutes(h)
    _ = try await h.source.checkForChanges()
    await h.transport.route("syncToken=t1", [.json(["items": [], "nextSyncToken": "t2"])])
    await h.transport.route("syncToken=m1", [.json(["items": [], "nextSyncToken": "m2"])])
    #expect(try await h.source.checkForChanges() == nil)
    #expect(await h.sync.token(for: "c1", scope: "me@x.com") == "m2")
    let poll = try #require(await h.transport.requests(matching: "syncToken=m1").first)
    let url = poll.url.absoluteString
    #expect(url.contains("showDeleted=true") && url.contains("maxResults=250"))
    #expect(!url.contains("timeMin") && !url.contains("timeMax"))
}

@Test func changedItemsReportEventsChangedForThatCalendar() async throws {
    let h = try await Harness()
    await bootstrapRoutes(h)
    _ = try await h.source.checkForChanges()
    await h.transport.route("syncToken=t1", [.json(["items": [], "nextSyncToken": "t2"])])
    await h.transport.route("syncToken=m1", [.json(["items": [["id": "e9"]], "nextSyncToken": "m2"])])
    #expect(try await h.source.checkForChanges() == .eventsChanged(calendarIDs: ["me@x.com"]))
}

@Test func pollPagesUntilTheFinalTokenEvenWhenTheFirstPageHasChanges() async throws {
    let h = try await Harness()
    await bootstrapRoutes(h)
    _ = try await h.source.checkForChanges()
    await h.transport.route("syncToken=t1", [.json(["items": [], "nextSyncToken": "t2"])])
    await h.transport.route("syncToken=m1", [.json(["items": [["id": "e1"]], "nextPageToken": "page2"])])
    await h.transport.route("pageToken=page2", [.json(["items": [], "nextSyncToken": "m-final"])])
    #expect(try await h.source.checkForChanges() == .eventsChanged(calendarIDs: ["me@x.com"]))
    #expect(await h.sync.token(for: "c1", scope: "me@x.com") == "m-final")
}

@Test func expiredTokenRebootstrapsAndReportsChanged() async throws {
    let h = try await Harness()
    await bootstrapRoutes(h)
    _ = try await h.source.checkForChanges()
    await h.transport.route("syncToken=t1", [.json(["items": [], "nextSyncToken": "t2"])])
    await h.transport.route(me, [.json(["nextSyncToken": "m-new"])]) // the re-bootstrap (no syncToken in its URL)
    await h.transport.route("syncToken=m1", [.json(["error": ["code": 410]], status: 410)]) // registered last: wins for the poll
    #expect(try await h.source.checkForChanges() == .eventsChanged(calendarIDs: ["me@x.com"]))
    #expect(await h.sync.token(for: "c1", scope: "me@x.com") == "m-new")
}

@Test func anAddedCalendarReportsCalendarsChangedAndIsBaselined() async throws {
    let sync = InMemorySyncStateStore()
    let first = try await Harness(calendarList: listJSON(["me@x.com", "team@group.calendar.google.com"]), sync: sync)
    await bootstrapRoutes(first)
    _ = try await first.source.checkForChanges()

    let second = try await Harness(
        calendarList: listJSON(["me@x.com", "team@group.calendar.google.com", "new@group.calendar.google.com"]), sync: sync)
    await second.transport.route("syncToken=t1", [.json(["items": [], "nextSyncToken": "t2"])])
    await second.transport.route("syncToken=m1", [.json(["items": [], "nextSyncToken": "m2"])])
    await second.transport.route("calendars/new%40group.calendar.google.com/events", [.json(["nextSyncToken": "n1"])])
    #expect(try await second.source.checkForChanges() == .calendarsChanged)
    #expect(await sync.token(for: "c1", scope: "new@group.calendar.google.com") == "n1")
    #expect(await sync.token(for: "c1", scope: "_calendars")?.contains("new@group.calendar.google.com") == true)
}

@Test func aRemovedCalendarReportsCalendarsChangedAndClearsItsToken() async throws {
    let sync = InMemorySyncStateStore()
    let first = try await Harness(calendarList: listJSON(["me@x.com", "team@group.calendar.google.com"]), sync: sync)
    await bootstrapRoutes(first)
    _ = try await first.source.checkForChanges()

    let second = try await Harness(calendarList: listJSON(["me@x.com"]), sync: sync)
    await second.transport.route("syncToken=m1", [.json(["items": [], "nextSyncToken": "m2"])])
    #expect(try await second.source.checkForChanges() == .calendarsChanged)
    #expect(await sync.token(for: "c1", scope: "team@group.calendar.google.com") == nil)
}

@Test func aFailureMidBootstrapStoresNothingAndConvergesOnRetry() async throws {
    let h = try await Harness()
    await h.transport.route(team, [.json(["nextSyncToken": "t1"])])
    await h.transport.route(me, [.json([:], status: 503), .json(["nextSyncToken": "m1"])])
    await #expect(throws: SourceError.server(status: 503)) { try await h.source.checkForChanges() }
    #expect(await h.sync.token(for: "c1", scope: "_calendars") == nil)
    #expect(try await h.source.checkForChanges() == nil)
    #expect(await h.sync.token(for: "c1", scope: "me@x.com") == "m1")
    #expect(await h.sync.token(for: "c1", scope: "_calendars") != nil)
}

@Test func monitorEmitsChangesFromTheSourceStream() async throws {
    let h = try await Harness()
    await bootstrapRoutes(h)
    await h.transport.route("syncToken=t1", [.json(["items": [], "nextSyncToken": "t2"])])
    await h.transport.route("syncToken=m1", [.json(["items": [["id": "x"]], "nextSyncToken": "m2"])])
    // The first check is the baseline (nil); the second finds the change. The harness monitor never sleeps.
    var iterator = h.source.changes().makeAsyncIterator()
    #expect(await iterator.next() == .eventsChanged(calendarIDs: ["me@x.com"]))
}
```
- [ ] **Step 2: Run to verify failure**

Run: `swift test --package-path Packages/CalendarConnectors --filter ChangeDetectionTests`
Expected: FAIL (assertions: the stub returns nil, so `.eventsChanged`/`.calendarsChanged` expectations fail; token expectations fail).

- [ ] **Step 3: Implement**

In `Sources/GoogleCalendar/GoogleCalendarSource.swift` replace the stub `checkForChanges` with:
```swift
    static let calendarSetScope = "_calendars"

    private struct SyncPageDTO: Decodable {
        struct Item: Decodable { var id: String }
        var items: [Item]?
        var nextPageToken: String?
        var nextSyncToken: String?
    }

    public func checkForChanges() async throws -> CalendarChange? {
        let calendars = try await self.calendars()
        let ids = calendars.map(\.id).sorted()
        let setKey = ids.joined(separator: "\n")
        let connectionID = connection.connectionID

        let previousKey = await syncState.token(for: connectionID, scope: Self.calendarSetScope)
        let setChanged = previousKey != nil && previousKey != setKey
        if let previousKey, setChanged {
            for removed in Set(previousKey.split(separator: "\n").map(String.init)).subtracting(ids) {
                await syncState.setToken(nil, for: connectionID, scope: removed)
            }
        }

        var changed = Set<String>()
        for id in ids {
            do {
                if let token = await syncState.token(for: connectionID, scope: id) {
                    if try await poll(calendarID: id, token: token) { changed.insert(id) }
                } else {
                    try await bootstrap(calendarID: id)
                }
            } catch let error as GoogleAPIError {
                if error == .notFound || error == .forbidden { continue } // removed or no longer readable
                throw error.sourceError
            }
        }
        await syncState.setToken(setKey, for: connectionID, scope: Self.calendarSetScope)

        if previousKey == nil { return nil } // first call: baseline only
        if setChanged { return .calendarsChanged }
        return changed.isEmpty ? nil : .eventsChanged(calendarIDs: changed)
    }

    /// Lists the whole calendar (no time window; Google forbids combining a sync token with one) only to obtain a token.
    private func bootstrap(calendarID: String) async throws {
        var token: String?
        try await api.pages(
            SyncPageDTO.self, path: GoogleAPIClient.calendarPath(calendarID, "/events"),
            query: [
                URLQueryItem(name: "showDeleted", value: "true"),
                URLQueryItem(name: "maxResults", value: "2500"),
                URLQueryItem(name: "fields", value: "nextPageToken,nextSyncToken"),
            ],
            next: { $0.nextPageToken }, handle: { if let t = $0.nextSyncToken { token = t } })
        guard let token else { throw SourceError.invalidResponse("no nextSyncToken") }
        await syncState.setToken(token, for: connection.connectionID, scope: calendarID)
    }

    /// True when anything changed since `token`. Always walks to the final page, because only it carries the new token.
    private func poll(calendarID: String, token: String) async throws -> Bool {
        var anyItems = false
        var newToken: String?
        do {
            try await api.pages(
                SyncPageDTO.self, path: GoogleAPIClient.calendarPath(calendarID, "/events"),
                query: [
                    URLQueryItem(name: "syncToken", value: token),
                    URLQueryItem(name: "showDeleted", value: "true"),
                    URLQueryItem(name: "maxResults", value: "250"),
                    URLQueryItem(name: "fields", value: "nextPageToken,nextSyncToken,items(id)"),
                ],
                next: { $0.nextPageToken },
                handle: {
                    if !($0.items ?? []).isEmpty { anyItems = true }
                    if let t = $0.nextSyncToken { newToken = t }
                })
        } catch GoogleAPIError.gone {
            await syncState.setToken(nil, for: connection.connectionID, scope: calendarID)
            try await bootstrap(calendarID: calendarID)
            return true
        }
        guard let newToken else { throw SourceError.invalidResponse("no nextSyncToken") }
        await syncState.setToken(newToken, for: connection.connectionID, scope: calendarID)
        return anyItems
    }
```

- [ ] **Step 4: Run to verify pass**

Run: `swift test --package-path Packages/CalendarConnectors --filter ChangeDetectionTests`
Expected: PASS (10 tests). Then run the whole package: `swift test --package-path Packages/CalendarConnectors` and expect all suites green.

Route-order note: `FakeTransport` picks the most recently registered matching route, so tests register the general calendar route first and the specific ones (`syncToken=...`, `pageToken=page2`) after; keep that order.

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "feat(google): incremental change detection with sync tokens

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 10: GoogleConnectorKind (authorize, reauthorize, makeSource)

**Files:**
- Create: `Sources/GoogleCalendar/GoogleConnectorKind.swift`, `Tests/GoogleCalendarTests/ConnectorKindTests.swift`

**Interfaces:**
- Consumes: `OAuthClient`, `OAuthConfig`, `PKCE`, `AccessTokenProvider`, `GoogleAPIClient`, `GoogleCalendarSource`, `Connection`, `AuthorizationInteraction`, `OAuthRedirectSession`.
- Produces: `public struct GoogleOAuthConfig(clientID:clientSecret:)`, `public struct GoogleConnectorKind: ConnectorKind` with `init(config:transport:now:sleep:pollInterval:)`, `static let kindID = "google"`, `authorize`, `reauthorize`, `makeSource`.

- [ ] **Step 1: Write the failing tests**

`Tests/GoogleCalendarTests/ConnectorKindTests.swift`:
```swift
import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import GoogleCalendar

private struct FakeSession: OAuthRedirectSession {
    let redirectURI = URL(string: "http://127.0.0.1:53211")!
    let recorder: SessionRecorder
    let mode: Mode
    enum Mode: Sendable { case ok, denied, wrongState }

    func authorize(at authorizationURL: URL) async throws -> URL {
        await recorder.opened(authorizationURL)
        let state = URLComponents(url: authorizationURL, resolvingAgainstBaseURL: false)!.queryItems!.first { $0.name == "state" }!.value!
        switch mode {
        case .ok: return URL(string: "http://127.0.0.1:53211/?code=the-code&state=\(state)")!
        case .denied: return URL(string: "http://127.0.0.1:53211/?error=access_denied&state=\(state)")!
        case .wrongState: return URL(string: "http://127.0.0.1:53211/?code=x&state=nope")!
        }
    }
    func close() async { await recorder.closed() }
}

private actor SessionRecorder {
    private(set) var authorizationURLs: [URL] = []
    private(set) var closeCount = 0
    func opened(_ url: URL) { authorizationURLs.append(url) }
    func closed() { closeCount += 1 }
}

private struct FakeInteraction: AuthorizationInteraction {
    let recorder = SessionRecorder()
    var mode: FakeSession.Mode = .ok
    func beginOAuthRedirect() async throws -> any OAuthRedirectSession { FakeSession(recorder: recorder, mode: mode) }
    func promptCredentials(_ fields: [CredentialField]) async throws -> [String: String] { [:] }
}

private func kind(_ transport: FakeTransport, now: TestNow = TestNow()) -> GoogleConnectorKind {
    GoogleConnectorKind(
        config: GoogleOAuthConfig(clientID: "cid", clientSecret: "csecret"),
        transport: transport, now: now.provider, sleep: { _ in }, pollInterval: .seconds(60))
}

private func routes(_ transport: FakeTransport, email: String = "me@x.com", refresh: String? = "rt-1") async {
    var token: [String: Any] = ["access_token": "at", "expires_in": 3600]
    if let refresh { token["refresh_token"] = refresh }
    await transport.route("oauth2.googleapis.com/token", [.json(token)])
    await transport.route("users/me/calendarList", [.json(["items": [["id": email, "primary": true, "accessRole": "owner"]]])])
}

@Test func authorizeRunsPKCEFlowStoresRefreshTokenAndReturnsAConnection() async throws {
    let transport = FakeTransport()
    await routes(transport)
    let store = InMemoryCredentialStore()
    let interaction = FakeInteraction()
    let connection = try await kind(transport).authorize(using: interaction, credentials: store)

    #expect(connection.kindID == "google" && connection.displayName == "me@x.com" && connection.config["email"] == "me@x.com")
    #expect(connection.sourceID == "google-\(connection.connectionID)")
    #expect(UUID(uuidString: connection.connectionID) != nil)
    let secrets = try #require(try await store.secrets(for: connection.connectionID))
    #expect(secrets[AccessTokenProvider.refreshTokenKey] == "rt-1")

    let url = try #require(await interaction.recorder.authorizationURLs.first)
    let q = Dictionary(uniqueKeysWithValues: URLComponents(url: url, resolvingAgainstBaseURL: false)!.queryItems!.map { ($0.name, $0.value ?? "") })
    #expect(q["client_id"] == "cid" && q["redirect_uri"] == "http://127.0.0.1:53211")
    #expect(q["access_type"] == "offline" && q["prompt"] == "consent" && q["code_challenge_method"] == "S256")
    #expect(q["scope"] == "https://www.googleapis.com/auth/calendar.events https://www.googleapis.com/auth/calendar.calendarlist.readonly")
    #expect(await interaction.recorder.closeCount == 1)

    let exchange = try #require(await transport.requests(matching: "oauth2.googleapis.com/token").first)
    let body = String(decoding: exchange.body ?? Data(), as: UTF8.self)
    #expect(body.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A53211") && body.contains("code=the-code"))
    #expect(body.contains("client_secret=csecret"))
}

@Test func authorizeWithoutARefreshTokenFailsAndStoresNothing() async throws {
    let transport = FakeTransport()
    await routes(transport, refresh: nil)
    let store = InMemoryCredentialStore()
    let interaction = FakeInteraction()
    await #expect(throws: SourceError.self) { try await kind(transport).authorize(using: interaction, credentials: store) }
    #expect(await interaction.recorder.closeCount == 1)
}

@Test func authorizeFailureAfterTokenExchangeStoresNothing() async throws {
    let transport = FakeTransport()
    await transport.route("oauth2.googleapis.com/token", [.json(["access_token": "at", "expires_in": 3600, "refresh_token": "rt"])])
    await transport.route("users/me/calendarList", [.json([:], status: 500)])
    let store = InMemoryCredentialStore()
    await #expect(throws: SourceError.server(status: 500)) {
        try await kind(transport).authorize(using: FakeInteraction(), credentials: store)
    }
    // Nothing is keyed by an id the caller never received: the store is empty for every id we could have used.
    #expect(await store.isEmpty)
}

@Test func userDenyingAccessThrowsCancelledAndClosesTheSession() async throws {
    let transport = FakeTransport()
    await routes(transport)
    let interaction = FakeInteraction(mode: .denied)
    await #expect(throws: AuthorizationError.cancelled) {
        try await kind(transport).authorize(using: interaction, credentials: InMemoryCredentialStore())
    }
    #expect(await interaction.recorder.closeCount == 1)
    #expect(await transport.requests(matching: "oauth2.googleapis.com/token").isEmpty)
}

@Test func aMismatchedStateIsRejected() async throws {
    let transport = FakeTransport()
    await routes(transport)
    await #expect(throws: AuthorizationError.stateMismatch) {
        try await kind(transport).authorize(using: FakeInteraction(mode: .wrongState), credentials: InMemoryCredentialStore())
    }
}

@Test func reauthorizeKeepsTheConnectionIDAndReplacesSecrets() async throws {
    let transport = FakeTransport()
    await routes(transport, refresh: "rt-2")
    let store = InMemoryCredentialStore()
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt-old"], for: "keep-me")
    let existing = Connection(kindID: "google", connectionID: "keep-me", displayName: "me@x.com", config: ["email": "me@x.com"])
    let result = try await kind(transport).reauthorize(existing, using: FakeInteraction(), credentials: store)
    #expect(result == existing)
    #expect(try await store.secrets(for: "keep-me")?[AccessTokenProvider.refreshTokenKey] == "rt-2")
}

@Test func reauthorizeAsADifferentAccountIsRejectedAndKeepsOldSecrets() async throws {
    let transport = FakeTransport()
    await routes(transport, email: "other@x.com", refresh: "rt-2")
    let store = InMemoryCredentialStore()
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt-old"], for: "keep-me")
    let existing = Connection(kindID: "google", connectionID: "keep-me", displayName: "me@x.com", config: ["email": "me@x.com"])
    await #expect(throws: SourceError.invalidResponse("signed in as a different account")) {
        try await kind(transport).reauthorize(existing, using: FakeInteraction(), credentials: store)
    }
    #expect(try await store.secrets(for: "keep-me")?[AccessTokenProvider.refreshTokenKey] == "rt-old")
}

@Test func makeSourceRefreshesAndListsCalendarsWithoutPrompting() async throws {
    let transport = FakeTransport()
    await transport.route("oauth2.googleapis.com/token", [.json(["access_token": "fresh", "expires_in": 3600])])
    await transport.route("users/me/calendarList", [.json(["items": [["id": "me@x.com", "primary": true, "accessRole": "owner"]]])])
    let store = InMemoryCredentialStore()
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt"], for: "c9")
    let connection = Connection(kindID: "google", connectionID: "c9", displayName: "me@x.com", config: ["email": "me@x.com"])
    let source = try kind(transport).makeSource(for: connection, credentials: store, syncState: InMemorySyncStateStore())
    #expect(source.id == "google-c9")
    #expect(try await source.calendars().map(\.id) == ["me@x.com"])
    #expect(await transport.requests(matching: "calendarList").first?.headers["Authorization"] == "Bearer fresh")
}

@Test func kindMetadata() {
    let k = kind(FakeTransport())
    #expect(k.id == "google" && k.displayName == "Google")
    #expect(k.supportedPlatforms.contains(.macOS) && k.supportedPlatforms.contains(.linux))
    if case .oauth = k.authorization {} else { Issue.record("expected oauth") }
}
```
This test file uses `InMemoryCredentialStore.isEmpty`. Add it in Step 3 (a small public `var isEmpty: Bool { get async }` on the actor).

- [ ] **Step 2: Run to verify failure**

Run: `swift test --package-path Packages/CalendarConnectors --filter ConnectorKindTests`
Expected: FAIL to compile ("cannot find 'GoogleConnectorKind' in scope").

- [ ] **Step 3: Implement**

Add to `InMemoryCredentialStore` in `Sources/CalendarCore/Connection.swift`:
```swift
    public var isEmpty: Bool { storage.isEmpty }
```
`Sources/GoogleCalendar/GoogleConnectorKind.swift`:
```swift
import CalendarCore
import Foundation

/// The host app's Google Cloud OAuth client (type Desktop). Supplied from git-ignored configuration; never embedded here.
public struct GoogleOAuthConfig: Sendable {
    public var clientID: String
    public var clientSecret: String
    public init(clientID: String, clientSecret: String) {
        self.clientID = clientID
        self.clientSecret = clientSecret
    }
}

public struct GoogleConnectorKind: ConnectorKind {
    public static let kindID = "google"
    public static let scopes = [
        "https://www.googleapis.com/auth/calendar.events",
        "https://www.googleapis.com/auth/calendar.calendarlist.readonly",
    ]

    public var id: String { Self.kindID }
    public var displayName: String { "Google" }
    public var supportedPlatforms: Platform { [.macOS, .iOS, .linux, .windows] }
    public var authorization: AuthorizationMethod { .oauth }

    private let oauth: OAuthClient
    private let transport: any HTTPTransport
    private let now: @Sendable () -> Date
    private let sleep: Sleeper
    private let pollInterval: Duration

    public init(
        config: GoogleOAuthConfig, transport: any HTTPTransport = URLSessionTransport(),
        now: @escaping @Sendable () -> Date = { Date() }, sleep: @escaping Sleeper = defaultSleeper,
        pollInterval: Duration = .seconds(60)
    ) {
        self.oauth = OAuthClient(
            config: OAuthConfig(
                authorizationEndpoint: URL(string: "https://accounts.google.com/o/oauth2/v2/auth")!,
                tokenEndpoint: URL(string: "https://oauth2.googleapis.com/token")!,
                clientID: config.clientID, clientSecret: config.clientSecret, scopes: Self.scopes,
                extraAuthParams: ["access_type": "offline", "prompt": "consent"]),
            transport: transport, now: now)
        self.transport = transport
        self.now = now
        self.sleep = sleep
        self.pollInterval = pollInterval
    }

    public func authorize(using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        let (email, refreshToken) = try await signIn(using: interaction)
        let connection = Connection(
            kindID: id, connectionID: UUID().uuidString, displayName: email, config: ["email": email])
        try await credentials.setSecrets([AccessTokenProvider.refreshTokenKey: refreshToken], for: connection.connectionID)
        return connection
    }

    public func reauthorize(
        _ connection: Connection, using interaction: any AuthorizationInteraction, credentials: any CredentialStore
    ) async throws -> Connection {
        let (email, refreshToken) = try await signIn(using: interaction)
        guard email == connection.config["email"] else {
            throw SourceError.invalidResponse("signed in as a different account")
        }
        try await credentials.setSecrets([AccessTokenProvider.refreshTokenKey: refreshToken], for: connection.connectionID)
        return connection
    }

    public func makeSource(
        for connection: Connection, credentials: any CredentialStore, syncState: any SyncStateStore
    ) throws -> any CalendarSource {
        let oauth = self.oauth
        let provider = AccessTokenProvider(
            connectionID: connection.connectionID, credentials: credentials,
            refresh: { try await oauth.refresh(refreshToken: $0) }, now: now)
        let api = GoogleAPIClient(transport: transport, tokens: provider, sleep: sleep)
        return GoogleCalendarSource(
            connection: connection, api: api, syncState: syncState, monitor: ChangeMonitor(interval: pollInterval, sleep: sleep))
    }

    /// Runs the browser flow and returns the account email (the primary calendar id) and the refresh token.
    /// Nothing is persisted here: callers store secrets only after this succeeds.
    private func signIn(using interaction: any AuthorizationInteraction) async throws -> (email: String, refreshToken: String) {
        let session = try await interaction.beginOAuthRedirect()
        let tokens: OAuthTokens
        do {
            let redirectURI = session.redirectURI
            let verifier = PKCE.randomString(length: 64)
            let state = PKCE.randomString(length: 32)
            let url = oauth.authorizationURL(redirectURI: redirectURI, state: state, codeChallenge: PKCE.challenge(for: verifier))
            let redirect = try await session.authorize(at: url)
            let code = try oauth.authorizationCode(from: redirect, expectedState: state)
            tokens = try await oauth.exchange(code: code, verifier: verifier, redirectURI: redirectURI)
        } catch {
            await session.close()
            throw error
        }
        await session.close()

        guard let refreshToken = tokens.refreshToken else {
            throw SourceError.invalidResponse("no refresh token returned")
        }
        let provider = AccessTokenProvider(
            connectionID: "signin", credentials: InMemoryCredentialStore(),
            refresh: { _ in throw SourceError.authExpired }, now: now, initial: tokens)
        let api = GoogleAPIClient(transport: transport, tokens: provider, sleep: sleep)
        guard let primary = try await api.calendarList().first(where: { $0.primary == true }) else {
            throw SourceError.invalidResponse("no primary calendar")
        }
        return (primary.id.lowercased(), refreshToken)
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `swift test --package-path Packages/CalendarConnectors`
Expected: the whole package passes (all suites). In `authorizeWithoutARefreshTokenFailsAndStoresNothing`, `FakeSession.close()` is called exactly once because `signIn` closes the session on both the error path and the success path (the refresh-token check happens after the session is closed).

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "feat(google): connector kind with authorize, reauthorize and makeSource

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 11: Docs, CI, Linux portability and review

**Files:**
- Create: `docs/decisions/0012-calendar-connector-library.md`
- Modify: `AGENTS.md`, `docs/architecture.md`, `.github/workflows/ci.yml`

- [ ] **Step 1: Write ADR 0012**

`docs/decisions/0012-calendar-connector-library.md`:
```markdown
# ADR 0012: Portable Calendar Connector Library

**Status:** Accepted

## Context

TimeTug reads calendars only through EventKit. Users also want to connect Google, Microsoft and CalDAV/iCloud accounts directly, with reads now and writes later, and the connectors should be reusable outside TimeTug. Google and Microsoft push notifications need a public HTTPS endpoint, which a desktop app does not have; iCloud has no third-party push at all. No Swift library maps across these providers.

## Decision

Build `Packages/CalendarConnectors` in this repository, extract it to its own repository after the Google connector has been used by TimeTug (Phase 2). It imports nothing from TimeTug; the package has no dependency on TimeTug packages, so the compiler enforces that.

- **Portable:** `CalendarCore` (model, `CalendarSource`, `ConnectorKind`, OAuth2 PKCE, `ChangeMonitor`) is pure Swift 6 and builds on Linux. No MSAL and no `GoogleAPIClientForREST`; networking goes through an injected `HTTPTransport`. The one dependency is `swift-crypto` (SHA-256 for PKCE).
- **The host app owns platform parts:** `CredentialStore` (Keychain on macOS), the browser and loopback redirect (`AuthorizationInteraction`), persistence of `Connection`s, and the OAuth client id/secret. EventKit stays in TimeTug as its own connector.
- **Naming:** the library's generic event is `CalendarEvent`; TimeTug's own event (with dedup fields) becomes `TimeTugCalendarEvent` in Phase 2.
- **Change detection:** poll using each provider's incremental sync token (Google `syncToken`, Graph delta, CalDAV sync-collection); `capabilities.syncKind`/`supportsPush` are the seam for a later webhook relay holding a websocket to the app. Google and Microsoft offer no persistent client connection.
- **Capabilities are per connector, not per provider:** iCloud via EventKit cannot invite or RSVP; via CalDAV (RFC 6638 server-side scheduling) it can.
- **Phases:** 1 library + read-only Google; 2 plug into TimeTug (Accounts pane, dynamic sources, Keychain); 3 write capabilities; 4 Microsoft.

## Google specifics

- OAuth for desktop apps uses a loopback redirect. Calendar scopes are "sensitive": while the Google Cloud project is in Testing only listed test users can sign in and refresh tokens expire after 7 days; a public release needs Google's OAuth verification.
- `syncToken` cannot be combined with `timeMin`/`timeMax` and requires `showDeleted=true`, so change detection lists whole calendars (field-minimal) only to obtain and advance tokens.

## Consequences

- Reads are provider-neutral; TimeTug maps `CalendarEvent` into `TimeTugCalendarEvent`.
- The first change check after connecting costs one field-minimal full listing per calendar; persisting sync tokens across launches (Phase 2) avoids repeating it.
- Extraction later is a `git subtree split`.
```

- [ ] **Step 2: Update AGENTS.md and architecture.md**

In `AGENTS.md`, under `## Layout` add after the `Packages/EventKitSource` line:
```
- `Packages/CalendarConnectors`: portable connector library (ADR 0012). `CalendarCore` (model, `CalendarSource`, `ConnectorKind`, OAuth PKCE, `ChangeMonitor`; pure Swift, builds on Linux) and `GoogleCalendar` (read-only Google connector). Imports nothing from TimeTug; TimeTug is not wired to it yet. The host app supplies `CredentialStore`, the OAuth browser/loopback redirect and persistence.
```
Under `## Commands` add: ``- Connector library tests: `swift test --package-path Packages/CalendarConnectors` (first resolve fetches swift-crypto).``
In `docs/architecture.md` under `## Modules` add a bullet: ``- `Packages/CalendarConnectors`: portable calendar connector library (ADR 0012); not yet used by the app.``

- [ ] **Step 3: CI**

In `.github/workflows/ci.yml`, in the `core` job add after the `Test AppleIntelligenceInference` step:
```yaml
      - name: Test CalendarConnectors
        run: swift test --package-path Packages/CalendarConnectors
```
and update the header comment's `core` line to mention `Packages/CalendarConnectors`. In the `core-linux` job add after `Test TimeTugCore`:
```yaml
      - name: Test CalendarConnectors
        run: swift test --package-path Packages/CalendarConnectors
```
(`core-linux` is `continue-on-error`, which is where a portability regression should show first.)

- [ ] **Step 4: Verify the full package and the untouched packages**

Run:
```bash
swift test --package-path Packages/CalendarConnectors
swift test --package-path Packages/TimeTugCore
swift build --package-path Packages/EventKitSource
git status --short
```
Expected: connector tests all pass; TimeTugCore tests pass (unchanged); EventKitSource builds; `git status` shows only the new package, ADR, and edits to `AGENTS.md`, `docs/architecture.md`, `.github/workflows/ci.yml`, `.gitignore` (if edited).

Linux portability (if Docker is available; otherwise rely on the CI job): `docker run --rm -v "$PWD":/w -w /w swift:6.0 swift test --package-path Packages/CalendarConnectors`. Expected: passes. Fix any Apple-only API found (most likely spots: `NSLock.withLock`, `ISO8601DateFormatter`, `URLSession` needing `FoundationNetworking`).

- [ ] **Step 5: Commit**

```bash
git add docs/decisions/0012-calendar-connector-library.md AGENTS.md docs/architecture.md .github/workflows/ci.yml
git commit -m "docs+ci: ADR 0012, layout notes, connector library in CI

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

- [ ] **Step 6: DeepSeek review of the whole diff (per the user's instruction)**

Use the `deepseek-review` skill: `--files` = every `.swift` file under `Packages/CalendarConnectors/Sources` plus `Package.swift` (list in a file), `--context` = the spec and this plan and the test files, `--background` = the spec's decisions (poll with sync tokens, host supplies credentials/redirect, read-only Phase 1, no Apple-only imports), and targeted `--question`s such as: (1) trace `checkForChanges` for calendar added/removed, 410, mid-bootstrap failure and first call; (2) `AccessTokenProvider` single-flight, cancellation of one caller, rotated token persistence; (3) `ChangeMonitor` termination, backoff overflow, `authExpired`; (4) Google mapping of all-day/timezone/cancelled/recurring instances; (5) authorize/reauthorize secret handling (nothing stored on failure, session closed on every path); (6) Swift 6 concurrency and Sendable problems; (7) Linux portability. Run `--dry-run` first (a secret-scan false positive on a `token`/`secret` identifier may need `--allow FILE:LINE` after verifying by eye; the only credential-shaped literals are test fixtures like `"csecret"`). Verify every Critical/Important finding against the code, fix real ones with a failing test first, and report real vs false counts to the user.

- [ ] **Step 7: Final verification before claiming done**

Use `superpowers:verification-before-completion`: re-run the three commands from Step 4, confirm output, then report.

---

## Self-Review

**Spec coverage:** Package/deps (T1, T11) · Model incl. composite id, all-day rule, `myResponse`, `kind` (T1, T7) · `CalendarSource`/`PollingCalendarSource`/capabilities/`CalendarChange` incl. `.sourceFailed` (T1) · `ChangeMonitor` interval/backoff/clamp/auth terminal/single subscriber (T6) · Connection/`sourceID`/`CredentialStore` keyed map/`SyncStateStore.removeAll`/in-memory stores/registry/platform/auth protocols/`reauthorize` (T2) · PKCE, `OAuthClient`, state validation, `access_denied` (T3, T4) · `AccessTokenProvider` cache/margin/single-flight/rotation/invalidate (T5) · Google mapping incl. all-day zone rule, cancelled dropped, conference, reminders, series, calendar descriptors (T7) · API client 401 retry, rate-limit backoff, 5xx, paging, 404/403 skip (T8) · `checkForChanges` bootstrap/poll/410/paging/`showDeleted=true`/calendar-set/mid-bootstrap failure (T9) · `authorize`/`reauthorize`/`makeSource`, secrets stored only after success, session closed, same redirect URI (T10) · ADR/AGENTS/architecture/CI/Linux (T11) · DeepSeek review (T11).
Deliberate spec deviations, called out here: `ChangeMonitor` takes an injected `Sleeper` closure instead of a `Clock` (simpler and equally deterministic); tests inject `HTTPTransport` fakes instead of `URLProtocol`; the spec's separate `CalendarTestSupport` target is new.

**Placeholder scan:** none.

**Type consistency:** `Sleeper`/`defaultSleeper` (T6) used in T8/T10; `AccessTokenProvider.refreshTokenKey` (T5) used in T8/T10 tests; `Harness(calendarList:sync:)` (T9 edit) matches uses; `GoogleAPIClient(transport:tokens:sleep:)` matches T8 tests and T10; `GoogleCalendarSource.init(connection:api:syncState:monitor:)` matches T8 tests and T10; `GoogleAPIError` cases `gone/notFound/forbidden` used consistently; `calendarSetScope = "_calendars"` matches T9 tests; `InMemoryCredentialStore.isEmpty` added in T10 before use.
