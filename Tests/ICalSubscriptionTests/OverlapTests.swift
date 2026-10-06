import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import ICalSubscription

/// Answers each request, in the order they arrive, with the next response, but only once the test releases it. That
/// lets a test finish overlapping fetches in any order.
private actor GatedTransport: HTTPTransport {
    private let responses: [HTTPResponse]
    private var waiting: [Int: CheckedContinuation<Void, Never>] = [:]
    private var released: Set<Int> = []
    private(set) var arrivals = 0
    private(set) var requests: [HTTPRequest] = []

    init(_ responses: [HTTPResponse]) { self.responses = responses }

    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        let index = arrivals
        arrivals += 1
        requests.append(request)
        let response = responses[min(index, responses.count - 1)]
        if !released.contains(index) { await withCheckedContinuation { waiting[index] = $0 } }
        return response
    }

    func release(_ index: Int) {
        released.insert(index)
        waiting.removeValue(forKey: index)?.resume()
    }

    func waitForArrivals(_ count: Int) async {
        while arrivals < count { await Task.yield() }
    }
}

private func makeSource(_ transport: any HTTPTransport, now: TestNow = TestNow(), retention: RetentionWindow? = nil) -> ICalSubscriptionSource {
    ICalSubscriptionSource(
        connection: Connection(kindID: "icalsub", connectionID: "c1", displayName: "My Meetups (www.example.test)", config: ["host": "www.example.test"]),
        link: { feedURL }, transport: transport, monitor: ChangeMonitor(interval: .seconds(900), sleep: { _ in }),
        maxAge: 900, now: now.provider, defaultZone: TimeZone(identifier: "Europe/Berlin")!, retention: retention)
}

private func ics(_ text: String) -> HTTPResponse { HTTPResponse(status: 200, body: Data(text.utf8)) }

@Test func overlappingChecksOfTheSameChangeReportItOnce() async throws {
    let old = ics(sampleFeed), new = ics(feedICS([boardGames]))
    let transport = GatedTransport([old, new, new])
    let source = makeSource(transport)
    let baseline = Task { try await source.checkForChanges() }
    await transport.waitForArrivals(1)
    await transport.release(0)
    #expect(try await baseline.value == nil)

    let first = Task { try await source.checkForChanges() }
    await transport.waitForArrivals(2)
    let second = Task { try await source.checkForChanges() }
    await transport.waitForArrivals(3)
    await transport.release(2)
    await transport.release(1)
    let results = try await [first.value, second.value]
    #expect(results.compactMap { $0 }.count == 1)
    #expect(results.compactMap { $0 } == [.eventsChanged(calendarIDs: ["feed"])])
}

@Test func anOlderFetchThatFinishesLastDoesNotReplaceTheNewerFeed() async throws {
    let old = ics(sampleFeed), new = ics(feedICS([boardGames]))
    let transport = GatedTransport([old, new])
    let source = makeSource(transport)
    let slow = Task { try await source.events(in: september) }
    await transport.waitForArrivals(1)
    let fast = Task { try await source.checkForChanges() }
    await transport.waitForArrivals(2)
    await transport.release(1)
    _ = try await fast.value
    await transport.release(0)
    _ = try await slow.value
    #expect(try await source.events(in: september).map(\.title) == ["Board games night"])
    #expect(await transport.arrivals == 2)
}

@Test func aChangeSeenOnlyByAnOlderFetchIsStillReportedOnceWithoutGoingBackwards() async throws {
    let old = ics(sampleFeed), new = ics(feedICS([boardGames]))
    // Check B starts after check A but the server answers it with the old feed; A's answer is the new one.
    let transport = GatedTransport([old, new, old, new])
    let source = makeSource(transport)
    let baseline = Task { try await source.checkForChanges() }
    await transport.waitForArrivals(1)
    await transport.release(0)
    #expect(try await baseline.value == nil)

    let older = Task { try await source.checkForChanges() }
    await transport.waitForArrivals(2)
    let newer = Task { try await source.checkForChanges() }
    await transport.waitForArrivals(3)
    await transport.release(2)
    #expect(try await newer.value == nil)
    await transport.release(1)
    #expect(try await older.value == .eventsChanged(calendarIDs: ["feed"]))
    #expect(try await source.events(in: september).map(\.title) == ["Board games night"])

    await transport.release(3)
    #expect(try await source.checkForChanges() == nil)
}

@Test func anOlderFetchThatFindsNothingNewDoesNotOverwriteNewerValidators() async throws {
    func versioned(_ text: String, _ etag: String) -> HTTPResponse {
        HTTPResponse(status: 200, headers: ["ETag": etag], body: Data(text.utf8))
    }
    let first = versioned(sampleFeed, "\"e1\""), second = versioned(feedICS([boardGames]), "\"e2\"")
    let transport = GatedTransport([first, first, second, second])
    let source = makeSource(transport)
    let baseline = Task { try await source.checkForChanges() }
    await transport.waitForArrivals(1)
    await transport.release(0)
    _ = try await baseline.value

    let slow = Task { try await source.checkForChanges() }
    await transport.waitForArrivals(2)
    let fast = Task { try await source.checkForChanges() }
    await transport.waitForArrivals(3)
    await transport.release(2)
    _ = try await fast.value
    await transport.release(1)
    _ = try await slow.value

    await transport.release(3)
    _ = try await source.checkForChanges()
    let last = try #require(await transport.requests.last)
    #expect(last.headers["If-None-Match"] == "\"e2\"")
}

@Test func anOlderFetchFinishingLateDoesNotPutAnOlderBodyBackAfterANewerUnchangedOne() async throws {
    func far(_ title: String, etag: String? = nil) -> HTTPResponse {
        let event = vevent(uid: "far@example.test", title: title, start: "20270125T100000", end: "20270125T110000")
        return HTTPResponse(status: 200, headers: etag.map { ["ETag": $0] } ?? [:], body: Data(feedICS([event]).utf8))
    }
    // The window (1 day back, 7 ahead of the start) leaves the far event out, so every edit to it is "unchanged".
    let now = TestNow()
    let transport = GatedTransport([far("Far v0", etag: "\"e0\""), far("Far v1"), far("Far v2"), HTTPResponse(status: 304)])
    let source = makeSource(transport, now: now, retention: RetentionWindow(daysBack: 1, daysAhead: 7))
    let baseline = Task { try await source.checkForChanges() }
    await transport.waitForArrivals(1)
    await transport.release(0)
    _ = try await baseline.value

    let older = Task { try await source.checkForChanges() }
    await transport.waitForArrivals(2)
    let newer = Task { try await source.checkForChanges() }
    await transport.waitForArrivals(3)
    await transport.release(2)
    _ = try await newer.value
    await transport.release(1)
    _ = try await older.value

    now.advance(5 * 86_400)
    await transport.release(3)
    #expect(try await source.checkForChanges() == .eventsChanged(calendarIDs: ["feed"]))
    #expect(try await source.events(in: DateInterval(start: now.date, duration: 30 * 86_400)).map(\.title) == ["Far v2"])
}
