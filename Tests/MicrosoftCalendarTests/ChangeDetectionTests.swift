import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import MicrosoftCalendar

private let deltaBase = "https://graph.microsoft.com/v1.0/me/calendars/cal1/calendarView/delta"

private func page(_ items: [[String: Any]] = [], delta token: String? = nil, next: String? = nil) -> HTTPResponse {
    var json: [String: Any] = ["value": items]
    if let token { json["@odata.deltaLink"] = "\(deltaBase)?$deltatoken=\(token)" }
    if let next { json["@odata.nextLink"] = next }
    return .json(json)
}

/// The delta position stored for calendar `cal1`.
private func storedDelta(_ sync: InMemorySyncStateStore) async throws -> DeltaState {
    let stored = try #require(await sync.token(for: "c1", scope: "cal1"))
    return try #require(DeltaState(stored))
}

private func oneCalendar(sync: InMemorySyncStateStore = InMemorySyncStateStore()) async throws -> SourceHarness {
    try await SourceHarness(calendars: calendarsJSON(["cal1"]), sync: sync)
}

@Test func theFirstCheckTakesABaselineAndReportsNothing() async throws {
    let h = try await oneCalendar()
    await h.transport.route("calendarView/delta", [page(delta: "t0")])
    #expect(try await h.source.checkForChanges() == nil)

    let request = try #require(await h.transport.requests(matching: "calendarView/delta").first)
    let url = request.url.absoluteString
    #expect(url.contains("startDateTime=") && url.contains("endDateTime="))
    #expect(request.headers["Prefer"]?.contains("odata.maxpagesize=200") == true)
    let state = try await storedDelta(h.sync)
    #expect(state.baselineDate == h.now.date && state.link.absoluteString.hasSuffix("$deltatoken=t0"))
    #expect(await h.sync.token(for: "c1", scope: MicrosoftCalendarSource.calendarSetScope) == "cal1")
}

@Test func theBaselineWindowIsThirtyDaysBackAndAYearAhead() async throws {
    let h = try await oneCalendar()
    await h.transport.route("calendarView/delta", [page(delta: "t0")])
    _ = try await h.source.checkForChanges()
    let request = try #require(await h.transport.requests(matching: "calendarView/delta").first)
    let items = URLComponents(url: request.url, resolvingAgainstBaseURL: false)!.queryItems!
    func value(_ name: String) -> String { items.first { $0.name == name }!.value! }
    #expect(value("startDateTime") == GraphTime.instantText(h.now.date.addingTimeInterval(-30 * 86_400)))
    #expect(value("endDateTime") == GraphTime.instantText(h.now.date.addingTimeInterval(365 * 86_400)))
}

@Test func aBaselineWalksEveryPageToTheLink() async throws {
    let h = try await oneCalendar()
    await h.transport.route("calendarView/delta", [page([graphEvent(id: "a")], next: "\(deltaBase)?$skiptoken=2"), page(delta: "t9")])
    #expect(try await h.source.checkForChanges() == nil)
    let state = try await storedDelta(h.sync)
    #expect(state.link.absoluteString.hasSuffix("$deltatoken=t9"))
}

@Test func aPollWithNothingNewReportsNothingAndKeepsTheBaselineDate() async throws {
    let h = try await oneCalendar()
    await h.transport.route("calendarView/delta", [page(delta: "t0")])
    _ = try await h.source.checkForChanges()
    let baselined = h.now.date
    h.now.advance(60)
    await h.transport.route("deltatoken=t0", [page(delta: "t1")])
    #expect(try await h.source.checkForChanges() == nil)
    let state = try await storedDelta(h.sync)
    #expect(state.baselineDate == baselined && state.link.absoluteString.hasSuffix("$deltatoken=t1"))
}

@Test func aPollWithChangesReportsTheCalendarIncludingRemovals() async throws {
    let h = try await oneCalendar()
    await h.transport.route("calendarView/delta", [page(delta: "t0")])
    _ = try await h.source.checkForChanges()
    await h.transport.route("deltatoken=t0", [page([["id": "x", "@removed": ["reason": "deleted"]]], delta: "t1")])
    #expect(try await h.source.checkForChanges() == .eventsChanged(calendarIDs: ["cal1"]))
    await h.transport.route("deltatoken=t1", [page([["id": 7]], delta: "t2")])   // an item that fails to parse is still a change
    #expect(try await h.source.checkForChanges() == .eventsChanged(calendarIDs: ["cal1"]))
}

@Test func aPollFollowsPagesAndTakesTheLinkFromTheLastOne() async throws {
    let h = try await oneCalendar()
    await h.transport.route("calendarView/delta", [page(delta: "t0")])
    _ = try await h.source.checkForChanges()
    await h.transport.route("deltatoken=t0", [page([graphEvent(id: "a")], next: "\(deltaBase)?$skiptoken=p2")])
    await h.transport.route("skiptoken=p2", [page(delta: "t1")])
    #expect(try await h.source.checkForChanges() == .eventsChanged(calendarIDs: ["cal1"]))
    let state = try await storedDelta(h.sync)
    #expect(state.link.absoluteString.hasSuffix("$deltatoken=t1"))
}

@Test func aLinkGraphNoLongerAcceptsTakesANewBaselineAndReportsAChange() async throws {
    let h = try await oneCalendar()
    await h.transport.route("calendarView/delta", [page(delta: "t0")])
    _ = try await h.source.checkForChanges()
    await h.transport.route("deltatoken=t0", [graphError("syncStateNotFound", status: 410)])
    await h.transport.route("calendarView/delta?startDateTime", [page(delta: "fresh")])
    #expect(try await h.source.checkForChanges() == .eventsChanged(calendarIDs: ["cal1"]))
    let state = try await storedDelta(h.sync)
    #expect(state.link.absoluteString.hasSuffix("$deltatoken=fresh"))
}

@Test func aBadRequestThatMeansResyncIsTreatedAsGone() async throws {
    let h = try await oneCalendar()
    await h.transport.route("calendarView/delta", [page(delta: "t0")])
    _ = try await h.source.checkForChanges()
    await h.transport.route("deltatoken=t0", [graphError("ResyncRequired", status: 400)])
    await h.transport.route("calendarView/delta?startDateTime", [page(delta: "fresh")])
    #expect(try await h.source.checkForChanges() == .eventsChanged(calendarIDs: ["cal1"]))
}

@Test func aWindowOlderThanTwoWeeksIsRenewedAfterThePollAndReportsAChange() async throws {
    let h = try await oneCalendar()
    await h.transport.route("calendarView/delta", [page(delta: "t0")])
    _ = try await h.source.checkForChanges()
    h.now.advance(13 * 86_400)
    await h.transport.route("deltatoken=t0", [page(delta: "t1")])
    #expect(try await h.source.checkForChanges() == nil)                       // 13 days: not yet
    h.now.advance(2 * 86_400)
    await h.transport.route("deltatoken=t1", [page(delta: "t2")])
    await h.transport.route("calendarView/delta?startDateTime", [page(delta: "renewed")])
    #expect(try await h.source.checkForChanges() == .eventsChanged(calendarIDs: ["cal1"]))
    let state = try await storedDelta(h.sync)
    #expect(state.baselineDate == h.now.date && state.link.absoluteString.hasSuffix("$deltatoken=renewed"))
    // The poll ran before the renewal, so nothing done in between is lost.
    let order = await h.transport.requests.map { $0.url.absoluteString }.filter { $0.contains("calendarView/delta") }
    #expect(order.suffix(2).first?.contains("deltatoken=t1") == true && order.last?.contains("startDateTime") == true)
}

@Test func aChangedCalendarSetReportsCalendarsChangedAndDropsRemovedTokens() async throws {
    let h = try await SourceHarness(calendars: calendarsJSON(["cal1", "cal2"]))
    await h.transport.route("calendarView/delta", [page(delta: "t0")])
    _ = try await h.source.checkForChanges()
    #expect(await h.sync.token(for: "c1", scope: "cal2") != nil)
    await h.transport.route("me/calendars?", [.json(calendarsJSON(["cal1"]))])
    await h.transport.route("deltatoken=t0", [page(delta: "t1")])
    #expect(try await h.source.checkForChanges() == .calendarsChanged)
    #expect(await h.sync.token(for: "c1", scope: "cal2") == nil)
}

@Test func aRemovedOrForbiddenCalendarIsSkipped() async throws {
    let h = try await oneCalendar()
    await h.transport.route("calendarView/delta", [graphError("ErrorItemNotFound", status: 404)])
    #expect(try await h.source.checkForChanges() == nil)
    await h.transport.route("calendarView/delta", [graphError("ErrorAccessDenied", status: 403)])
    #expect(try await h.source.checkForChanges() == nil)
}

@Test func theDeltaPositionSurvivesARelaunch() async throws {
    let sync = InMemorySyncStateStore()
    let first = try await oneCalendar(sync: sync)
    await first.transport.route("calendarView/delta", [page(delta: "t0")])
    _ = try await first.source.checkForChanges()

    let second = try await oneCalendar(sync: sync)
    await second.transport.route("deltatoken=t0", [page([graphEvent(id: "a")], delta: "t1")])
    #expect(try await second.source.checkForChanges() == .eventsChanged(calendarIDs: ["cal1"]))
}

@Test func aStoredValueThatIsNotADeltaStateIsBaselinedAgain() async throws {
    let sync = InMemorySyncStateStore()
    await sync.setToken("garbage", for: "c1", scope: "cal1")
    let h = try await oneCalendar(sync: sync)
    await h.transport.route("calendarView/delta", [page(delta: "t0")])
    #expect(try await h.source.checkForChanges() == nil)
    _ = try await storedDelta(sync)
}

@Test func deltaStateRoundTrips() throws {
    let state = DeltaState(baselineDate: Date(timeIntervalSince1970: 1_790_000_000), link: URL(string: "https://graph.microsoft.com/v1.0/x?$deltatoken=a|b")!)
    #expect(DeltaState(state.stored) == state)   // a `|` inside the link survives: only the first one splits
    #expect(DeltaState("no-bar") == nil && DeltaState("abc|https://x") == nil)
}
