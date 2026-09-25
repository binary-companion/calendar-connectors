import CalendarCore
import Foundation
import Testing
@testable import GoogleCalendar

private let utc = TimeZone(identifier: "UTC")!
private let newYork = TimeZone(identifier: "America/New_York")!
private func instant(_ s: String) -> Date { ISO8601DateFormatter().date(from: s)! }
private func timed(_ zone: TimeZone? = utc) -> EventTiming {
    EventTiming(start: instant("2026-09-21T10:00:00Z"), end: instant("2026-09-21T10:30:00Z"), timeZone: zone, isAllDay: false)
}
private func dict(_ any: Any?) -> [String: Any] { any as? [String: Any] ?? [:] }
/// Structural equality of two JSON containers via canonical (sorted-key) serialization; portable to Linux, where `AnyObject.isEqual` is unavailable.
private func same(_ a: Any, _ b: Any) -> Bool {
    func canonical(_ x: Any) -> Data? { try? JSONSerialization.data(withJSONObject: x, options: [.sortedKeys]) }
    guard let l = canonical(a), let r = canonical(b) else { return false }
    return l == r
}

// Foundation on Darwin reports the "UTC" zone's identifier as "GMT" while Linux says "UTC"; the wire format is always "UTC".
@Test func createBodyForATimedEvent() throws {
    let draft = EventDraft(
        title: "Sync", timing: timed(), notes: "n", location: "Room", availability: .free, visibility: .privateEvent,
        reminders: [Reminder(minutesBefore: 10)],
        attendees: [AttendeeDraft(email: "a@x.com", name: "A", role: .optional), AttendeeDraft(email: "r@x.com", role: .resource)])
    let body = try GoogleWriteMapper.createBody(draft)
    let json = body.json
    #expect(json["summary"] as? String == "Sync" && json["description"] as? String == "n" && json["location"] as? String == "Room")
    #expect(dict(json["start"])["dateTime"] as? String == "2026-09-21T10:00:00Z" && dict(json["start"])["timeZone"] as? String == "UTC")
    #expect(dict(json["end"])["dateTime"] as? String == "2026-09-21T10:30:00Z")
    #expect(json["transparency"] as? String == "transparent" && json["visibility"] as? String == "private")
    #expect(same(json["reminders"]!, ["useDefault": false, "overrides": [["method": "popup", "minutes": 10]]] as NSDictionary))
    let attendees = try #require(json["attendees"] as? [[String: Any]])
    #expect(attendees.count == 2 && attendees[0]["optional"] as? Bool == true && attendees[0]["displayName"] as? String == "A")
    #expect(attendees[1]["resource"] as? Bool == true)
    #expect(!body.needsConferenceVersion && json["recurrence"] == nil && json["conferenceData"] == nil)
}

@Test func allDayUsesDatesWithTheExclusiveEnd() throws {
    let timing = EventTiming(start: instant("2026-09-18T04:00:00Z"), end: instant("2026-09-20T04:00:00Z"), timeZone: newYork, isAllDay: true)
    let json = try GoogleWriteMapper.createBody(EventDraft(title: "Trip", timing: timing)).json
    #expect(same(json["start"]!, ["date": "2026-09-18"] as NSDictionary) && same(json["end"]!, ["date": "2026-09-20"] as NSDictionary))
}

@Test func nilRemindersAreOmittedAndGenerateRequestsAMeetLink() throws {
    var draft = EventDraft(title: "Sync", timing: timed())
    #expect(try GoogleWriteMapper.createBody(draft).json["reminders"] == nil)
    draft.conference = .generate
    let body = try GoogleWriteMapper.createBody(draft)
    let request = dict(dict(body.json["conferenceData"])["createRequest"])
    #expect(body.needsConferenceVersion && dict(request["conferenceSolutionKey"])["type"] as? String == "hangoutsMeet")
    #expect((request["requestId"] as? String)?.isEmpty == false)
}

@Test func recurrenceRendersAnRruleAndNeedsAZoneUnlessAllDay() async throws {
    var draft = EventDraft(title: "Weekly", timing: timed(), recurrence: RecurrenceRule(frequency: .weekly, weekdays: [.init(.monday)], end: .count(4)))
    #expect(try GoogleWriteMapper.createBody(draft).json["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;BYDAY=MO;COUNT=4"])
    draft.timing = timed(nil)
    await expectWriteError(.invalid("recurring events need a time zone")) { _ = try GoogleWriteMapper.createBody(draft) }
}

@Test func tooManyRemindersAreRejected() async {
    let many = (1...6).map { Reminder(minutesBefore: $0) }
    await expectWriteError(.invalid("Google allows at most 5 reminders")) {
        _ = try GoogleWriteMapper.createBody(EventDraft(title: "T", timing: timed(), reminders: many))
    }
}

@Test func patchBodyContainsOnlyTouchedFields() throws {
    let body = try GoogleWriteMapper.patchBody(EventPatch(title: "New", availability: .busy), currentAttendees: nil)
    #expect(same(body.json, ["summary": "New", "transparency": "opaque"] as NSDictionary) && !body.needsConferenceVersion)
    #expect(try GoogleWriteMapper.patchBody(EventPatch(), currentAttendees: nil).json.isEmpty)
}

@Test func patchBodyClearsWithNullAndResetsRemindersToDefaults() throws {
    let patch = EventPatch(notes: .clear, location: .clear, reminders: .clear, recurrence: .clear, conference: .remove)
    let json = try GoogleWriteMapper.patchBody(patch, currentAttendees: nil).json
    #expect(json["description"] is NSNull && json["location"] is NSNull && json["recurrence"] is NSNull && json["conferenceData"] is NSNull)
    #expect(same(json["reminders"]!, ["useDefault": true] as NSDictionary))
}

@Test func patchBodySendsTimeAsAPairAndRecurrenceUsesThePatchTiming() throws {
    let patch = EventPatch(timing: timed(newYork), recurrence: .set(RecurrenceRule(frequency: .daily, end: .count(3))))
    let json = try GoogleWriteMapper.patchBody(patch, currentAttendees: nil).json
    #expect(dict(json["start"])["timeZone"] as? String == "America/New_York" && dict(json["end"])["dateTime"] as? String == "2026-09-21T10:30:00Z")
    #expect(json["recurrence"] as? [String] == ["RRULE:FREQ=DAILY;COUNT=3"])
}

@Test func attendeeChangesMergeIntoTheCurrentArrayAndKeepTheRest() throws {
    let current: [[String: Any]] = [
        ["email": "me@x.com", "self": true, "responseStatus": "accepted"],
        ["email": "Bob@x.com", "responseStatus": "accepted", "optional": true],
        ["email": "cy@x.com", "responseStatus": "declined"],
    ]
    let changes = AttendeeChanges(add: [AttendeeDraft(email: "bob@x.com", name: "Bob", role: .required), AttendeeDraft(email: "dee@x.com")], remove: ["cy@x.com"])
    let json = try GoogleWriteMapper.patchBody(EventPatch(attendees: changes), currentAttendees: current).json
    let merged = try #require(json["attendees"] as? [[String: Any]])
    #expect(merged.map { $0["email"] as? String } == ["me@x.com", "Bob@x.com", "dee@x.com"])
    #expect(merged[1]["responseStatus"] as? String == "accepted" && merged[1]["optional"] == nil && merged[1]["displayName"] as? String == "Bob")
    #expect(merged[0]["self"] as? Bool == true)
}

@Test func attendeeChangesNeedTheCurrentAttendees() async {
    await expectWriteError(.invalid("attendee changes need the current attendees")) {
        _ = try GoogleWriteMapper.patchBody(EventPatch(attendees: AttendeeChanges(add: [AttendeeDraft(email: "a@b.c")])), currentAttendees: nil)
    }
}

@Test func respondingSetsTheSelfAttendeeOnly() async throws {
    let current: [[String: Any]] = [["email": "me@x.com", "self": true, "responseStatus": "needsAction"], ["email": "bob@x.com", "responseStatus": "accepted"]]
    let updated = try GoogleWriteMapper.respondAttendees(current: current, response: .tentative)
    #expect(updated[0]["responseStatus"] as? String == "tentative" && updated[1]["responseStatus"] as? String == "accepted")
    await expectWriteError(.invalid("cannot respond with needsAction")) { _ = try GoogleWriteMapper.respondAttendees(current: current, response: .needsAction) }
    await expectWriteError(.invalid("you are not an attendee of this event")) {
        _ = try GoogleWriteMapper.respondAttendees(current: [["email": "bob@x.com"]], response: .accepted)
    }
}

@Test func sendUpdatesMapsEveryPolicy() {
    #expect(GoogleWriteMapper.sendUpdates(.all) == "all" && GoogleWriteMapper.sendUpdates(.externalOnly) == "externalOnly" && GoogleWriteMapper.sendUpdates(.none) == "none")
}

// MARK: Hostile input and Google's limits

@Test func reminderMinutesMustBeWithinGoogleLimitsOnCreateAndPatch() async throws {
    // Google accepts 0 through 40320 minutes (four weeks); a patch skips `EventDraft.validate`, so the mapper checks.
    #expect(try GoogleWriteMapper.remindersJSON([Reminder(minutesBefore: 0), Reminder(minutesBefore: 40320)]).isEmpty == false)
    await expectWriteError(.invalid("reminder minutes must be between 0 and 40320")) {
        _ = try GoogleWriteMapper.patchBody(EventPatch(reminders: .set([Reminder(minutesBefore: -1)])), currentAttendees: nil)
    }
    await expectWriteError(.invalid("reminder minutes must be between 0 and 40320")) {
        _ = try GoogleWriteMapper.createBody(EventDraft(title: "T", timing: timed(), reminders: [Reminder(minutesBefore: 40321)]))
    }
}

@Test func serialisingNonFiniteNumbersThrowsInsteadOfCrashing() async {
    await expectWriteError(.invalid("the request body is not valid JSON")) { _ = try GoogleWriteMapper.data(["x": Double.nan]) }
    #expect(String(decoding: (try? GoogleWriteMapper.data(["b": 1, "a": "é"])) ?? Data(), as: UTF8.self) == #"{"a":"é","b":1}"#)
}

@Test func timesOutsideGooglesYearRangeAreRejected() async {
    let far = EventTiming(start: Date(timeIntervalSince1970: 300_000_000_000), end: Date(timeIntervalSince1970: 300_000_003_600), timeZone: utc, isAllDay: false)
    await expectWriteError(.invalid("times must fall in the years 1 to 9999")) { _ = try GoogleWriteMapper.createBody(EventDraft(title: "T", timing: far)) }
    await expectWriteError(.invalid("times must fall in the years 1 to 9999")) { _ = try GoogleWriteMapper.patchBody(EventPatch(timing: far), currentAttendees: nil) }
}

@Test func aFixedOffsetZoneIsNotSentAsATimeZoneName() async throws {
    // `GMT+0500` is not an IANA name Google accepts; the `dateTime` offset already pins the instant.
    let offset = try #require(TimeZone(identifier: "GMT+0500"))
    let json = try GoogleWriteMapper.createBody(EventDraft(title: "T", timing: timed(offset))).json
    #expect(dict(json["start"])["timeZone"] == nil && dict(json["start"])["dateTime"] as? String == "2026-09-21T10:00:00Z")
    await expectWriteError(.invalid("recurring events need a time zone")) {
        _ = try GoogleWriteMapper.createBody(EventDraft(title: "T", timing: timed(offset), recurrence: RecurrenceRule(frequency: .daily)))
    }
}

@Test func patchingTimeNullsTheOtherFormSoAnAllDayEventCanBecomeTimedAndBack() throws {
    let toTimed = try GoogleWriteMapper.patchBody(EventPatch(timing: timed()), currentAttendees: nil).json
    #expect(dict(toTimed["start"])["date"] is NSNull && dict(toTimed["end"])["date"] is NSNull)
    let allDay = EventTiming(start: instant("2026-09-18T04:00:00Z"), end: instant("2026-09-19T04:00:00Z"), timeZone: newYork, isAllDay: true)
    let toAllDay = try GoogleWriteMapper.patchBody(EventPatch(timing: allDay), currentAttendees: nil).json
    #expect(dict(toAllDay["start"])["date"] as? String == "2026-09-18" && dict(toAllDay["start"])["dateTime"] is NSNull && dict(toAllDay["start"])["timeZone"] is NSNull)
    // Creating never sends nulls.
    #expect(dict(try GoogleWriteMapper.createBody(EventDraft(title: "T", timing: timed())).json["start"])["date"] == nil)
}

@Test func aPatchTimingWithoutAZoneDoesNotBorrowTheBaseZoneForRecurrence() async throws {
    let base = CalendarEvent(eventID: "e", calendarID: "c", title: "T", start: instant("2026-09-21T10:00:00Z"), end: instant("2026-09-21T10:30:00Z"), timeZone: newYork)
    var edited = base
    edited.timeZone = nil
    var patch = EventPatch(from: base, to: edited)
    patch.recurrence = .set(RecurrenceRule(frequency: .daily))
    await expectWriteError(.invalid("recurring events need a time zone")) { _ = try GoogleWriteMapper.patchBody(patch, currentAttendees: nil) }
    // With no timing in the patch the base's zone is what the series will use.
    var recurrenceOnly = EventPatch(from: base, to: base)
    recurrenceOnly.recurrence = .set(RecurrenceRule(frequency: .daily))
    #expect(try GoogleWriteMapper.patchBody(recurrenceOnly, currentAttendees: nil).json["recurrence"] as? [String] == ["RRULE:FREQ=DAILY"])
}

@Test func removingAttendeesIgnoresCaseEvenWhenTheListWasEditedAfterConstruction() {
    var changes = AttendeeChanges()
    changes.remove = ["CY@X.com"]
    let merged = GoogleWriteMapper.mergeAttendees(current: [["email": "cy@x.com"], ["email": "Dee@x.com"]], changes: changes)
    #expect(merged.map { $0["email"] as? String } == ["Dee@x.com"])
}

@Test func aGmtOrUtcZoneIsSentAsUtcOnEveryPlatform() throws {
    for name in ["UTC", "GMT"] {
        let zone = try #require(TimeZone(identifier: name))
        let json = GoogleWriteMapper.timeJSON(timed(zone))
        #expect(json.start["timeZone"] as? String == "UTC" && json.end["timeZone"] as? String == "UTC")
    }
    // The same zone gates and renders a recurrence, so a series on a GMT zone still gets its rule.
    let draft = EventDraft(title: "Daily", timing: timed(try #require(TimeZone(identifier: "GMT"))), recurrence: RecurrenceRule(frequency: .daily))
    #expect(try GoogleWriteMapper.createBody(draft).json["recurrence"] != nil)
}
