import CalendarCore
import Foundation

// recurrence/rule-instances.json
//
// Entry point: RecurrenceRule.instances(anchor:timeZone:isAllDay:before:limit:skipTo:) -> RuleExpansion
// (the plan's "window" is the pair `before` and `skipTo` of the real signature; there is no window argument).
// The rule is built with RecurrenceRule(rrule:in:) from the text, in the case's zone.
//
//   input:    { "rrule": "<RRULE text, without RRULE:>", "anchor": <ms>, "zone": "<IANA id>", "isAllDay": <Bool>,
//               "before": <ms>, "limit": <Int>, "skipTo": <ms> | null }
//   expected: { "starts": [<ms>, ...], "truncated": <Bool> }
//
// 2,000 cases: 40 hand-written ones (the RFC 5545 listings that RuleExpanderTests and RecurrenceRuleTests use,
// anchored on 1997-09-02 09:00 in America/New_York), 25 on anchors that are hard (29 February, the 31st, DST gaps
// and overlaps) with each of the common rules, then seeded random rules covering every FREQ, INTERVAL 1 to 4,
// BYDAY with and without ordinals, BYMONTHDAY positive and negative, BYMONTH, BYSETPOS, BYWEEKNO, BYYEARDAY,
// WKST, COUNT, UNTIL (date, UTC date-time and floating forms) and skipTo.

func exportRecurrenceRuleInstances() -> [VectorCase] {
    var rng = LCG(state: 0x9ECC_17E5_0001)
    var cases: [VectorCase] = []

    func add(_ tag: String, rule: String, anchor: Anchor, before: Int, limit: Int, skipTo: Int?) {
        guard let parsed = try? RecurrenceRule(rrule: rule, in: zone(anchor.zone)) else { return }
        let result = parsed.instances(
            anchor: instant(ms: anchor.ms), timeZone: zone(anchor.zone), isAllDay: anchor.isAllDay,
            before: instant(ms: before), limit: limit, skipTo: skipTo.map { instant(ms: $0) })
        cases.append(VectorCase(
            name: "rule-instances-\(tag)-\(fourDigits(cases.count + 1))",
            input: ["rrule": rule, "anchor": anchor.ms, "zone": anchor.zone, "isAllDay": anchor.isAllDay,
                    "before": before, "limit": limit, "skipTo": orNull(skipTo)],
            expected: ["starts": result.starts.map { ms($0) }, "truncated": result.truncated]))
    }

    // Hand-written: the RFC listings.
    let ny = "America/New_York"
    let rfcAnchor = Anchor(zone: ny, isAllDay: false, ms: ms(WallClock.date(CalendarDate(year: 1997, month: 9, day: 2), hour: 9, minute: 0, second: 0, in: zone(ny))!))
    let janAnchor = Anchor(zone: ny, isAllDay: false, ms: ms(WallClock.date(CalendarDate(year: 1998, month: 1, day: 1), hour: 9, minute: 0, second: 0, in: zone(ny))!))
    let rfcBefore = ms(WallClock.date(CalendarDate(year: 2000, month: 1, day: 1), hour: 9, minute: 0, second: 0, in: zone(ny))!)
    let rfcRules = [
        "FREQ=DAILY;COUNT=10", "FREQ=DAILY;UNTIL=19971224T000000Z", "FREQ=DAILY;INTERVAL=2", "FREQ=DAILY;INTERVAL=10;COUNT=5",
        "FREQ=WEEKLY;COUNT=10", "FREQ=WEEKLY;INTERVAL=2;WKST=SU", "FREQ=WEEKLY;UNTIL=19971224T000000Z;WKST=SU;BYDAY=MO,WE,FR",
        "FREQ=WEEKLY;INTERVAL=2;COUNT=8;WKST=SU;BYDAY=TU,TH", "FREQ=WEEKLY;UNTIL=19971007T000000Z;WKST=SU;BYDAY=TU,TH",
        "FREQ=WEEKLY;COUNT=4;INTERVAL=2;BYDAY=TU,SU;WKST=MO", "FREQ=WEEKLY;COUNT=4;INTERVAL=2;BYDAY=TU,SU;WKST=SU",
        "FREQ=MONTHLY;COUNT=10;BYDAY=1FR", "FREQ=MONTHLY;INTERVAL=2;COUNT=10;BYDAY=1SU,-1SU", "FREQ=MONTHLY;COUNT=6;BYDAY=-2MO",
        "FREQ=MONTHLY;BYMONTHDAY=-3", "FREQ=MONTHLY;COUNT=10;BYMONTHDAY=2,15", "FREQ=MONTHLY;COUNT=10;BYMONTHDAY=1,-1",
        "FREQ=MONTHLY;INTERVAL=18;COUNT=10;BYMONTHDAY=10,11,12,13,14,15", "FREQ=MONTHLY;INTERVAL=2;BYDAY=TU",
        "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1", "FREQ=MONTHLY;COUNT=3;BYDAY=TU,WE,TH;BYSETPOS=3",
        "FREQ=MONTHLY;BYMONTHDAY=7,8,9,10,11,12,13;BYDAY=SA;COUNT=6",
        "FREQ=YEARLY;COUNT=10;BYMONTH=6,7", "FREQ=YEARLY;INTERVAL=2;COUNT=10;BYMONTH=1,2,3",
        "FREQ=YEARLY;INTERVAL=3;COUNT=10;BYYEARDAY=1,100,200", "FREQ=YEARLY;BYDAY=20MO;COUNT=5", "FREQ=YEARLY;BYWEEKNO=20;BYDAY=MO;COUNT=5",
        "FREQ=YEARLY;BYMONTH=3;BYDAY=TH;COUNT=8", "FREQ=YEARLY;BYDAY=TU,TH;BYMONTH=6,7,8;COUNT=6",
        "FREQ=YEARLY;BYMONTH=11;BYDAY=TU;BYMONTHDAY=2,3,4,5,6,7,8;COUNT=4",
        "FREQ=HOURLY;INTERVAL=3;UNTIL=19970902T170000Z", "FREQ=MINUTELY;INTERVAL=15;COUNT=6", "FREQ=MINUTELY;INTERVAL=90;COUNT=4",
        "FREQ=DAILY;BYHOUR=9,10,11,12,13,14,15,16;BYMINUTE=0,20,40;COUNT=20", "FREQ=MINUTELY;INTERVAL=20;BYHOUR=9,10,11,12,13,14,15,16;COUNT=20",
        "FREQ=MONTHLY;BYMONTHDAY=29,30,31;BYSETPOS=-1;COUNT=6", "FREQ=YEARLY;BYMONTH=2;BYMONTHDAY=29;COUNT=3",
        "FREQ=MONTHLY;BYMONTHDAY=31;COUNT=7", "FREQ=DAILY;BYMONTH=1;BYDAY=SU,MO,TU,WE,TH,FR,SA;UNTIL=19980131T140000Z",
    ]
    for rule in rfcRules { add("rfc", rule: rule, anchor: rfcAnchor, before: rfcBefore, limit: 200, skipTo: nil) }
    add("rfc", rule: "FREQ=YEARLY;UNTIL=20000131T140000Z;BYMONTH=1;BYDAY=SU,MO,TU,WE,TH,FR,SA", anchor: janAnchor, before: rfcBefore, limit: 200, skipTo: nil)
    precondition(cases.count == 40, "hand-written rule instances: \(cases.count)")

    // Hard anchors, each with a handful of common rules.
    let commonRules = ["FREQ=DAILY;COUNT=5", "FREQ=WEEKLY", "FREQ=MONTHLY", "FREQ=YEARLY", "FREQ=MONTHLY;BYDAY=2TU;COUNT=4", "FREQ=DAILY;INTERVAL=2"]
    for anchor in specialAnchors() {
        for rule in commonRules {
            guard cases.count < 40 + 160 else { break }
            // Havana's rules for 2027 differ between zone databases: its cases stop at the horizon.
            let before = anchor.zone == "America/Havana" ? zoneHorizon : anchor.ms + 3 * 366 * 86_400_000
            add("anchor", rule: rule, anchor: anchor, before: before, limit: 14, skipTo: nil)
        }
    }

    // Random.
    while cases.count < 2000 {
        let anchor = rng.chance(8) ? rng.pick(specialAnchors().filter { $0.zone != "America/Havana" }) : randomAnchor(&rng)
        let (text, freq, span) = randomRuleText(&rng, anchor: anchor)
        let limit = rng.pick([5, 12, 25, 40, 60])
        let before = anchor.ms + span * 1000
        let skip: Int? = rng.chance(30) ? anchor.ms + Int(Double(span) * 1000 * (0.25 + 0.65 * Double(rng.below(100)) / 100)) : nil
        add("\(freq.lowercased())", rule: text, anchor: anchor, before: before, limit: limit, skipTo: skip)
    }
    return cases
}
