import Foundation

/// Everything that makes up a series' repetition: its rules and the extra and skipped dates. Read from a provider's
/// iCalendar lines (`RRULE`, `EXDATE`, `RDATE`) and written back unchanged; the library does no expansion, providers
/// expand occurrences on read.
public struct RecurrenceSet: Hashable, Sendable {
    public var rules: [RecurrenceRule]
    /// Extra occurrences (RDATE). nil means the source cannot say (EventKit); `[]` means there are none.
    public var extraDates: [Date]?
    /// Skipped occurrences (EXDATE). nil means the source cannot say (EventKit); `[]` means there are none.
    public var excludedDates: [Date]?
    /// Whole lines the library does not model (`EXRULE`, `RDATE;VALUE=PERIOD`, a rule that is malformed, dates in a
    /// zone it cannot resolve such as a Windows zone name), kept verbatim so a write does not lose them.
    public var unparsed: [String]

    public init(rules: [RecurrenceRule] = [], extraDates: [Date]? = [], excludedDates: [Date]? = [], unparsed: [String] = []) {
        self.rules = rules
        self.extraDates = extraDates
        self.excludedDates = excludedDates
        self.unparsed = unparsed
    }

    // MARK: Reading

    /// Reads a provider's recurrence lines. A read never fails: anything it does not understand goes to `unparsed`.
    /// `timeZone` and `isAllDay` come from the series' first event: floating date-times and date-only values are read in
    /// that zone (all-day dates through `AllDay`, so they land on the canonical midnight).
    public init(iCalendarLines lines: [String], timeZone: TimeZone, isAllDay: Bool) {
        var rules: [RecurrenceRule] = []
        var extra: [Date] = [], excluded: [Date] = []
        var unparsed: [String] = []
        for line in lines {
            let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
            guard let colon = trimmed.firstIndex(of: ":") else { unparsed.append(line); continue }
            let head = trimmed[..<colon].split(separator: ";").map(String.init)
            let value = String(trimmed[trimmed.index(after: colon)...])
            switch head.first?.uppercased() {
            case "RRULE":
                if let rule = try? RecurrenceRule(rrule: value, in: timeZone) { rules.append(rule) } else { unparsed.append(line) }
            case "EXDATE", "RDATE":
                if let dates = Self.parseDates(params: Array(head.dropFirst()), value: value, zone: timeZone, isAllDay: isAllDay) {
                    if head.first?.uppercased() == "EXDATE" { excluded += dates } else { extra += dates }
                } else {
                    unparsed.append(line)
                }
            default:
                unparsed.append(line)
            }
        }
        self.init(rules: rules, extraDates: extra, excludedDates: excluded, unparsed: unparsed)
    }

    /// nil when the value is a period, has an unknown zone, or is malformed.
    private static func parseDates(params: [String], value: String, zone: TimeZone, isAllDay: Bool) -> [Date]? {
        var tzid: TimeZone?
        var isDate = false
        for param in params {
            let pair = param.split(separator: "=", maxSplits: 1).map(String.init)
            guard pair.count == 2 else { return nil }
            switch pair[0].uppercased() {
            case "TZID":
                guard let resolved = TimeZone(identifier: pair[1]) else { return nil }
                tzid = resolved
            case "VALUE":
                switch pair[1].uppercased() {
                case "DATE": isDate = true
                case "DATE-TIME": break
                default: return nil   // PERIOD
                }
            default: break
            }
        }
        var dates: [Date] = []
        for token in value.split(separator: ",") {
            guard let date = parseOne(String(token), tzid: tzid, zone: zone, isDate: isDate) else { return nil }
            dates.append(date)
        }
        return dates.isEmpty ? nil : dates
    }

    private static func parseOne(_ text: String, tzid: TimeZone?, zone: TimeZone, isDate: Bool) -> Date? {
        let chars = Array(text.uppercased())
        func number(_ range: Range<Int>) -> Int? {
            guard chars.count >= range.upperBound, chars[range].allSatisfy({ $0.isASCII && $0.isNumber }) else { return nil }
            return Int(String(chars[range]))
        }
        guard let year = number(0..<4), let month = number(4..<6), let day = number(6..<8) else { return nil }
        if isDate || chars.count == 8 {
            guard chars.count == 8 else { return nil }
            return AllDay.startOfDay(CalendarDate(year: year, month: month, day: day), in: zone)
        }
        guard chars.count >= 15, chars[8] == "T", let hour = number(9..<11), let minute = number(11..<13), let second = number(13..<15) else { return nil }
        let utc = TimeZone(identifier: "UTC")!
        let isUTC = chars.count == 16 && chars[15] == "Z"
        guard chars.count == 15 || isUTC else { return nil }
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = isUTC ? utc : (tzid ?? zone)
        let parts = DateComponents(year: year, month: month, day: day, hour: hour, minute: minute, second: second)
        guard let date = calendar.date(from: parts) else { return nil }
        let back = calendar.dateComponents([.year, .month, .day, .hour, .minute, .second], from: date)
        guard back.year == year, back.month == month, back.day == day, back.hour == hour, back.minute == minute, back.second == second else { return nil }
        return date
    }

    // MARK: Writing

    /// The lines to store: one `RRULE` per rule, then `RDATE` and `EXDATE` (a date-time list per zone, `VALUE=DATE`
    /// for all-day series), then the unparsed lines, unchanged.
    public func iCalendarLines(timeZone: TimeZone, isAllDay: Bool) -> [String] {
        var lines = rules.map { "RRULE:" + $0.rruleString(allDay: isAllDay, in: timeZone) }
        if let extraDates, !extraDates.isEmpty { lines.append(Self.line("RDATE", extraDates, zone: timeZone, isAllDay: isAllDay)) }
        if let excludedDates, !excludedDates.isEmpty { lines.append(Self.line("EXDATE", excludedDates, zone: timeZone, isAllDay: isAllDay)) }
        return lines + unparsed
    }

    private static func line(_ name: String, _ dates: [Date], zone: TimeZone, isAllDay: Bool) -> String {
        var calendar = Calendar(identifier: .gregorian)
        if isAllDay {
            let days = dates.map { RecurrenceRule.dateText(AllDay.date(of: $0, in: zone)) }
            return "\(name);VALUE=DATE:" + days.joined(separator: ",")
        }
        let isUTC = zone.identifier == "GMT" || zone.identifier == "UTC"
        calendar.timeZone = zone
        let texts = dates.map { date -> String in
            let c = calendar.dateComponents([.year, .month, .day, .hour, .minute, .second], from: date)
            return String(format: "%04d%02d%02dT%02d%02d%02d", c.year!, c.month!, c.day!, c.hour!, c.minute!, c.second!) + (isUTC ? "Z" : "")
        }
        return (isUTC ? "\(name):" : "\(name);TZID=\(zone.identifier):") + texts.joined(separator: ",")
    }
}

/// A recurring series as its own object: the first occurrence's anchor time and everything that repeats it.
/// Instances stay parented to a series through `SeriesInfo`; the series owns the rule.
public struct CalendarSeries: Hashable, Sendable {
    public var seriesID: String
    public var calendarID: String
    /// The rule's anchor time (iCalendar `DTSTART`); an all-day series uses the canonical all-day form.
    public var start: Date
    public var timeZone: TimeZone
    public var isAllDay: Bool
    public var recurrence: RecurrenceSet

    public init(seriesID: String, calendarID: String, start: Date, timeZone: TimeZone, isAllDay: Bool, recurrence: RecurrenceSet) {
        self.seriesID = seriesID
        self.calendarID = calendarID
        self.start = start
        self.timeZone = timeZone
        self.isAllDay = isAllDay
        self.recurrence = recurrence
    }
}
