import Foundation
import Testing
@testable import CalendarCore

private let utc = TimeZone(identifier: "UTC")!
private let newYork = TimeZone(identifier: "America/New_York")!
private func instant(_ s: String) -> Date { ISO8601DateFormatter().date(from: s)! }
private func timed() -> EventTiming {
    EventTiming(start: instant("2026-09-21T10:00:00Z"), end: instant("2026-09-21T11:00:00Z"), timeZone: utc, isAllDay: false)
}

@Test func timingRejectsEndBeforeStartAndNonCanonicalAllDay() async {
    await expectWriteError(.invalid("end must be after start")) {
        try EventTiming(start: instant("2026-09-21T11:00:00Z"), end: instant("2026-09-21T10:00:00Z"), timeZone: utc, isAllDay: false).validate()
    }
    await expectWriteError(.invalid("an all-day event needs a time zone")) {
        try EventTiming(start: instant("2026-09-21T00:00:00Z"), end: instant("2026-09-22T00:00:00Z"), timeZone: nil, isAllDay: true).validate()
    }
    await expectWriteError(.invalid("all-day times must be midnight in America/New_York")) {
        try EventTiming(start: instant("2026-09-21T00:00:00Z"), end: instant("2026-09-22T00:00:00Z"), timeZone: newYork, isAllDay: true).validate()
    }
    #expect(throws: Never.self) {
        try EventTiming(start: instant("2026-09-21T04:00:00Z"), end: instant("2026-09-22T04:00:00Z"), timeZone: newYork, isAllDay: true).validate()
    }
}

@Test func timingRejectsNonFiniteDates() async {
    let bad = [Date(timeIntervalSinceReferenceDate: .nan), Date(timeIntervalSinceReferenceDate: .infinity),
               Date(timeIntervalSinceReferenceDate: -.infinity)]
    let fine = instant("2026-09-21T10:00:00Z")
    for isAllDay in [false, true] {
        for date in bad {
            await expectWriteError(.invalid("times must be finite")) {
                try EventTiming(start: date, end: fine, timeZone: utc, isAllDay: isAllDay).validate()
            }
            await expectWriteError(.invalid("times must be finite")) {
                try EventTiming(start: fine, end: date, timeZone: utc, isAllDay: isAllDay).validate()
            }
        }
        await expectWriteError(.invalid("times must be finite")) {
            try EventTiming(start: bad[2], end: bad[1], timeZone: utc, isAllDay: isAllDay).validate()
        }
    }
}

@Test func attendeeDraftNormalizesEmail() {
    #expect(AttendeeDraft(email: "  Ann@Example.COM ").email == "ann@example.com")
}

@Test func draftValidationChecksTimingAttendeesRemindersAndRecurrence() async {
    let base = EventDraft(title: "T", timing: timed())
    #expect(throws: Never.self) { try base.validate() }
    var noAt = base; noAt.attendees = [AttendeeDraft(email: "nobody")]
    await expectWriteError(.invalid("attendee email is not valid: nobody")) { try noAt.validate() }
    var negative = base; negative.reminders = [Reminder(minutesBefore: -5)]
    await expectWriteError(.invalid("reminder minutes must not be negative")) { try negative.validate() }
    var badRule = base; badRule.recurrence = RecurrenceRule(frequency: .daily, interval: 0)
    await expectWriteError(.invalid("recurrence interval must be at least 1")) { try badRule.validate() }
}

@Test func draftValidationRejectsMalformedAddressesBeyondAMissingAt() async {
    for bad in ["@", "a@", "@b.c", "a@@b.c", "a b@c.d", "a@b@c.d", "", "a@b.c,d@e.f", "a@b.c;d@e.f", "<a@b.c>",
                "a@b.c\nd@e.f", "a@b.c\u{0}", "a\u{7}@b.c"] {
        var draft = EventDraft(title: "T", timing: timed())
        draft.attendees = [AttendeeDraft(email: bad)]
        await expectWriteError(.invalid("attendee email is not valid: \(AttendeeDraft(email: bad).email)")) { try draft.validate() }
    }
    var ok = EventDraft(title: "T", timing: timed())
    ok.attendees = [AttendeeDraft(email: "a@b.c")]
    #expect(throws: Never.self) { try ok.validate() }
}

@Test func usedFieldsListsWhatTheDraftSetsBeyondDefaults() {
    #expect(EventDraft(title: "T", timing: timed()).usedFields == [.title, .timing])
    var rich = EventDraft(title: "T", timing: timed(), notes: "n", location: "l", availability: .free, visibility: .privateEvent,
                          reminders: [], attendees: [AttendeeDraft(email: "a@b.c")], conference: .generate,
                          recurrence: RecurrenceRule(frequency: .daily))
    #expect(rich.usedFields == Set(EventField.allCases))
    rich.reminders = nil
    #expect(!rich.usedFields.contains(.reminders))
}

private func source() -> CalendarEvent {
    CalendarEvent(
        eventID: "e", calendarID: "c", title: "Planning", notes: "n", location: "Room",
        start: instant("2026-09-21T10:00:00Z"), end: instant("2026-09-21T11:00:00Z"), timeZone: utc,
        availability: .free, visibility: .privateEvent, series: .occurrence(seriesID: "s", originalStart: nil),
        attendees: [Attendee(email: "me@x.com", isSelf: true), Attendee(email: "Bob@x.com", role: .optional),
                    Attendee(name: "No Email")],
        conferences: [ConferenceInfo(url: URL(string: "https://meet.google.com/abc")!, provider: .meet)],
        reminders: [Reminder(minutesBefore: 10)])
}

@Test func copyingToAFullyWritableTargetKeepsEverythingRepresentable() {
    let caps = SourceCapabilities(canWrite: true, canEditAttendees: true, writableFields: Set(EventField.allCases))
    let draft = EventDraft(copying: source(), for: caps)
    #expect(draft.title == "Planning" && draft.notes == "n" && draft.location == "Room")
    #expect(draft.timing == EventTiming(start: instant("2026-09-21T10:00:00Z"), end: instant("2026-09-21T11:00:00Z"), timeZone: utc, isAllDay: false))
    #expect(draft.availability == .free && draft.visibility == .privateEvent)
    #expect(draft.reminders == [Reminder(minutesBefore: 10)])
    #expect(draft.attendees == [AttendeeDraft(email: "bob@x.com", role: .optional)])   // self and email-less attendees dropped
    #expect(draft.conference == .generate)   // only a Meet link is re-requested
    #expect(draft.recurrence == nil)
}

@Test func copyingDropsFieldsTheTargetCannotWrite() {
    let caps = SourceCapabilities(canWrite: true, writableFields: [.title, .notes, .location, .timing, .availability, .reminders, .recurrence])
    let draft = EventDraft(copying: source(), for: caps)
    #expect(draft.attendees.isEmpty && draft.visibility == .default && draft.conference == .none)
    #expect(draft.notes == "n" && draft.availability == .free)
    #expect(draft.recurrence == nil)
}

@Test func copyingTurnsEmptyRemindersIntoCalendarDefaults() {
    var event = source()
    event.reminders = []
    let draft = EventDraft(copying: event, for: SourceCapabilities(canWrite: true, writableFields: Set(EventField.allCases)))
    #expect(draft.reminders == nil)
}

@Test func copyingToAReadOnlyTargetKeepsOnlyTheRequiredParts() {
    let draft = EventDraft(copying: source(), for: SourceCapabilities())
    #expect(draft.title == "Planning" && draft.notes == nil && draft.location == nil && draft.reminders == nil)
    #expect(draft.availability == .busy && draft.visibility == .default && draft.attendees.isEmpty)
    #expect(draft.conference == .none && draft.recurrence == nil)
}

@Test func copyingDoesNotReRequestANonMeetConference() {
    var event = source()
    event.conferences = [ConferenceInfo(url: URL(string: "https://zoom.us/j/1")!, provider: .zoom)]
    let draft = EventDraft(copying: event, for: SourceCapabilities(canWrite: true, writableFields: Set(EventField.allCases)))
    #expect(draft.conference == .none)
}

@Test func copyingDropsSourceDataThatWouldFailValidation() throws {
    var event = source()
    event.attendees = [Attendee(email: "nobody"), Attendee(email: "a@b.c,d@e.f"), Attendee(email: "ok@x.com")]
    event.reminders = [Reminder(minutesBefore: -5), Reminder(minutesBefore: 15)]
    let caps = SourceCapabilities(canWrite: true, canEditAttendees: true, writableFields: Set(EventField.allCases))
    let draft = EventDraft(copying: event, for: caps)
    #expect(draft.attendees == [AttendeeDraft(email: "ok@x.com")])
    #expect(draft.reminders == [Reminder(minutesBefore: 15)])
    try draft.validate()

    event.reminders = [Reminder(minutesBefore: -1)]
    #expect(EventDraft(copying: event, for: caps).reminders == nil)
}

@Test func copyingKeepsAnAllDayEventInCanonicalForm() throws {
    var event = source()
    event.isAllDay = true
    event.timeZone = newYork
    event.start = instant("2026-09-21T04:00:00Z")
    event.end = instant("2026-09-22T04:00:00Z")
    let draft = EventDraft(copying: event, for: SourceCapabilities())
    #expect(draft.timing.isAllDay && draft.timing.timeZone == newYork)
    try draft.validate()
}

@Test func copyingAnEventWithUnknownFieldsUsesTheDraftDefaults() {
    let event = CalendarEvent(eventID: "e", calendarID: "c", title: "T", start: Date(timeIntervalSince1970: 1000), end: Date(timeIntervalSince1970: 2000))
    let draft = EventDraft(copying: event, for: SourceCapabilities(canWrite: true, writableFields: Set(EventField.allCases)))
    #expect(draft.availability == .busy && draft.visibility == .default && draft.reminders == nil)
    #expect(draft.usedFields == [.title, .timing])
}

@Test func copyingCarriesAUIDOnlyWhenItIsGlobal() {
    var event = CalendarEvent(eventID: "e", uid: "u-1", uidScope: .global, calendarID: "c", title: "T", start: Date(timeIntervalSince1970: 1000), end: Date(timeIntervalSince1970: 2000))
    let capabilities = SourceCapabilities(canWrite: true, writableFields: Set(EventField.allCases))
    #expect(EventDraft(copying: event, for: capabilities).uid == "u-1")
    event.uidScope = .provider
    #expect(EventDraft(copying: event, for: capabilities).uid == nil)
    event.uidScope = nil
    #expect(EventDraft(copying: event, for: capabilities).uid == nil)
}
