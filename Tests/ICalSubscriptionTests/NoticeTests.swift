import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import ICalSubscription

private let pacificZone = pacific

private func source(_ transport: FakeTransport, _ log: CollectingDiagnosticLog = CollectingDiagnosticLog()) -> ICalSubscriptionSource {
    ICalSubscriptionSource(
        connection: Connection(kindID: "icalsub", connectionID: "c1", displayName: "x (www.example.test)", config: ["host": "www.example.test"]),
        link: { feedURL }, transport: transport, monitor: ChangeMonitor(interval: .seconds(900), sleep: { _ in }),
        maxAge: 900, now: TestNow().provider, defaultZone: pacificZone, diagnostics: log)
}

private func series(_ name: String, rule: String) -> [String] {
    vevent(uid: "\(name)@example.test", title: name, start: "20260901T100000", end: "20260901T110000", extra: ["RRULE:\(rule)"])
}

private func serve(_ responses: [HTTPResponse]) async -> FakeTransport {
    let transport = FakeTransport()
    await transport.route(privatePath, responses)
    return transport
}

private func ics(_ events: [[String]], etag: String? = nil) -> HTTPResponse {
    HTTPResponse(status: 200, headers: etag.map { ["ETag": $0] } ?? [:], body: Data(feedICS(events).utf8))
}

@Test func aCleanFeedHasNoNotices() async throws {
    let feed = source(await serve([ics([boardGames, weeklyWalk])]))
    #expect(await feed.notices().isEmpty)          // nothing loaded yet
    _ = try await feed.events(in: september)
    #expect(await feed.notices().isEmpty)
}

@Test func eachUnreadableSeriesCountsAndTheNoticeClearsWhenTheFeedIsFixed() async throws {
    let feed = source(await serve([
        ics([series("a", rule: "INTERVAL=2"), series("b", rule: "FREQ=BOGUS"), boardGames]),
        ics([series("a", rule: "FREQ=WEEKLY"), series("b", rule: "FREQ=DAILY"), boardGames]),
    ]))
    _ = try await feed.checkForChanges()
    #expect(await feed.notices() == [SourceNotice(kind: .unreadableRecurrence, count: 2)])
    _ = try await feed.checkForChanges()
    #expect(await feed.notices().isEmpty)
}

@Test func theNoticeSurvivesANotModifiedReParseAndTheLogDedupe() async throws {
    let feed = source(await serve([ics([series("a", rule: "INTERVAL=2")], etag: "\"v1\""), HTTPResponse(status: 304), HTTPResponse(status: 304)]))
    for _ in 0..<3 {
        _ = try await feed.checkForChanges()
        #expect(await feed.notices() == [SourceNotice(kind: .unreadableRecurrence, count: 1)])
    }
}

@Test func anOlderFetchFinishingLateDoesNotMoveTheReportedSetBack() async {
    let state = FeedState()
    let rule = UnreadableRule(uid: "a", isSynthetic: false, ruleHash: "1")
    #expect(await state.newUnreadable([rule], generation: 2) == [rule])
    #expect(await state.newUnreadable([], generation: 1).isEmpty)
    #expect(await state.newUnreadable([rule], generation: 3).isEmpty)
}

@Test func aMadeUpUIDIsNeverLogged() async throws {
    let log = CollectingDiagnosticLog()
    let nameless = vevent(uid: nil, title: "Secret title", start: "20260901T100000", end: "20260901T110000", extra: ["RRULE:INTERVAL=2"])
    _ = try await source(await serve([ics([nameless])]), log).checkForChanges()
    let event = try #require(log.events(named: "rruleUnreadable").first)
    #expect(event.field("uid") == nil && event.field("syntheticUID")?.value == .bool(true))
    #expect(!log.transcript.contains("feed-") && !log.transcript.contains("Secret title"))
    // The sign-in parse path logs through the parser itself.
    let parserLog = CollectingDiagnosticLog()
    _ = try FeedParser.parse(Data(feedICS([nameless]).utf8), diagnostics: parserLog)
    #expect(parserLog.events(named: "rruleUnreadable").first?.field("uid") == nil)
}
