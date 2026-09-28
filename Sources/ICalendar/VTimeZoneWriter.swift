import Foundation

/// Builds the `VTIMEZONE` a written event needs for its `TZID` (RFC 4791 requires one for each TZID used).
public enum VTimeZoneWriter {
    /// One observance per transition from a year before `start` through `end` (at most 400), or a single `STANDARD`
    /// observance for a zone without transitions. nil for UTC, which is written with `Z` instead.
    public static func component(for zone: TimeZone, from start: Date, through end: Date) -> ICalComponent? {
        if zone.identifier == "UTC" || zone.identifier == "GMT" { return nil }
        let utc = TimeZone(identifier: "UTC")!
        var observances: [ICalComponent] = []
        // The start of the year before `start`'s year, so a full prior year's transitions (including one that
        // falls before `start`'s own month, e.g. a March DST start when `start` is in June) are always included.
        var utcCalendar = Calendar(identifier: .gregorian)
        utcCalendar.timeZone = utc
        let startYear = utcCalendar.component(.year, from: start)
        var cursor = utcCalendar.date(from: DateComponents(year: startYear - 1, month: 1, day: 1))
            ?? start.addingTimeInterval(-366 * 86_400)
        while observances.count < 400, let transition = zone.nextDaylightSavingTimeTransition(after: cursor), transition <= end {
            let from = zone.secondsFromGMT(for: transition.addingTimeInterval(-1))
            let to = zone.secondsFromGMT(for: transition)
            var properties = [
                // DTSTART is the transition's local time read with the offset before it.
                ICalProperty(name: "DTSTART", value: ICalValues.localText(transition.addingTimeInterval(TimeInterval(from)), in: utc)),
                ICalProperty(name: "TZOFFSETFROM", value: offsetText(from)),
                ICalProperty(name: "TZOFFSETTO", value: offsetText(to)),
            ]
            if let name = zone.abbreviation(for: transition) { properties.append(ICalProperty(name: "TZNAME", text: name)) }
            observances.append(ICalComponent(name: zone.isDaylightSavingTime(for: transition) ? "DAYLIGHT" : "STANDARD", properties: properties))
            cursor = transition
        }
        if observances.isEmpty {
            let offset = zone.secondsFromGMT(for: start)
            observances = [ICalComponent(name: "STANDARD", properties: [
                ICalProperty(name: "DTSTART", value: "19700101T000000"),
                ICalProperty(name: "TZOFFSETFROM", value: offsetText(offset)),
                ICalProperty(name: "TZOFFSETTO", value: offsetText(offset)),
            ])]
        }
        return ICalComponent(name: "VTIMEZONE", properties: [ICalProperty(name: "TZID", value: zone.identifier)], components: observances)
    }

    static func offsetText(_ seconds: Int) -> String {
        let sign = seconds < 0 ? "-" : "+"
        let value = abs(seconds)
        let text = String(format: "%@%02d%02d", sign, value / 3600, (value % 3600) / 60)
        return value % 60 == 0 ? text : text + String(format: "%02d", value % 60)
    }
}
