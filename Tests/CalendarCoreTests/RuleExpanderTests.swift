import CalendarCore
import Foundation
import Testing

private let ny = TimeZone(identifier: "America/New_York")!

private func local(_ y: Int, _ mo: Int, _ d: Int, _ h: Int = 9, _ mi: Int = 0, _ s: Int = 0, zone: TimeZone = ny) -> Date {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = zone
    return calendar.date(from: DateComponents(year: y, month: mo, day: d, hour: h, minute: mi, second: s))!
}

/// Local "yyyy-MM-dd HH:mm" texts, easy to compare with the RFC's listings.
private func texts(_ dates: [Date], zone: TimeZone = ny) -> [String] {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = zone
    return dates.map {
        let c = calendar.dateComponents([.year, .month, .day, .hour, .minute], from: $0)
        return String(format: "%04d-%02d-%02d %02d:%02d", c.year!, c.month!, c.day!, c.hour!, c.minute!)
    }
}

private func expand(_ rule: String, _ anchor: Date, before: Date = local(2050, 1, 1), limit: Int = 5000, skipTo: Date? = nil) throws -> [String] {
    let r = try RecurrenceRule(rrule: rule, in: ny)
    return texts(r.instances(anchor: anchor, timeZone: ny, isAllDay: false, before: before, limit: limit, skipTo: skipTo).starts)
}

private func days(_ list: [String]) -> [String] { list.map { String($0.prefix(10)) } }

@Test func dailyForTenOccurrences() throws {
    #expect(days(try expand("FREQ=DAILY;COUNT=10", local(1997, 9, 2))) ==
        (2...11).map { String(format: "1997-09-%02d", $0) })
}

@Test func dailyUntilIsInclusiveAndCountsCorrectly() throws {
    let result = try expand("FREQ=DAILY;UNTIL=19971224T000000Z", local(1997, 9, 2))
    #expect(result.count == 113)
    #expect(result.last == "1997-12-23 09:00")
}

@Test func everyTenDaysFiveTimes() throws {
    #expect(days(try expand("FREQ=DAILY;INTERVAL=10;COUNT=5", local(1997, 9, 2))) ==
        ["1997-09-02", "1997-09-12", "1997-09-22", "1997-10-02", "1997-10-12"])
}

@Test func everyDayInJanuaryForThreeYears() throws {
    let result = try expand("FREQ=YEARLY;UNTIL=20000131T140000Z;BYMONTH=1;BYDAY=SU,MO,TU,WE,TH,FR,SA", local(1998, 1, 1))
    #expect(result.count == 93)
    #expect(result.allSatisfy { $0.dropFirst(5).hasPrefix("01-") })
}

@Test func weeklyForTenOccurrences() throws {
    #expect(days(try expand("FREQ=WEEKLY;COUNT=10", local(1997, 9, 2))) ==
        ["1997-09-02", "1997-09-09", "1997-09-16", "1997-09-23", "1997-09-30",
         "1997-10-07", "1997-10-14", "1997-10-21", "1997-10-28", "1997-11-04"])
}

@Test func weeklyTuesdayThursdayForFiveWeeks() throws {
    #expect(days(try expand("FREQ=WEEKLY;UNTIL=19971007T000000Z;WKST=SU;BYDAY=TU,TH", local(1997, 9, 2))) ==
        ["1997-09-02", "1997-09-04", "1997-09-09", "1997-09-11", "1997-09-16",
         "1997-09-18", "1997-09-23", "1997-09-25", "1997-09-30", "1997-10-02"])
}

@Test func everyOtherWeekMondayWednesdayFriday() throws {
    #expect(days(try expand("FREQ=WEEKLY;INTERVAL=2;UNTIL=19971224T000000Z;WKST=SU;BYDAY=MO,WE,FR", local(1997, 9, 1))) ==
        ["1997-09-01", "1997-09-03", "1997-09-05", "1997-09-15", "1997-09-17", "1997-09-19", "1997-09-29",
         "1997-10-01", "1997-10-03", "1997-10-13", "1997-10-15", "1997-10-17", "1997-10-27", "1997-10-29", "1997-10-31",
         "1997-11-10", "1997-11-12", "1997-11-14", "1997-11-24", "1997-11-26", "1997-11-28",
         "1997-12-08", "1997-12-10", "1997-12-12", "1997-12-22"])
}

@Test func weekStartChangesTheResult() throws {
    #expect(days(try expand("FREQ=WEEKLY;INTERVAL=2;COUNT=4;BYDAY=TU,SU;WKST=MO", local(1997, 8, 5))) ==
        ["1997-08-05", "1997-08-10", "1997-08-19", "1997-08-24"])
    #expect(days(try expand("FREQ=WEEKLY;INTERVAL=2;COUNT=4;BYDAY=TU,SU;WKST=SU", local(1997, 8, 5))) ==
        ["1997-08-05", "1997-08-17", "1997-08-19", "1997-08-31"])
}

@Test func monthlyFirstFriday() throws {
    #expect(days(try expand("FREQ=MONTHLY;COUNT=10;BYDAY=1FR", local(1997, 9, 5))) ==
        ["1997-09-05", "1997-10-03", "1997-11-07", "1997-12-05", "1998-01-02",
         "1998-02-06", "1998-03-06", "1998-04-03", "1998-05-01", "1998-06-05"])
}

@Test func monthlySecondToLastMonday() throws {
    #expect(days(try expand("FREQ=MONTHLY;COUNT=6;BYDAY=-2MO", local(1997, 9, 22))) ==
        ["1997-09-22", "1997-10-20", "1997-11-17", "1997-12-22", "1998-01-19", "1998-02-16"])
}

@Test func monthlyThirdToLastDay() throws {
    #expect(days(try expand("FREQ=MONTHLY;COUNT=6;BYMONTHDAY=-3", local(1997, 9, 28))) ==
        ["1997-09-28", "1997-10-29", "1997-11-28", "1997-12-29", "1998-01-29", "1998-02-26"])
}

@Test func lastWorkdayOfTheMonth() throws {
    #expect(days(try expand("FREQ=MONTHLY;COUNT=7;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1", local(1997, 9, 30))) ==
        ["1997-09-30", "1997-10-31", "1997-11-28", "1997-12-31", "1998-01-30", "1998-02-27", "1998-03-31"])
}

@Test func thirdTuesdayWednesdayOrThursday() throws {
    #expect(days(try expand("FREQ=MONTHLY;COUNT=3;BYDAY=TU,WE,TH;BYSETPOS=3", local(1997, 9, 4))) ==
        ["1997-09-04", "1997-10-07", "1997-11-06"])
}

@Test func invalidDatesAreSkipped() throws {
    #expect(days(try expand("FREQ=MONTHLY;BYMONTHDAY=15,30;COUNT=5", local(2007, 1, 15))) ==
        ["2007-01-15", "2007-01-30", "2007-02-15", "2007-03-15", "2007-03-30"])
}

@Test func fridayThe13th() throws {
    // The RFC lists the rule with an EXDATE for the anchor; the rule alone starts with the anchor.
    let result = try expand("FREQ=MONTHLY;BYDAY=FR;BYMONTHDAY=13", local(1997, 9, 2), before: local(2000, 12, 31))
    #expect(days(Array(result.dropFirst())) == ["1998-02-13", "1998-03-13", "1998-11-13", "1999-08-13", "2000-10-13"])
}

@Test func yearlyInJuneAndJuly() throws {
    #expect(days(try expand("FREQ=YEARLY;COUNT=10;BYMONTH=6,7", local(1997, 6, 10))) ==
        ["1997-06-10", "1997-07-10", "1998-06-10", "1998-07-10", "1999-06-10",
         "1999-07-10", "2000-06-10", "2000-07-10", "2001-06-10", "2001-07-10"])
}

@Test func mondayOfWeekTwenty() throws {
    #expect(days(try expand("FREQ=YEARLY;BYWEEKNO=20;BYDAY=MO", local(1997, 5, 12), before: local(2000, 1, 1))) ==
        ["1997-05-12", "1998-05-11", "1999-05-17"])
}

@Test func everyThirdYearOnYearDays() throws {
    #expect(days(try expand("FREQ=YEARLY;INTERVAL=3;COUNT=10;BYYEARDAY=1,100,200", local(1997, 1, 1))) ==
        ["1997-01-01", "1997-04-10", "1997-07-19", "2000-01-01", "2000-04-09",
         "2000-07-18", "2003-01-01", "2003-04-10", "2003-07-19", "2006-01-01"])
}

@Test func everyThursdayInMarch() throws {
    #expect(days(try expand("FREQ=YEARLY;BYMONTH=3;BYDAY=TH", local(1997, 3, 13), before: local(1999, 12, 31))) ==
        ["1997-03-13", "1997-03-20", "1997-03-27", "1998-03-05", "1998-03-12", "1998-03-19", "1998-03-26",
         "1999-03-04", "1999-03-11", "1999-03-18", "1999-03-25"])
}

@Test func twentiethMondayOfTheYear() throws {
    #expect(days(try expand("FREQ=YEARLY;BYDAY=20MO", local(1997, 5, 19), before: local(2000, 1, 1))) ==
        ["1997-05-19", "1998-05-18", "1999-05-17"])
}

@Test func presidentialElectionDay() throws {
    #expect(days(try expand("FREQ=YEARLY;INTERVAL=4;BYMONTH=11;BYDAY=TU;BYMONTHDAY=2,3,4,5,6,7,8", local(1996, 11, 5), before: local(2005, 1, 1))) ==
        ["1996-11-05", "2000-11-07", "2004-11-02"])
}

@Test func everyFifteenMinutesSixTimes() throws {
    #expect(try expand("FREQ=MINUTELY;INTERVAL=15;COUNT=6", local(1997, 9, 2)) ==
        ["1997-09-02 09:00", "1997-09-02 09:15", "1997-09-02 09:30", "1997-09-02 09:45", "1997-09-02 10:00", "1997-09-02 10:15"])
}

@Test func everyTwentyMinutesDuringTheDay() throws {
    let result = try expand("FREQ=DAILY;BYHOUR=9,10,11,12,13,14,15,16;BYMINUTE=0,20,40", local(1997, 9, 2), before: local(1997, 9, 3, 0))
    #expect(result.count == 24)
    #expect(result.first == "1997-09-02 09:00")
    #expect(result.last == "1997-09-02 16:40")
}

@Test func keepsWallClockAcrossDST() throws {
    let result = try expand("FREQ=DAILY;COUNT=3", local(2026, 3, 7, 9), before: local(2027, 1, 1))
    #expect(result == ["2026-03-07 09:00", "2026-03-08 09:00", "2026-03-09 09:00"])
}

@Test func missingLocalTimeMovesForward() throws {
    let result = try expand("FREQ=DAILY;COUNT=3", local(2026, 3, 7, 2, 30), before: local(2027, 1, 1))
    #expect(result == ["2026-03-07 02:30", "2026-03-08 03:30", "2026-03-09 02:30"])
}

@Test func repeatedLocalTimeUsesTheFirst() throws {
    let rule = try RecurrenceRule(rrule: "FREQ=DAILY;COUNT=2")
    let starts = rule.instances(anchor: local(2026, 10, 31, 1, 30), timeZone: ny, isAllDay: false, before: local(2027, 1, 1), limit: 10).starts
    #expect(starts[1] == Date(timeIntervalSince1970: 1_793_511_000))   // 2026-11-01 05:30Z = 01:30 EDT, not EST
}

@Test func allDayRulesProduceCanonicalMidnights() throws {
    let rule = try RecurrenceRule(rrule: "FREQ=WEEKLY;COUNT=3")
    let anchor = AllDay.startOfDay(CalendarDate(year: 2026, month: 3, day: 2), in: ny)!
    let starts = rule.instances(anchor: anchor, timeZone: ny, isAllDay: true, before: local(2027, 1, 1), limit: 10).starts
    #expect(starts == [2, 9, 16].map { AllDay.startOfDay(CalendarDate(year: 2026, month: 3, day: $0), in: ny)! })
}

@Test func leapDayYearlyOnlyInLeapYears() throws {
    #expect(days(try expand("FREQ=YEARLY;COUNT=3", local(2024, 2, 29))) == ["2024-02-29", "2028-02-29", "2032-02-29"])
}

@Test func limitTruncates() throws {
    let rule = try RecurrenceRule(rrule: "FREQ=DAILY")
    let result = rule.instances(anchor: local(2026, 1, 1), timeZone: ny, isAllDay: false, before: local(2027, 1, 1), limit: 5)
    #expect(result.starts.count == 5)
    #expect(result.truncated)
}

@Test func anchorAtOrAfterBoundGivesNothing() throws {
    let rule = try RecurrenceRule(rrule: "FREQ=DAILY")
    #expect(rule.instances(anchor: local(2026, 1, 1), timeZone: ny, isAllDay: false, before: local(2026, 1, 1), limit: 5).starts.isEmpty)
}

@Test func skipToJumpsWholePeriodsWithoutChangingResults() throws {
    let rule = try RecurrenceRule(rrule: "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,TH")
    let anchor = local(2015, 1, 5)
    let cut = local(2026, 9, 1)
    let full = rule.instances(anchor: anchor, timeZone: ny, isAllDay: false, before: local(2026, 10, 1), limit: 100_000).starts
    let skipped = rule.instances(anchor: anchor, timeZone: ny, isAllDay: false, before: local(2026, 10, 1), limit: 100_000, skipTo: cut).starts
    #expect(skipped.first == anchor)
    #expect(skipped.filter { $0 >= cut } == full.filter { $0 >= cut })
    #expect(skipped.count < 20)   // the years before the cut were not generated
}
