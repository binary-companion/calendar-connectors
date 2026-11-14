import CalendarCore
import Foundation
@testable import ICalendar

nonisolated(unsafe) var rng = LCG(state: 0x9B0BE_0001)
func esc(_ s: String) -> String { s }
func scalars(_ s: String) -> [UInt32] { s.unicodeScalars.map { $0.value } }

var out: [String: Any] = [:]

let pool = ["a", "b", "n", "N", ";", ":", ",", "=", "\"", "\\", " ", "\t", "\n", "\r", "\r\n", "\u{0301}", "\u{0300}", "\u{200D}", "\u{1F468}", "\u{1F469}",
            "\u{1F3FB}", "\u{1F1E6}", "\u{1F1E7}", "\u{1F1E8}", "\u{AC00}", "\u{11A8}", "\u{1100}", "\u{1161}", "\u{0600}", "\u{0903}", "\u{037E}", "\u{FE0F}",
            "\u{00E9}", "e\u{0301}", "\u{2028}", "\u{0085}", "\u{0915}\u{094D}\u{0937}", "\u{1F642}", "\u{200C}", "\u{00A9}", "\u{FEFF}", "A", "T", "Z", "P", "1", "9"]
func randomString(_ maxLen: Int) -> String {
    var s = ""
    for _ in 0..<rng.int(0...maxLen) { s += rng.pick(pool) }
    return s
}

// graphemes, escape, unescape
var g: [[String: Any]] = []
for _ in 0..<600 {
    let s = randomString(12)
    g.append(["s": scalars(s), "clusters": Array(s).map { scalars(String($0)) },
              "escape": scalars(ICalText.escape(s)), "unescape": scalars(ICalText.unescape(s)),
              "prefix3": scalars(String(s.prefix(3))), "dropFirst2": scalars(String(s.dropFirst(2)))])
}
out["graphemes"] = g

// unfold, parseLine, serializer
var lines: [[String: Any]] = []
let linePool = ["SUMMARY", "A", "X-B", ";", ":", "=", ",", "\"", "P", "V", "1", " ", "\u{0301}", "\u{037E}", "\u{00E9}", "\u{1F642}", "x", "\\", "\n", "\r\n", "\r", "\t"]
func randomLine() -> String { var s = ""; for _ in 0..<rng.int(1...14) { s += rng.pick(linePool) }; return s }
for _ in 0..<800 {
    let s = randomLine()
    let unf = ICalParser.unfold(s).map { scalars($0) }
    var parsed: [String: Any]
    do {
        let p = try ICalParser.parseLine(s)
        parsed = ["ok": true, "name": scalars(p.name), "parameters": p.parameters.map { ["name": scalars($0.name), "values": $0.values.map { scalars($0) }] as [String: Any] }, "value": scalars(p.value)]
    } catch ICalError.malformed(let m) { parsed = ["ok": false, "message": scalars(m)] } catch { parsed = ["ok": false] }
    lines.append(["s": scalars(s), "unfold": unf, "parse": parsed])
}
out["lines"] = lines

// fold: serialize a property with a long value
var folds: [[String: Any]] = []
for _ in 0..<60 {
    var v = ""
    for _ in 0..<rng.int(1...80) { v += rng.pick(["a", "é", "🙂", "中", ";", ",", ":", "\\", "e\u{0301}", " "]) }
    let p = ICalProperty(name: "x-name", parameters: [ICalParameter(name: "cn", values: [String(v.prefix(5)), "q;r"])], value: v)
    folds.append(["name": "x-name", "value": scalars(v), "serialized": scalars(ICalSerializer.serialize(ICalComponent(name: "vevent", properties: [p])))])
}
out["folds"] = folds

// durations and date values
var durs: [[String: Any]] = []
let durPool = ["P", "T", "W", "D", "H", "M", "S", "0", "1", "2", "15", "-", "+", " ", "\u{0301}", "p", "t", "m", "10", "99999999999999999999", "3"]
for _ in 0..<600 {
    var s = ""; for _ in 0..<rng.int(1...8) { s += rng.pick(durPool) }
    durs.append(["s": scalars(s), "d": ICalValues.duration(s).map { String($0.bitPattern) } ?? NSNull()])
}
for s in ["PT15M", "-PT15M", "P1DT2H3M4S", "P2W", "+P1D", "PT", "P", "-P1W2D", "PT1H30M", "P1Y"] { durs.append(["s": scalars(s), "d": ICalValues.duration(s).map { String($0.bitPattern) } ?? NSNull()]) }
out["durations"] = durs
var dts: [[String: Any]] = []
let dtPool = ["2026", "0927", "T", "150000", "Z", "09", "27", "15", "00", "60", "99", "1", "0", "-", ":", " ", "\u{0301}", "z", "t", "2", "13", "32", "30", "٣"]
for _ in 0..<800 {
    var s = ""; for _ in 0..<rng.int(1...7) { s += rng.pick(dtPool) }
    if rng.chance(50) { s = ["20260927T150000Z", "20260927T150000", "20260927", "20260230", "20260230T253000Z", "20261301", "20260927T150060Z"][rng.below(7)] + (rng.chance(30) ? rng.pick(["\u{0301}", " ", "Z", ",20260101"]) : "") }
    let kind = rng.pick([nil, "DATE", "PERIOD", "date"] as [String?])
    let tzid = rng.chance(30) ? "Europe/Berlin" : nil
    let p = ICalProperty(name: "DTSTART", parameters: (kind.map { [ICalParameter("VALUE", $0)] } ?? []) + (tzid.map { [ICalParameter("TZID", $0)] } ?? []), value: s)
    let vs = ICalValues.dateValues(p)
    func desc(_ v: ICalDateValue) -> String {
        switch v {
        case .date(let d): return "date \(d.year)-\(d.month)-\(d.day)"
        case .utc(let t): return "utc \(t.timeIntervalSince1970)"
        case .local(let l, let z): return "local \(l.date.year)-\(l.date.month)-\(l.date.day) \(l.hour):\(l.minute):\(l.second) \(z ?? "-")"
        }
    }
    dts.append(["s": scalars(s), "kind": kind ?? NSNull(), "tzid": tzid ?? NSNull(), "values": vs.map(desc)])
}
out["dates"] = dts

// UTF-8 decoding
var dec: [[String: Any]] = []
let bytePool: [UInt8] = [0x41, 0x0A, 0x0D, 0x20, 0xC3, 0xA9, 0xE2, 0x82, 0xAC, 0xF0, 0x9F, 0x99, 0x82, 0xFF, 0xFE, 0x80, 0xC0, 0xC1, 0xED, 0xA0, 0xF4, 0x90, 0xE0, 0x9F, 0xF8]
for _ in 0..<500 {
    var b: [UInt8] = []; for _ in 0..<rng.int(1...10) { b.append(rng.pick(bytePool)) }
    dec.append(["bytes": b.map { Int($0) }, "scalars": scalars(String(decoding: b, as: UTF8.self))])
}
out["decode"] = dec

// Double parsing
var dbl: [[String: Any]] = []
for s in ["37.33", "-122.03", "+5", "5.", ".5", "1e3", "1E3", "inf", "-inf", "Infinity", "nan", "NaN", "0x10", "0x1p3", " 1", "1 ", "", "-", "+", "1_0", "1,5", "1e", "e5", ".", "1e400",
          "-1e400", "1e-400", "infinit", "INF", "0x", "0x.8", "0x1.8p1", "0X1P-1", "0xAbC", "++1", "--1", "+-1", "1e+3", "1e-3", "1.5e", "0x1p", "0x1p+", "nan(1)", "-nan", "+inf", "0b1", "007", "1.", "-.5", "+.5e1", "٣", "1\u{0301}"] {
    dbl.append(["s": scalars(s), "d": Double(s).map { $0.isNaN ? "nan" : String($0.bitPattern) } ?? NSNull()])
}
out["doubles"] = dbl

// VTIMEZONE text
let zoneIds = ["America/New_York", "America/Los_Angeles", "America/Phoenix", "America/Denver", "America/Chicago", "America/Toronto", "America/Halifax", "America/St_Johns", "America/Anchorage", "America/Adak", "Pacific/Honolulu",
               "Europe/Berlin", "Europe/London", "Europe/Dublin", "Europe/Moscow", "Europe/Istanbul", "Africa/Casablanca", "Africa/Cairo", "Africa/Lagos", "Asia/Tokyo", "Asia/Kolkata", "Asia/Calcutta", "Asia/Kathmandu", "Asia/Tehran",
               "Australia/Sydney", "Australia/Lord_Howe", "Australia/Adelaide", "Pacific/Auckland", "Pacific/Chatham", "Pacific/Apia", "America/Sao_Paulo", "America/Santiago", "America/Havana", "Asia/Jerusalem", "Antarctica/Troll",
               "GMT+0100", "GMT-0530", "Etc/GMT+5", "Asia/Shanghai", "America/Panama", "Asia/Manila", "America/Mazatlan", "Europe/Lisbon", "Atlantic/Azores", "America/Argentina/Buenos_Aires", "Asia/Dhaka"]
var vtz: [[String: Any]] = []
let ranges: [(Double, Double)] = [(1_780_000_000, 1_800_000_000), (1_262_304_000, 1_330_000_000), (1_790_000_000, 1_791_000_000)]
for id in zoneIds {
    guard let z = TimeZone(identifier: id) else { continue }
    for (a, b) in ranges {
        let c = VTimeZoneWriter.component(for: z, from: Date(timeIntervalSince1970: a), through: Date(timeIntervalSince1970: b))
        vtz.append(["zone": id, "from": a, "through": b, "text": c.map { scalars(ICalSerializer.serialize($0)) } ?? NSNull()])
    }
}
out["vtz"] = vtz

// Resolver on random VTIMEZONE definitions
var res: [[String: Any]] = []
let offsets = ["-0800", "-0700", "-0600", "-0500", "-0400", "-0330", "-0300", "+0000", "+0100", "+0200", "+0300", "+0330", "+0530", "+0545", "+0800", "+0900", "+0930", "+1000", "+1030", "+1100", "+1200", "+1245", "+013045", "-0337", "+1400", "bad", "+99"]
for i in 0..<250 {
    let std = rng.pick(offsets)
    let dst: String? = rng.chance(60) ? rng.pick(offsets) : nil
    let year = rng.int(1995...2030)
    let dMonth = rng.int(1...12), sMonth = rng.int(1...12)
    let loc = rng.chance(10) ? rng.pick(["Europe/Berlin", "Mars/Olympus"]) : nil
    var props = ["BEGIN:VTIMEZONE", "TZID:Z\(i)"]
    if let loc { props.append("X-LIC-LOCATION:\(loc)") }
    if let dst {
        props += ["BEGIN:DAYLIGHT", "DTSTART:\(year)0\(rng.int(1...9))15T020000", "TZOFFSETFROM:\(std)", "TZOFFSETTO:\(dst)"]
        if rng.chance(70) { props.append("RRULE:FREQ=YEARLY;BYMONTH=\(dMonth);BYDAY=2SU") }
        props.append("END:DAYLIGHT")
    }
    props += ["BEGIN:STANDARD", "DTSTART:\(rng.chance(80) ? "\(year)1104T020000" : "bad")", "TZOFFSETFROM:\(dst ?? std)", "TZOFFSETTO:\(std)"]
    if dst != nil && rng.chance(70) { props.append("RRULE:FREQ=YEARLY;BYMONTH=\(sMonth);BYDAY=1SU") }
    props += ["END:STANDARD", "END:VTIMEZONE"]
    let text = (["BEGIN:VCALENDAR"] + props + ["END:VCALENDAR", ""]).joined(separator: "\r\n")
    let r = TimeZoneResolver(calendar: try! ICalParser.parse(text))
    res.append(["calendar": text, "tzid": "Z\(i)", "zone": r.zone(for: "Z\(i)")?.identifier ?? NSNull()])
}
out["resolver"] = res

// transitions and abbreviations
var trans: [[String: Any]] = []
for id in zoneIds {
    guard let z = TimeZone(identifier: id) else { continue }
    for after in [1_262_304_000.0, 1_700_000_000.0, 1_780_000_000.0, 2_100_000_000.0] {
        var c = Date(timeIntervalSince1970: after)
        var list: [[String: Any]] = []
        for _ in 0..<3 {
            guard let n = z.nextDaylightSavingTimeTransition(after: c) else { break }
            list.append(["t": n.timeIntervalSince1970, "dst": z.isDaylightSavingTime(for: n), "ab": z.abbreviation(for: n) ?? "nil", "from": z.secondsFromGMT(for: n.addingTimeInterval(-1)), "to": z.secondsFromGMT(for: n)])
            c = n
        }
        trans.append(["zone": id, "after": after, "list": list])
    }
}
out["transitions"] = trans

let data = try JSONSerialization.data(withJSONObject: out, options: [.sortedKeys])
try data.write(to: URL(fileURLWithPath: CommandLine.arguments[1]))
