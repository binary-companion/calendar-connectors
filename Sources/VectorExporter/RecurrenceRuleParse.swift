import CalendarCore
import Foundation

// recurrence/rule-parse.json
//
// Entry points: RecurrenceRule.init(rrule:in:) (the parser) and RecurrenceRule.rruleString(allDay:in:) (the serializer).
//
//   input:    { "rrule": "<text, exactly as given, with or without RRULE:>", "zone": "<IANA id>" | null }
//   expected: a parse that succeeds:
//               { "ok": true, "frequency": "DAILY", "interval": <Int>, "weekStart": "MO",
//                 "weekdays": [ { "weekday": "TU", "ordinal": <Int> | null } ], "monthDays": [..], "months": [..],
//                 "yearDays": [..], "weekNumbers": [..], "setPositions": [..], "hours": [..], "minutes": [..], "seconds": [..],
//                 "end": { "kind": "never" } | { "kind": "count", "count": <Int> } | { "kind": "until", "instant": <ms> },
//                 "unrecognizedParts": [ { "name": "X-FOO", "value": "bar" } ],
//                 "isWritableSubset": <Bool>,
//                 "timedText": rruleString(allDay: false, in: nil),
//                 "allDayText": rruleString(allDay: true, in: zone) }
//             a parse that fails: { "ok": false, "message": "<RecurrenceParseError.malformed payload>" }
//
// 300 valid and 100 malformed texts. "zone" is the zone a date-only or floating UNTIL is read in (null means UTC).
// "allDayText" uses the zone (UTC when null). Valid texts: the Swift tests' texts, spelling variants (lower case,
// "RRULE:" prefix, surrounding space), and seeded random rules. Malformed: the tests' texts, then one bad part
// added to a valid rule.

func exportRecurrenceRuleParse() -> [VectorCase] {
    var rng = LCG(state: 0x9A25_E000_0003)
    var cases: [VectorCase] = []
    var valid = 0, malformed = 0

    func weekday(_ w: RecurrenceRule.WeekdayOccurrence) -> [String: Any] { ["weekday": w.weekday.rawValue, "ordinal": orNull(w.ordinal)] }
    func add(_ text: String, zoneID: String?) {
        let tz = zoneID.map { zone($0) }
        let expected: [String: Any]
        do {
            let r = try RecurrenceRule(rrule: text, in: tz)
            let end: [String: Any]
            switch r.end {
            case .never: end = ["kind": "never"]
            case .count(let n): end = ["kind": "count", "count": n]
            case .until(let d): end = ["kind": "until", "instant": ms(d)]
            }
            expected = [
                "ok": true, "frequency": r.frequency.rawValue, "interval": r.interval, "weekStart": r.weekStart.rawValue,
                "weekdays": r.weekdays.map(weekday), "monthDays": r.monthDays, "months": r.months, "yearDays": r.yearDays,
                "weekNumbers": r.weekNumbers, "setPositions": r.setPositions, "hours": r.hours, "minutes": r.minutes,
                "seconds": r.seconds, "end": end,
                "unrecognizedParts": r.unrecognizedParts.map { ["name": $0.name, "value": $0.value] },
                "isWritableSubset": r.isWritableSubset,
                "timedText": r.rruleString(allDay: false, in: nil),
                "allDayText": r.rruleString(allDay: true, in: tz),
            ]
            valid += 1
        } catch RecurrenceParseError.malformed(let message) {
            expected = ["ok": false, "message": message]
            malformed += 1
        } catch {
            fatalError("unexpected error \(error)")
        }
        cases.append(VectorCase(name: "rule-parse-\(fourDigits(cases.count + 1))", input: ["rrule": text, "zone": orNull(zoneID)], expected: expected))
    }

    // Valid, hand-written.
    let handValid = [
        "RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=6", "FREQ=MONTHLY;BYDAY=2TU,-1FR", "FREQ=MONTHLY;BYMONTHDAY=1,15,-1",
        "FREQ=YEARLY;BYMONTH=3,9;BYMONTHDAY=5", "FREQ=MONTHLY;INTERVAL=3;BYDAY=2TU;COUNT=4", "FREQ=DAILY", "FREQ=WEEKLY;UNTIL=20261005T150000Z",
        "FREQ=WEEKLY;UNTIL=20261006", "FREQ=WEEKLY;UNTIL=20261006T090000", "FREQ=MONTHLY;BYSETPOS=1;BYDAY=MO", "FREQ=HOURLY", "FREQ=DAILY;BYHOUR=9",
        "FREQ=WEEKLY;WKST=SU", "FREQ=WEEKLY;BYYEARDAY=3", "FREQ=DAILY;BYDAY=MO", "FREQ=WEEKLY;BYWEEKNO=3", "FREQ=MINUTELY;INTERVAL=15",
        "FREQ=DAILY;BYMINUTE=0,30;BYSECOND=0", "FREQ=DAILY;X-FOO=bar", "FREQ=DAILY;X-FOO=bar;X-BAZ=1",
        "FREQ=YEARLY;INTERVAL=2;BYMONTH=1,7;BYWEEKNO=20;BYYEARDAY=1,-1;BYMONTHDAY=15;BYDAY=-1SU;BYHOUR=9;BYMINUTE=30;BYSECOND=0;BYSETPOS=-1;WKST=SU;COUNT=10;X-A=1;X-B=2",
        "FREQ=YEARLY;BYMONTH=1;BYYEARDAY=1;BYSETPOS=1;WKST=SU", "FREQ=MINUTELY;INTERVAL=15;BYHOUR=9;COUNT=4", "FREQ=WEEKLY;WKST=MO;BYDAY=TU",
        "  rrule:freq=weekly;interval=2;byday=mo,we;until=20261005t150000z", "RRULE:freq=monthly;byday=-1fr", "FREQ=DAILY;BYSECOND=60",
        "FREQ=YEARLY;BYDAY=53MO", "FREQ=YEARLY;BYDAY=-53SU", "FREQ=MONTHLY;BYMONTHDAY=-31,31", "FREQ=YEARLY;BYYEARDAY=366,-366", "FREQ=SECONDLY;COUNT=2",
        "FREQ=DAILY;UNTIL=20240229", "FREQ=DAILY;UNTIL=20240229T235959Z", "FREQ=DAILY;UNTIL=20240229T235959", "FREQ=DAILY;X-EMPTY-NOT=1;COUNT=2",
        "RRULE:FREQ=DAILY;COUNT=1\n", "\tFREQ=DAILY\t", "FREQ=DAILY;;COUNT=2", "FREQ=DAILY;",
    ]
    for text in handValid { add(text, zoneID: nil) }
    for id in ["Asia/Tokyo", "America/Los_Angeles", "Pacific/Auckland", "America/Sao_Paulo"] {
        for text in ["FREQ=WEEKLY;UNTIL=20261006", "FREQ=WEEKLY;UNTIL=20261006T090000", "FREQ=WEEKLY;UNTIL=20261005T150000Z", "FREQ=DAILY;UNTIL=20261101", "FREQ=DAILY;UNTIL=20260308T023000"] {
            add(text, zoneID: id)
        }
    }
    // Valid, random: the text as generated, then a spelling variant of some.
    let zonesForParse: [String?] = [nil, nil, "Asia/Tokyo", "America/New_York", "Australia/Sydney", "Europe/London"]
    while valid < 300 {
        let anchor = randomAnchor(&rng)
        var text = randomRuleText(&rng, anchor: anchor).text
        switch rng.below(8) {
        case 0: text = "RRULE:" + text
        case 1: text = text.lowercased()
        case 2: text = "  " + text + " \n"
        case 3: text += ";X-VENDOR=\(rng.int(1...99));X-OTHER=a:b"
        default: break
        }
        add(text, zoneID: rng.pick(zonesForParse))
    }
    let validCount = valid
    precondition(validCount >= 300)

    // Malformed, hand-written.
    let handMalformed = [
        "", "RRULE:", ";", "INTERVAL=2", "FREQ", "FREQ=", "FREQ=SOMETIMES", "FREQ=DAILY;INTERVAL=x", "FREQ=DAILY;INTERVAL=0", "FREQ=DAILY;INTERVAL=-1",
        "FREQ=DAILY;COUNT=3;UNTIL=20261005", "FREQ=DAILY;COUNT=0", "FREQ=DAILY;COUNT=x", "FREQ=DAILY;BYHOUR=24", "FREQ=YEARLY;BYMONTH=13",
        "FREQ=YEARLY;BYMONTH=0", "FREQ=YEARLY;BYYEARDAY=0", "FREQ=YEARLY;BYYEARDAY=367", "FREQ=WEEKLY;WKST=XX", "FREQ=WEEKLY;BYDAY=XX",
        "FREQ=WEEKLY;BYDAY=", "FREQ=WEEKLY;BYDAY=M", "FREQ=WEEKLY;BYDAY=0MO", "FREQ=WEEKLY;BYDAY=54MO", "FREQ=WEEKLY;BYDAY=MO,,WE", "FREQ=WEEKLY;BYDAY=MO,",
        "FREQ=MONTHLY;BYMONTHDAY=0", "FREQ=MONTHLY;BYMONTHDAY=32", "FREQ=MONTHLY;BYMONTHDAY=1,,2", "FREQ=MONTHLY;BYMONTHDAY=-9223372036854775808",
        "FREQ=MONTHLY;BYMONTHDAY=9223372036854775807", "FREQ=MONTHLY;BYDAY=-9223372036854775808TU", "FREQ=YEARLY;BYMONTH=3,,4", "FREQ=YEARLY;BYWEEKNO=54",
        "FREQ=YEARLY;BYWEEKNO=0", "FREQ=YEARLY;BYSETPOS=0", "FREQ=YEARLY;BYSETPOS=367", "FREQ=DAILY;BYMINUTE=60", "FREQ=DAILY;BYSECOND=61",
        "FREQ=DAILY;UNTIL=20261345", "FREQ=DAILY;UNTIL=20261005T256199Z", "FREQ=DAILY;UNTIL=20260230", "FREQ=DAILY;UNTIL=20261005T240000Z",
        "FREQ=DAILY;UNTIL=00000000", "FREQ=DAILY;UNTIL=abcdefgh", "FREQ=DAILY;UNTIL=20261005TAB0000Z", "FREQ=DAILY;UNTIL=2026",
        "FREQ=DAILY;UNTIL=\u{0662}\u{0660}\u{0662}\u{0666}\u{0661}\u{0660}\u{0660}\u{0665}", "FREQ=DAILY;UNTIL=2026100\u{0665}T150000Z",
        "FREQ=DAILY;FREQ=WEEKLY", "FREQ=DAILY;COUNT=3;COUNT=5", "FREQ=DAILY;freq=WEEKLY", "FREQ=DAILY;X-A=", "FREQ=DAILY;=5", "FREQ=DAILY;COUNT",
    ]
    for text in handMalformed { add(text, zoneID: nil) }
    // Malformed, one bad part added to a valid rule.
    let bad = ["INTERVAL=0", "COUNT=0", "BYHOUR=24", "BYMONTH=13", "BYMONTHDAY=0", "BYMONTHDAY=32", "BYDAY=XX", "BYDAY=54MO", "WKST=ZZ",
               "UNTIL=2026", "FREQ=HOURLY", "BYSETPOS=0", "BYWEEKNO=54", "BYYEARDAY=367", "BYMINUTE=60", "BYSECOND=61", "INTERVAL=", "X-A", "COUNT=3"]
    while malformed < 100 {
        let anchor = randomAnchor(&rng)
        let base = randomRuleText(&rng, anchor: anchor).text
        add(base + ";" + rng.pick(bad), zoneID: nil)
    }
    return cases
}
