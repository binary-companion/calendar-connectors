import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import ICalendar

private let september = DateInterval(start: laTime(2026, 9, 1, 0), end: laTime(2026, 10, 1, 0))

@Test func weeklySeriesExpandsWithExdateAndMovedOverride() throws {
    let events = EventReader.events(in: try resource(weeklySeries), overlapping: september, context: context())
    #expect(events.map(\.start) == [laTime(2026, 9, 1), laTime(2026, 9, 16, 14), laTime(2026, 9, 22), laTime(2026, 9, 29)])
    let moved = events[1]
    #expect(moved.title == "Team sync (moved)")
    #expect(moved.series == .occurrence(seriesID: "4F2A.ics", originalStart: laTime(2026, 9, 15)))
    #expect(moved.eventID == "4F2A.ics#20260915T170000Z")
    #expect(events[0].eventID == "4F2A.ics#20260901T170000Z")
    #expect(events[0].end == laTime(2026, 9, 1, 10, 30))
}

@Test func mapsEveryField() throws {
    let first = try #require(EventReader.events(in: try resource(weeklySeries), overlapping: september, context: context()).first)
    #expect(first.uid == "4F2A-UID")
    #expect(first.uidScope == .global)
    #expect(first.calendarID == "home")
    #expect(first.sourceID == "icloud-conn")
    #expect(first.title == "Team sync")
    #expect(first.notes == "Agenda\nhttps://zoom.us/j/123456789")
    #expect(first.location == "Room 4")
    #expect(first.timeZone.identifier == "America/Los_Angeles")
    #expect(!first.isAllDay)
    #expect(first.status == .confirmed)
    #expect(first.availability == .busy)
    #expect(first.visibility == .privateEvent)
    #expect(first.version == "\"e1\"")
    #expect(first.lastModified == Date(timeIntervalSince1970: 1_788_350_400))   // 2026-09-02T12:00:00Z
    #expect(first.created == Date(timeIntervalSince1970: 1_785_585_600))        // 2026-08-01T12:00:00Z
    #expect(first.conference?.provider == .zoom)
    #expect(first.reminders == [Reminder(trigger: .relative(offset: -900, to: .start), isCalendarDefault: true)])
}

@Test func attendeesSelfAndParticipation() throws {
    let first = try #require(EventReader.events(in: try resource(weeklySeries), overlapping: september, context: context()).first)
    #expect(first.organizer?.email == "me@icloud.test")
    #expect(first.organizer?.isSelf == true)
    #expect(first.attendees.map(\.email) == ["me@icloud.test", "ann@example.test", "room@example.test", nil])
    #expect(first.attendees.map(\.response) == [.accepted, .tentative, .accepted, .needsAction])
    #expect(first.attendees.map(\.role) == [.required, .required, .resource, .optional])
    #expect(first.attendees[0].isSelf && first.attendees[0].isOrganizer)
    #expect(first.attendees[3].name == "Bo")
    #expect(first.participation == .invited(.accepted))
}

@Test func noSelfAddressesGivesNilParticipation() throws {
    let first = try #require(EventReader.events(in: try resource(weeklySeries), overlapping: september,
                                                context: context(selfAddresses: [])).first)
    #expect(first.participation == nil)
    #expect(first.attendees.allSatisfy { !$0.isSelf })
}

@Test func overrideMovedIntoWindowIsShownAndItsSlotIsNot() throws {
    // A window holding only Wednesday the 16th: the override is in it; its slot (Tuesday the 15th) is not.
    let window = DateInterval(start: laTime(2026, 9, 16, 0), end: laTime(2026, 9, 17, 0))
    let events = EventReader.events(in: try resource(weeklySeries), overlapping: window, context: context())
    #expect(events.map(\.title) == ["Team sync (moved)"])
    // A window holding only Tuesday the 15th shows nothing: the occurrence moved away.
    let slot = DateInterval(start: laTime(2026, 9, 15, 0), end: laTime(2026, 9, 16, 0))
    #expect(EventReader.events(in: try resource(weeklySeries), overlapping: slot, context: context()).isEmpty)
}

@Test func overrideWithoutMasterIsShown() throws {
    let text = """
    BEGIN:VCALENDAR
    VERSION:2.0
    BEGIN:VEVENT
    UID:only-one
    RECURRENCE-ID:20260915T170000Z
    DTSTART:20260915T170000Z
    DTEND:20260915T180000Z
    SUMMARY:Just this one
    END:VEVENT
    END:VCALENDAR
    """
    let events = EventReader.events(in: try resource(text), overlapping: september, context: context("only.ics"))
    #expect(events.count == 1)
    #expect(events.first?.series == .occurrence(seriesID: "only.ics", originalStart: Date(timeIntervalSince1970: 1_789_491_600)))
    #expect(events.first?.timeZone.identifier == "UTC" || events.first?.timeZone.identifier == "GMT")
}

@Test func cancelledOverrideIsReturnedCancelled() throws {
    let text = weeklySeries.replacingOccurrences(of: "SUMMARY:Team sync (moved)", with: "SUMMARY:Team sync (moved)\nSTATUS:CANCELLED")
    let moved = try #require(EventReader.events(in: try resource(text), overlapping: september, context: context())
        .first { $0.title == "Team sync (moved)" })
    #expect(moved.status == .cancelled)
}

@Test func singleEventIsNotRecurring() throws {
    let text = """
    BEGIN:VCALENDAR
    VERSION:2.0
    BEGIN:VEVENT
    UID:single
    DTSTART;TZID=America/Los_Angeles:20260910T090000
    DURATION:PT45M
    SUMMARY:One off
    TRANSP:TRANSPARENT
    END:VEVENT
    END:VCALENDAR
    """
    let event = try #require(EventReader.events(in: try resource(text), overlapping: september, context: context("single.ics")).first)
    #expect(event.eventID == "single.ics")
    #expect(event.series == .notRecurring)
    #expect(event.end == laTime(2026, 9, 10, 9, 45))
    #expect(event.availability == .free)
    #expect(event.visibility == .default)
    #expect(event.reminders == [])
    #expect(event.participation == .notInvited)
}

@Test func allDayUsesTheCalendarZoneAndPassesConformance() throws {
    let text = """
    BEGIN:VCALENDAR
    VERSION:2.0
    BEGIN:VEVENT
    UID:day
    DTSTART;VALUE=DATE:20260910
    DTEND;VALUE=DATE:20260912
    RRULE:FREQ=WEEKLY;COUNT=2
    SUMMARY:Offsite
    END:VEVENT
    END:VCALENDAR
    """
    let events = EventReader.events(in: try resource(text), overlapping: september, context: context("day.ics"))
    #expect(events.count == 2)
    for event in events {
        #expect(event.isAllDay)
        #expect(AllDayConformance.violations(event) == [])
    }
    #expect(events[0].start == AllDay.startOfDay(CalendarDate(year: 2026, month: 9, day: 10), in: la))
    #expect(events[0].end == AllDay.startOfDay(CalendarDate(year: 2026, month: 9, day: 12), in: la))
    #expect(events[0].eventID == "day.ics#20260910")
}

@Test func unreadableRuleShowsFirstOccurrenceAndOverrides() throws {
    let text = weeklySeries.replacingOccurrences(of: "RRULE:FREQ=WEEKLY;BYDAY=TU", with: "RRULE:FREQ=FORTNIGHTLY")
    let events = EventReader.events(in: try resource(text), overlapping: september, context: context())
    #expect(events.map(\.start) == [laTime(2026, 9, 1), laTime(2026, 9, 16, 14)])
}

@Test func windowsTZIDIsResolved() throws {
    let text = weeklySeries.replacingOccurrences(of: "TZID=America/Los_Angeles", with: "TZID=Pacific Standard Time")
    let first = try #require(EventReader.events(in: try resource(text), overlapping: september, context: context()).first)
    #expect(first.start == laTime(2026, 9, 1))
}

@Test func masterOccurrenceAndSeriesLookups() throws {
    let r = try resource(weeklySeries)
    let master = try #require(EventReader.masterEvent(of: r, context: context()))
    #expect(master.eventID == "4F2A.ics")
    #expect(master.series == .occurrence(seriesID: "4F2A.ics", originalStart: laTime(2026, 9, 1)))
    #expect(EventReader.occurrence(in: r, originalStart: laTime(2026, 9, 22), context: context())?.eventID == "4F2A.ics#20260922T170000Z")
    #expect(EventReader.occurrence(in: r, originalStart: laTime(2026, 9, 15), context: context())?.title == "Team sync (moved)")
    #expect(EventReader.occurrence(in: r, originalStart: laTime(2026, 9, 8), context: context()) == nil)        // excluded
    #expect(EventReader.occurrence(in: r, originalStart: laTime(2026, 9, 23), context: context()) == nil)       // not a slot
    let series = try #require(EventReader.series(of: r, context: context()))
    #expect(series.seriesID == "4F2A.ics")
    #expect(series.recurrence.rules.first?.frequency == .weekly)
    #expect(series.recurrence.excludedDates == [laTime(2026, 9, 8)])
}

@Test func readEventsMeetTheProvidedFieldRule() throws {
    let capabilities = SourceCapabilities(providedFields: [.visibility, .availability, .reminders, .series, .participation, .version, .uidScope])
    for event in EventReader.events(in: try resource(weeklySeries), overlapping: september, context: context()) {
        #expect(ProvidedFieldsConformance.violations(event: event, capabilities: capabilities) == [])
    }
}
