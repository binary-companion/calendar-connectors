import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import CalDAVCalendar

@Test func firstCheckIsABaselineAndAQuietOneIsNil() async throws {
    let h = CalDAVHarness()
    let source = try await h.source()
    #expect(try await source.checkForChanges() == nil)
    #expect(try await source.checkForChanges() == nil)
    #expect(await h.server.requests("REPORT").isEmpty)   // unchanged tokens need no sync-collection
}

@Test func anotherClientsEditAndRemovalAreReported() async throws {
    let h = CalDAVHarness()
    let source = try await h.source()
    _ = try await source.checkForChanges()
    await h.server.store("home", "single.ics", singleICS())
    #expect(try await source.checkForChanges() == .eventsChanged(calendarIDs: ["home"]))
    #expect(try await source.checkForChanges() == nil)
    await h.server.remove("home", "single.ics")
    #expect(try await source.checkForChanges() == .eventsChanged(calendarIDs: ["home"]))
}

@Test func aNewCalendarIsCalendarsChanged() async throws {
    let h = CalDAVHarness()
    let source = try await h.source()
    _ = try await source.checkForChanges()
    await h.server.configure { $0.collections["work"] = .init(displayName: "Work") }
    #expect(try await source.checkForChanges() == .calendarsChanged)
    #expect(try await source.checkForChanges() == nil)
    await h.server.configure { $0.collections["work"]?.displayName = "Work (renamed)" }
    #expect(try await source.checkForChanges() == .calendarsChanged)
}

@Test func anExpiredTokenRebaselinesAndReportsTheCalendar() async throws {
    let h = CalDAVHarness()
    let source = try await h.source()
    _ = try await source.checkForChanges()
    await h.server.expireSyncTokens()
    await h.server.store("home", "single.ics", singleICS())
    #expect(try await source.checkForChanges() == .eventsChanged(calendarIDs: ["home"]))
    #expect(try await source.checkForChanges() == nil)
}

@Test func aServerWithoutSyncTokensFallsBackToTheCtag() async throws {
    let h = CalDAVHarness()
    await h.server.configure { $0.supportsSync = false }
    let source = try await h.source()
    _ = try await source.checkForChanges()
    await h.server.store("home", "single.ics", singleICS())
    #expect(try await source.checkForChanges() == .eventsChanged(calendarIDs: ["home"]))
    #expect(await h.server.requests("REPORT").isEmpty)
}

@Test func authExpiredEndsTheStream() async throws {
    let h = CalDAVHarness()
    let source = try await h.source()
    try await h.credentials.setSecrets(["username": "me@icloud.test", "password": "revoked"], for: "c1")
    var received: [CalendarChange] = []
    for await change in source.changes() { received.append(change) }
    #expect(received == [.sourceFailed(.authExpired)])
}

@Test func aFailureOnOneCalendarDoesNotSwallowAnothersChange() async throws {
    let h = CalDAVHarness()
    await h.server.configure { $0.collections["work"] = .init(displayName: "Work") }
    let source = try await h.source()
    _ = try await source.checkForChanges()
    await h.server.store("home", "a.ics", singleICS(uid: "a"))
    await h.server.store("work", "b.ics", singleICS(uid: "b"))
    await h.server.fail("REPORT", pathContains: "/work/", status: 500)
    await #expect(throws: SourceError.server(status: 500)) { try await source.checkForChanges() }
    // `home` was found changed before `work` failed; the retry must still report both.
    #expect(try await source.checkForChanges() == .eventsChanged(calendarIDs: ["home", "work"]))
    #expect(try await source.checkForChanges() == nil)
}

@Test func switchingTheDefaultCalendarIsCalendarsChanged() async throws {
    let h = CalDAVHarness()
    await h.server.configure { $0.collections["work"] = .init(displayName: "Work") }
    let source = try await h.source()
    _ = try await source.checkForChanges()
    #expect(try await source.checkForChanges() == nil)
    await h.server.configure { $0.defaultCalendar = "work" }
    #expect(try await source.checkForChanges() == .calendarsChanged)
    #expect(try await source.checkForChanges() == nil)
}
