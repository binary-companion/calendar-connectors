use super::support::{iso, zone};
use crate::all_day::CalendarDate;
use crate::recurrence::rule::{
    Frequency, RecurrenceParseError, RecurrenceRule, RuleEnd, UnrecognizedPart, Weekday,
    WeekdayOccurrence,
};
use crate::write::types::{EventField, WriteError};

fn plain(weekday: Weekday) -> WeekdayOccurrence {
    WeekdayOccurrence::new(weekday, None)
}

fn nth(weekday: Weekday, ordinal: i64) -> WeekdayOccurrence {
    WeekdayOccurrence::new(weekday, Some(ordinal))
}

fn parse(text: &str) -> RecurrenceRule {
    RecurrenceRule::from_rrule(text, None).unwrap()
}

/// `expectParseError`: the parse fails with exactly `RecurrenceParseError::Malformed(message)`.
fn expect_parse_error(expected: &str, text: &str) {
    assert_eq!(
        RecurrenceRule::from_rrule(text, None),
        Err(RecurrenceParseError::Malformed(expected.to_string())),
        "{text}"
    );
}

#[test]
fn parses_a_weekly_rule_with_days_and_count() {
    let rule = parse("RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=6");
    assert!(rule.frequency == Frequency::Weekly && rule.interval == 2);
    assert_eq!(
        rule.weekdays,
        vec![plain(Weekday::Monday), plain(Weekday::Wednesday)]
    );
    assert_eq!(rule.end, RuleEnd::Count(6));
}

#[test]
fn parses_ordinal_weekdays_month_days_and_months() {
    let monthly = parse("FREQ=MONTHLY;BYDAY=2TU,-1FR");
    assert_eq!(
        monthly.weekdays,
        vec![nth(Weekday::Tuesday, 2), nth(Weekday::Friday, -1)]
    );
    let by_day = parse("FREQ=MONTHLY;BYMONTHDAY=1,15,-1");
    assert_eq!(by_day.month_days, vec![1, 15, -1]);
    let yearly = parse("FREQ=YEARLY;BYMONTH=3,9;BYMONTHDAY=5");
    assert!(yearly.months == vec![3, 9] && yearly.month_days == vec![5]);
}

#[test]
fn renders_in_canonical_order() {
    let rule = RecurrenceRule {
        interval: 3,
        weekdays: vec![nth(Weekday::Tuesday, 2)],
        end: RuleEnd::Count(4),
        ..RecurrenceRule::new(Frequency::Monthly)
    };
    assert_eq!(
        rule.rrule_string(false, None),
        "FREQ=MONTHLY;INTERVAL=3;BYDAY=2TU;COUNT=4"
    );
    assert_eq!(
        RecurrenceRule::new(Frequency::Daily).rrule_string(false, None),
        "FREQ=DAILY"
    );
}

#[test]
fn until_renders_as_utc_date_time_for_timed_and_date_in_zone_for_all_day() {
    let tokyo = zone("Asia/Tokyo");
    let until = iso("2026-10-05T15:00:00Z"); // 2026-10-06 00:00 in Tokyo
    let rule = RecurrenceRule {
        end: RuleEnd::Until(until),
        ..RecurrenceRule::new(Frequency::Daily)
    };
    assert_eq!(
        rule.rrule_string(false, Some(&tokyo)),
        "FREQ=DAILY;UNTIL=20261005T150000Z"
    );
    assert_eq!(
        rule.rrule_string(true, Some(&tokyo)),
        "FREQ=DAILY;UNTIL=20261006"
    );
}

#[test]
fn until_round_trips_through_parse_and_render() {
    let tokyo = zone("Asia/Tokyo");
    let timed = parse("FREQ=WEEKLY;UNTIL=20261005T150000Z");
    assert_eq!(timed.end, RuleEnd::Until(iso("2026-10-05T15:00:00Z")));
    assert_eq!(
        timed.rrule_string(false, None),
        "FREQ=WEEKLY;UNTIL=20261005T150000Z"
    );
    let all_day = RecurrenceRule::from_rrule("FREQ=WEEKLY;UNTIL=20261006", Some(&tokyo)).unwrap();
    assert_eq!(all_day.end, RuleEnd::Until(iso("2026-10-05T15:00:00Z")));
    assert_eq!(
        all_day.rrule_string(true, Some(&tokyo)),
        "FREQ=WEEKLY;UNTIL=20261006"
    );
}

// A rule outside what writers can express still parses; `validate()` refuses to write it.
#[test]
fn a_rule_outside_the_writable_subset_parses_but_is_not_writable() {
    for text in [
        "FREQ=MONTHLY;BYSETPOS=1;BYDAY=MO",
        "FREQ=HOURLY",
        "FREQ=DAILY;BYHOUR=9",
        "FREQ=WEEKLY;WKST=SU",
        "FREQ=WEEKLY;BYYEARDAY=3",
        "FREQ=DAILY;BYDAY=MO",
        "FREQ=WEEKLY;BYWEEKNO=3",
        "FREQ=MINUTELY;INTERVAL=15",
        "FREQ=DAILY;BYMINUTE=0,30;BYSECOND=0",
        "FREQ=DAILY;X-FOO=bar",
    ] {
        let rule = parse(text);
        assert_eq!(
            rule.validate(),
            Err(WriteError::Unsupported {
                fields: [EventField::Recurrence].into()
            }),
            "{text}"
        );
    }
}

#[test]
fn malformed_text_throws_a_parse_error() {
    for (text, message) in [
        ("INTERVAL=2", "RRULE has no FREQ"),
        ("FREQ=DAILY;INTERVAL=x", "bad INTERVAL: x"),
        ("FREQ=DAILY;INTERVAL=0", "bad INTERVAL: 0"),
        (
            "FREQ=DAILY;COUNT=3;UNTIL=20261005",
            "RRULE has both COUNT and UNTIL",
        ),
        ("FREQ=SOMETIMES", "unknown FREQ: SOMETIMES"),
        ("FREQ=DAILY;BYHOUR=24", "bad BYHOUR value: 24"),
        ("FREQ=YEARLY;BYMONTH=13", "bad BYMONTH value: 13"),
        ("FREQ=YEARLY;BYYEARDAY=0", "bad BYYEARDAY value: 0"),
        ("FREQ=WEEKLY;WKST=XX", "bad WKST: XX"),
    ] {
        expect_parse_error(message, text);
    }
}

#[test]
fn every_part_of_the_full_grammar_is_read() {
    let rule = parse(
        "FREQ=YEARLY;INTERVAL=2;BYMONTH=1,7;BYWEEKNO=20;BYYEARDAY=1,-1;BYMONTHDAY=15;BYDAY=-1SU;BYHOUR=9;BYMINUTE=30;BYSECOND=0;BYSETPOS=-1;WKST=SU;COUNT=10;X-A=1;X-B=2",
    );
    assert!(
        rule.frequency == Frequency::Yearly
            && rule.interval == 2
            && rule.months == vec![1, 7]
            && rule.week_numbers == vec![20]
            && rule.year_days == vec![1, -1]
    );
    assert!(
        rule.month_days == vec![15]
            && rule.weekdays == vec![nth(Weekday::Sunday, -1)]
            && rule.hours == vec![9]
            && rule.minutes == vec![30]
            && rule.seconds == vec![0]
    );
    assert!(
        rule.set_positions == vec![-1]
            && rule.week_start == Weekday::Sunday
            && rule.end == RuleEnd::Count(10)
    );
    assert_eq!(
        rule.unrecognized_parts,
        vec![
            UnrecognizedPart::new("X-A", "1"),
            UnrecognizedPart::new("X-B", "2")
        ]
    );
}

#[test]
fn unrecognized_parts_and_the_full_grammar_survive_a_round_trip() {
    for text in [
        "FREQ=DAILY;X-FOO=bar;X-BAZ=1",
        "FREQ=YEARLY;BYMONTH=1;BYYEARDAY=1;BYSETPOS=1;WKST=SU",
        "FREQ=MINUTELY;INTERVAL=15;BYHOUR=9;COUNT=4",
    ] {
        let rule = parse(text);
        assert_eq!(parse(&rule.rrule_string(false, None)), rule);
    }
    assert_eq!(
        parse("FREQ=DAILY;X-FOO=bar").rrule_string(false, None),
        "FREQ=DAILY;X-FOO=bar"
    );
}

#[test]
fn the_rfc_examples_round_trip() {
    for text in [
        // RFC 5545 section 3.8.5.3 examples.
        "FREQ=DAILY;COUNT=10",
        "FREQ=DAILY;UNTIL=19971224T000000Z",
        "FREQ=DAILY;INTERVAL=2",
        "FREQ=DAILY;INTERVAL=10;COUNT=5",
        "FREQ=YEARLY;UNTIL=20000131T140000Z;BYMONTH=1;BYDAY=SU,MO,TU,WE,TH,FR,SA",
        "FREQ=WEEKLY;COUNT=10",
        "FREQ=WEEKLY;INTERVAL=2;WKST=SU",
        "FREQ=WEEKLY;UNTIL=19971224T000000Z;WKST=SU;BYDAY=MO,WE,FR",
        "FREQ=WEEKLY;INTERVAL=2;COUNT=8;WKST=SU;BYDAY=TU,TH",
        "FREQ=MONTHLY;COUNT=10;BYDAY=1FR",
        "FREQ=MONTHLY;INTERVAL=2;COUNT=10;BYDAY=1SU,-1SU",
        "FREQ=MONTHLY;COUNT=6;BYDAY=-2MO",
        "FREQ=MONTHLY;BYMONTHDAY=-3",
        "FREQ=MONTHLY;COUNT=10;BYMONTHDAY=2,15",
        "FREQ=YEARLY;INTERVAL=2;COUNT=10;BYMONTH=1,2,3",
        "FREQ=YEARLY;INTERVAL=3;COUNT=10;BYYEARDAY=1,100,200",
        "FREQ=YEARLY;BYDAY=20MO",
        "FREQ=YEARLY;BYWEEKNO=20;BYDAY=MO",
        "FREQ=YEARLY;BYMONTH=3;BYDAY=TH",
        "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1",
        "FREQ=HOURLY;INTERVAL=3;UNTIL=19970902T170000Z",
        "FREQ=MINUTELY;INTERVAL=15;COUNT=6",
        "FREQ=DAILY;BYHOUR=9,10,11,12,13,14,15,16;BYMINUTE=0,20,40",
    ] {
        let rule = parse(text);
        let rendered = rule.rrule_string(false, None);
        assert_eq!(parse(&rendered), rule, "{text}");
    }
}

#[test]
fn a_floating_until_is_read_in_the_series_zone() {
    let tokyo = zone("Asia/Tokyo");
    let rule =
        RecurrenceRule::from_rrule("FREQ=WEEKLY;UNTIL=20261006T090000", Some(&tokyo)).unwrap();
    assert_eq!(rule.end, RuleEnd::Until(iso("2026-10-06T00:00:00Z"))); // 09:00 in Tokyo
}

#[test]
fn validation_catches_impossible_rules() {
    let invalid = |message: &str| Err(WriteError::Invalid(message.to_string()));
    assert_eq!(
        RecurrenceRule {
            interval: 0,
            ..RecurrenceRule::new(Frequency::Daily)
        }
        .validate(),
        invalid("recurrence interval must be at least 1")
    );
    assert_eq!(
        RecurrenceRule {
            weekdays: vec![nth(Weekday::Monday, 1)],
            ..RecurrenceRule::new(Frequency::Weekly)
        }
        .validate(),
        invalid("an ordinal weekday needs a monthly or yearly rule")
    );
    assert_eq!(
        RecurrenceRule {
            weekdays: vec![nth(Weekday::Monday, 6)],
            ..RecurrenceRule::new(Frequency::Monthly)
        }
        .validate(),
        invalid("weekday ordinal must be 1...5 or -5...-1")
    );
    assert_eq!(
        RecurrenceRule {
            month_days: vec![1],
            ..RecurrenceRule::new(Frequency::Weekly)
        }
        .validate(),
        invalid("month days apply to monthly and yearly rules")
    );
    assert_eq!(
        RecurrenceRule {
            months: vec![3],
            ..RecurrenceRule::new(Frequency::Monthly)
        }
        .validate(),
        invalid("months apply to yearly rules only")
    );
    assert_eq!(
        RecurrenceRule {
            end: RuleEnd::Count(0),
            ..RecurrenceRule::new(Frequency::Daily)
        }
        .validate(),
        invalid("recurrence count must be at least 1")
    );
    assert_eq!(
        RecurrenceRule {
            weekdays: vec![plain(Weekday::Monday)],
            end: RuleEnd::Count(3),
            ..RecurrenceRule::new(Frequency::Weekly)
        }
        .validate(),
        Ok(())
    );
}

// MARK: Review fixes

#[test]
fn rejects_bad_until_values() {
    for text in [
        "FREQ=DAILY;UNTIL=20261345",
        "FREQ=DAILY;UNTIL=20261005T256199Z",
        "FREQ=DAILY;UNTIL=20260230",
        "FREQ=DAILY;UNTIL=20261005T240000Z",
        "FREQ=DAILY;UNTIL=00000000",
        "FREQ=DAILY;UNTIL=\u{0662}\u{0660}\u{0662}\u{0666}\u{0661}\u{0660}\u{0660}\u{0665}",
        "FREQ=DAILY;UNTIL=2026100\u{0665}T150000Z",
        "FREQ=DAILY;UNTIL=abcdefgh",
        "FREQ=DAILY;UNTIL=20261005TAB0000Z",
    ] {
        let until_text: String = text
            .chars()
            .skip("FREQ=DAILY;UNTIL=".chars().count())
            .collect();
        expect_parse_error(&format!("bad UNTIL: {until_text}"), text);
    }
}

#[test]
fn huge_month_days_throw_instead_of_trapping() {
    for text in [
        "FREQ=MONTHLY;BYMONTHDAY=-9223372036854775808",
        "FREQ=MONTHLY;BYMONTHDAY=9223372036854775807",
    ] {
        let value = &text["FREQ=MONTHLY;BYMONTHDAY=".len()..];
        expect_parse_error(&format!("bad BYMONTHDAY value: {value}"), text);
    }
}

#[test]
fn huge_ordinals_throw_instead_of_trapping() {
    for text in [
        "FREQ=MONTHLY;BYDAY=-9223372036854775808TU",
        "FREQ=MONTHLY;BYDAY=9223372036854775807TU",
    ] {
        let value = &text["FREQ=MONTHLY;BYDAY=".len()..];
        expect_parse_error(&format!("bad BYDAY value: {value}"), text);
    }
}

#[test]
fn validate_accepts_the_range_edges() {
    assert_eq!(
        RecurrenceRule {
            weekdays: vec![nth(Weekday::Monday, 5), nth(Weekday::Friday, -5)],
            month_days: vec![31, -31, 1, -1],
            ..RecurrenceRule::new(Frequency::Monthly)
        }
        .validate(),
        Ok(())
    );
    assert_eq!(
        RecurrenceRule {
            month_days: vec![32],
            ..RecurrenceRule::new(Frequency::Monthly)
        }
        .validate(),
        Err(WriteError::Invalid(
            "month days must be 1...31 or -31...-1".to_string()
        ))
    );
}

#[test]
fn rejects_duplicate_parts() {
    for (text, key) in [
        ("FREQ=DAILY;FREQ=WEEKLY", "FREQ"),
        ("FREQ=DAILY;COUNT=3;COUNT=5", "COUNT"),
        ("FREQ=DAILY;freq=WEEKLY", "FREQ"),
    ] {
        expect_parse_error(&format!("duplicate RRULE part: {key}"), text);
    }
}

#[test]
fn rejects_empty_list_elements() {
    for text in [
        "FREQ=WEEKLY;BYDAY=MO,,WE",
        "FREQ=WEEKLY;BYDAY=MO,",
        "FREQ=MONTHLY;BYMONTHDAY=1,,2",
        "FREQ=YEARLY;BYMONTH=3,,4",
    ] {
        assert!(
            matches!(
                RecurrenceRule::from_rrule(text, None),
                Err(RecurrenceParseError::Malformed(_))
            ),
            "expected {text} to throw"
        );
    }
}

#[test]
fn accepts_monday_week_start() {
    let rule = parse("FREQ=WEEKLY;WKST=MO;BYDAY=TU");
    assert_eq!(
        rule,
        RecurrenceRule {
            weekdays: vec![plain(Weekday::Tuesday)],
            ..RecurrenceRule::new(Frequency::Weekly)
        }
    );
    // WKST=MO is the default and is not rendered.
    assert_eq!(rule.rrule_string(false, None), "FREQ=WEEKLY;BYDAY=TU");
}

#[test]
fn parsing_is_case_insensitive_and_accepts_the_prefix() {
    let rule = parse("  rrule:freq=weekly;interval=2;byday=mo,we;until=20261005t150000z");
    assert!(rule.frequency == Frequency::Weekly && rule.interval == 2);
    assert_eq!(
        rule.weekdays,
        vec![plain(Weekday::Monday), plain(Weekday::Wednesday)]
    );
    assert_eq!(rule.end, RuleEnd::Until(iso("2026-10-05T15:00:00Z")));
    let monthly = parse("RRULE:freq=monthly;byday=-1fr");
    assert_eq!(monthly.weekdays, vec![nth(Weekday::Friday, -1)]);
}

#[test]
fn a_parsed_rule_is_checked_for_writing_by_separating_validate() {
    let ordinal_on_a_weekly_rule = parse("FREQ=WEEKLY;BYDAY=1MO");
    assert_eq!(
        ordinal_on_a_weekly_rule.validate(),
        Err(WriteError::Invalid(
            "an ordinal weekday needs a monthly or yearly rule".to_string()
        ))
    );
    let plain_rule = parse("FREQ=WEEKLY;BYDAY=MO");
    assert_eq!(plain_rule.validate(), Ok(()));
}

#[test]
fn date_text_is_eight_digits() {
    assert_eq!(
        RecurrenceRule::date_text(&CalendarDate::new(2026, 3, 7)),
        "20260307"
    );
    assert_eq!(
        RecurrenceRule::date_text(&CalendarDate::new(2026, 12, 31)),
        "20261231"
    );
}
