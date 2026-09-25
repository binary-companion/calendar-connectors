import Foundation

/// A recurrence rule in the RFC 5545 subset every connector can express. Rules outside it are rejected, not mangled.
/// EXDATE and RDATE are not authorable: delete one occurrence with `RecurrenceScope.thisInstance`.
public struct RecurrenceRule: Hashable, Sendable {
    public enum Frequency: String, Sendable, Hashable { case daily = "DAILY", weekly = "WEEKLY", monthly = "MONTHLY", yearly = "YEARLY" }
    public enum Weekday: String, Sendable, Hashable, CaseIterable {
        case monday = "MO", tuesday = "TU", wednesday = "WE", thursday = "TH", friday = "FR", saturday = "SA", sunday = "SU"
    }
    public struct WeekdayOccurrence: Hashable, Sendable {
        public var weekday: Weekday
        /// 1...5 or -5...-1 ("second Tuesday", "last Friday"); monthly and yearly rules only.
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

    public var frequency: Frequency
    public var interval: Int
    public var weekdays: [WeekdayOccurrence]
    public var monthDays: [Int]
    public var months: [Int]
    public var end: End

    public init(frequency: Frequency, interval: Int = 1, weekdays: [WeekdayOccurrence] = [], monthDays: [Int] = [],
                months: [Int] = [], end: End = .never) {
        self.frequency = frequency
        self.interval = interval
        self.weekdays = weekdays
        self.monthDays = monthDays
        self.months = months
        self.end = end
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
    }

    // MARK: Parsing

    /// Accepts an optional `RRULE:` prefix. A date-only `UNTIL` is read as the start of that day in `zone` (default UTC).
    /// Anything outside the subset throws `.unsupported([.recurrence])`; malformed text throws `.invalid`.
    public init(rrule text: String, in zone: TimeZone? = nil) throws {
        var body = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if body.uppercased().hasPrefix("RRULE:") { body = String(body.dropFirst(6)) }
        var parts: [String: String] = [:]
        for piece in body.split(separator: ";") {
            let pair = piece.split(separator: "=", maxSplits: 1).map(String.init)
            guard pair.count == 2, !pair[1].isEmpty else { throw WriteError.invalid("malformed RRULE part: \(piece)") }
            let key = pair[0].uppercased()
            guard parts[key] == nil else { throw WriteError.invalid("duplicate RRULE part: \(key)") }
            parts[key] = pair[1]
        }
        let supported: Set<String> = ["FREQ", "INTERVAL", "COUNT", "UNTIL", "BYDAY", "BYMONTHDAY", "BYMONTH", "WKST"]
        guard Set(parts.keys).isSubset(of: supported) else { throw WriteError.unsupported(fields: [.recurrence]) }
        guard let freqText = parts["FREQ"] else { throw WriteError.invalid("RRULE has no FREQ") }
        guard let frequency = Frequency(rawValue: freqText.uppercased()) else { throw WriteError.unsupported(fields: [.recurrence]) }
        if let wkst = parts["WKST"], wkst.uppercased() != "MO" { throw WriteError.unsupported(fields: [.recurrence]) }
        if parts["COUNT"] != nil && parts["UNTIL"] != nil { throw WriteError.unsupported(fields: [.recurrence]) }

        var interval = 1
        if let text = parts["INTERVAL"] {
            guard let value = Int(text) else { throw WriteError.invalid("bad INTERVAL: \(text)") }
            interval = value
        }
        var end = End.never
        if let text = parts["COUNT"] {
            guard let value = Int(text) else { throw WriteError.invalid("bad COUNT: \(text)") }
            end = .count(value)
        }
        if let text = parts["UNTIL"] { end = .until(try Self.parseUntil(text, zone: zone)) }

        self.init(
            frequency: frequency, interval: interval,
            weekdays: try parts["BYDAY"].map { try $0.split(separator: ",", omittingEmptySubsequences: false).map { try Self.parseWeekday(String($0)) } } ?? [],
            monthDays: try parts["BYMONTHDAY"].map { try Self.parseInts($0, name: "BYMONTHDAY") } ?? [],
            months: try parts["BYMONTH"].map { try Self.parseInts($0, name: "BYMONTH") } ?? [],
            end: end)
        try validate()
    }

    private static func parseInts(_ text: String, name: String) throws -> [Int] {
        try text.split(separator: ",", omittingEmptySubsequences: false).map {
            guard let value = Int($0) else { throw WriteError.invalid("bad \(name) value: \($0)") }
            return value
        }
    }

    private static func parseWeekday(_ token: String) throws -> WeekdayOccurrence {
        let upper = token.uppercased()
        guard upper.count >= 2, let weekday = Weekday(rawValue: String(upper.suffix(2))) else {
            throw WriteError.invalid("bad BYDAY value: \(token)")
        }
        let prefix = String(upper.dropLast(2))
        if prefix.isEmpty { return WeekdayOccurrence(weekday) }
        guard let ordinal = Int(prefix) else { throw WriteError.invalid("bad BYDAY value: \(token)") }
        return WeekdayOccurrence(weekday, ordinal: ordinal)
    }

    private static let utc = TimeZone(identifier: "UTC")!

    private static func parseUntil(_ text: String, zone: TimeZone?) throws -> Date {
        let chars = Array(text.uppercased())
        let bad = WriteError.invalid("bad UNTIL: \(text)")
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
        throw WriteError.unsupported(fields: [.recurrence])   // a floating date-time has no zone to resolve against
    }

    // MARK: Rendering

    /// The RRULE value without the `RRULE:` prefix, in a fixed order. Callers must `validate()` first; this does not.
    public func rruleString(allDay: Bool, in zone: TimeZone?) -> String {
        var parts = ["FREQ=\(frequency.rawValue)"]
        if interval > 1 { parts.append("INTERVAL=\(interval)") }
        if !weekdays.isEmpty {
            parts.append("BYDAY=" + weekdays.map { ($0.ordinal.map(String.init) ?? "") + $0.weekday.rawValue }.joined(separator: ","))
        }
        if !monthDays.isEmpty { parts.append("BYMONTHDAY=" + monthDays.map(String.init).joined(separator: ",")) }
        if !months.isEmpty { parts.append("BYMONTH=" + months.map(String.init).joined(separator: ",")) }
        switch end {
        case .never: break
        case .count(let n): parts.append("COUNT=\(n)")
        case .until(let date): parts.append("UNTIL=" + Self.untilText(date, allDay: allDay, zone: zone))
        }
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
