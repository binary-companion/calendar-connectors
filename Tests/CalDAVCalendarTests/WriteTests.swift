import CalendarCore
import CalendarTestSupport
import Foundation
import ICalendar
import Testing
@testable import CalDAVCalendar

private func expectWriteError(_ expected: WriteError, _ body: () async throws -> Void) async {
    do {
        try await body()
        Issue.record("expected \(expected), but nothing was thrown")
    } catch let error as WriteError {
        #expect(error == expected)
    } catch {
        Issue.record("expected \(expected), got \(error)")
    }
}

private func stored(_ h: CalDAVHarness, _ name: String, calendar: String = "home") async throws -> EventResource {
    let body = try #require(await h.server.body(calendar, name))
    return try EventResource(data: Data(body.utf8))
}

private func occurrence(_ source: CalDAVCalendarSource, on day: Int, hour: Int = 10) async throws -> CalendarEvent {
    try #require(try await source.events(in: september).first { $0.originalStart == pt(2026, 9, day, hour) })
}

private let timing = EventTiming(start: pt(2026, 9, 10, 9), end: pt(2026, 9, 10, 10), timeZone: pacific, isAllDay: false)

@Test func passesTheWritableConformanceSuite() async throws {
    for autoSchedule in [true, false] {
        let h = CalDAVHarness()
        let source = try await h.source(autoSchedule: autoSchedule)
        #expect(await WritableSourceConformance.violations(of: source, calendarID: "home", window: september) == [])
    }
}

@Test func createPutsANewResourceAndReturnsItsVersion() async throws {
    let h = CalDAVHarness()
    let source = try await h.source()
    let created = try await source.create(EventDraft(title: "Design review", timing: timing, reminders: [.before(minutes: 10)]), in: "home", notify: .none)
    #expect(created.eventID == "uuid-2.ics" && created.uid == "uuid-1" && created.series == .notRecurring)
    let etag = await h.server.etag("home", "uuid-2.ics")
    #expect(created.version == etag)
    let put = try #require(await h.server.requests("PUT").first)
    #expect(put.headers["If-None-Match"] == "*" && put.headers["Content-Type"]?.hasPrefix("text/calendar") == true)
    let resource = try await stored(h, "uuid-2.ics")
    #expect(resource.calendar.components(named: "VTIMEZONE").first?.property("TZID")?.value == "America/Los_Angeles")
    #expect(created.reminders == [Reminder(trigger: .relative(offset: -600, to: .start), isCalendarDefault: false)])
}

@Test func createWithAKnownUIDIsAlreadyExists() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "single.ics", singleICS(uid: "shared-uid"))
    let source = try await h.source()
    do {
        _ = try await source.create(EventDraft(title: "Copy", timing: timing, uid: "shared-uid"), in: "home", notify: .none)
        Issue.record("expected alreadyExists")
    } catch WriteError.alreadyExists(let existing) {
        #expect(existing.eventID == "single.ics" && existing.title == "Dentist")
    }
    #expect(await h.server.requests("PUT").isEmpty)
}

@Test func createRetriesANameClashOnce() async throws {
    let h = CalDAVHarness()
    let source = try await h.source()
    await h.server.fail("PUT", pathContains: "uuid-2.ics", status: 412)
    let created = try await source.create(EventDraft(title: "x", timing: timing), in: "home", notify: .none)
    #expect(created.eventID == "uuid-3.ics")
}

@Test func createRecurringReturnsTheSeriesMaster() async throws {
    let h = CalDAVHarness()
    let source = try await h.source()
    let series = try await source.create(
        EventDraft(title: "Weekly", timing: timing, recurrence: try RecurrenceRule(rrule: "FREQ=WEEKLY;COUNT=3")), in: "home", notify: .none)
    #expect(series.seriesID == series.eventID && series.originalStart == series.start)
    await expectWriteError(.invalid("this is a recurring series; use .allInSeries or read the occurrence first")) {
        _ = try await source.update(EventRef(series), EventPatch(title: "y"), scope: .thisInstance, notify: .none)
    }
}

@Test func refusedFieldsAndPoliciesWriteNothing() async throws {
    let h = CalDAVHarness()
    let source = try await h.source()
    let guests = [AttendeeDraft(email: "ann@example.test")]
    await expectWriteError(.unsupported(fields: [.attendees])) {
        _ = try await source.create(EventDraft(title: "x", timing: timing, attendees: guests), in: "home", notify: .none)
    }
    await expectWriteError(.unsupported(fields: [.conference])) {
        _ = try await source.create(EventDraft(title: "x", timing: timing, conference: .generate), in: "home", notify: .all)
    }
    await expectWriteError(.unsupported(fields: [.reminders])) {
        _ = try await source.create(EventDraft(title: "x", timing: timing, reminders: [.before(minutes: 5, type: .email(address: nil))]), in: "home", notify: .all)
    }
    let plain = try await h.source(autoSchedule: false)
    await expectWriteError(.unsupported(fields: [.attendees])) {
        _ = try await plain.create(EventDraft(title: "x", timing: timing, attendees: guests), in: "home", notify: .all)
    }
    #expect(await h.server.requests("PUT").isEmpty)
    let invited = try await source.create(EventDraft(title: "Meet", timing: timing, attendees: guests), in: "home", notify: .all)
    #expect(invited.organizer?.isSelf == true && invited.attendees.map(\.email) == ["me@icloud.test", "ann@example.test"])
}

@Test func updateKeepsPropertiesTheModelDoesNotCover() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "single.ics", singleICS())
    let source = try await h.source()
    let event = try #require(try await source.events(in: september).first)
    let renamed = try await source.update(EventRef(event), EventPatch(title: "Dentist (moved)"), scope: .thisInstance, notify: .none)
    #expect(renamed.title == "Dentist (moved)" && renamed.version != event.version)
    let put = try #require(await h.server.requests("PUT").last)
    #expect(put.headers["If-Match"] == event.version)
    #expect(try await stored(h, "single.ics").master?.property("X-APPLE-TRAVEL-ADVISORY-BEHAVIOR")?.value == "AUTOMATIC")
    await expectWriteError(.unsupported(fields: [.reminders])) {
        _ = try await source.update(EventRef(renamed), EventPatch(reminders: .clear), scope: .thisInstance, notify: .none)
    }
}

@Test func thisInstanceAddsThenEditsAnOverride() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    let tuesday = try await occurrence(source, on: 22)
    let edited = try await source.update(EventRef(tuesday), EventPatch(title: "Special"), scope: .thisInstance, notify: .none)
    #expect(edited.eventID == tuesday.eventID && edited.title == "Special")
    #expect(try await stored(h, "weekly.ics").overrides.count == 2)
    let again = try await source.update(EventRef(edited), EventPatch(location: .set("Room 9")), scope: .thisInstance, notify: .none)
    #expect(again.title == "Special" && again.location == "Room 9")
    #expect(try await stored(h, "weekly.ics").overrides.count == 2)
    let titles = try await source.events(in: september).map(\.title)
    #expect(titles == ["Team sync", "Team sync (moved)", "Special", "Team sync"])
}

@Test func allInSeriesTimingShiftsExceptionsWithTheSeries() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    let tuesday = try await occurrence(source, on: 22)
    let later = EventTiming(start: pt(2026, 9, 22, 11), end: pt(2026, 9, 22, 11, 30), timeZone: pacific, isAllDay: false)
    let series = try await source.update(EventRef(tuesday), EventPatch(timing: later), scope: .allInSeries, notify: .none)
    #expect(series.eventID == "weekly.ics" && series.start == pt(2026, 9, 1, 11))
    let starts = try await source.events(in: september).map(\.start)
    #expect(starts == [pt(2026, 9, 1, 11), pt(2026, 9, 16, 14), pt(2026, 9, 22, 11), pt(2026, 9, 29, 11)])
    var noSlot = EventRef(tuesday)
    noSlot.originalStart = nil
    await expectWriteError(.unsupported(fields: [.timing])) {
        _ = try await source.update(noSlot, EventPatch(timing: later), scope: .allInSeries, notify: .none)
    }
    await expectWriteError(.invalid("needs the occurrence's original start")) {
        _ = try await source.update(noSlot, EventPatch(title: "x"), scope: .thisInstance, notify: .none)
    }
}

@Test func aStaleVersionRetriesUnlessTheSameFieldChanged() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    let tuesday = try await occurrence(source, on: 22)
    // Someone else renames another occurrence: the resource's ETag moves, but not the field we touch.
    let other = try await occurrence(source, on: 29)
    _ = try await source.update(EventRef(other), EventPatch(title: "Other change"), scope: .thisInstance, notify: .none)
    var withRoom = tuesday
    withRoom.location = "Room 2"
    let moved = try await source.update(EventRef(tuesday), EventPatch(from: tuesday, to: withRoom), scope: .thisInstance, notify: .none)
    #expect(moved.location == "Room 2" && moved.title == "Team sync")
    // Now the same occurrence's title changes under a stale base.
    let fresh = try await occurrence(source, on: 22)
    _ = try await source.update(EventRef(fresh), EventPatch(title: "Theirs"), scope: .thisInstance, notify: .none)
    var mine = fresh
    mine.title = "Mine"
    await expectWriteError(.conflict(fields: [.title])) {
        _ = try await source.update(EventRef(fresh), EventPatch(from: fresh, to: mine), scope: .thisInstance, notify: .none)
    }
}

@Test func notifyPolicyIsRefusedWhenOthersWouldBeTold() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "meeting.ics", singleICS(uid: "m", title: "Meeting", attendees: [
        "ORGANIZER:mailto:me@icloud.test", "ATTENDEE;PARTSTAT=ACCEPTED:mailto:me@icloud.test", "ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:ann@example.test",
    ]))
    let source = try await h.source()
    let meeting = try #require(try await source.events(in: september).first)
    for policy in [NotifyPolicy.none, .externalOnly] {
        await expectWriteError(.unsupported(fields: [.attendees])) {
            _ = try await source.update(EventRef(meeting), EventPatch(title: "x"), scope: .thisInstance, notify: policy)
        }
        await expectWriteError(.unsupported(fields: [.attendees])) { try await source.delete(EventRef(meeting), scope: .thisInstance, notify: policy) }
    }
    let puts = await h.server.requests("PUT")
    let deletes = await h.server.requests("DELETE")
    #expect(puts.isEmpty && deletes.isEmpty)
    _ = try await source.update(EventRef(meeting), EventPatch(title: "Renamed"), scope: .thisInstance, notify: .all)
}

@Test func deletesByScope() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    try await source.delete(EventRef(try await occurrence(source, on: 22)), scope: .thisInstance, notify: .none)
    #expect(try await source.events(in: september).map(\.start) == [pt(2026, 9, 1), pt(2026, 9, 16, 14), pt(2026, 9, 29)])
    // The moved occurrence's slot is the 15th at 10:00, not its new time.
    try await source.delete(EventRef(try await occurrence(source, on: 15)), scope: .thisAndFollowing, notify: .none)
    #expect(try await source.events(in: september).map(\.start) == [pt(2026, 9, 1)])
    try await source.delete(EventRef(try await occurrence(source, on: 1)), scope: .allInSeries, notify: .none)
    #expect(await h.server.names("home").isEmpty)
    let deletion = try #require(await h.server.requests("DELETE").last)
    #expect(deletion.headers["If-Match"] == nil)
}

@Test func deletingAnOccurrenceRetriesThenReportsAConflict() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    let tuesday = try await occurrence(source, on: 22)
    await h.server.fail("PUT", pathContains: "weekly.ics", status: 412, times: 2)
    try await source.delete(EventRef(tuesday), scope: .thisInstance, notify: .none)
    #expect(await h.server.requests("PUT").count == 3)
    let thursday = try await occurrence(source, on: 29)
    await h.server.fail("PUT", pathContains: "weekly.ics", status: 412, times: 3)
    await expectWriteError(.conflict(fields: [.recurrence])) { try await source.delete(EventRef(thursday), scope: .thisInstance, notify: .none) }
    await expectWriteError(.notFound) { try await source.delete(EventRef(calendarID: "home", eventID: "gone.ics"), scope: .allInSeries, notify: .none) }
}

@Test func respondSetsTheAccountsAnswer() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS())
    let source = try await h.source()
    let tuesday = try await occurrence(source, on: 22)
    #expect(tuesday.participation == .invited(.needsAction))
    let answered = try await source.respond(to: EventRef(tuesday), .accepted, scope: .thisInstance, notify: .all)
    #expect(answered.participation == .invited(.accepted))
    #expect(try await occurrence(source, on: 29).participation == .invited(.needsAction))
    let series = try await source.respond(to: EventRef(tuesday), .declined, scope: .allInSeries, notify: .all)
    #expect(series.participation == .invited(.declined))
    #expect(try await source.events(in: september).allSatisfy { $0.participation == .invited(.declined) })
    await expectWriteError(.unsupported(fields: [.attendees])) { _ = try await source.respond(to: EventRef(tuesday), .accepted, scope: .thisInstance, notify: .none) }
    await expectWriteError(.unsupported(fields: [.attendees])) { _ = try await source.respond(to: EventRef(tuesday), .accepted, scope: .thisAndFollowing, notify: .all) }
    await expectWriteError(.invalid("a response must be accepted, tentative or declined")) {
        _ = try await source.respond(to: EventRef(tuesday), .needsAction, scope: .thisInstance, notify: .all)
    }
    await h.server.store("home", "single.ics", singleICS())
    let mine = try #require(try await source.events(in: september).first { $0.title == "Dentist" })
    await expectWriteError(.unsupported(fields: [.attendees])) { _ = try await source.respond(to: EventRef(mine), .accepted, scope: .thisInstance, notify: .all) }
}

@Test func forbiddenCalendarIsForbidden() async throws {
    let h = CalDAVHarness()
    await h.server.configure { $0.collections["shared"] = .init(displayName: "Shared", privileges: ["read"]) }
    await h.server.store("shared", "single.ics", singleICS())
    let source = try await h.source()
    await expectWriteError(.forbidden(nil)) { _ = try await source.create(EventDraft(title: "x", timing: timing), in: "shared", notify: .none) }
}

// MARK: Review round 1

/// Forwards to the server, but lets `beforeFirstPUT` (another client's edit) happen just before the first PUT, so that
/// PUT is genuinely stale.
private final class InterferingTransport: HTTPTransport, @unchecked Sendable {
    private let server: FakeCalDAVServer
    private let lock = NSLock()
    private var pending: (@Sendable (FakeCalDAVServer) async -> Void)?

    init(_ server: FakeCalDAVServer, beforeFirstPUT: @escaping @Sendable (FakeCalDAVServer) async -> Void) {
        self.server = server
        pending = beforeFirstPUT
    }

    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        if request.method == "PUT", let action = lock.withLock({ () -> (@Sendable (FakeCalDAVServer) async -> Void)? in
            defer { pending = nil }
            return pending
        }) {
            await action(server)
        }
        return try await server.send(request)
    }
}

/// An invite to single occurrences: overrides, no master.
private func overridesOnlyICS(_ slots: [Int] = [22]) -> String {
    let events = slots.flatMap { day -> [String] in
        ["BEGIN:VEVENT", "UID:only-uid", "DTSTAMP:20260901T000000Z", "RECURRENCE-ID;TZID=America/Los_Angeles:202609\(day)T100000",
         "SUMMARY:Guest spot \(day)", "DTSTART;TZID=America/Los_Angeles:202609\(day)T100000",
         "DTEND;TZID=America/Los_Angeles:202609\(day)T103000", "ORGANIZER;CN=Me:mailto:me@icloud.test",
         "ATTENDEE;CN=Me;PARTSTAT=NEEDS-ACTION:mailto:me@icloud.test", "END:VEVENT"]
    }
    return (["BEGIN:VCALENDAR", "VERSION:2.0", "PRODID:-//Apple Inc.//iCloud//EN"] + events + ["END:VCALENDAR"]).joined(separator: "\r\n") + "\r\n"
}

@Test func anOrganizerWhoIsNotTheAccountIsToldEvenWhenNoOtherAttendeeIsListed() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "invite.ics", singleICS(uid: "i", title: "Invite", attendees: [
        "ORGANIZER:mailto:boss@example.test", "ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:me@icloud.test",
    ]))
    let source = try await h.source()
    let invite = try #require(try await source.events(in: september).first)
    for policy in [NotifyPolicy.none, .externalOnly] {
        await expectWriteError(.unsupported(fields: [.attendees])) { try await source.delete(EventRef(invite), scope: .thisInstance, notify: policy) }
        await expectWriteError(.unsupported(fields: [.attendees])) {
            _ = try await source.update(EventRef(invite), EventPatch(title: "x"), scope: .thisInstance, notify: policy)
        }
    }
    let puts = await h.server.requests("PUT")
    let deletes = await h.server.requests("DELETE")
    #expect(puts.isEmpty && deletes.isEmpty)
}

@Test func deletingAnOccurrenceOfASeriesThatBecameASingleEventIsNotFound() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    let tuesday = try await occurrence(source, on: 22)
    await h.server.store("home", "weekly.ics", singleICS(uid: "weekly-uid", title: "Now a one-off"))
    await expectWriteError(.notFound) { try await source.delete(EventRef(tuesday), scope: .thisInstance, notify: .none) }
    await expectWriteError(.notFound) { try await source.delete(EventRef(tuesday), scope: .thisAndFollowing, notify: .none) }
    let deletes = await h.server.requests("DELETE")
    let puts = await h.server.requests("PUT")
    let names = await h.server.names("home")
    #expect(deletes.isEmpty && puts.isEmpty && names == ["weekly.ics"])
}

@Test func aWholeSeriesEditIsJudgedAgainstTheSeriesMaster() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    let moved = try await occurrence(source, on: 15, hour: 10)   // the override: its own title, not the master's
    let tuesday = try await occurrence(source, on: 22)
    _ = try await source.update(EventRef(tuesday), EventPatch(title: "Planning"), scope: .allInSeries, notify: .none)
    var mine = moved
    mine.title = "Planning (mine)"
    await expectWriteError(.conflict(fields: [.title])) {
        _ = try await source.update(EventRef(moved), EventPatch(from: moved, to: mine), scope: .allInSeries, notify: .none)
    }
    #expect(try await stored(h, "weekly.ics").master?.property("SUMMARY")?.value == "Planning")
}

@Test func anOccurrenceInAResourceWithoutAMasterCanBeDeletedUpdatedAndAnswered() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "only.ics", overridesOnlyICS([22, 29]))
    let source = try await h.source()
    let first = try await occurrence(source, on: 22)
    try await source.delete(EventRef(first), scope: .thisInstance, notify: .all)
    #expect(try await source.events(in: september).map(\.start) == [pt(2026, 9, 29, 10)])
    let last = try await occurrence(source, on: 29)
    let renamed = try await source.update(EventRef(last), EventPatch(title: "Renamed"), scope: .allInSeries, notify: .all)
    #expect(renamed.title == "Renamed")
    let answered = try await source.respond(to: EventRef(renamed), .accepted, scope: .allInSeries, notify: .all)
    #expect(answered.participation == .invited(.accepted))
    try await source.delete(EventRef(answered), scope: .thisInstance, notify: .all)
    #expect(await h.server.names("home").isEmpty)
    let deletion = try #require(await h.server.requests("DELETE").last)
    #expect(deletion.headers["If-Match"] != nil)
}

@Test func deleteRerunsTheNotifyCheckOnEveryAttempt() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let transport = InterferingTransport(h.server) { server in
        await server.store("home", "weekly.ics", weeklyICS())   // meanwhile someone invites people
    }
    let source = try await h.source(transport: transport)
    let tuesday = try await occurrence(source, on: 22)
    await expectWriteError(.unsupported(fields: [.attendees])) { try await source.delete(EventRef(tuesday), scope: .thisInstance, notify: .none) }
    let body = try #require(await h.server.body("home", "weekly.ics"))
    #expect(!body.contains("20260922"))
}

@Test func deleteTreatsAResourceGoneOnARetryAsDone() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let transport = InterferingTransport(h.server) { server in await server.remove("home", "weekly.ics") }
    let source = try await h.source(transport: transport)
    let tuesday = try await occurrence(source, on: 22)
    try await source.delete(EventRef(tuesday), scope: .thisInstance, notify: .none)   // the PUT 412s, the re-fetch 404s
    #expect(await h.server.names("home").isEmpty)
}

private func allDayICS(uid: String = "allday-uid", first: String = "20260305", rule: String) -> String {
    ["BEGIN:VCALENDAR", "VERSION:2.0", "PRODID:-//Apple Inc.//iCloud//EN", "BEGIN:VEVENT", "UID:\(uid)", "DTSTAMP:20260301T000000Z",
     "SEQUENCE:0", "SUMMARY:Holiday", "DTSTART;VALUE=DATE:\(first)", "DTEND;VALUE=DATE:\(next(first))", "RRULE:\(rule)", "END:VEVENT", "END:VCALENDAR"]
        .joined(separator: "\r\n") + "\r\n"
}

private func next(_ date: String) -> String { String(Int(date)! + 1) }

private func day(_ zone: TimeZone, _ y: Int, _ m: Int, _ d: Int) -> Date {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = zone
    return calendar.date(from: DateComponents(year: y, month: m, day: d))!
}

@Test func deletingFromTheDayAfterASpringForwardDayKeepsThatDay() async throws {
    let newYork = TimeZone(identifier: "America/New_York")!
    let h = CalDAVHarness()
    await h.server.configure { $0.collections["home"]?.timeZoneID = "America/New_York" }
    await h.server.store("home", "holiday.ics", allDayICS(rule: "FREQ=DAILY"))
    let source = try await h.source()
    let window = DateInterval(start: day(newYork, 2026, 3, 1), end: day(newYork, 2026, 3, 15))
    let mar9 = try #require(try await source.events(in: window).first { $0.originalStart == day(newYork, 2026, 3, 9) })
    try await source.delete(EventRef(mar9), scope: .thisAndFollowing, notify: .none)
    // Mar 8 is 23 hours long in New York; the series must still cover it.
    #expect(try await source.events(in: window).map(\.start) == (5...8).map { day(newYork, 2026, 3, $0) })
    let head = try await stored(h, "holiday.ics")
    #expect(head.master?.property("RRULE")?.value == "FREQ=DAILY;UNTIL=20260308")
}

@Test func anAllDayRuleChangeKeepsItsLastDayInTheCalendarZone() async throws {
    let berlin = TimeZone(identifier: "Europe/Berlin")!
    let h = CalDAVHarness()
    await h.server.configure { $0.collections["home"]?.timeZoneID = "Europe/Berlin" }
    await h.server.store("home", "holiday.ics", allDayICS(rule: "FREQ=DAILY"))
    let source = try await h.source()
    let window = DateInterval(start: day(berlin, 2026, 3, 1), end: day(berlin, 2026, 3, 20))
    let first = try #require(try await source.events(in: window).first { $0.originalStart == day(berlin, 2026, 3, 5) })
    // The caller reads a date UNTIL in the calendar's zone, so this rule ends on (and includes) March 10.
    let rule = try RecurrenceRule(rrule: "FREQ=DAILY;UNTIL=20260310", in: berlin)
    _ = try await source.update(EventRef(first), EventPatch(recurrence: .set(rule)), scope: .allInSeries, notify: .none)
    #expect(try await stored(h, "holiday.ics").master?.property("RRULE")?.value == "FREQ=DAILY;UNTIL=20260310")
    #expect(try await source.events(in: window).map(\.start) == (5...10).map { day(berlin, 2026, 3, $0) })
}

@Test func anAllDaySplitTailKeepsItsRuleChangeLastDayInTheCalendarZone() async throws {
    let berlin = TimeZone(identifier: "Europe/Berlin")!
    let h = CalDAVHarness()
    await h.server.configure { $0.collections["home"]?.timeZoneID = "Europe/Berlin" }
    await h.server.store("home", "holiday.ics", allDayICS(rule: "FREQ=DAILY"))
    let source = try await h.source()
    let window = DateInterval(start: day(berlin, 2026, 3, 1), end: day(berlin, 2026, 3, 20))
    let eighth = try #require(try await source.events(in: window).first { $0.originalStart == day(berlin, 2026, 3, 8) })
    let rule = try RecurrenceRule(rrule: "FREQ=DAILY;UNTIL=20260312", in: berlin)
    _ = try await source.update(EventRef(eighth), EventPatch(recurrence: .set(rule)), scope: .thisAndFollowing, notify: .none)
    #expect(try await stored(h, "uuid-2.ics").master?.property("RRULE")?.value == "FREQ=DAILY;UNTIL=20260312")
    #expect(try await source.events(in: window).map(\.start) == (5...12).map { day(berlin, 2026, 3, $0) })
}

@Test func updatingOrAnsweringAnOccurrenceOfASeriesThatBecameASingleEventIsNotFound() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    let tuesday = try await occurrence(source, on: 22)
    let invite = ["ORGANIZER;CN=Boss:mailto:boss@example.test", "ATTENDEE;CN=Me;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:me@icloud.test"]
    await h.server.store("home", "weekly.ics", singleICS(uid: "weekly-uid", title: "Now a one-off", attendees: invite))
    let before = await h.server.body("home", "weekly.ics")
    // Without a version the write goes straight at the current copy; with the stale version it is refused on the way in.
    var unversioned = EventRef(tuesday)
    unversioned.version = nil
    for ref in [unversioned, EventRef(tuesday)] {
        await expectWriteError(.notFound) { _ = try await source.update(ref, EventPatch(title: "x"), scope: .thisInstance, notify: .all) }
        await expectWriteError(.notFound) { _ = try await source.update(ref, EventPatch(title: "x"), scope: .thisAndFollowing, notify: .all) }
        await expectWriteError(.notFound) { _ = try await source.respond(to: ref, .accepted, scope: .thisInstance, notify: .all) }
    }
    let puts = await h.server.requests("PUT")
    let after = await h.server.body("home", "weekly.ics")
    #expect(after == before)
    // The only PUT is the stale-version attempt on the copy the caller read, which the server refuses.
    #expect(puts.allSatisfy { $0.headers["If-Match"] != nil } && puts.count <= 3)
}

@Test func aRecurrenceChangeOnOneOccurrenceIsUnsupported() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    let tuesday = try await occurrence(source, on: 22)
    let rule = try RecurrenceRule(rrule: "FREQ=DAILY;COUNT=3")
    await expectWriteError(.unsupported(fields: [.recurrence])) {
        _ = try await source.update(EventRef(tuesday), EventPatch(recurrence: .set(rule)), scope: .thisInstance, notify: .none)
    }
    let puts = await h.server.requests("PUT")
    #expect(puts.isEmpty)
}
