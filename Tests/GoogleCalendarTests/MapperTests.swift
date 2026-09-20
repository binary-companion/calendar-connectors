import CalendarCore
import Foundation
import Testing
@testable import GoogleCalendar

private let tokyo = TimeZone(identifier: "Asia/Tokyo")!
private let calendar = CalendarDescriptor(id: "cal1", title: "Work", accessRole: .owner, isPrimary: true, timeZone: tokyo)

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
    #expect(e.timeZone?.identifier == "America/Los_Angeles")
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
    #expect(e.timeZone?.identifier == "Asia/Tokyo")
    #expect(e.start == cal.date(from: DateComponents(year: 2026, month: 9, day: 20)))
    #expect(e.end == cal.date(from: DateComponents(year: 2026, month: 9, day: 22)))
}

@Test func allDayFallsBackToUTCWhenTheCalendarHasNoZone() throws {
    let bare = CalendarDescriptor(id: "c", title: "C")
    let dto = try event(#"{"id":"a","start":{"date":"2026-09-20"},"end":{"date":"2026-09-21"}}"#)
    let e = try #require(GoogleEventMapper.map(dto, calendar: bare))
    #expect(e.timeZone?.identifier == "UTC" || e.timeZone?.identifier == "GMT")
    #expect(e.start == instant("2026-09-20T00:00:00Z"))
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

@Test func mapsKindAvailabilityVisibilityRemindersAndSeries() throws {
    let e = try #require(try map("""
    {"id":"r_20260921","recurringEventId":"r","eventType":"focusTime","transparency":"transparent","visibility":"private",
     "originalStartTime":{"dateTime":"2026-09-21T10:00:00Z"},
     "reminders":{"useDefault":false,"overrides":[{"method":"popup","minutes":10},{"method":"email","minutes":60}]},
     "start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"}}
    """))
    #expect(e.kind == .focusTime && e.availability == .free && e.visibility == .privateEvent)
    #expect(e.seriesID == "r" && e.originalStart == instant("2026-09-21T10:00:00Z"))
    #expect(e.reminders == [Reminder(minutesBefore: 10), Reminder(minutesBefore: 60)])
}

@Test func toleratesFractionalSecondsAndMissingTitle() throws {
    let e = try #require(try map(#"{"id":"f","start":{"dateTime":"2026-09-21T10:00:00.500Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"}}"#))
    #expect(e.title == "")
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
    #expect(d.accessRole == .owner && d.isPrimary && d.timeZone?.identifier == "Asia/Tokyo" && d.accountName == "me@x.com")
    let hidden = try JSONDecoder().decode(GoogleCalendarListEntryDTO.self, from: Data(#"{"id":"h","hidden":true}"#.utf8))
    #expect(GoogleEventMapper.descriptor(from: hidden, accountName: nil) == nil)
    let deleted = try JSONDecoder().decode(GoogleCalendarListEntryDTO.self, from: Data(#"{"id":"d","deleted":true}"#.utf8))
    #expect(GoogleEventMapper.descriptor(from: deleted, accountName: nil) == nil)
}
