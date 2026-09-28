import CalendarCore
import Foundation

/// Turns a `TZID` into a `TimeZone`: an IANA id; the IANA id inside a path-style id (`/mozilla.org/.../America/New_York`);
/// a Windows name; the file's own `VTIMEZONE` (its `X-LIC-LOCATION`, else the system zone with the same offsets and
/// transition months, else a fixed offset). nil when nothing fits.
public struct TimeZoneResolver: Sendable {
    private let definitions: [String: ICalComponent]

    public init() { definitions = [:] }

    public init(calendar: ICalComponent) {
        var found: [String: ICalComponent] = [:]
        for definition in calendar.components(named: "VTIMEZONE") {
            if let id = definition.property("TZID")?.value { found[id] = definition }
        }
        definitions = found
    }

    public func zone(for tzid: String) -> TimeZone? {
        let id = tzid.trimmingCharacters(in: .whitespaces)
        if let zone = TimeZone(identifier: id) { return zone }
        let parts = id.split(separator: "/").map(String.init)
        if parts.count > 1 {
            for start in 1..<parts.count {
                if let zone = TimeZone(identifier: parts[start...].joined(separator: "/")) { return zone }
            }
        }
        if let zone = WindowsTimeZones.timeZone(for: id) { return zone }
        if let definition = definitions[id] { return Self.zone(matching: definition) }
        return nil
    }

    /// The instant a value names. Floating times and dates are read in `zone`; an unknown TZID too.
    public func date(_ value: ICalDateValue, floating zone: TimeZone) -> Date? {
        switch value {
        case .date(let day): return AllDay.startOfDay(day, in: zone)
        case .utc(let instant): return instant
        case .local(let local, let tzid):
            let resolved = tzid.flatMap { self.zone(for: $0) } ?? zone
            return WallClock.date(local.date, hour: local.hour, minute: local.minute, second: local.second, in: resolved)
        }
    }

    /// The zone to show a value in: its TZID's zone, UTC for a UTC value, `fallback` for floating times and dates.
    public func displayZone(_ value: ICalDateValue, fallback: TimeZone) -> TimeZone {
        switch value {
        case .utc: return TimeZone(identifier: "UTC")!
        case .local(_, let tzid?): return zone(for: tzid) ?? fallback
        default: return fallback
        }
    }

    // MARK: VTIMEZONE matching

    private static func zone(matching definition: ICalComponent) -> TimeZone? {
        if let location = definition.property("X-LIC-LOCATION")?.value, let zone = TimeZone(identifier: location) { return zone }
        let observances = definition.components.filter { $0.name == "STANDARD" || $0.name == "DAYLIGHT" }
        func latest(_ kind: String) -> ICalComponent? {
            observances.filter { $0.name == kind }.max { ($0.property("DTSTART")?.value ?? "") < ($1.property("DTSTART")?.value ?? "") }
        }
        guard let standard = latest("STANDARD"), let standardOffset = offset(standard.property("TZOFFSETTO")?.value) else { return nil }
        let daylight = latest("DAYLIGHT")
        let daylightOffset = daylight.flatMap { offset($0.property("TZOFFSETTO")?.value) }
        let year = max(2000, Int(String((standard.property("DTSTART")?.value ?? "2000").prefix(4))) ?? 2000)
        var utc = Calendar(identifier: .gregorian)
        utc.timeZone = TimeZone(identifier: "UTC")!
        let january = utc.date(from: DateComponents(year: year, month: 1, day: 15, hour: 12))!
        let july = utc.date(from: DateComponents(year: year, month: 7, day: 15, hour: 12))!
        let wanted: Set<Int> = daylightOffset.map { [standardOffset, $0] } ?? [standardOffset]
        let candidates = TimeZone.knownTimeZoneIdentifiers.sorted().compactMap(TimeZone.init(identifier:)).filter {
            Set([$0.secondsFromGMT(for: january), $0.secondsFromGMT(for: july)]) == wanted
        }
        if let daylight, let months = transitionMonths(daylight: daylight, standard: standard) {
            let start = utc.date(from: DateComponents(year: year, month: 1, day: 1))!
            if let match = candidates.first(where: { transitionMonths(of: $0, after: start, calendar: utc) == months }) { return match }
        }
        return candidates.first ?? TimeZone(secondsFromGMT: standardOffset)
    }

    /// `+HHMM`, `-HHMM` or `+HHMMSS` in seconds.
    static func offset(_ text: String?) -> Int? {
        guard let text, text.count == 5 || text.count == 7, let sign = text.first, sign == "+" || sign == "-" else { return nil }
        let digits = Array(text.dropFirst())
        guard digits.allSatisfy({ $0.isASCII && $0.isNumber }) else { return nil }
        let hours = Int(String(digits[0..<2]))!, minutes = Int(String(digits[2..<4]))!
        let seconds = digits.count == 6 ? Int(String(digits[4..<6]))! : 0
        return (sign == "-" ? -1 : 1) * (hours * 3600 + minutes * 60 + seconds)
    }

    /// The months daylight time starts and ends in, from the observances' `RRULE:...;BYMONTH=` (or their `DTSTART`).
    private static func transitionMonths(daylight: ICalComponent, standard: ICalComponent) -> [Int]? {
        func month(_ observance: ICalComponent) -> Int? {
            if let rule = observance.property("RRULE")?.value,
               let part = rule.split(separator: ";").first(where: { $0.uppercased().hasPrefix("BYMONTH=") }) {
                return Int(part.dropFirst(8))
            }
            return observance.property("DTSTART").flatMap { Int(String($0.value.dropFirst(4).prefix(2))) }
        }
        guard let start = month(daylight), let end = month(standard) else { return nil }
        return [start, end]
    }

    private static func transitionMonths(of zone: TimeZone, after start: Date, calendar: Calendar) -> [Int]? {
        guard let first = zone.nextDaylightSavingTimeTransition(after: start),
              let second = zone.nextDaylightSavingTimeTransition(after: first) else { return nil }
        let (a, b) = zone.isDaylightSavingTime(for: first) ? (first, second) : (second, first)
        return [calendar.component(.month, from: a), calendar.component(.month, from: b)]
    }
}
