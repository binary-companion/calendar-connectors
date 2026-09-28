import CalendarCore
import Foundation
import Testing
@testable import ICalendar

private let now = Date(timeIntervalSince1970: 1_790_000_000)

private func draft(_ timing: EventTiming? = nil) -> EventDraft {
    EventDraft(title: "Design review", timing: timing ?? EventTiming(start: laTime(2026, 9, 10, 9), end: laTime(2026, 9, 10, 10), timeZone: la, isAllDay: false),
               notes: "Bring, notes; please", location: "Room 2", availability: .free, visibility: .privateEvent,
               reminders: [.before(minutes: 10)], recurrence: try! RecurrenceRule(rrule: "FREQ=WEEKLY;COUNT=3"))
}

private func readBack(_ vevent: ICalComponent, zones: [TimeZone] = [la]) throws -> [CalendarEvent] {
    let resource = try EventWriter.resource(for: vevent, zones: zones, from: laTime(2026, 9, 1), through: laTime(2027, 9, 1))
    let parsed = try EventResource(data: resource.serialized())
    return EventReader.events(in: parsed, overlapping: DateInterval(start: laTime(2026, 9, 1, 0), end: laTime(2026, 10, 1, 0)), context: context("new.ics"))
}

@Test func draftRoundTripsThroughTheReader() throws {
    let events = try readBack(try EventWriter.vevent(from: draft(), uid: "new-uid", now: now, organizerAddress: nil))
    #expect(events.count == 3)
    let first = events[0]
    #expect(first.uid == "new-uid")
    #expect(first.title == "Design review")
    #expect(first.notes == "Bring, notes; please")
    #expect(first.location == "Room 2")
    #expect(first.start == laTime(2026, 9, 10, 9) && first.end == laTime(2026, 9, 10, 10))
    #expect(first.timeZone.identifier == "America/Los_Angeles")
    #expect(first.availability == .free)
    #expect(first.visibility == .privateEvent)
    #expect(first.reminders == [Reminder(trigger: .relative(offset: -600, to: .start), isCalendarDefault: false)])
}

@Test func writesTZIDAndVTIMEZONE() throws {
    let vevent = try EventWriter.vevent(from: draft(), uid: "u", now: now, organizerAddress: nil)
    #expect(vevent.property("DTSTART")?.parameter("TZID") == "America/Los_Angeles")
    #expect(vevent.property("DTSTART")?.value == "20260910T090000")
    #expect(vevent.property("DTSTAMP")?.value == ICalValues.utcText(now))
    #expect(vevent.property("SEQUENCE")?.value == "0")
    let resource = try EventWriter.resource(for: vevent, zones: [la], from: laTime(2026, 9, 1), through: laTime(2027, 9, 1))
    #expect(resource.calendar.components(named: "VTIMEZONE").first?.property("TZID")?.value == "America/Los_Angeles")
    #expect(resource.calendar.property("PRODID")?.value == EventWriter.productID)
    #expect(resource.calendar.property("VERSION")?.value == "2.0")
}

@Test func utcAndZonelessDraftsUseZ() throws {
    let timing = EventTiming(start: laTime(2026, 9, 10, 9), end: laTime(2026, 9, 10, 10), timeZone: nil, isAllDay: false)
    let vevent = try EventWriter.vevent(from: draft(timing), uid: "u", now: now, organizerAddress: nil)
    #expect(vevent.property("DTSTART")?.value == "20260910T160000Z")
    #expect(vevent.property("DTSTART")?.parameter("TZID") == nil)
}

@Test func allDayDraftUsesValueDate() throws {
    let start = AllDay.startOfDay(CalendarDate(year: 2026, month: 9, day: 10), in: la)!
    let end = AllDay.startOfDay(CalendarDate(year: 2026, month: 9, day: 12), in: la)!
    let vevent = try EventWriter.vevent(from: draft(EventTiming(start: start, end: end, timeZone: la, isAllDay: true)), uid: "u", now: now, organizerAddress: nil)
    #expect(vevent.property("DTSTART")?.parameter("VALUE") == "DATE")
    #expect(vevent.property("DTSTART")?.value == "20260910")
    #expect(vevent.property("DTEND")?.value == "20260912")
    #expect(vevent.property("RRULE")?.value == "FREQ=WEEKLY;COUNT=3")
}

@Test func remindersNilAndEmptyWriteNoAlarmAndUnsupportedOnesThrow() throws {
    var noReminders = draft()
    noReminders.reminders = nil
    #expect(try EventWriter.vevent(from: noReminders, uid: "u", now: now, organizerAddress: nil).components(named: "VALARM").isEmpty)
    noReminders.reminders = []
    #expect(try EventWriter.vevent(from: noReminders, uid: "u", now: now, organizerAddress: nil).components(named: "VALARM").isEmpty)
    #expect(throws: WriteError.unsupported(fields: [.reminders])) { try AlarmMapper.alarms(from: [Reminder(minutesBefore: 5), .before(minutes: 5, type: .email(address: nil))]) }
    #expect(throws: WriteError.unsupported(fields: [.reminders])) {
        try AlarmMapper.alarms(from: [Reminder(trigger: .location(StructuredLocation(title: "x"), .enter))])
    }
}

@Test func alarmsRoundTrip() throws {
    let reminders = [
        Reminder(trigger: .relative(offset: -600, to: .start)),
        Reminder(trigger: .relative(offset: 0, to: .end), type: .audio(soundName: "Chord")),
        Reminder(trigger: .absolute(Date(timeIntervalSince1970: 1_790_521_200)), repeatCount: 2, repeatInterval: 300),
    ]
    let back = AlarmMapper.reminders(from: try AlarmMapper.alarms(from: reminders))
    #expect(Reminder.sameSet(back, reminders))
}

@Test func attendeesWriteWithOrganizer() throws {
    var withGuests = draft()
    withGuests.attendees = [AttendeeDraft(email: "ann@example.test", name: "Ann"), AttendeeDraft(email: "room@example.test", role: .resource)]
    let vevent = try EventWriter.vevent(from: withGuests, uid: "u", now: now, organizerAddress: "mailto:me@icloud.test")
    #expect(vevent.property("ORGANIZER")?.value == "mailto:me@icloud.test")
    let attendees = vevent.properties(named: "ATTENDEE")
    #expect(attendees.map(\.value) == ["mailto:me@icloud.test", "mailto:ann@example.test", "mailto:room@example.test"])
    #expect(attendees[0].parameter("PARTSTAT") == "ACCEPTED")
    #expect(attendees[1].parameter("PARTSTAT") == "NEEDS-ACTION" && attendees[1].parameter("RSVP") == "TRUE" && attendees[1].parameter("CN") == "Ann")
    #expect(attendees[2].parameter("CUTYPE") == "RESOURCE")
}

@Test func setResponseChangesOnlyTheAccountsEntry() throws {
    var vevent = try #require(try resource(weeklySeries).master)
    #expect(AttendeeMapper.setResponse(.declined, in: &vevent, selfAddresses: ["mailto:me@icloud.test"]))
    let entries = vevent.properties(named: "ATTENDEE")
    #expect(entries[0].parameter("PARTSTAT") == "DECLINED" && entries[0].parameter("RSVP") == nil)
    #expect(entries[1].parameter("PARTSTAT") == "TENTATIVE")
    #expect(!AttendeeMapper.setResponse(.accepted, in: &vevent, selfAddresses: ["mailto:someone@else.test"]))
}

@Test func patchChangesOnlyTouchedPropertiesAndKeepsUnknownOnes() throws {
    var vevent = try #require(try resource(weeklySeries).master)
    let before = vevent
    try EventWriter.apply(EventPatch(title: "Renamed", location: .clear), to: &vevent, now: now, organizerAddress: nil)
    #expect(vevent.property("SUMMARY")?.text == "Renamed")
    #expect(vevent.property("LOCATION") == nil)
    #expect(vevent.property("DESCRIPTION") == before.property("DESCRIPTION"))
    #expect(vevent.property("X-APPLE-TRAVEL-ADVISORY-BEHAVIOR") != nil)
    #expect(vevent.property("SEQUENCE")?.value == "2")              // content only: no bump
    #expect(vevent.property("LAST-MODIFIED")?.value == ICalValues.utcText(now))
}

@Test func timingAndAttendeeChangesBumpTheSequence() throws {
    var vevent = try #require(try resource(weeklySeries).master)
    let timing = EventTiming(start: laTime(2026, 9, 1, 11), end: laTime(2026, 9, 1, 12), timeZone: la, isAllDay: false)
    try EventWriter.apply(EventPatch(timing: timing, attendees: AttendeeChanges(add: [AttendeeDraft(email: "new@example.test")], remove: ["ann@example.test"])),
                          to: &vevent, now: now, organizerAddress: "mailto:me@icloud.test")
    #expect(vevent.property("SEQUENCE")?.value == "3")
    #expect(vevent.property("DTSTART")?.value == "20260901T110000")
    #expect(vevent.property("DURATION") == nil)
    let addresses = vevent.properties(named: "ATTENDEE").map(\.value)
    #expect(addresses.contains("mailto:new@example.test"))
    #expect(!addresses.contains { $0.lowercased() == "mailto:ann@example.test" })
}

@Test func clearingRemindersIsUnsupported() throws {
    var vevent = try #require(try resource(weeklySeries).master)
    #expect(throws: WriteError.unsupported(fields: [.reminders])) {
        try EventWriter.apply(EventPatch(reminders: .clear), to: &vevent, now: now, organizerAddress: nil)
    }
}

@Test func recurrencePatchReplacesAndClears() throws {
    var vevent = try #require(try resource(weeklySeries).master)
    try EventWriter.apply(EventPatch(recurrence: .set(try RecurrenceRule(rrule: "FREQ=DAILY;COUNT=2"))), to: &vevent, now: now, organizerAddress: nil)
    #expect(vevent.properties(named: "RRULE").map(\.value) == ["FREQ=DAILY;COUNT=2"])
    try EventWriter.apply(EventPatch(recurrence: .clear), to: &vevent, now: now, organizerAddress: nil)
    #expect(vevent.property("RRULE") == nil && vevent.property("EXDATE") == nil && vevent.property("RDATE") == nil)
}

@Test func anAllDayRecurrenceUntilIsWrittenInTheCalendarZone() throws {
    let berlin = TimeZone(identifier: "Europe/Berlin")!
    var vevent = try #require(try resource("""
    BEGIN:VCALENDAR
    VERSION:2.0
    PRODID:-//Test//EN
    BEGIN:VEVENT
    UID:ALLDAY
    DTSTAMP:20260301T000000Z
    DTSTART;VALUE=DATE:20260305
    DTEND;VALUE=DATE:20260306
    RRULE:FREQ=DAILY
    END:VEVENT
    END:VCALENDAR
    """).master)
    // Read the way EventReader reads a date UNTIL: in the calendar's zone.
    let rule = try RecurrenceRule(rrule: "FREQ=DAILY;UNTIL=20260310", in: berlin)
    try EventWriter.apply(EventPatch(recurrence: .set(rule)), to: &vevent, now: now, organizerAddress: nil, calendarZone: berlin)
    #expect(vevent.property("RRULE")?.value == "FREQ=DAILY;UNTIL=20260310")
}
