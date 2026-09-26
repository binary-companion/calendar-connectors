import CalendarCore
import CalendarOAuth
import CalendarTestSupport
import Foundation
import Testing
@testable import MicrosoftCalendar

/// A small stateful Graph server: one writable calendar `cal1`, events created, read, patched, deleted and answered
/// through the same URLs the source uses. Just enough Graph to run the shared conformance checks; the request-shape
/// tests use `FakeTransport` instead.
actor FakeGraph: HTTPTransport {
    private(set) var requests: [HTTPRequest] = []
    private var events: [String: [String: Any]] = [:]
    private var counter = 0
    /// When set, the next POST to create an event fails with this response (then the fake behaves normally again).
    var failNextCreate: HTTPResponse?

    func setFailNextCreate(_ response: HTTPResponse?) { failNextCreate = response }
    func event(_ id: String) -> [String: Any]? { events[id] }
    func requests(method: String, containing text: String) -> [HTTPRequest] {
        requests.filter { $0.method == method && $0.url.absoluteString.contains(text) }
    }

    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        requests.append(request)
        let url = request.url
        let path = url.path
        let body = (try? JSONSerialization.jsonObject(with: request.body ?? Data())) as? [String: Any] ?? [:]
        if path.hasSuffix("/mailboxSettings/timeZone") { return .json(["value": "UTC"]) }
        if path == "/v1.0/me/calendars" { return .json(calendarsJSON) }
        let marker = "/v1.0/me/calendars/cal1/"
        guard path.hasPrefix(marker) else { return graphError("ErrorItemNotFound", status: 404) }
        let tail = String(path.dropFirst(marker.count))
        if tail == "calendarView" { return calendarView(url) }
        if tail == "events", request.method == "POST" { return create(body) }
        guard tail.hasPrefix("events/") else { return graphError("ErrorItemNotFound", status: 404) }
        let parts = tail.dropFirst("events/".count).split(separator: "/").map(String.init)
        guard let id = parts.first, events[id] != nil else { return graphError("ErrorItemNotFound", status: 404) }
        if parts.count == 2 { return act(parts[1], id: id, body: body) }
        switch request.method {
        case "GET": return .json(events[id]!)
        case "PATCH": return patch(id, body)
        case "DELETE": events[id] = nil; return HTTPResponse(status: 204)
        default: return graphError("BadRequest", status: 400)
        }
    }

    private func create(_ body: [String: Any]) -> HTTPResponse {
        if let failure = failNextCreate { failNextCreate = nil; return failure }
        if let transaction = body["transactionId"] as? String,
           let existing = events.values.first(where: { $0["transactionId"] as? String == transaction }) { return .json(existing, status: 201) }
        counter += 1
        var event = body
        event["id"] = "ev\(counter)"
        event["iCalUId"] = "uid-\(counter)"
        event["changeKey"] = "ck\(counter)"
        event["isCancelled"] = false
        event["type"] = body["recurrence"] == nil ? "singleInstance" : "seriesMaster"
        events["ev\(counter)"] = event
        return .json(event, status: 201)
    }

    private func patch(_ id: String, _ body: [String: Any]) -> HTTPResponse {
        var event = events[id]!
        for (key, value) in body { event[key] = value }
        counter += 1
        event["changeKey"] = "ck\(counter)"
        events[id] = event
        return .json(event)
    }

    private func act(_ action: String, id: String, body: [String: Any]) -> HTTPResponse {
        let responses = ["accept": "accepted", "tentativelyAccept": "tentativelyAccepted", "decline": "declined"]
        guard let response = responses[action] else { return graphError("BadRequest", status: 400) }
        var event = events[id]!
        event["responseStatus"] = ["response": response]
        var attendees = event["attendees"] as? [[String: Any]] ?? []
        for index in attendees.indices where ((attendees[index]["emailAddress"] as? [String: Any])?["address"] as? String) == "me@x.com" {
            attendees[index]["status"] = ["response": response]
        }
        event["attendees"] = attendees
        events[id] = event
        return HTTPResponse(status: 202)
    }

    private func calendarView(_ url: URL) -> HTTPResponse {
        let items = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems ?? []
        let from = items.first { $0.name == "startDateTime" }?.value.flatMap(GraphTime.parseInstant) ?? .distantPast
        let to = items.first { $0.name == "endDateTime" }?.value.flatMap(GraphTime.parseInstant) ?? .distantFuture
        let utc = TimeZone(identifier: "UTC")!
        let inWindow = events.values.filter { event in
            guard let start = ((event["start"] as? [String: Any])?["dateTime"] as? String).flatMap({ GraphTime.parse($0, in: utc) }),
                  let end = ((event["end"] as? [String: Any])?["dateTime"] as? String).flatMap({ GraphTime.parse($0, in: utc) })
            else { return false }
            return start < to && end > from
        }
        return .json(["value": inWindow.sorted { ($0["id"] as? String ?? "") < ($1["id"] as? String ?? "") }])
    }
}

/// A source over `FakeGraph`.
struct GraphServerHarness {
    let graph = FakeGraph()
    let source: MicrosoftCalendarSource

    init() async throws {
        let now = TestNow()
        let store = InMemoryCredentialStore()
        try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt"], for: "c1")
        let provider = AccessTokenProvider(
            connectionID: "c1", credentials: store,
            refresh: { _ in OAuthTokens(accessToken: "at", expiresAt: now.date.addingTimeInterval(3600)) }, now: now.provider)
        let api = GraphAPIClient(transport: graph, tokens: provider, sleep: { _ in })
        let connection = Connection(kindID: "microsoft", connectionID: "c1", displayName: "me@x.com", config: ["email": "me@x.com"])
        source = MicrosoftCalendarSource(
            connection: connection, api: api, syncState: InMemorySyncStateStore(), monitor: ChangeMonitor(sleep: { _ in }), now: now.provider)
    }
}

@Test func theSourcePassesTheWritableConformanceChecks() async throws {
    let h = try await GraphServerHarness()
    let window = DateInterval(start: Date(timeIntervalSince1970: 1_790_000_000), duration: 7 * 86_400)
    let problems = await WritableSourceConformance.violations(of: h.source, calendarID: "cal1", window: window)
    #expect(problems.isEmpty, "\(problems)")
}
