import CalendarCore
import Foundation

// zones/all-day.json
//
// Entry points: AllDay.startOfDay, AllDay.date(of:in:), AllDay.canonical(first:endExclusive:in:),
// AllDay.dates(start:end:in:) and CalendarDate.adding(days:). Every case has an "fn" saying which one.
//
//   fn "startOfDay": input { "fn", "zone", "date": D }                         expected { "instant": <ms> | null }
//   fn "dateOf":     input { "fn", "zone", "instant": <ms> }                   expected { "date": D }
//   fn "canonical":  input { "fn", "zone", "first": D, "endExclusive": D }     expected { "range": { "start": <ms>, "end": <ms> } | null }
//   fn "dates":      input { "fn", "zone", "start": <ms>, "end": <ms> }        expected { "first": D, "endExclusive": D }
//   fn "adding":     input { "fn", "date": D, "days": <Int> }                  expected { "date": D }
//
// where D is { "year", "month", "day" }. 500 cases: 150 startOfDay, 100 dateOf, 100 canonical, 70 dates, 80 adding.
// The startOfDay and canonical cases include days whose midnight does not exist (every transition from 2024 to
// 2027 in a zone that moves at midnight) and the days the date line moved (Apia and Fakaofo, December 2011;
// Kiritimati, December 1994).

func exportZonesAllDay() -> [VectorCase] {
    var rng = LCG(state: 0xA11_DA7_5EED)
    var cases: [VectorCase] = []
    func add(_ kind: String, _ input: [String: Any], _ expected: [String: Any]) {
        var input = input
        input["fn"] = kind
        cases.append(VectorCase(name: "all-day-\(kind)-\(fourDigits(cases.count + 1))", input: input, expected: expected))
    }
    func randomDay() -> CalendarDate {
        let c = civilFromDays(daysFromCivil(2000, 1, 1) + rng.below(36 * 365))
        return CalendarDate(year: c.year, month: c.month, day: c.day)
    }
    func dayOf(_ ms: Int, _ id: String) -> CalendarDate {
        let seconds = ms / 1000 + zone(id).secondsFromGMT(for: instant(ms: ms))
        let c = civilFromSeconds(seconds)
        return CalendarDate(year: c.year, month: c.month, day: c.day)
    }
    func startOfDay(_ id: String, _ day: CalendarDate) {
        add("startOfDay", ["zone": id, "date": dayJSON(day)], ["instant": orNull(AllDay.startOfDay(day, in: zone(id)))])
    }
    // Days around transitions, in the zones that have them.
    var transitionDays: [(String, CalendarDate)] = []
    for id in vectorZones {
        let tz = zone(id)
        for t in dstTransitions(tz, from: utcMillis(2024, 1, 1), to: utcMillis(2028, 1, 1)) {
            transitionDays.append((id, dayOf(t, id)))
        }
    }
    // startOfDay: hand picked first.
    startOfDay("Pacific/Apia", CalendarDate(year: 2011, month: 12, day: 29))
    startOfDay("Pacific/Apia", CalendarDate(year: 2011, month: 12, day: 30))
    startOfDay("Pacific/Apia", CalendarDate(year: 2011, month: 12, day: 31))
    startOfDay("Pacific/Fakaofo", CalendarDate(year: 2011, month: 12, day: 30))
    startOfDay("Pacific/Kiritimati", CalendarDate(year: 1994, month: 12, day: 30))
    startOfDay("Pacific/Kiritimati", CalendarDate(year: 1995, month: 1, day: 1))
    for id in ["America/New_York", "Asia/Tokyo", "UTC", "Pacific/Auckland"] {
        for day in [CalendarDate(year: 2024, month: 2, day: 29), CalendarDate(year: 2026, month: 12, day: 31),
                    CalendarDate(year: 2027, month: 1, day: 1), CalendarDate(year: 2100, month: 2, day: 28)] { startOfDay(id, day) }
    }
    var shuffled = transitionDays
    for i in stride(from: shuffled.count - 1, to: 0, by: -1) { shuffled.swapAt(i, rng.below(i + 1)) }
    for (id, day) in shuffled.prefix(70) { startOfDay(id, day) }
    while cases.count < 150 { startOfDay(rng.pick(vectorZones), randomDay()) }

    // dateOf: instants around transitions, midnights, and random ones.
    for id in ["Pacific/Apia", "Pacific/Kiritimati", "Asia/Kathmandu", "Australia/Lord_Howe"] {
        for t in [utcMillis(2011, 12, 30, 9, 59, 59), utcMillis(2011, 12, 30, 10), utcMillis(1994, 12, 30, 9, 59, 59), utcMillis(1994, 12, 31, 10), 0] {
            add("dateOf", ["zone": id, "instant": t], ["date": dayJSON(AllDay.date(of: instant(ms: t), in: zone(id)))])
        }
    }
    var count = 0
    for (id, day) in shuffled.suffix(40) {
        let t = utcMillis(day.year, day.month, day.day, 12) - 86_400_000 + rng.below(172_800) * 1000
        add("dateOf", ["zone": id, "instant": t], ["date": dayJSON(AllDay.date(of: instant(ms: t), in: zone(id)))])
        count += 1
    }
    while cases.count < 150 + 100 {
        let id = rng.pick(vectorZones)
        let t = utcMillis(2000, 1, 1) + rng.below(36 * 365 * 86_400) * 1000
        add("dateOf", ["zone": id, "instant": t], ["date": dayJSON(AllDay.date(of: instant(ms: t), in: zone(id)))])
    }

    // canonical: one to ten days, mostly random; some across a transition.
    func canonical(_ id: String, _ first: CalendarDate, _ length: Int) {
        let end = first.adding(days: length)
        let range = AllDay.canonical(first: first, endExclusive: end, in: zone(id))
        add("canonical", ["zone": id, "first": dayJSON(first), "endExclusive": dayJSON(end)],
            ["range": range.map { ["start": ms($0.start), "end": ms($0.end)] as [String: Any] } ?? NSNull()])
    }
    canonical("America/New_York", CalendarDate(year: 2024, month: 3, day: 9), 3)
    canonical("America/New_York", CalendarDate(year: 2024, month: 11, day: 2), 3)
    canonical("Europe/Berlin", CalendarDate(year: 2024, month: 12, day: 31), 1)
    canonical("Pacific/Apia", CalendarDate(year: 2011, month: 12, day: 29), 3)
    for (id, day) in shuffled.prefix(35) { canonical(id, day, rng.int(1...5)) }
    while cases.count < 150 + 100 + 100 { canonical(rng.pick(vectorZones), randomDay(), rng.int(1...10)) }

    // dates: the inverse, from canonical instants and from arbitrary instants.
    while cases.count < 150 + 100 + 100 + 70 {
        let id = rng.pick(vectorZones)
        let day = randomDay()
        var start = AllDay.startOfDay(day, in: zone(id)) ?? instant(ms: 0)
        var end = AllDay.startOfDay(day.adding(days: rng.int(1...6)), in: zone(id)) ?? instant(ms: 0)
        if rng.chance(30) {
            start = instant(ms: ms(start) + rng.below(86_400) * 1000)
            end = instant(ms: ms(end) + rng.below(86_400) * 1000)
        }
        let result = AllDay.dates(start: start, end: end, in: zone(id))
        add("dates", ["zone": id, "start": ms(start), "end": ms(end)],
            ["first": dayJSON(result.first), "endExclusive": dayJSON(result.endExclusive)])
    }

    // adding(days:): month and year ends, leap days, negative and large steps.
    let hand: [(CalendarDate, Int)] = [
        (CalendarDate(year: 2024, month: 2, day: 28), 1), (CalendarDate(year: 2024, month: 2, day: 29), 1),
        (CalendarDate(year: 2023, month: 2, day: 28), 1), (CalendarDate(year: 2024, month: 12, day: 31), 1),
        (CalendarDate(year: 2024, month: 1, day: 1), -1), (CalendarDate(year: 2024, month: 3, day: 1), -1),
        (CalendarDate(year: 2024, month: 1, day: 31), 0), (CalendarDate(year: 2024, month: 1, day: 1), 366),
        (CalendarDate(year: 2024, month: 1, day: 1), 365), (CalendarDate(year: 1999, month: 12, day: 31), 1),
        (CalendarDate(year: 2100, month: 2, day: 28), 1), (CalendarDate(year: 2000, month: 2, day: 28), 1),
        (CalendarDate(year: 2024, month: 1, day: 1), 146_097), (CalendarDate(year: 2024, month: 1, day: 1), -146_097),
    ]
    for (day, days) in hand { add("adding", ["date": dayJSON(day), "days": days], ["date": dayJSON(day.adding(days: days))]) }
    while cases.count < 500 {
        let day = randomDay()
        let days = rng.chance(20) ? rng.int(-4000...4000) : rng.int(-40...40)
        add("adding", ["date": dayJSON(day), "days": days], ["date": dayJSON(day.adding(days: days))])
    }
    return cases
}
