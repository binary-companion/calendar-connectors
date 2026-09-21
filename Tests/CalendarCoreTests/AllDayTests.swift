import Foundation
import Testing
@testable import CalendarCore

private func zone(_ id: String) -> TimeZone { TimeZone(identifier: id)! }
private func iso(_ s: String) -> Date { ISO8601DateFormatter().date(from: s)! }

@Test func canonicalOneDayInNewYork() throws {
    let r = try #require(AllDay.canonical(
        first: CalendarDate(year: 2026, month: 9, day: 18), endExclusive: CalendarDate(year: 2026, month: 9, day: 19),
        in: zone("America/New_York")))
    #expect(r.start == iso("2026-09-18T04:00:00Z"))
    #expect(r.end == iso("2026-09-19T04:00:00Z"))
}

@Test func datesRoundTripInAnotherZone() throws {
    let tokyo = zone("Asia/Tokyo")
    let r = try #require(AllDay.canonical(
        first: CalendarDate(year: 2026, month: 9, day: 18), endExclusive: CalendarDate(year: 2026, month: 9, day: 20), in: tokyo))
    let d = AllDay.dates(start: r.start, end: r.end, in: tokyo)
    #expect(d.first == CalendarDate(year: 2026, month: 9, day: 18))
    #expect(d.endExclusive == CalendarDate(year: 2026, month: 9, day: 20))
}

@Test func startOfDayWhenMidnightDoesNotExist() throws {
    // Sao Paulo skipped 00:00 on 2018-11-04 (clocks jumped to 01:00).
    let start = try #require(AllDay.startOfDay(CalendarDate(year: 2018, month: 11, day: 4), in: zone("America/Sao_Paulo")))
    #expect(start == iso("2018-11-04T03:00:00Z"))
}

@Test func addingDaysCrossesMonthsAndLeapDays() {
    #expect(CalendarDate(year: 2026, month: 2, day: 28).adding(days: 1) == CalendarDate(year: 2026, month: 3, day: 1))
    #expect(CalendarDate(year: 2028, month: 2, day: 28).adding(days: 1) == CalendarDate(year: 2028, month: 2, day: 29))
    #expect(AllDay.endExclusive(afterLast: CalendarDate(year: 2026, month: 12, day: 31)) == CalendarDate(year: 2027, month: 1, day: 1))
}

@Test func needsPermissionIsADistinctError() {
    #expect(SourceError.needsPermission != SourceError.authExpired)
}

@Test func calendarDatesOrderChronologically() {
    #expect(CalendarDate(year: 2026, month: 9, day: 20) < CalendarDate(year: 2026, month: 9, day: 21))
    #expect(CalendarDate(year: 2026, month: 12, day: 31) < CalendarDate(year: 2027, month: 1, day: 1))
    #expect(!(CalendarDate(year: 2026, month: 9, day: 21) < CalendarDate(year: 2026, month: 9, day: 21)))
}
