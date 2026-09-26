import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import MicrosoftCalendar

private let la = TimeZone(identifier: "America/Los_Angeles")!
private let firstStart = Date(timeIntervalSince1970: 1_790_355_600)              // 2026-09-25 10:00 in Los Angeles (a Friday)
private let splitStart = firstStart.addingTimeInterval(14 * 86_400)               // 2026-10-09 10:00

private func recurrence(range: [String: Any]) -> [String: Any] {
    var range = range
    range["startDate"] = "2026-09-25"
    range["recurrenceTimeZone"] = "Pacific Standard Time"
    return ["pattern": ["type": "weekly", "interval": 1, "daysOfWeek": ["friday"], "firstDayOfWeek": "monday"], "range": range]
}

private func master(range: [String: Any] = ["type": "noEnd"]) -> [String: Any] {
    graphEvent(id: "master1", extra: ["type": "seriesMaster", "recurrence": recurrence(range: range)])
}

private func occurrence() -> [String: Any] {
    graphEvent(id: "occ3", extra: [
        "type": "occurrence", "seriesMasterId": "master1",
        "start": ["dateTime": "2026-10-09T10:00:00.0000000", "timeZone": "Pacific Standard Time"],
        "end": ["dateTime": "2026-10-09T10:30:00.0000000", "timeZone": "Pacific Standard Time"]])
}

private let ref = EventRef(calendarID: "cal1", eventID: "occ3", version: "ck1", seriesID: "master1", originalStart: splitStart)

private func routes(_ h: SourceHarness, master: [String: Any] = master(), insert: [HTTPResponse] = [.json(graphEvent(id: "new"), status: 201)]) async {
    await h.transport.route("cal1/events", insert)                                   // the insert (least specific first)
    await h.transport.route("cal1/events/master1", [.json(master)])
    await h.transport.route("cal1/events/occ3", [.json(occurrence())])
}

private func patchesToMaster(_ h: SourceHarness) async -> [[String: Any]] {
    await h.transport.requests(matching: "cal1/events/master1").filter { $0.method == "PATCH" }.map(bodyJSON)
}

private func inserts(_ h: SourceHarness) async -> [[String: Any]] {
    await h.transport.requests(matching: "cal1/events").filter { $0.method == "POST" }.map(bodyJSON)
}

private func range(_ body: [String: Any]) -> [String: Any] { (body["recurrence"] as? [String: Any])?["range"] as? [String: Any] ?? [:] }

@Test func deletingThisAndFollowingCutsTheSeriesTheDayBefore() async throws {
    let h = try await SourceHarness()
    await routes(h)
    try await h.source.delete(ref, scope: .thisAndFollowing, notify: .none)
    let patches = await patchesToMaster(h)
    #expect(patches.count == 1 && range(patches[0])["type"] as? String == "endDate" && range(patches[0])["endDate"] as? String == "2026-10-08")
    #expect(await inserts(h).isEmpty)
}

@Test func updatingThisAndFollowingCutsTheSeriesAndStartsANewOneWithThePatch() async throws {
    let h = try await SourceHarness()
    await routes(h)
    let event = try await h.source.update(ref, EventPatch(title: "Renamed"), scope: .thisAndFollowing, notify: .none)
    #expect(event.eventID == "new")
    let patches = await patchesToMaster(h)
    #expect(patches.count == 1 && range(patches[0])["endDate"] as? String == "2026-10-08")
    let created = await inserts(h)
    try #require(created.count == 1)
    #expect(created[0]["subject"] as? String == "Renamed" && created[0]["transactionId"] is String)
    #expect(range(created[0])["type"] as? String == "noEnd" && range(created[0])["startDate"] as? String == "2026-10-09")
    #expect((created[0]["start"] as? [String: Any])?["dateTime"] as? String == "2026-10-09T10:00:00")
    #expect(created[0]["attendees"] is [[String: Any]] && created[0]["id"] == nil && created[0]["iCalUId"] == nil)
}

@Test func aNumberedSeriesKeepsTheOccurrencesLeftOver() async throws {
    let h = try await SourceHarness()
    await routes(h, master: master(range: ["type": "numbered", "numberOfOccurrences": 8]))
    await h.transport.route("master1/instances", [.json(["value": [["id": "i1"], ["id": "i2"]]])])
    _ = try await h.source.update(ref, EventPatch(title: "Renamed"), scope: .thisAndFollowing, notify: .none)
    let created = await inserts(h)
    #expect(range(created[0])["type"] as? String == "numbered" && range(created[0])["numberOfOccurrences"] as? Int == 6)
    let patches = await patchesToMaster(h)
    #expect(range(patches[0])["type"] as? String == "endDate")
}

@Test func splittingAtTheFirstOccurrenceIsTheSameAsWritingTheWholeSeries() async throws {
    let h = try await SourceHarness()
    await routes(h)
    let first = EventRef(calendarID: "cal1", eventID: "master1", version: "ck1", seriesID: "master1", originalStart: firstStart)
    _ = try await h.source.update(first, EventPatch(title: "All"), scope: .thisAndFollowing, notify: .none)
    let patches = await patchesToMaster(h)
    #expect(patches.count == 1 && patches[0]["subject"] as? String == "All" && patches[0]["recurrence"] == nil)
    #expect(await inserts(h).isEmpty)
}

@Test func aDefiniteInsertFailurePutsTheOriginalRecurrenceBack() async throws {
    let h = try await SourceHarness()
    await routes(h, insert: [graphError("ErrorInvalidRequest", message: "no", status: 400)])
    await expectWriteError(.invalid("no")) { _ = try await h.source.update(ref, EventPatch(title: "Renamed"), scope: .thisAndFollowing, notify: .none) }
    let patches = await patchesToMaster(h)
    #expect(patches.count == 2)
    #expect(range(patches[0])["type"] as? String == "endDate")
    #expect(range(patches[1])["type"] as? String == "noEnd" && range(patches[1])["startDate"] as? String == "2026-09-25")
    #expect(await inserts(h).count == 1)
}

@Test func aLostInsertReplyIsRetriedOnceWithTheSameTransactionID() async throws {
    let h = try await SourceHarness()
    await routes(h, insert: [graphError("Oops", status: 500), .json(graphEvent(id: "new"), status: 201)])
    let event = try await h.source.update(ref, EventPatch(title: "Renamed"), scope: .thisAndFollowing, notify: .none)
    #expect(event.eventID == "new")
    let created = await inserts(h)
    #expect(created.count == 2 && created[0]["transactionId"] as? String == created[1]["transactionId"] as? String)
    #expect(await patchesToMaster(h).count == 1, "the master is not restored: the insert may have applied")
}

@Test func anInsertThatKeepsFailingWithAnUnknownOutcomeIsReportedAsPartial() async throws {
    let h = try await SourceHarness()
    await routes(h, insert: [graphError("Oops", status: 500)])
    do {
        _ = try await h.source.update(ref, EventPatch(title: "Renamed"), scope: .thisAndFollowing, notify: .none)
        Issue.record("expected partial")
    } catch WriteError.partial {
    }
    #expect(await patchesToMaster(h).count == 1)
}

@Test func aFailedRestoreAfterADefiniteInsertFailureIsReportedAsPartial() async throws {
    let h = try await SourceHarness()
    await routes(h, insert: [graphError("ErrorInvalidRequest", message: "no", status: 400)])
    // The truncation succeeds, the restore that follows the failed insert does not.
    await h.transport.route("cal1/events/master1", [.json(master()), .json(master()), graphError("Oops", status: 400)])
    do {
        _ = try await h.source.update(ref, EventPatch(title: "Renamed"), scope: .thisAndFollowing, notify: .none)
        Issue.record("expected partial")
    } catch WriteError.partial {
    }
}

@Test func nothingIsWrittenWhenThePatchIsInvalid() async throws {
    let h = try await SourceHarness()
    await routes(h)
    await expectWriteError(.unsupported(fields: [.reminders])) {
        _ = try await h.source.update(ref, EventPatch(reminders: .clear), scope: .thisAndFollowing, notify: .none)
    }
    let patched = await patchesToMaster(h)
    let inserted = await inserts(h)
    #expect(patched.isEmpty && inserted.isEmpty)
}

@Test func aSplitNeedsTheOccurrencesOriginalStart() async throws {
    let h = try await SourceHarness()
    await routes(h)
    let noSlot = EventRef(calendarID: "cal1", eventID: "occ3", seriesID: "master1")
    await expectWriteError(.invalid("this and following needs the occurrence's original start")) {
        try await h.source.delete(noSlot, scope: .thisAndFollowing, notify: .none)
    }
}
