import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import CalDAVCalendar

private func addCollections(_ server: FakeCalDAVServer) async {
    await server.configure {
        $0.collections["tasks"] = .init(displayName: "Reminders", components: ["VTODO"])
        $0.collections["shared"] = .init(displayName: "Shared", color: nil, privileges: ["read"], timeZoneID: nil)
        $0.collections["holidays"] = .init(displayName: "Holidays", privileges: ["read"], subscribed: true)
    }
}

@Test func listsEventCalendarsOnly() async throws {
    let h = CalDAVHarness()
    await addCollections(h.server)
    let source = try await h.source()
    let calendars = try await source.calendars()
    #expect(calendars.map(\.id) == ["holidays", "home", "shared"])
    let home = try #require(calendars.first { $0.id == "home" })
    #expect(home.title == "Home" && home.colorHex == "#1BADF8")
    #expect(home.permissions.canEdit && home.permissions.canViewDetails)
    #expect(home.isDefault == true)
    #expect(home.timeZone?.identifier == "America/Los_Angeles")
    #expect(home.service == .calDAV && home.provider == .iCloud && home.kind == .standard)
    #expect(home.accountName == "me@icloud.test")
    #expect(home.supportedAvailabilities == [.busy, .free])
    let shared = try #require(calendars.first { $0.id == "shared" })
    #expect(!shared.permissions.canEdit && shared.isDefault == false && shared.colorHex == nil)
    #expect(shared.timeZone == harnessDefaultZone)
    let holidays = try #require(calendars.first { $0.id == "holidays" })
    #expect(holidays.kind == .subscribed && !holidays.permissions.canEdit)
    for calendar in calendars {
        #expect(ProvidedFieldsConformance.violations(calendar: calendar, capabilities: source.capabilities) == [])
    }
}

@Test func defaultIsNilWhenTheServerDoesNotSay() async throws {
    let h = CalDAVHarness()
    await h.server.configure { $0.defaultCalendar = nil }
    #expect(try await h.source().calendars().allSatisfy { $0.isDefault == nil })
}

@Test func readsEventsFromEveryVisibleCalendar() async throws {
    let h = CalDAVHarness()
    await addCollections(h.server)
    await h.server.store("home", "single.ics", singleICS())
    await h.server.store("shared", "weekly.ics", weeklyICS())
    let source = try await h.source()
    let events = try await source.events(in: september)
    #expect(events.map(\.title) == ["Team sync", "Dentist", "Team sync (moved)", "Team sync", "Team sync"])
    let dentist = try #require(events.first { $0.title == "Dentist" })
    #expect(dentist.eventID == "single.ics" && dentist.calendarID == "home" && dentist.series == .notRecurring)
    let etag = await h.server.etag("home", "single.ics")
    #expect(dentist.version == etag)
    #expect(dentist.sourceID == "icloud-c1")
    let moved = try #require(events.first { $0.title == "Team sync (moved)" })
    #expect(moved.calendarID == "shared" && moved.series == .occurrence(seriesID: "weekly.ics", originalStart: pt(2026, 9, 15)))
    #expect(moved.participation == .invited(.needsAction))
    for event in events {
        #expect(ProvidedFieldsConformance.violations(event: event, capabilities: source.capabilities) == [])
        #expect(AllDayConformance.violations(event) == [])
    }
    let query = try #require(await h.server.requests("REPORT").first)
    #expect(String(decoding: query.body ?? Data(), as: UTF8.self).contains("start=\"20260901T070000Z\""))
}

@Test func withoutAddressesParticipationIsNotDeclared() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS())
    let source = try await h.source(userAddresses: [])
    #expect(!source.capabilities.providedFields.contains(.participation))
    #expect(!source.capabilities.canRespondToInvite)
    let events = try await source.events(in: september)
    #expect(events.allSatisfy { $0.participation == nil && $0.attendees.allSatisfy { !$0.isSelf } })
}

@Test func skipsACalendarThatDisappearedAndUnreadableResources() async throws {
    let h = CalDAVHarness()
    await addCollections(h.server)
    await h.server.store("home", "single.ics", singleICS())
    await h.server.store("home", "broken.ics", "this is not iCalendar")
    await h.server.store("shared", "weekly.ics", weeklyICS())
    let source = try await h.source()
    _ = try await source.calendars()
    await h.server.fail("REPORT", pathContains: "/shared/", status: 404)
    #expect(try await source.events(in: september).map(\.title) == ["Dentist"])
}

@Test func freeBusyOnlyCalendarsAreNotQueried() async throws {
    let h = CalDAVHarness()
    await h.server.configure { $0.collections["busy"] = .init(displayName: "Busy", privileges: ["read-free-busy"]) }
    let source = try await h.source()
    let busy = try #require(try await source.calendars().first { $0.id == "busy" })
    #expect(!busy.permissions.canViewDetails)
    _ = try await source.events(in: september)
    #expect(await h.server.requests("REPORT").allSatisfy { !$0.url.path.contains("/busy") })
}

@Test func wrongStoredPasswordIsAuthExpired() async throws {
    let h = CalDAVHarness()
    let source = try await h.source()
    try await h.credentials.setSecrets(["username": "me@icloud.test", "password": "revoked"], for: "c1")
    await #expect(throws: SourceError.authExpired) { try await source.calendars() }
}

@Test func capabilitiesFollowAutoSchedule() async throws {
    let h = CalDAVHarness()
    let scheduling = try await h.source(autoSchedule: true).capabilities
    #expect(scheduling.canEditAttendees && scheduling.canRespondToInvite && scheduling.writableFields.contains(.attendees))
    let plain = try await h.source(autoSchedule: false).capabilities
    #expect(!plain.canEditAttendees && !plain.canRespondToInvite && !plain.writableFields.contains(.attendees))
    for caps in [scheduling, plain] {
        #expect(caps.canEditAttendees == caps.writableFields.contains(.attendees))
        #expect(caps.canWrite && !caps.controlsNotifications && caps.syncKind == .token)
        #expect(caps.recurrenceScopes == Set(RecurrenceScope.allCases))
        #expect(!caps.writableFields.contains(.conference))
    }
}

@Test func seriesReturnsTheRulesOrNotFound() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS())
    await h.server.store("home", "single.ics", singleICS())
    let source = try await h.source()
    let series = try await source.series(id: "weekly.ics", calendarID: "home")
    #expect(series.seriesID == "weekly.ics" && series.start == pt(2026, 9, 1) && series.timeZone == pacific)
    #expect(series.recurrence.rules.first?.frequency == .weekly)
    #expect(series.recurrence.excludedDates == [pt(2026, 9, 8)])
    await #expect(throws: SourceError.notFound) { try await source.series(id: "single.ics", calendarID: "home") }
    await #expect(throws: SourceError.notFound) { try await source.series(id: "nope.ics", calendarID: "home") }
    #expect(ProvidedFieldsConformance.violations(source: source) == [])
}
