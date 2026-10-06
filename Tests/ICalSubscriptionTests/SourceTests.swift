import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import ICalSubscription

private let berlin = TimeZone(identifier: "Europe/Berlin")!

private func makeSource(_ transport: FakeTransport, now: TestNow = TestNow(), maxAge: TimeInterval = 900) -> ICalSubscriptionSource {
    ICalSubscriptionSource(
        connection: Connection(kindID: "icalsub", connectionID: "c1", displayName: "My Meetups (www.example.test)", config: ["host": "www.example.test"]),
        link: { feedURL }, transport: transport, monitor: ChangeMonitor(interval: .seconds(900), sleep: { _ in }),
        maxAge: maxAge, now: now.provider, defaultZone: berlin)
}

private func ics(_ text: String, etag: String? = nil) -> HTTPResponse {
    HTTPResponse(status: 200, headers: etag.map { ["ETag": $0] } ?? [:], body: Data(text.utf8))
}

@Test func theFeedIsOneReadOnlySubscribedCalendar() async throws {
    let transport = FakeTransport()
    await transport.route(privatePath, [ics(sampleFeed)])
    let source = makeSource(transport)
    let calendars = try await source.calendars()
    let calendar = try #require(calendars.first)
    #expect(calendars.count == 1 && calendar.id == "feed" && calendar.title == "My Meetups")
    #expect(calendar.service == .iCalSubscription && calendar.provider == .subscription && calendar.kind == .subscribed)
    #expect(calendar.permissions.canEdit == false && calendar.permissions.canViewDetails)
    #expect(calendar.timeZone == berlin && calendar.accountName == "My Meetups (www.example.test)")
    #expect(source.id == "icalsub-c1" && !source.capabilities.canWrite && source.capabilities.syncKind == .token)
    #expect(ProvidedFieldsConformance.violations(calendar: calendar, capabilities: source.capabilities).isEmpty)
    #expect(ProvidedFieldsConformance.violations(source: source).isEmpty)
}

@Test func theCalendarNameFallsBackToTheAccountName() async throws {
    let transport = FakeTransport()
    await transport.route(privatePath, [ics(feedICS([boardGames], header: []))])
    let calendar = try #require(try await makeSource(transport).calendars().first)
    #expect(calendar.title == "My Meetups (www.example.test)")
}

@Test func eventsAreExpandedSortedAndConform() async throws {
    let transport = FakeTransport()
    await transport.route(privatePath, [ics(sampleFeed)])
    let source = makeSource(transport)
    let events = try await source.events(in: september)
    #expect(events.count == 6 && events.map(\.start) == events.map(\.start).sorted())
    #expect(Array(events.map(\.title).prefix(3)) == ["Weekly walk", "Weekly walk", "Board games night"])
    #expect(events.allSatisfy { $0.calendarID == "feed" && $0.sourceID == "icalsub-c1" && $0.uidScope == .global })
    for event in events {
        #expect(ProvidedFieldsConformance.violations(event: event, capabilities: source.capabilities).isEmpty, "\(event.title)")
        #expect(AllDayConformance.violations(event).isEmpty, "\(event.title)")
    }
}

@Test func floatingTimesUseTheFeedsZoneElseTheDefault() async throws {
    let floating = ["BEGIN:VEVENT", "UID:float@example.test", "DTSTAMP:20260901T000000Z", "SUMMARY:Floating",
                    "DTSTART:20260910T120000", "DTEND:20260910T130000", "END:VEVENT"]
    let transport = FakeTransport()
    await transport.route(privatePath, [ics(feedICS([floating], header: ["X-WR-TIMEZONE:Asia/Tokyo"]))])
    let tokyo = TimeZone(identifier: "Asia/Tokyo")!
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = tokyo
    let event = try #require(try await makeSource(transport).events(in: september).first)
    #expect(event.start == calendar.date(from: DateComponents(year: 2026, month: 9, day: 10, hour: 12)))

    let plain = FakeTransport()
    await plain.route(privatePath, [ics(feedICS([floating], header: []))])
    let fallback = try #require(try await makeSource(plain).events(in: september).first)
    var berlinCalendar = Calendar(identifier: .gregorian)
    berlinCalendar.timeZone = berlin
    #expect(fallback.start == berlinCalendar.date(from: DateComponents(year: 2026, month: 9, day: 10, hour: 12)))
}

@Test func theParsedFeedIsReusedUntilItIsOlderThanTheMaximumAge() async throws {
    let transport = FakeTransport()
    await transport.route(privatePath, [ics(sampleFeed)])
    let now = TestNow()
    let source = makeSource(transport, now: now)
    _ = try await source.events(in: september)
    _ = try await source.calendars()
    _ = try await source.events(in: september)
    #expect(await transport.requests(matching: privatePath).count == 1)
    now.advance(901)
    _ = try await source.events(in: september)
    #expect(await transport.requests(matching: privatePath).count == 2)
}

@Test func theFirstCheckIsTheBaselineAndLaterChecksReportOnlyRealChanges() async throws {
    let changed = feedICS([boardGames])
    let transport = FakeTransport()
    await transport.route(privatePath, [ics(sampleFeed), ics(sampleFeed), ics(changed)])
    let source = makeSource(transport)
    #expect(try await source.checkForChanges() == nil)
    #expect(try await source.checkForChanges() == nil)
    #expect(try await source.checkForChanges() == .eventsChanged(calendarIDs: ["feed"]))
    #expect(try await source.events(in: september).map(\.title) == ["Board games night"])
}

@Test func aChangeSinceTheEventsWereLoadedIsReportedByTheFirstCheck() async throws {
    let transport = FakeTransport()
    await transport.route(privatePath, [ics(sampleFeed), ics(feedICS([boardGames]))])
    let source = makeSource(transport)
    #expect(try await source.events(in: september).count == 6)
    #expect(try await source.checkForChanges() == .eventsChanged(calendarIDs: ["feed"]))
}

@Test func validatorsLetAnUnchangedFeedAnswer304() async throws {
    let transport = FakeTransport()
    await transport.route(privatePath, [ics(sampleFeed, etag: "\"v1\""), HTTPResponse(status: 304)])
    let source = makeSource(transport)
    #expect(try await source.checkForChanges() == nil)
    #expect(try await source.checkForChanges() == nil)
    let second = try #require(await transport.requests(matching: privatePath).last)
    #expect(second.headers["If-None-Match"] == "\"v1\"")
    #expect(try await source.events(in: september).count == 6)
}

@Test func aRevokedLinkIsAuthExpiredEverywhere() async throws {
    let transport = FakeTransport()
    await transport.route(privatePath, [HTTPResponse(status: 404)])
    let source = makeSource(transport)
    await #expect(throws: SourceError.authExpired) { _ = try await source.events(in: september) }
    await #expect(throws: SourceError.authExpired) { _ = try await source.calendars() }
    await #expect(throws: SourceError.authExpired) { _ = try await source.checkForChanges() }
}

@Test func aPageThatStopsBeingACalendarKeepsTheLastGoodFeed() async throws {
    let transport = FakeTransport()
    await transport.route(privatePath, [ics(sampleFeed), ics("<html>Please sign in</html>")])
    let source = makeSource(transport)
    #expect(try await source.checkForChanges() == nil)
    await #expect(throws: SourceError.invalidResponse("that link did not return a calendar")) { _ = try await source.checkForChanges() }
    #expect(try await source.events(in: september).count == 6)
}

@Test func theChangesStreamReportsAChangeAndStopsOnAuthExpired() async throws {
    let transport = FakeTransport()
    await transport.route(privatePath, [ics(sampleFeed), ics(feedICS([boardGames])), HTTPResponse(status: 404)])
    var iterator = makeSource(transport).changes().makeAsyncIterator()
    #expect(await iterator.next() == .eventsChanged(calendarIDs: ["feed"]))
    #expect(await iterator.next() == .sourceFailed(.authExpired))
    #expect(await iterator.next() == nil)
}
