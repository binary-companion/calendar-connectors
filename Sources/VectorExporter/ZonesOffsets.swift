import Foundation

// zones/offsets.json
//
// Entry point: TimeZone.secondsFromGMT(for: Date) -> Int
//
//   input:    { "zone": "<IANA id>", "instant": <Unix ms> }
//   expected: { "offsetSeconds": <Int> }
//
// Sixty zones, at least 40 instants each: the second before, the instant of and the second after every daylight
// saving transition from 2024 up to 2026-11-01 (found by scanning the offsets, see dstTransitions), then a fixed grid of
// quarter days and seeded random instants between 1990 and 2026 up to 40 instants. Each zone has its own
// generator (derivedLCG) and nothing is later than 2026-11-01 (zoneHorizon).

func exportZonesOffsets() -> [VectorCase] {
    var cases: [VectorCase] = []
    let from = utcMillis(2024, 1, 1)
    for (zoneIndex, id) in vectorZones.enumerated() {
        var rng = derivedLCG(seed: 0x0FF5_E7C0_DE24, zoneIndex)
        let tz = zone(id)
        let to = horizon(of: id)
        var instants: [Int] = []
        var seen = Set<Int>()
        func add(_ t: Int) { if t <= to + 1000, seen.insert(t).inserted { instants.append(t) } }
        for t in dstTransitions(tz, from: from, to: to) { add(t - 1000); add(t); add(t + 1000) }
        for (y, m) in [(2024, 1), (2024, 4), (2024, 7), (2024, 10), (2026, 1), (2026, 7)] { add(utcMillis(y, m, 1, 12)) }
        add(0); add(utcMillis(2000, 2, 29, 12)); add(utcMillis(2026, 10, 31, 23, 59, 59))
        while instants.count < 40 { add(utcMillis(1990, 1, 1) + rng.below(35 * 365 * 86_400) * 1000) }
        for (i, t) in instants.enumerated() {
            cases.append(VectorCase(
                name: "offsets-\(slug(id))-\(fourDigits(i + 1))",
                input: ["zone": id, "instant": t],
                expected: ["offsetSeconds": tz.secondsFromGMT(for: instant(ms: t))]))
        }
    }
    return cases
}
