import CalendarCore
import Foundation
import Testing
@testable import MicrosoftCalendar

private let la = TimeZone(identifier: "America/Los_Angeles")!
private let sept25 = CalendarDate(year: 2026, month: 9, day: 25)   // a Friday

private func dto(_ pattern: [String: Any], _ range: [String: Any] = ["type": "noEnd", "startDate": "2026-09-25"]) throws -> GraphRecurrenceDTO {
    let data = try JSONSerialization.data(withJSONObject: ["pattern": pattern, "range": range])
    return try JSONDecoder().decode(GraphRecurrenceDTO.self, from: data)
}

@Test func readsEveryPatternType() throws {
    let daily = GraphRecurrenceMapper.rule(from: try dto(["type": "daily", "interval": 2]), in: la)
    #expect(daily == RecurrenceRule(frequency: .daily, interval: 2))

    let weekly = GraphRecurrenceMapper.rule(from: try dto(["type": "weekly", "interval": 1, "daysOfWeek": ["monday", "friday"], "firstDayOfWeek": "sunday"]), in: la)
    #expect(weekly == RecurrenceRule(frequency: .weekly, weekdays: [.init(.monday), .init(.friday)], weekStart: .sunday))

    let monthly = GraphRecurrenceMapper.rule(from: try dto(["type": "absoluteMonthly", "interval": 1, "dayOfMonth": 15]), in: la)
    #expect(monthly == RecurrenceRule(frequency: .monthly, monthDays: [15]))

    let relative = GraphRecurrenceMapper.rule(from: try dto(["type": "relativeMonthly", "interval": 1, "daysOfWeek": ["tuesday"], "index": "last"]), in: la)
    #expect(relative == RecurrenceRule(frequency: .monthly, weekdays: [.init(.tuesday, ordinal: -1)]))

    let yearly = GraphRecurrenceMapper.rule(from: try dto(["type": "absoluteYearly", "interval": 1, "month": 3, "dayOfMonth": 9]), in: la)
    #expect(yearly == RecurrenceRule(frequency: .yearly, monthDays: [9], months: [3]))

    let relativeYearly = GraphRecurrenceMapper.rule(from: try dto(["type": "relativeYearly", "interval": 1, "month": 11, "daysOfWeek": ["thursday"], "index": "fourth"]), in: la)
    #expect(relativeYearly == RecurrenceRule(frequency: .yearly, weekdays: [.init(.thursday, ordinal: 4)], months: [11]))
}

@Test func readsRanges() throws {
    let count = GraphRecurrenceMapper.rule(from: try dto(["type": "daily", "interval": 1], ["type": "numbered", "startDate": "2026-09-25", "numberOfOccurrences": 10]), in: la)
    #expect(count?.end == .count(10))

    let until = GraphRecurrenceMapper.rule(
        from: try dto(["type": "daily", "interval": 1], ["type": "endDate", "startDate": "2026-09-25", "endDate": "2026-10-01", "recurrenceTimeZone": "Pacific Standard Time"]), in: .gmt)
    // The end of Oct 1 in Pacific time is 2026-10-02T06:59:59Z.
    #expect(until?.end == .until(Date(timeIntervalSince1970: 1_790_924_399)))
    #expect(GraphRecurrenceMapper.rule(from: try dto(["type": "daily", "interval": 1]), in: la)?.end == .never)
}

@Test func anUnknownPatternReadsAsNil() throws {
    #expect(GraphRecurrenceMapper.rule(from: try dto(["type": "lunar"]), in: la) == nil)
}

@Test func writesEveryPatternType() throws {
    func pattern(_ rule: RecurrenceRule) throws -> [String: Any] {
        try #require(GraphRecurrenceMapper.recurrence(from: rule, start: sept25, in: la)["pattern"] as? [String: Any])
    }
    let daily = try pattern(RecurrenceRule(frequency: .daily, interval: 3))
    #expect(daily["type"] as? String == "daily" && daily["interval"] as? Int == 3 && daily["firstDayOfWeek"] as? String == "monday")

    let weekly = try pattern(RecurrenceRule(frequency: .weekly, weekdays: [.init(.monday), .init(.wednesday)]))
    #expect(weekly["type"] as? String == "weekly" && weekly["daysOfWeek"] as? [String] == ["monday", "wednesday"])

    let weeklyDefault = try pattern(RecurrenceRule(frequency: .weekly))   // no BYDAY: the start's weekday
    #expect(weeklyDefault["daysOfWeek"] as? [String] == ["friday"])

    let monthly = try pattern(RecurrenceRule(frequency: .monthly))
    #expect(monthly["type"] as? String == "absoluteMonthly" && monthly["dayOfMonth"] as? Int == 25)

    let relative = try pattern(RecurrenceRule(frequency: .monthly, weekdays: [.init(.friday, ordinal: -1)]))
    #expect(relative["type"] as? String == "relativeMonthly" && relative["index"] as? String == "last" && relative["daysOfWeek"] as? [String] == ["friday"])

    let yearly = try pattern(RecurrenceRule(frequency: .yearly))
    #expect(yearly["type"] as? String == "absoluteYearly" && yearly["month"] as? Int == 9 && yearly["dayOfMonth"] as? Int == 25)

    let relativeYearly = try pattern(RecurrenceRule(frequency: .yearly, weekdays: [.init(.thursday, ordinal: 4)], months: [11]))
    #expect(relativeYearly["type"] as? String == "relativeYearly" && relativeYearly["index"] as? String == "fourth" && relativeYearly["month"] as? Int == 11)
}

@Test func writesRanges() throws {
    func range(_ end: RecurrenceRule.End) throws -> [String: Any] {
        try #require(GraphRecurrenceMapper.recurrence(from: RecurrenceRule(frequency: .daily, end: end), start: sept25, in: la)["range"] as? [String: Any])
    }
    let never = try range(.never)
    #expect(never["type"] as? String == "noEnd" && never["startDate"] as? String == "2026-09-25" && never["recurrenceTimeZone"] as? String == "Pacific Standard Time")
    let count = try range(.count(4))
    #expect(count["type"] as? String == "numbered" && count["numberOfOccurrences"] as? Int == 4)
    let until = try range(.until(Date(timeIntervalSince1970: 1_790_924_399)))   // the end of Oct 1 in Pacific time
    #expect(until["type"] as? String == "endDate" && until["endDate"] as? String == "2026-10-01")
}

@Test func aZoneWithoutAWindowsNameIsWrittenAsUTC() throws {
    let fakaofo = TimeZone(identifier: "Pacific/Fakaofo")!
    let range = try #require(GraphRecurrenceMapper.recurrence(from: RecurrenceRule(frequency: .daily), start: sept25, in: fakaofo)["range"] as? [String: Any])
    #expect(range["recurrenceTimeZone"] as? String == "UTC")
}

@Test func refusesWhatGraphCannotExpress() {
    let cases: [RecurrenceRule] = [
        RecurrenceRule(frequency: .monthly, monthDays: [1, 15]),                           // several month days
        RecurrenceRule(frequency: .monthly, monthDays: [-1]),                              // last day of the month
        RecurrenceRule(frequency: .monthly, weekdays: [.init(.friday, ordinal: 5)]),        // no fifth
        RecurrenceRule(frequency: .monthly, weekdays: [.init(.friday, ordinal: -2)]),       // second to last
        RecurrenceRule(frequency: .monthly, weekdays: [.init(.monday, ordinal: 1), .init(.friday, ordinal: 2)]),   // two indexes
        RecurrenceRule(frequency: .yearly, months: [3, 9]),
        RecurrenceRule(frequency: .weekly, weekStart: .sunday),
        RecurrenceRule(frequency: .hourly),
        RecurrenceRule(frequency: .daily, unrecognizedParts: [.init(name: "X-FOO", value: "1")]),
    ]
    for rule in cases {
        #expect(throws: WriteError.unsupported(fields: [.recurrence]), "\(rule)") {
            try GraphRecurrenceMapper.recurrence(from: rule, start: sept25, in: la)
        }
    }
}

@Test func roundTripsThroughGraphsShape() throws {
    let rules: [RecurrenceRule] = [
        RecurrenceRule(frequency: .daily, interval: 2, end: .count(5)),
        RecurrenceRule(frequency: .weekly, weekdays: [.init(.tuesday), .init(.thursday)]),
        RecurrenceRule(frequency: .monthly, weekdays: [.init(.monday, ordinal: 2)]),
        RecurrenceRule(frequency: .yearly, monthDays: [4], months: [7]),
    ]
    for rule in rules {
        let json = try GraphRecurrenceMapper.recurrence(from: rule, start: sept25, in: la)
        let decoded = try JSONDecoder().decode(GraphRecurrenceDTO.self, from: try JSONSerialization.data(withJSONObject: json))
        #expect(GraphRecurrenceMapper.rule(from: decoded, in: la) == rule, "\(rule)")
    }
}

@Test func truncatesAndRestartsARange() throws {
    let original: [String: Any] = [
        "pattern": ["type": "daily", "interval": 1],
        "range": ["type": "numbered", "startDate": "2026-09-01", "numberOfOccurrences": 10, "recurrenceTimeZone": "UTC"],
    ]
    #expect(GraphRecurrenceMapper.occurrenceCount(in: original) == 10)
    let cut = GraphRecurrenceMapper.truncated(original, endingBefore: CalendarDate(year: 2026, month: 9, day: 5))
    let range = try #require(cut["range"] as? [String: Any])
    #expect(range["type"] as? String == "endDate" && range["endDate"] as? String == "2026-09-04")
    #expect(range["numberOfOccurrences"] == nil && range["startDate"] as? String == "2026-09-01")
    #expect(GraphRecurrenceMapper.occurrenceCount(in: cut) == nil)

    let next = GraphRecurrenceMapper.restarted(original, at: CalendarDate(year: 2026, month: 9, day: 5), remaining: 6)
    let nextRange = try #require(next["range"] as? [String: Any])
    #expect(nextRange["startDate"] as? String == "2026-09-05" && nextRange["numberOfOccurrences"] as? Int == 6 && nextRange["type"] as? String == "numbered")
}
