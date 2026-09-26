import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import MicrosoftCalendar

private let la = TimeZone(identifier: "America/Los_Angeles")!
private let laCalendar = CalendarDescriptor(id: "cal1", title: "Calendar", service: .microsoft, timeZone: la)

/// A Graph event as `calendarView` returns it with `Prefer: outlook.timezone="Pacific Standard Time"`.
func graphEvent(id: String = "ev1", extra: [String: Any] = [:]) -> [String: Any] {
    var json: [String: Any] = [
        "id": id, "iCalUId": "uid-\(id)", "subject": "Standup", "type": "singleInstance", "isAllDay": false, "isCancelled": false,
        "start": ["dateTime": "2026-09-25T10:00:00.0000000", "timeZone": "Pacific Standard Time"],
        "end": ["dateTime": "2026-09-25T10:30:00.0000000", "timeZone": "Pacific Standard Time"],
        "originalStartTimeZone": "Pacific Standard Time", "showAs": "busy", "sensitivity": "normal",
        "changeKey": "ck1", "lastModifiedDateTime": "2026-09-24T12:00:00.1234567Z", "createdDateTime": "2026-09-01T08:00:00Z",
        "webLink": "https://outlook.office.com/calendar/item/x", "isReminderOn": true, "reminderMinutesBeforeStart": 10,
        "body": ["contentType": "text", "content": "Agenda"], "location": ["displayName": "Room 1"],
        "organizer": ["emailAddress": ["name": "Boss", "address": "boss@x.com"]], "isOrganizer": false,
        "attendees": [
            ["type": "required", "status": ["response": "accepted"], "emailAddress": ["name": "Me", "address": "Me@X.com"]],
            ["type": "optional", "status": ["response": "tentativelyAccepted"], "emailAddress": ["name": "Opt", "address": "opt@x.com"]],
            ["type": "resource", "status": ["response": "none"], "emailAddress": ["name": "Room", "address": "room@x.com"]],
        ],
    ]
    for (key, value) in extra { json[key] = value }
    return json
}

func decodeEvent(_ json: [String: Any]) throws -> GraphEventDTO {
    try JSONDecoder().decode(GraphEventDTO.self, from: try JSONSerialization.data(withJSONObject: json))
}

private func map(_ extra: [String: Any] = [:]) throws -> CalendarEvent {
    try #require(GraphEventMapper.map(try decodeEvent(graphEvent(extra: extra)), calendar: laCalendar, accountEmail: "me@x.com", sourceID: "microsoft-c1"))
}

@Test func mapsATimedEvent() throws {
    let event = try map()
    #expect(event.eventID == "ev1" && event.uid == "uid-ev1" && event.uidScope == .global && event.calendarID == "cal1")
    #expect(event.title == "Standup" && event.notes == "Agenda" && event.location == "Room 1")
    #expect(event.start == Date(timeIntervalSince1970: 1_790_355_600) && event.end == event.start.addingTimeInterval(1800))
    #expect(event.timeZone.identifier == "America/Los_Angeles" && !event.isAllDay && event.status == .confirmed)
    #expect(event.availability == .busy && event.visibility == .default && event.kind == .standard)
    #expect(event.series == .notRecurring && event.version == "ck1" && event.sourceID == "microsoft-c1")
    #expect(event.lastModified == Date(timeIntervalSince1970: 1_790_251_200) && event.created != nil)
    #expect(event.url?.host == "outlook.office.com")
    #expect(event.reminders == [.before(minutes: 10)])
}

@Test func aTimedEventIsShownInTheZoneItWasScheduledIn() throws {
    let event = try map(["originalStartTimeZone": "W. Europe Standard Time"])
    #expect(event.timeZone.identifier == "Europe/Berlin")
    #expect(event.start == Date(timeIntervalSince1970: 1_790_355_600))   // the instant is unchanged
    let unknown = try map(["originalStartTimeZone": "Customized Time Zone"])
    #expect(unknown.timeZone.identifier == "America/Los_Angeles")
}

@Test func mapsAttendeesOrganizerAndParticipation() throws {
    let event = try map()
    #expect(event.attendees.map(\.email) == ["me@x.com", "opt@x.com", "room@x.com"])
    #expect(event.attendees.map(\.role) == [.required, .optional, .resource])
    #expect(event.attendees.map(\.response) == [.accepted, .tentative, .needsAction])
    #expect(event.attendees.map(\.isSelf) == [true, false, false])
    #expect(event.organizer?.email == "boss@x.com" && event.organizer?.isOrganizer == true && event.organizer?.isSelf == false)
    #expect(event.participation == .invited(.accepted))
}

@Test func anEventYouOrganizeWithNoGuestsCountsAsAccepted() throws {
    let event = try map(["isOrganizer": true, "attendees": [], "organizer": ["emailAddress": ["name": "Me", "address": "me@x.com"]]])
    #expect(event.participation == .invited(.accepted) && event.organizer?.isSelf == true)
    let stranger = try map(["attendees": []])
    #expect(stranger.participation == .notInvited)
}

@Test func mapsShowAsAndSensitivity() throws {
    for (raw, expected) in [("free", Availability.free), ("tentative", .tentative), ("oof", .unavailable), ("workingElsewhere", .free), ("busy", .busy)] {
        #expect(try map(["showAs": raw]).availability == expected)
    }
    for (raw, expected) in [("normal", Visibility.default), ("personal", .privateEvent), ("private", .privateEvent), ("confidential", .confidential)] {
        #expect(try map(["sensitivity": raw]).visibility == expected)
    }
}

@Test func anAllDayEventUsesTheCanonicalFormInTheAccountZone() throws {
    let event = try map([
        "isAllDay": true,
        "start": ["dateTime": "2026-09-25T00:00:00.0000000", "timeZone": "Pacific Standard Time"],
        "end": ["dateTime": "2026-09-27T00:00:00.0000000", "timeZone": "Pacific Standard Time"],
    ])
    #expect(event.isAllDay && event.timeZone.identifier == "America/Los_Angeles")
    #expect(AllDayConformance.violations(event).isEmpty)
    let dates = AllDay.dates(start: event.start, end: event.end, in: event.timeZone)
    #expect(dates.first == CalendarDate(year: 2026, month: 9, day: 25) && dates.endExclusive == CalendarDate(year: 2026, month: 9, day: 27))
}

@Test func anAllDayEventInUTCKeepsItsDates() throws {
    let utc = CalendarDescriptor(id: "cal1", title: "C", service: .microsoft, timeZone: TimeZone(identifier: "UTC")!)
    let dto = try decodeEvent(graphEvent(extra: [
        "isAllDay": true, "originalStartTimeZone": "UTC",
        "start": ["dateTime": "2026-09-25T00:00:00.0000000", "timeZone": "UTC"], "end": ["dateTime": "2026-09-26T00:00:00.0000000", "timeZone": "UTC"]]))
    let event = try #require(GraphEventMapper.map(dto, calendar: utc, accountEmail: nil))
    #expect(event.start == Date(timeIntervalSince1970: 1_790_294_400) && event.end == event.start.addingTimeInterval(86_400))
    #expect(AllDayConformance.violations(event).isEmpty)
}

@Test func aCancelledOrUnreadableEventMapsToNil() throws {
    #expect(GraphEventMapper.map(try decodeEvent(graphEvent(extra: ["isCancelled": true])), calendar: laCalendar, accountEmail: nil) == nil)
    #expect(GraphEventMapper.map(try decodeEvent(graphEvent(extra: ["start": ["dateTime": "junk"]])), calendar: laCalendar, accountEmail: nil) == nil)
}

@Test func mapsSeriesInstancesMastersAndExceptions() throws {
    let occurrence = try map(["type": "occurrence", "seriesMasterId": "master1", "originalStart": "2026-09-25T17:00:00Z"])
    #expect(occurrence.series == .occurrence(seriesID: "master1", originalStart: Date(timeIntervalSince1970: 1_790_355_600)))
    let exception = try map(["type": "exception", "seriesMasterId": "master1", "originalStart": "2026-09-24T17:00:00Z"])
    #expect(exception.seriesID == "master1" && exception.originalStart == Date(timeIntervalSince1970: 1_790_269_200))
    let master = try map(["type": "seriesMaster", "id": "master1"])
    #expect(master.series == .occurrence(seriesID: "master1", originalStart: master.start))
}

@Test func findsTheTeamsLinkInStructuredDataAndInTheNotes() throws {
    let structured = try map(["isOnlineMeeting": true, "onlineMeeting": ["joinUrl": "https://teams.microsoft.com/l/meetup-join/abc"]])
    #expect(structured.conferences.first?.provider == .teams && structured.conferences.first?.origin == .structured)
    let inNotes = try map(["body": ["contentType": "text", "content": "Join https://zoom.us/j/12345 please"]])
    #expect(inNotes.conferences.first?.provider == .zoom && inNotes.conferences.first?.origin == .notes)
    #expect(try map().conferences.isEmpty)
}

@Test func remindersFollowTheEventsFlag() throws {
    #expect(try map(["isReminderOn": false]).reminders == [])
    #expect(try map(["isReminderOn": true, "reminderMinutesBeforeStart": 0]).reminders == [.before(minutes: 0)])
}

@Test func reducesAnHTMLBodyToText() {
    let html = GraphBodyDTO(contentType: "html", content: "<html><body><p>Hello &amp; welcome</p><br/>Line two&nbsp;here</body></html>")
    #expect(GraphEventMapper.plainText(html) == "Hello & welcome\n\nLine two here")
    #expect(GraphEventMapper.plainText(GraphBodyDTO(contentType: "text", content: "  ")) == nil)
    #expect(GraphEventMapper.plainText(nil) == nil)
}

@Test func mapsACalendar() throws {
    let dto = try JSONDecoder().decode(GraphCalendarDTO.self, from: try JSONSerialization.data(withJSONObject: [
        "id": "cal1", "name": "Work", "hexColor": "#0078d4", "isDefaultCalendar": true, "canEdit": true, "canShare": true,
        "canViewPrivateItems": true, "owner": ["name": "Me", "address": "me@x.com"]]))
    let calendar = GraphEventMapper.descriptor(from: dto, accountName: "me@x.com", zone: la)
    #expect(calendar.id == "cal1" && calendar.title == "Work" && calendar.colorHex == "#0078D4" && calendar.isDefault == true)
    #expect(calendar.service == .microsoft && calendar.provider == .microsoft && calendar.accountName == "me@x.com")
    #expect(calendar.timeZone == la && calendar.accessRole == .owner)
    #expect(calendar.supportedAvailabilities == [.busy, .free, .tentative, .unavailable])
}

@Test func aReadOnlyCalendarAndMissingFlags() throws {
    let dto = try JSONDecoder().decode(GraphCalendarDTO.self, from: try JSONSerialization.data(withJSONObject: ["id": "c", "canEdit": false]))
    let calendar = GraphEventMapper.descriptor(from: dto, accountName: nil, zone: la)
    #expect(calendar.title == "c" && calendar.permissions.canEdit == false && calendar.permissions.canShare == false)
    #expect(calendar.accessRole == .reader && calendar.colorHex == nil && calendar.isDefault == false)
}
