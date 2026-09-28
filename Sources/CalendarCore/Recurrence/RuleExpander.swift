import Foundation

/// The starts a rule generated, and whether a limit stopped it early.
public struct RuleExpansion: Sendable, Equatable {
    public var starts: [Date]
    /// True when `limit` or the iteration cap stopped the expansion before the rule's end or the bound.
    public var truncated: Bool
    public init(starts: [Date], truncated: Bool) {
        self.starts = starts
        self.truncated = truncated
    }
}

extension RecurrenceRule {
    /// The starts this rule generates from `anchor` (iCalendar `DTSTART`), in order. `anchor` is always the first start
    /// (RFC 5545 counts it as the first instance even when the rule would not generate it); then every generated start
    /// after it, until the rule's `COUNT` (which includes the anchor) or `UNTIL` (inclusive), the first start at or
    /// after `bound`, or `limit` starts. Expansion is on wall-clock time in `timeZone`, so a 09:00 rule stays at 09:00
    /// across daylight saving; a local time that does not exist moves forward by the gap and a repeated one uses its
    /// first occurrence. An all-day rule gives canonical all-day starts (`AllDay.startOfDay`).
    ///
    /// `skipTo` lets a caller that only needs starts from some instant on skip whole periods before it without
    /// generating them (a daily series from years ago stays cheap). It is ignored when the rule has a `COUNT`, which
    /// needs every earlier instance. The anchor is still returned first.
    ///
    /// Limitation: `BYWEEKNO` only matches days inside the period's own year (week 1 days in the previous December
    /// are not generated).
    public func instances(
        anchor: Date, timeZone: TimeZone, isAllDay: Bool, before bound: Date, limit: Int, skipTo: Date? = nil
    ) -> RuleExpansion {
        RuleExpander(rule: self, anchor: anchor, zone: timeZone, isAllDay: isAllDay).run(bound: bound, limit: max(1, limit), skipTo: skipTo)
    }
}

struct RuleExpander {
    static let iterationCap = 200_000

    let rule: RecurrenceRule
    let anchor: Date
    let zone: TimeZone
    let isAllDay: Bool
    let calendar: Calendar
    let anchorDate: CalendarDate
    let anchorHour: Int, anchorMinute: Int, anchorSecond: Int

    // BY-parts after RFC 5545 defaults (see `init`).
    let byMonth: Set<Int>
    let byMonthDay: [Int]
    let byWeekday: [RecurrenceRule.WeekdayOccurrence]
    let byYearDay: [Int]
    let byWeekNo: [Int]
    let byHour: [Int], byMinute: [Int], bySecond: [Int]

    init(rule: RecurrenceRule, anchor: Date, zone: TimeZone, isAllDay: Bool) {
        self.rule = rule
        self.anchor = anchor
        self.zone = zone
        self.isAllDay = isAllDay
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = zone
        self.calendar = calendar
        let c = calendar.dateComponents([.year, .month, .day, .hour, .minute, .second], from: anchor)
        anchorDate = CalendarDate(year: c.year!, month: c.month!, day: c.day!)
        anchorHour = isAllDay ? 0 : c.hour!
        anchorMinute = isAllDay ? 0 : c.minute!
        anchorSecond = isAllDay ? 0 : c.second!

        var months = Set(rule.months)
        var monthDays = rule.monthDays
        var weekdays = rule.weekdays
        // RFC 5545: parts the rule leaves out come from DTSTART (the same defaults as python-dateutil).
        if rule.weekNumbers.isEmpty && rule.yearDays.isEmpty && rule.monthDays.isEmpty && rule.weekdays.isEmpty {
            switch rule.frequency {
            case .yearly:
                if months.isEmpty { months = [anchorDate.month] }
                monthDays = [anchorDate.day]
            case .monthly:
                monthDays = [anchorDate.day]
            case .weekly:
                weekdays = [RecurrenceRule.WeekdayOccurrence(Self.weekday(of: anchorDate))]
            default:
                break
            }
        }
        byMonth = months
        byMonthDay = monthDays
        byWeekday = weekdays
        byYearDay = rule.yearDays
        byWeekNo = rule.weekNumbers
        byHour = rule.hours
        byMinute = rule.minutes
        bySecond = rule.seconds
    }

    // MARK: Driver

    func run(bound: Date, limit: Int, skipTo: Date?) -> RuleExpansion {
        guard anchor < bound else { return RuleExpansion(starts: [], truncated: false) }
        var starts = [anchor]
        var produced = 1
        let maxCount: Int? = { if case .count(let n) = rule.end { return n } else { return nil } }()
        let until: Date? = { if case .until(let d) = rule.end { return d } else { return nil } }()
        if let maxCount, produced >= maxCount { return RuleExpansion(starts: starts, truncated: false) }
        if starts.count >= limit { return RuleExpansion(starts: starts, truncated: true) }

        var index = maxCount == nil ? firstIndex(skipTo: skipTo) : 0
        var iterations = 0
        while true {
            iterations += 1
            if iterations > Self.iterationCap { return RuleExpansion(starts: starts, truncated: true) }
            guard let period = period(at: index) else { break }
            if period.start >= bound { break }
            if let until, period.start > until { break }
            for candidate in candidates(in: period) {
                guard candidate > anchor else { continue }
                if let until, candidate > until { return RuleExpansion(starts: starts, truncated: false) }
                if candidate >= bound { return RuleExpansion(starts: starts, truncated: false) }
                starts.append(candidate)
                produced += 1
                if let maxCount, produced >= maxCount { return RuleExpansion(starts: starts, truncated: false) }
                if starts.count >= limit { return RuleExpansion(starts: starts, truncated: true) }
            }
            index += rule.interval
        }
        return RuleExpansion(starts: starts, truncated: false)
    }

    // MARK: Periods

    /// One period of the rule: the days it covers (daily and longer) or its first instant (sub-daily).
    struct Period {
        var start: Date
        var days: [CalendarDate]
    }

    /// The period `index` periods after the anchor's (a multiple of `interval`).
    func period(at index: Int) -> Period? {
        switch rule.frequency {
        case .yearly:
            let year = anchorDate.year + index
            let first = CalendarDate(year: year, month: 1, day: 1)
            return Period(start: startInstant(first), days: (0..<Self.daysInYear(year)).map { first.adding(days: $0) })
        case .monthly:
            let total = anchorDate.month - 1 + index
            let year = anchorDate.year + Int(floor(Double(total) / 12)), month = ((total % 12) + 12) % 12 + 1
            let first = CalendarDate(year: year, month: month, day: 1)
            return Period(start: startInstant(first), days: (0..<Self.daysInMonth(year, month)).map { first.adding(days: $0) })
        case .weekly:
            let first = weekStart(of: anchorDate).adding(days: 7 * index)
            return Period(start: startInstant(first), days: (0..<7).map { first.adding(days: $0) })
        case .daily:
            let day = anchorDate.adding(days: index)
            return Period(start: startInstant(day), days: [day])
        case .hourly, .minutely, .secondly:
            let unit: TimeInterval = rule.frequency == .hourly ? 3600 : rule.frequency == .minutely ? 60 : 1
            let floored = floorToUnit(anchor, unit: unit)
            return Period(start: floored.addingTimeInterval(unit * Double(index)), days: [])
        }
    }

    /// With no COUNT, the first period worth generating for `skipTo`: a multiple of `interval`, one interval early for safety.
    func firstIndex(skipTo: Date?) -> Int {
        guard let skipTo, skipTo > anchor else { return 0 }
        let target = CalendarDate(skipTo, calendar: calendar)
        let raw: Int
        switch rule.frequency {
        case .yearly: raw = target.year - anchorDate.year
        case .monthly: raw = (target.year - anchorDate.year) * 12 + (target.month - anchorDate.month)
        case .weekly: raw = Self.daysBetween(weekStart(of: anchorDate), weekStart(of: target)) / 7
        case .daily: raw = Self.daysBetween(anchorDate, target)
        case .hourly: raw = Int(skipTo.timeIntervalSince(anchor) / 3600)
        case .minutely: raw = Int(skipTo.timeIntervalSince(anchor) / 60)
        case .secondly: raw = Int(skipTo.timeIntervalSince(anchor))
        }
        return max(0, (raw / rule.interval - 1) * rule.interval)
    }

    // MARK: Candidates

    func candidates(in period: Period) -> [Date] {
        var list: [Date]
        switch rule.frequency {
        case .hourly, .minutely, .secondly:
            list = subDailyCandidates(periodStart: period.start)
        default:
            let days = period.days.filter(dayMatches)
            list = []
            for day in days {
                if isAllDay {
                    if let start = AllDay.startOfDay(day, in: zone) { list.append(start) }
                    continue
                }
                for h in (byHour.isEmpty ? [anchorHour] : byHour.sorted()) {
                    for m in (byMinute.isEmpty ? [anchorMinute] : byMinute.sorted()) {
                        for s in (bySecond.isEmpty ? [anchorSecond] : bySecond.sorted()) {
                            if let date = localDate(day, h, m, s) { list.append(date) }
                        }
                    }
                }
            }
        }
        list = Array(Set(list)).sorted()
        guard !rule.setPositions.isEmpty, !list.isEmpty else { return list }
        var picked: [Date] = []
        for position in rule.setPositions {
            let i = position > 0 ? position - 1 : list.count + position
            if list.indices.contains(i) { picked.append(list[i]) }
        }
        return Array(Set(picked)).sorted()
    }

    private func subDailyCandidates(periodStart: Date) -> [Date] {
        let unit: TimeInterval = rule.frequency == .hourly ? 3600 : rule.frequency == .minutely ? 60 : 1
        var offsets: [TimeInterval] = []
        switch rule.frequency {
        case .hourly:
            for m in (byMinute.isEmpty ? [anchorMinute] : byMinute.sorted()) {
                for s in (bySecond.isEmpty ? [anchorSecond] : bySecond.sorted()) { offsets.append(Double(m * 60 + s)) }
            }
        case .minutely:
            offsets = (bySecond.isEmpty ? [anchorSecond] : bySecond.sorted()).map(Double.init)
        default:
            offsets = [0]
        }
        return offsets.compactMap { offset -> Date? in
            guard offset < unit else { return nil }
            let date = periodStart.addingTimeInterval(offset)
            let c = calendar.dateComponents([.year, .month, .day, .hour, .minute, .second], from: date)
            let day = CalendarDate(year: c.year!, month: c.month!, day: c.day!)
            guard dayMatches(day) else { return nil }
            if !byHour.isEmpty && !byHour.contains(c.hour!) { return nil }
            if rule.frequency != .hourly && !byMinute.isEmpty && !byMinute.contains(c.minute!) { return nil }
            if rule.frequency == .secondly && !bySecond.isEmpty && !bySecond.contains(c.second!) { return nil }
            return date
        }
    }

    func dayMatches(_ day: CalendarDate) -> Bool {
        if !byMonth.isEmpty && !byMonth.contains(day.month) { return false }
        if !byWeekNo.isEmpty {
            guard let (week, weeksInYear) = weekNumber(of: day) else { return false }
            if !byWeekNo.contains(where: { $0 == week || $0 == week - weeksInYear - 1 }) { return false }
        }
        if !byYearDay.isEmpty {
            let n = Self.daysBetween(CalendarDate(year: day.year, month: 1, day: 1), day) + 1
            let total = Self.daysInYear(day.year)
            if !byYearDay.contains(where: { $0 == n || $0 == n - total - 1 }) { return false }
        }
        if !byMonthDay.isEmpty {
            let total = Self.daysInMonth(day.year, day.month)
            if !byMonthDay.contains(where: { $0 == day.day || $0 == day.day - total - 1 }) { return false }
        }
        if !byWeekday.isEmpty && !byWeekday.contains(where: { weekdayMatches($0, day) }) { return false }
        return true
    }

    /// A plain weekday matches any such day; an ordinal counts within the month (monthly rules, and yearly rules with
    /// `BYMONTH`) or within the year (other yearly rules). Other frequencies ignore the ordinal.
    private func weekdayMatches(_ occurrence: RecurrenceRule.WeekdayOccurrence, _ day: CalendarDate) -> Bool {
        guard Self.weekday(of: day) == occurrence.weekday else { return false }
        guard let ordinal = occurrence.ordinal else { return true }
        let inMonth = rule.frequency == .monthly || (rule.frequency == .yearly && !byMonth.isEmpty)
        guard inMonth || rule.frequency == .yearly else { return true }
        let first = inMonth ? CalendarDate(year: day.year, month: day.month, day: 1) : CalendarDate(year: day.year, month: 1, day: 1)
        let length = inMonth ? Self.daysInMonth(day.year, day.month) : Self.daysInYear(day.year)
        let offset = Self.daysBetween(first, day)
        let fromStart = offset / 7 + 1
        let fromEnd = -((length - 1 - offset) / 7 + 1)
        return ordinal == fromStart || ordinal == fromEnd
    }

    // MARK: Weeks

    /// The day on or before `day` that is the rule's week start.
    func weekStart(of day: CalendarDate) -> CalendarDate {
        let order = RecurrenceRule.Weekday.mondayFirst
        let back = (order.firstIndex(of: Self.weekday(of: day))! - order.firstIndex(of: rule.weekStart)! + 7) % 7
        return day.adding(days: -back)
    }

    /// Week 1 is the first week (starting on `WKST`) with at least four days in the year. Returns the week number and
    /// the number of weeks in that year, or nil for a day that belongs to the neighbouring year's weeks.
    func weekNumber(of day: CalendarDate) -> (Int, Int)? {
        let firstWeek = weekStart(of: CalendarDate(year: day.year, month: 1, day: 4))
        let nextFirstWeek = weekStart(of: CalendarDate(year: day.year + 1, month: 1, day: 4))
        guard day >= firstWeek, day < nextFirstWeek else { return nil }
        return (Self.daysBetween(firstWeek, day) / 7 + 1, Self.daysBetween(firstWeek, nextFirstWeek) / 7)
    }

    // MARK: Date helpers

    private func localDate(_ day: CalendarDate, _ h: Int, _ m: Int, _ s: Int) -> Date? {
        WallClock.date(day, hour: h, minute: m, second: s, in: zone)
    }

    private func startInstant(_ day: CalendarDate) -> Date {
        AllDay.startOfDay(day, in: zone) ?? anchor
    }

    private func floorToUnit(_ date: Date, unit: TimeInterval) -> Date {
        let c = calendar.dateComponents([.year, .month, .day, .hour, .minute, .second], from: date)
        let parts: DateComponents
        switch unit {
        case 3600: parts = DateComponents(year: c.year, month: c.month, day: c.day, hour: c.hour)
        case 60: parts = DateComponents(year: c.year, month: c.month, day: c.day, hour: c.hour, minute: c.minute)
        default: parts = c
        }
        return calendar.date(from: parts) ?? date
    }

    static func weekday(of day: CalendarDate) -> RecurrenceRule.Weekday {
        var utc = Calendar(identifier: .gregorian)
        utc.timeZone = TimeZone(identifier: "UTC")!
        let noon = utc.date(from: DateComponents(year: day.year, month: day.month, day: day.day, hour: 12))!
        // Calendar weekday: 1 = Sunday ... 7 = Saturday.
        return [.sunday, .monday, .tuesday, .wednesday, .thursday, .friday, .saturday][utc.component(.weekday, from: noon) - 1]
    }

    static func daysBetween(_ a: CalendarDate, _ b: CalendarDate) -> Int {
        var utc = Calendar(identifier: .gregorian)
        utc.timeZone = TimeZone(identifier: "UTC")!
        let da = utc.date(from: DateComponents(year: a.year, month: a.month, day: a.day, hour: 12))!
        let db = utc.date(from: DateComponents(year: b.year, month: b.month, day: b.day, hour: 12))!
        return Int((db.timeIntervalSince(da) / 86_400).rounded())
    }

    static func daysInMonth(_ year: Int, _ month: Int) -> Int {
        switch month {
        case 2: return isLeap(year) ? 29 : 28
        case 4, 6, 9, 11: return 30
        default: return 31
        }
    }

    static func daysInYear(_ year: Int) -> Int { isLeap(year) ? 366 : 365 }
    static func isLeap(_ year: Int) -> Bool { (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 }
}

/// Wall-clock times in a zone, resolved the way iCalendar needs them: a time skipped by a daylight-saving jump moves
/// forward by the gap, and a time that happens twice uses the first. Computed from offsets rather than `Calendar`
/// matching policies so it behaves the same on swift-corelibs-foundation.
public enum WallClock {
    public static func date(_ day: CalendarDate, hour: Int, minute: Int, second: Int, in zone: TimeZone) -> Date? {
        var utc = Calendar(identifier: .gregorian)
        utc.timeZone = TimeZone(identifier: "UTC")!
        guard let wall = utc.date(from: DateComponents(year: day.year, month: day.month, day: day.day, hour: hour, minute: minute, second: second)),
              let noon = utc.date(from: DateComponents(year: day.year, month: day.month, day: day.day, hour: 12)) else { return nil }
        let before = TimeInterval(zone.secondsFromGMT(for: noon.addingTimeInterval(-86_400)))
        let after = TimeInterval(zone.secondsFromGMT(for: noon.addingTimeInterval(86_400)))
        let early = wall.addingTimeInterval(-before)   // read with the offset in force before a transition that day
        let late = wall.addingTimeInterval(-after)     // and with the offset after it
        let earlyOK = TimeInterval(zone.secondsFromGMT(for: early)) == before
        let lateOK = TimeInterval(zone.secondsFromGMT(for: late)) == after
        switch (earlyOK, lateOK) {
        case (true, true): return min(early, late)     // equal on an ordinary day; the first of a repeated time
        case (true, false): return early
        case (false, true): return late
        case (false, false): return early              // a skipped time, moved forward by the gap
        }
    }
}

extension RecurrenceRule.Weekday {
    static let mondayFirst: [RecurrenceRule.Weekday] = [.monday, .tuesday, .wednesday, .thursday, .friday, .saturday, .sunday]
}

extension CalendarDate {
    init(_ date: Date, calendar: Calendar) {
        let c = calendar.dateComponents([.year, .month, .day], from: date)
        self.init(year: c.year!, month: c.month!, day: c.day!)
    }

    /// Whole days from `self` to `other` (negative when `other` is earlier).
    public func days(to other: CalendarDate) -> Int { RuleExpander.daysBetween(self, other) }
}
