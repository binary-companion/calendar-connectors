import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import GoogleCalendar

private let me = "calendars/me%40x.com/events"
private let team = "calendars/team%40group.calendar.google.com/events"

private func bootstrapRoutes(_ h: Harness, meToken: String = "m1", teamToken: String = "t1") async {
    await h.transport.route(team, [.json(["items": [], "nextSyncToken": teamToken])])
    await h.transport.route(me, [.json(["items": [], "nextSyncToken": meToken])])
}

@Test func firstCallBootstrapsTokensAndReportsNothing() async throws {
    let h = try await Harness()
    await bootstrapRoutes(h)
    #expect(try await h.source.checkForChanges() == nil)
    #expect(await h.sync.token(for: "c1", scope: "me@x.com") == "m1")
    #expect(await h.sync.token(for: "c1", scope: "team@group.calendar.google.com") == "t1")
    let request = try #require(await h.transport.requests(matching: me).first)
    let url = request.url.absoluteString
    #expect(url.contains("showDeleted=true") && url.contains("maxResults=2500"))
    #expect(!url.contains("timeMin") && !url.contains("syncToken"))
}

@Test func aRetryAfterAPartlyFailedFirstCallStillReportsRealChanges() async throws {
    // A first call bootstrapped calendar "me" then failed before the calendar-set key was written.
    let sync = InMemorySyncStateStore()
    await sync.setToken("m1", for: "c1", scope: "me@x.com")
    let h = try await Harness(calendarList: listJSON(["me@x.com"]), sync: sync)
    await h.transport.route("syncToken=m1", [.json(["items": [["id": "e1"]], "nextSyncToken": "m2"])])
    #expect(try await h.source.checkForChanges() == .eventsChanged(calendarIDs: ["me@x.com"]))
}

@Test func bootstrapWalksEveryPageAndTakesTheLastToken() async throws {
    let h = try await Harness()
    await h.transport.route(team, [.json(["items": [], "nextSyncToken": "t1"])])
    await h.transport.route(me, [.json(["nextPageToken": "page2"])])
    await h.transport.route("pageToken=page2", [.json(["nextSyncToken": "m-final"])])
    #expect(try await h.source.checkForChanges() == nil)
    #expect(await h.sync.token(for: "c1", scope: "me@x.com") == "m-final")
}

@Test func unchangedPollReportsNothingAndAdvancesTheToken() async throws {
    let h = try await Harness()
    await bootstrapRoutes(h)
    _ = try await h.source.checkForChanges()
    await h.transport.route("syncToken=t1", [.json(["items": [], "nextSyncToken": "t2"])])
    await h.transport.route("syncToken=m1", [.json(["items": [], "nextSyncToken": "m2"])])
    #expect(try await h.source.checkForChanges() == nil)
    #expect(await h.sync.token(for: "c1", scope: "me@x.com") == "m2")
    let poll = try #require(await h.transport.requests(matching: "syncToken=m1").first)
    let url = poll.url.absoluteString
    #expect(url.contains("showDeleted=true") && url.contains("maxResults=250"))
    #expect(!url.contains("timeMin") && !url.contains("timeMax"))
}

@Test func changedItemsReportEventsChangedForThatCalendar() async throws {
    let h = try await Harness()
    await bootstrapRoutes(h)
    _ = try await h.source.checkForChanges()
    await h.transport.route("syncToken=t1", [.json(["items": [], "nextSyncToken": "t2"])])
    await h.transport.route("syncToken=m1", [.json(["items": [["id": "e9"]], "nextSyncToken": "m2"])])
    #expect(try await h.source.checkForChanges() == .eventsChanged(calendarIDs: ["me@x.com"]))
}

@Test func pollPagesUntilTheFinalTokenEvenWhenTheFirstPageHasChanges() async throws {
    let h = try await Harness()
    await bootstrapRoutes(h)
    _ = try await h.source.checkForChanges()
    await h.transport.route("syncToken=t1", [.json(["items": [], "nextSyncToken": "t2"])])
    await h.transport.route("syncToken=m1", [.json(["items": [["id": "e1"]], "nextPageToken": "page2"])])
    await h.transport.route("pageToken=page2", [.json(["items": [], "nextSyncToken": "m-final"])])
    #expect(try await h.source.checkForChanges() == .eventsChanged(calendarIDs: ["me@x.com"]))
    #expect(await h.sync.token(for: "c1", scope: "me@x.com") == "m-final")
}

@Test func expiredTokenRebootstrapsAndReportsChanged() async throws {
    let h = try await Harness()
    await bootstrapRoutes(h)
    _ = try await h.source.checkForChanges()
    await h.transport.route("syncToken=t1", [.json(["items": [], "nextSyncToken": "t2"])])
    await h.transport.route(me, [.json(["nextSyncToken": "m-new"])]) // the re-bootstrap (no syncToken in its URL)
    await h.transport.route("syncToken=m1", [.json(["error": ["code": 410]], status: 410)]) // registered last: wins for the poll
    #expect(try await h.source.checkForChanges() == .eventsChanged(calendarIDs: ["me@x.com"]))
    #expect(await h.sync.token(for: "c1", scope: "me@x.com") == "m-new")
}

@Test func anAddedCalendarReportsCalendarsChangedAndIsBaselined() async throws {
    let sync = InMemorySyncStateStore()
    let first = try await Harness(calendarList: listJSON(["me@x.com", "team@group.calendar.google.com"]), sync: sync)
    await bootstrapRoutes(first)
    _ = try await first.source.checkForChanges()

    let second = try await Harness(
        calendarList: listJSON(["me@x.com", "team@group.calendar.google.com", "new@group.calendar.google.com"]), sync: sync)
    await second.transport.route("syncToken=t1", [.json(["items": [], "nextSyncToken": "t2"])])
    await second.transport.route("syncToken=m1", [.json(["items": [], "nextSyncToken": "m2"])])
    await second.transport.route("calendars/new%40group.calendar.google.com/events", [.json(["nextSyncToken": "n1"])])
    #expect(try await second.source.checkForChanges() == .calendarsChanged)
    #expect(await sync.token(for: "c1", scope: "new@group.calendar.google.com") == "n1")
    #expect(await sync.token(for: "c1", scope: "_calendars")?.contains("new@group.calendar.google.com") == true)
}

@Test func aRemovedCalendarReportsCalendarsChangedAndClearsItsToken() async throws {
    let sync = InMemorySyncStateStore()
    let first = try await Harness(calendarList: listJSON(["me@x.com", "team@group.calendar.google.com"]), sync: sync)
    await bootstrapRoutes(first)
    _ = try await first.source.checkForChanges()

    let second = try await Harness(calendarList: listJSON(["me@x.com"]), sync: sync)
    await second.transport.route("syncToken=m1", [.json(["items": [], "nextSyncToken": "m2"])])
    #expect(try await second.source.checkForChanges() == .calendarsChanged)
    #expect(await sync.token(for: "c1", scope: "team@group.calendar.google.com") == nil)
}

@Test func aFailureMidBootstrapStoresNothingAndConvergesOnRetry() async throws {
    let h = try await Harness()
    await h.transport.route(team, [.json(["nextSyncToken": "t1"])])
    await h.transport.route(me, [.json([:], status: 503), .json(["nextSyncToken": "m1"])])
    await #expect(throws: SourceError.server(status: 503)) { try await h.source.checkForChanges() }
    #expect(await h.sync.token(for: "c1", scope: "_calendars") == nil)
    #expect(try await h.source.checkForChanges() == nil)
    #expect(await h.sync.token(for: "c1", scope: "me@x.com") == "m1")
    #expect(await h.sync.token(for: "c1", scope: "_calendars") != nil)
}

@Test func monitorEmitsChangesFromTheSourceStream() async throws {
    let h = try await Harness()
    await bootstrapRoutes(h)
    await h.transport.route("syncToken=t1", [.json(["items": [], "nextSyncToken": "t2"])])
    await h.transport.route("syncToken=m1", [.json(["items": [["id": "x"]], "nextSyncToken": "m2"])])
    // The first check is the baseline (nil); the second finds the change. The harness monitor never sleeps.
    let stream = h.source.changes()
    let first = try await nextChange(of: stream, timeout: .seconds(5))
    #expect(first == .eventsChanged(calendarIDs: ["me@x.com"]))
}

private struct MonitorTimeout: Error {}

/// Awaits the stream's first element, failing the test instead of hanging if none arrives in time.
private func nextChange<S: AsyncSequence & Sendable>(of stream: S, timeout: Duration) async throws -> S.Element? where S.Element: Sendable {
    do {
        return try await withThrowingTaskGroup(of: S.Element?.self) { group in
            group.addTask {
                var iterator = stream.makeAsyncIterator()
                return try await iterator.next()
            }
            group.addTask {
                try await Task.sleep(for: timeout)
                throw MonitorTimeout()
            }
            defer { group.cancelAll() }
            return try await group.next()!
        }
    } catch is MonitorTimeout {
        Issue.record("timed out after \(timeout) waiting for the monitor to emit a change")
        return nil
    }
}

@Test func aPollItemThatCannotBeParsedStillCountsAsAChangeAndAdvancesTheToken() async throws {
    let h = try await Harness()
    await bootstrapRoutes(h)
    _ = try await h.source.checkForChanges()
    await h.transport.route("syncToken=t1", [.json(["items": [], "nextSyncToken": "t2"])])
    await h.transport.route("syncToken=m1", [.json(["items": [["noId": true]], "nextSyncToken": "m2"])])
    #expect(try await h.source.checkForChanges() == .eventsChanged(calendarIDs: ["me@x.com"]))
    #expect(await h.sync.token(for: "c1", scope: "me@x.com") == "m2")
}
