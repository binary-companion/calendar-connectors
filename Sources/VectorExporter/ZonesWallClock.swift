import CalendarCore
import Foundation

// zones/wall-clock.json
//
// Entry point: WallClock.date(_ day: CalendarDate, hour:minute:second:in: TimeZone) -> Date?
//
//   input:    { "zone": "<IANA id>", "date": { "year", "month", "day" }, "hour": 0...23, "minute": 0...59, "second": 0...59 }
//   expected: { "instant": <Unix ms> | null }
//
// The name of a case says what the wall time is in its zone: "gap" (skipped by a daylight saving jump, resolved
// forward by the gap), "overlap" (happens twice, resolved to the first) or "ordinary". Gap and overlap cases sit
// within an hour and a half of every transition from 2024 through 2027; the ordinary cases are seeded random
// times between 2000 and 2035. Every input is a real calendar date, so null does not occur.

func exportZonesWallClock() -> [VectorCase] {
    var rng = LCG(state: 0x3A11_C10C_4B)
    let from = utcMillis(2024, 1, 1), to = utcMillis(2028, 1, 1)
    let deltas = [-5400, -3600, -1800, -900, -60, -1, 0, 1, 60, 900, 1800, 3599, 3600, 5400]
    var special: [(String, Int)] = []      // zone, wall seconds
    for id in vectorZones {
        let tz = zone(id)
        for t in dstTransitions(tz, from: from, to: to) {
            let before = tz.secondsFromGMT(for: instant(ms: t - 1000))
            for d in deltas { special.append((id, t / 1000 + before + d)) }
        }
    }
    func make(_ id: String, _ wall: Int, _ index: Int) -> VectorCase {
        let c = civilFromSeconds(wall)
        let day = CalendarDate(year: c.year, month: c.month, day: c.day)
        let result = WallClock.date(day, hour: c.hour, minute: c.minute, second: c.second, in: zone(id))
        return VectorCase(
            name: "wall-clock-\(classify(id, wall))-\(slug(id))-\(fourDigits(index))",
            input: ["zone": id, "date": dayJSON(day), "hour": c.hour, "minute": c.minute, "second": c.second],
            expected: ["instant": orNull(result)])
    }
    var cases: [VectorCase] = []
    var shuffled = special
    for i in stride(from: shuffled.count - 1, to: 0, by: -1) { shuffled.swapAt(i, rng.below(i + 1)) }
    var gaps = 0, overlaps = 0
    var taken = Set<String>()
    for (id, wall) in shuffled where cases.count < 330 {
        let kind = classify(id, wall)
        guard kind != "ordinary", taken.insert("\(id)|\(wall)").inserted else { continue }
        if kind == "gap" { gaps += 1 } else { overlaps += 1 }
        cases.append(make(id, wall, cases.count + 1))
    }
    // Then the ordinary times around the same transitions.
    for (id, wall) in shuffled where cases.count < 400 {
        guard classify(id, wall) == "ordinary", taken.insert("\(id)|\(wall)").inserted else { continue }
        cases.append(make(id, wall, cases.count + 1))
    }
    while cases.count < 600 {
        let id = rng.pick(vectorZones)
        let wall = utcMillis(2000, 1, 1) / 1000 + rng.below(35 * 365 * 86_400)
        guard taken.insert("\(id)|\(wall)").inserted else { continue }
        cases.append(make(id, wall, cases.count + 1))
    }
    precondition(gaps + overlaps >= 150, "fewer than 150 gap and overlap cases")
    return cases
}

/// "gap", "overlap" or "ordinary" for a wall time (seconds, read as if the zone were UTC), by the same two
/// offsets WallClock.date compares: the offsets in force a day before and a day after noon.
private func classify(_ id: String, _ wall: Int) -> String {
    let tz = zone(id)
    let c = civilFromSeconds(wall)
    let noon = utcMillis(c.year, c.month, c.day, 12)
    let before = tz.secondsFromGMT(for: instant(ms: noon - 86_400_000))
    let after = tz.secondsFromGMT(for: instant(ms: noon + 86_400_000))
    let early = wall - before, late = wall - after
    let earlyOK = tz.secondsFromGMT(for: instant(ms: early * 1000)) == before
    let lateOK = tz.secondsFromGMT(for: instant(ms: late * 1000)) == after
    switch (earlyOK, lateOK) {
    case (true, true): return early == late ? "ordinary" : "overlap"
    case (false, false): return "gap"
    default: return "ordinary"
    }
}
