import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import ICalSubscription

// TestNow starts at 2027-01-15 00:00 Pacific, so the window is Jan 14 00:00 to Jan 22 00:00 Pacific.
private let window = RetentionWindow(daysBack: 1, daysAhead: 7)
private let everything = DateInterval(start: pt(2026, 8, 1, 0), end: pt(2027, 12, 1, 0))

private func oneOff(_ title: String, _ start: String, _ end: String) -> [String] {
    vevent(uid: "\(title.replacingOccurrences(of: " ", with: "-"))@example.test", title: title, start: start, end: end)
}

private func allDay(_ title: String, _ first: String, _ next: String) -> [String] {
    ["BEGIN:VEVENT", "UID:\(title.replacingOccurrences(of: " ", with: "-"))@example.test", "DTSTAMP:20260901T000000Z", "SUMMARY:\(title)",
     "DTSTART;VALUE=DATE:\(first)", "DTEND;VALUE=DATE:\(next)", "END:VEVENT"]
}

private let farPast = oneOff("Far past", "20260901T100000", "20260901T110000")
private let farFuture = oneOff("Far future", "20270301T100000", "20270301T110000")
private let inWindow = oneOff("In window", "20270118T100000", "20270118T110000")
private let comingUp = oneOff("Coming up", "20270125T100000", "20270125T110000")
private let oldWeekly = vevent(uid: "walk@example.test", title: "Old weekly", start: "20260901T100000", end: "20260901T110000",
                               extra: ["RRULE:FREQ=WEEKLY;BYDAY=TU"])

private func makeSource(
    _ transport: FakeTransport, now: TestNow, retention: RetentionWindow? = window
) -> ICalSubscriptionSource {
    ICalSubscriptionSource(
        connection: Connection(kindID: "icalsub", connectionID: "c1", displayName: "x (www.example.test)", config: ["host": "www.example.test"]),
        link: { feedURL }, transport: transport, monitor: ChangeMonitor(interval: .seconds(900), sleep: { _ in }),
        maxAge: 900, now: now.provider, defaultZone: pacific, retention: retention)
}

private func serve(_ transport: FakeTransport, _ responses: [HTTPResponse]) async { await transport.route(privatePath, responses) }

private func ics(_ events: [[String]], etag: String? = nil) -> HTTPResponse {
    HTTPResponse(status: 200, headers: etag.map { ["ETag": $0] } ?? [:], body: Data(feedICS(events).utf8))
}

private func titles(_ source: ICalSubscriptionSource) async throws -> Set<String> {
    Set(try await source.events(in: everything).map(\.title))
}

@Test func theWindowDropsFarOneOffsAndKeepsInWindowAndRecurringEvents() async throws {
    let transport = FakeTransport()
    await serve(transport, [ics([farPast, farFuture, inWindow, oldWeekly])])
    let found = try await titles(makeSource(transport, now: TestNow()))
    #expect(found == ["In window", "Old weekly"])
}

@Test func allDayEventsAtTheEdgesOfTheWindow() async throws {
    let transport = FakeTransport()
    await serve(transport, [ics([
        allDay("Ends as the window starts", "20270113", "20270114"), allDay("Last day before the window", "20270114", "20270115"),
        allDay("Last day inside", "20270121", "20270122"), allDay("Starts as the window ends", "20270122", "20270123"),
    ])])
    #expect(try await titles(makeSource(transport, now: TestNow())) == ["Last day before the window", "Last day inside"])
}

@Test func noWindowKeepsEverything() async throws {
    let transport = FakeTransport()
    await serve(transport, [ics([farPast, farFuture, inWindow])])
    #expect(try await titles(makeSource(transport, now: TestNow(), retention: nil)) == ["Far past", "Far future", "In window"])
}

@Test func anEditOutsideTheWindowIsNotAChangeButOneInsideIs() async throws {
    let editedFar = oneOff("Far future renamed", "20270301T100000", "20270301T110000")
    let editedIn = oneOff("In window renamed", "20270118T100000", "20270118T110000")
    let transport = FakeTransport()
    await serve(transport, [ics([farFuture, inWindow]), ics([editedFar, inWindow]), ics([editedFar, inWindow]), ics([editedFar, editedIn])])
    let source = makeSource(transport, now: TestNow())
    #expect(try await source.checkForChanges() == nil)
    #expect(try await source.checkForChanges() == nil)
    #expect(try await source.checkForChanges() == nil)
    #expect(try await source.checkForChanges() == .eventsChanged(calendarIDs: ["feed"]))
    #expect(try await titles(source) == ["In window renamed"])
}

@Test func anEventThatComesIntoTheWindowAsTimePassesIsReportedAndReadable() async throws {
    let now = TestNow()
    let transport = FakeTransport()
    await serve(transport, [ics([comingUp, inWindow])])
    let source = makeSource(transport, now: now)
    #expect(try await source.checkForChanges() == nil)
    #expect(try await titles(source) == ["In window"])
    now.advance(5 * 86_400)
    #expect(try await source.checkForChanges() == .eventsChanged(calendarIDs: ["feed"]))
    #expect(try await titles(source).contains("Coming up"))
}

@Test func aNotModifiedAnswerStillBringsAnEventIntoTheWindow() async throws {
    let now = TestNow()
    let transport = FakeTransport()
    await serve(transport, [ics([comingUp], etag: "\"v1\""), HTTPResponse(status: 304)])
    let source = makeSource(transport, now: now)
    #expect(try await source.checkForChanges() == nil)
    now.advance(5 * 86_400)
    #expect(try await source.checkForChanges() == .eventsChanged(calendarIDs: ["feed"]))
    let second = try #require(await transport.requests(matching: privatePath).last)
    #expect(second.headers["If-None-Match"] == "\"v1\"")
    #expect(try await titles(source).contains("Coming up"))
}

@Test func anEditOutsideTheWindowStillUpdatesWhatIsKeptForLater() async throws {
    // The edited far event comes into the window later; the 304 must re-read the edited version, not the first one.
    let now = TestNow()
    let edited = oneOff("Coming up edited", "20270125T100000", "20270125T110000")
    let transport = FakeTransport()
    await serve(transport, [ics([comingUp], etag: "\"v1\""), ics([edited], etag: "\"v2\""), HTTPResponse(status: 304)])
    let source = makeSource(transport, now: now)
    #expect(try await source.checkForChanges() == nil)
    #expect(try await source.checkForChanges() == nil)
    now.advance(5 * 86_400)
    #expect(try await source.checkForChanges() == .eventsChanged(calendarIDs: ["feed"]))
    #expect(try await titles(source) == ["Coming up edited"])
}

@Test func aFeedWithEverythingOutsideTheWindowIsAValidSignInAndTheKindPassesTheWindowOn() async throws {
    let transport = FakeTransport()
    await serve(transport, [ics([farPast, farFuture])])
    let store = InMemoryCredentialStore()
    let kind = ICalSubscriptionKind(transport: transport, now: TestNow().provider, sleep: { _ in }, defaultZone: pacific, retention: window)
    let connection = try await kind.authorize(using: StubInteraction(["link": feedURL.absoluteString]), credentials: store)
    #expect(connection.displayName == "My Meetups (www.example.test)")
    let source = try kind.makeSource(for: connection, credentials: store, syncState: InMemorySyncStateStore())
    #expect(try await source.events(in: everything).isEmpty)
}

@Test func aFeedThatRestampsItselfOnEveryRequestIsNotAChange() async throws {
    func feed(stamp: String, modified: String) -> HTTPResponse {
        let event = vevent(uid: "in@example.test", title: "In window", start: "20270118T100000", end: "20270118T110000",
                           extra: ["LAST-MODIFIED:\(modified)", "CREATED:\(modified)"])
        let text = feedICS([event]).replacingOccurrences(of: "DTSTAMP:20260901T000000Z", with: "DTSTAMP:\(stamp)")
            .replacingOccurrences(of: "PRODID:-//Example//Feed 1.0//EN", with: "PRODID:-//Example//Feed \(stamp)//EN")
        return HTTPResponse(status: 200, body: Data(text.utf8))
    }
    let transport = FakeTransport()
    await serve(transport, [
        feed(stamp: "20261001T000000Z", modified: "20260901T000000Z"), feed(stamp: "20261002T000000Z", modified: "20260901T000000Z"),
        feed(stamp: "20261003T000000Z", modified: "20261003T000000Z"),
    ])
    let source = makeSource(transport, now: TestNow())
    #expect(try await source.checkForChanges() == nil)
    #expect(try await source.checkForChanges() == nil)
    #expect(try await source.checkForChanges() == nil)
}

@Test func aRealEditStillReportsWhenOnlyTheStampsAlsoMoved() async throws {
    let before = vevent(uid: "in@example.test", title: "In window", start: "20270118T100000", end: "20270118T110000")
    let after = vevent(uid: "in@example.test", title: "In window", start: "20270118T110000", end: "20270118T120000",
                       extra: ["LAST-MODIFIED:20261003T000000Z"])
    let transport = FakeTransport()
    await serve(transport, [ics([before]), ics([after])])
    let source = makeSource(transport, now: TestNow())
    #expect(try await source.checkForChanges() == nil)
    #expect(try await source.checkForChanges() == .eventsChanged(calendarIDs: ["feed"]))
}

// MARK: recurring groups are kept only when an occurrence overlaps the window

private func series(_ title: String, start: String, rule: String, extra: [String] = []) -> [String] {
    vevent(uid: "\(title.replacingOccurrences(of: " ", with: "-"))@example.test", title: title, start: start,
           end: String(start.prefix(9)) + "110000", extra: ["RRULE:\(rule)"] + extra)
}

private func override(of title: String, slot: String, start: String, as newTitle: String) -> [String] {
    vevent(uid: "\(title.replacingOccurrences(of: " ", with: "-"))@example.test", title: newTitle, start: start,
           end: String(start.prefix(9)) + "110000", extra: ["RECURRENCE-ID;TZID=America/Los_Angeles:\(slot)"])
}

private func readTitles(_ events: [[String]]) async throws -> Set<String> {
    let transport = FakeTransport()
    await serve(transport, [ics(events)])
    return try await titles(makeSource(transport, now: TestNow()))
}

@Test func aWeeklySeriesWithAnOldStartAndAnOccurrenceInTheWindowIsKept() async throws {
    #expect(try await readTitles([oldWeekly]) == ["Old weekly"])
}

@Test func aSeriesThatEndedBeforeTheWindowIsDropped() async throws {
    let untilDate = series("Ended by date", start: "20260901T100000", rule: "FREQ=WEEKLY;UNTIL=20261201T000000Z")
    let byCount = series("Ended by count", start: "20260901T100000", rule: "FREQ=WEEKLY;COUNT=3")
    #expect(try await readTitles([untilDate, byCount, inWindow]) == ["In window"])
}

@Test func aSeriesThatStartsAfterTheWindowIsDropped() async throws {
    #expect(try await readTitles([series("Later series", start: "20270301T100000", rule: "FREQ=WEEKLY"), inWindow]) == ["In window"])
}

@Test func anUnboundedDailySeriesFromManyYearsAgoIsCheap() async throws {
    // Expansion skips to the window, so a decade of daily instances is not walked.
    let transport = FakeTransport()
    await serve(transport, [ics([series("Daily forever", start: "20170101T100000", rule: "FREQ=DAILY")])])
    let found = try await titles(makeSource(transport, now: TestNow()))
    #expect(found == ["Daily forever"])
}

@Test func aSeriesWhoseOnlyInWindowOccurrenceIsAnOverrideMovedInIsKept() async throws {
    let master = series("Moved in series", start: "20261001T100000", rule: "FREQ=WEEKLY;COUNT=2")
    let moved = override(of: "Moved in series", slot: "20261008T100000", start: "20270118T100000", as: "Moved in")
    // The whole group is kept, so its out-of-window occurrence is readable too.
    #expect(try await readTitles([master + moved]) == ["Moved in", "Moved in series"])
}

@Test func anOverrideMovedOutOfTheWindowDropsTheGroupWhenNothingElseIsInside() async throws {
    // Occurrences Jan 12 and Jan 19; the Jan 19 slot (the only one in the window) is moved to March.
    let master = series("Moved out series", start: "20270112T100000", rule: "FREQ=WEEKLY;COUNT=2")
    let moved = override(of: "Moved out series", slot: "20270119T100000", start: "20270301T100000", as: "Moved out")
    #expect(try await readTitles([master + moved]).isEmpty)
}

@Test func anExdateThatRemovesTheOnlyInWindowOccurrenceDropsTheGroup() async throws {
    let master = series("Excluded series", start: "20270112T100000", rule: "FREQ=WEEKLY;COUNT=2",
                        extra: ["EXDATE;TZID=America/Los_Angeles:20270119T100000"])
    #expect(try await readTitles([master]).isEmpty)
}

@Test func anEditToADroppedSeriesIsNotAChange() async throws {
    let transport = FakeTransport()
    await serve(transport, [
        ics([series("Later series", start: "20270301T100000", rule: "FREQ=WEEKLY"), inWindow]),
        ics([series("Later series renamed", start: "20270301T100000", rule: "FREQ=WEEKLY"), inWindow]),
    ])
    let source = makeSource(transport, now: TestNow())
    #expect(try await source.checkForChanges() == nil)
    #expect(try await source.checkForChanges() == nil)
}

@Test func aSeriesWhoseFirstOccurrenceComesIntoTheWindowIsPickedUp() async throws {
    let later = series("Starts soon", start: "20270125T100000", rule: "FREQ=WEEKLY")
    for useNotModified in [false, true] {
        let now = TestNow()
        let transport = FakeTransport()
        await serve(transport, [ics([later], etag: "\"v1\""), useNotModified ? HTTPResponse(status: 304) : ics([later], etag: "\"v1\"")])
        let source = makeSource(transport, now: now)
        #expect(try await source.checkForChanges() == nil)
        #expect(try await titles(source).isEmpty)
        now.advance(5 * 86_400)
        #expect(try await source.checkForChanges() == .eventsChanged(calendarIDs: ["feed"]), "304: \(useNotModified)")
        #expect(try await titles(source) == ["Starts soon"])
    }
}
