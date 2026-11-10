import CalendarCore
import Foundation

// Shared by the recurrence areas: seeded generation of rule texts and anchors. The recurrence areas draw from
// the same shapes (every FREQ; INTERVAL 1 to 4; BYDAY with and without ordinals; BYMONTHDAY positive and
// negative; BYMONTH, BYSETPOS, BYWEEKNO, BYYEARDAY, WKST; COUNT and UNTIL) but each seeds its own generator.

let weekdayCodes = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"]

/// An anchor: a local wall time in a zone, or a day (all-day).
struct Anchor {
    var zone: String
    var isAllDay: Bool
    var ms: Int
}

/// Anchors on the days that make recurrence hard: 29 February, the 31st, and wall times in or next to DST gaps
/// and overlaps (including a midnight that does not exist and one that happens twice).
func specialAnchors() -> [Anchor] {
    let timed: [(String, Int, Int, Int, Int, Int)] = [
        ("America/New_York", 2024, 3, 10, 2, 30), ("America/New_York", 2024, 3, 9, 2, 30), ("America/New_York", 2025, 3, 9, 2, 30),
        ("America/New_York", 2024, 11, 3, 1, 30), ("Europe/London", 2024, 3, 31, 1, 30), ("Europe/Berlin", 2024, 3, 31, 2, 30),
        ("Europe/Berlin", 2024, 10, 27, 2, 30), ("Australia/Sydney", 2024, 10, 6, 2, 30), ("Australia/Sydney", 2024, 4, 7, 2, 30),
        ("America/Havana", 2024, 3, 10, 0, 30), ("America/Sao_Paulo", 2018, 11, 4, 0, 30), ("Australia/Lord_Howe", 2024, 10, 6, 2, 15),
        ("Pacific/Auckland", 2024, 9, 29, 2, 30), ("America/Los_Angeles", 2026, 3, 8, 2, 0),
        ("America/New_York", 2024, 2, 29, 9, 0), ("Europe/Berlin", 2024, 2, 29, 23, 59), ("Asia/Tokyo", 2024, 2, 29, 0, 0),
        ("UTC", 2024, 2, 29, 12, 0), ("America/Chicago", 2026, 1, 31, 9, 0), ("Europe/London", 2025, 3, 31, 8, 0),
        ("Asia/Kolkata", 2024, 5, 31, 22, 30), ("America/New_York", 2025, 10, 31, 17, 0), ("Pacific/Auckland", 2026, 12, 31, 23, 0),
    ]
    var anchors = timed.map { z, y, m, d, h, mi in
        Anchor(zone: z, isAllDay: false, ms: ms(WallClock.date(CalendarDate(year: y, month: m, day: d), hour: h, minute: mi, second: 0, in: zone(z))!))
    }
    for (z, y, m, d) in [("America/New_York", 2024, 3, 10), ("America/Havana", 2024, 3, 10), ("America/Havana", 2024, 11, 3),
                         ("Pacific/Auckland", 2024, 9, 29), ("UTC", 2024, 2, 29), ("Asia/Tokyo", 2025, 1, 31), ("America/Sao_Paulo", 2018, 11, 4)] {
        anchors.append(Anchor(zone: z, isAllDay: true, ms: ms(AllDay.startOfDay(CalendarDate(year: y, month: m, day: d), in: zone(z))!)))
    }
    return anchors
}

func randomAnchor(_ rng: inout LCG) -> Anchor {
    let id = rng.pick(rulesZones)
    let allDay = rng.chance(22)
    var year = rng.int(2023...2027), month = rng.int(1...12), day = rng.int(1...28)
    if rng.chance(15) { day = rng.int(29...31) }
    let days = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    day = min(day, month == 2 ? (year % 4 == 0 ? 29 : 28) : days[month - 1])
    if rng.chance(3) { year = 2024; month = 2; day = 29 }
    let date = CalendarDate(year: year, month: month, day: day)
    if allDay { return Anchor(zone: id, isAllDay: true, ms: ms(AllDay.startOfDay(date, in: zone(id))!)) }
    let h = rng.chance(10) ? rng.pick([0, 1, 2, 3, 23]) : rng.int(0...23)
    let mi = rng.pick([0, 0, 0, 15, 30, 45, rng.int(0...59)])
    return Anchor(zone: id, isAllDay: false, ms: ms(WallClock.date(date, hour: h, minute: mi, second: rng.chance(10) ? rng.int(0...59) : 0, in: zone(id))!))
}

/// The seconds a frequency's period is about long.
func periodSeconds(_ freq: String) -> Int {
    switch freq {
    case "SECONDLY": 1
    case "MINUTELY": 60
    case "HOURLY": 3600
    case "DAILY": 86_400
    case "WEEKLY": 604_800
    case "MONTHLY": 2_592_000
    default: 31_536_000
    }
}

/// A random, parseable rule text for `freq`, together with its interval (for sizing the window).
func randomRule(_ freq: String, _ rng: inout LCG, anchor: Anchor) -> (text: String, interval: Int) {
    var parts = ["FREQ=\(freq)"]
    var interval = 1
    if rng.chance(55) {
        interval = freq == "SECONDLY" || freq == "MINUTELY" ? rng.pick([1, 2, 3, 4, 5, 15, 30]) : rng.int(1...4)
        if freq == "HOURLY" && rng.chance(25) { interval = rng.pick([6, 12]) }
        parts.append("INTERVAL=\(interval)")
    }
    func ints(_ lo: Int, _ hi: Int, count: Int, signedToo: Bool = false) -> String {
        var values = Set<Int>()
        while values.count < count {
            var v = rng.int(lo...hi)
            if signedToo && rng.chance(40) { v = -v }
            values.insert(v)
        }
        return values.sorted().map(String.init).joined(separator: ",")
    }
    func days(_ count: Int) -> String { rng.subset(weekdayCodes, count: count).joined(separator: ",") }
    func ordinalDays(maxOrdinal: Int, count: Int) -> String {
        (0..<count).map { _ in
            let n = rng.int(1...maxOrdinal) * (rng.chance(35) ? -1 : 1)
            return "\(n)\(rng.pick(weekdayCodes))"
        }.joined(separator: ",")
    }
    switch freq {
    case "SECONDLY":
        if rng.chance(25) { parts.append("BYSECOND=\(ints(0, 59, count: rng.int(1...3)))") }
    case "MINUTELY":
        if rng.chance(35) { parts.append("BYHOUR=\(ints(0, 23, count: rng.int(1...4)))") }
        if rng.chance(25) { parts.append("BYMINUTE=\(ints(0, 59, count: rng.int(1...3)))") }
    case "HOURLY":
        if rng.chance(35) { parts.append("BYHOUR=\(ints(0, 23, count: rng.int(2...8)))") }
        if rng.chance(30) { parts.append("BYMINUTE=\(ints(0, 59, count: rng.int(1...3)))") }
        if rng.chance(20) { parts.append("BYDAY=\(days(rng.int(1...5)))") }
    case "DAILY":
        if rng.chance(25) { parts.append("BYMONTH=\(ints(1, 12, count: rng.int(1...4)))") }
        if rng.chance(20) { parts.append("BYMONTHDAY=\(ints(1, 31, count: rng.int(1...4), signedToo: true))") }
        if rng.chance(25) { parts.append("BYDAY=\(days(rng.int(1...6)))") }
        if rng.chance(12) { parts.append("BYHOUR=\(ints(0, 23, count: rng.int(1...3)))") }
        if rng.chance(8) { parts.append("BYMINUTE=\(ints(0, 59, count: rng.int(1...3)))") }
    case "WEEKLY":
        if rng.chance(85) { parts.append("BYDAY=\(days(rng.int(1...4)))") }
        if rng.chance(50) { parts.append("WKST=\(rng.pick(weekdayCodes))") }
        if rng.chance(10) { parts.append("BYMONTH=\(ints(1, 12, count: rng.int(1...4)))") }
    case "MONTHLY":
        switch rng.below(6) {
        case 0, 1: parts.append("BYMONTHDAY=\(ints(1, 31, count: rng.int(1...3), signedToo: true))")
        case 2: parts.append("BYDAY=\(ordinalDays(maxOrdinal: 5, count: rng.int(1...2)))")
        case 3:
            parts.append("BYDAY=\(rng.chance(50) ? "MO,TU,WE,TH,FR" : days(rng.int(2...4)))")
            parts.append("BYSETPOS=\(ints(1, 4, count: rng.int(1...2), signedToo: true))")
        case 4: parts.append("BYDAY=\(days(rng.int(1...3)))")
        default: break
        }
        if rng.chance(15) { parts.append("BYMONTH=\(ints(1, 12, count: rng.int(2...6)))") }
        if rng.chance(8) { parts.append("WKST=\(rng.pick(weekdayCodes))") }
    default:   // YEARLY
        switch rng.below(7) {
        case 0:
            parts.append("BYMONTH=\(ints(1, 12, count: rng.int(1...3)))")
            parts.append("BYMONTHDAY=\(ints(1, 31, count: rng.int(1...2), signedToo: true))")
        case 1:
            parts.append("BYMONTH=\(ints(1, 12, count: rng.int(1...2)))")
            parts.append("BYDAY=\(ordinalDays(maxOrdinal: 5, count: rng.int(1...2)))")
        case 2: parts.append("BYYEARDAY=\(ints(1, 366, count: rng.int(1...4), signedToo: true))")
        case 3:
            parts.append("BYWEEKNO=\(ints(1, 53, count: rng.int(1...3), signedToo: true))")
            if rng.chance(80) { parts.append("BYDAY=\(days(rng.int(1...3)))") }
            if rng.chance(50) { parts.append("WKST=\(rng.pick(weekdayCodes))") }
        case 4: parts.append("BYDAY=\(ordinalDays(maxOrdinal: 53, count: 1))")
        case 5:
            parts.append("BYMONTH=\(ints(1, 12, count: rng.int(2...6)))")
            parts.append("BYDAY=\(days(rng.int(2...5)))")
            parts.append("BYSETPOS=\(ints(1, 3, count: 1, signedToo: true))")
        default: break
        }
    }
    return (parts.joined(separator: ";"), interval)
}

/// The end part of a rule: nothing, `COUNT` or `UNTIL` in one of the three forms the parser reads.
func randomEnd(_ rng: inout LCG, anchor: Anchor, span: Int) -> String? {
    switch rng.below(100) {
    case 0..<40: return nil
    case 40..<75: return "COUNT=\(rng.int(1...14))"
    default:
        let until = anchor.ms + Int(Double(span) * 1000 * (0.2 + 0.6 * Double(rng.below(100)) / 100))
        let tz = zone(anchor.zone)
        let c = civilFromSeconds(until / 1000 + tz.secondsFromGMT(for: instant(ms: until)))
        let u = civilFromSeconds(until / 1000)
        let day = String(format: "%04d%02d%02d", c.year, c.month, c.day)
        if anchor.isAllDay { return "UNTIL=\(rng.chance(80) ? day : String(format: "%04d%02d%02dT%02d%02d%02dZ", u.year, u.month, u.day, u.hour, u.minute, u.second))" }
        switch rng.below(10) {
        case 0: return "UNTIL=\(day)"
        case 1: return "UNTIL=\(String(format: "%04d%02d%02dT%02d%02d%02d", c.year, c.month, c.day, c.hour, c.minute, c.second))"
        default: return "UNTIL=\(String(format: "%04d%02d%02dT%02d%02d%02dZ", u.year, u.month, u.day, u.hour, u.minute, u.second))"
        }
    }
}

/// Weighted frequency choice (daily, weekly, monthly and yearly dominate; the sub-daily ones stay cheap).
func randomFrequency(_ rng: inout LCG) -> String {
    switch rng.below(100) {
    case 0..<3: "SECONDLY"
    case 3..<7: "MINUTELY"
    case 7..<12: "HOURLY"
    case 12..<34: "DAILY"
    case 34..<56: "WEEKLY"
    case 56..<80: "MONTHLY"
    default: "YEARLY"
    }
}

/// A complete random rule text (end included) and the window span in seconds that suits it.
func randomRuleText(_ rng: inout LCG, anchor: Anchor, freq: String? = nil) -> (text: String, freq: String, span: Int) {
    let f = freq ?? randomFrequency(&rng)
    var (text, interval) = randomRule(f, &rng, anchor: anchor)
    let units = f == "YEARLY" ? rng.int(3...12) : rng.int(3...30)
    let span = units * interval * periodSeconds(f)
    if let end = randomEnd(&rng, anchor: anchor, span: span) { text += ";" + end }
    return (text, f, span)
}
