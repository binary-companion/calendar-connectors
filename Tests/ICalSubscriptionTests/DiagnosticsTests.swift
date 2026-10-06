import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import ICalSubscription

// TestNow starts at 2027-01-15 00:00 Pacific, so this window is Jan 14 to Jan 22.
private let window = RetentionWindow(daysBack: 1, daysAhead: 7)
private let everything = DateInterval(start: pt(2026, 8, 1, 0), end: pt(2027, 12, 1, 0))

private struct ThrowingTransport: HTTPTransport {
    let error: any Error
    func send(_ request: HTTPRequest) async throws -> HTTPResponse { throw error }
}

private func makeSource(
    _ transport: any HTTPTransport, _ log: CollectingDiagnosticLog, now: TestNow = TestNow(), retention: RetentionWindow? = nil
) -> ICalSubscriptionSource {
    ICalSubscriptionSource(
        connection: Connection(kindID: "icalsub", connectionID: "c1", displayName: "x (www.example.test)", config: ["host": "www.example.test"]),
        link: { feedURL }, transport: transport, monitor: ChangeMonitor(interval: .seconds(900), sleep: { _ in }),
        maxAge: 900, now: now.provider, defaultZone: pacific, retention: retention, diagnostics: log)
}

private func ics(_ events: [[String]], etag: String? = nil) -> HTTPResponse {
    HTTPResponse(status: 200, headers: etag.map { ["ETag": $0] } ?? [:], body: Data(feedICS(events).utf8))
}

private func serve(_ responses: [HTTPResponse]) async -> FakeTransport {
    let transport = FakeTransport()
    await transport.route(privatePath, responses)
    return transport
}

private func oneOff(_ title: String, _ start: String, _ end: String) -> [String] {
    vevent(uid: "\(title.replacingOccurrences(of: " ", with: "-"))@example.test", title: title, start: start, end: end)
}

private func int(_ event: DiagnosticEvent?, _ name: String) -> Int? {
    if case .int(let value)? = event?.field(name)?.value { return value }
    return nil
}

private func bool(_ event: DiagnosticEvent?, _ name: String) -> Bool? {
    if case .bool(let value)? = event?.field(name)?.value { return value }
    return nil
}

private func string(_ event: DiagnosticEvent?, _ name: String) -> String? {
    if case .string(let value)? = event?.field(name)?.value { return value }
    return nil
}

private let farPast = oneOff("Far past", "20260901T100000", "20260901T110000")
private let farFuture = oneOff("Far future", "20270301T100000", "20270301T110000")
private let inWindow = oneOff("In window", "20270118T100000", "20270118T110000")

@Test func aNormalFetchReportsTheFetchAndTheParse() async throws {
    let log = CollectingDiagnosticLog()
    let source = makeSource(await serve([HTTPResponse(status: 200, body: Data(sampleFeed.utf8))]), log)
    _ = try await source.events(in: september)
    let fetched = try #require(log.events(named: "fetchCompleted").first)
    #expect(fetched.level == .info && fetched.category == "icalsub")
    #expect(int(fetched, "status") == 200 && bool(fetched, "conditional") == false && bool(fetched, "notModified") == false)
    #expect(int(fetched, "hops") == 0 && int(fetched, "bytes") == sampleFeed.utf8.count && int(fetched, "ms") != nil)
    let parsed = try #require(log.events(named: "feedParsed").first)
    #expect(parsed.level == .info)
    #expect(int(parsed, "eventsInFeed") == 2 && int(parsed, "groupsKept") == 2 && int(parsed, "groupsDropped") == 0)
    #expect(bool(parsed, "retentionActive") == false)
    #expect(log.events(named: "syntheticUID").isEmpty && log.events(named: "fetchFailed").isEmpty)
}

@Test func aRedirectCountsAsAHop() async throws {
    let transport = FakeTransport()
    await transport.route("example.test/events", [HTTPResponse(status: 302, headers: ["Location": "https://cdn.example.test/\(privatePath)/x"])])
    await transport.route("cdn.example.test", [ics([boardGames])])
    let log = CollectingDiagnosticLog()
    _ = try await makeSource(transport, log).events(in: september)
    #expect(int(log.events(named: "fetchCompleted").first, "hops") == 1)
}

@Test func aNotModifiedAnswerIsReported() async throws {
    let log = CollectingDiagnosticLog()
    let source = makeSource(await serve([ics([boardGames], etag: "\"v1\""), HTTPResponse(status: 304)]), log)
    _ = try await source.checkForChanges()
    _ = try await source.checkForChanges()
    let second = try #require(log.events(named: "fetchCompleted").last)
    #expect(int(second, "status") == 304 && bool(second, "conditional") == true && bool(second, "notModified") == true)
    #expect(int(second, "bytes") == 0)
    #expect(log.events(named: "feedParsed").count == 2)
}

@Test func failuresAreReportedWithAFixedReasonAndTheStatus() async throws {
    for (status, reason) in [(404, "authExpired"), (500, "server"), (418, "invalidResponse")] {
        let log = CollectingDiagnosticLog()
        let source = makeSource(await serve([HTTPResponse(status: status)]), log)
        await #expect(throws: (any Error).self) { try await source.events(in: september) }
        let failed = try #require(log.events(named: "fetchFailed").first, "\(status)")
        #expect(failed.level == .warning && string(failed, "reason") == reason && int(failed, "status") == status)
        #expect(log.events(named: "fetchCompleted").isEmpty)
    }
}

@Test func transportFailuresAndRefusedRedirectsHaveNoStatusOrLeakedText() async throws {
    let log = CollectingDiagnosticLog()
    let source = makeSource(ThrowingTransport(error: SourceError.network("failed: \(feedURL.absoluteString)")), log)
    await #expect(throws: (any Error).self) { try await source.events(in: september) }
    let failed = try #require(log.events(named: "fetchFailed").first)
    #expect(string(failed, "reason") == "network" && failed.field("status") == nil)

    let refused = CollectingDiagnosticLog()
    let redirect = HTTPResponse(status: 302, headers: ["Location": "http://example.test/x"])
    let refusing = makeSource(await serve([redirect]), refused)
    await #expect(throws: (any Error).self) { try await refusing.events(in: september) }
    #expect(string(refused.events(named: "fetchFailed").first, "reason") == "redirectRefused")

    let loop = CollectingDiagnosticLog()
    let again = HTTPResponse(status: 302, headers: ["Location": feedURL.absoluteString])
    let looping = makeSource(await serve([again]), loop)
    await #expect(throws: (any Error).self) { try await looping.events(in: september) }
    #expect(string(loop.events(named: "fetchFailed").first, "reason") == "tooManyRedirects")

    let big = CollectingDiagnosticLog()
    let body = HTTPResponse(status: 200, body: Data(count: FeedFetcher.maxBytes + 1))
    let large = makeSource(await serve([body]), big)
    await #expect(throws: (any Error).self) { try await large.events(in: september) }
    #expect(string(big.events(named: "fetchFailed").first, "reason") == "tooLarge")
}

@Test func aPageThatIsNotACalendarIsReported() async throws {
    let log = CollectingDiagnosticLog()
    let source = makeSource(await serve([HTTPResponse(status: 200, body: Data("<html>login</html>".utf8))]), log)
    await #expect(throws: (any Error).self) { try await source.events(in: september) }
    #expect(log.events(named: "fetchCompleted").count == 1)
    let event = try #require(log.events(named: "feedNotCalendar").first)
    #expect(event.level == .warning && event.fields.isEmpty)
    #expect(log.events(named: "feedParsed").isEmpty)
}

@Test func anUnreadableRuleIsKeptWhateverTheWindowAndReported() async throws {
    let broken = vevent(uid: "broken@example.test", title: "Odd series", start: "20260901T100000", end: "20260901T110000",
                        extra: ["RRULE:INTERVAL=2"])
    let log = CollectingDiagnosticLog()
    let source = makeSource(await serve([ics([broken, farPast, inWindow])]), log, retention: window)
    let titles = Set(try await source.events(in: everything).map(\.title))
    #expect(titles == ["Odd series", "In window"])
    let event = try #require(log.events(named: "rruleUnreadable").first)
    #expect(event.level == .notice && log.events(named: "rruleUnreadable").count == 1)
    let uid = try #require(event.field("uid"))
    #expect(uid.isPrivate && uid.value == .string("broken@example.test"))
    let parsed = try #require(log.events(named: "feedParsed").first)
    #expect(int(parsed, "groupsKept") == 2 && int(parsed, "groupsDropped") == 1)
}

private func broken(_ rule: String) -> [String] {
    vevent(uid: "broken@example.test", title: "Odd series", start: "20260901T100000", end: "20260901T110000", extra: ["RRULE:\(rule)"])
}

@Test func anUnreadableRuleIsReportedOncePerPollRunAndAgainWhenEditedOrReBroken() async throws {
    let log = CollectingDiagnosticLog()
    let fixed = vevent(uid: "broken@example.test", title: "Odd series", start: "20260901T100000", end: "20260901T110000", extra: ["RRULE:FREQ=WEEKLY"])
    let now = TestNow()
    let responses = [ics([broken("INTERVAL=2")], etag: "\"v1\""), HTTPResponse(status: 304), ics([broken("INTERVAL=2")], etag: "\"v2\""),
                     ics([broken("INTERVAL=3")]), ics([fixed]), ics([broken("INTERVAL=3")])]
    let source = makeSource(await serve(responses), log, now: now, retention: window)
    var counts: [Int] = []
    for _ in responses {
        _ = try await source.checkForChanges()
        counts.append(log.events(named: "rruleUnreadable").count)
    }
    // first sight, 304 re-parse, same rule again, edited rule, fixed, broken again
    #expect(counts == [1, 1, 1, 2, 2, 3])
    #expect(int(log.events(named: "feedParsed").first, "unreadableRules") == 1)
    #expect(int(log.events(named: "feedParsed")[4], "unreadableRules") == 0)
}

@Test func eventsWithoutAUIDAreCounted() async throws {
    let log = CollectingDiagnosticLog()
    let nameless = vevent(uid: nil, title: "Nameless one", start: "20260910T100000", end: "20260910T110000")
    let other = vevent(uid: nil, title: "Nameless two", start: "20260911T100000", end: "20260911T110000")
    let source = makeSource(await serve([ics([nameless, other, boardGames])]), log)
    _ = try await source.events(in: september)
    let event = try #require(log.events(named: "syntheticUID").first)
    #expect(event.level == .info && int(event, "count") == 2)
}

@Test func theRetentionWindowDropsAreCounted() async throws {
    let log = CollectingDiagnosticLog()
    let source = makeSource(await serve([ics([farPast, farFuture, inWindow])]), log, retention: window)
    _ = try await source.events(in: everything)
    let parsed = try #require(log.events(named: "feedParsed").first)
    #expect(int(parsed, "eventsInFeed") == 3 && int(parsed, "groupsKept") == 1 && int(parsed, "groupsDropped") == 2)
    #expect(bool(parsed, "retentionActive") == true)
}

@Test func aChangeIsReportedAndAnOutsideEditIsSuppressed() async throws {
    let editedFar = oneOff("Far future renamed", "20270301T100000", "20270301T110000")
    let editedIn = oneOff("In window renamed", "20270118T100000", "20270118T110000")
    let log = CollectingDiagnosticLog()
    let source = makeSource(await serve([ics([farFuture, inWindow]), ics([editedFar, inWindow]), ics([editedFar, editedIn])]), log, retention: window)
    _ = try await source.checkForChanges()
    #expect(log.events(named: "changeReported").isEmpty && log.events(named: "changeSuppressed").isEmpty)
    _ = try await source.checkForChanges()
    let suppressed = try #require(log.events(named: "changeSuppressed").first)
    #expect(suppressed.level == .info && string(suppressed, "reason") == "unchangedRetained")
    #expect(log.events(named: "changeReported").isEmpty)
    _ = try await source.checkForChanges()
    #expect(log.events(named: "changeReported").first?.level == .info)
    #expect(log.events(named: "changeSuppressed").count == 1)
}

@Test func signInIsReportedWithAReasonOnly() async throws {
    let accepted = CollectingDiagnosticLog()
    let kind = ICalSubscriptionKind(transport: await serve([ics([boardGames])]), diagnostics: accepted)
    _ = try await kind.authorize(using: StubInteraction(["link": feedURL.absoluteString]), credentials: InMemoryCredentialStore())
    let ok = try #require(accepted.events(named: "signInAccepted").first)
    #expect(ok.level == .info && ok.fields.allSatisfy { $0.name == "reason" })

    for (name, link, status, reason) in [("revoked", feedURL.absoluteString, 404, "authExpired"), ("bad link", "nonsense", 200, "invalidLink")] {
        let log = CollectingDiagnosticLog()
        let rejecting = ICalSubscriptionKind(transport: await serve([HTTPResponse(status: status)]), diagnostics: log)
        await #expect(throws: (any Error).self) {
            try await rejecting.authorize(using: StubInteraction(["link": link]), credentials: InMemoryCredentialStore())
        }
        let rejected = try #require(log.events(named: "signInRejected").first, "\(name)")
        #expect(rejected.level == .warning && string(rejected, "reason") == reason && rejected.fields.count == 1)
    }
}

@Test func noDiagnosticEverContainsTheLinkOrAnEventText() async throws {
    let log = CollectingDiagnosticLog()
    let titles = ["Board games night", "Weekly walk", "Far past", "Far future", "In window", "Odd series", "Nameless one"]
    let broken = vevent(uid: "broken@example.test", title: "Odd series", start: "20260901T100000", end: "20260901T110000", extra: ["RRULE:INTERVAL=2"])
    let nameless = vevent(uid: nil, title: "Nameless one", start: "20260910T100000", end: "20260910T110000")
    let feeds: [[HTTPResponse]] = [
        [HTTPResponse(status: 200, body: Data(sampleFeed.utf8))], [ics([broken, nameless, farPast, farFuture, inWindow])],
        [HTTPResponse(status: 404, body: Data(feedURL.absoluteString.utf8))], [HTTPResponse(status: 200, body: Data("<html>\(feedURL)</html>".utf8))],
    ]
    for responses in feeds {
        let source = makeSource(await serve(responses), log, retention: window)
        _ = try? await source.events(in: everything)
        _ = try? await source.checkForChanges()
    }
    let kind = ICalSubscriptionKind(transport: await serve([ics([boardGames])]), diagnostics: log)
    _ = try await kind.authorize(using: StubInteraction(["link": feedURL.absoluteString]), credentials: InMemoryCredentialStore())
    #expect(log.events.count > 10)

    let ring = RingBufferDiagnosticLog()
    log.events.forEach(ring.record)
    let everythingSaid = log.transcript + "\n" + ring.render(includePrivate: true) + "\n\(log.events)"
    let secrets = [privatePath, feedURL.absoluteString, "www.example.test/events", "/events/ical", "ical/42"] + titles
    for secret in secrets { #expect(!everythingSaid.contains(secret), "leaked \(secret)") }
    for event in log.events {
        for field in event.fields {
            if case .string = field.value { #expect(field.isPrivate || ["reason"].contains(field.name), "\(event.name).\(field.name)") }
        }
    }
}
