use jiff::Timestamp;
use serde_json::Value;

use super::support::zone_or_none;
use super::{assert_no_failures, vector_cases};
use crate::all_day::{AllDay, CalendarDate};

const ALL_DAY: &str = include_str!("../../../../vectors/zones/all-day.json");

fn date(value: &Value) -> CalendarDate {
    CalendarDate::new(
        value["year"].as_i64().unwrap(),
        value["month"].as_i64().unwrap(),
        value["day"].as_i64().unwrap(),
    )
}

fn date_json(date: &CalendarDate) -> Value {
    serde_json::json!({"year": date.year, "month": date.month, "day": date.day})
}

fn instant(ms: i64) -> Timestamp {
    Timestamp::from_millisecond(ms).unwrap()
}

fn ms(value: &Value) -> Timestamp {
    instant(value.as_i64().unwrap())
}

#[test]
fn all_day_vector() {
    let cases = vector_cases(ALL_DAY);
    let mut failures = Vec::new();
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let input = &case["input"];
        let expected = &case["expected"];
        let function = input["fn"].as_str().unwrap();
        if function == "adding" {
            let actual =
                date_json(&date(&input["date"]).adding_days(input["days"].as_i64().unwrap()));
            if actual != expected["date"] {
                failures.push(format!(
                    "{name}: adding gave {actual}, expected {}",
                    expected["date"]
                ));
            }
            continue;
        }
        let zone_id = input["zone"].as_str().unwrap();
        let Some(zone) = zone_or_none(zone_id) else {
            failures.push(format!("{name}: Zone::named({zone_id:?}) is None"));
            continue;
        };
        let (actual, want): (Value, &Value) = match function {
            "startOfDay" => (
                serde_json::json!({"instant": AllDay::start_of_day(&date(&input["date"]), &zone).map(|t| t.as_millisecond())}),
                expected,
            ),
            "dateOf" => (
                serde_json::json!({"date": date_json(&AllDay::date_of(ms(&input["instant"]), &zone))}),
                expected,
            ),
            "canonical" => (
                serde_json::json!({"range": AllDay::canonical(&date(&input["first"]), &date(&input["endExclusive"]), &zone)
                    .map(|(start, end)| serde_json::json!({"start": start.as_millisecond(), "end": end.as_millisecond()}))}),
                expected,
            ),
            "dates" => {
                let (first, end_exclusive) =
                    AllDay::dates(ms(&input["start"]), ms(&input["end"]), &zone);
                (
                    serde_json::json!({"first": date_json(&first), "endExclusive": date_json(&end_exclusive)}),
                    expected,
                )
            }
            other => {
                failures.push(format!("{name}: unknown fn {other}"));
                continue;
            }
        };
        if &actual != want {
            failures.push(format!(
                "{name}: {function} in {zone_id} gave {actual}, expected {want}"
            ));
        }
    }
    assert_eq!(cases.len(), 500);
    assert_no_failures("zones/all-day", cases.len(), failures);
}
