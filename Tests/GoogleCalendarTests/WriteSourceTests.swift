import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import GoogleCalendar

private let utc = TimeZone(identifier: "UTC")!
private func instant(_ s: String) -> Date { ISO8601DateFormatter().date(from: s)! }
private let cal = "me@x.com"
private let calPath = "calendars/me%40x.com/events"

private func timing() -> EventTiming {
    EventTiming(start: instant("2026-09-21T10:00:00Z"), end: instant("2026-09-21T10:30:00Z"), timeZone: utc, isAllDay: false)
}
private func base(title: String = "Standup", version: String = "e1", attendees: [Attendee] = []) -> CalendarEvent {
    CalendarEvent(eventID: "ev1", calendarID: cal, title: title, start: instant("2026-09-21T10:00:00Z"), end: instant("2026-09-21T10:30:00Z"),
                  timeZone: utc, attendees: attendees, version: version)
}

@Test func googleDeclaresItsWriteCapabilities() async throws {
    let h = try await Harness()
    let c = h.source.capabilities
    #expect(c.canWrite && c.canEditAttendees && c.canRespondToInvite && c.controlsNotifications)
    #expect(c.writableFields == Set(EventField.allCases) && c.recurrenceScopes == Set(RecurrenceScope.allCases))
}

@Test func createPostsToTheCalendarAndMapsTheResult() async throws {
    let h = try await Harness()
    await h.transport.route(calPath, [.json(googleEvent(id: "new1", etag: "e9", summary: "Sync"))])
    let draft = EventDraft(title: "Sync", timing: timing(), attendees: [AttendeeDraft(email: "bob@x.com")], conference: .generate)
    let created = try await h.source.create(draft, in: cal, notify: .all)
    let post = try #require(await h.transport.requests(matching: calPath).last)
    #expect(post.method == "POST" && post.url.absoluteString.contains("sendUpdates=all") && post.url.absoluteString.contains("conferenceDataVersion=1"))
    #expect(bodyJSON(post)["summary"] as? String == "Sync")
    #expect(created.eventID == "new1" && created.version == "e9" && created.sourceID == "google-c1" && created.calendarID == cal)
}

@Test func writesToAReadOnlyCalendarAreForbiddenBeforeAnyEventRequest() async throws {
    let h = try await Harness()
    await expectWriteError(.forbidden("read-only calendar")) {
        _ = try await h.source.create(EventDraft(title: "T", timing: timing()), in: "team@group.calendar.google.com", notify: .none)
    }
    await expectWriteError(.notFound) { _ = try await h.source.create(EventDraft(title: "T", timing: timing()), in: "nope", notify: .none) }
    #expect(await h.transport.requests(matching: "/events").isEmpty)
}

@Test func createRefusesAnInvalidDraftBeforeAnyRequest() async throws {
    let h = try await Harness()
    var draft = EventDraft(title: "T", timing: timing())
    draft.reminders = [Reminder(minutesBefore: -1)]
    await expectWriteError(.invalid("reminder minutes must not be negative")) { _ = try await h.source.create(draft, in: cal, notify: .none) }
    #expect(await h.transport.requests.isEmpty)
    // A rule only Google's mapper enforces is also caught before the calendar lookup.
    draft.reminders = [Reminder(minutesBefore: 40321)]
    await expectWriteError(.invalid("reminder minutes must be between 0 and 40320")) { _ = try await h.source.create(draft, in: cal, notify: .none) }
    #expect(await h.transport.requests.isEmpty)
}

@Test func updateSendsOnlyTheChangedFieldsWithIfMatch() async throws {
    let h = try await Harness()
    await h.transport.route("\(calPath)/ev1", [.json(googleEvent(id: "ev1", etag: "e2", summary: "New"))])
    let updated = try await h.source.update(EventRef(calendarID: cal, eventID: "ev1", version: "e1"), EventPatch(title: "New"),
                                            scope: .thisInstance, notify: .none)
    let patch = try #require(await h.transport.requests(matching: "\(calPath)/ev1").last)
    #expect(patch.method == "PATCH" && patch.headers["If-Match"] == "e1" && patch.url.absoluteString.contains("sendUpdates=none"))
    #expect((bodyJSON(patch) as NSDictionary).isEqual(["summary": "New"] as NSDictionary))
    #expect(updated.title == "New" && updated.version == "e2")
}

@Test func aStaleVersionWithNoOverlapIsRetriedOnTheFreshEtag() async throws {
    let h = try await Harness()
    var edit = EventEdit(base())
    edit.event.title = "New"
    await h.transport.route("\(calPath)/ev1", [
        googleError("conditionNotMet", status: 412),
        .json(googleEvent(id: "ev1", etag: "e2", summary: "Standup", extra: ["location": "Elsewhere"])),   // someone else set the location
        .json(googleEvent(id: "ev1", etag: "e3", summary: "New", extra: ["location": "Elsewhere"])),
    ])
    let updated = try await h.source.update(EventRef(base()), edit.patch, scope: .thisInstance, notify: .none)
    let sent = await h.transport.requests(matching: "\(calPath)/ev1")
    #expect(sent.map(\.method) == ["PATCH", "GET", "PATCH"] && sent[0].headers["If-Match"] == "e1" && sent[2].headers["If-Match"] == "e2")
    #expect(updated.title == "New" && updated.location == "Elsewhere")
    // The retry resends only what this edit changed, not the other writer's location.
    #expect((bodyJSON(sent[2]) as NSDictionary).isEqual(["summary": "New"] as NSDictionary))
}

@Test func aStaleVersionWithAnOverlapIsAConflict() async throws {
    let h = try await Harness()
    var edit = EventEdit(base())
    edit.event.title = "Mine"
    await h.transport.route("\(calPath)/ev1", [googleError("conditionNotMet", status: 412), .json(googleEvent(id: "ev1", etag: "e2", summary: "Theirs"))])
    await expectWriteError(.conflict(fields: [.title])) { _ = try await h.source.update(EventRef(base()), edit.patch, scope: .thisInstance, notify: .none) }
}

@Test func attendeeChangesFetchThenPatchTheFullArrayWithTheFetchedEtag() async throws {
    let h = try await Harness()
    let attendees: [[String: Any]] = [["email": "me@x.com", "self": true, "responseStatus": "accepted"], ["email": "bob@x.com", "responseStatus": "accepted"]]
    await h.transport.route("\(calPath)/ev1", [
        .json(googleEvent(id: "ev1", etag: "e1", extra: ["attendees": attendees])),
        .json(googleEvent(id: "ev1", etag: "e2")),
    ])
    let patch = EventPatch(attendees: AttendeeChanges(add: [AttendeeDraft(email: "dee@x.com")], remove: ["bob@x.com"]))
    _ = try await h.source.update(EventRef(calendarID: cal, eventID: "ev1", version: "e1"), patch, scope: .thisInstance, notify: .externalOnly)
    let sent = await h.transport.requests(matching: "\(calPath)/ev1")
    #expect(sent.map(\.method) == ["GET", "PATCH"] && sent[1].headers["If-Match"] == "e1" && sent[1].url.absoluteString.contains("sendUpdates=externalOnly"))
    let merged = try #require(bodyJSON(sent[1])["attendees"] as? [[String: Any]])
    #expect(merged.map { $0["email"] as? String } == ["me@x.com", "dee@x.com"])
}

@Test func anAttendeeChangeConflictsWhenSomeoneElseChangedTheSameAttendee() async throws {
    let h = try await Harness()
    var edit = EventEdit(base(version: "e5", attendees: [Attendee(email: "bob@x.com", role: .required)]))
    edit.event.attendees[0].role = .resource
    await h.transport.route("\(calPath)/ev1", [.json(googleEvent(id: "ev1", etag: "e6", extra: ["attendees": [["email": "bob@x.com", "optional": true]]]))])
    await expectWriteError(.conflict(fields: [.attendees])) {
        _ = try await h.source.update(EventRef(calendarID: cal, eventID: "ev1", version: "e5"), edit.patch, scope: .thisInstance, notify: .none)
    }
    #expect(await h.transport.requests(matching: "\(calPath)/ev1").allSatisfy { $0.method == "GET" })
}

@Test func anEmptyPatchReturnsTheBaseWithoutAnyRequest() async throws {
    let h = try await Harness()
    let edit = EventEdit(base())
    let result = try await h.source.update(EventRef(base()), edit.patch, scope: .thisInstance, notify: .none)
    let requestCount = await h.transport.requests.count
    #expect(result == base() && requestCount == 1)   // only the calendar list used for the access check
}

@Test func instanceAndSeriesScopesTargetTheInstanceOrTheMaster() async throws {
    let h = try await Harness()
    await h.transport.route("\(calPath)/master1", [.json(googleEvent(id: "master1", etag: "m2", summary: "All"))])
    await h.transport.route("\(calPath)/master1_20260921T100000Z", [.json(googleEvent(id: "master1_20260921T100000Z", etag: "i2", summary: "One"))])
    let ref = EventRef(calendarID: cal, eventID: "master1_20260921T100000Z", version: "i1", seriesID: "master1", originalStart: instant("2026-09-21T10:00:00Z"))
    _ = try await h.source.update(ref, EventPatch(title: "One"), scope: .thisInstance, notify: .none)
    _ = try await h.source.update(ref, EventPatch(title: "All"), scope: .allInSeries, notify: .none)
    let instance = try #require(await h.transport.requests(matching: "master1_2026").last)
    let master = try #require(await h.transport.requests(matching: "\(calPath)/master1?").last)
    #expect(instance.headers["If-Match"] == "i1")
    #expect(master.headers["If-Match"] == nil && master.method == "PATCH")   // an instance etag cannot lock the master
}

@Test func respondPatchesOnlyTheSelfAttendeeWithTheFetchedEtag() async throws {
    let h = try await Harness()
    let attendees: [[String: Any]] = [["email": "me@x.com", "self": true, "responseStatus": "needsAction"], ["email": "bob@x.com", "responseStatus": "accepted"]]
    await h.transport.route("\(calPath)/ev1", [.json(googleEvent(id: "ev1", etag: "e4", extra: ["attendees": attendees])), .json(googleEvent(id: "ev1", etag: "e5"))])
    _ = try await h.source.respond(to: EventRef(calendarID: cal, eventID: "ev1", version: "e1"), .declined, scope: .thisInstance, notify: .all)
    let patch = try #require(await h.transport.requests(matching: "\(calPath)/ev1").last)
    #expect(patch.method == "PATCH" && patch.headers["If-Match"] == "e4" && patch.url.absoluteString.contains("sendUpdates=all"))
    let sent = try #require(bodyJSON(patch)["attendees"] as? [[String: Any]])
    #expect(sent[0]["responseStatus"] as? String == "declined" && sent[1]["responseStatus"] as? String == "accepted")
    await expectWriteError(.unsupported(fields: [.attendees])) {
        _ = try await h.source.respond(to: EventRef(calendarID: cal, eventID: "ev1", seriesID: "m", originalStart: Date()), .accepted, scope: .thisAndFollowing, notify: .none)
    }
}

@Test func deleteSendsDeleteWithTheNotifyPolicyAndMapsGoneToNotFound() async throws {
    let h = try await Harness()
    await h.transport.route("\(calPath)/ev1", [HTTPResponse(status: 204)])
    try await h.source.delete(EventRef(calendarID: cal, eventID: "ev1"), scope: .thisInstance, notify: .all)
    let request = try #require(await h.transport.requests(matching: "\(calPath)/ev1").last)
    #expect(request.method == "DELETE" && request.url.absoluteString.contains("sendUpdates=all"))
    await h.transport.route("\(calPath)/ev2", [googleError("deleted", status: 410)])
    await expectWriteError(.notFound) { try await h.source.delete(EventRef(calendarID: cal, eventID: "ev2"), scope: .thisInstance, notify: .none) }
}

@Test func aForbiddenWriteIsReportedAsForbidden() async throws {
    let h = try await Harness()
    await h.transport.route("\(calPath)/ev1", [googleError("forbiddenForNonOrganizer", status: 403)])
    await expectWriteError(.forbidden(nil)) { _ = try await h.source.update(EventRef(calendarID: cal, eventID: "ev1", version: "e1"), EventPatch(title: "X"), scope: .thisInstance, notify: .none) }
}

// MARK: Edge cases beyond the brief

@Test func aRequestThatCannotBeBuiltIsRefusedBeforeAnyEventRequest() async throws {
    let h = try await Harness()
    // An invalid patch is caught even though the attendee path would otherwise fetch first.
    var patch = EventPatch(reminders: .set(Array(repeating: Reminder(minutesBefore: 5), count: 6)))
    patch.attendees = AttendeeChanges(add: [AttendeeDraft(email: "dee@x.com")])
    await expectWriteError(.invalid("Google allows at most 5 reminders")) {
        _ = try await h.source.update(EventRef(calendarID: cal, eventID: "ev1", version: "e1"), patch, scope: .thisInstance, notify: .none)
    }
    // An empty event id would address the events collection itself.
    await expectWriteError(.invalid("the event id must not be empty")) {
        try await h.source.delete(EventRef(calendarID: cal, eventID: ""), scope: .thisInstance, notify: .none)
    }
    await expectWriteError(.invalid("the event id must not be empty")) {
        _ = try await h.source.update(EventRef(calendarID: cal, eventID: "", version: "e1"), EventPatch(title: "X"), scope: .thisInstance, notify: .none)
    }
    await expectWriteError(.invalid("cannot respond with needsAction")) {
        _ = try await h.source.respond(to: EventRef(calendarID: cal, eventID: "ev1"), .needsAction, scope: .thisInstance, notify: .none)
    }
    #expect(await h.transport.requests(matching: "/events").isEmpty)
}

@Test func anUnreadableEventResponseIsAnInvalidResponseNotACocoaError() async throws {
    let h = try await Harness()
    await h.transport.route("\(calPath)/ev1", [.text("<html>oops</html>"), .json([1, 2, 3])])
    for _ in 0..<2 {
        do {
            _ = try await h.source.respond(to: EventRef(calendarID: cal, eventID: "ev1"), .accepted, scope: .thisInstance, notify: .none)
            Issue.record("expected an error")
        } catch let error as SourceError {
            #expect(error == .invalidResponse("google: unreadable event"))
        } catch {
            Issue.record("expected SourceError, got \(error)")
        }
    }
}

@Test func respondingToADeletedEventIsNotFound() async throws {
    let h = try await Harness()
    await h.transport.route("\(calPath)/ev1", [.json(googleEvent(id: "ev1", extra: ["status": "cancelled"]))])
    await expectWriteError(.notFound) {
        _ = try await h.source.respond(to: EventRef(calendarID: cal, eventID: "ev1"), .accepted, scope: .thisInstance, notify: .none)
    }
    #expect(await h.transport.requests(matching: "\(calPath)/ev1").allSatisfy { $0.method == "GET" })
}

@Test func respondRetriesOnTheFreshEtagWhenTheEventChangesBetweenFetchAndPatch() async throws {
    let h = try await Harness()
    let attendees: [[String: Any]] = [["email": "me@x.com", "self": true, "responseStatus": "needsAction"]]
    await h.transport.route("\(calPath)/ev1", [
        .json(googleEvent(id: "ev1", etag: "e4", extra: ["attendees": attendees])), googleError("conditionNotMet", status: 412),
        .json(googleEvent(id: "ev1", etag: "e5", extra: ["attendees": attendees])), .json(googleEvent(id: "ev1", etag: "e6")),
    ])
    let result = try await h.source.respond(to: EventRef(calendarID: cal, eventID: "ev1"), .accepted, scope: .thisInstance, notify: .none)
    let sent = await h.transport.requests(matching: "\(calPath)/ev1")
    #expect(sent.map(\.method) == ["GET", "PATCH", "GET", "PATCH"] && sent[3].headers["If-Match"] == "e5" && result.version == "e6")
}

@Test func aQuotaForbiddenWriteIsRateLimitedNotAPermissionError() async throws {
    let h = try await Harness()
    await h.transport.route("\(calPath)/ev1", [googleError("quotaExceeded", status: 403, headers: ["Retry-After": "30"])])
    do {
        _ = try await h.source.update(EventRef(calendarID: cal, eventID: "ev1", version: "e1"), EventPatch(title: "X"), scope: .thisInstance, notify: .none)
        Issue.record("expected an error")
    } catch let error as SourceError {
        #expect(error == .rateLimited(retryAfter: 30))
    } catch {
        Issue.record("expected SourceError, got \(error)")
    }
    await h.transport.route("\(calPath)/ev3", [googleError("calendarUsageLimitsExceeded", status: 403)])
    await #expect(throws: SourceError.rateLimited(retryAfter: nil)) {
        try await h.source.delete(EventRef(calendarID: cal, eventID: "ev3"), scope: .thisInstance, notify: .none)
    }
    // Reads keep their behaviour.
    await h.transport.route("calendars/me%40x.com/events", [googleError("quotaExceeded", status: 403)])
    await #expect(throws: SourceError.invalidResponse("HTTP 403: quotaExceeded")) {
        _ = try await h.source.events(in: DateInterval(start: .now, duration: 3600))
    }
}

// MARK: Review fixes

private func masterRequests(_ h: Harness) async -> [HTTPRequest] {
    await h.transport.requests(matching: "\(calPath)/master1").filter { $0.url.path.hasSuffix("/master1") }
}

private func laterOccurrence() -> EventRef {
    EventRef(calendarID: cal, eventID: "master1_20260921T100000Z", version: "i1", seriesID: "master1", originalStart: instant("2026-09-21T10:00:00Z"))
}

nonisolated(unsafe) private let masterStart: [String: Any] = ["dateTime": "2026-09-14T10:00:00Z", "timeZone": "UTC"]
nonisolated(unsafe) private let masterEnd: [String: Any] = ["dateTime": "2026-09-14T10:30:00Z", "timeZone": "UTC"]

private func moved() -> EventTiming {
    EventTiming(start: instant("2026-09-21T11:00:00Z"), end: instant("2026-09-21T11:30:00Z"), timeZone: utc, isAllDay: false)
}

@Test func aSeriesWideTimeChangeFromALaterOccurrenceIsRefusedBeforeAnyPatch() async throws {
    let h = try await Harness()
    await h.transport.route("\(calPath)/master1", [.json(googleEvent(id: "master1", extra: ["start": masterStart, "end": masterEnd]))])
    await expectWriteError(.unsupported(fields: [.timing])) {
        _ = try await h.source.update(laterOccurrence(), EventPatch(timing: moved()), scope: .allInSeries, notify: .none)
    }
    // Without the occurrence's slot there is nothing to compare, so it is refused too.
    var noSlot = laterOccurrence()
    noSlot.originalStart = nil
    await expectWriteError(.unsupported(fields: [.timing])) {
        _ = try await h.source.update(noSlot, EventPatch(timing: moved()), scope: .allInSeries, notify: .none)
    }
    #expect(await masterRequests(h).allSatisfy { $0.method == "GET" })
}

@Test func aSeriesWideTimeChangeFromTheFirstOccurrenceIsSent() async throws {
    let h = try await Harness()
    await h.transport.route("\(calPath)/master1", [
        .json(googleEvent(id: "master1", extra: ["start": masterStart, "end": masterEnd])), .json(googleEvent(id: "master1", etag: "m2")),
    ])
    let first = EventRef(calendarID: cal, eventID: "master1_20260914T100000Z", version: "i1", seriesID: "master1", originalStart: instant("2026-09-14T10:00:00Z"))
    _ = try await h.source.update(first, EventPatch(timing: moved()), scope: .allInSeries, notify: .none)
    let sent = await masterRequests(h)
    #expect(sent.map(\.method) == ["GET", "PATCH"])
}

@Test func aSeriesWideTitleChangeNeedsNoMasterFetch() async throws {
    let h = try await Harness()
    await h.transport.route("\(calPath)/master1", [.json(googleEvent(id: "master1", etag: "m2", summary: "All"))])
    _ = try await h.source.update(laterOccurrence(), EventPatch(title: "All"), scope: .allInSeries, notify: .none)
    #expect(await masterRequests(h).map(\.method) == ["PATCH"])
}

@Test func aTimeChangeToASingleInstanceOrANonRecurringEventIsNotRestricted() async throws {
    let h = try await Harness()
    await h.transport.route("\(calPath)/master1_20260921T100000Z", [.json(googleEvent(id: "master1_20260921T100000Z", etag: "i2"))])
    _ = try await h.source.update(laterOccurrence(), EventPatch(timing: moved()), scope: .thisInstance, notify: .none)
    await h.transport.route("\(calPath)/ev1", [.json(googleEvent(id: "ev1", etag: "e2"))])
    _ = try await h.source.update(EventRef(calendarID: cal, eventID: "ev1", version: "e1"), EventPatch(timing: moved()), scope: .allInSeries, notify: .none)
    #expect(await h.transport.requests.filter { $0.method == "GET" && $0.url.path.contains("/events/") }.isEmpty)
}

@Test func deleteAndRespondWithSeriesScopeTargetTheMaster() async throws {
    let h = try await Harness()
    let attendees: [[String: Any]] = [["email": "me@x.com", "self": true, "responseStatus": "needsAction"]]
    await h.transport.route("\(calPath)/master1", [HTTPResponse(status: 204)])
    try await h.source.delete(laterOccurrence(), scope: .allInSeries, notify: .none)
    #expect(await masterRequests(h).map(\.method) == ["DELETE"])
    await h.transport.route("\(calPath)/master1", [.json(googleEvent(id: "master1", etag: "m1", extra: ["attendees": attendees])), .json(googleEvent(id: "master1", etag: "m2"))])
    _ = try await h.source.respond(to: laterOccurrence(), .tentative, scope: .allInSeries, notify: .none)
    #expect(await masterRequests(h).map(\.method) == ["DELETE", "GET", "PATCH"])
    #expect(await h.transport.requests(matching: "master1_2026").isEmpty)
}

@Test func aStaleAttendeeEditWithNoOverlapMergesOnTheFreshEtag() async throws {
    let h = try await Harness()
    var edit = EventEdit(base(version: "e1"))
    edit.event.attendees = [Attendee(email: "dee@x.com", role: .required)]
    let current = googleEvent(id: "ev1", etag: "e2", extra: ["attendees": [["email": "me@x.com", "self": true]]])
    await h.transport.route("\(calPath)/ev1", [.json(current), .json(current), .json(current), .json(googleEvent(id: "ev1", etag: "e3"))])
    _ = try await h.source.update(EventRef(base(version: "e1")), edit.patch, scope: .thisInstance, notify: .none)
    let sent = await h.transport.requests(matching: "\(calPath)/ev1")
    #expect(sent.map(\.method) == ["GET", "GET", "GET", "PATCH"] && sent[3].headers["If-Match"] == "e2")
    let merged = try #require(bodyJSON(sent[3])["attendees"] as? [[String: Any]])
    #expect(merged.map { $0["email"] as? String } == ["me@x.com", "dee@x.com"])
}

@Test func aPreconditionFailedOnTheAttendeePatchIsRetried() async throws {
    let h = try await Harness()
    var edit = EventEdit(base(version: "e1"))
    edit.event.attendees = [Attendee(email: "dee@x.com", role: .required)]
    let first = googleEvent(id: "ev1", etag: "e1", extra: ["attendees": [["email": "me@x.com", "self": true]]])
    let second = googleEvent(id: "ev1", etag: "e2", extra: ["attendees": [["email": "me@x.com", "self": true]]])
    await h.transport.route("\(calPath)/ev1", [
        .json(first), googleError("conditionNotMet", status: 412), .json(second), .json(second), .json(googleEvent(id: "ev1", etag: "e3")),
    ])
    _ = try await h.source.update(EventRef(base(version: "e1")), edit.patch, scope: .thisInstance, notify: .none)
    let sent = await h.transport.requests(matching: "\(calPath)/ev1")
    #expect(sent.map(\.method) == ["GET", "PATCH", "GET", "GET", "PATCH"] && sent[1].headers["If-Match"] == "e1" && sent[4].headers["If-Match"] == "e2")
}

// MARK: Final review: a series master is not a single event

private let masterMessage = "this is a recurring series; use .allInSeries or read the occurrence first"
private let weekly = RecurrenceRule(frequency: .weekly, end: .count(4))

/// Creates a recurring event (the reply is the series master) and returns the ref a caller would build from it.
private func createdSeries(_ h: Harness) async throws -> (event: CalendarEvent, ref: EventRef) {
    await h.transport.route(calPath, [.json(googleEvent(id: "new1", etag: "e9", extra: ["recurrence": ["RRULE:FREQ=WEEKLY;COUNT=4"]]))])
    let event = try await h.source.create(EventDraft(title: "Standup", timing: timing(), recurrence: weekly), in: cal, notify: .none)
    return (event, EventRef(event))
}

@Test func creatingARecurringEventReturnsTheMasterAsASeries() async throws {
    let h = try await Harness()
    let (event, ref) = try await createdSeries(h)
    #expect(event.eventID == "new1" && event.seriesID == "new1" && event.originalStart == event.start)
    #expect(ref.seriesID == "new1" && ref.originalStart == instant("2026-09-21T10:00:00Z"))
}

@Test func anEventWithoutARecurrenceIsNotASeriesInWriteResults() async throws {
    let h = try await Harness()
    await h.transport.route(calPath, [.json(googleEvent(id: "new1")), .json(googleEvent(id: "new2", extra: ["recurrence": []]))])
    let plain = try await h.source.create(EventDraft(title: "T", timing: timing()), in: cal, notify: .none)
    #expect(plain.seriesID == nil && plain.originalStart == nil)
    let empty = try await h.source.create(EventDraft(title: "T", timing: timing()), in: cal, notify: .none)
    #expect(empty.seriesID == nil && empty.originalStart == nil)
}

@Test func aThisInstanceWriteOnASeriesMasterIsRefusedBeforeAnyRequest() async throws {
    let h = try await Harness()
    let (_, ref) = try await createdSeries(h)
    let before = await h.transport.requests.count
    await expectWriteError(.invalid(masterMessage)) { try await h.source.delete(ref, scope: .thisInstance, notify: .none) }
    await expectWriteError(.invalid(masterMessage)) { _ = try await h.source.update(ref, EventPatch(title: "X"), scope: .thisInstance, notify: .none) }
    await expectWriteError(.invalid(masterMessage)) { _ = try await h.source.respond(to: ref, .accepted, scope: .thisInstance, notify: .none) }
    #expect(await h.transport.requests.count == before)   // no DELETE, PATCH or even a lookup
}

@Test func seriesWideScopesStillWorkOnASeriesMasterRef() async throws {
    let h = try await Harness()
    let (_, ref) = try await createdSeries(h)
    await h.transport.route("\(calPath)/new1", [
        .json(googleEvent(id: "new1", etag: "e10", summary: "All", extra: ["recurrence": ["RRULE:FREQ=WEEKLY;COUNT=4"]])),
        HTTPResponse(status: 204),
    ])
    let updated = try await h.source.update(ref, EventPatch(title: "All"), scope: .allInSeries, notify: .none)
    #expect(updated.seriesID == "new1" && updated.title == "All")
    try await h.source.delete(ref, scope: .allInSeries, notify: .none)
    let sent = await h.transport.requests(matching: "\(calPath)/new1")
    #expect(sent.map(\.method) == ["PATCH", "DELETE"])
    // The ref's version IS the master's, so the series-wide update keeps the lock (an instance ref sends none, see above).
    #expect(sent[0].headers["If-Match"] == ref.version && ref.version == "e9")
}

@Test func thisAndFollowingAtTheMasterStartIsTheWholeSeries() async throws {
    let h = try await Harness()
    let (_, ref) = try await createdSeries(h)
    await h.transport.route("\(calPath)/new1", [
        .json(googleEvent(id: "new1", etag: "e9", extra: ["recurrence": ["RRULE:FREQ=WEEKLY;COUNT=4"]])), HTTPResponse(status: 204),
    ])
    try await h.source.delete(ref, scope: .thisAndFollowing, notify: .none)
    let sent = await h.transport.requests(matching: "\(calPath)/new1")
    #expect(sent.map(\.method) == ["GET", "DELETE"])
    #expect(await h.transport.requests(matching: "\(calPath)?").filter { $0.method == "POST" }.count == 1)   // only the create
}

// MARK: Final review: a recurrence change needs the whole series

@Test func aRecurrenceChangeOnASingleOccurrenceIsRefusedBeforeAnyRequest() async throws {
    let h = try await Harness()
    let occurrence = EventRef(calendarID: cal, eventID: "m1_20260921T100000Z", version: "i1", seriesID: "m1", originalStart: instant("2026-09-21T10:00:00Z"))
    for change in [FieldUpdate<RecurrenceRule>.clear, .set(weekly)] {
        await expectWriteError(.unsupported(fields: [.recurrence])) {
            _ = try await h.source.update(occurrence, EventPatch(recurrence: change), scope: .thisInstance, notify: .none)
        }
    }
    #expect(await h.transport.requests.isEmpty)
    // Other scopes and non-series events are not affected by the check.
    await h.transport.route("\(calPath)/m1", [.json(googleEvent(id: "m1", etag: "m2"))])
    _ = try await h.source.update(occurrence, EventPatch(recurrence: .clear), scope: .allInSeries, notify: .none)
    await h.transport.route("\(calPath)/ev1", [.json(googleEvent(id: "ev1", etag: "e2"))])
    _ = try await h.source.update(EventRef(calendarID: cal, eventID: "ev1", version: "e1"), EventPatch(timing: timing(), recurrence: .set(weekly)), scope: .thisInstance, notify: .none)
}

// Copying a meeting that may already be on the calendar (Issue 11).

private func draftWithUID(_ uid: String?) -> EventDraft {
    EventDraft(title: "Sync", timing: timing(), uid: uid)
}

@Test func createWithAUIDThatIsAlreadyOnTheCalendarThrowsAlreadyExistsAndInsertsNothing() async throws {
    let h = try await Harness()
    await h.transport.route(calPath, [.json(["items": [googleEvent(id: "there", etag: "e5", extra: ["iCalUID": "u-1"])]])])
    do {
        _ = try await h.source.create(draftWithUID("u-1"), in: cal, notify: .none)
        Issue.record("expected alreadyExists")
    } catch WriteError.alreadyExists(let existing) {
        #expect(existing.eventID == "there" && existing.version == "e5" && existing.calendarID == cal)
    }
    let requests = await h.transport.requests(matching: calPath)
    #expect(requests.count == 1 && requests[0].method == "GET" && requests[0].url.absoluteString.contains("iCalUID=u-1"))
}

@Test func createWithAnUnknownUIDLooksThenInsertsWithTheICalUID() async throws {
    let h = try await Harness()
    await h.transport.route(calPath, [.json(["items": []]), .json(googleEvent(id: "new", extra: ["iCalUID": "u-2"]))])
    let created = try await h.source.create(draftWithUID("u-2"), in: cal, notify: .none)
    let requests = await h.transport.requests(matching: calPath)
    #expect(requests.map(\.method) == ["GET", "POST"])
    #expect(bodyJSON(requests[1])["iCalUID"] as? String == "u-2")
    #expect(created.eventID == "new" && created.uid == "u-2")
}

@Test func createWithoutAUIDMakesNoLookup() async throws {
    let h = try await Harness()
    await h.transport.route(calPath, [.json(googleEvent(id: "new"))])
    _ = try await h.source.create(draftWithUID(nil), in: cal, notify: .none)
    #expect(await h.transport.requests(matching: calPath).map(\.method) == ["POST"])
}

@Test func aCancelledMatchIsIgnored() async throws {
    let h = try await Harness()
    await h.transport.route(calPath, [.json(["items": [googleEvent(id: "gone", extra: ["status": "cancelled", "iCalUID": "u-3"])]]), .json(googleEvent(id: "new"))])
    let created = try await h.source.create(draftWithUID("u-3"), in: cal, notify: .none)
    #expect(created.eventID == "new")
}

@Test func copyingAnEventKitShapedEventCreatesACompleteGoogleEvent() async throws {
    let h = try await Harness()
    var source = CalendarEvent(eventID: "x", uid: "u-4", uidScope: .global, calendarID: "ek", title: "Sync", start: instant("2026-09-21T10:00:00Z"),
                               end: instant("2026-09-21T10:30:00Z"), timeZone: utc)
    let draft = EventDraft(copying: source, for: h.source.capabilities)
    #expect(draft.uid == "u-4" && draft.availability == .busy && draft.visibility == .default && draft.reminders == nil)
    await h.transport.route(calPath, [.json(["items": []]), .json(googleEvent(id: "new", extra: ["iCalUID": "u-4", "reminders": ["useDefault": true]]))])
    let created = try await h.source.create(draft, in: cal, notify: .none)
    let capabilities = SourceCapabilities(providedFields: [.kind, .visibility, .availability, .reminders, .series, .participation, .version])
    #expect(ProvidedFieldsConformance.violations(event: created, capabilities: capabilities).isEmpty)
    source.uidScope = .provider   // an Exchange-style id is never sent as an iCalendar UID
    #expect(EventDraft(copying: source, for: h.source.capabilities).uid == nil)
}

// Setting a series' rule must not drop its skipped and extra dates.

@Test func settingASeriesRuleKeepsTheMastersExdateAndRdateLines() async throws {
    let h = try await Harness()
    let master = googleEvent(id: "master1", extra: ["start": masterStart, "end": masterEnd,
        "recurrence": ["RRULE:FREQ=WEEKLY", "EXDATE;TZID=UTC:20260921T100000", "RDATE;TZID=UTC:20261001T100000"]])
    await h.transport.route("\(calPath)/master1", [.json(master), .json(googleEvent(id: "master1", etag: "m2"))])
    var change = EventPatch(from: base(), to: base())   // the base gives the series' zone
    change.recurrence = .set(RecurrenceRule(frequency: .daily))
    _ = try await h.source.update(laterOccurrence(), change, scope: .allInSeries, notify: .none)
    let patch = try #require(await masterRequests(h).first { $0.method == "PATCH" })
    #expect(bodyJSON(patch)["recurrence"] as? [String] == ["RRULE:FREQ=DAILY", "EXDATE;TZID=UTC:20260921T100000", "RDATE;TZID=UTC:20261001T100000"])
}
