import CalendarCore
import CalendarTestSupport
import Foundation
import ICalendar
import Testing
@testable import CalDAVCalendar

private func occurrence(_ source: CalDAVCalendarSource, on day: Int, hour: Int = 10) async throws -> CalendarEvent {
    try #require(try await source.events(in: september).first { $0.originalStart == pt(2026, 9, day, hour) })
}

private let autumn = DateInterval(start: pt(2026, 9, 1, 0), end: pt(2026, 11, 1, 0))

@Test func splitAnUntilSeriesMovesLaterExceptions() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(rule: "FREQ=WEEKLY;BYDAY=TU;UNTIL=20261013T235959Z", organizer: nil))
    let source = try await h.source()
    let fifteenth = try await occurrence(source, on: 15)   // the moved occurrence
    let tail = try await source.update(EventRef(fifteenth), EventPatch(title: "New sync"), scope: .thisAndFollowing, notify: .none)
    #expect(tail.eventID == "uuid-2.ics" && tail.seriesID == "uuid-2.ics" && tail.uid == "uuid-1")
    let head = try EventResource(data: Data(try #require(await h.server.body("home", "weekly.ics")).utf8))
    #expect(head.master?.property("RRULE")?.value.contains("UNTIL=20260915T165959Z") == true)
    #expect(head.overrides.isEmpty)
    let newer = try EventResource(data: Data(try #require(await h.server.body("home", "uuid-2.ics")).utf8))
    #expect(newer.overrides.count == 1 && newer.overrides.first?.property("UID")?.text == "uuid-1")
    let events = try await source.events(in: autumn)
    #expect(events.map(\.title) == ["Team sync", "Team sync (moved)", "New sync", "New sync", "New sync", "New sync"])
    #expect(events.map(\.start) == [pt(2026, 9, 1), pt(2026, 9, 16, 14), pt(2026, 9, 22), pt(2026, 9, 29), pt(2026, 10, 6), pt(2026, 10, 13)])
}

@Test func splitACountSeriesKeepsTheTotal() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(rule: "FREQ=WEEKLY;BYDAY=TU;COUNT=6", organizer: nil))
    let source = try await h.source()
    _ = try await source.update(EventRef(try await occurrence(source, on: 22)), EventPatch(title: "Later"), scope: .thisAndFollowing, notify: .none)
    let head = try EventResource(data: Data(try #require(await h.server.body("home", "weekly.ics")).utf8))
    let tail = try EventResource(data: Data(try #require(await h.server.body("home", "uuid-2.ics")).utf8))
    #expect(head.master?.property("RRULE")?.value == "FREQ=WEEKLY;BYDAY=TU;COUNT=3")
    #expect(tail.master?.property("RRULE")?.value == "FREQ=WEEKLY;BYDAY=TU;COUNT=3")
    let events = try await source.events(in: autumn)
    #expect(events.map(\.start) == [pt(2026, 9, 1), pt(2026, 9, 16, 14), pt(2026, 9, 22), pt(2026, 9, 29), pt(2026, 10, 6)])
}

@Test func splitWithATimeChangeMovesTheNewSeries() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    let later = EventTiming(start: pt(2026, 9, 22, 15), end: pt(2026, 9, 22, 16), timeZone: pacific, isAllDay: false)
    _ = try await source.update(EventRef(try await occurrence(source, on: 22)), EventPatch(timing: later), scope: .thisAndFollowing, notify: .none)
    let starts = try await source.events(in: september).map(\.start)
    #expect(starts == [pt(2026, 9, 1), pt(2026, 9, 16, 14), pt(2026, 9, 22, 15), pt(2026, 9, 29, 15)])
}

@Test func splitAtTheFirstOccurrenceIsAllInSeries() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    let series = try await source.update(EventRef(try await occurrence(source, on: 1)), EventPatch(title: "Renamed"), scope: .thisAndFollowing, notify: .none)
    #expect(series.eventID == "weekly.ics")
    #expect(await h.server.names("home") == ["weekly.ics"])
    #expect(try await source.events(in: september).filter { $0.title == "Renamed" }.count == 3)
}

@Test func aFailedSecondStepRestoresTheOriginal() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    let before = try await source.events(in: september).map(\.start)
    await h.server.fail("PUT", pathContains: "uuid-2.ics", status: 507)
    await #expect(throws: SourceError.server(status: 507)) {
        _ = try await source.update(EventRef(try await occurrence(source, on: 22)), EventPatch(title: "x"), scope: .thisAndFollowing, notify: .none)
    }
    #expect(try await source.events(in: september).map(\.start) == before)
    #expect(await h.server.names("home") == ["weekly.ics"])
}

@Test func aStaleRestoreIsPartial() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    await h.server.fail("PUT", pathContains: "uuid-2.ics", status: 507)
    await h.server.fail("PUT", pathContains: "weekly.ics", status: 412, after: 1)   // the truncation passes, the restore is stale
    do {
        _ = try await source.update(EventRef(try await occurrence(source, on: 22)), EventPatch(title: "x"), scope: .thisAndFollowing, notify: .none)
        Issue.record("expected partial")
    } catch WriteError.partial(let message) {
        #expect(message.contains("not restored"))
    }
}

// MARK: Beyond the brief: ordering, conditional headers, notify rule, restore under a concurrent edit, re-stamped overrides

/// Lets one edit from "another client" land on the original resource just before the new series is written, then
/// fails that write.
private struct MeddlingTransport: HTTPTransport {
    let server: FakeCalDAVServer

    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        if request.method == "PUT", request.url.path.contains("uuid-2.ics") {
            await server.store("home", "weekly.ics", weeklyICS(organizer: nil).replacingOccurrences(of: "Team sync", with: "Edited elsewhere"))
            return HTTPResponse(status: 507)
        }
        return try await server.send(request)
    }
}

@Test func theHeadIsWrittenFirstUnderIfMatchAndTheTailUnderIfNoneMatch() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    let original = try #require(await h.server.etag("home", "weekly.ics"))
    let target = try await occurrence(source, on: 22)
    await h.server.clearLog()
    _ = try await source.update(EventRef(target), EventPatch(title: "x"), scope: .thisAndFollowing, notify: .none)
    let puts = await h.server.requests("PUT")
    #expect(puts.count == 2)
    #expect(puts[0].url.path.hasSuffix("weekly.ics") && puts[0].headers["If-Match"] == original && puts[0].headers["If-None-Match"] == nil)
    #expect(puts[1].url.path.hasSuffix("uuid-2.ics") && puts[1].headers["If-None-Match"] == "*" && puts[1].headers["If-Match"] == nil)
}

@Test func theRestoreGoesBackUnderTheTruncationsETag() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    let original = try #require(await h.server.etag("home", "weekly.ics"))
    let target = try await occurrence(source, on: 22)
    await h.server.fail("PUT", pathContains: "uuid-2.ics", status: 507)
    await h.server.clearLog()
    await #expect(throws: SourceError.server(status: 507)) {
        _ = try await source.update(EventRef(target), EventPatch(title: "x"), scope: .thisAndFollowing, notify: .none)
    }
    let puts = await h.server.requests("PUT")
    #expect(puts.count == 3)
    let restoreMatch = try #require(puts.last?.headers["If-Match"])
    #expect(restoreMatch != original)   // the truncation's ETag, not the one read at the start
    let restored = try EventResource(data: Data(try #require(await h.server.body("home", "weekly.ics")).utf8))
    #expect(restored.master?.property("RRULE")?.value == "FREQ=WEEKLY;BYDAY=TU")
    #expect(restored.overrides.count == 1)
    // Attendees ignore an update that is not newer than the truncation they already hold: SEQUENCE 0 -> 1 (cut) -> 2 (restore).
    #expect(restored.master?.property("SEQUENCE")?.value == "2")
    #expect(restored.master?.property("DTSTAMP")?.value != "20260901T000000Z")
    #expect(restored.master?.property("LAST-MODIFIED") != nil)
}

@Test func aRestoreIsNeverForcedOverAnEditMadeMeanwhile() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source(transport: MeddlingTransport(server: h.server))
    let target = try await occurrence(source, on: 22)
    do {
        _ = try await source.update(EventRef(target), EventPatch(title: "x"), scope: .thisAndFollowing, notify: .none)
        Issue.record("expected partial")
    } catch WriteError.partial(let message) {
        #expect(message.contains("not restored"))
    }
    #expect(try #require(await h.server.body("home", "weekly.ics")).contains("Edited elsewhere"))
    #expect(await h.server.names("home") == ["weekly.ics"])
}

/// The account organizes the series and invites a guest.
private func organizedByMeICS() -> String {
    weeklyICS(organizer: "mailto:me@icloud.test")
        .replacingOccurrences(of: "ATTENDEE;CN=Me;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:me@icloud.test", with: "ATTENDEE;CN=Guest:mailto:guest@example.test")
}

@Test func aSplitThatTellsOthersIsRefusedBeforeAnyRequestUnlessNotifyIsAll() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", organizedByMeICS())
    let source = try await h.source()
    let target = try await occurrence(source, on: 22)
    for policy in [NotifyPolicy.none, .externalOnly] {
        await h.server.clearLog()
        await #expect(throws: WriteError.unsupported(fields: [.attendees])) {
            _ = try await source.update(EventRef(target), EventPatch(title: "x"), scope: .thisAndFollowing, notify: policy)
        }
        #expect(await h.server.requests("PUT").isEmpty)
    }
    _ = try await source.update(EventRef(target), EventPatch(title: "x"), scope: .thisAndFollowing, notify: .all)
    #expect(await h.server.names("home") == ["uuid-2.ics", "weekly.ics"])
}

@Test func carriedOverExceptionsAreRestampedUnderTheNewUID() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    _ = try await source.update(EventRef(try await occurrence(source, on: 15)), EventPatch(title: "New sync"), scope: .thisAndFollowing, notify: .none)
    let tail = try EventResource(data: Data(try #require(await h.server.body("home", "uuid-2.ics")).utf8))
    let carried = try #require(tail.overrides.first)
    let master = try #require(tail.master)
    #expect(carried.property("DTSTAMP")?.value == master.property("DTSTAMP")?.value)
    #expect(carried.property("DTSTAMP")?.value != "20260901T000000Z")
    #expect(carried.property("LAST-MODIFIED")?.value == master.property("LAST-MODIFIED")?.value)
}

// MARK: Fix round 1: unclear failures, all-day guard, organizer guard

/// Wraps the fake server; the hook may answer a request itself (or throw) before the server sees it, or call
/// `server.send` and change what comes back. Nil passes the request through.
private struct ScriptedTransport: HTTPTransport {
    let server: FakeCalDAVServer
    let hook: @Sendable (HTTPRequest, FakeCalDAVServer) async throws -> HTTPResponse?

    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        if let answer = try await hook(request, server) { return answer }
        return try await server.send(request)
    }
}

private func isPUT(_ request: HTTPRequest, _ name: String) -> Bool { request.method == "PUT" && request.url.path.hasSuffix(name) }
private func isGET(_ request: HTTPRequest, _ name: String) -> Bool { request.method == "GET" && request.url.path.hasSuffix(name) }

/// Counts requests so a hook can act on the n-th one.
private actor Count {
    private var value = 0
    func next() -> Int { value += 1; return value }
    var current: Int { value }
}

private func seriesStarts(_ source: CalDAVCalendarSource) async throws -> [Date] {
    try await source.events(in: autumn).map(\.start)
}

// 1. The tail's outcome is checked before anything is restored.

@Test func aReadBackFailureAfterTheNewSeriesWasStoredDoesNotRestoreTheHead() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    await h.server.configure { $0.sendsETagOnPut = false }   // put() must GET the new resource back
    let source = try await h.source()
    let target = try await occurrence(source, on: 22)
    await h.server.fail("GET", pathContains: "uuid-2.ics", status: 500)   // the read-back fails once
    let tail = try await source.update(EventRef(target), EventPatch(title: "Later"), scope: .thisAndFollowing, notify: .none)
    #expect(tail.eventID == "uuid-2.ics")
    #expect(await h.server.names("home") == ["uuid-2.ics", "weekly.ics"])
    let head = try EventResource(data: Data(try #require(await h.server.body("home", "weekly.ics")).utf8))
    #expect(head.master?.property("RRULE")?.value.contains("UNTIL") == true)
    let starts = try await seriesStarts(source)
    #expect(Set(starts).count == starts.count)   // no occurrence twice
}

@Test func aLostReplyToTheNewSeriesThatWentThroughKeepsTheSplit() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let transport = ScriptedTransport(server: h.server) { request, server in
        guard isPUT(request, "uuid-2.ics") else { return nil }
        _ = try await server.send(request)   // applied ...
        throw SourceError.network("connection lost")   // ... but the reply never arrived
    }
    let source = try await h.source(transport: transport)
    let target = try await occurrence(source, on: 22)
    let tail = try await source.update(EventRef(target), EventPatch(title: "Later"), scope: .thisAndFollowing, notify: .none)
    #expect(tail.eventID == "uuid-2.ics")
    let starts = try await seriesStarts(source)
    #expect(Set(starts).count == starts.count)
    #expect(starts.count == 8)
}

@Test func aCancelledCallerWhoseNewSeriesWentThroughDoesNotGetDuplicates() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let transport = ScriptedTransport(server: h.server) { request, server in
        guard isPUT(request, "uuid-2.ics") else { return nil }
        _ = try await server.send(request)
        throw CancellationError()
    }
    let source = try await h.source(transport: transport)
    let target = try await occurrence(source, on: 22)
    _ = try? await source.update(EventRef(target), EventPatch(title: "Later"), scope: .thisAndFollowing, notify: .none)
    let starts = try await seriesStarts(source)
    #expect(Set(starts).count == starts.count)
    #expect(await h.server.names("home") == ["uuid-2.ics", "weekly.ics"])
}

@Test func anUnknownOutcomeOfTheNewSeriesIsPartialAndNotRestoredBlindly() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let transport = ScriptedTransport(server: h.server) { request, server in
        if isPUT(request, "uuid-2.ics") { _ = try await server.send(request); throw SourceError.network("connection lost") }
        if isGET(request, "uuid-2.ics") { throw SourceError.network("still down") }
        return nil
    }
    let source = try await h.source(transport: transport)
    let target = try await occurrence(source, on: 22)
    await h.server.clearLog()
    do {
        _ = try await source.update(EventRef(target), EventPatch(title: "Later"), scope: .thisAndFollowing, notify: .none)
        Issue.record("expected partial")
    } catch WriteError.partial(let message) {
        #expect(message.contains("unknown"))
    }
    #expect(await h.server.requests("PUT").count == 2)   // the truncation and the (lost) new series; no restore
}

@Test func aNameClashOnTheNewSeriesRestoresTheOriginalAndLeavesTheOtherResourceAlone() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    await h.server.store("home", "uuid-2.ics", singleICS(uid: "someone-elses"))
    let theirs = try #require(await h.server.body("home", "uuid-2.ics"))
    let source = try await h.source()
    let before = try await source.events(in: september).filter { $0.uid == "weekly-uid" }.map(\.start)
    let target = try await occurrence(source, on: 22)
    await #expect(throws: SourceError.invalidResponse("a resource with the new series' name already exists")) {
        _ = try await source.update(EventRef(target), EventPatch(title: "x"), scope: .thisAndFollowing, notify: .none)
    }
    #expect(await h.server.body("home", "uuid-2.ics") == theirs)
    #expect(try await source.events(in: september).filter { $0.uid == "weekly-uid" }.map(\.start) == before)
    let head = try EventResource(data: Data(try #require(await h.server.body("home", "weekly.ics")).utf8))
    #expect(head.master?.property("RRULE")?.value == "FREQ=WEEKLY;BYDAY=TU")
}

// 2. An unclear failure of the truncation is looked up.

@Test func aReadBackFailureAfterTheTruncationRestoresTheSeries() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    await h.server.configure { $0.sendsETagOnPut = false }
    let puts = Count()
    let source = try await h.source(transport: ScriptedTransport(server: h.server) { request, server in
        // After the truncation was stored, the read-back GET (the first GET of the resource after it) fails once.
        if isPUT(request, "weekly.ics") {
            let answer = try await server.send(request)
            if await puts.next() == 1 { await server.fail("GET", pathContains: "weekly.ics", status: 500) }
            return answer
        }
        return nil
    })
    let before = try await source.events(in: september).map(\.start)
    let target = try await occurrence(source, on: 22)
    await #expect(throws: SourceError.server(status: 500)) {
        _ = try await source.update(EventRef(target), EventPatch(title: "x"), scope: .thisAndFollowing, notify: .none)
    }
    #expect(await h.server.names("home") == ["weekly.ics"])
    #expect(try await source.events(in: september).map(\.start) == before)
    let restored = try EventResource(data: Data(try #require(await h.server.body("home", "weekly.ics")).utf8))
    #expect(restored.master?.property("RRULE")?.value == "FREQ=WEEKLY;BYDAY=TU")
    #expect(restored.master?.property("SEQUENCE")?.value == "2")
}

@Test func aLostReplyToTheTruncationThatWentThroughIsRestored() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let puts = Count()
    let source = try await h.source(transport: ScriptedTransport(server: h.server) { request, server in
        guard isPUT(request, "weekly.ics"), await puts.next() == 1 else { return nil }
        _ = try await server.send(request)
        throw SourceError.network("connection lost")
    })
    let before = try await source.events(in: september).map(\.start)
    let target = try await occurrence(source, on: 22)
    await #expect(throws: SourceError.network("connection lost")) {
        _ = try await source.update(EventRef(target), EventPatch(title: "x"), scope: .thisAndFollowing, notify: .none)
    }
    #expect(await h.server.names("home") == ["weekly.ics"])
    #expect(try await source.events(in: september).map(\.start) == before)
}

@Test func aLostTruncationThatWasNeverAppliedChangesNothing() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source(transport: ScriptedTransport(server: h.server) { request, _ in
        if isPUT(request, "weekly.ics") { throw SourceError.network("connection lost") }
        return nil
    })
    let target = try await occurrence(source, on: 22)
    let original = try #require(await h.server.body("home", "weekly.ics"))
    await h.server.clearLog()
    await #expect(throws: SourceError.network("connection lost")) {
        _ = try await source.update(EventRef(target), EventPatch(title: "x"), scope: .thisAndFollowing, notify: .none)
    }
    #expect(await h.server.body("home", "weekly.ics") == original)
    #expect(await h.server.requests("PUT").isEmpty)   // the hook swallowed the only PUT; nothing was restored
}

@Test func aLostTruncationReplyOverAnEditByAnotherClientIsPartial() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source(transport: ScriptedTransport(server: h.server) { request, server in
        guard isPUT(request, "weekly.ics") else { return nil }
        await server.store("home", "weekly.ics", weeklyICS(organizer: nil).replacingOccurrences(of: "Team sync", with: "Edited elsewhere"))
        throw SourceError.network("connection lost")
    })
    let target = try await occurrence(source, on: 22)
    do {
        _ = try await source.update(EventRef(target), EventPatch(title: "x"), scope: .thisAndFollowing, notify: .none)
        Issue.record("expected partial")
    } catch WriteError.partial {}
    #expect(try #require(await h.server.body("home", "weekly.ics")).contains("Edited elsewhere"))
}

@Test func aLostTruncationReplyThatCannotBeLookedUpIsPartial() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let puts = Count()
    let source = try await h.source(transport: ScriptedTransport(server: h.server) { request, server in
        if isPUT(request, "weekly.ics") { _ = await puts.next(); _ = try await server.send(request); throw SourceError.network("connection lost") }
        // Once the truncation was sent, the resource can no longer be looked at.
        if isGET(request, "weekly.ics"), await puts.current > 0 { throw SourceError.network("still down") }
        return nil
    })
    let target = try await occurrence(source, on: 22)
    do {
        _ = try await source.update(EventRef(target), EventPatch(title: "x"), scope: .thisAndFollowing, notify: .none)
        Issue.record("expected partial")
    } catch WriteError.partial(let message) {
        #expect(message.contains("unknown"))
    }
}

// 3. No switch between all-day and timed when exceptions would be carried.

@Test func aSplitThatSwitchesToAllDayIsRefusedWhenExceptionsWouldBeCarried() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    let target = try await occurrence(source, on: 15)   // the series has an exception at the 15th
    let allDay = EventTiming(start: pt(2026, 9, 15, 0), end: pt(2026, 9, 16, 0), timeZone: pacific, isAllDay: true)
    await h.server.clearLog()
    await #expect(throws: WriteError.unsupported(fields: [.timing])) {
        _ = try await source.update(EventRef(target), EventPatch(timing: allDay), scope: .thisAndFollowing, notify: .none)
    }
    #expect(await h.server.requests("PUT").isEmpty)
}

// 5. Only the organizer (or nobody) splits.

@Test func anAttendeeCannotSplitTheOrganizersSeries() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS())   // organized by boss@example.test, the account is invited
    let source = try await h.source()
    let target = try await occurrence(source, on: 22)
    await h.server.clearLog()
    await #expect(throws: WriteError.unsupported(fields: [.attendees])) {
        _ = try await source.update(EventRef(target), EventPatch(title: "x"), scope: .thisAndFollowing, notify: .all)
    }
    #expect(await h.server.requests("PUT").isEmpty)
    #expect(await h.server.names("home") == ["weekly.ics"])
}

// MARK: Fix round 2: our truncation is recognised on every component

@Test func anEditToAKeptOverrideAfterALostTruncationReplyIsNotRestoredOver() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let puts = Count()
    let source = try await h.source(transport: ScriptedTransport(server: h.server) { request, server in
        guard isPUT(request, "weekly.ics"), await puts.next() == 1 else { return nil }
        let landed = try await server.send(request)
        precondition(landed.status == 204 || landed.status == 201)
        // Our truncation is on the server; before the caller looks again another client edits the kept override at the 15th.
        var body = try #require(await server.body("home", "weekly.ics"))
        body = body.replacingOccurrences(of: "Team sync (moved)", with: "Edited by another client")
        // A real client stamps its edit (only the override carries this DTSTAMP and is followed by its RECURRENCE-ID).
        body = body.replacingOccurrences(of: "DTSTAMP:20260901T000000Z\r\nRECURRENCE-ID", with: "DTSTAMP:20260922T000000Z\r\nRECURRENCE-ID")
        await server.store("home", "weekly.ics", body)
        throw SourceError.network("connection lost")
    })
    let target = try await occurrence(source, on: 22)
    await h.server.clearLog()
    do {
        _ = try await source.update(EventRef(target), EventPatch(title: "x"), scope: .thisAndFollowing, notify: .none)
        Issue.record("expected partial")
    } catch WriteError.partial {}
    #expect(try #require(await h.server.body("home", "weekly.ics")).contains("Edited by another client"))
    #expect(await h.server.requests("PUT").count == 1)   // only our truncation; nothing was restored over the edit
    #expect(await h.server.names("home") == ["weekly.ics"])
}
