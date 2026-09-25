import Foundation
import Testing
@testable import CalendarCore

private let tokyo = TimeZone(identifier: "Asia/Tokyo")!
private func instant(_ s: String) -> Date { ISO8601DateFormatter().date(from: s)! }

@Test func parsesAWeeklyRuleWithDaysAndCount() throws {
    let rule = try RecurrenceRule(rrule: "RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=6")
    #expect(rule.frequency == .weekly && rule.interval == 2)
    #expect(rule.weekdays == [.init(.monday), .init(.wednesday)])
    #expect(rule.end == .count(6))
}

@Test func parsesOrdinalWeekdaysMonthDaysAndMonths() throws {
    let monthly = try RecurrenceRule(rrule: "FREQ=MONTHLY;BYDAY=2TU,-1FR")
    #expect(monthly.weekdays == [.init(.tuesday, ordinal: 2), .init(.friday, ordinal: -1)])
    let byDay = try RecurrenceRule(rrule: "FREQ=MONTHLY;BYMONTHDAY=1,15,-1")
    #expect(byDay.monthDays == [1, 15, -1])
    let yearly = try RecurrenceRule(rrule: "FREQ=YEARLY;BYMONTH=3,9;BYMONTHDAY=5")
    #expect(yearly.months == [3, 9] && yearly.monthDays == [5])
}

@Test func rendersInCanonicalOrder() {
    let rule = RecurrenceRule(frequency: .monthly, interval: 3, weekdays: [.init(.tuesday, ordinal: 2)], end: .count(4))
    #expect(rule.rruleString(allDay: false, in: nil) == "FREQ=MONTHLY;INTERVAL=3;BYDAY=2TU;COUNT=4")
    #expect(RecurrenceRule(frequency: .daily).rruleString(allDay: false, in: nil) == "FREQ=DAILY")
}

@Test func untilRendersAsUTCDateTimeForTimedAndDateInZoneForAllDay() {
    let until = instant("2026-10-05T15:00:00Z")   // 2026-10-06 00:00 in Tokyo
    let rule = RecurrenceRule(frequency: .daily, end: .until(until))
    #expect(rule.rruleString(allDay: false, in: tokyo) == "FREQ=DAILY;UNTIL=20261005T150000Z")
    #expect(rule.rruleString(allDay: true, in: tokyo) == "FREQ=DAILY;UNTIL=20261006")
}

@Test func untilRoundTripsThroughParseAndRender() throws {
    let timed = try RecurrenceRule(rrule: "FREQ=WEEKLY;UNTIL=20261005T150000Z")
    #expect(timed.end == .until(instant("2026-10-05T15:00:00Z")))
    #expect(timed.rruleString(allDay: false, in: nil) == "FREQ=WEEKLY;UNTIL=20261005T150000Z")
    let allDay = try RecurrenceRule(rrule: "FREQ=WEEKLY;UNTIL=20261006", in: tokyo)
    #expect(allDay.end == .until(instant("2026-10-05T15:00:00Z")))
    #expect(allDay.rruleString(allDay: true, in: tokyo) == "FREQ=WEEKLY;UNTIL=20261006")
}

/// Runs `body` and records an issue unless it throws exactly `expected`.
private func expectParseError(_ expected: RecurrenceParseError, _ body: () throws -> Void) {
    do {
        try body()
        Issue.record("expected \(expected), but nothing was thrown")
    } catch let error as RecurrenceParseError {
        #expect(error == expected)
    } catch {
        Issue.record("expected \(expected), got \(error)")
    }
}

// A rule outside what writers can express still parses; `validate()` refuses to write it.
@Test(arguments: ["FREQ=MONTHLY;BYSETPOS=1;BYDAY=MO", "FREQ=HOURLY", "FREQ=DAILY;BYHOUR=9", "FREQ=WEEKLY;WKST=SU",
                  "FREQ=WEEKLY;BYYEARDAY=3", "FREQ=DAILY;BYDAY=MO", "FREQ=WEEKLY;BYWEEKNO=3",
                  "FREQ=MINUTELY;INTERVAL=15", "FREQ=DAILY;BYMINUTE=0,30;BYSECOND=0", "FREQ=DAILY;X-FOO=bar"])
func aRuleOutsideTheWritableSubsetParsesButIsNotWritable(text: String) async throws {
    let rule = try RecurrenceRule(rrule: text)
    await expectWriteError(.unsupported(fields: [.recurrence])) { try rule.validate() }
}

@Test(arguments: [("INTERVAL=2", "RRULE has no FREQ"), ("FREQ=DAILY;INTERVAL=x", "bad INTERVAL: x"), ("FREQ=DAILY;INTERVAL=0", "bad INTERVAL: 0"),
                  ("FREQ=DAILY;COUNT=3;UNTIL=20261005", "RRULE has both COUNT and UNTIL"), ("FREQ=SOMETIMES", "unknown FREQ: SOMETIMES"),
                  ("FREQ=DAILY;BYHOUR=24", "bad BYHOUR value: 24"), ("FREQ=YEARLY;BYMONTH=13", "bad BYMONTH value: 13"),
                  ("FREQ=YEARLY;BYYEARDAY=0", "bad BYYEARDAY value: 0"), ("FREQ=WEEKLY;WKST=XX", "bad WKST: XX")])
func malformedTextThrowsAParseError(text: String, message: String) {
    expectParseError(.malformed(message)) { _ = try RecurrenceRule(rrule: text) }
}

@Test func everyPartOfTheFullGrammarIsRead() throws {
    let rule = try RecurrenceRule(rrule: "FREQ=YEARLY;INTERVAL=2;BYMONTH=1,7;BYWEEKNO=20;BYYEARDAY=1,-1;BYMONTHDAY=15;BYDAY=-1SU;BYHOUR=9;BYMINUTE=30;BYSECOND=0;BYSETPOS=-1;WKST=SU;COUNT=10;X-A=1;X-B=2")
    #expect(rule.frequency == .yearly && rule.interval == 2 && rule.months == [1, 7] && rule.weekNumbers == [20] && rule.yearDays == [1, -1])
    #expect(rule.monthDays == [15] && rule.weekdays == [.init(.sunday, ordinal: -1)] && rule.hours == [9] && rule.minutes == [30] && rule.seconds == [0])
    #expect(rule.setPositions == [-1] && rule.weekStart == .sunday && rule.end == .count(10))
    #expect(rule.unrecognizedParts == [.init(name: "X-A", value: "1"), .init(name: "X-B", value: "2")])
}

@Test func unrecognizedPartsAndTheFullGrammarSurviveARoundTrip() throws {
    for text in ["FREQ=DAILY;X-FOO=bar;X-BAZ=1", "FREQ=YEARLY;BYMONTH=1;BYYEARDAY=1;BYSETPOS=1;WKST=SU", "FREQ=MINUTELY;INTERVAL=15;BYHOUR=9;COUNT=4"] {
        let rule = try RecurrenceRule(rrule: text)
        #expect(try RecurrenceRule(rrule: rule.rruleString(allDay: false, in: nil)) == rule)
    }
    #expect(try RecurrenceRule(rrule: "FREQ=DAILY;X-FOO=bar").rruleString(allDay: false, in: nil) == "FREQ=DAILY;X-FOO=bar")
}

@Test(arguments: [
    // RFC 5545 section 3.8.5.3 examples.
    "FREQ=DAILY;COUNT=10", "FREQ=DAILY;UNTIL=19971224T000000Z", "FREQ=DAILY;INTERVAL=2", "FREQ=DAILY;INTERVAL=10;COUNT=5",
    "FREQ=YEARLY;UNTIL=20000131T140000Z;BYMONTH=1;BYDAY=SU,MO,TU,WE,TH,FR,SA", "FREQ=WEEKLY;COUNT=10",
    "FREQ=WEEKLY;INTERVAL=2;WKST=SU", "FREQ=WEEKLY;UNTIL=19971224T000000Z;WKST=SU;BYDAY=MO,WE,FR",
    "FREQ=WEEKLY;INTERVAL=2;COUNT=8;WKST=SU;BYDAY=TU,TH", "FREQ=MONTHLY;COUNT=10;BYDAY=1FR", "FREQ=MONTHLY;INTERVAL=2;COUNT=10;BYDAY=1SU,-1SU",
    "FREQ=MONTHLY;COUNT=6;BYDAY=-2MO", "FREQ=MONTHLY;BYMONTHDAY=-3", "FREQ=MONTHLY;COUNT=10;BYMONTHDAY=2,15",
    "FREQ=YEARLY;INTERVAL=2;COUNT=10;BYMONTH=1,2,3", "FREQ=YEARLY;INTERVAL=3;COUNT=10;BYYEARDAY=1,100,200",
    "FREQ=YEARLY;BYDAY=20MO", "FREQ=YEARLY;BYWEEKNO=20;BYDAY=MO", "FREQ=YEARLY;BYMONTH=3;BYDAY=TH",
    "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1", "FREQ=HOURLY;INTERVAL=3;UNTIL=19970902T170000Z",
    "FREQ=MINUTELY;INTERVAL=15;COUNT=6", "FREQ=DAILY;BYHOUR=9,10,11,12,13,14,15,16;BYMINUTE=0,20,40",
])
func theRFCExamplesRoundTrip(text: String) throws {
    let rule = try RecurrenceRule(rrule: text)
    let rendered = rule.rruleString(allDay: false, in: nil)
    #expect(try RecurrenceRule(rrule: rendered) == rule)
}

@Test func aFloatingUntilIsReadInTheSeriesZone() throws {
    let rule = try RecurrenceRule(rrule: "FREQ=WEEKLY;UNTIL=20261006T090000", in: tokyo)
    #expect(rule.end == .until(instant("2026-10-06T00:00:00Z")))   // 09:00 in Tokyo
}

@Test func validationCatchesImpossibleRules() async {
    await expectWriteError(.invalid("recurrence interval must be at least 1")) {
        try RecurrenceRule(frequency: .daily, interval: 0).validate()
    }
    await expectWriteError(.invalid("an ordinal weekday needs a monthly or yearly rule")) {
        try RecurrenceRule(frequency: .weekly, weekdays: [.init(.monday, ordinal: 1)]).validate()
    }
    await expectWriteError(.invalid("weekday ordinal must be 1...5 or -5...-1")) {
        try RecurrenceRule(frequency: .monthly, weekdays: [.init(.monday, ordinal: 6)]).validate()
    }
    await expectWriteError(.invalid("month days apply to monthly and yearly rules")) {
        try RecurrenceRule(frequency: .weekly, monthDays: [1]).validate()
    }
    await expectWriteError(.invalid("months apply to yearly rules only")) {
        try RecurrenceRule(frequency: .monthly, months: [3]).validate()
    }
    await expectWriteError(.invalid("recurrence count must be at least 1")) {
        try RecurrenceRule(frequency: .daily, end: .count(0)).validate()
    }
    #expect(throws: Never.self) { try RecurrenceRule(frequency: .weekly, weekdays: [.init(.monday)], end: .count(3)).validate() }
}

// MARK: Review fixes

@Test(arguments: ["FREQ=DAILY;UNTIL=20261345", "FREQ=DAILY;UNTIL=20261005T256199Z", "FREQ=DAILY;UNTIL=20260230",
                  "FREQ=DAILY;UNTIL=20261005T240000Z", "FREQ=DAILY;UNTIL=00000000",
                  "FREQ=DAILY;UNTIL=\u{0662}\u{0660}\u{0662}\u{0666}\u{0661}\u{0660}\u{0660}\u{0665}",
                  "FREQ=DAILY;UNTIL=2026100\u{0665}T150000Z", "FREQ=DAILY;UNTIL=abcdefgh", "FREQ=DAILY;UNTIL=20261005TAB0000Z"])
func rejectsBadUntilValues(text: String) {
    let untilText = String(text.dropFirst("FREQ=DAILY;UNTIL=".count))
    expectParseError(.malformed("bad UNTIL: \(untilText)")) { _ = try RecurrenceRule(rrule: text) }
}

@Test(arguments: ["FREQ=MONTHLY;BYMONTHDAY=-9223372036854775808", "FREQ=MONTHLY;BYMONTHDAY=9223372036854775807"])
func hugeMonthDaysThrowInsteadOfTrapping(text: String) {
    let value = String(text.dropFirst("FREQ=MONTHLY;BYMONTHDAY=".count))
    expectParseError(.malformed("bad BYMONTHDAY value: \(value)")) { _ = try RecurrenceRule(rrule: text) }
}

@Test(arguments: ["FREQ=MONTHLY;BYDAY=-9223372036854775808TU", "FREQ=MONTHLY;BYDAY=9223372036854775807TU"])
func hugeOrdinalsThrowInsteadOfTrapping(text: String) {
    let value = String(text.dropFirst("FREQ=MONTHLY;BYDAY=".count))
    expectParseError(.malformed("bad BYDAY value: \(value)")) { _ = try RecurrenceRule(rrule: text) }
}

@Test func validateAcceptsTheRangeEdges() async throws {
    try RecurrenceRule(frequency: .monthly, weekdays: [.init(.monday, ordinal: 5), .init(.friday, ordinal: -5)],
                       monthDays: [31, -31, 1, -1]).validate()
    await expectWriteError(.invalid("month days must be 1...31 or -31...-1")) {
        try RecurrenceRule(frequency: .monthly, monthDays: [32]).validate()
    }
}

@Test(arguments: [("FREQ=DAILY;FREQ=WEEKLY", "FREQ"), ("FREQ=DAILY;COUNT=3;COUNT=5", "COUNT"), ("FREQ=DAILY;freq=WEEKLY", "FREQ")])
func rejectsDuplicateParts(text: String, key: String) {
    expectParseError(.malformed("duplicate RRULE part: \(key)")) { _ = try RecurrenceRule(rrule: text) }
}

@Test(arguments: [("FREQ=WEEKLY;BYDAY=MO,,WE", "BYDAY"), ("FREQ=WEEKLY;BYDAY=MO,", "BYDAY"),
                  ("FREQ=MONTHLY;BYMONTHDAY=1,,2", "BYMONTHDAY"), ("FREQ=YEARLY;BYMONTH=3,,4", "BYMONTH")])
func rejectsEmptyListElements(text: String, name: String) {
    _ = name
    do {
        _ = try RecurrenceRule(rrule: text)
        Issue.record("expected \(text) to throw")
    } catch is RecurrenceParseError {
    } catch {
        Issue.record("\(text): unexpected \(error)")
    }
}

@Test func acceptsMondayWeekStart() throws {
    let rule = try RecurrenceRule(rrule: "FREQ=WEEKLY;WKST=MO;BYDAY=TU")
    #expect(rule == RecurrenceRule(frequency: .weekly, weekdays: [.init(.tuesday)]))
    #expect(rule.rruleString(allDay: false, in: nil) == "FREQ=WEEKLY;BYDAY=TU")   // WKST=MO is the default and is not rendered
}

@Test func parsingIsCaseInsensitiveAndAcceptsThePrefix() throws {
    let rule = try RecurrenceRule(rrule: "  rrule:freq=weekly;interval=2;byday=mo,we;until=20261005t150000z")
    #expect(rule.frequency == .weekly && rule.interval == 2)
    #expect(rule.weekdays == [.init(.monday), .init(.wednesday)])
    #expect(rule.end == .until(instant("2026-10-05T15:00:00Z")))
    let monthly = try RecurrenceRule(rrule: "RRULE:freq=monthly;byday=-1fr")
    #expect(monthly.weekdays == [.init(.friday, ordinal: -1)])
}

@Test func aParsedRuleIsCheckedForWritingBySeparatingValidate() async throws {
    let ordinalOnAWeeklyRule = try RecurrenceRule(rrule: "FREQ=WEEKLY;BYDAY=1MO")
    await expectWriteError(.invalid("an ordinal weekday needs a monthly or yearly rule")) { try ordinalOnAWeeklyRule.validate() }
    let plain = try RecurrenceRule(rrule: "FREQ=WEEKLY;BYDAY=MO")
    #expect(throws: Never.self) { try plain.validate() }
}

@Test func dateTextIsEightDigits() {
    #expect(RecurrenceRule.dateText(CalendarDate(year: 2026, month: 3, day: 7)) == "20260307")
    #expect(RecurrenceRule.dateText(CalendarDate(year: 2026, month: 12, day: 31)) == "20261231")
}
