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

@Test(arguments: ["FREQ=MONTHLY;BYSETPOS=1;BYDAY=MO", "FREQ=HOURLY", "FREQ=DAILY;BYHOUR=9", "FREQ=DAILY;COUNT=3;UNTIL=20261005",
                  "FREQ=WEEKLY;WKST=SU", "FREQ=WEEKLY;BYYEARDAY=3", "FREQ=WEEKLY;UNTIL=20261005T150000",
                  "FREQ=DAILY;BYDAY=MO", "FREQ=WEEKLY;BYWEEKNO=3"])
func rejectsWhatIsOutsideTheSubset(text: String) async {
    await expectWriteError(.unsupported(fields: [.recurrence])) { _ = try RecurrenceRule(rrule: text) }
}

@Test func rejectsMalformedText() async {
    await expectWriteError(.invalid("RRULE has no FREQ")) { _ = try RecurrenceRule(rrule: "INTERVAL=2") }
    await expectWriteError(.invalid("bad INTERVAL: x")) { _ = try RecurrenceRule(rrule: "FREQ=DAILY;INTERVAL=x") }
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
func rejectsBadUntilValues(text: String) async {
    let untilText = String(text.dropFirst("FREQ=DAILY;UNTIL=".count))
    await expectWriteError(.invalid("bad UNTIL: \(untilText)")) { _ = try RecurrenceRule(rrule: text) }
}

@Test(arguments: ["FREQ=MONTHLY;BYMONTHDAY=-9223372036854775808", "FREQ=MONTHLY;BYMONTHDAY=9223372036854775807"])
func hugeMonthDaysThrowInsteadOfTrapping(text: String) async {
    await expectWriteError(.invalid("month days must be 1...31 or -31...-1")) { _ = try RecurrenceRule(rrule: text) }
}

@Test(arguments: ["FREQ=MONTHLY;BYDAY=-9223372036854775808TU", "FREQ=MONTHLY;BYDAY=9223372036854775807TU"])
func hugeOrdinalsThrowInsteadOfTrapping(text: String) async {
    await expectWriteError(.invalid("weekday ordinal must be 1...5 or -5...-1")) { _ = try RecurrenceRule(rrule: text) }
}

@Test func validateAcceptsTheRangeEdges() async throws {
    try RecurrenceRule(frequency: .monthly, weekdays: [.init(.monday, ordinal: 5), .init(.friday, ordinal: -5)],
                       monthDays: [31, -31, 1, -1]).validate()
    await expectWriteError(.invalid("month days must be 1...31 or -31...-1")) {
        try RecurrenceRule(frequency: .monthly, monthDays: [32]).validate()
    }
}

@Test(arguments: [("FREQ=DAILY;FREQ=WEEKLY", "FREQ"), ("FREQ=DAILY;COUNT=3;COUNT=5", "COUNT"), ("FREQ=DAILY;freq=WEEKLY", "FREQ")])
func rejectsDuplicateParts(text: String, key: String) async {
    await expectWriteError(.invalid("duplicate RRULE part: \(key)")) { _ = try RecurrenceRule(rrule: text) }
}

@Test(arguments: [("FREQ=WEEKLY;BYDAY=MO,,WE", "BYDAY"), ("FREQ=WEEKLY;BYDAY=MO,", "BYDAY"),
                  ("FREQ=MONTHLY;BYMONTHDAY=1,,2", "BYMONTHDAY"), ("FREQ=YEARLY;BYMONTH=3,,4", "BYMONTH")])
func rejectsEmptyListElements(text: String, name: String) async {
    _ = name
    do {
        _ = try RecurrenceRule(rrule: text)
        Issue.record("expected \(text) to throw")
    } catch let error as WriteError {
        guard case .invalid = error else { Issue.record("\(text): expected .invalid, got \(error)"); return }
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

@Test func parsedRulesAreValidated() async {
    await expectWriteError(.invalid("an ordinal weekday needs a monthly or yearly rule")) {
        _ = try RecurrenceRule(rrule: "FREQ=WEEKLY;BYDAY=1MO")
    }
    await expectWriteError(.invalid("recurrence interval must be at least 1")) {
        _ = try RecurrenceRule(rrule: "FREQ=DAILY;INTERVAL=0")
    }
}

@Test func dateTextIsEightDigits() {
    #expect(RecurrenceRule.dateText(CalendarDate(year: 2026, month: 3, day: 7)) == "20260307")
    #expect(RecurrenceRule.dateText(CalendarDate(year: 2026, month: 12, day: 31)) == "20261231")
}
