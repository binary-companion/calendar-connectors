import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import MicrosoftCalendar

private actor GraphReadProbe: HTTPTransport {
    let base: FakeTransport
    let delay: Duration
    var active = 0
    var peak = 0
    var started = 0
    init(_ base: FakeTransport, delay: Duration = .milliseconds(30)) { self.base = base; self.delay = delay }
    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        guard request.url.path.contains("/calendarView") else { return try await base.send(request) }
        active += 1
        started += 1
        peak = max(peak, active)
        defer { active -= 1 }
        try await Task.sleep(for: delay)
        return try await base.send(request)
    }
}

@Test func cancellingGraphFetchDoesNotStartQueuedCalendarReads() async throws {
    let ids = (0..<20).map { "cal\($0)" }
    let box = GraphProbeBox()
    let h = try await SourceHarness(calendars: calendarsJSON(ids), wrap: { base in
        let probe = GraphReadProbe(base, delay: .seconds(2))
        box.probe = probe
        return probe
    })
    let probe = try #require(box.probe)
    let fetch = Task { try await h.source.events(in: DateInterval(start: .now, duration: 3600)) }
    for _ in 0..<100 {
        if await probe.started == 4 { break }
        try await Task.sleep(for: .milliseconds(10))
    }
    #expect(await probe.started == 4)
    fetch.cancel()
    await #expect(throws: CancellationError.self) { _ = try await fetch.value }
    #expect(await probe.started == 4)
}

private final class GraphProbeBox: @unchecked Sendable {
    private let lock = NSLock()
    private var value: GraphReadProbe?
    var probe: GraphReadProbe? {
        get { lock.withLock { value } }
        set { lock.withLock { value = newValue } }
    }
}

@Test func twentyGraphCalendarsNeverHaveMoreThanFourActiveEventReads() async throws {
    let ids = (0..<20).map { "cal\($0)" }
    let box = GraphProbeBox()
    let h = try await SourceHarness(calendars: calendarsJSON(ids), wrap: { base in
        let probe = GraphReadProbe(base)
        box.probe = probe
        return probe
    })
    for (index, id) in ids.enumerated() {
        await h.transport.route("\(id)/calendarView", [.json(["value": [graphEvent(id: "e\(index)")]])])
    }
    let events = try await h.source.events(in: DateInterval(start: .now, duration: 3600))
    let probe = try #require(box.probe)
    #expect(await probe.peak <= 4)
    #expect(events.map(\.eventID) == ids.indices.map { "e\($0)" }.sorted())
    #expect(await h.transport.requests.filter { $0.url.path.contains("/calendarView") }.count == 20)
}

@Test func calendarsMapsAndReadsTheMailboxZone() async throws {
    let h = try await SourceHarness()
    let calendars = try await h.source.calendars()
    #expect(calendars.map(\.id) == ["cal1", "cal2"])
    #expect(calendars[0].accountName == "me@x.com" && calendars[0].colorHex == "#112233" && calendars[0].isDefault == true)
    #expect(calendars.allSatisfy { $0.timeZone?.identifier == "America/Los_Angeles" && $0.service == .microsoft })
    let request = try #require(await h.transport.requests(matching: "me/calendars?").first)
    #expect(request.headers["Authorization"] == "Bearer at1")
    #expect(request.url.absoluteString.contains("$top=100"))
}

@Test func aRefusedOrUnknownMailboxZoneIsUTC() async throws {
    let refused = try await SourceHarness()
    await refused.transport.route("me/mailboxSettings/timeZone", [graphError("ErrorAccessDenied", status: 403)])
    #expect(try await refused.source.calendars().allSatisfy { $0.timeZone?.identifier == "UTC" || $0.timeZone?.secondsFromGMT() == 0 })

    let unknown = try await SourceHarness(mailboxZone: "Customized Time Zone")
    #expect(try await unknown.source.calendars().first?.timeZone?.secondsFromGMT() == 0)
}

@Test func aFailingMailboxZoneLookupIsNotTurnedIntoUTC() async throws {
    let h = try await SourceHarness()
    await h.transport.route("me/mailboxSettings/timeZone", [.json([:], status: 500)])
    await #expect(throws: SourceError.server(status: 500)) { _ = try await h.source.calendars() }
}

@Test func calendarsReportsProvidedFieldsItDeclares() async throws {
    let h = try await SourceHarness()
    for calendar in try await h.source.calendars() {
        #expect(ProvidedFieldsConformance.violations(calendar: calendar, capabilities: h.source.capabilities).isEmpty)
    }
    #expect(ProvidedFieldsConformance.violations(source: h.source).isEmpty)
}

@Test func eventsFetchesEveryCalendarWithTheWindowAndReadsInTheAccountZone() async throws {
    let h = try await SourceHarness()
    await h.transport.route("cal1/calendarView", [.json(["value": [graphEvent(id: "b", extra: [
        "start": ["dateTime": "2026-09-25T12:00:00.0000000", "timeZone": "Pacific Standard Time"],
        "end": ["dateTime": "2026-09-25T13:00:00.0000000", "timeZone": "Pacific Standard Time"]])]])])
    await h.transport.route("cal2/calendarView", [.json(["value": [graphEvent(id: "a")]])])
    let interval = DateInterval(start: Date(timeIntervalSince1970: 1_790_000_000), end: Date(timeIntervalSince1970: 1_790_086_400))
    let events = try await h.source.events(in: interval)
    #expect(events.map(\.eventID) == ["a", "b"])
    #expect(events.map(\.calendarID) == ["cal2", "cal1"] && events.allSatisfy { $0.sourceID == "microsoft-c1" })

    let request = try #require(await h.transport.requests(matching: "cal1/calendarView").first)
    let url = request.url.absoluteString
    #expect(url.contains("startDateTime=2026-09-2") && url.contains("endDateTime=2026-09-2") && url.contains("$top=100"))
    #expect(url.contains("$select=id,iCalUId"))
    #expect(request.headers["Prefer"] == "IdType=\"ImmutableId\", outlook.timezone=\"Pacific Standard Time\", outlook.body-content-type=\"text\"")
}

@Test func eventsReusesTheStoredCalendarListAndZone() async throws {
    let h = try await SourceHarness()
    await h.transport.route("calendarView", [.json(["value": []])])
    let interval = DateInterval(start: Date(timeIntervalSince1970: 1_790_000_000), end: Date(timeIntervalSince1970: 1_790_086_400))
    _ = try await h.source.calendars()
    _ = try await h.source.events(in: interval)
    _ = try await h.source.events(in: interval)
    #expect(await h.transport.requests(matching: "me/calendars?").count == 1)
    #expect(await h.transport.requests(matching: "mailboxSettings").count == 1)
}

@Test func eventsSkipsACalendarThatWasRemovedOrLostAccess() async throws {
    let h = try await SourceHarness()
    await h.transport.route("cal1/calendarView", [graphError("ErrorItemNotFound", status: 404)])
    await h.transport.route("cal2/calendarView", [.json(["value": [graphEvent(id: "a")]])])
    let interval = DateInterval(start: Date(timeIntervalSince1970: 1_790_000_000), end: Date(timeIntervalSince1970: 1_790_086_400))
    #expect(try await h.source.events(in: interval).map(\.eventID) == ["a"])
    await h.transport.route("cal2/calendarView", [graphError("ErrorAccessDenied", status: 403)])
    #expect(try await h.source.events(in: interval).isEmpty)
}

@Test func eventsDropsCancelledAndUnreadableItemsAndFollowsPages() async throws {
    let h = try await SourceHarness(calendars: calendarsJSON(["cal1"]))
    await h.transport.route("cal1/calendarView", [
        .json(["value": [graphEvent(id: "a"), graphEvent(id: "gone", extra: ["isCancelled": true])],
               "@odata.nextLink": "https://graph.microsoft.com/v1.0/me/calendars/cal1/calendarView?$skiptoken=2"]),
        .json(["value": [["id": 5], graphEvent(id: "c")]]),
    ])
    let interval = DateInterval(start: Date(timeIntervalSince1970: 1_790_000_000), end: Date(timeIntervalSince1970: 1_790_086_400))
    #expect(try await h.source.events(in: interval).map(\.eventID) == ["a", "c"])
}

@Test func mappedEventsMeetTheProvidedFieldsAndAllDayContracts() async throws {
    let h = try await SourceHarness(calendars: calendarsJSON(["cal1"]))
    await h.transport.route("cal1/calendarView", [.json(["value": [
        graphEvent(id: "timed"),
        graphEvent(id: "day", extra: [
            "isAllDay": true, "start": ["dateTime": "2026-09-25T00:00:00.0000000", "timeZone": "Pacific Standard Time"],
            "end": ["dateTime": "2026-09-26T00:00:00.0000000", "timeZone": "Pacific Standard Time"]]),
    ]])])
    let interval = DateInterval(start: Date(timeIntervalSince1970: 1_790_000_000), end: Date(timeIntervalSince1970: 1_790_086_400))
    for event in try await h.source.events(in: interval) {
        #expect(ProvidedFieldsConformance.violations(event: event, capabilities: h.source.capabilities).isEmpty, "\(event.eventID)")
        #expect(AllDayConformance.violations(event).isEmpty, "\(event.eventID)")
    }
}

@Test func anExpiredTokenSurfacesAsAuthExpired() async throws {
    let h = try await SourceHarness()
    await h.transport.route("me/mailboxSettings/timeZone", [.json([:], status: 401)])
    await #expect(throws: SourceError.authExpired) { _ = try await h.source.calendars() }
}

@Test func capabilitiesDescribeAFullyWritableSyncedSource() async throws {
    let h = try await SourceHarness()
    let caps = h.source.capabilities
    #expect(caps.canWrite && caps.canEditAttendees && caps.canRespondToInvite && caps.syncKind == .token && !caps.supportsPush)
    #expect(caps.writableFields == Set(EventField.allCases) && caps.recurrenceScopes == Set(RecurrenceScope.allCases))
    #expect(!caps.controlsNotifications)
    #expect(h.source is any WritableCalendarSource == caps.canWrite)
    #expect(caps.canEditAttendees == caps.writableFields.contains(.attendees))
}

// MARK: Series

@Test func seriesReadsTheMastersRuleAndAnchor() async throws {
    let h = try await SourceHarness()
    await h.transport.route("cal1/events/master1", [.json(graphEvent(id: "master1", extra: [
        "type": "seriesMaster",
        "recurrence": ["pattern": ["type": "weekly", "interval": 1, "daysOfWeek": ["friday"], "firstDayOfWeek": "monday"],
                       "range": ["type": "numbered", "startDate": "2026-09-25", "numberOfOccurrences": 8, "recurrenceTimeZone": "Pacific Standard Time"]]]))])
    let series = try await h.source.series(id: "master1", calendarID: "cal1")
    #expect(series.seriesID == "master1" && series.calendarID == "cal1" && !series.isAllDay)
    #expect(series.start == Date(timeIntervalSince1970: 1_790_355_600) && series.timeZone.identifier == "America/Los_Angeles")
    #expect(series.recurrence.rules == [RecurrenceRule(frequency: .weekly, weekdays: [.init(.friday)], end: .count(8))])
    #expect(series.recurrence.excludedDates == nil && series.recurrence.extraDates == nil && series.recurrence.unparsed.isEmpty)
}

@Test func seriesRejectsAnEventThatIsNotASeriesMaster() async throws {
    let h = try await SourceHarness()
    await h.transport.route("cal1/events/single", [.json(graphEvent(id: "single"))])
    await h.transport.route("cal1/events/missing", [graphError("ErrorItemNotFound", status: 404)])
    await #expect(throws: SourceError.notFound) { _ = try await h.source.series(id: "single", calendarID: "cal1") }
    await #expect(throws: SourceError.notFound) { _ = try await h.source.series(id: "missing", calendarID: "cal1") }
}

@Test func seriesKeepsAnUnknownPatternAsAnUnparsedLine() async throws {
    let h = try await SourceHarness()
    await h.transport.route("cal1/events/m", [.json(graphEvent(id: "m", extra: [
        "type": "seriesMaster", "recurrence": ["pattern": ["type": "lunar"], "range": ["type": "noEnd", "startDate": "2026-09-25"]]]))])
    let series = try await h.source.series(id: "m", calendarID: "cal1")
    #expect(series.recurrence.rules.isEmpty && series.recurrence.unparsed == ["X-MS-RECURRENCE:lunar"])
}
