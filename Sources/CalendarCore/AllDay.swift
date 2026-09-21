import Foundation

/// A calendar day without a time or zone.
public struct CalendarDate: Hashable, Sendable {
    public var year: Int
    public var month: Int
    public var day: Int
    public init(year: Int, month: Int, day: Int) { self.year = year; self.month = month; self.day = day }

    public func adding(days: Int) -> CalendarDate {
        var utc = Calendar(identifier: .gregorian)
        utc.timeZone = TimeZone(identifier: "UTC")!
        let base = utc.date(from: DateComponents(year: year, month: month, day: day, hour: 12))!
        let moved = utc.date(byAdding: .day, value: days, to: base)!
        let c = utc.dateComponents([.year, .month, .day], from: moved)
        return CalendarDate(year: c.year!, month: c.month!, day: c.day!)
    }
}

/// Conversions between all-day dates and the library's canonical instants: `start` is the start of the first
/// day in `zone`, `end` the start of the day after the last (exclusive). Connectors call this instead of doing
/// date math themselves.
public enum AllDay {
    private static func calendar(_ zone: TimeZone) -> Calendar {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = zone
        return c
    }

    /// The start of `date` in `zone`. Built from noon so a zone whose midnight does not exist that day still
    /// yields a valid instant (the first moment of the day).
    public static func startOfDay(_ date: CalendarDate, in zone: TimeZone) -> Date? {
        let cal = calendar(zone)
        guard let noon = cal.date(from: DateComponents(year: date.year, month: date.month, day: date.day, hour: 12))
        else { return nil }
        return cal.startOfDay(for: noon)
    }

    public static func date(of instant: Date, in zone: TimeZone) -> CalendarDate {
        let c = calendar(zone).dateComponents([.year, .month, .day], from: instant)
        return CalendarDate(year: c.year!, month: c.month!, day: c.day!)
    }

    public static func canonical(first: CalendarDate, endExclusive: CalendarDate, in zone: TimeZone) -> (start: Date, end: Date)? {
        guard let start = startOfDay(first, in: zone), let end = startOfDay(endExclusive, in: zone) else { return nil }
        return (start, end)
    }

    /// The dates a canonical range covers, read in `zone`.
    public static func dates(start: Date, end: Date, in zone: TimeZone) -> (first: CalendarDate, endExclusive: CalendarDate) {
        (date(of: start, in: zone), date(of: end, in: zone))
    }

    /// For providers that report the last covered day: the exclusive end date.
    public static func endExclusive(afterLast last: CalendarDate) -> CalendarDate { last.adding(days: 1) }
}

extension CalendarDate: Comparable {
    public static func < (a: CalendarDate, b: CalendarDate) -> Bool {
        (a.year, a.month, a.day) < (b.year, b.month, b.day)
    }
}
