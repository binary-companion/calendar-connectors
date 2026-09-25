import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import GoogleCalendar

private let tokyo = TimeZone(identifier: "Asia/Tokyo")!
private let calendar = CalendarDescriptor(id: "cal1", title: "Work", service: .google, permissions: CalendarPermissions(canViewDetails: true, canEdit: true, canShare: true, canViewPrivate: true), isDefault: true, timeZone: tokyo)

private func event(_ json: String) throws -> GoogleEventDTO {
    try JSONDecoder().decode(GoogleEventDTO.self, from: Data(json.utf8))
}
private func map(_ json: String) throws -> CalendarEvent? { GoogleEventMapper.map(try event(json), calendar: calendar) }
private func instant(_ s: String) -> Date { ISO8601DateFormatter().date(from: s)! }

@Test func mapsATimedEvent() throws {
    let e = try #require(try map("""
    {"id":"e1","iCalUID":"uid@google.com","status":"confirmed","summary":"Standup","description":"notes","location":"Room 1",
     "htmlLink":"https://www.google.com/calendar/event?eid=x","etag":"\\"123\\"",
     "start":{"dateTime":"2026-09-21T10:00:00-07:00","timeZone":"America/Los_Angeles"},
     "end":{"dateTime":"2026-09-21T10:30:00-07:00","timeZone":"America/Los_Angeles"}}
    """))
    #expect(e.eventID == "e1" && e.calendarID == "cal1" && e.id == "cal1/e1")
    #expect(e.uid == "uid@google.com")
    #expect(e.title == "Standup" && e.notes == "notes" && e.location == "Room 1")
    #expect(e.start == instant("2026-09-21T10:00:00-07:00") && e.end == instant("2026-09-21T10:30:00-07:00"))
    #expect(e.timeZone.identifier == "America/Los_Angeles")
    #expect(!e.isAllDay && e.status == .confirmed && e.availability == .busy && e.kind == .standard)
    #expect(e.url?.absoluteString == "https://www.google.com/calendar/event?eid=x")
    #expect(e.version == "\"123\"")
    #expect(e.myResponse == nil && e.attendees.isEmpty)
}

@Test func allDayEventsUseMidnightInTheCalendarZoneWithExclusiveEnd() throws {
    let e = try #require(try map(#"{"id":"a","summary":"Off","start":{"date":"2026-09-20"},"end":{"date":"2026-09-22"}}"#))
    var cal = Calendar(identifier: .gregorian)
    cal.timeZone = tokyo
    #expect(e.isAllDay)
    #expect(AllDayConformance.violations(e).isEmpty)
    #expect(e.timeZone.identifier == "Asia/Tokyo")
    #expect(e.start == cal.date(from: DateComponents(year: 2026, month: 9, day: 20)))
    #expect(e.end == cal.date(from: DateComponents(year: 2026, month: 9, day: 22)))
}

@Test func allDayFallsBackToUTCWhenTheCalendarHasNoZone() throws {
    let bare = CalendarDescriptor(id: "c", title: "C", service: .google)
    let dto = try event(#"{"id":"a","start":{"date":"2026-09-20"},"end":{"date":"2026-09-21"}}"#)
    let e = try #require(GoogleEventMapper.map(dto, calendar: bare))
    #expect(e.timeZone.identifier == "UTC" || e.timeZone.identifier == "GMT")
    #expect(e.start == instant("2026-09-20T00:00:00Z"))
    #expect(AllDayConformance.violations(e).isEmpty)
}

@Test func aTimedEventWithoutItsOwnZoneTakesTheCalendarsZoneThenUTC() throws {
    let noZone = #"{"id":"t","start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"}}"#
    #expect(try #require(try map(noZone)).timeZone.identifier == "Asia/Tokyo")              // the fixture calendar's zone
    let bare = CalendarDescriptor(id: "c", title: "C", service: .google)
    let dto = try event(noZone)
    let e = try #require(GoogleEventMapper.map(dto, calendar: bare))
    #expect(e.timeZone.identifier == "UTC" || e.timeZone.identifier == "GMT")
    let own = #"{"id":"t","start":{"dateTime":"2026-09-21T10:00:00-07:00","timeZone":"America/Los_Angeles"},"end":{"dateTime":"2026-09-21T11:00:00-07:00","timeZone":"America/Los_Angeles"}}"#
    #expect(try #require(try map(own)).timeZone.identifier == "America/Los_Angeles")
}

@Test func allDayInTokyoIsCanonical() throws {
    let e = try #require(try map(#"{"id":"a","summary":"Off","start":{"date":"2026-09-18"},"end":{"date":"2026-09-19"}}"#))
    #expect(e.start == instant("2026-09-17T15:00:00Z"))
    #expect(e.end == instant("2026-09-18T15:00:00Z"))
    #expect(e.timeZone.identifier == "Asia/Tokyo")
    #expect(AllDayConformance.violations(e).isEmpty)
}

@Test func cancelledEventsAreDropped() throws {
    #expect(try map(#"{"id":"x","status":"cancelled","start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"}}"#) == nil)
}

@Test func mapsAttendeesOrganizerAndMyResponse() throws {
    let e = try #require(try map("""
    {"id":"m","summary":"Sync","start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"},
     "organizer":{"email":"Boss@x.com","displayName":"Boss"},
     "attendees":[{"email":"Boss@x.com","responseStatus":"accepted","organizer":true},
                  {"email":"me@x.com","responseStatus":"declined","self":true},
                  {"email":"opt@x.com","optional":true,"responseStatus":"tentative"},
                  {"email":"room@x.com","resource":true}]}
    """))
    #expect(e.attendees.count == 4)
    #expect(e.organizer?.email == "boss@x.com" && e.organizer?.isOrganizer == true)
    #expect(e.myResponse == .declined)
    #expect(e.attendees[1].isSelf)
    #expect(e.attendees[2].role == .optional && e.attendees[2].response == .tentative)
    #expect(e.attendees[3].role == .resource && e.attendees[3].response == .needsAction)
}

@Test func mapsConferenceFromEntryPointsThenHangoutLink() throws {
    let a = try #require(try map("""
    {"id":"c1","start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"},
     "conferenceData":{"entryPoints":[{"entryPointType":"phone","uri":"tel:+1"},{"entryPointType":"video","uri":"https://meet.google.com/abc-defg-hij"}],
       "conferenceSolution":{"key":{"type":"hangoutsMeet"},"name":"Google Meet"}}}
    """))
    #expect(a.conference?.url.absoluteString == "https://meet.google.com/abc-defg-hij")
    #expect(a.conference?.provider == .meet)
    let b = try #require(try map("""
    {"id":"c2","start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"},
     "conferenceData":{"entryPoints":[{"entryPointType":"video","uri":"https://acme.zoom.us/j/123"}],"conferenceSolution":{"key":{"type":"addOn"},"name":"Zoom Meeting"}}}
    """))
    #expect(b.conference?.provider == .zoom)
    let c = try #require(try map(#"{"id":"c3","hangoutLink":"https://meet.google.com/zzz","start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"}}"#))
    #expect(c.conference?.provider == .meet)
}

@Test func aTeamsLinkOnlyInTheDescriptionIsFound() throws {
    // An invite imported from ICS carries its link in the description, not in `conferenceData`.
    let e = try #require(try map("""
    {"id":"c4","description":"Join https://teams.microsoft.com/l/meetup-join/19%3ameeting_x/0","start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"}}
    """))
    #expect(e.conferences.map(\.provider) == [.teams] && e.conferences[0].origin == .notes)
}

@Test func structuredLinksComeBeforeOnesFoundInTheDescription() throws {
    let e = try #require(try map("""
    {"id":"c5","description":"Backup: https://acme.zoom.us/j/9","hangoutLink":"https://meet.google.com/aaa-bbbb-ccc",
     "start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"},
     "htmlLink":"https://www.google.com/calendar/event?eid=abc"}
    """))
    #expect(e.conferences.map(\.provider) == [.meet, .zoom])
    #expect(e.conferences.map(\.origin) == [.structured, .notes])
}

@Test func everyVideoEntryPointIsKept() throws {
    let e = try #require(try map("""
    {"id":"c6","start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"},
     "conferenceData":{"entryPoints":[{"entryPointType":"video","uri":"https://acme.zoom.us/j/1"},{"entryPointType":"video","uri":"https://meet.google.com/aaa-bbbb-ccc"}]}}
    """))
    #expect(e.conferences.map(\.provider) == [.zoom, .meet])
}

@Test func mapsKindAvailabilityVisibilityRemindersAndSeries() throws {
    let e = try #require(try map("""
    {"id":"r_20260921","recurringEventId":"r","eventType":"focusTime","transparency":"transparent","visibility":"private",
     "originalStartTime":{"dateTime":"2026-09-21T10:00:00Z"},
     "reminders":{"useDefault":false,"overrides":[{"method":"popup","minutes":10},{"method":"email","minutes":60}]},
     "start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"}}
    """))
    #expect(e.kind == .focusTime && e.availability == .free && e.visibility == .privateEvent)
    #expect(e.seriesID == "r" && e.originalStart == instant("2026-09-21T10:00:00Z"))
    #expect(e.reminders == [.before(minutes: 10, isCalendarDefault: false), .before(minutes: 60, type: .email(address: nil), isCalendarDefault: false)])
}

@Test func toleratesFractionalSecondsAndMissingTitle() throws {
    let e = try #require(try map(#"{"id":"f","start":{"dateTime":"2026-09-21T10:00:00.500Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"}}"#))
    #expect(e.title == "(No title)")
    #expect(e.start == instant("2026-09-21T10:00:00Z").addingTimeInterval(0.5))
}

@Test func eventsWithoutUsableTimesAreDropped() throws {
    #expect(try map(#"{"id":"bad","start":{"dateTime":"garbage"},"end":{"dateTime":"garbage"}}"#) == nil)
    #expect(try map(#"{"id":"bad2"}"#) == nil)
}

@Test func mapsCalendarListEntries() throws {
    let dto = try JSONDecoder().decode(GoogleCalendarListEntryDTO.self, from: Data("""
    {"id":"me@x.com","summary":"me@x.com","summaryOverride":"Personal","backgroundColor":"#9fe1e7","accessRole":"owner","primary":true,"timeZone":"Asia/Tokyo"}
    """.utf8))
    let d = try #require(GoogleEventMapper.descriptor(from: dto, accountName: "me@x.com"))
    #expect(d.id == "me@x.com" && d.title == "Personal" && d.colorHex == "#9FE1E7")
    #expect(d.accessRole == .owner && d.isDefault == true && d.timeZone?.identifier == "Asia/Tokyo" && d.accountName == "me@x.com")
    let hidden = try JSONDecoder().decode(GoogleCalendarListEntryDTO.self, from: Data(#"{"id":"h","hidden":true}"#.utf8))
    #expect(GoogleEventMapper.descriptor(from: hidden, accountName: nil) == nil)
    let deleted = try JSONDecoder().decode(GoogleCalendarListEntryDTO.self, from: Data(#"{"id":"d","deleted":true}"#.utf8))
    #expect(GoogleEventMapper.descriptor(from: deleted, accountName: nil) == nil)
}

@Test func googleBirthdayAndHolidayCalendarsAreNotStandard() throws {
    func kind(_ id: String) throws -> CalendarKind {
        let dto = try JSONDecoder().decode(GoogleCalendarListEntryDTO.self, from: Data(#"{"id":"\#(id)","summary":"x"}"#.utf8))
        return try #require(GoogleEventMapper.descriptor(from: dto, accountName: nil)).kind
    }
    #expect(try kind("addressbook#contacts@group.v.calendar.google.com") == .birthdays)
    #expect(try kind("en.usa#holiday@group.v.calendar.google.com") == .subscribed)
    #expect(try kind("me@x.com") == .standard)
    #expect(try kind("abc123@group.calendar.google.com") == .standard)
}

// Provided fields (Issue 2).

private let withDefaults = CalendarDescriptor(id: "cal1", title: "Work", service: .google, permissions: CalendarPermissions(canViewDetails: true, canEdit: true, canShare: true, canViewPrivate: true), timeZone: tokyo,
    defaultReminders: [Reminder(minutesBefore: 30), Reminder(minutesBefore: 5)])
private let times = #""start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"}"#

@Test func useDefaultRemindersResolveToTheCalendarsDefaults() throws {
    let dto = try event(#"{"id":"d","reminders":{"useDefault":true},\#(times)}"#)
    #expect(GoogleEventMapper.map(dto, calendar: withDefaults)?.reminders == [.before(minutes: 30, isCalendarDefault: true), .before(minutes: 5, isCalendarDefault: true)])
    let none = try event(#"{"id":"n","reminders":{"useDefault":false},\#(times)}"#)
    #expect(GoogleEventMapper.map(none, calendar: withDefaults)?.reminders == [])
}

@Test func aRecurringInstanceIsAnOccurrenceAndASingleEventIsNotRecurring() throws {
    let instance = try #require(try map(#"{"id":"i","recurringEventId":"master","originalStartTime":{"dateTime":"2026-09-21T10:00:00Z"},\#(times)}"#))
    #expect(instance.series == .occurrence(seriesID: "master", originalStart: instant("2026-09-21T10:00:00Z")))
    let single = try #require(try map(#"{"id":"s",\#(times)}"#))
    #expect(single.series == .notRecurring)
}

@Test func participationSeparatesInvitedFromNotInvited() throws {
    let selfAttendee = try #require(try map(#"{"id":"a","attendees":[{"email":"me@x.com","self":true,"responseStatus":"tentative"}],\#(times)}"#))
    #expect(selfAttendee.participation == .invited(.tentative))
    let organizerOnly = try #require(try map(#"{"id":"b","organizer":{"email":"me@x.com","self":true},\#(times)}"#))
    #expect(organizerOnly.participation == .invited(.accepted))
    let someoneElses = try #require(try map(#"{"id":"c","organizer":{"email":"boss@x.com"},"attendees":[{"email":"boss@x.com"}],\#(times)}"#))
    #expect(someoneElses.participation == .notInvited)
}

@Test func everyFieldGoogleDeclaresIsPresent() throws {
    let capabilities = SourceCapabilities(providedFields: [.kind, .visibility, .availability, .reminders, .series, .participation, .structuredConference, .version])
    let minimal = try #require(try map(#"{"id":"m","etag":"\"1\"",\#(times)}"#))
    #expect(ProvidedFieldsConformance.violations(event: minimal, capabilities: capabilities).isEmpty)
    let dto = try event(#"{"id":"r","etag":"\"2\"","reminders":{"useDefault":true},\#(times)}"#)
    #expect(ProvidedFieldsConformance.violations(event: try #require(GoogleEventMapper.map(dto, calendar: withDefaults)), capabilities: capabilities).isEmpty)
}

@Test func calendarListEntriesCarryTheirDefaultReminders() throws {
    let dto = try JSONDecoder().decode(GoogleCalendarListEntryDTO.self, from: Data(#"{"id":"c","defaultReminders":[{"method":"popup","minutes":10}]}"#.utf8))
    #expect(GoogleEventMapper.descriptor(from: dto, accountName: nil)?.defaultReminders == [.before(minutes: 10, isCalendarDefault: true)])
}

// Calendar identity and permissions (Issues 6 and 7).

private func descriptor(_ json: String) throws -> CalendarDescriptor {
    let dto = try JSONDecoder().decode(GoogleCalendarListEntryDTO.self, from: Data(json.utf8))
    return try #require(GoogleEventMapper.descriptor(from: dto, accountName: "me@x.com"))
}

@Test func googleRolesMapToPermissions() throws {
    let owner = try descriptor(#"{"id":"a","accessRole":"owner"}"#).permissions
    #expect(owner == CalendarPermissions(canViewDetails: true, canEdit: true, canShare: true, canViewPrivate: true))
    let writer = try descriptor(#"{"id":"a","accessRole":"writer"}"#).permissions
    #expect(writer == CalendarPermissions(canViewDetails: true, canEdit: true, canShare: false, canViewPrivate: true))
    let reader = try descriptor(#"{"id":"a","accessRole":"reader"}"#).permissions
    #expect(reader == CalendarPermissions(canViewDetails: true, canEdit: false, canShare: false, canViewPrivate: false))
    let busy = try descriptor(#"{"id":"a","accessRole":"freeBusyReader"}"#).permissions
    #expect(busy == CalendarPermissions(canViewDetails: false, canEdit: false, canShare: false, canViewPrivate: false))
    #expect(try descriptor(#"{"id":"a","accessRole":"owner"}"#).accessRole == .owner)
}

@Test func googleCalendarsCarryServiceProviderDefaultAndAvailabilities() throws {
    let primary = try descriptor(#"{"id":"me@x.com","primary":true,"accessRole":"owner","timeZone":"UTC"}"#)
    #expect(primary.service == .google && primary.provider == .google && primary.isDefault == true)
    #expect(primary.supportedAvailabilities == [.busy, .free])
    #expect(try descriptor(#"{"id":"b","accessRole":"reader"}"#).isDefault == false)
    let holidays = try descriptor(#"{"id":"en.usa#holiday@group.v.calendar.google.com","accessRole":"reader"}"#)
    #expect(holidays.provider == .subscription)
}

@Test func everyCalendarFieldGoogleDeclaresIsPresent() throws {
    let capabilities = SourceCapabilities(providedFields: [.isDefault, .calendarTimeZone, .defaultReminders, .provider, .supportedAvailabilities, .permissionDetails])
    let d = try descriptor(#"{"id":"me@x.com","primary":true,"accessRole":"owner","timeZone":"UTC"}"#)
    #expect(ProvidedFieldsConformance.violations(calendar: d, capabilities: capabilities).isEmpty)
    let bare = CalendarDescriptor(id: "c", title: "C", service: .eventKit)
    #expect(ProvidedFieldsConformance.violations(calendar: bare, capabilities: capabilities).count == 6)
}

@Test func eventsCarryLastModifiedAndCreatedDates() throws {
    let e = try #require(try map(#"{"id":"u","updated":"2026-09-20T08:00:00.123Z","created":"2026-09-01T00:00:00Z",\#(times)}"#))
    #expect(e.created == instant("2026-09-01T00:00:00Z"))
    #expect(abs(try #require(e.lastModified).timeIntervalSince(instant("2026-09-20T08:00:00Z")) - 0.123) < 0.001)
    #expect(try #require(try map(#"{"id":"v",\#(times)}"#)).lastModified == nil)
}

@Test func aReminderMethodOtherThanPopupOrEmailIsKept() throws {
    let e = try #require(try map(#"{"id":"m","reminders":{"useDefault":false,"overrides":[{"method":"sms","minutes":5}]},\#(times)}"#))
    #expect(e.reminders == [.before(minutes: 5, type: .other("sms"), isCalendarDefault: false)])
}
