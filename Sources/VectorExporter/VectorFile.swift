import CalendarCore
import Foundation

// The vector file envelope (conventions section 9) and the helpers every area exporter shares.
//
//   { "schema": 1, "area": "<directory>/<name>", "baseline": "1.2.0", "cases": [ { "name", "input", "expected" } ] }
//
// Instants are integer Unix milliseconds, zones are IANA identifiers, durations are whole seconds, a calendar
// day is { "year", "month", "day" }. Keys are sorted and the text is pretty printed, so two runs give identical bytes.

/// The tag of the Swift release the vectors are produced from.
let vectorBaseline = "1.2.0"

struct VectorCase {
    var name: String
    var input: [String: Any]
    var expected: [String: Any]
}

struct VectorFile {
    var path: String          // relative to the output directory, e.g. "zones/offsets.json"
    var area: String          // path without ".json"
    var cases: [VectorCase]
}

func writeVectorFile(_ file: VectorFile, to directory: URL) throws {
    var names = Set<String>()
    for c in file.cases where !names.insert(c.name).inserted { fatalError("duplicate case name \(c.name) in \(file.path)") }
    let envelope: [String: Any] = [
        "schema": 1,
        "area": file.area,
        "baseline": vectorBaseline,
        "cases": file.cases.map { ["name": $0.name, "input": $0.input, "expected": $0.expected] as [String: Any] },
    ]
    let options: JSONSerialization.WritingOptions = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
    var data = try JSONSerialization.data(withJSONObject: envelope, options: options)
    data.append(0x0A)
    let url = directory.appendingPathComponent(file.path)
    try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
    try data.write(to: url, options: .atomic)
}

// MARK: Values

func ms(_ date: Date) -> Int { Int((date.timeIntervalSince1970 * 1000).rounded()) }
func instant(ms: Int) -> Date { Date(timeIntervalSince1970: Double(ms) / 1000) }
func orNull(_ value: Any?) -> Any { value ?? NSNull() }
func orNull(_ date: Date?) -> Any { date.map { ms($0) } ?? NSNull() }

func dayJSON(_ d: CalendarDate) -> [String: Any] { ["year": d.year, "month": d.month, "day": d.day] }
func dayJSON(_ d: (year: Int, month: Int, day: Int)) -> [String: Any] { ["year": d.year, "month": d.month, "day": d.day] }

private let timeZoneCache = ZoneCache()
private final class ZoneCache: @unchecked Sendable {
    private let lock = NSLock()
    private var zones: [String: TimeZone] = [:]
    func zone(_ id: String) -> TimeZone {
        lock.lock(); defer { lock.unlock() }
        if let z = zones[id] { return z }
        guard let z = TimeZone(identifier: id) else { fatalError("this system does not know the zone \(id)") }
        zones[id] = z
        return z
    }
}
func zone(_ id: String) -> TimeZone { timeZoneCache.zone(id) }

// MARK: Civil arithmetic (proleptic Gregorian, days since 1970-01-01)

func daysFromCivil(_ y0: Int, _ m: Int, _ d: Int) -> Int {
    let y = m <= 2 ? y0 - 1 : y0
    let era = (y >= 0 ? y : y - 399) / 400
    let yoe = y - era * 400
    let doy = (153 * (m + (m > 2 ? -3 : 9)) + 2) / 5 + d - 1
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy
    return era * 146097 + doe - 719468
}

func civilFromDays(_ z0: Int) -> (year: Int, month: Int, day: Int) {
    let z = z0 + 719468
    let era = (z >= 0 ? z : z - 146096) / 146097
    let doe = z - era * 146097
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365
    let y = yoe + era * 400
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100)
    let mp = (5 * doy + 2) / 153
    let d = doy - (153 * mp + 2) / 5 + 1
    let m = mp < 10 ? mp + 3 : mp - 9
    return (m <= 2 ? y + 1 : y, m, d)
}

/// Unix milliseconds of a UTC date and time.
func utcMillis(_ y: Int, _ m: Int, _ d: Int, _ h: Int = 0, _ mi: Int = 0, _ s: Int = 0) -> Int {
    ((daysFromCivil(y, m, d) * 24 + h) * 60 + mi) * 60_000 + s * 1000
}

func floorDiv(_ a: Int, _ b: Int) -> Int { a >= 0 ? a / b : -((-a + b - 1) / b) }

/// Year, month, day, hour, minute, second of "seconds since 1970 on a wall clock".
func civilFromSeconds(_ t: Int) -> (year: Int, month: Int, day: Int, hour: Int, minute: Int, second: Int) {
    let days = floorDiv(t, 86_400)
    let rest = t - days * 86_400
    let c = civilFromDays(days)
    return (c.year, c.month, c.day, rest / 3600, (rest % 3600) / 60, rest % 60)
}

/// The last instant the zone-dependent cases may reach (Unix ms): 2026-11-01. Rules for later dates are the zone
/// database's forecast and differ between operating system releases (America/Havana's 2027 rule did between
/// macOS 26 and 27), so no case asks a zone about a later date; see vectors/README.md.
let zoneHorizon = utcMillis(2026, 11, 1)

/// The horizon for one zone. Africa/Casablanca stops on 2026-06-01: the runner's zone database predates the change
/// of Morocco's rules on 2026-09-20 that the macOS 27 database has (the first transition the two disagree on).
func horizon(of id: String) -> Int { id == "Africa/Casablanca" ? utcMillis(2026, 6, 1) : zoneHorizon }

/// FNV-1a over UTF-8: a sort key that does not depend on the other cases, so a change to one zone's rules moves
/// only that zone's cases.
func stableHash(_ text: String) -> UInt64 {
    var hash: UInt64 = 0xcbf29ce484222325
    for byte in text.utf8 { hash = (hash ^ UInt64(byte)) &* 0x100000001b3 }
    return hash
}

/// An LCG for one case or zone, derived from the area's seed and an index, so cases do not share a stream.
func derivedLCG(seed: UInt64, _ index: Int) -> LCG { LCG(state: seed &+ UInt64(index + 1) &* 0x9E37_79B9_7F4A_7C15) }

/// The name part of a case: a zone id made safe for a name.
func slug(_ text: String) -> String {
    String(text.map { $0.isLetter || $0.isNumber || $0 == "-" ? $0 : "_" })
}

func fourDigits(_ n: Int) -> String { String(format: "%04d", n) }

/// The instants (Unix ms, whole seconds) at which `tz`'s offset from UTC changes between `from` and `to`: the first
/// second of each new offset. Found by scanning every six hours and bisecting, not with nextDaylightSavingTimeTransition,
/// because that method's answer depends on how the zone database flags daylight time (Africa/Casablanca's transitions
/// differed between macOS 26 and 27 although the offsets agreed) and it skips changes of the standard offset.
func dstTransitions(_ tz: TimeZone, from: Int, to: Int) -> [Int] {
    func offset(_ t: Int) -> Int { tz.secondsFromGMT(for: instant(ms: t)) }
    var found: [Int] = []
    var t = from
    var current = offset(t)
    while t < to {
        let next = min(t + 6 * 3_600_000, to)
        let after = offset(next)
        if after != current {
            var low = t, high = next            // offset(low) == current, offset(high) != current
            while high - low > 1000 {
                let mid = low + (high - low) / 2000 * 1000
                if offset(mid) == current { low = mid } else { high = mid }
            }
            found.append(high)
        }
        current = after
        t = next
    }
    return found
}
