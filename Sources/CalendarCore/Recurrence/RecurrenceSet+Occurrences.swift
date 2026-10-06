import Foundation

/// The occurrences of a series that overlap a window.
public struct RecurrenceExpansion: Sendable, Equatable {
    /// Original starts (the instants the rules put the occurrences at), sorted and without duplicates.
    public var starts: [Date]
    /// True when the limit stopped the expansion early.
    public var truncated: Bool
    /// True when an `RRULE` line could not be read (it sits in `unparsed`): the starts then hold only the anchor, and a
    /// caller should show just the first occurrence and the series' exceptions.
    public var hasUnreadableRule: Bool
    public init(starts: [Date], truncated: Bool, hasUnreadableRule: Bool) {
        self.starts = starts
        self.truncated = truncated
        self.hasUnreadableRule = hasUnreadableRule
    }
}

extension RecurrenceSet {
    /// True when an `RRULE` line could not be read (it sits in `unparsed`).
    public var hasUnreadableRule: Bool {
        unparsed.contains { $0.trimmingCharacters(in: .whitespaces).uppercased().hasPrefix("RRULE") }
    }

    /// Every occurrence whose span (`start ..< start + duration`) overlaps `window`: the rules' instances from `anchor`
    /// plus `extraDates`, minus `excludedDates` (all-day series compare calendar days in `timeZone`). At most `limit`
    /// starts. Expansion skips whole periods before the window, so an old series stays cheap.
    public func occurrences(
        anchor: Date, duration: TimeInterval, timeZone: TimeZone, isAllDay: Bool, overlapping window: DateInterval,
        limit: Int = 5000
    ) -> RecurrenceExpansion {
        let unreadable = hasUnreadableRule
        let span = max(duration, 0)
        func overlaps(_ start: Date) -> Bool {
            span == 0 ? (start >= window.start && start < window.end) : (start < window.end && start.addingTimeInterval(span) > window.start)
        }
        if unreadable {
            return RecurrenceExpansion(starts: overlaps(anchor) ? [anchor] : [], truncated: false, hasUnreadableRule: true)
        }
        var candidates: [Date] = [anchor]
        var truncated = false
        let skipTo = window.start.addingTimeInterval(-span)
        for rule in rules {
            // A COUNT rule cannot skip, so its instances before the window must not use up the limit; COUNT itself
            // bounds it (and the iteration cap guards the rest). Otherwise skipping leaves only a few early instances.
            let ruleLimit: Int = { if case .count = rule.end { return Int.max / 2 } else { return limit + 64 } }()
            let expansion = rule.instances(anchor: anchor, timeZone: timeZone, isAllDay: isAllDay, before: window.end,
                                           limit: ruleLimit, skipTo: skipTo)
            candidates += expansion.starts
            truncated = truncated || expansion.truncated
        }
        candidates += extraDates ?? []
        let excluded = excludedDates ?? []
        func key(_ date: Date) -> String {
            isAllDay ? "\(AllDay.date(of: date, in: timeZone))" : "\(date.timeIntervalSince1970)"
        }
        let excludedKeys = Set(excluded.map(key))
        var seen = Set<String>()
        var result: [Date] = []
        for start in candidates.sorted() where overlaps(start) && !excludedKeys.contains(key(start)) {
            if seen.insert(key(start)).inserted { result.append(start) }
        }
        if result.count > limit {
            result = Array(result.prefix(limit))
            truncated = true
        }
        return RecurrenceExpansion(starts: result, truncated: truncated, hasUnreadableRule: false)
    }

    /// How many instances the first rule generates strictly before `date`, the anchor included and excluded dates
    /// counted (RFC 5545 `COUNT` counts generated instances before `EXDATE` removes any). Used to split a `COUNT` rule.
    public func ruleInstanceCount(anchor: Date, timeZone: TimeZone, isAllDay: Bool, before date: Date) -> Int {
        guard let rule = rules.first else { return anchor < date ? 1 : 0 }
        return rule.instances(anchor: anchor, timeZone: timeZone, isAllDay: isAllDay, before: date, limit: Int.max / 2).starts.count
    }
}
