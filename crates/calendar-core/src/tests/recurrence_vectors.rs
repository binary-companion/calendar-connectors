use jiff::Timestamp;
use serde_json::{Value, json};

use super::{assert_no_failures, vector_cases};
use crate::all_day::CalendarDate;
use crate::recurrence::expander::WallClock;
use crate::recurrence::rule::{RecurrenceRule, RuleEnd};
use crate::recurrence::set::RecurrenceSet;
use crate::time::{DateInterval, Zone};

const WALL_CLOCK: &str = include_str!("../../../../vectors/zones/wall-clock.json");
const RULE_PARSE: &str = include_str!("../../../../vectors/recurrence/rule-parse.json");
const RULE_INSTANCES: &str = include_str!("../../../../vectors/recurrence/rule-instances.json");
const SET_OCCURRENCES: &str = include_str!("../../../../vectors/recurrence/set-occurrences.json");

fn instant(ms: i64) -> Timestamp {
    Timestamp::from_millisecond(ms).unwrap()
}

fn ms(value: &Value) -> Timestamp {
    instant(value.as_i64().unwrap())
}

fn zone(value: &Value) -> Zone {
    Zone::named(value.as_str().unwrap()).unwrap()
}

fn instants(list: &[Timestamp]) -> Vec<i64> {
    list.iter().map(|t| t.as_millisecond()).collect()
}

#[test]
fn wall_clock_vector() {
    let cases = vector_cases(WALL_CLOCK);
    let mut failures = Vec::new();
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let input = &case["input"];
        let get = |v: &Value| v.as_i64().unwrap();
        let date = CalendarDate::new(
            get(&input["date"]["year"]),
            get(&input["date"]["month"]),
            get(&input["date"]["day"]),
        );
        let got = WallClock::date(
            &date,
            get(&input["hour"]),
            get(&input["minute"]),
            get(&input["second"]),
            &zone(&input["zone"]),
        )
        .map(|t| t.as_millisecond());
        let expected = case["expected"]["instant"].as_i64();
        if got != expected {
            failures.push(format!("{name}: got {got:?}, expected {expected:?}"));
        }
    }
    assert_eq!(cases.len(), 600);
    assert_no_failures("zones/wall-clock", cases.len(), failures);
}

fn weekday_json(rule: &RecurrenceRule) -> Vec<Value> {
    rule.weekdays
        .iter()
        .map(|w| json!({"weekday": w.weekday.as_str(), "ordinal": w.ordinal}))
        .collect()
}

#[test]
fn rule_parse_vector() {
    let cases = vector_cases(RULE_PARSE);
    let mut failures = Vec::new();
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let input = &case["input"];
        let zone = input["zone"].as_str().map(|id| Zone::named(id).unwrap());
        let expected = &case["expected"];
        let actual = match RecurrenceRule::from_rrule(
            input["rrule"].as_str().unwrap(),
            zone.as_ref(),
        ) {
            Err(crate::recurrence::rule::RecurrenceParseError::Malformed(message)) => {
                json!({"ok": false, "message": message})
            }
            Ok(r) => {
                let end = match r.end {
                    RuleEnd::Never => json!({"kind": "never"}),
                    RuleEnd::Count(n) => json!({"kind": "count", "count": n}),
                    RuleEnd::Until(t) => json!({"kind": "until", "instant": t.as_millisecond()}),
                };
                json!({
                    "ok": true, "frequency": r.frequency.as_str(), "interval": r.interval,
                    "weekStart": r.week_start.as_str(), "weekdays": weekday_json(&r),
                    "monthDays": r.month_days, "months": r.months, "yearDays": r.year_days,
                    "weekNumbers": r.week_numbers, "setPositions": r.set_positions, "hours": r.hours,
                    "minutes": r.minutes, "seconds": r.seconds, "end": end,
                    "unrecognizedParts": r.unrecognized_parts.iter()
                        .map(|p| json!({"name": p.name, "value": p.value})).collect::<Vec<_>>(),
                    "isWritableSubset": r.is_writable_subset(),
                    "timedText": r.rrule_string(false, None),
                    "allDayText": r.rrule_string(true, zone.as_ref()),
                })
            }
        };
        if &actual != expected {
            failures.push(format!(
                "{name}: {} gave {actual}, expected {expected}",
                input["rrule"]
            ));
        }
    }
    assert_eq!(cases.len(), 400);
    assert_no_failures("recurrence/rule-parse", cases.len(), failures);
}

#[test]
fn rule_instances_vector() {
    let cases = vector_cases(RULE_INSTANCES);
    let mut failures = Vec::new();
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let input = &case["input"];
        let zone = zone(&input["zone"]);
        let rule = match RecurrenceRule::from_rrule(input["rrule"].as_str().unwrap(), Some(&zone)) {
            Ok(rule) => rule,
            Err(error) => {
                failures.push(format!(
                    "{name}: {} did not parse: {error:?}",
                    input["rrule"]
                ));
                continue;
            }
        };
        let result = rule.instances(
            ms(&input["anchor"]),
            &zone,
            input["isAllDay"].as_bool().unwrap(),
            ms(&input["before"]),
            input["limit"].as_i64().unwrap(),
            input["skipTo"].as_i64().map(instant),
        );
        let actual = json!({"starts": instants(&result.starts), "truncated": result.truncated});
        if actual != case["expected"] {
            failures.push(format!(
                "{name}: {} in {}: got {} starts (truncated {}), expected {} (truncated {})",
                input["rrule"],
                input["zone"],
                result.starts.len(),
                result.truncated,
                case["expected"]["starts"].as_array().map_or(0, Vec::len),
                case["expected"]["truncated"]
            ));
        }
    }
    assert_eq!(cases.len(), 2000);
    assert_no_failures("recurrence/rule-instances", cases.len(), failures);
}

fn build_set(input: &Value, zone: &Zone) -> Option<RecurrenceSet> {
    let rules: Option<Vec<RecurrenceRule>> = input["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|text| RecurrenceRule::from_rrule(text.as_str().unwrap(), Some(zone)).ok())
        .collect();
    let dates = |key: &str| -> Option<Vec<Timestamp>> {
        input
            .get(key)
            .and_then(Value::as_array)
            .map(|list| list.iter().map(ms).collect())
    };
    let is_occurrences = input["fn"] == "occurrences";
    Some(RecurrenceSet {
        rules: rules?,
        extra_dates: if is_occurrences {
            dates("rdates")
        } else {
            Some(vec![])
        },
        excluded_dates: if is_occurrences {
            dates("exdates")
        } else {
            Some(vec![])
        },
        unparsed: input
            .get("unparsed")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .map(|l| l.as_str().unwrap().to_string())
                    .collect()
            })
            .unwrap_or_default(),
    })
}

#[test]
fn set_occurrences_vector() {
    let cases = vector_cases(SET_OCCURRENCES);
    let mut failures = Vec::new();
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let input = &case["input"];
        let zone = zone(&input["zone"]);
        let Some(set) = build_set(input, &zone) else {
            failures.push(format!("{name}: a rule did not parse"));
            continue;
        };
        let is_all_day = input["isAllDay"].as_bool().unwrap();
        let actual = match input["fn"].as_str().unwrap() {
            "occurrences" => {
                let result = set.occurrences(
                    ms(&input["anchor"]),
                    input["durationSeconds"].as_f64().unwrap(),
                    &zone,
                    is_all_day,
                    &DateInterval::new(ms(&input["windowStart"]), ms(&input["windowEnd"])),
                    input["limit"].as_i64().unwrap(),
                );
                json!({
                    "starts": instants(&result.starts), "truncated": result.truncated,
                    "hasUnreadableRule": result.has_unreadable_rule,
                })
            }
            "ruleInstanceCount" => json!({
                "count": set.rule_instance_count(ms(&input["anchor"]), &zone, is_all_day, ms(&input["before"])),
            }),
            other => {
                failures.push(format!("{name}: unknown fn {other}"));
                continue;
            }
        };
        if actual != case["expected"] {
            failures.push(format!(
                "{name}: got {actual}, expected {}",
                case["expected"]
            ));
        }
    }
    assert_eq!(cases.len(), 300);
    assert_no_failures("recurrence/set-occurrences", cases.len(), failures);
}
