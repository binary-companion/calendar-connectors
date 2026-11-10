import CalendarCore
import Foundation

// recurrence/set-occurrences.json
//
// Entry points: RecurrenceSet.occurrences(anchor:duration:timeZone:isAllDay:overlapping:limit:) -> RecurrenceExpansion
// and RecurrenceSet.ruleInstanceCount(anchor:timeZone:isAllDay:before:) -> Int. A set is built from its parts:
// rules from RecurrenceRule(rrule:in:), `extraDates` from "rdates", `excludedDates` from "exdates", `unparsed` as given.
//
//   fn "occurrences":       input { "fn", "rules": ["<RRULE text>"], "rdates": [<ms>] | null, "exdates": [<ms>] | null,
//                                   "unparsed": ["<line>"], "anchor": <ms>, "durationSeconds": <Int>, "zone": "<IANA id>",
//                                   "isAllDay": <Bool>, "windowStart": <ms>, "windowEnd": <ms>, "limit": <Int> }
//                           expected { "starts": [<ms>], "truncated": <Bool>, "hasUnreadableRule": <Bool> }
//   fn "ruleInstanceCount": input { "fn", "rules": [...], "anchor", "zone", "isAllDay", "before": <ms> }
//                           expected { "count": <Int> }
//
// "rdates" or "exdates" null is the EventKit case (the source cannot say). 300 cases: 240 "occurrences" and
// 60 "ruleInstanceCount". The window is Foundation's DateInterval (the end is included in intersection tests, but the
// library compares with its own rules, so the file only needs the two instants). Exdates and rdates of all-day
// sets are canonical midnights.

func exportRecurrenceSetOccurrences() -> [VectorCase] {
    var rng = LCG(state: 0x5E7_0CC0_0002)
    var cases: [VectorCase] = []

    func build(_ rules: [String], _ rdates: [Int]?, _ exdates: [Int]?, _ unparsed: [String], _ anchor: Anchor) -> RecurrenceSet? {
        var parsed: [RecurrenceRule] = []
        for text in rules {
            guard let rule = try? RecurrenceRule(rrule: text, in: zone(anchor.zone)) else { return nil }
            parsed.append(rule)
        }
        return RecurrenceSet(rules: parsed, extraDates: rdates.map { $0.map { instant(ms: $0) } },
                             excludedDates: exdates.map { $0.map { instant(ms: $0) } }, unparsed: unparsed)
    }
    func occurrences(_ tag: String, rules: [String], rdates: [Int]?, exdates: [Int]?, unparsed: [String] = [], anchor: Anchor,
                     duration: Int, windowStart: Int, windowEnd: Int, limit: Int) {
        guard let set = build(rules, rdates, exdates, unparsed, anchor) else { return }
        let result = set.occurrences(
            anchor: instant(ms: anchor.ms), duration: TimeInterval(duration), timeZone: zone(anchor.zone), isAllDay: anchor.isAllDay,
            overlapping: DateInterval(start: instant(ms: windowStart), end: instant(ms: windowEnd)), limit: limit)
        cases.append(VectorCase(
            name: "set-occurrences-\(tag)-\(fourDigits(cases.count + 1))",
            input: ["fn": "occurrences", "rules": rules, "rdates": orNull(rdates), "exdates": orNull(exdates), "unparsed": unparsed,
                    "anchor": anchor.ms, "durationSeconds": duration, "zone": anchor.zone, "isAllDay": anchor.isAllDay,
                    "windowStart": windowStart, "windowEnd": windowEnd, "limit": limit],
            expected: ["starts": result.starts.map { ms($0) }, "truncated": result.truncated, "hasUnreadableRule": result.hasUnreadableRule]))
    }
    func count(_ rules: [String], anchor: Anchor, before: Int) {
        guard let set = build(rules, [], [], [], anchor) else { return }
        let n = set.ruleInstanceCount(anchor: instant(ms: anchor.ms), timeZone: zone(anchor.zone), isAllDay: anchor.isAllDay, before: instant(ms: before))
        cases.append(VectorCase(
            name: "set-count-\(fourDigits(cases.count + 1))",
            input: ["fn": "ruleInstanceCount", "rules": rules, "anchor": anchor.ms, "zone": anchor.zone, "isAllDay": anchor.isAllDay, "before": before],
            expected: ["count": n]))
    }

    // Hand-written: no rules, a single occurrence, exclusions, the nil (EventKit) forms, unreadable rules.
    let ny = "America/New_York"
    let anchor = Anchor(zone: ny, isAllDay: false, ms: ms(WallClock.date(CalendarDate(year: 2026, month: 3, day: 2), hour: 9, minute: 0, second: 0, in: zone(ny))!))
    let day = 86_400_000
    let windowStart = anchor.ms - 7 * day, windowEnd = anchor.ms + 60 * day
    occurrences("none", rules: [], rdates: [], exdates: [], anchor: anchor, duration: 3600, windowStart: windowStart, windowEnd: windowEnd, limit: 100)
    occurrences("none-outside", rules: [], rdates: [], exdates: [], anchor: anchor, duration: 3600, windowStart: anchor.ms + day, windowEnd: anchor.ms + 5 * day, limit: 100)
    occurrences("none-overlap-end", rules: [], rdates: [], exdates: [], anchor: anchor, duration: 7200, windowStart: anchor.ms + 3_600_000, windowEnd: anchor.ms + 5 * day, limit: 100)
    occurrences("zero-duration-edge", rules: [], rdates: [], exdates: [], anchor: anchor, duration: 0, windowStart: anchor.ms, windowEnd: anchor.ms + day, limit: 100)
    occurrences("zero-duration-end", rules: [], rdates: [], exdates: [], anchor: anchor, duration: 0, windowStart: anchor.ms - day, windowEnd: anchor.ms, limit: 100)
    occurrences("weekly", rules: ["FREQ=WEEKLY;COUNT=6"], rdates: [], exdates: [], anchor: anchor, duration: 3600, windowStart: windowStart, windowEnd: windowEnd, limit: 100)
    occurrences("weekly-exdate", rules: ["FREQ=WEEKLY;COUNT=6"], rdates: [], exdates: [anchor.ms + 7 * day], anchor: anchor, duration: 3600, windowStart: windowStart, windowEnd: windowEnd, limit: 100)
    occurrences("weekly-rdate", rules: ["FREQ=WEEKLY;COUNT=3"], rdates: [anchor.ms + 3 * day], exdates: [], anchor: anchor, duration: 3600, windowStart: windowStart, windowEnd: windowEnd, limit: 100)
    occurrences("nil-lists", rules: ["FREQ=DAILY;COUNT=4"], rdates: nil, exdates: nil, anchor: anchor, duration: 1800, windowStart: windowStart, windowEnd: windowEnd, limit: 100)
    occurrences("unreadable", rules: [], rdates: [], exdates: [], unparsed: ["RRULE:FREQ=SOMETIMES"], anchor: anchor, duration: 3600, windowStart: windowStart, windowEnd: windowEnd, limit: 100)
    occurrences("unreadable-outside", rules: [], rdates: [], exdates: [], unparsed: ["rrule;x=1:FREQ=SOMETIMES"], anchor: anchor, duration: 3600, windowStart: anchor.ms + day, windowEnd: anchor.ms + 9 * day, limit: 100)
    occurrences("exrule-ignored", rules: ["FREQ=DAILY;COUNT=3"], rdates: [], exdates: [], unparsed: ["EXRULE:FREQ=DAILY;COUNT=1"], anchor: anchor, duration: 3600, windowStart: windowStart, windowEnd: windowEnd, limit: 100)
    occurrences("limit", rules: ["FREQ=DAILY"], rdates: [], exdates: [], anchor: anchor, duration: 3600, windowStart: windowStart, windowEnd: windowEnd, limit: 10)
    occurrences("two-rules", rules: ["FREQ=WEEKLY;BYDAY=MO;COUNT=4", "FREQ=WEEKLY;BYDAY=WE;COUNT=4"], rdates: [], exdates: [], anchor: anchor, duration: 3600, windowStart: windowStart, windowEnd: windowEnd, limit: 100)
    occurrences("duplicate-rdate", rules: ["FREQ=DAILY;COUNT=3"], rdates: [anchor.ms + day], exdates: [], anchor: anchor, duration: 3600, windowStart: windowStart, windowEnd: windowEnd, limit: 100)
    occurrences("exdate-the-anchor", rules: ["FREQ=DAILY;COUNT=3"], rdates: [], exdates: [anchor.ms], anchor: anchor, duration: 3600, windowStart: windowStart, windowEnd: windowEnd, limit: 100)

    // Random sets.
    while cases.count < 240 {
        let a = randomAnchor(&rng)
        let (text, freq, span) = randomRuleText(&rng, anchor: a)
        var rules = [text]
        if rng.chance(15) {
            let more = randomRuleText(&rng, anchor: a)
            rules.append(more.text)
        }
        if rng.chance(5) { rules = [] }
        let spanMs = span * 1000
        let start = a.ms - Int(Double(spanMs) * Double(rng.below(30)) / 100)
        let end = a.ms + spanMs
        // A first look at the series gives realistic exclusions: some of its own starts.
        var exdates: [Int]? = []
        var rdates: [Int]? = []
        let base = build(rules, [], [], [], a)
        let own = base?.occurrences(anchor: instant(ms: a.ms), duration: 0, timeZone: zone(a.zone), isAllDay: a.isAllDay,
                                    overlapping: DateInterval(start: instant(ms: a.ms), end: instant(ms: end)), limit: 30).starts.map { ms($0) } ?? []
        if rng.chance(45), !own.isEmpty { exdates = rng.subset(own, count: min(own.count, rng.int(1...3))) }
        if rng.chance(30) {
            rdates = (0..<rng.int(1...3)).map { _ in
                let t = a.ms + rng.below(max(1, spanMs))
                return a.isAllDay ? ms(AllDay.startOfDay(AllDay.date(of: instant(ms: t), in: zone(a.zone)), in: zone(a.zone))!) : t
            }
        }
        if rng.chance(5) { rdates = nil }
        if rng.chance(5) { exdates = nil }
        var unparsed: [String] = []
        if rng.chance(8) { unparsed.append("EXRULE:FREQ=DAILY;COUNT=2") }
        if rng.chance(4) { unparsed.append("RRULE:FREQ=BOGUS") }
        let duration = a.isAllDay ? rng.pick([86_400, 2 * 86_400, 0]) : rng.pick([0, 900, 1800, 3600, 5400, 86_400, 3 * 86_400])
        occurrences("\(freq.lowercased())", rules: rules, rdates: rdates, exdates: exdates, unparsed: unparsed, anchor: a, duration: duration,
                    windowStart: start, windowEnd: end, limit: rng.pick([3, 8, 20, 50, 100]))
    }

    // ruleInstanceCount.
    while cases.count < 300 {
        let a = randomAnchor(&rng)
        let (text, _, span) = randomRuleText(&rng, anchor: a)
        let rules = rng.chance(8) ? [] : [text]
        count(rules, anchor: a, before: a.ms + Int(Double(span) * 1000 * (rng.chance(10) ? -0.1 : 1)))
    }
    return cases
}
