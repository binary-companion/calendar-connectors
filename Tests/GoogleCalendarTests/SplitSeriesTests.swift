import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import GoogleCalendar

private let cal = "me@x.com"
private let calPath = "calendars/me%40x.com/events"
private let tokyo = TimeZone(identifier: "Asia/Tokyo")!
private func instant(_ s: String) -> Date { ISO8601DateFormatter().date(from: s)! }
private let split = "2026-09-15T15:00:00Z"   // the third weekly occurrence

private func master(recurrence: [String] = ["RRULE:FREQ=WEEKLY;COUNT=10"]) -> [String: Any] {
    googleEvent(id: "m1", etag: "em1", summary: "Standup", extra: [
        "start": ["dateTime": "2026-09-01T15:00:00Z", "timeZone": "UTC"], "end": ["dateTime": "2026-09-01T15:30:00Z", "timeZone": "UTC"],
        "description": "notes", "location": "Room", "recurrence": recurrence, "colorId": "5",
        "attendees": [["email": "me@x.com", "self": true, "organizer": true, "responseStatus": "accepted"], ["email": "bob@x.com", "responseStatus": "accepted"]],
        "conferenceData": ["conferenceSolution": ["key": ["type": "hangoutsMeet"]], "conferenceId": "abc"],
        "reminders": ["useDefault": false, "overrides": [["method": "popup", "minutes": 10]]],
        "htmlLink": "https://x", "iCalUID": "u1", "sequence": 3,
        "attachments": [["fileUrl": "https://drive.example/notes", "title": "Notes"]],
    ])
}
private func instanceJSON() -> [String: Any] {
    googleEvent(id: "m1_20260915T150000Z", etag: "ei3", summary: "Standup", extra: [
        "start": ["dateTime": split, "timeZone": "UTC"], "end": ["dateTime": "2026-09-15T15:30:00Z", "timeZone": "UTC"],
        "recurringEventId": "m1", "originalStartTime": ["dateTime": split]])
}
private func instancesPage() -> [String: Any] {
    let starts = (0..<10).map { i in ["originalStartTime": ["dateTime": ISO8601DateFormatter().string(from: instant("2026-09-01T15:00:00Z").addingTimeInterval(Double(i) * 604_800))]] }
    return ["items": starts]
}
private var ref: EventRef {
    EventRef(calendarID: cal, eventID: "m1_20260915T150000Z", version: "ei3", seriesID: "m1", originalStart: instant(split))
}

/// Answers `hook`'s response instead of the routed one when it returns non-nil; the routed transport still records the request.
private struct Intercept: HTTPTransport {
    let inner: FakeTransport
    let hook: @Sendable (HTTPRequest) -> HTTPResponse?
    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        let routed = try await inner.send(request)
        return hook(request) ?? routed
    }
}

private func harness(
    masterResponses: [HTTPResponse], post: HTTPResponse = .json(googleEvent(id: "n1", etag: "en1", summary: "Standup")),
    hook: (@Sendable (HTTPRequest) -> HTTPResponse?)? = nil
) async throws -> Harness {
    var wrap: (@Sendable (FakeTransport) -> any HTTPTransport)?
    if let hook { wrap = { (inner: FakeTransport) -> any HTTPTransport in Intercept(inner: inner, hook: hook) } }
    let h = try await Harness(wrap: wrap)
    await h.transport.route("\(calPath)/m1", masterResponses)
    await h.transport.route("\(calPath)/m1_2026", [.json(instanceJSON())])
    await h.transport.route("\(calPath)/m1/instances", [.json(instancesPage())])
    await h.transport.route("\(calPath)?", [post])
    return h
}

private func masterWrites(_ h: Harness) async -> [HTTPRequest] { await h.transport.requests(matching: "\(calPath)/m1?") }

/// Remembers the id the insert was sent with, so a lookup hook can insist on it.
private final class SentID: @unchecked Sendable {
    private let lock = NSLock()
    private var id: String?
    var value: String? { get { lock.withLock { id } } set { lock.withLock { id = newValue } } }
}

/// Answers `answer` to a GET of exactly the id the POST carried (and nothing else).
private func lookup(_ sent: SentID, answer: HTTPResponse) -> @Sendable (HTTPRequest) -> HTTPResponse? {
    { request in
        if request.method == "POST" { sent.value = bodyJSON(request)["id"] as? String; return nil }
        if request.method == "GET", let id = sent.value, request.url.absoluteString.hasSuffix("/events/\(id)") { return answer }
        return nil
    }
}

private func partialMessage(_ body: () async throws -> Void) async -> String? {
    do { try await body() } catch WriteError.partial(let message) { return message } catch { Issue.record("expected .partial, got \(error)") }
    return nil
}

// MARK: Pure helpers

@Test func truncationCutsTheRuleJustBeforeTheSplitAndKeepsOtherLines() {
    let lines = ["RRULE:FREQ=WEEKLY;COUNT=10;BYDAY=TU", "EXDATE:20260908T150000Z"]
    #expect(GoogleWriteMapper.truncated(lines, before: instant(split), allDay: false, zone: nil)
        == ["RRULE:FREQ=WEEKLY;BYDAY=TU;UNTIL=20260915T145959Z", "EXDATE:20260908T150000Z"])
    let allDay = GoogleWriteMapper.truncated(["RRULE:FREQ=DAILY;UNTIL=20261231"], before: instant("2026-09-14T15:00:00Z"), allDay: true, zone: tokyo)
    #expect(allDay == ["RRULE:FREQ=DAILY;UNTIL=20260914"])   // split is Sep 15 in Tokyo; the last kept day is Sep 14
}

@Test func truncationMovesExceptionAndAddedDatesFromTheSplitOnToTheNewSeries() {
    let lines = [
        "RRULE:FREQ=WEEKLY;COUNT=10", "EXDATE:20260908T150000Z,20260922T150000Z", "RDATE:20260910T150000Z,20260917T150000Z",
        "EXDATE;TZID=Asia/Tokyo:20260916T000000", "RDATE:20260920T150000Z", "EXDATE;VALUE=DATE:20260901",
    ]
    // Sep 16 00:00 in Tokyo is Sep 15 15:00Z, exactly the split, so that EXDATE moves; a line left with no values is dropped.
    #expect(GoogleWriteMapper.truncated(lines, before: instant(split), allDay: false, zone: TimeZone(identifier: "UTC")!) == [
        "RRULE:FREQ=WEEKLY;UNTIL=20260915T145959Z", "EXDATE:20260908T150000Z", "RDATE:20260910T150000Z", "EXDATE;VALUE=DATE:20260901",
    ])
    #expect(GoogleWriteMapper.carriedOver(lines, from: instant(split), zone: TimeZone(identifier: "UTC")!) == [
        "EXDATE:20260922T150000Z", "RDATE:20260917T150000Z", "EXDATE;TZID=Asia/Tokyo:20260916T000000", "RDATE:20260920T150000Z",
    ])
}

@Test func countHelpersReadAndReplaceCount() {
    #expect(GoogleWriteMapper.count(in: ["RRULE:FREQ=WEEKLY;COUNT=10"]) == 10)
    #expect(GoogleWriteMapper.count(in: ["RRULE:FREQ=WEEKLY;UNTIL=20261231"]) == nil)
    #expect(GoogleWriteMapper.replacingCount(["RRULE:FREQ=WEEKLY;COUNT=10", "EXDATE:x"], with: 8) == ["RRULE:FREQ=WEEKLY;COUNT=8", "EXDATE:x"])
    #expect(GoogleWriteMapper.rruleLines(["RRULE:FREQ=WEEKLY", "EXDATE:x"]) == ["RRULE:FREQ=WEEKLY"])
}

@Test func onlyExceptionDatesFromTheSplitOnCarryToTheNewSeries() {
    let lines = [
        "RRULE:FREQ=WEEKLY", "EXDATE:20260908T150000Z,20260922T150000Z", "EXDATE;TZID=Asia/Tokyo:20260901T000000",
        "EXDATE;VALUE=DATE:20260914,20260930", "RDATE:20260920T150000Z",
    ]
    #expect(GoogleWriteMapper.carriedOver(lines, from: instant(split), zone: TimeZone(identifier: "UTC")!) == [
        "EXDATE:20260922T150000Z", "EXDATE;VALUE=DATE:20260930", "RDATE:20260920T150000Z",
    ])
}

@Test func theNewSeriesBodyIsBuiltFromTheMasterWithoutOutputOnlyFields() throws {
    let patch = EventPatch(location: .set("Lab"))
    let body = try GoogleWriteMapper.newSeriesBody(master: master(), slot: instant(split), patch: patch,
                                                   recurrence: ["RRULE:FREQ=WEEKLY;COUNT=8"], fallbackZone: "UTC")
    let json = body.json
    for key in ["id", "etag", "iCalUID", "htmlLink", "sequence", "recurringEventId", "originalStartTime"] { #expect(json[key] == nil, "\(key) must not be copied") }
    #expect(json["summary"] as? String == "Standup" && json["description"] as? String == "notes" && json["colorId"] as? String == "5")
    #expect(json["location"] as? String == "Lab" && json["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;COUNT=8"])
    #expect((json["start"] as? [String: Any])?["dateTime"] as? String == split)
    let attendees = try #require(json["attendees"] as? [[String: Any]])
    #expect(attendees.allSatisfy { $0["responseStatus"] == nil && $0["self"] == nil && $0["organizer"] == nil })
    let request = ((json["conferenceData"] as? [String: Any])?["createRequest"] as? [String: Any])
    #expect(request != nil && body.needsConferenceVersion)   // the old Meet link is not copied; a new one is requested
    #expect(json["reminders"] != nil)
}

@Test func aPatchThatRemovesTheConferenceDoesNotRequestANewOne() throws {
    let body = try GoogleWriteMapper.newSeriesBody(master: master(), slot: instant(split), patch: EventPatch(conference: .remove),
                                                   recurrence: ["RRULE:FREQ=WEEKLY"], fallbackZone: "UTC")
    #expect(body.json["conferenceData"] == nil)
}

@Test func aTimingPatchOnTheNewSeriesCarriesNoNullsInsideStartAndEnd() throws {
    let allDay = EventTiming(start: instant("2026-09-15T00:00:00Z"), end: instant("2026-09-16T00:00:00Z"), timeZone: TimeZone(identifier: "UTC")!, isAllDay: true)
    let body = try GoogleWriteMapper.newSeriesBody(master: master(), slot: instant(split), patch: EventPatch(timing: allDay),
                                                   recurrence: ["RRULE:FREQ=WEEKLY"], fallbackZone: "UTC")
    let start = try #require(body.json["start"] as? [String: Any])
    #expect(start["date"] as? String == "2026-09-15" && start["dateTime"] == nil && start["timeZone"] == nil)
    #expect(!(start.values.contains { $0 is NSNull }))
}

// MARK: Flows

@Test func updateThisAndFollowingTruncatesTheMasterThenInsertsTheNewSeries() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(master())])
    let event = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .all)
    let truncate = try #require(await masterWrites(h).last)
    #expect(truncate.method == "PATCH" && truncate.headers["If-Match"] == "em1" && truncate.url.absoluteString.contains("sendUpdates=all"))
    #expect(bodyJSON(truncate)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;UNTIL=20260915T145959Z"])
    let post = try #require(await h.transport.requests(matching: "\(calPath)?").last)
    #expect(post.method == "POST" && post.url.absoluteString.contains("sendUpdates=all"))
    // The master has attachments and a Meet link: the insert must be allowed to keep the first and re-request the second.
    #expect(post.url.absoluteString.contains("supportsAttachments=true") && post.url.absoluteString.contains("conferenceDataVersion=1"))
    #expect((bodyJSON(post)["attachments"] as? [[String: Any]])?.count == 1)
    #expect(bodyJSON(post)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;COUNT=8"] && bodyJSON(post)["location"] as? String == "Lab")
    #expect(event.eventID == "n1")
}

@Test func everyReadAndCheckHappensBeforeTheMasterIsTouched() async throws {
    // The occurrence cannot be read, so nothing may have been cut off.
    let h = try await harness(masterResponses: [.json(master()), .json(master())])
    await h.transport.route("\(calPath)/m1_2026", [.json([:], status: 404)])
    await expectWriteError(.notFound) { _ = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none) }
    #expect(await masterWrites(h).isEmpty)
    // The earlier occurrences cannot be counted, so nothing may have been cut off either.
    let h2 = try await harness(masterResponses: [.json(master()), .json(master())])
    await h2.transport.route("\(calPath)/m1/instances", [.json([:], status: 500)])
    await #expect(throws: SourceError.server(status: 500)) {
        _ = try await h2.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none)
    }
    #expect(await masterWrites(h2).isEmpty)
}

@Test func theNewSeriesIsInsertedWithAClientIDSoALostReplyCanBeChecked() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(master())])
    _ = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none)
    let post = try #require(await h.transport.requests(matching: "\(calPath)?").last)
    let id = try #require(bodyJSON(post)["id"] as? String)
    #expect(id.count == 32 && id.allSatisfy { "0123456789abcdef".contains($0) })   // base32hex, as Google requires
}

@Test func splittingAtTheFirstOccurrenceIsTheWholeSeries() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(googleEvent(id: "m1", etag: "em2", summary: "All"))])
    let first = EventRef(calendarID: cal, eventID: "m1_20260901T150000Z", version: "i1", seriesID: "m1", originalStart: instant("2026-09-01T15:00:00Z"))
    _ = try await h.source.update(first, EventPatch(title: "All"), scope: .thisAndFollowing, notify: .none)
    let last = try #require(await masterWrites(h).last)
    #expect(last.method == "PATCH" && last.headers["If-Match"] == nil && bodyJSON(last)["summary"] as? String == "All")
    #expect(await h.transport.requests(matching: "\(calPath)?").isEmpty)
}

@Test func splittingAtTheFirstOccurrenceMayChangeTheTimeOfTheWholeSeries() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(master()), .json(googleEvent(id: "m1", etag: "em2", summary: "All"))])
    let first = EventRef(calendarID: cal, eventID: "m1_20260901T150000Z", version: "i1", seriesID: "m1", originalStart: instant("2026-09-01T15:00:00Z"))
    let timing = EventTiming(start: instant("2026-09-01T16:00:00Z"), end: instant("2026-09-01T16:30:00Z"), timeZone: TimeZone(identifier: "UTC")!, isAllDay: false)
    _ = try await h.source.update(first, EventPatch(timing: timing), scope: .thisAndFollowing, notify: .none)
    let last = try #require(await masterWrites(h).last)
    #expect(last.method == "PATCH" && (bodyJSON(last)["start"] as? [String: Any])?["dateTime"] as? String == "2026-09-01T16:00:00Z")
}

@Test(arguments: [(NotifyPolicy.all, "all"), (.externalOnly, "externalOnly"), (.none, "none")])
func theTruncationAndTheInsertBothCarryTheCallersPolicy(policy: NotifyPolicy, wire: String) async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(master())])
    _ = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: policy)
    let truncate = try #require(await masterWrites(h).last)
    #expect(truncate.url.absoluteString.contains("sendUpdates=\(wire)"))
    let post = try #require(await h.transport.requests(matching: "\(calPath)?").last)
    #expect(post.url.absoluteString.contains("sendUpdates=\(wire)"))
}

@Test(arguments: [(NotifyPolicy.all, "all"), (.externalOnly, "externalOnly"), (.none, "none")])
func aFailedInsertRestoresTheOriginalRuleWithTheCallersPolicy(policy: NotifyPolicy, wire: String) async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(master()), .json(master())], post: googleError("invalid", message: "bad start", status: 400))
    await expectWriteError(.invalid("bad start")) { _ = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: policy) }
    let writes = await masterWrites(h)
    #expect(writes.count == 2 && writes[0].url.absoluteString.contains("sendUpdates=\(wire)"))
    let restore = try #require(writes.last)
    #expect(restore.headers["If-Match"] == nil && restore.url.absoluteString.contains("sendUpdates=\(wire)"))
    #expect(bodyJSON(restore)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;COUNT=10"])
}

@Test func aFailedInsertRestoresTheOriginalRule() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(master()), .json(master())], post: googleError("invalid", message: "bad start", status: 400))
    await expectWriteError(.invalid("bad start")) { _ = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none) }
    let restore = try #require(await masterWrites(h).last)
    #expect(restore.method == "PATCH" && restore.headers["If-Match"] == nil && restore.url.absoluteString.contains("sendUpdates=none"))
    #expect(bodyJSON(restore)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;COUNT=10"])
}

@Test func aFailedInsertAndAFailedRestoreIsReportedAsPartial() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(master()), HTTPResponse(status: 500)],
                              post: googleError("invalid", message: "bad start", status: 400))
    let message = await partialMessage { _ = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none) }
    #expect(message?.contains("bad start") == true)   // the insert's own error is not lost
}

@Test func theRestoreStillRunsWhenTheCallerIsCancelledDuringTheInsert() async throws {
    let h = try await harness(
        masterResponses: [.json(master()), .json(master()), .json(master())],
        post: googleError("invalid", message: "bad start", status: 400),
        hook: { request in
            guard request.method == "POST" else { return nil }
            withUnsafeCurrentTask { $0?.cancel() }
            return googleError("invalid", message: "bad start", status: 400)
        })
    await expectWriteError(.invalid("bad start")) { _ = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none) }
    let restore = try #require(await masterWrites(h).last)
    #expect(restore.method == "PATCH" && bodyJSON(restore)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;COUNT=10"])
}

@Test func aLostInsertReplyThatDidApplyIsNotRolledBack() async throws {
    // The POST answers 500 but the event exists under the id we chose: the split is complete, so keep it.
    let sent = SentID()
    let h = try await harness(
        masterResponses: [.json(master()), .json(master()), .json(master())], post: HTTPResponse(status: 500),
        hook: lookup(sent, answer: .json(googleEvent(id: "n1", etag: "en1", summary: "Standup"))))
    let event = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none)
    #expect(event.eventID == "n1")
    let writes = await masterWrites(h)
    #expect(writes.count == 1 && bodyJSON(writes[0])["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;UNTIL=20260915T145959Z"])
}

@Test func aConflictOnTheInsertMeansOurOwnRequestAlreadyApplied() async throws {
    // The id is ours alone, so a 409 (the typed `GoogleAPIError.conflict`) can only be an earlier attempt of this same POST.
    let sent = SentID()
    let h = try await harness(
        masterResponses: [.json(master()), .json(master()), .json(master())], post: googleError("duplicate", message: "exists", status: 409),
        hook: lookup(sent, answer: .json(googleEvent(id: "n1", etag: "en1", summary: "Standup"))))
    let event = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none)
    #expect(event.eventID == "n1")
    #expect(await masterWrites(h).count == 1)
}

@Test func aConflictOnTheInsertThatFindsNothingIsRolledBackAndReportedAsUnexpected() async throws {
    let sent = SentID()
    let h = try await harness(
        masterResponses: [.json(master()), .json(master()), .json(master())], post: googleError("duplicate", message: "exists", status: 409),
        hook: lookup(sent, answer: googleError("notFound", status: 404)))
    await #expect(throws: SourceError.invalidResponse("google: unexpected 409")) {
        _ = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none)
    }
    let restore = try #require(await masterWrites(h).last)
    #expect(bodyJSON(restore)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;COUNT=10"])
}

@Test func aLookupThatFailsAfterALostReplyLeavesTheMasterAloneAndReportsPartial() async throws {
    let sent = SentID()
    let h = try await harness(
        masterResponses: [.json(master()), .json(master()), .json(master())], post: HTTPResponse(status: 500),
        hook: lookup(sent, answer: HTTPResponse(status: 503)))
    let message = await partialMessage { _ = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none) }
    #expect(message?.contains("unknown") == true && message?.contains("503") == true && message?.contains("500") == true)
    #expect(await masterWrites(h).count == 1)   // only the truncation: no restore that could duplicate the series
}

@Test func aLostInsertReplyThatDidNotApplyIsRolledBack() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(master()), .json(master())], post: HTTPResponse(status: 500))
    await #expect(throws: SourceError.server(status: 500)) {
        _ = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none)
    }
    let restore = try #require(await masterWrites(h).last)
    #expect(bodyJSON(restore)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;COUNT=10"])
}

@Test func anInsertThatSucceededIsNeverRolledBackEvenIfItsReplyCannotBeRead() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(master()), .json(master())], post: .json([:]))
    await #expect(throws: SourceError.self) {
        _ = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none)
    }
    #expect(await masterWrites(h).count == 1)   // only the truncation
}

@Test func anUnreadableReplyToASuccessfulInsertIsRecoveredByLookingTheEventUp() async throws {
    let sent = SentID()
    let h = try await harness(
        masterResponses: [.json(master()), .json(master()), .json(master())], post: .json([:]),
        hook: lookup(sent, answer: .json(googleEvent(id: "n1", etag: "en1", summary: "Standup"))))
    let event = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none)
    #expect(event.eventID == "n1")
    #expect(await masterWrites(h).count == 1)
}

@Test func aStaleMasterIsAConflictAndNothingIsInserted() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json([:], status: 412)])
    await expectWriteError(.conflict(fields: [.recurrence])) { _ = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none) }
    #expect(await masterWrites(h).count == 1)
    #expect(await h.transport.requests(matching: "\(calPath)?").filter { $0.method == "POST" }.isEmpty)
}

// MARK: A lost reply to the truncation

@Test func aTruncationWhoseReplyIsLostIsRestoredAndTheOriginalErrorRethrown() async throws {
    // The PATCH may have been applied. Restore the original rule so a retry is safe (it would otherwise lose the EXDATE and
    // RDATE values the first truncation moved), and never insert.
    let h = try await harness(masterResponses: [.json(master()), HTTPResponse(status: 502), .json(master())])
    await #expect(throws: SourceError.server(status: 502)) {
        _ = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .all)
    }
    let writes = await masterWrites(h)
    #expect(writes.count == 2)
    #expect(bodyJSON(writes[0])["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;UNTIL=20260915T145959Z"])
    #expect(writes[0].url.absoluteString.contains("sendUpdates=all"))
    let restore = try #require(writes.last)
    // Guests told about the truncation hear about its reversal too.
    #expect(restore.method == "PATCH" && restore.headers["If-Match"] == nil && restore.url.absoluteString.contains("sendUpdates=all"))
    #expect(bodyJSON(restore)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;COUNT=10"])
    #expect(await h.transport.requests(matching: "\(calPath)?").filter { $0.method == "POST" }.isEmpty)
}

@Test func aLostTruncationReplyAndAFailedRestoreIsReportedAsPartial() async throws {
    let h = try await harness(masterResponses: [.json(master()), HTTPResponse(status: 502), HTTPResponse(status: 503)])
    let message = await partialMessage { _ = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none) }
    #expect(message?.contains("502") == true && message?.contains("503") == true)   // both errors are named
    #expect(await h.transport.requests(matching: "\(calPath)?").filter { $0.method == "POST" }.isEmpty)
}

@Test func theRestoreAfterALostTruncationReplyStillRunsWhenTheCallerIsCancelled() async throws {
    let h = try await harness(
        masterResponses: [.json(master()), HTTPResponse(status: 502), .json(master())],
        hook: { request in
            guard request.method == "PATCH", request.url.path.hasSuffix("/m1"), bodyJSON(request)["recurrence"] as? [String] != ["RRULE:FREQ=WEEKLY;COUNT=10"] else { return nil }
            withUnsafeCurrentTask { $0?.cancel() }
            return nil
        })
    _ = try? await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none)
    let restore = try #require(await masterWrites(h).last)
    #expect(restore.method == "PATCH" && bodyJSON(restore)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;COUNT=10"])
}

@Test func aDefiniteTruncationFailureIsNotRestored() async throws {
    let refused = try await harness(masterResponses: [.json(master()), googleError("invalid", message: "bad rule", status: 400)])
    await expectWriteError(.invalid("bad rule")) { _ = try await refused.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none) }
    #expect(await masterWrites(refused).count == 1)
    let forbidden = try await harness(masterResponses: [.json(master()), googleError("forbidden", status: 403)])
    await expectWriteError(.forbidden(nil)) { _ = try await forbidden.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none) }
    #expect(await masterWrites(forbidden).count == 1)
    let gone = try await harness(masterResponses: [.json(master()), .json([:], status: 404)])
    await expectWriteError(.notFound) { _ = try await gone.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none) }
    #expect(await masterWrites(gone).count == 1)
}

@Test func deleteWithALostTruncationReplyKeepsItsPlainErrorAndDoesNotRestore() async throws {
    // For a delete the truncation is the operation and repeating it is idempotent.
    let h = try await harness(masterResponses: [.json(master()), HTTPResponse(status: 502)])
    await #expect(throws: SourceError.server(status: 502)) { try await h.source.delete(ref, scope: .thisAndFollowing, notify: .none) }
    #expect(await masterWrites(h).count == 1)
}

@Test func deleteThisAndFollowingOnlyTruncatesTheMaster() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(master())])
    try await h.source.delete(ref, scope: .thisAndFollowing, notify: .all)
    let truncate = try #require(await masterWrites(h).last)
    #expect(truncate.method == "PATCH" && truncate.url.absoluteString.contains("sendUpdates=all"))
    #expect(bodyJSON(truncate)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;UNTIL=20260915T145959Z"])
    #expect(await h.transport.requests(matching: "\(calPath)?").isEmpty)
}

@Test func deleteThisAndFollowingAtTheFirstOccurrenceDeletesTheWholeSeries() async throws {
    let h = try await harness(masterResponses: [.json(master()), HTTPResponse(status: 204)])
    let first = EventRef(calendarID: cal, eventID: "m1_20260901T150000Z", version: "i1", seriesID: "m1", originalStart: instant("2026-09-01T15:00:00Z"))
    try await h.source.delete(first, scope: .thisAndFollowing, notify: .none)
    let last = try #require(await masterWrites(h).last)
    #expect(last.method == "DELETE")
}

@Test func aRuleWithoutACountIsSplitWithoutListingInstances() async throws {
    let h = try await harness(masterResponses: [.json(master(recurrence: ["RRULE:FREQ=WEEKLY;UNTIL=20261231T000000Z"])), .json(master())])
    _ = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none)
    #expect(await h.transport.requests(matching: "/instances").isEmpty)
    let post = try #require(await h.transport.requests(matching: "\(calPath)?").last)
    #expect(bodyJSON(post)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;UNTIL=20261231T000000Z"])
}

@Test func exceptionAndAddedDatesFollowTheSideOfTheSplitTheyBelongTo() async throws {
    let lines = ["RRULE:FREQ=WEEKLY;UNTIL=20261231T000000Z", "EXDATE:20260908T150000Z,20260922T150000Z", "RDATE:20260910T150000Z,20260917T150000Z"]
    let h = try await harness(masterResponses: [.json(master(recurrence: lines)), .json(master())])
    _ = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none)
    let truncate = try #require(await masterWrites(h).last)
    #expect(bodyJSON(truncate)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;UNTIL=20260915T145959Z", "EXDATE:20260908T150000Z", "RDATE:20260910T150000Z"])
    let post = try #require(await h.transport.requests(matching: "\(calPath)?").last)
    #expect(bodyJSON(post)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;UNTIL=20261231T000000Z", "EXDATE:20260922T150000Z", "RDATE:20260917T150000Z"])
}

@Test func deleteThisAndFollowingRemovesLaterAddedDatesFromTheMaster() async throws {
    let lines = ["RRULE:FREQ=WEEKLY;COUNT=10", "RDATE:20260920T150000Z", "EXDATE:20260908T150000Z,20260922T150000Z"]
    let h = try await harness(masterResponses: [.json(master(recurrence: lines)), .json(master())])
    try await h.source.delete(ref, scope: .thisAndFollowing, notify: .none)
    let truncate = try #require(await masterWrites(h).last)
    #expect(bodyJSON(truncate)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;UNTIL=20260915T145959Z", "EXDATE:20260908T150000Z"])
}

@Test func aPatchThatReplacesTheRuleDoesNotListInstances() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(master())])
    let daily = RecurrenceRule(frequency: .daily, end: .count(3))
    let timing = EventTiming(start: instant(split), end: instant("2026-09-15T15:30:00Z"), timeZone: TimeZone(identifier: "UTC")!, isAllDay: false)
    _ = try await h.source.update(ref, EventPatch(timing: timing, recurrence: .set(daily)), scope: .thisAndFollowing, notify: .none)
    #expect(await h.transport.requests(matching: "/instances").isEmpty)
    let post = try #require(await h.transport.requests(matching: "\(calPath)?").last)
    #expect(bodyJSON(post)["recurrence"] as? [String] == ["RRULE:FREQ=DAILY;COUNT=3"])
}

@Test func aMovedOccurrenceIsCutAtItsOriginalSlotAndTheNewSeriesStartsThereToo() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(master())])
    var moved = instanceJSON()
    // This occurrence was moved an hour later and made longer; the rest of the series was not.
    moved["start"] = ["dateTime": "2026-09-15T16:00:00Z", "timeZone": "UTC"]
    moved["end"] = ["dateTime": "2026-09-15T17:00:00Z", "timeZone": "UTC"]
    await h.transport.route("\(calPath)/m1_2026", [.json(moved)])
    _ = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none)
    let truncate = try #require(await masterWrites(h).last)
    #expect(bodyJSON(truncate)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;UNTIL=20260915T145959Z"])
    // The patch does not touch timing, so the following occurrences keep the series' time and length.
    let post = try #require(await h.transport.requests(matching: "\(calPath)?").last)
    let start = try #require(bodyJSON(post)["start"] as? [String: Any])
    #expect(start["dateTime"] as? String == "2026-09-15T15:00:00Z" && start["timeZone"] as? String == "UTC")
    let end = try #require(bodyJSON(post)["end"] as? [String: Any])
    #expect(end["dateTime"] as? String == "2026-09-15T15:30:00Z" && end["timeZone"] as? String == "UTC")
}

@Test func aMovedAllDayOccurrenceStartsTheNewSeriesOnItsOriginalDayWithTheSeriesLength() async throws {
    let allDayMaster = googleEvent(id: "m1", etag: "em1", extra: [
        "start": ["date": "2026-09-01"], "end": ["date": "2026-09-03"], "recurrence": ["RRULE:FREQ=WEEKLY;UNTIL=20261231"]])
    let allDayInstance = googleEvent(id: "m1_20260915", etag: "ei3", extra: [
        "start": ["date": "2026-09-16"], "end": ["date": "2026-09-19"], "recurringEventId": "m1", "originalStartTime": ["date": "2026-09-15"]])
    let h = try await harness(masterResponses: [.json(allDayMaster), .json(allDayMaster)])
    await h.transport.route("\(calPath)/m1_2026", [.json(allDayInstance)])
    let day = EventRef(calendarID: cal, eventID: "m1_20260915", version: "ei3", seriesID: "m1", originalStart: instant("2026-09-15T00:00:00Z"))
    _ = try await h.source.update(day, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none)
    let post = try #require(await h.transport.requests(matching: "\(calPath)?").last)
    #expect(bodyJSON(post)["start"] as? [String: String] == ["date": "2026-09-15"])
    #expect(bodyJSON(post)["end"] as? [String: String] == ["date": "2026-09-17"])   // the master spans two days
}

@Test func aTimingPatchStillSetsTheNewSeriesTimeWhateverTheSlotWas() throws {
    let timing = EventTiming(start: instant("2026-09-15T18:00:00Z"), end: instant("2026-09-15T19:00:00Z"), timeZone: TimeZone(identifier: "UTC")!, isAllDay: false)
    let body = try GoogleWriteMapper.newSeriesBody(master: master(), slot: instant(split), patch: EventPatch(timing: timing),
                                                   recurrence: ["RRULE:FREQ=WEEKLY"], fallbackZone: "UTC")
    #expect((body.json["start"] as? [String: Any])?["dateTime"] as? String == "2026-09-15T18:00:00Z")
    #expect((body.json["end"] as? [String: Any])?["dateTime"] as? String == "2026-09-15T19:00:00Z")
}

@Test func theNewSeriesKeepsTheMastersOwnZoneNameAndTheFallbackUsesTheWireName() throws {
    // The master's zone name carries over to the slot.
    var berlin = master()
    berlin["start"] = ["dateTime": "2026-09-01T17:00:00+02:00", "timeZone": "Europe/Berlin"]
    berlin["end"] = ["dateTime": "2026-09-01T17:30:00+02:00", "timeZone": "Europe/Berlin"]
    let carried = try GoogleWriteMapper.newSeriesBody(master: berlin, slot: instant(split), patch: EventPatch(title: "X"),
                                                      recurrence: ["RRULE:FREQ=WEEKLY"], fallbackZone: "UTC")
    #expect((carried.json["start"] as? [String: Any])?["timeZone"] as? String == "Europe/Berlin")
    // A patch time without a zone gets the fallback, spelled as the wire format wants ("UTC", not Darwin's "GMT").
    let zoneless = EventTiming(start: instant("2026-09-15T18:00:00Z"), end: instant("2026-09-15T19:00:00Z"), timeZone: nil, isAllDay: false)
    let body = try GoogleWriteMapper.newSeriesBody(master: master(), slot: instant(split), patch: EventPatch(timing: zoneless),
                                                   recurrence: ["RRULE:FREQ=WEEKLY"], fallbackZone: "GMT")
    #expect((body.json["start"] as? [String: Any])?["timeZone"] as? String == "UTC")
    #expect((body.json["end"] as? [String: Any])?["timeZone"] as? String == "UTC")
}

@Test func earlierOccurrencesAreCountedAcrossPagesIncludingCancelledOnes() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(master())])
    func page(_ weeks: [Int], token: String? = nil, cancelled: Set<Int> = []) -> [String: Any] {
        var json: [String: Any] = ["items": weeks.map { week -> [String: Any] in
            var item: [String: Any] = ["originalStartTime": ["dateTime": ISO8601DateFormatter().string(from: instant("2026-09-01T15:00:00Z").addingTimeInterval(Double(week) * 604_800))]]
            if cancelled.contains(week) { item["status"] = "cancelled" }
            return item
        }]
        if let token { json["nextPageToken"] = token }
        return json
    }
    await h.transport.route("\(calPath)/m1/instances", [.json(page([0, 1], token: "p2", cancelled: [1])), .json(page(Array(2..<10)))])
    _ = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none)
    let listings = await h.transport.requests(matching: "/instances")
    #expect(listings.count == 2)
    let first = listings[0].url.absoluteString
    #expect(first.contains("showDeleted=true") && first.contains("maxResults=250") && first.contains("fields="))
    #expect(listings[1].url.absoluteString.contains("pageToken=p2"))
    let post = try #require(await h.transport.requests(matching: "\(calPath)?").last)
    #expect(bodyJSON(post)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;COUNT=8"])   // 10 in all, 2 before the split (one of them cancelled)
}

@Test func anAllDaySeriesIsCutAtTheDayBeforeAndSplitOnDates() async throws {
    let allDayMaster = googleEvent(id: "m1", etag: "em1", extra: [
        "start": ["date": "2026-09-01"], "end": ["date": "2026-09-02"], "recurrence": ["RRULE:FREQ=WEEKLY;UNTIL=20261231"]])
    let allDayInstance = googleEvent(id: "m1_20260915", etag: "ei3", extra: [
        "start": ["date": "2026-09-15"], "end": ["date": "2026-09-16"], "recurringEventId": "m1", "originalStartTime": ["date": "2026-09-15"]])
    let h = try await harness(masterResponses: [.json(allDayMaster), .json(allDayMaster)])
    await h.transport.route("\(calPath)/m1_2026", [.json(allDayInstance)])
    let day = EventRef(calendarID: cal, eventID: "m1_20260915", version: "ei3", seriesID: "m1", originalStart: instant("2026-09-15T00:00:00Z"))
    _ = try await h.source.update(day, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none)
    let truncate = try #require(await masterWrites(h).last)
    #expect(bodyJSON(truncate)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;UNTIL=20260914"])
    let post = try #require(await h.transport.requests(matching: "\(calPath)?").last)
    #expect(bodyJSON(post)["start"] as? [String: String] == ["date": "2026-09-15"] && bodyJSON(post)["end"] as? [String: String] == ["date": "2026-09-16"])
    #expect(bodyJSON(post)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;UNTIL=20261231"])
}

@Test func clearingTheRuleMakesTheRestOfTheSeriesASingleEvent() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(master())])
    _ = try await h.source.update(ref, EventPatch(recurrence: .clear), scope: .thisAndFollowing, notify: .none)
    #expect(await h.transport.requests(matching: "/instances").isEmpty)
    let truncate = try #require(await masterWrites(h).last)
    #expect(bodyJSON(truncate)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;UNTIL=20260915T145959Z"])
    let post = try #require(await h.transport.requests(matching: "\(calPath)?").last)
    #expect(bodyJSON(post)["recurrence"] == nil)
}

// MARK: Staleness

private func staleInstance(location: String? = nil, summary: String = "Standup") -> [String: Any] {
    var json = instanceJSON()
    json["etag"] = "ei9"
    json["summary"] = summary
    if let location { json["location"] = location }
    return json
}

private func editing(location: String) -> EventPatch {
    let base = CalendarEvent(eventID: "m1_20260915T150000Z", calendarID: cal, title: "Standup", start: instant(split), end: instant("2026-09-15T15:30:00Z"),
                             timeZone: TimeZone(identifier: "UTC")!, version: "ei3")
    var edited = base
    edited.location = location
    return EventPatch(from: base, to: edited)
}

@Test func aStaleOccurrenceThatOverlapsThePatchIsAConflictAndNothingIsWritten() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(master())])
    await h.transport.route("\(calPath)/m1_2026", [.json(staleInstance(location: "Elsewhere"))])
    await expectWriteError(.conflict(fields: [.location])) { _ = try await h.source.update(ref, editing(location: "Lab"), scope: .thisAndFollowing, notify: .none) }
    #expect(await masterWrites(h).isEmpty)
    #expect(await h.transport.requests(matching: "\(calPath)?").isEmpty)
}

@Test func aHandBuiltPatchConflictsOnAnyStaleVersion() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(master())])
    await h.transport.route("\(calPath)/m1_2026", [.json(staleInstance())])
    await expectWriteError(.conflict(fields: [.location])) { _ = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none) }
    #expect(await masterWrites(h).isEmpty)
}

@Test func aStaleOccurrenceThatOnlyChangedOtherFieldsStillSplits() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(master())])
    await h.transport.route("\(calPath)/m1_2026", [.json(staleInstance(summary: "Renamed"))])
    let event = try await h.source.update(ref, editing(location: "Lab"), scope: .thisAndFollowing, notify: .none)
    #expect(event.eventID == "n1")
    #expect(await masterWrites(h).count == 1)
}

@Test func aRefWithoutAVersionIsNotCheckedForStaleness() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(master())])
    await h.transport.route("\(calPath)/m1_2026", [.json(staleInstance(location: "Elsewhere"))])
    var unversioned = ref
    unversioned.version = nil
    _ = try await h.source.update(unversioned, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none)
    #expect(await masterWrites(h).count == 1)
}

@Test func splittingNeedsTheOccurrencesOriginalStart() async throws {
    let h = try await harness(masterResponses: [.json(master())])
    let noStart = EventRef(calendarID: cal, eventID: "m1_x", version: "i", seriesID: "m1")
    await expectWriteError(.invalid("this and following needs the occurrence's original start")) {
        _ = try await h.source.update(noStart, EventPatch(title: "X"), scope: .thisAndFollowing, notify: .none)
    }
}

@Test func googleNowAcceptsAllThreeScopes() async throws {
    let h = try await Harness()
    #expect(h.source.capabilities.recurrenceScopes == Set(RecurrenceScope.allCases))
}
