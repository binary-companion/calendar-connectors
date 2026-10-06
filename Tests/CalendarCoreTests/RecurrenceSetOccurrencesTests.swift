import CalendarCore
import Foundation
import Testing

private let ny = TimeZone(identifier: "America/New_York")!
private func local(_ y: Int, _ mo: Int, _ d: Int, _ h: Int = 9, _ mi: Int = 0) -> Date {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = ny
    return calendar.date(from: DateComponents(year: y, month: mo, day: d, hour: h, minute: mi))!
}

@Test func windowKeepsOnlyOverlappingInstances() throws {
    let set = RecurrenceSet(rules: [try RecurrenceRule(rrule: "FREQ=DAILY")])
    let result = set.occurrences(anchor: local(2026, 9, 1), duration: 3600, timeZone: ny, isAllDay: false,
                                 overlapping: DateInterval(start: local(2026, 9, 10, 9, 30), end: local(2026, 9, 12, 9)))
    // 10th 09:00-10:00 overlaps (started before the window), 11th inside, 12th starts at the end (excluded).
    #expect(result.starts == [local(2026, 9, 10), local(2026, 9, 11)])
    #expect(!result.truncated)
}

@Test func exdatesRemoveAndRdatesAdd() throws {
    let set = RecurrenceSet(rules: [try RecurrenceRule(rrule: "FREQ=DAILY;COUNT=5")],
                            extraDates: [local(2026, 9, 20)], excludedDates: [local(2026, 9, 3)])
    let result = set.occurrences(anchor: local(2026, 9, 1), duration: 1800, timeZone: ny, isAllDay: false,
                                 overlapping: DateInterval(start: local(2026, 9, 1, 0), end: local(2026, 10, 1)))
    #expect(result.starts == [local(2026, 9, 1), local(2026, 9, 2), local(2026, 9, 4), local(2026, 9, 5), local(2026, 9, 20)])
}

@Test func allDayExdateMatchesByDay() throws {
    let day = { (d: Int) in AllDay.startOfDay(CalendarDate(year: 2026, month: 9, day: d), in: ny)! }
    let set = RecurrenceSet(rules: [try RecurrenceRule(rrule: "FREQ=DAILY;COUNT=3")], excludedDates: [day(2)])
    let result = set.occurrences(anchor: day(1), duration: 86_400, timeZone: ny, isAllDay: true,
                                 overlapping: DateInterval(start: day(1), end: day(10)))
    #expect(result.starts == [day(1), day(3)])
}

@Test func oldDailySeriesStillShowsThisWeek() throws {
    let set = RecurrenceSet(rules: [try RecurrenceRule(rrule: "FREQ=DAILY")])
    let result = set.occurrences(anchor: local(2015, 1, 1), duration: 1800, timeZone: ny, isAllDay: false,
                                 overlapping: DateInterval(start: local(2026, 9, 27, 0), end: local(2026, 10, 4, 0)))
    #expect(result.starts.count == 7)
    #expect(result.starts.first == local(2026, 9, 27))
    #expect(!result.truncated)
}

@Test func oldWeeklySeriesWithIntervalStillShowsThisMonth() throws {
    let set = RecurrenceSet(rules: [try RecurrenceRule(rrule: "FREQ=WEEKLY;INTERVAL=2;BYDAY=TU")])
    let anchor = local(2015, 1, 6)   // a Tuesday
    let result = set.occurrences(anchor: anchor, duration: 1800, timeZone: ny, isAllDay: false,
                                 overlapping: DateInterval(start: local(2026, 9, 1, 0), end: local(2026, 10, 1, 0)))
    // Every second Tuesday from 2015-01-06: 2026-09-01 is 608 weeks later (even), so 09-01, 09-15 and 09-29.
    #expect(result.starts == [local(2026, 9, 1), local(2026, 9, 15), local(2026, 9, 29)])
}

@Test func largeCountSeriesStillShowsTheWindow() throws {
    // COUNT cannot skip ahead, so the years of instances before the window must not use up the limit.
    let set = RecurrenceSet(rules: [try RecurrenceRule(rrule: "FREQ=DAILY;COUNT=10000")])
    let result = set.occurrences(anchor: local(2015, 1, 1), duration: 1800, timeZone: ny, isAllDay: false,
                                 overlapping: DateInterval(start: local(2026, 9, 27, 0), end: local(2026, 10, 4, 0)))
    #expect(result.starts.count == 7)
    #expect(!result.truncated)
}

@Test func unreadableRuleIsReported() {
    let set = RecurrenceSet(iCalendarLines: ["RRULE:FREQ=FORTNIGHTLY"], timeZone: ny, isAllDay: false)
    let result = set.occurrences(anchor: local(2026, 9, 1), duration: 1800, timeZone: ny, isAllDay: false,
                                 overlapping: DateInterval(start: local(2026, 9, 1, 0), end: local(2026, 10, 1)))
    #expect(result.hasUnreadableRule)
    #expect(result.starts == [local(2026, 9, 1)])
}

@Test func limitIsReported() throws {
    let set = RecurrenceSet(rules: [try RecurrenceRule(rrule: "FREQ=MINUTELY")])
    let result = set.occurrences(anchor: local(2026, 9, 1), duration: 60, timeZone: ny, isAllDay: false,
                                 overlapping: DateInterval(start: local(2026, 9, 1), end: local(2026, 9, 30)), limit: 100)
    #expect(result.starts.count == 100)
    #expect(result.truncated)
}

@Test func ruleInstanceCountIncludesAnchorAndExcludedDates() throws {
    let set = RecurrenceSet(rules: [try RecurrenceRule(rrule: "FREQ=WEEKLY;COUNT=10")], excludedDates: [local(2026, 9, 8)])
    // Instances before 09-22: 09-01, 09-08 (excluded but counted, as RFC 5545 COUNT does), 09-15.
    #expect(set.ruleInstanceCount(anchor: local(2026, 9, 1), timeZone: ny, isAllDay: false, before: local(2026, 9, 22)) == 3)
}

@Test func aSetWithAnUnparsedRuleReportsItAsUnreadable() {
    #expect(RecurrenceSet(rules: [], unparsed: ["RRULE:INTERVAL=2"]).hasUnreadableRule)
    #expect(!RecurrenceSet(rules: [], unparsed: ["EXRULE:FREQ=DAILY"]).hasUnreadableRule)
    #expect(!RecurrenceSet(rules: []).hasUnreadableRule)
}
