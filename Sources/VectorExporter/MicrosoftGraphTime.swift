import CalendarCore
import Foundation
@testable import MicrosoftCalendar

// microsoft/graph-time.json
//
// Entry points (all internal to MicrosoftCalendar, read with @testable): GraphTime.parse(_:in:), .date(_:), .format(_:in:),
// .midnight(_:), .dateText(_:), .parseInstant(_:), .instantText(_:). A case has an "fn"; D is { "year", "month", "day" }.
//
//   fn "parse":        input { "fn", "text", "zone" }        expected { "instant": <ms> | null }
//   fn "date":         input { "fn", "text" }                expected { "date": D | null }
//   fn "format":       input { "fn", "instant": <ms>, "zone" }  expected { "text" }
//   fn "midnight":     input { "fn", "date": D }             expected { "text" }
//   fn "dateText":     input { "fn", "date": D }             expected { "text" }
//   fn "parseInstant": input { "fn", "text" }                expected { "instant": <ms> | null }
//   fn "instantText":  input { "fn", "instant": <ms> }       expected { "text" }
//
// 300 cases: 60 parse, 40 date, 50 format, 30 midnight, 30 dateText, 60 parseInstant, 30 instantText. The texts
// include Graph's seven-digit fractions, a space instead of the T, offsets (Z, +hh:mm, -hh:mm), impossible dates that must
// not roll over, times in DST gaps and overlaps, short text and non-digit text.

func exportMicrosoftGraphTime() -> [VectorCase] {
    var rng = LCG(state: 0x6A4F_71E0_0006)
    var cases: [VectorCase] = []
    func add(_ fn: String, _ input: [String: Any], _ expected: [String: Any]) {
        var input = input
        input["fn"] = fn
        cases.append(VectorCase(name: "graph-time-\(fn)-\(fourDigits(cases.count + 1))", input: input, expected: expected))
    }
    func parse(_ text: String, _ id: String) { add("parse", ["text": text, "zone": id], ["instant": orNull(GraphTime.parse(text, in: zone(id)))]) }
    func date(_ text: String) { add("date", ["text": text], ["date": GraphTime.date(text).map { dayJSON($0) } ?? NSNull()]) }
    func format(_ t: Int, _ id: String) { add("format", ["instant": t, "zone": id], ["text": GraphTime.format(instant(ms: t), in: zone(id))]) }
    func parseInstant(_ text: String) { add("parseInstant", ["text": text], ["instant": orNull(GraphTime.parseInstant(text))]) }
    func randomDay() -> CalendarDate { CalendarDate(year: rng.int(1990...2025), month: rng.int(1...12), day: rng.int(1...28)) }
    func two(_ n: Int) -> String { String(format: "%02d", n) }
    func randomLocal(fraction: Bool = true) -> String {
        let d = randomDay()
        var text = "\(fourDigits(d.year))-\(two(d.month))-\(two(d.day))\(rng.chance(90) ? "T" : " ")\(two(rng.int(0...23))):\(two(rng.int(0...59))):\(two(rng.int(0...59)))"
        if fraction && rng.chance(60) { text += "." + String(repeating: "0", count: 7) }
        else if fraction && rng.chance(30) { text += ".\(rng.int(1...9999999))" }
        return text
    }

    // parse.
    for text in ["2026-09-25T10:00:00.0000000", "2026-09-25T10:00:00", "2026-13-25T10:00:00", "2026-02-30T10:00:00", "nonsense", "", "2026-09-25",
                 "2026-09-25T10:00", "2026-09-25 10:00:00", "2026-09-25X10:00:00", "2026/09/25T10:00:00", "2026-09-25T25:00:00", "2026-09-25T10:60:00",
                 "2026-09-25T10:00:60", "2024-02-29T12:00:00", "2023-02-29T12:00:00", "2026-04-31T12:00:00", "2026-00-10T10:00:00", "2026-01-00T10:00:00",
                 "2026-09-25T10:00:00Z", "2026-09-25T10:00:00+02:00", "20Z6-09-25T10:00:00", "2026-09-25T1\u{0660}:00:00", "\u{0662}026-09-25T10:00:00"] {
        parse(text, "America/Los_Angeles")
    }
    for (id, text) in [("America/New_York", "2026-03-08T02:30:00"), ("America/New_York", "2026-03-08T02:00:00"), ("America/New_York", "2026-11-01T01:30:00"),
                       ("Europe/Berlin", "2026-03-29T02:30:00"), ("Europe/Berlin", "2026-10-25T02:30:00"), ("Australia/Sydney", "2026-10-04T02:30:00"),
                       ("Australia/Sydney", "2026-04-05T02:30:00"), ("Australia/Lord_Howe", "2026-10-04T02:15:00"), ("America/Havana", "2026-03-08T00:30:00"),
                       ("Pacific/Apia", "2011-12-30T12:00:00"), ("Asia/Kolkata", "2026-09-25T10:00:00"), ("UTC", "2026-09-25T10:00:00.5")] { parse(text, id) }
    while cases.count < 60 { parse(randomLocal(), rng.pick(rulesZones)) }

    // date.
    for text in ["2026-09-25T00:00:00.0000000", "2026-09-25", "2026-09-25T10:00:00", "2026-9-5", "2026-09-5T00", "2026-13-45", "20260925", "", "2026-09",
                 "abc", "2026-09-25garbage", "0000-00-00", "-2026-09-25", "2026--09-25", "2026-09-25\u{0660}", "26-09-25T00:00:00", "2026-02-30T00:00:00"] { date(text) }
    while cases.count < 60 + 40 {
        let d = randomDay()
        date("\(fourDigits(d.year))-\(two(d.month))-\(two(d.day))" + rng.pick(["", "T00:00:00", "T00:00:00.0000000", "T10:30:00Z", " junk"]))
    }

    // format.
    for (t, id) in [(1_790_355_600_000, "America/Los_Angeles"), (0, "UTC"), (utcMillis(2026, 3, 8, 7), "America/New_York"), (utcMillis(2026, 3, 8, 6, 59, 59), "America/New_York"),
                    (utcMillis(2026, 11, 1, 5, 30), "America/New_York"), (utcMillis(2026, 11, 1, 6, 30), "America/New_York"), (utcMillis(2024, 2, 29, 23, 59, 59), "Asia/Kolkata"),
                    (utcMillis(1999, 12, 31, 23, 59, 59), "Pacific/Kiritimati"), (utcMillis(2011, 12, 30, 10), "Pacific/Apia"), (utcMillis(9999, 12, 31, 23), "UTC")] { format(t, id) }
    while cases.count < 60 + 40 + 50 { format(utcMillis(1990, 1, 1) + rng.below(35 * 365 * 86_400) * 1000, rng.pick(vectorZones)) }

    // midnight and dateText.
    for d in [CalendarDate(year: 2026, month: 9, day: 5), CalendarDate(year: 2026, month: 12, day: 31), CalendarDate(year: 2024, month: 2, day: 29),
              CalendarDate(year: 1, month: 1, day: 1), CalendarDate(year: 9999, month: 12, day: 31), CalendarDate(year: 12345, month: 1, day: 1),
              CalendarDate(year: 2026, month: 13, day: 45), CalendarDate(year: 2026, month: 0, day: 0)] {
        add("midnight", ["date": dayJSON(d)], ["text": GraphTime.midnight(d)])
    }
    while cases.count < 60 + 40 + 50 + 30 { let d = randomDay(); add("midnight", ["date": dayJSON(d)], ["text": GraphTime.midnight(d)]) }
    for d in [CalendarDate(year: 2026, month: 9, day: 5), CalendarDate(year: 2026, month: 12, day: 31), CalendarDate(year: 2024, month: 2, day: 29),
              CalendarDate(year: 1, month: 1, day: 1), CalendarDate(year: 9999, month: 12, day: 31), CalendarDate(year: 12345, month: 1, day: 1),
              CalendarDate(year: 2026, month: 13, day: 45), CalendarDate(year: 2026, month: 0, day: 0)] {
        add("dateText", ["date": dayJSON(d)], ["text": GraphTime.dateText(d)])
    }
    while cases.count < 60 + 40 + 50 + 30 + 30 { let d = randomDay(); add("dateText", ["date": dayJSON(d)], ["text": GraphTime.dateText(d)]) }

    // parseInstant.
    for text in ["2026-09-25T17:00:00Z", "2026-09-25T17:00:00z", "2026-09-25T17:00:00.1234567Z", "2026-09-25T10:00:00-07:00", "2026-09-25T22:30:00+05:30",
                 "2026-09-25T17:00:00.0000000", "garbage", "", "Z", "2026-09-25T17:00:00+0530", "2026-09-25T17:00:00+05", "2026-09-25T17:00:00+05:3",
                 "2026-09-25T17:00:00 +05:30", "2026-09-25T17:00:00-00:00", "2026-09-25T17:00:00+14:00", "2026-09-25T17:00:00-12:00", "2026-09-25T17:00:00+99:99",
                 "2026-09-25T17:00:00.5-03:30", "2026-09-25+05:30", "2026-09-25", "2026-09-25T00:00:00Z", "2026-02-30T00:00:00Z", "2026-09-25T17:00:00ZZ",
                 "2026-09-25T17:00:00+aa:bb", "2026-09-25T17:00:00+05:30:00"] { parseInstant(text) }
    while cases.count < 60 + 40 + 50 + 30 + 30 + 60 {
        let local = randomLocal(fraction: true)
        let suffix = rng.pick(["Z", "Z", "z", "", "+05:30", "-07:00", "+00:00", "-03:30", "+14:00", "+12:45"])
        parseInstant(local + suffix)
    }

    // instantText.
    for t in [1_790_355_600_000, 0, utcMillis(2024, 2, 29, 23, 59, 59), utcMillis(1999, 12, 31, 23, 59, 59), -86_400_000, utcMillis(9999, 12, 31, 23, 59, 59)] {
        add("instantText", ["instant": t], ["text": GraphTime.instantText(instant(ms: t))])
    }
    while cases.count < 300 {
        let t = utcMillis(1970, 1, 1) + rng.below(70 * 365 * 86_400) * 1000
        add("instantText", ["instant": t], ["text": GraphTime.instantText(instant(ms: t))])
    }
    return cases
}
