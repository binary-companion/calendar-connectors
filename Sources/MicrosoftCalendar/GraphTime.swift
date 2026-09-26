import CalendarCore
import Foundation

/// Graph's date-time text: `2026-09-25T10:00:00.0000000` (no offset; the zone travels beside it) and ISO 8601
/// instants with a `Z` or an offset. Pure parsing and formatting with no formatter objects, so it is thread-safe.
enum GraphTime {
    private static let utc = TimeZone(identifier: "UTC")!

    /// `dateTime` read as local time in `zone`. A fractional part is ignored. nil for text that is not a date-time.
    static func parse(_ text: String, in zone: TimeZone) -> Date? {
        let c = Array(text.utf8)
        guard c.count >= 19, c[4] == 45, c[7] == 45, c[10] == 84 || c[10] == 32, c[13] == 58, c[16] == 58 else { return nil }
        func number(_ range: Range<Int>) -> Int? {
            var value = 0
            for i in range {
                guard c[i] >= 48, c[i] <= 57 else { return nil }
                value = value * 10 + Int(c[i] - 48)
            }
            return value
        }
        guard let year = number(0..<4), let month = number(5..<7), let day = number(8..<10),
              let hour = number(11..<13), let minute = number(14..<16), let second = number(17..<19) else { return nil }
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = zone
        guard let date = calendar.date(from: DateComponents(year: year, month: month, day: day, hour: hour, minute: minute, second: second))
        else { return nil }
        // A month or day out of range would roll over into another date; refuse it.
        let back = calendar.dateComponents([.year, .month, .day], from: date)
        guard back.year == year, back.month == month, back.day == day else { return nil }
        return date
    }

    /// The date part of `dateTime` (`2026-09-25T00:00:00.0000000` gives 2026-09-25).
    static func date(_ text: String) -> CalendarDate? {
        let parts = text.prefix(10).split(separator: "-").compactMap { Int($0) }
        guard text.count >= 10, parts.count == 3 else { return nil }
        return CalendarDate(year: parts[0], month: parts[1], day: parts[2])
    }

    /// `yyyy-MM-dd'T'HH:mm:ss` in `zone`.
    static func format(_ date: Date, in zone: TimeZone) -> String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = zone
        let c = calendar.dateComponents([.year, .month, .day, .hour, .minute, .second], from: date)
        return String(format: "%04d-%02d-%02dT%02d:%02d:%02d", c.year!, c.month!, c.day!, c.hour!, c.minute!, c.second!)
    }

    /// `yyyy-MM-dd'T'00:00:00` for a calendar day.
    static func midnight(_ date: CalendarDate) -> String {
        String(format: "%04d-%02d-%02dT00:00:00", date.year, date.month, date.day)
    }

    /// `yyyy-MM-dd` for a calendar day (a recurrence range's `startDate` and `endDate`).
    static func dateText(_ date: CalendarDate) -> String {
        String(format: "%04d-%02d-%02d", date.year, date.month, date.day)
    }

    /// An instant with a `Z` or `+hh:mm` offset (`lastModifiedDateTime`, `originalStart`), with any fractional part.
    /// Text with no offset is read as UTC.
    static func parseInstant(_ text: String) -> Date? {
        var body = text
        var offset: TimeInterval = 0
        if body.hasSuffix("Z") || body.hasSuffix("z") {
            body.removeLast()
        } else if body.count >= 6, let sign = body.dropLast(5).last, sign == "+" || sign == "-" {
            // Only an offset that follows the time part counts (a date's own hyphens are earlier).
            let tail = body.suffix(6)
            let parts = tail.dropFirst().split(separator: ":").compactMap { Int($0) }
            if tail.dropFirst(3).first == ":", parts.count == 2, body.count > 19 {
                offset = TimeInterval(parts[0] * 3600 + parts[1] * 60) * (tail.first == "-" ? -1 : 1)
                body.removeLast(6)
            }
        }
        guard let local = parse(body, in: utc) else { return nil }
        return local.addingTimeInterval(-offset)
    }

    /// `yyyy-MM-dd'T'HH:mm:ss'Z'` in UTC (query parameters such as `startDateTime`).
    static func instantText(_ date: Date) -> String { format(date, in: utc) + "Z" }
}
