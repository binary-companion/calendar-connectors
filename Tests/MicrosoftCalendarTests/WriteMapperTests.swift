import CalendarCore
import Foundation
import Testing
@testable import MicrosoftCalendar

private let la = TimeZone(identifier: "America/Los_Angeles")!
private let utc = TimeZone(identifier: "UTC")!
/// 2026-09-25 10:00 in Los Angeles.
private let tenAM = Date(timeIntervalSince1970: 1_790_355_600)

private func timing(_ start: Date = tenAM, minutes: Double = 30, zone: TimeZone? = la) -> EventTiming {
    EventTiming(start: start, end: start.addingTimeInterval(minutes * 60), timeZone: zone, isAllDay: false)
}

private func dict(_ value: Any?) -> [String: Any] { value as? [String: Any] ?? [:] }

@Test func createBodyWritesATimedEventInTheZonesWindowsName() throws {
    let body = try GraphWriteMapper.createBody(EventDraft(title: "Standup", timing: timing()))
    #expect(body["subject"] as? String == "Standup" && body["isAllDay"] as? Bool == false)
    #expect(dict(body["start"])["dateTime"] as? String == "2026-09-25T10:00:00" && dict(body["start"])["timeZone"] as? String == "Pacific Standard Time")
    #expect(dict(body["end"])["dateTime"] as? String == "2026-09-25T10:30:00")
    #expect(body["showAs"] as? String == "busy" && body["sensitivity"] as? String == "normal")
    #expect(body["body"] == nil && body["location"] == nil && body["attendees"] == nil && body["recurrence"] == nil)
}

@Test func createBodyCarriesEveryField() throws {
    let draft = EventDraft(
        title: "Review", timing: timing(), notes: "Agenda", location: "Room 1", availability: .free, visibility: .privateEvent,
        reminders: [Reminder(minutesBefore: 15)],
        attendees: [AttendeeDraft(email: "A@x.com", name: "Ann"), AttendeeDraft(email: "r@x.com", role: .resource)],
        conference: .generate, recurrence: RecurrenceRule(frequency: .weekly, weekdays: [.init(.friday)], end: .count(4)))
    let body = try GraphWriteMapper.createBody(draft)
    #expect(dict(body["body"])["contentType"] as? String == "text" && dict(body["body"])["content"] as? String == "Agenda")
    #expect(dict(body["location"])["displayName"] as? String == "Room 1")
    #expect(body["showAs"] as? String == "free" && body["sensitivity"] as? String == "private")
    #expect(body["isReminderOn"] as? Bool == true && body["reminderMinutesBeforeStart"] as? Int == 15)
    let attendees = body["attendees"] as? [[String: Any]] ?? []
    #expect(attendees.count == 2 && attendees[0]["type"] as? String == "required" && attendees[1]["type"] as? String == "resource")
    #expect(dict(attendees[0]["emailAddress"])["address"] as? String == "a@x.com" && dict(attendees[0]["emailAddress"])["name"] as? String == "Ann")
    #expect(body["isOnlineMeeting"] as? Bool == true && body["onlineMeetingProvider"] as? String == "teamsForBusiness")
    let range = dict(dict(body["recurrence"])["range"])
    #expect(range["type"] as? String == "numbered" && range["numberOfOccurrences"] as? Int == 4 && range["startDate"] as? String == "2026-09-25")
}

@Test func aZoneWithoutAWindowsNameIsWrittenAsTheSameInstantsInUTC() throws {
    let odd = TimeZone(identifier: "Etc/GMT+5")!
    try #require(WindowsTimeZones.windowsName(for: odd) == nil)
    let body = try GraphWriteMapper.createBody(EventDraft(title: "x", timing: timing(zone: odd)))
    #expect(dict(body["start"])["timeZone"] as? String == "UTC" && dict(body["start"])["dateTime"] as? String == "2026-09-25T17:00:00")
}

@Test func anAllDayEventIsWrittenAsMidnightToMidnight() throws {
    let start = try #require(AllDay.startOfDay(CalendarDate(year: 2026, month: 9, day: 25), in: la))
    let end = try #require(AllDay.startOfDay(CalendarDate(year: 2026, month: 9, day: 27), in: la))
    let body = try GraphWriteMapper.createBody(EventDraft(title: "Trip", timing: EventTiming(start: start, end: end, timeZone: la, isAllDay: true)))
    #expect(body["isAllDay"] as? Bool == true)
    #expect(dict(body["start"])["dateTime"] as? String == "2026-09-25T00:00:00" && dict(body["end"])["dateTime"] as? String == "2026-09-27T00:00:00")
    #expect(dict(body["start"])["timeZone"] as? String == "Pacific Standard Time")
}

@Test func createRefusesWhatGraphCannotStore() throws {
    let repeating = Reminder(trigger: .relative(offset: -600, to: .start), repeatCount: 2, repeatInterval: 60)
    for reminders in [[Reminder(minutesBefore: 5), Reminder(minutesBefore: 10)], [repeating], [Reminder(trigger: .relative(offset: 0, to: .end))]] {
        #expect(throws: WriteError.unsupported(fields: [.reminders])) {
            try GraphWriteMapper.createBody(EventDraft(title: "x", timing: timing(), reminders: reminders))
        }
    }
    let body = try GraphWriteMapper.createBody(EventDraft(title: "x", timing: timing(), reminders: []))
    #expect(body["isReminderOn"] as? Bool == false)
}

@Test func anEmptyPatchProducesAnEmptyBody() throws {
    #expect(try GraphWriteMapper.patchBody(EventPatch(), currentAttendees: [], anchor: .placeholder).isEmpty)
}

@Test func patchBodyCarriesOnlyWhatChanged() throws {
    let patch = EventPatch(title: "New", notes: .clear, location: .set("Room 2"), availability: .tentative, visibility: .confidential, conference: .remove)
    let body = try GraphWriteMapper.patchBody(patch, currentAttendees: [], anchor: .placeholder)
    #expect(Set(body.keys) == ["subject", "body", "location", "showAs", "sensitivity", "isOnlineMeeting"])
    #expect(dict(body["body"])["content"] as? String == "" && dict(body["location"])["displayName"] as? String == "Room 2")
    #expect(body["showAs"] as? String == "tentative" && body["sensitivity"] as? String == "confidential" && body["isOnlineMeeting"] as? Bool == false)
}

@Test func patchRefusesRemovingRemindersOrARecurrence() {
    #expect(throws: WriteError.unsupported(fields: [.reminders])) { try GraphWriteMapper.patchBody(EventPatch(reminders: .clear), currentAttendees: [], anchor: .placeholder) }
    #expect(throws: WriteError.unsupported(fields: [.recurrence])) { try GraphWriteMapper.patchBody(EventPatch(recurrence: .clear), currentAttendees: [], anchor: .placeholder) }
}

@Test func attendeeChangesStartFromTheCurrentListAndKeepEveryoneElse() throws {
    let current: [[String: Any]] = [
        ["type": "required", "status": ["response": "accepted"], "emailAddress": ["name": "Me", "address": "Me@X.com"]],
        ["type": "optional", "status": ["response": "declined"], "emailAddress": ["name": "Opt", "address": "opt@x.com"]],
    ]
    let changes = AttendeeChanges(
        add: [AttendeeDraft(email: "new@x.com"), AttendeeDraft(email: "opt@x.com", name: "Optional", role: .required)], remove: ["ME@x.com"])
    let list = GraphWriteMapper.attendeeList(current: current, changes: changes)
    #expect(list.count == 2)
    #expect(dict(list[0]["emailAddress"])["address"] as? String == "opt@x.com" && list[0]["type"] as? String == "required")
    #expect(dict(list[0]["emailAddress"])["name"] as? String == "Optional")
    #expect(dict(list[1]["emailAddress"])["address"] as? String == "new@x.com")
    #expect(list.allSatisfy { $0["status"] == nil }, "responses stay on the server")
}

@Test func respondActionNamesTheGraphAction() throws {
    #expect(try GraphWriteMapper.respondAction(.accepted) == "accept" && GraphWriteMapper.respondAction(.tentative) == "tentativelyAccept")
    #expect(try GraphWriteMapper.respondAction(.declined) == "decline")
    #expect(throws: WriteError.invalid("a response must be accepted, tentative or declined")) { try GraphWriteMapper.respondAction(.needsAction) }
}
