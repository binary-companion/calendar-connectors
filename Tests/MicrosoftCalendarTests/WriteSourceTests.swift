import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import MicrosoftCalendar

private let la = TimeZone(identifier: "America/Los_Angeles")!
private let tenAM = Date(timeIntervalSince1970: 1_790_355_600)   // 2026-09-25 10:00 in Los Angeles
private let eventPath = "cal1/events/ev1"

private func draft(uid: String? = nil) -> EventDraft {
    EventDraft(title: "Standup", timing: EventTiming(start: tenAM, end: tenAM.addingTimeInterval(1800), timeZone: la, isAllDay: false), uid: uid)
}

private func sent(_ h: SourceHarness, _ method: String, _ match: String) async -> [HTTPRequest] {
    await h.transport.requests(matching: match).filter { $0.method == method }
}

@Test func createPostsTheBodyAndReturnsTheStoredEvent() async throws {
    let h = try await SourceHarness()
    await h.transport.route("cal1/events", [.json(graphEvent(id: "new"), status: 201)])
    let event = try await h.source.create(draft(), in: "cal1", notify: .all)
    #expect(event.eventID == "new" && event.calendarID == "cal1")
    let posts = await sent(h, "POST", "cal1/events")
    #expect(posts.count == 1 && bodyJSON(posts[0])["subject"] as? String == "Standup")
    #expect(posts[0].headers["Prefer"]?.contains("outlook.timezone=\"Pacific Standard Time\"") == true)
}

@Test func createFindsAnExistingCopyByICalUID() async throws {
    let h = try await SourceHarness()
    await h.transport.route("cal1/events", [.json(["value": [graphEvent(id: "old", extra: ["iCalUId": "uid-1"])]])])
    do {
        _ = try await h.source.create(draft(uid: "uid-1"), in: "cal1", notify: .none)
        Issue.record("expected alreadyExists")
    } catch WriteError.alreadyExists(let existing) {
        #expect(existing.eventID == "old")
    }
    let filter = await h.transport.requests(matching: "$filter").first?.url.absoluteString ?? ""
    #expect(filter.contains("iCalUId%20eq%20'uid-1'") || filter.contains("iCalUId eq 'uid-1'"))
    #expect(await sent(h, "POST", "cal1/events").isEmpty)
}

@Test func createRefusesAReadOnlyOrMissingCalendar() async throws {
    let h = try await SourceHarness()
    await expectWriteError(.forbidden("read-only calendar")) { _ = try await h.source.create(draft(), in: "cal2", notify: .none) }
    await expectWriteError(.notFound) { _ = try await h.source.create(draft(), in: "nope", notify: .none) }
    #expect(await sent(h, "POST", "events").isEmpty)
}

@Test func createValidatesBeforeAnyRequest() async throws {
    let h = try await SourceHarness()
    let bad = EventDraft(title: "x", timing: EventTiming(start: tenAM, end: tenAM, timeZone: la, isAllDay: false))
    await expectWriteError(.invalid("end must be after start")) { _ = try await h.source.create(bad, in: "cal1", notify: .none) }
}

@Test func updatePatchesOnlyTheChangedFields() async throws {
    let h = try await SourceHarness()
    await h.transport.route(eventPath, [.json(graphEvent())])
    let ref = EventRef(calendarID: "cal1", eventID: "ev1", version: "ck1")
    _ = try await h.source.update(ref, EventPatch(title: "New"), scope: .thisInstance, notify: .none)
    let patches = await sent(h, "PATCH", eventPath)
    #expect(patches.count == 1 && Set(bodyJSON(patches[0]).keys) == ["subject"])
}

@Test func anEmptyPatchSendsNothing() async throws {
    let h = try await SourceHarness()
    await h.transport.route(eventPath, [.json(graphEvent())])
    let event = try await h.source.update(EventRef(calendarID: "cal1", eventID: "ev1"), EventPatch(), scope: .thisInstance, notify: .none)
    #expect(event.eventID == "ev1")
    #expect(await sent(h, "PATCH", eventPath).isEmpty)
}

@Test func aStaleVersionConflictsOnlyOnOverlappingFields() async throws {
    let h = try await SourceHarness()
    // Someone changed the title after the caller read version ck0.
    await h.transport.route(eventPath, [.json(graphEvent(extra: ["subject": "Theirs", "changeKey": "ck9"]))])
    let base = try #require(GraphEventMapper.map(try decodeEvent(graphEvent(extra: ["changeKey": "ck0"])), calendar: CalendarDescriptor(id: "cal1", title: "c", service: .microsoft, timeZone: la), accountEmail: "me@x.com", sourceID: "s"))
    var edited = base
    edited.title = "Mine"
    let ref = EventRef(base)
    await expectWriteError(.conflict(fields: [.title])) {
        _ = try await h.source.update(ref, EventPatch(from: base, to: edited), scope: .thisInstance, notify: .none)
    }
    var relocated = base
    relocated.location = "Room 9"
    _ = try await h.source.update(ref, EventPatch(from: base, to: relocated), scope: .thisInstance, notify: .none)
    #expect(await sent(h, "PATCH", eventPath).count == 1)
}

@Test func attendeeEditsResendTheCurrentListWithTheChange() async throws {
    let h = try await SourceHarness()
    await h.transport.route(eventPath, [.json(graphEvent())])
    let patch = EventPatch(attendees: AttendeeChanges(add: [AttendeeDraft(email: "new@x.com")], remove: ["opt@x.com"]))
    _ = try await h.source.update(EventRef(calendarID: "cal1", eventID: "ev1", version: "ck1"), patch, scope: .thisInstance, notify: .none)
    let attendees = bodyJSON(try #require(await sent(h, "PATCH", eventPath).first))["attendees"] as? [[String: Any]] ?? []
    let addresses = attendees.compactMap { ($0["emailAddress"] as? [String: Any])?["address"] as? String }
    #expect(addresses == ["Me@X.com", "room@x.com", "new@x.com"])
}

@Test func aSeriesWideWriteTargetsTheMasterAndASingleWriteOnTheMasterIsRefused() async throws {
    let h = try await SourceHarness()
    await h.transport.route("cal1/events/master1", [.json(graphEvent(id: "master1", extra: ["type": "seriesMaster"]))])
    let occurrence = EventRef(calendarID: "cal1", eventID: "occ1", seriesID: "master1", originalStart: tenAM)
    _ = try await h.source.update(occurrence, EventPatch(title: "All"), scope: .allInSeries, notify: .none)
    let toMaster = await sent(h, "PATCH", "cal1/events/master1").count
    let toOccurrence = await sent(h, "PATCH", "cal1/events/occ1").count
    #expect(toMaster == 1 && toOccurrence == 0)
    let master = EventRef(calendarID: "cal1", eventID: "master1", seriesID: "master1")
    await expectWriteError(.invalid("this is a recurring series; use .allInSeries or read the occurrence first")) {
        _ = try await h.source.update(master, EventPatch(title: "x"), scope: .thisInstance, notify: .none)
    }
}

@Test func aSeriesWideTimeChangeFromALaterOccurrenceIsRefused() async throws {
    let h = try await SourceHarness()
    await h.transport.route("cal1/events/master1", [.json(graphEvent(id: "master1", extra: ["type": "seriesMaster"]))])
    let later = EventRef(calendarID: "cal1", eventID: "occ2", seriesID: "master1", originalStart: tenAM.addingTimeInterval(7 * 86_400))
    let patch = EventPatch(timing: EventTiming(start: tenAM, end: tenAM.addingTimeInterval(3600), timeZone: la, isAllDay: false))
    await expectWriteError(.unsupported(fields: [.timing])) { _ = try await h.source.update(later, patch, scope: .allInSeries, notify: .none) }
    #expect(await sent(h, "PATCH", "cal1/events").isEmpty)
}

@Test func movingTheFirstOccurrenceResendsTheRangeWithTheNewStartDate() async throws {
    let h = try await SourceHarness()
    let recurrence: [String: Any] = [
        "pattern": ["type": "weekly", "interval": 1, "daysOfWeek": ["friday"], "firstDayOfWeek": "monday"],
        "range": ["type": "noEnd", "startDate": "2026-09-25", "recurrenceTimeZone": "Pacific Standard Time"]]
    await h.transport.route("cal1/events/master1", [.json(graphEvent(id: "master1", extra: ["type": "seriesMaster", "recurrence": recurrence]))])
    let first = EventRef(calendarID: "cal1", eventID: "master1", seriesID: "master1", originalStart: tenAM)
    let nextDay = tenAM.addingTimeInterval(86_400)
    let patch = EventPatch(timing: EventTiming(start: nextDay, end: nextDay.addingTimeInterval(1800), timeZone: la, isAllDay: false))
    _ = try await h.source.update(first, patch, scope: .allInSeries, notify: .none)
    let body = bodyJSON(try #require(await sent(h, "PATCH", "cal1/events/master1").first))
    #expect((body["recurrence"] as? [String: Any]).flatMap { $0["range"] as? [String: Any] }?["startDate"] as? String == "2026-09-26")
}

@Test func deleteRemovesTheInstanceOrTheWholeSeries() async throws {
    let h = try await SourceHarness()
    await h.transport.route("cal1/events/", [HTTPResponse(status: 204)])
    try await h.source.delete(EventRef(calendarID: "cal1", eventID: "occ1", seriesID: "master1", originalStart: tenAM), scope: .thisInstance, notify: .none)
    try await h.source.delete(EventRef(calendarID: "cal1", eventID: "occ1", seriesID: "master1", originalStart: tenAM), scope: .allInSeries, notify: .none)
    let deletes = await h.transport.requests(matching: "cal1/events/").filter { $0.method == "DELETE" }.map { $0.url.path }
    #expect(deletes == ["/v1.0/me/calendars/cal1/events/occ1", "/v1.0/me/calendars/cal1/events/master1"])
}

@Test func respondHonorsTheNotifyPolicyAndReadsTheEventBack() async throws {
    let h = try await SourceHarness()
    await h.transport.route(eventPath, [.json(graphEvent(extra: ["responseStatus": ["response": "accepted"]]))])
    await h.transport.route("\(eventPath)/accept", [HTTPResponse(status: 202)])
    await h.transport.route("\(eventPath)/decline", [HTTPResponse(status: 202)])
    let ref = EventRef(calendarID: "cal1", eventID: "ev1")
    let event = try await h.source.respond(to: ref, .accepted, scope: .thisInstance, notify: .all)
    #expect(event.eventID == "ev1")
    _ = try await h.source.respond(to: ref, .declined, scope: .thisInstance, notify: .none)
    let accept = try #require(await sent(h, "POST", "\(eventPath)/accept").first)
    let decline = try #require(await sent(h, "POST", "\(eventPath)/decline").first)
    #expect(bodyJSON(accept)["sendResponse"] as? Bool == true && bodyJSON(decline)["sendResponse"] as? Bool == false)
    await expectWriteError(.invalid("a response must be accepted, tentative or declined")) {
        _ = try await h.source.respond(to: ref, .needsAction, scope: .thisInstance, notify: .none)
    }
    let series = EventRef(calendarID: "cal1", eventID: "ev1", seriesID: "master1", originalStart: tenAM)
    await expectWriteError(.unsupported(fields: [.attendees])) { _ = try await h.source.respond(to: series, .accepted, scope: .thisAndFollowing, notify: .none) }
}

@Test func serviceErrorsBecomeWriteErrors() async throws {
    let h = try await SourceHarness()
    let ref = EventRef(calendarID: "cal1", eventID: "ev1")
    await h.transport.route(eventPath, [graphError("ErrorItemNotFound", status: 404)])
    await expectWriteError(.notFound) { _ = try await h.source.update(ref, EventPatch(title: "x"), scope: .thisInstance, notify: .none) }
    await h.transport.route(eventPath, [graphError("ErrorAccessDenied", status: 403)])
    await expectWriteError(.forbidden(nil)) { try await h.source.delete(ref, scope: .thisInstance, notify: .none) }
    await h.transport.route(eventPath, [graphError("ErrorInvalidRequest", message: "bad", status: 400)])
    await expectWriteError(.invalid("bad")) { try await h.source.delete(ref, scope: .thisInstance, notify: .none) }
    await h.transport.route(eventPath, [.json(graphEvent(extra: ["isCancelled": true]))])
    await expectWriteError(.notFound) { _ = try await h.source.update(ref, EventPatch(title: "x"), scope: .thisInstance, notify: .none) }
}

@Test func anUnwritableFieldIsRefusedBeforeAnyRequest() async throws {
    let h = try await SourceHarness()
    await expectWriteError(.unsupported(fields: [.reminders])) {
        _ = try await h.source.update(EventRef(calendarID: "cal1", eventID: "ev1"), EventPatch(reminders: .clear), scope: .thisInstance, notify: .none)
    }
    #expect(await h.transport.requests(matching: "cal1/events").isEmpty)
}
