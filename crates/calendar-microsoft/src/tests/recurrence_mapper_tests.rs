//! Port of `RecurrenceMapperTests.swift`.

use calendar_core::all_day::CalendarDate;
use calendar_core::recurrence::rule::{
    Frequency, RecurrenceRule, RuleEnd, UnrecognizedPart, Weekday, WeekdayOccurrence,
};
use calendar_core::time::Zone;
use calendar_core::write::types::{EventField, WriteError};
use jiff::Timestamp;
use serde_json::{Map, Value, json};

use crate::dtos::GraphRecurrenceDTO;
use crate::graph_time::GraphTime;
use crate::recurrence_mapper::GraphRecurrenceMapper;

fn la() -> Zone {
    Zone::named("America/Los_Angeles").unwrap()
}

/// A Friday.
fn sept25() -> CalendarDate {
    CalendarDate::new(2026, 9, 25)
}

fn dto(pattern: Value, range: Value) -> GraphRecurrenceDTO {
    serde_json::from_value(json!({"pattern": pattern, "range": range})).unwrap()
}

fn no_end() -> Value {
    json!({"type": "noEnd", "startDate": "2026-09-25"})
}

fn day(weekday: Weekday) -> WeekdayOccurrence {
    WeekdayOccurrence::new(weekday, None)
}

fn nth(weekday: Weekday, ordinal: i64) -> WeekdayOccurrence {
    WeekdayOccurrence::new(weekday, Some(ordinal))
}

fn object(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
}

fn unsupported() -> WriteError {
    WriteError::Unsupported {
        fields: [EventField::Recurrence].into(),
    }
}

#[test]
fn reads_every_pattern_type() {
    let daily = GraphRecurrenceMapper::rule(
        &dto(json!({"type": "daily", "interval": 2}), no_end()),
        &la(),
    );
    assert_eq!(
        daily,
        Some(RecurrenceRule {
            interval: 2,
            ..RecurrenceRule::new(Frequency::Daily)
        })
    );

    let weekly = GraphRecurrenceMapper::rule(
        &dto(
            json!({"type": "weekly", "interval": 1, "daysOfWeek": ["monday", "friday"], "firstDayOfWeek": "sunday"}),
            no_end(),
        ),
        &la(),
    );
    assert_eq!(
        weekly,
        Some(RecurrenceRule {
            weekdays: vec![day(Weekday::Monday), day(Weekday::Friday)],
            week_start: Weekday::Sunday,
            ..RecurrenceRule::new(Frequency::Weekly)
        })
    );

    let monthly = GraphRecurrenceMapper::rule(
        &dto(
            json!({"type": "absoluteMonthly", "interval": 1, "dayOfMonth": 15}),
            no_end(),
        ),
        &la(),
    );
    assert_eq!(
        monthly,
        Some(RecurrenceRule {
            month_days: vec![15],
            ..RecurrenceRule::new(Frequency::Monthly)
        })
    );

    let relative = GraphRecurrenceMapper::rule(
        &dto(
            json!({"type": "relativeMonthly", "interval": 1, "daysOfWeek": ["tuesday"], "index": "last"}),
            no_end(),
        ),
        &la(),
    );
    assert_eq!(
        relative,
        Some(RecurrenceRule {
            weekdays: vec![nth(Weekday::Tuesday, -1)],
            ..RecurrenceRule::new(Frequency::Monthly)
        })
    );

    let yearly = GraphRecurrenceMapper::rule(
        &dto(
            json!({"type": "absoluteYearly", "interval": 1, "month": 3, "dayOfMonth": 9}),
            no_end(),
        ),
        &la(),
    );
    assert_eq!(
        yearly,
        Some(RecurrenceRule {
            month_days: vec![9],
            months: vec![3],
            ..RecurrenceRule::new(Frequency::Yearly)
        })
    );

    let relative_yearly = GraphRecurrenceMapper::rule(
        &dto(
            json!({"type": "relativeYearly", "interval": 1, "month": 11, "daysOfWeek": ["thursday"], "index": "fourth"}),
            no_end(),
        ),
        &la(),
    );
    assert_eq!(
        relative_yearly,
        Some(RecurrenceRule {
            weekdays: vec![nth(Weekday::Thursday, 4)],
            months: vec![11],
            ..RecurrenceRule::new(Frequency::Yearly)
        })
    );
}

#[test]
fn reads_ranges() {
    let count = GraphRecurrenceMapper::rule(
        &dto(
            json!({"type": "daily", "interval": 1}),
            json!({"type": "numbered", "startDate": "2026-09-25", "numberOfOccurrences": 10}),
        ),
        &la(),
    );
    assert_eq!(count.map(|rule| rule.end), Some(RuleEnd::Count(10)));

    let until = GraphRecurrenceMapper::rule(
        &dto(
            json!({"type": "daily", "interval": 1}),
            json!({"type": "endDate", "startDate": "2026-09-25", "endDate": "2026-10-01", "recurrenceTimeZone": "Pacific Standard Time"}),
        ),
        &Zone::named("GMT").unwrap(),
    );
    // The end of Oct 1 in Pacific time is 2026-10-02T06:59:59Z.
    assert_eq!(
        until.map(|rule| rule.end),
        Some(RuleEnd::Until(
            Timestamp::from_second(1_790_924_399).unwrap()
        ))
    );
    assert_eq!(
        GraphRecurrenceMapper::rule(
            &dto(json!({"type": "daily", "interval": 1}), no_end()),
            &la()
        )
        .map(|rule| rule.end),
        Some(RuleEnd::Never)
    );
}

#[test]
fn an_unknown_pattern_reads_as_nil() {
    assert_eq!(
        GraphRecurrenceMapper::rule(&dto(json!({"type": "lunar"}), no_end()), &la()),
        None
    );
}

fn written_pattern(rule: &RecurrenceRule) -> Map<String, Value> {
    object(GraphRecurrenceMapper::recurrence(rule, &sept25(), &la()).unwrap()["pattern"].clone())
}

#[test]
fn writes_every_pattern_type() {
    let daily = written_pattern(&RecurrenceRule {
        interval: 3,
        ..RecurrenceRule::new(Frequency::Daily)
    });
    assert!(
        daily["type"] == "daily" && daily["interval"] == 3 && daily["firstDayOfWeek"] == "monday"
    );

    let weekly = written_pattern(&RecurrenceRule {
        weekdays: vec![day(Weekday::Monday), day(Weekday::Wednesday)],
        ..RecurrenceRule::new(Frequency::Weekly)
    });
    assert!(weekly["type"] == "weekly" && weekly["daysOfWeek"] == json!(["monday", "wednesday"]));

    // No BYDAY: the start's weekday.
    let weekly_default = written_pattern(&RecurrenceRule::new(Frequency::Weekly));
    assert_eq!(weekly_default["daysOfWeek"], json!(["friday"]));

    let monthly = written_pattern(&RecurrenceRule::new(Frequency::Monthly));
    assert!(monthly["type"] == "absoluteMonthly" && monthly["dayOfMonth"] == 25);

    let relative = written_pattern(&RecurrenceRule {
        weekdays: vec![nth(Weekday::Friday, -1)],
        ..RecurrenceRule::new(Frequency::Monthly)
    });
    assert!(
        relative["type"] == "relativeMonthly"
            && relative["index"] == "last"
            && relative["daysOfWeek"] == json!(["friday"])
    );

    let yearly = written_pattern(&RecurrenceRule::new(Frequency::Yearly));
    assert!(
        yearly["type"] == "absoluteYearly" && yearly["month"] == 9 && yearly["dayOfMonth"] == 25
    );

    let relative_yearly = written_pattern(&RecurrenceRule {
        weekdays: vec![nth(Weekday::Thursday, 4)],
        months: vec![11],
        ..RecurrenceRule::new(Frequency::Yearly)
    });
    assert!(
        relative_yearly["type"] == "relativeYearly"
            && relative_yearly["index"] == "fourth"
            && relative_yearly["month"] == 11
    );
}

fn written_range(end: RuleEnd) -> Map<String, Value> {
    let rule = RecurrenceRule {
        end,
        ..RecurrenceRule::new(Frequency::Daily)
    };
    object(GraphRecurrenceMapper::recurrence(&rule, &sept25(), &la()).unwrap()["range"].clone())
}

#[test]
fn writes_ranges() {
    let never = written_range(RuleEnd::Never);
    assert!(
        never["type"] == "noEnd"
            && never["startDate"] == "2026-09-25"
            && never["recurrenceTimeZone"] == "Pacific Standard Time"
    );
    let count = written_range(RuleEnd::Count(4));
    assert!(count["type"] == "numbered" && count["numberOfOccurrences"] == 4);
    // The end of Oct 1 in Pacific time.
    let until = written_range(RuleEnd::Until(
        Timestamp::from_second(1_790_924_399).unwrap(),
    ));
    assert!(until["type"] == "endDate" && until["endDate"] == "2026-10-01");
}

#[test]
fn a_zone_without_a_windows_name_is_written_as_utc() {
    let fakaofo = Zone::named("Pacific/Fakaofo").unwrap();
    let recurrence = GraphRecurrenceMapper::recurrence(
        &RecurrenceRule::new(Frequency::Daily),
        &sept25(),
        &fakaofo,
    )
    .unwrap();
    assert_eq!(recurrence["range"]["recurrenceTimeZone"], "UTC");
}

#[test]
fn refuses_what_graph_cannot_express() {
    let cases: Vec<(&str, RecurrenceRule)> = vec![
        // several month days
        (
            "several month days",
            RecurrenceRule {
                month_days: vec![1, 15],
                ..RecurrenceRule::new(Frequency::Monthly)
            },
        ),
        // last day of the month
        (
            "last day of the month",
            RecurrenceRule {
                month_days: vec![-1],
                ..RecurrenceRule::new(Frequency::Monthly)
            },
        ),
        // no fifth
        (
            "no fifth",
            RecurrenceRule {
                weekdays: vec![nth(Weekday::Friday, 5)],
                ..RecurrenceRule::new(Frequency::Monthly)
            },
        ),
        // second to last
        (
            "second to last",
            RecurrenceRule {
                weekdays: vec![nth(Weekday::Friday, -2)],
                ..RecurrenceRule::new(Frequency::Monthly)
            },
        ),
        // two indexes
        (
            "two indexes",
            RecurrenceRule {
                weekdays: vec![nth(Weekday::Monday, 1), nth(Weekday::Friday, 2)],
                ..RecurrenceRule::new(Frequency::Monthly)
            },
        ),
        (
            "two months",
            RecurrenceRule {
                months: vec![3, 9],
                ..RecurrenceRule::new(Frequency::Yearly)
            },
        ),
        (
            "weekly from sunday",
            RecurrenceRule {
                week_start: Weekday::Sunday,
                ..RecurrenceRule::new(Frequency::Weekly)
            },
        ),
        ("hourly", RecurrenceRule::new(Frequency::Hourly)),
        (
            "unrecognized part",
            RecurrenceRule {
                unrecognized_parts: vec![UnrecognizedPart::new("X-FOO", "1")],
                ..RecurrenceRule::new(Frequency::Daily)
            },
        ),
    ];
    for (label, rule) in cases {
        assert_eq!(
            GraphRecurrenceMapper::recurrence(&rule, &sept25(), &la()),
            Err(unsupported()),
            "{label}: {rule:?}"
        );
    }
}

#[test]
fn round_trips_through_graphs_shape() {
    let rules = [
        RecurrenceRule {
            interval: 2,
            end: RuleEnd::Count(5),
            ..RecurrenceRule::new(Frequency::Daily)
        },
        RecurrenceRule {
            weekdays: vec![day(Weekday::Tuesday), day(Weekday::Thursday)],
            ..RecurrenceRule::new(Frequency::Weekly)
        },
        RecurrenceRule {
            weekdays: vec![nth(Weekday::Monday, 2)],
            ..RecurrenceRule::new(Frequency::Monthly)
        },
        RecurrenceRule {
            month_days: vec![4],
            months: vec![7],
            ..RecurrenceRule::new(Frequency::Yearly)
        },
    ];
    for rule in rules {
        let json = GraphRecurrenceMapper::recurrence(&rule, &sept25(), &la()).unwrap();
        let decoded: GraphRecurrenceDTO = serde_json::from_value(Value::Object(json)).unwrap();
        assert_eq!(
            GraphRecurrenceMapper::rule(&decoded, &la()),
            Some(rule.clone()),
            "{rule:?}"
        );
    }
}

#[test]
fn truncates_and_restarts_a_range() {
    let original = object(json!({
        "pattern": {"type": "daily", "interval": 1},
        "range": {"type": "numbered", "startDate": "2026-09-01", "numberOfOccurrences": 10, "recurrenceTimeZone": "UTC"},
    }));
    assert_eq!(GraphRecurrenceMapper::occurrence_count(&original), Some(10));
    let cut = GraphRecurrenceMapper::truncated(&original, &CalendarDate::new(2026, 9, 5));
    let range = object(cut["range"].clone());
    assert!(range["type"] == "endDate" && range["endDate"] == "2026-09-04");
    assert!(!range.contains_key("numberOfOccurrences") && range["startDate"] == "2026-09-01");
    assert_eq!(GraphRecurrenceMapper::occurrence_count(&cut), None);

    let next = GraphRecurrenceMapper::restarted(&original, &CalendarDate::new(2026, 9, 5), Some(6));
    let next_range = object(next["range"].clone());
    assert!(
        next_range["startDate"] == "2026-09-05"
            && next_range["numberOfOccurrences"] == 6
            && next_range["type"] == "numbered"
    );
}

#[test]
fn numbered_position_uses_scheduled_slots_across_patterns() {
    fn remaining(pattern: Value, start: &str, split: &str, count: i64) -> i64 {
        let recurrence = object(json!({
            "pattern": pattern,
            "range": {"type": "numbered", "startDate": start, "numberOfOccurrences": count},
        }));
        GraphRecurrenceMapper::remaining_count(&recurrence, &GraphTime::date(split).unwrap())
            .unwrap()
    }
    assert_eq!(
        remaining(
            json!({"type": "weekly", "interval": 2, "daysOfWeek": ["monday", "friday"], "firstDayOfWeek": "monday"}),
            "2026-09-25",
            "2026-10-19",
            8
        ),
        5
    );
    assert_eq!(
        remaining(
            json!({"type": "relativeMonthly", "interval": 1, "daysOfWeek": ["tuesday"], "index": "last"}),
            "2026-09-29",
            "2026-11-24",
            8
        ),
        6
    );
    assert_eq!(
        remaining(
            json!({"type": "daily", "interval": 1}),
            "2026-03-07",
            "2026-03-10",
            5
        ),
        2
    );
    assert_eq!(
        remaining(
            json!({"type": "absoluteMonthly", "interval": 1, "dayOfMonth": 31}),
            "2026-01-31",
            "2026-03-31",
            8
        ),
        7
    );
    assert_eq!(
        remaining(
            json!({"type": "absoluteYearly", "interval": 1, "month": 2, "dayOfMonth": 29}),
            "2024-02-29",
            "2028-02-29",
            8
        ),
        7
    );
    assert_eq!(
        remaining(
            json!({"type": "relativeYearly", "interval": 1, "month": 11, "daysOfWeek": ["monday"], "index": "last"}),
            "2026-11-30",
            "2027-11-29",
            8
        ),
        7
    );
}
