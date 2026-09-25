import Foundation

/// Text that is not a well-formed RFC 5545 recurrence rule. Reading a rule never throws for a rule that is merely
/// unusual (that is `RecurrenceRule.validate()`'s job when writing); only malformed text does.
public enum RecurrenceParseError: Error, Sendable, Equatable {
    case malformed(String)
}

/// An iCalendar recurrence rule (RFC 5545 `RRULE`), used to read every provider's rules and to write the subset every
/// writer can express. `validate()` decides whether a rule can be written: what a writer cannot express is rejected,
/// not mangled. `unrecognizedParts` keeps `X-` parts and future extensions so a read rule can be written back
/// unchanged where a provider allows it. EXDATE and RDATE are not authorable through `EventPatch` (delete one
/// occurrence with `RecurrenceScope.thisInstance`); they are read through `RecurrenceSet`.
public struct RecurrenceRule: Hashable, Sendable {
    public enum Frequency: String, Sendable, Hashable {
        case secondly = "SECONDLY", minutely = "MINUTELY", hourly = "HOURLY"
        case daily = "DAILY", weekly = "WEEKLY", monthly = "MONTHLY", yearly = "YEARLY"
    }
    public enum Weekday: String, Sendable, Hashable, CaseIterable {
        case monday = "MO", tuesday = "TU", wednesday = "WE", thursday = "TH", friday = "FR", saturday = "SA", sunday = "SU"
    }
    public struct WeekdayOccurrence: Hashable, Sendable {
        public var weekday: Weekday
        /// "Second Tuesday" is 2, "last Friday" is -1. Reading accepts -53...53 (yearly rules count weeks of the
        /// year); a writable rule is limited to 1...5 or -5...-1 on monthly and yearly rules.
        public var ordinal: Int?
        public init(_ weekday: Weekday, ordinal: Int? = nil) {
            self.weekday = weekday
            self.ordinal = ordinal
        }
    }
    public enum End: Hashable, Sendable {
        case never, count(Int)
        /// An instant. Rendered as a UTC date-time for timed events and as a date in the event's zone for all-day ones.
        case until(Date)
    }
    /// A rule part the library does not model (an `X-` part or a future extension), kept in order.
    public struct UnrecognizedPart: Hashable, Sendable {
        public var name: String
        public var value: String
        public init(name: String, value: String) {
            self.name = name
            self.value = value
        }
    }

    public var frequency: Frequency
    public var interval: Int
    /// The day the week starts on (`WKST`); the iCalendar default is Monday.
    public var weekStart: Weekday
    public var weekdays: [WeekdayOccurrence]
    public var monthDays: [Int]
    public var months: [Int]
    public var yearDays: [Int]
    public var weekNumbers: [Int]
    public var setPositions: [Int]
    public var hours: [Int]
    public var minutes: [Int]
    public var seconds: [Int]
    public var end: End
    public var unrecognizedParts: [UnrecognizedPart]

    public init(frequency: Frequency, interval: Int = 1, weekdays: [WeekdayOccurrence] = [], monthDays: [Int] = [],
                months: [Int] = [], end: End = .never, weekStart: Weekday = .monday, yearDays: [Int] = [],
                weekNumbers: [Int] = [], setPositions: [Int] = [], hours: [Int] = [], minutes: [Int] = [],
                seconds: [Int] = [], unrecognizedParts: [UnrecognizedPart] = []) {
        self.frequency = frequency
        self.interval = interval
        self.weekStart = weekStart
        self.weekdays = weekdays
        self.monthDays = monthDays
        self.months = months
        self.yearDays = yearDays
        self.weekNumbers = weekNumbers
        self.setPositions = setPositions
        self.hours = hours
        self.minutes = minutes
        self.seconds = seconds
        self.end = end
        self.unrecognizedParts = unrecognizedParts
    }

    /// Whether every writer can store the rule: a daily-or-longer frequency and only the parts in the writable
    /// subset (`validate()` throws `.unsupported` for the rest).
    public var isWritableSubset: Bool {
        ![.secondly, .minutely, .hourly].contains(frequency) && weekStart == .monday && yearDays.isEmpty
            && weekNumbers.isEmpty && setPositions.isEmpty && hours.isEmpty && minutes.isEmpty && seconds.isEmpty
            && unrecognizedParts.isEmpty
    }

    public func validate() throws {
        guard interval >= 1 else { throw WriteError.invalid("recurrence interval must be at least 1") }
        if case .count(let n) = end, n < 1 { throw WriteError.invalid("recurrence count must be at least 1") }
        if frequency == .daily && !weekdays.isEmpty { throw WriteError.unsupported(fields: [.recurrence]) }
        for occurrence in weekdays {
            guard let ordinal = occurrence.ordinal else { continue }
            guard frequency == .monthly || frequency == .yearly else {
                throw WriteError.invalid("an ordinal weekday needs a monthly or yearly rule")
            }
            guard (1...5).contains(ordinal) || (-5 ... -1).contains(ordinal) else { throw WriteError.invalid("weekday ordinal must be 1...5 or -5...-1") }
        }
        if !monthDays.isEmpty {
            guard frequency == .monthly || frequency == .yearly else {
                throw WriteError.invalid("month days apply to monthly and yearly rules")
            }
            guard monthDays.allSatisfy({ (1...31).contains($0) || (-31 ... -1).contains($0) }) else { throw WriteError.invalid("month days must be 1...31 or -31...-1") }
        }
        if !months.isEmpty {
            guard frequency == .yearly else { throw WriteError.invalid("months apply to yearly rules only") }
            guard months.allSatisfy({ (1...12).contains($0) }) else { throw WriteError.invalid("months must be 1...12") }
        }
        // A rule read from a provider can be inspected and edited, but is written only when every writer can express it.
        if !isWritableSubset { throw WriteError.unsupported(fields: [.recurrence]) }
    }

    // MARK: Parsing

    /// Accepts an optional `RRULE:` prefix and every RFC 5545 rule part; parts the library does not model go to
    /// `unrecognizedParts`. A date-only `UNTIL` and a floating date-time `UNTIL` are read in `zone` (default UTC).
    /// Throws `RecurrenceParseError.malformed` for text that is not a well-formed rule (`INTERVAL=0`, `COUNT` with
    /// `UNTIL`, an unknown `FREQ`, an out-of-range value, a duplicate part). Whether a writer can store the rule is
    /// `validate()`'s job, so a rule is never refused at read time just because it is unusual.
    public init(rrule text: String, in zone: TimeZone? = nil) throws {
        var body = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if body.uppercased().hasPrefix("RRULE:") { body = String(body.dropFirst(6)) }
        var parts: [String: String] = [:]
        var order: [String] = []
        for piece in body.split(separator: ";") {
            let pair = piece.split(separator: "=", maxSplits: 1).map(String.init)
            guard pair.count == 2, !pair[1].isEmpty else { throw RecurrenceParseError.malformed("malformed RRULE part: \(piece)") }
            let key = pair[0].uppercased()
            guard parts[key] == nil else { throw RecurrenceParseError.malformed("duplicate RRULE part: \(key)") }
            parts[key] = pair[1]
            order.append(key)
        }
        guard let freqText = parts["FREQ"] else { throw RecurrenceParseError.malformed("RRULE has no FREQ") }
        guard let frequency = Frequency(rawValue: freqText.uppercased()) else {
            throw RecurrenceParseError.malformed("unknown FREQ: \(freqText)")
        }
        if parts["COUNT"] != nil && parts["UNTIL"] != nil { throw RecurrenceParseError.malformed("RRULE has both COUNT and UNTIL") }

        var interval = 1
        if let text = parts["INTERVAL"] {
            guard let value = Int(text), value >= 1 else { throw RecurrenceParseError.malformed("bad INTERVAL: \(text)") }
            interval = value
        }
        var end = End.never
        if let text = parts["COUNT"] {
            guard let value = Int(text), value >= 1 else { throw RecurrenceParseError.malformed("bad COUNT: \(text)") }
            end = .count(value)
        }
        if let text = parts["UNTIL"] { end = .until(try Self.parseUntil(text, zone: zone)) }
        var weekStart = Weekday.monday
        if let text = parts["WKST"] {
            guard let day = Weekday(rawValue: text.uppercased()) else { throw RecurrenceParseError.malformed("bad WKST: \(text)") }
            weekStart = day
        }
        let known: Set<String> = ["FREQ", "INTERVAL", "COUNT", "UNTIL", "WKST", "BYDAY", "BYMONTHDAY", "BYMONTH", "BYYEARDAY",
                                  "BYWEEKNO", "BYSETPOS", "BYHOUR", "BYMINUTE", "BYSECOND"]
        let unrecognized = order.filter { !known.contains($0) }.map { UnrecognizedPart(name: $0, value: parts[$0] ?? "") }

        try self.init(
            frequency: frequency, interval: interval,
            weekdays: try parts["BYDAY"].map { try $0.split(separator: ",", omittingEmptySubsequences: false).map { try Self.parseWeekday(String($0)) } } ?? [],
            monthDays: try Self.ints(parts["BYMONTHDAY"], "BYMONTHDAY", valid: { Self.signed($0, upTo: 31) }),
            months: try Self.ints(parts["BYMONTH"], "BYMONTH", valid: { (1...12).contains($0) }),
            end: end, weekStart: weekStart,
            yearDays: try Self.ints(parts["BYYEARDAY"], "BYYEARDAY", valid: { Self.signed($0, upTo: 366) }),
            weekNumbers: try Self.ints(parts["BYWEEKNO"], "BYWEEKNO", valid: { Self.signed($0, upTo: 53) }),
            setPositions: try Self.ints(parts["BYSETPOS"], "BYSETPOS", valid: { Self.signed($0, upTo: 366) }),
            hours: try Self.ints(parts["BYHOUR"], "BYHOUR", valid: { (0...23).contains($0) }),
            minutes: try Self.ints(parts["BYMINUTE"], "BYMINUTE", valid: { (0...59).contains($0) }),
            seconds: try Self.ints(parts["BYSECOND"], "BYSECOND", valid: { (0...60).contains($0) }),
            unrecognizedParts: unrecognized)
    }

    /// A non-zero value from -`limit` to `limit` (without `abs`, which traps on `Int.min`).
    private static func signed(_ value: Int, upTo limit: Int) -> Bool { value != 0 && value >= -limit && value <= limit }

    private static func ints(_ text: String?, _ name: String, valid: (Int) -> Bool) throws -> [Int] {
        guard let text else { return [] }
        return try text.split(separator: ",", omittingEmptySubsequences: false).map {
            guard let value = Int($0), valid(value) else { throw RecurrenceParseError.malformed("bad \(name) value: \($0)") }
            return value
        }
    }

    private static func parseWeekday(_ token: String) throws -> WeekdayOccurrence {
        let upper = token.uppercased()
        guard upper.count >= 2, let weekday = Weekday(rawValue: String(upper.suffix(2))) else {
            throw RecurrenceParseError.malformed("bad BYDAY value: \(token)")
        }
        let prefix = String(upper.dropLast(2))
        if prefix.isEmpty { return WeekdayOccurrence(weekday) }
        guard let ordinal = Int(prefix), Self.signed(ordinal, upTo: 53) else { throw RecurrenceParseError.malformed("bad BYDAY value: \(token)") }
        return WeekdayOccurrence(weekday, ordinal: ordinal)
    }

    private static let utc = TimeZone(identifier: "UTC")!

    private static func parseUntil(_ text: String, zone: TimeZone?) throws -> Date {
        let chars = Array(text.uppercased())
        let bad = RecurrenceParseError.malformed("bad UNTIL: \(text)")
        func isDigit(_ c: Character) -> Bool { c.isASCII && c.isNumber }
        /// The ASCII-digit value of `chars[range]`.
        func number(_ range: Range<Int>) throws -> Int {
            guard chars[range].allSatisfy(isDigit), let value = Int(String(chars[range])) else { throw bad }
            return value
        }
        var calendar = Calendar(identifier: .gregorian)
        if chars.count == 8 {
            calendar.timeZone = zone ?? utc
            let (year, month, day) = (try number(0..<4), try number(4..<6), try number(6..<8))
            guard let date = calendar.date(from: DateComponents(year: year, month: month, day: day)) else { throw bad }
            let back = calendar.dateComponents([.year, .month, .day], from: date)
            guard back.year == year, back.month == month, back.day == day else { throw bad }
            return date
        }
        if chars.count == 16, chars[8] == "T", chars[15] == "Z" {
            calendar.timeZone = utc
            let (year, month, day) = (try number(0..<4), try number(4..<6), try number(6..<8))
            let (hour, minute, second) = (try number(9..<11), try number(11..<13), try number(13..<15))
            let parts = DateComponents(year: year, month: month, day: day, hour: hour, minute: minute, second: second)
            guard let date = calendar.date(from: parts) else { throw bad }
            let back = calendar.dateComponents([.year, .month, .day, .hour, .minute, .second], from: date)
            guard back.year == year, back.month == month, back.day == day,
                  back.hour == hour, back.minute == minute, back.second == second else { throw bad }
            return date
        }
        if chars.count == 15, chars[8] == "T" {   // floating date-time: read in the series' zone
            calendar.timeZone = zone ?? utc
            let (year, month, day) = (try number(0..<4), try number(4..<6), try number(6..<8))
            let (hour, minute, second) = (try number(9..<11), try number(11..<13), try number(13..<15))
            let parts = DateComponents(year: year, month: month, day: day, hour: hour, minute: minute, second: second)
            guard let date = calendar.date(from: parts) else { throw bad }
            let back = calendar.dateComponents([.year, .month, .day, .hour, .minute, .second], from: date)
            guard back.year == year, back.month == month, back.day == day,
                  back.hour == hour, back.minute == minute, back.second == second else { throw bad }
            return date
        }
        throw bad
    }

    // MARK: Rendering

    /// The RRULE value without the `RRULE:` prefix, in a fixed order (unrecognized parts last, unchanged). Callers
    /// must `validate()` first when writing; this does not.
    public func rruleString(allDay: Bool, in zone: TimeZone?) -> String {
        var parts = ["FREQ=\(frequency.rawValue)"]
        if interval > 1 { parts.append("INTERVAL=\(interval)") }
        func list(_ name: String, _ values: [Int]) { if !values.isEmpty { parts.append(name + "=" + values.map(String.init).joined(separator: ",")) } }
        if !weekdays.isEmpty {
            parts.append("BYDAY=" + weekdays.map { ($0.ordinal.map(String.init) ?? "") + $0.weekday.rawValue }.joined(separator: ","))
        }
        list("BYMONTHDAY", monthDays)
        list("BYMONTH", months)
        list("BYYEARDAY", yearDays)
        list("BYWEEKNO", weekNumbers)
        list("BYHOUR", hours)
        list("BYMINUTE", minutes)
        list("BYSECOND", seconds)
        list("BYSETPOS", setPositions)
        if weekStart != .monday { parts.append("WKST=\(weekStart.rawValue)") }
        switch end {
        case .never: break
        case .count(let n): parts.append("COUNT=\(n)")
        case .until(let date): parts.append("UNTIL=" + Self.untilText(date, allDay: allDay, zone: zone))
        }
        parts += unrecognizedParts.map { "\($0.name)=\($0.value)" }
        return parts.joined(separator: ";")
    }

    /// `yyyyMMdd` in `zone` for all-day events, `yyyyMMdd'T'HHmmss'Z'` in UTC for timed ones.
    public static func untilText(_ date: Date, allDay: Bool, zone: TimeZone?) -> String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = allDay ? (zone ?? utc) : utc
        let c = calendar.dateComponents([.year, .month, .day, .hour, .minute, .second], from: date)
        if allDay { return String(format: "%04d%02d%02d", c.year!, c.month!, c.day!) }
        return String(format: "%04d%02d%02dT%02d%02d%02dZ", c.year!, c.month!, c.day!, c.hour!, c.minute!, c.second!)
    }

    public static func dateText(_ date: CalendarDate) -> String {
        String(format: "%04d%02d%02d", date.year, date.month, date.day)
    }
}
