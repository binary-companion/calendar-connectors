import CalendarCore
import Foundation

/// Graph's `patternedRecurrence` to and from `RecurrenceRule`. Reading understands every pattern Graph produces.
/// Writing accepts only what Graph can express and throws `WriteError.unsupported(fields: [.recurrence])` for the
/// rest; it never mangles a rule.
enum GraphRecurrenceMapper {
    private static let weekdays: [(name: String, day: RecurrenceRule.Weekday)] = [
        ("monday", .monday), ("tuesday", .tuesday), ("wednesday", .wednesday), ("thursday", .thursday),
        ("friday", .friday), ("saturday", .saturday), ("sunday", .sunday),
    ]
    /// Graph's `index` is first to fourth or last (there is no fifth).
    private static let indexes: [(name: String, ordinal: Int)] = [("first", 1), ("second", 2), ("third", 3), ("fourth", 4), ("last", -1)]

    private static func weekday(named name: String) -> RecurrenceRule.Weekday? { weekdays.first { $0.name == name.lowercased() }?.day }
    private static func name(of day: RecurrenceRule.Weekday) -> String { weekdays.first { $0.day == day }!.name }

    // MARK: Reading

    /// nil for a pattern this mapper does not know. `zone` is the event's zone, used when the range names none.
    static func rule(from dto: GraphRecurrenceDTO, in zone: TimeZone) -> RecurrenceRule? {
        guard let pattern = dto.pattern, let type = pattern.type else { return nil }
        let interval = max(1, pattern.interval ?? 1)
        let weekStart = pattern.firstDayOfWeek.flatMap(weekday(named:)) ?? .monday
        let end = end(of: dto.range, in: zone)
        let plain = (pattern.daysOfWeek ?? []).compactMap(weekday(named:)).map { RecurrenceRule.WeekdayOccurrence($0) }
        let ordinal = indexes.first { $0.name == pattern.index?.lowercased() }?.ordinal
        let counted = plain.map { RecurrenceRule.WeekdayOccurrence($0.weekday, ordinal: ordinal) }
        func rule(_ frequency: RecurrenceRule.Frequency, weekdays: [RecurrenceRule.WeekdayOccurrence] = [], monthDays: [Int] = [], months: [Int] = []) -> RecurrenceRule {
            RecurrenceRule(frequency: frequency, interval: interval, weekdays: weekdays, monthDays: monthDays, months: months, end: end, weekStart: weekStart)
        }
        switch type {
        case "daily": return rule(.daily)
        case "weekly": return rule(.weekly, weekdays: plain)
        case "absoluteMonthly": return rule(.monthly, monthDays: pattern.dayOfMonth.map { [$0] } ?? [])
        case "relativeMonthly": return rule(.monthly, weekdays: counted)
        case "absoluteYearly":
            return rule(.yearly, monthDays: pattern.dayOfMonth.map { [$0] } ?? [], months: pattern.month.map { [$0] } ?? [])
        case "relativeYearly": return rule(.yearly, weekdays: counted, months: pattern.month.map { [$0] } ?? [])
        default: return nil
        }
    }

    private static func end(of range: GraphRecurrenceDTO.Range?, in zone: TimeZone) -> RecurrenceRule.End {
        switch range?.type {
        case "numbered": return .count(max(1, range?.numberOfOccurrences ?? 1))
        case "endDate":
            let rangeZone = range?.recurrenceTimeZone.flatMap(WindowsTimeZones.timeZone(for:)) ?? zone
            guard let text = range?.endDate, let last = GraphTime.date(text),
                  let after = AllDay.startOfDay(last.adding(days: 1), in: rangeZone) else { return .never }
            return .until(after.addingTimeInterval(-1))   // the end of the last day
        default: return .never
        }
    }

    // MARK: Writing

    /// The `recurrence` object for an event whose first occurrence is on `start` (a date in `zone`).
    static func recurrence(from rule: RecurrenceRule, start: CalendarDate, in zone: TimeZone) throws -> [String: Any] {
        try rule.validate()
        let unsupported = WriteError.unsupported(fields: [.recurrence])
        var pattern: [String: Any] = ["interval": rule.interval, "firstDayOfWeek": name(of: rule.weekStart)]
        switch rule.frequency {
        case .daily:
            pattern["type"] = "daily"
        case .weekly:
            pattern["type"] = "weekly"
            pattern["daysOfWeek"] = rule.weekdays.isEmpty ? [weekdayName(of: start)] : rule.weekdays.map { name(of: $0.weekday) }
        case .monthly:
            if rule.weekdays.isEmpty {
                let days = rule.monthDays.isEmpty ? [start.day] : rule.monthDays
                guard days.count == 1, days[0] >= 1 else { throw unsupported }
                pattern["type"] = "absoluteMonthly"
                pattern["dayOfMonth"] = days[0]
            } else {
                guard rule.monthDays.isEmpty else { throw unsupported }
                pattern["type"] = "relativeMonthly"
                try addRelative(rule.weekdays, to: &pattern)
            }
        case .yearly:
            let months = rule.months.isEmpty ? [start.month] : rule.months
            guard months.count == 1 else { throw unsupported }
            pattern["month"] = months[0]
            if rule.weekdays.isEmpty {
                let days = rule.monthDays.isEmpty ? [start.day] : rule.monthDays
                guard days.count == 1, days[0] >= 1 else { throw unsupported }
                pattern["type"] = "absoluteYearly"
                pattern["dayOfMonth"] = days[0]
            } else {
                guard rule.monthDays.isEmpty else { throw unsupported }
                pattern["type"] = "relativeYearly"
                try addRelative(rule.weekdays, to: &pattern)
            }
        case .secondly, .minutely, .hourly:
            throw unsupported
        }
        var range: [String: Any] = ["startDate": GraphTime.dateText(start), "recurrenceTimeZone": WindowsTimeZones.windowsName(for: zone) ?? "UTC"]
        switch rule.end {
        case .never: range["type"] = "noEnd"
        case .count(let n): range["type"] = "numbered"; range["numberOfOccurrences"] = n
        case .until(let date): range["type"] = "endDate"; range["endDate"] = GraphTime.dateText(AllDay.date(of: date, in: zone))
        }
        return ["pattern": pattern, "range": range]
    }

    /// Graph has one `index` for the whole pattern, so every weekday must carry the same ordinal, and it must be one
    /// Graph has (first to fourth, or last).
    private static func addRelative(_ days: [RecurrenceRule.WeekdayOccurrence], to pattern: inout [String: Any]) throws {
        let unsupported = WriteError.unsupported(fields: [.recurrence])
        guard let ordinal = days.first?.ordinal, days.allSatisfy({ $0.ordinal == ordinal }),
              let index = indexes.first(where: { $0.ordinal == ordinal }) else { throw unsupported }
        pattern["index"] = index.name
        pattern["daysOfWeek"] = days.map { name(of: $0.weekday) }
    }

    private static func weekdayName(of date: CalendarDate) -> String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        let noon = calendar.date(from: DateComponents(year: date.year, month: date.month, day: date.day, hour: 12))!
        // Calendar weekdays run Sunday = 1 ... Saturday = 7.
        let index = calendar.component(.weekday, from: noon)
        return ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"][index - 1]
    }

    // MARK: Splitting a series

    /// The number of occurrences of a `numbered` range; nil for any other range.
    static func occurrenceCount(in recurrence: [String: Any]) -> Int? {
        let range = recurrence["range"] as? [String: Any]
        guard range?["type"] as? String == "numbered" else { return nil }
        return range?["numberOfOccurrences"] as? Int
    }

    /// Counts the master's scheduled slots, including deleted and moved occurrences. Graph's instances endpoint
    /// reports current instances, so it cannot establish a numbered range's position after exceptions.
    static func remainingCount(in recurrence: [String: Any], from split: CalendarDate) throws -> Int {
        let unsupported = WriteError.unsupported(fields: [.recurrence])
        guard let total = occurrenceCount(in: recurrence), total > 0, total <= 10_000,
              let range = recurrence["range"] as? [String: Any],
              let startText = range["startDate"] as? String, let start = GraphTime.date(startText),
              let pattern = recurrence["pattern"] as? [String: Any],
              let type = pattern["type"] as? String,
              let interval = pattern["interval"] as? Int, interval > 0 else { throw unsupported }
        guard start <= split else { throw WriteError.invalid("the split precedes the series start") }

        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        func date(_ day: CalendarDate) -> Date? {
            calendar.date(from: DateComponents(year: day.year, month: day.month, day: day.day, hour: 12))
        }
        guard let origin = date(start), let target = date(split),
              AllDay.date(of: origin, in: calendar.timeZone) == start,
              AllDay.date(of: target, in: calendar.timeZone) == split else { throw unsupported }
        let distance = calendar.dateComponents([.day], from: origin, to: target).day ?? -1
        // A finite scan avoids unbounded work on malformed Graph data. Older sparse series beyond this horizon
        // are rejected before the master is changed, rather than assigned an unsafe continuation count.
        guard distance >= 0, distance <= 36_600 else { throw unsupported }
        let names = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"]
        let days = pattern["daysOfWeek"] as? [String] ?? []
        guard days.allSatisfy({ names.contains($0.lowercased()) }) else { throw unsupported }
        let startWeekday = calendar.component(.weekday, from: origin)
        let selected = Set((type == "weekly" && days.isEmpty ? [names[startWeekday - 1]] : days).map { $0.lowercased() })
        let firstDayName = (pattern["firstDayOfWeek"] as? String ?? "monday").lowercased()
        guard let firstDayIndex = names.firstIndex(of: firstDayName) else { throw unsupported }
        let weekOffset = (startWeekday - 1 - firstDayIndex + 7) % 7
        let indexName = (pattern["index"] as? String)?.lowercased()
        let ordinal = ["first": 1, "second": 2, "third": 3, "fourth": 4, "last": -1][indexName ?? ""]
        if (type == "relativeMonthly" || type == "relativeYearly") && (ordinal == nil || selected.isEmpty) { throw unsupported }
        if (type == "absoluteMonthly" || type == "absoluteYearly") && (pattern["dayOfMonth"] as? Int).map({ !(1...31).contains($0) }) != false { throw unsupported }
        if (type == "absoluteYearly" || type == "relativeYearly") && (pattern["month"] as? Int).map({ !(1...12).contains($0) }) != false { throw unsupported }
        guard ["daily", "weekly", "absoluteMonthly", "relativeMonthly", "absoluteYearly", "relativeYearly"].contains(type) else { throw unsupported }

        func relativeMatch(_ value: Date, _ components: DateComponents) -> Bool {
            guard let ordinal, let monthRange = calendar.range(of: .day, in: .month, for: value) else { return false }
            var matches: [Int] = []
            for day in monthRange {
                guard let candidate = calendar.date(from: DateComponents(year: components.year, month: components.month, day: day, hour: 12)) else { continue }
                if selected.contains(names[calendar.component(.weekday, from: candidate) - 1]) { matches.append(day) }
            }
            guard !matches.isEmpty else { return false }
            let selectedDay = ordinal == -1 ? matches.last : (ordinal > 0 && ordinal <= matches.count ? matches[ordinal - 1] : nil)
            return components.day == selectedDay
        }
        var counted = 0
        for offset in 0...distance {
            guard let value = calendar.date(byAdding: .day, value: offset, to: origin) else { throw unsupported }
            let c = calendar.dateComponents([.year, .month, .day, .weekday], from: value)
            let monthDistance = (c.year! - start.year) * 12 + c.month! - start.month
            let yearDistance = c.year! - start.year
            let occurs: Bool
            switch type {
            case "daily": occurs = offset % interval == 0
            case "weekly":
                let week = (offset + weekOffset) / 7
                occurs = week % interval == 0 && selected.contains(names[c.weekday! - 1])
            case "absoluteMonthly": occurs = monthDistance % interval == 0 && c.day == pattern["dayOfMonth"] as? Int
            case "relativeMonthly": occurs = monthDistance % interval == 0 && relativeMatch(value, c)
            case "absoluteYearly":
                occurs = yearDistance % interval == 0 && c.month == pattern["month"] as? Int && c.day == pattern["dayOfMonth"] as? Int
            default:
                occurs = yearDistance % interval == 0 && c.month == pattern["month"] as? Int && relativeMatch(value, c)
            }
            if occurs { counted += 1 }
            if offset == distance {
                guard occurs, counted <= total else { throw WriteError.invalid("the split is not a remaining recurrence slot") }
                return total - counted + 1
            }
            if counted >= total { throw WriteError.invalid("the split is beyond the numbered series") }
        }
        throw unsupported
    }

    /// The same recurrence cut so its last occurrence is before `split`: the range becomes an `endDate` of the day
    /// before the split. Graph counts a series in days at the finest, so one occurrence a day is the most there is.
    static func truncated(_ recurrence: [String: Any], endingBefore split: CalendarDate) -> [String: Any] {
        var range = recurrence["range"] as? [String: Any] ?? [:]
        range["type"] = "endDate"
        range["endDate"] = GraphTime.dateText(split.adding(days: -1))
        range["numberOfOccurrences"] = nil
        var copy = recurrence
        copy["range"] = range
        return copy
    }

    /// The recurrence for the series that continues from `start`: the same pattern, starting there, with `remaining`
    /// occurrences when the original was counted.
    static func restarted(_ recurrence: [String: Any], at start: CalendarDate, remaining: Int?) -> [String: Any] {
        var range = recurrence["range"] as? [String: Any] ?? [:]
        range["startDate"] = GraphTime.dateText(start)
        if let remaining { range["numberOfOccurrences"] = remaining }
        var copy = recurrence
        copy["range"] = range
        return copy
    }
}
