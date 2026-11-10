import CalendarCore
import Foundation
@testable import ICalendar

// ical/timezones.json
//
// Entry points: TimeZoneResolver.zone(for: String) -> TimeZone?, TimeZoneResolver.date(_ value: ICalDateValue, floating: TimeZone) -> Date?
// and TimeZoneResolver.displayZone(_ value: ICalDateValue, fallback: TimeZone) -> TimeZone. The resolver is built with
// TimeZoneResolver(calendar:) from the parsed "calendar" text (a VCALENDAR with VTIMEZONEs), or with
// TimeZoneResolver() when "calendar" is null.
//
// A date value V is one of
//   { "kind": "date", "date": { "year", "month", "day" } }
//   { "kind": "utc", "instant": <ms> }
//   { "kind": "local", "local": { "date": { "year", "month", "day" }, "hour", "minute", "second" }, "tzid": "<text>" | null }
//
//   fn "zone":        input { "fn", "calendar": "<text>" | null, "tzid": "<text>" }                       expected { "zone": "<identifier>" | null }
//   fn "date":        input { "fn", "calendar": ..., "value": V, "floating": "<IANA id>" }                expected { "instant": <ms> | null }
//   fn "displayZone": input { "fn", "calendar": ..., "value": V, "fallback": "<IANA id>" }                expected { "zone": "<identifier>" }
//
// 150 cases: 80 zone, 50 date, 20 displayZone. The calendars are synthetic: VTIMEZONEs written the way Outlook,
// Apple Calendar and Google write them (display-name TZIDs such as "(UTC-05:00) Eastern Time (US & Canada)", X-LIC-LOCATION,
// path-style TZIDs, Windows names) and zones with no IANA match (an offset of -03:37, a daylight rule on no real
// zone). The zone a definition matches without X-LIC-LOCATION is the first system zone with the same offsets and
// transition months, so those cases depend on the zone database of the macOS that ran the exporter.

private struct ZoneRule {
    var tzid: String
    var standardOffset: String       // TZOFFSETTO of STANDARD
    var daylightOffset: String?      // TZOFFSETTO of DAYLIGHT
    var daylightMonth: Int = 3, daylightWeek: String = "2SU"
    var standardMonth: Int = 11, standardWeek: String = "1SU"
    var location: String? = nil
}

private func vtimezone(_ r: ZoneRule) -> String {
    var lines = ["BEGIN:VTIMEZONE", "TZID:\(r.tzid)"]
    if let location = r.location { lines.append("X-LIC-LOCATION:\(location)") }
    if let dst = r.daylightOffset {
        lines += ["BEGIN:DAYLIGHT", "DTSTART:20070311T020000", "TZOFFSETFROM:\(r.standardOffset)", "TZOFFSETTO:\(dst)",
                  "RRULE:FREQ=YEARLY;BYMONTH=\(r.daylightMonth);BYDAY=\(r.daylightWeek)", "END:DAYLIGHT"]
    }
    lines += ["BEGIN:STANDARD", "DTSTART:20071104T020000", "TZOFFSETFROM:\(r.daylightOffset ?? r.standardOffset)", "TZOFFSETTO:\(r.standardOffset)"]
    if r.daylightOffset != nil { lines.append("RRULE:FREQ=YEARLY;BYMONTH=\(r.standardMonth);BYDAY=\(r.standardWeek)") }
    lines.append("END:STANDARD")
    lines.append("END:VTIMEZONE")
    return lines.joined(separator: "\r\n")
}

private func calendarText(_ zones: [ZoneRule]) -> String {
    (["BEGIN:VCALENDAR", "VERSION:2.0", "PRODID:-//Example//Vectors//EN"] + zones.map(vtimezone) + ["END:VCALENDAR", ""]).joined(separator: "\r\n")
}

private let outlookStyle: [ZoneRule] = [
    ZoneRule(tzid: "(UTC-05:00) Eastern Time (US & Canada)", standardOffset: "-0500", daylightOffset: "-0400"),
    ZoneRule(tzid: "(UTC-06:00) Central Time (US & Canada)", standardOffset: "-0600", daylightOffset: "-0500"),
    ZoneRule(tzid: "(UTC-08:00) Pacific Time (US & Canada)", standardOffset: "-0800", daylightOffset: "-0700"),
    ZoneRule(tzid: "(UTC+01:00) Amsterdam, Berlin, Bern, Rome, Stockholm, Vienna", standardOffset: "+0100", daylightOffset: "+0200",
             daylightMonth: 3, daylightWeek: "-1SU", standardMonth: 10, standardWeek: "-1SU"),
    ZoneRule(tzid: "(UTC+05:30) Chennai, Kolkata, Mumbai, New Delhi", standardOffset: "+0530", daylightOffset: nil),
    ZoneRule(tzid: "(UTC+09:00) Osaka, Sapporo, Tokyo", standardOffset: "+0900", daylightOffset: nil),
    ZoneRule(tzid: "(UTC+10:00) Canberra, Melbourne, Sydney", standardOffset: "+1000", daylightOffset: "+1100",
             daylightMonth: 10, daylightWeek: "1SU", standardMonth: 4, standardWeek: "1SU"),
    ZoneRule(tzid: "(UTC-03:30) Newfoundland", standardOffset: "-0330", daylightOffset: "-0230"),
    ZoneRule(tzid: "(UTC+12:45) Chatham Islands", standardOffset: "+1245", daylightOffset: "+1345",
             daylightMonth: 9, daylightWeek: "-1SU", standardMonth: 4, standardWeek: "1SU"),
    ZoneRule(tzid: "(UTC) Coordinated Universal Time", standardOffset: "+0000", daylightOffset: nil),
]
private let noMatch: [ZoneRule] = [
    ZoneRule(tzid: "Customized Time Zone", standardOffset: "-0337", daylightOffset: nil),
    ZoneRule(tzid: "Custom +0530", standardOffset: "+0530", daylightOffset: nil),
    ZoneRule(tzid: "Odd DST", standardOffset: "-0500", daylightOffset: "-0300", daylightMonth: 5, standardMonth: 8),
    ZoneRule(tzid: "Seconds", standardOffset: "+013045", daylightOffset: nil),
]
private let withLocation: [ZoneRule] = [
    ZoneRule(tzid: "My Office Zone", standardOffset: "+0100", daylightOffset: "+0200", location: "Europe/Berlin"),
    ZoneRule(tzid: "Weird", standardOffset: "+0900", daylightOffset: nil, location: "Asia/Tokyo"),
    ZoneRule(tzid: "Bad Location", standardOffset: "+0100", daylightOffset: nil, location: "Mars/Olympus"),
    ZoneRule(tzid: "Home", standardOffset: "-0800", daylightOffset: "-0700", location: "America/Los_Angeles"),
]

func exportICalTimezones() -> [VectorCase] {
    var rng = LCG(state: 0x72E5_01FE_0005)
    var cases: [VectorCase] = []
    let calendars: [(String?, [String])] = [
        (nil, []),
        (calendarText(outlookStyle), outlookStyle.map(\.tzid)),
        (calendarText(noMatch), noMatch.map(\.tzid)),
        (calendarText(withLocation), withLocation.map(\.tzid)),
        (calendarText([outlookStyle[0], noMatch[0], withLocation[0]]), [outlookStyle[0].tzid, noMatch[0].tzid, withLocation[0].tzid]),
        (calendarText([]), []),
    ]
    func resolver(_ text: String?) -> TimeZoneResolver {
        guard let text else { return TimeZoneResolver() }
        return TimeZoneResolver(calendar: try! ICalParser.parse(text))
    }
    func zoneVector(_ calendar: String?, _ tzid: String) {
        cases.append(VectorCase(
            name: "tz-zone-\(fourDigits(cases.count + 1))", input: ["fn": "zone", "calendar": orNull(calendar), "tzid": tzid],
            expected: ["zone": orNull(resolver(calendar).zone(for: tzid)?.identifier)]))
    }
    func valueJSON(_ v: ICalDateValue) -> [String: Any] {
        switch v {
        case .date(let d): return ["kind": "date", "date": dayJSON(d)]
        case .utc(let t): return ["kind": "utc", "instant": ms(t)]
        case .local(let l, let tzid):
            return ["kind": "local", "local": ["date": dayJSON(l.date), "hour": l.hour, "minute": l.minute, "second": l.second] as [String: Any], "tzid": orNull(tzid)]
        }
    }
    func dateVector(_ calendar: String?, _ v: ICalDateValue, floating: String) {
        cases.append(VectorCase(
            name: "tz-date-\(fourDigits(cases.count + 1))", input: ["fn": "date", "calendar": orNull(calendar), "value": valueJSON(v), "floating": floating],
            expected: ["instant": orNull(resolver(calendar).date(v, floating: zone(floating)))]))
    }
    func displayVector(_ calendar: String?, _ v: ICalDateValue, fallback: String) {
        cases.append(VectorCase(
            name: "tz-display-\(fourDigits(cases.count + 1))", input: ["fn": "displayZone", "calendar": orNull(calendar), "value": valueJSON(v), "fallback": fallback],
            expected: ["zone": resolver(calendar).displayZone(v, fallback: zone(fallback)).identifier]))
    }

    // zone(for:): hand-written.
    let plain = ["America/New_York", "UTC", "Europe/Berlin", " America/Chicago ", "/mozilla.org/20050126_1/America/New_York", "/softwarestudio.org/Tzfile/Europe/Paris",
                 "/citadel.org/20070101_1/Asia/Tokyo", "Eastern Standard Time", "Pacific Standard Time", "W. Europe Standard Time", "India Standard Time",
                 "Customized Time Zone", "", "Mars/Olympus", "Nowhere", "GMT", "Etc/GMT+5", "US/Pacific", "/a/b/c/Not/AZone"]
    for tzid in plain { zoneVector(nil, tzid) }
    for tzid in plain.prefix(6) { zoneVector(calendarText([]), tzid) }
    // The Outlook-style, no-match and X-LIC-LOCATION definitions.
    for (text, tzids) in calendars.dropFirst() {
        for tzid in tzids { zoneVector(text, tzid) }
    }
    zoneVector(calendars[1].0, "(UTC-05:00) Eastern Time (US & Canada) ")   // trailing space is trimmed before the lookup
    zoneVector(calendars[1].0, "(utc-05:00) eastern time (us & canada)")    // lookups are case sensitive
    zoneVector(calendars[2].0, "Not Defined Here")
    while cases.count < 80 {
        let (text, tzids) = rng.pick(calendars)
        let tzid = tzids.isEmpty || rng.chance(25) ? rng.pick(plain) : rng.pick(tzids)
        zoneVector(text, tzid)
    }

    // date(_:floating:).
    let day = CalendarDate(year: 2026, month: 3, day: 8)
    let sample: [ICalDateValue] = [
        .date(day), .date(CalendarDate(year: 2024, month: 2, day: 29)), .utc(instant(ms: utcMillis(2026, 1, 2, 9))),
        .local(LocalDateTime(date: day, hour: 9, minute: 30, second: 0), tzid: nil),
        .local(LocalDateTime(date: day, hour: 2, minute: 30, second: 0), tzid: "America/New_York"),
        .local(LocalDateTime(date: CalendarDate(year: 2026, month: 11, day: 1), hour: 1, minute: 30, second: 0), tzid: "America/New_York"),
        .local(LocalDateTime(date: day, hour: 9, minute: 0, second: 0), tzid: "(UTC-05:00) Eastern Time (US & Canada)"),
        .local(LocalDateTime(date: day, hour: 9, minute: 0, second: 0), tzid: "Customized Time Zone"),
        .local(LocalDateTime(date: day, hour: 9, minute: 0, second: 0), tzid: "My Office Zone"),
        .local(LocalDateTime(date: day, hour: 9, minute: 0, second: 0), tzid: "Eastern Standard Time"),
        .local(LocalDateTime(date: day, hour: 9, minute: 0, second: 0), tzid: "Unknown/Zone"),
    ]
    let floatingZones = ["America/New_York", "Asia/Tokyo", "UTC", "Pacific/Auckland"]
    for (i, v) in sample.enumerated() { dateVector(calendars[4].0, v, floating: floatingZones[i % floatingZones.count]) }
    while cases.count < 80 + 50 {
        let (text, tzids) = rng.pick(calendars)
        let d = CalendarDate(year: rng.int(2024...2027), month: rng.int(1...12), day: rng.int(1...28))
        let value: ICalDateValue
        switch rng.below(5) {
        case 0: value = .date(d)
        case 1: value = .utc(instant(ms: utcMillis(d.year, d.month, d.day, rng.int(0...23), rng.int(0...59))))
        default:
            let tzid: String? = rng.chance(15) ? nil : (tzids.isEmpty || rng.chance(40) ? rng.pick(plain) : rng.pick(tzids))
            value = .local(LocalDateTime(date: d, hour: rng.int(0...23), minute: rng.pick([0, 15, 30, 45]), second: 0), tzid: tzid)
        }
        dateVector(text, value, floating: rng.pick(floatingZones))
    }

    // displayZone.
    for (i, v) in sample.enumerated() { displayVector(calendars[4].0, v, fallback: floatingZones[i % floatingZones.count]) }
    while cases.count < 150 {
        let (text, tzids) = rng.pick(calendars)
        let d = CalendarDate(year: 2026, month: rng.int(1...12), day: rng.int(1...28))
        let tzid: String? = rng.chance(15) ? nil : (tzids.isEmpty || rng.chance(40) ? rng.pick(plain) : rng.pick(tzids))
        displayVector(text, .local(LocalDateTime(date: d, hour: 9, minute: 0, second: 0), tzid: tzid), fallback: rng.pick(floatingZones))
    }
    return cases
}
