import CalendarCore
import Foundation

public struct LocalDateTime: Hashable, Sendable {
    public var date: CalendarDate
    public var hour: Int
    public var minute: Int
    public var second: Int
    public init(date: CalendarDate, hour: Int, minute: Int, second: Int) {
        self.date = date
        self.hour = hour
        self.minute = minute
        self.second = second
    }
}

/// A DATE or DATE-TIME value as written: a date, a UTC instant, or a wall-clock time in a named zone (`tzid`) or
/// floating (`tzid == nil`, read in the calendar's zone).
public enum ICalDateValue: Hashable, Sendable {
    case date(CalendarDate)
    case utc(Date)
    case local(LocalDateTime, tzid: String?)
}

public enum ICalValues {
    private static let utcZone = TimeZone(identifier: "UTC")!

    /// The property's first value, read with its `VALUE` and `TZID` parameters; nil when it is malformed.
    public static func dateValue(_ property: ICalProperty) -> ICalDateValue? {
        dateValues(property).first
    }

    /// Every value of a list property (`EXDATE`, `RDATE`); empty when any token is malformed or the values are periods.
    public static func dateValues(_ property: ICalProperty) -> [ICalDateValue] {
        let kind = property.parameter("VALUE")?.uppercased()
        if kind == "PERIOD" { return [] }
        let tzid = property.parameter("TZID")
        var values: [ICalDateValue] = []
        for token in property.value.split(separator: ",") {
            guard let value = parse(String(token), isDate: kind == "DATE", tzid: tzid) else { return [] }
            values.append(value)
        }
        return values
    }

    static func parse(_ token: String, isDate: Bool, tzid: String?) -> ICalDateValue? {
        let chars = Array(token.trimmingCharacters(in: .whitespaces).uppercased())
        func number(_ range: Range<Int>) -> Int? {
            guard chars.count >= range.upperBound, chars[range].allSatisfy({ $0.isASCII && $0.isNumber }) else { return nil }
            return Int(String(chars[range]))
        }
        guard let year = number(0..<4), let month = number(4..<6), let day = number(6..<8),
              (1...12).contains(month), (1...31).contains(day) else { return nil }
        let date = CalendarDate(year: year, month: month, day: day)
        if chars.count == 8 { return .date(date) }
        guard !isDate, chars.count >= 15, chars[8] == "T", let hour = number(9..<11), let minute = number(11..<13),
              let second = number(13..<15), hour < 24, minute < 60, second <= 60 else { return nil }
        let local = LocalDateTime(date: date, hour: hour, minute: minute, second: min(second, 59))
        if chars.count == 16, chars[15] == "Z" {
            var calendar = Calendar(identifier: .gregorian)
            calendar.timeZone = utcZone
            guard let instant = calendar.date(from: DateComponents(year: year, month: month, day: day, hour: hour, minute: minute, second: local.second))
            else { return nil }
            return .utc(instant)
        }
        guard chars.count == 15 else { return nil }
        return .local(local, tzid: tzid)
    }

    /// RFC 5545 DURATION (`[+-]P[nW][nD][T[nH][nM][nS]]`); nil when malformed.
    public static func duration(_ text: String) -> TimeInterval? {
        var chars = Array(text.trimmingCharacters(in: .whitespaces).uppercased())
        var sign: Double = 1
        if chars.first == "-" { sign = -1; chars.removeFirst() } else if chars.first == "+" { chars.removeFirst() }
        guard chars.first == "P" else { return nil }
        chars.removeFirst()
        var total: Double = 0
        var number = ""
        var inTime = false
        var sawPart = false
        for c in chars {
            switch c {
            case "T": guard number.isEmpty, !inTime else { return nil }; inTime = true
            case "0"..."9": number.append(c)
            case "W", "D", "H", "M", "S":
                guard let n = Double(number) else { return nil }
                let unit: Double
                switch (c, inTime) {
                case ("W", false): unit = 604_800
                case ("D", false): unit = 86_400
                case ("H", true): unit = 3600
                case ("M", true): unit = 60
                case ("S", true): unit = 1
                default: return nil
                }
                total += n * unit
                number = ""
                sawPart = true
            default: return nil
            }
        }
        guard number.isEmpty, sawPart else { return nil }
        return sign * total
    }

    public static func durationText(_ seconds: TimeInterval) -> String {
        var rest = Int(abs(seconds).rounded())
        let sign = seconds < 0 ? "-" : ""
        if rest == 0 { return "PT0S" }
        let days = rest / 86_400; rest %= 86_400
        let hours = rest / 3600; rest %= 3600
        let minutes = rest / 60
        let secs = rest % 60
        var text = sign + "P" + (days > 0 ? "\(days)D" : "")
        if hours + minutes + secs > 0 {
            text += "T" + (hours > 0 ? "\(hours)H" : "") + (minutes > 0 ? "\(minutes)M" : "") + (secs > 0 ? "\(secs)S" : "")
        }
        return text
    }

    public static func utcText(_ date: Date) -> String { localText(date, in: utcZone) + "Z" }

    public static func dateText(_ date: CalendarDate) -> String {
        String(format: "%04d%02d%02d", date.year, date.month, date.day)
    }

    public static func localText(_ date: Date, in zone: TimeZone) -> String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = zone
        let c = calendar.dateComponents([.year, .month, .day, .hour, .minute, .second], from: date)
        return String(format: "%04d%02d%02dT%02d%02d%02d", c.year!, c.month!, c.day!, c.hour!, c.minute!, c.second!)
    }
}
