import Foundation
import Testing
@testable import CalendarCore

private let newYork = TimeZone(identifier: "America/New_York")!
private let utc = TimeZone(identifier: "UTC")!
private func instant(_ s: String) -> Date { ISO8601DateFormatter().date(from: s)! }

@Test func aSeriesWithRuleExdateAndRdateIsReadWithEveryDateForm() {
    let lines = [
        "RRULE:FREQ=WEEKLY;BYDAY=MO",
        "EXDATE;TZID=America/New_York:20260921T100000,20260928T100000",   // a named zone
        "EXDATE:20261005T140000Z",                                        // UTC
        "EXDATE:20261012T100000",                                         // floating: read in the series zone
        "RDATE;TZID=America/New_York:20261001T090000",
    ]
    let set = RecurrenceSet(iCalendarLines: lines, timeZone: newYork, isAllDay: false)
    #expect(set.rules.count == 1 && set.rules[0].frequency == .weekly)
    #expect(set.excludedDates == [instant("2026-09-21T14:00:00Z"), instant("2026-09-28T14:00:00Z"), instant("2026-10-05T14:00:00Z"), instant("2026-10-12T14:00:00Z")])
    #expect(set.extraDates == [instant("2026-10-01T13:00:00Z")])
    #expect(set.unparsed.isEmpty)
}

@Test func anAllDaySeriesReadsValueDateThroughTheCanonicalMidnight() {
    let set = RecurrenceSet(iCalendarLines: ["RRULE:FREQ=DAILY", "EXDATE;VALUE=DATE:20260921,20260922"], timeZone: newYork, isAllDay: true)
    let first = AllDay.startOfDay(CalendarDate(year: 2026, month: 9, day: 21), in: newYork)!
    let second = AllDay.startOfDay(CalendarDate(year: 2026, month: 9, day: 22), in: newYork)!
    #expect(set.excludedDates == [first, second])
}

@Test func linesTheLibraryDoesNotModelAreKeptVerbatim() {
    let lines = [
        "EXRULE:FREQ=WEEKLY", "RDATE;VALUE=PERIOD:19960403T020000Z/19960403T040000Z",
        "EXDATE;TZID=W. Europe Standard Time:20260921T100000",   // a Windows zone name
        "RRULE:FREQ=NEVER", "X-CUSTOM:1", "RRULE:FREQ=DAILY",
    ]
    let set = RecurrenceSet(iCalendarLines: lines, timeZone: utc, isAllDay: false)
    #expect(set.unparsed == ["EXRULE:FREQ=WEEKLY", "RDATE;VALUE=PERIOD:19960403T020000Z/19960403T040000Z",
                             "EXDATE;TZID=W. Europe Standard Time:20260921T100000", "RRULE:FREQ=NEVER", "X-CUSTOM:1"])
    #expect(set.rules.count == 1 && set.excludedDates == [] && set.extraDates == [])
}

@Test func linesRoundTrip() {
    let lines = [
        "RRULE:FREQ=WEEKLY;BYDAY=MO", "RDATE;TZID=America/New_York:20261001T090000",
        "EXDATE;TZID=America/New_York:20260921T100000,20260928T100000", "EXRULE:FREQ=DAILY",
    ]
    let set = RecurrenceSet(iCalendarLines: lines, timeZone: newYork, isAllDay: false)
    #expect(set.iCalendarLines(timeZone: newYork, isAllDay: false) == lines)
    let allDay = ["RRULE:FREQ=DAILY", "EXDATE;VALUE=DATE:20260921"]
    #expect(RecurrenceSet(iCalendarLines: allDay, timeZone: newYork, isAllDay: true).iCalendarLines(timeZone: newYork, isAllDay: true) == allDay)
    let inUTC = ["RRULE:FREQ=DAILY", "EXDATE:20260921T100000Z"]
    #expect(RecurrenceSet(iCalendarLines: inUTC, timeZone: utc, isAllDay: false).iCalendarLines(timeZone: utc, isAllDay: false) == inUTC)
}

@Test func aSourceThatCannotListExtraOrSkippedDatesUsesNil() {
    let set = RecurrenceSet(rules: [RecurrenceRule(frequency: .daily)], extraDates: nil, excludedDates: nil)
    #expect(set.extraDates == nil && set.excludedDates == nil)
    #expect(set.iCalendarLines(timeZone: utc, isAllDay: false) == ["RRULE:FREQ=DAILY"])
}
