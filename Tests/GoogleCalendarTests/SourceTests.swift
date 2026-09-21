import CalendarCore
import CalendarOAuth
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
        let counter = RefreshCounter()
        let provider = AccessTokenProvider(
            connectionID: "c1", credentials: store,
            refresh: { _ in OAuthTokens(accessToken: "at\(counter.next())", expiresAt: now.date.addingTimeInterval(3600)) }, now: now.provider)
        let api = GoogleAPIClient(transport: transport, tokens: provider, sleep: { await sleeps.record($0) })
        source = GoogleCalendarSource(connection: connection, api: api, syncState: sync, monitor: ChangeMonitor(sleep: { _ in }))
        await transport.route("users/me/calendarList", [.json(calendarList)])
    }
}

/// Hands out 1, 2, 3... so each token refresh yields a distinct access token ("at1", "at2", ...).
final class RefreshCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var value = 0
    func next() -> Int { lock.withLock { value += 1; return value } }
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
    #expect(request.headers["Authorization"] == "Bearer at1")
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
    let h = try await Harness(calendarList: listJSON(["me@x.com"]))
    await h.transport.route("calendars/me%40x.com/events", [.json([:], status: 401), .json(["items": []])])
    _ = try await h.source.events(in: DateInterval(start: .now, duration: 3600))
    let requests = await h.transport.requests(matching: "calendars/me%40x.com/events")
    #expect(requests.map { $0.headers["Authorization"] } == ["Bearer at1", "Bearer at2"])
}

@Test func aForbiddenReasonThatAffectsEveryCalendarIsNotSwallowed() async throws {
    let h = try await Harness(calendarList: listJSON(["me@x.com"]))
    await h.transport.route("calendars/me%40x.com/events", [.json(["error": ["errors": [["reason": "accessNotConfigured"]]]], status: 403)])
    await #expect(throws: SourceError.invalidResponse("HTTP 403: accessNotConfigured")) {
        try await h.source.events(in: DateInterval(start: .now, duration: 3600))
    }
}

@Test func insufficientPermissionsNeedsReconsent() async throws {
    let h = try await Harness(calendarList: listJSON(["me@x.com"]))
    await h.transport.route("calendars/me%40x.com/events", [.json(["error": ["errors": [["reason": "insufficientPermissions"]]]], status: 403)])
    await #expect(throws: SourceError.authExpired) {
        try await h.source.events(in: DateInterval(start: .now, duration: 3600))
    }
}

@Test func anUnparseableForbiddenBodyIsInvalidResponseNotSkipped() async throws {
    let h = try await Harness(calendarList: listJSON(["me@x.com"]))
    await h.transport.route("calendars/me%40x.com/events", [.text("nope", status: 403)])
    await #expect(throws: SourceError.invalidResponse("HTTP 403: unknown")) {
        try await h.source.events(in: DateInterval(start: .now, duration: 3600))
    }
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

@Test func anItemWithoutAnIdIsDroppedNotFatal() async throws {
    let h = try await Harness()
    await h.transport.route("calendars/team%40group.calendar.google.com/events", [.json(["items": []])])
    await h.transport.route("calendars/me%40x.com/events", [.json(["items": [
        eventJSON("one", start: "2026-09-21T10:00:00Z"), ["noId": true], eventJSON("two", start: "2026-09-21T11:00:00Z"),
    ]])])
    let events = try await h.source.events(in: DateInterval(start: .now, duration: 3600))
    #expect(events.map(\.eventID) == ["one", "two"])
}

@Test func anItemWithAWronglyTypedFieldIsDroppedNotFatal() async throws {
    let h = try await Harness()
    var bad = eventJSON("bad", start: "2026-09-21T09:00:00Z")
    bad["reminders"] = ["overrides": [["minutes": "ten"]]]
    await h.transport.route("calendars/team%40group.calendar.google.com/events", [.json(["items": []])])
    await h.transport.route("calendars/me%40x.com/events", [.json(["items": [bad, eventJSON("good", start: "2026-09-21T10:00:00Z")]])])
    let events = try await h.source.events(in: DateInterval(start: .now, duration: 3600))
    #expect(events.map(\.eventID) == ["good"])
}
