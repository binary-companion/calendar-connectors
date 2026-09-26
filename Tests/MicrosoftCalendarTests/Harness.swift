import CalendarCore
import CalendarOAuth
import CalendarTestSupport
import Foundation
import Testing
@testable import MicrosoftCalendar

/// Runs `body` and records an issue unless it throws exactly `expected`.
func expectWriteError(_ expected: WriteError, _ body: () async throws -> Void) async {
    do {
        try await body()
        Issue.record("expected \(expected), but nothing was thrown")
    } catch let error as WriteError {
        #expect(error == expected)
    } catch {
        Issue.record("expected \(expected), got \(error)")
    }
}

func bodyJSON(_ request: HTTPRequest) -> [String: Any] {
    (try? JSONSerialization.jsonObject(with: request.body ?? Data())) as? [String: Any] ?? [:]
}

/// Graph's error body.
func graphError(_ code: String, message: String = "m", status: Int, headers: [String: String] = [:]) -> HTTPResponse {
    .json(["error": ["code": code, "message": message]], status: status, headers: headers)
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

/// A token provider that always has a valid access token, plus the transport and sleep recorder around it.
struct ClientHarness {
    let transport = FakeTransport()
    let now = TestNow()
    let store = InMemoryCredentialStore()
    let sleeps = SleepRecorder()
    let client: GraphAPIClient

    init() async throws {
        try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt"], for: "c1")
        let now = self.now
        let counter = RefreshCounter()
        let provider = AccessTokenProvider(
            connectionID: "c1", credentials: store,
            refresh: { _ in OAuthTokens(accessToken: "at\(counter.next())", expiresAt: now.date.addingTimeInterval(3600)) }, now: now.provider)
        let sleeps = self.sleeps
        client = GraphAPIClient(transport: transport, tokens: provider, sleep: { await sleeps.record($0) })
    }
}

nonisolated(unsafe) let calendarsJSON: [String: Any] = [
    "value": [
        ["id": "cal1", "name": "Calendar", "hexColor": "#112233", "isDefaultCalendar": true, "canEdit": true, "canShare": true,
         "canViewPrivateItems": true, "owner": ["name": "Me", "address": "me@x.com"]],
        ["id": "cal2", "name": "Team", "isDefaultCalendar": false, "canEdit": false, "canShare": false, "canViewPrivateItems": false,
         "owner": ["name": "Boss", "address": "boss@x.com"]],
    ]
]

/// A calendars response with these calendar ids (all read-only).
func calendarsJSON(_ ids: [String]) -> [String: Any] {
    ["value": ids.map { ["id": $0, "name": $0, "canEdit": false] as [String: Any] }]
}

/// A source over a fake Graph. `mailboxZone` is what `/me/mailboxSettings/timeZone` answers.
struct SourceHarness {
    let transport = FakeTransport()
    let now = TestNow()
    let sync: InMemorySyncStateStore
    let store = InMemoryCredentialStore()
    let source: MicrosoftCalendarSource
    let sleeps = SleepRecorder()

    init(
        calendars: [String: Any] = calendarsJSON, mailboxZone: String = "Pacific Standard Time",
        sync: InMemorySyncStateStore = InMemorySyncStateStore()
    ) async throws {
        self.sync = sync
        let connection = Connection(kindID: "microsoft", connectionID: "c1", displayName: "me@x.com", config: ["email": "me@x.com"])
        try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt"], for: "c1")
        let now = self.now
        let counter = RefreshCounter()
        let provider = AccessTokenProvider(
            connectionID: "c1", credentials: store,
            refresh: { _ in OAuthTokens(accessToken: "at\(counter.next())", expiresAt: now.date.addingTimeInterval(3600)) }, now: now.provider)
        let sleeps = self.sleeps
        let api = GraphAPIClient(transport: transport, tokens: provider, sleep: { await sleeps.record($0) })
        source = MicrosoftCalendarSource(
            connection: connection, api: api, syncState: sync, monitor: ChangeMonitor(sleep: { _ in }), now: now.provider)
        await transport.route("me/mailboxSettings/timeZone", [.json(["value": mailboxZone])])
        await transport.route("me/calendars?", [.json(calendars)])
    }
}
