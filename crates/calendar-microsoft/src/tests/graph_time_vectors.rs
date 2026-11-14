//! Replays `vectors/microsoft/graph-time.json` (Swift `GraphTime.*` at the baseline) through the Rust `GraphTime`.

use calendar_core::all_day::CalendarDate;
use calendar_core::time::Zone;
use jiff::Timestamp;
use serde_json::Value;

use super::{assert_no_failures, vector_cases};
use crate::graph_time::GraphTime;

const GRAPH_TIME: &str = include_str!("../../../../vectors/microsoft/graph-time.json");

fn instant(milliseconds: i64) -> Timestamp {
    Timestamp::from_millisecond(milliseconds).unwrap()
}

/// `jiff::Timestamp` ends on 9999-12-30T22:00:00Z, so these two cases (the last two hours of year 9999) cannot be
/// built in Rust. They are skipped by name, and the count is pinned so no other case can slip through.
const NOT_REPRESENTABLE: [&str; 2] = ["graph-time-format-0110", "graph-time-instantText-0276"];

fn expected_instant(expected: &Value) -> Option<Timestamp> {
    expected["instant"].as_i64().map(instant)
}

fn date_of(value: &Value) -> CalendarDate {
    CalendarDate::new(
        value["year"].as_i64().unwrap(),
        value["month"].as_i64().unwrap(),
        value["day"].as_i64().unwrap(),
    )
}

#[test]
fn graph_time_vector() {
    let cases = vector_cases(GRAPH_TIME);
    let mut failures = Vec::new();
    let mut skipped = 0;
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        if NOT_REPRESENTABLE.contains(&name) {
            skipped += 1;
            continue;
        }
        let input = &case["input"];
        let expected = &case["expected"];
        let outcome: Result<(), String> = match input["fn"].as_str().unwrap() {
            "parse" => {
                let text = input["text"].as_str().unwrap();
                let id = input["zone"].as_str().unwrap();
                match Zone::named(id) {
                    None => Err(format!("Zone::named({id:?}) is None")),
                    Some(zone) => {
                        let actual = GraphTime::parse(text, &zone);
                        let want = expected_instant(expected);
                        if actual == want {
                            Ok(())
                        } else {
                            Err(format!(
                                "parse {text:?} in {id} gave {actual:?}, expected {want:?}"
                            ))
                        }
                    }
                }
            }
            "parseInstant" => {
                let text = input["text"].as_str().unwrap();
                let actual = GraphTime::parse_instant(text);
                let want = expected_instant(expected);
                if actual == want {
                    Ok(())
                } else {
                    Err(format!(
                        "parseInstant {text:?} gave {actual:?}, expected {want:?}"
                    ))
                }
            }
            "format" => {
                let id = input["zone"].as_str().unwrap();
                match Zone::named(id) {
                    None => Err(format!("Zone::named({id:?}) is None")),
                    Some(zone) => {
                        let actual =
                            GraphTime::format(instant(input["instant"].as_i64().unwrap()), &zone);
                        let want = expected["text"].as_str().unwrap();
                        if actual == want {
                            Ok(())
                        } else {
                            Err(format!("format gave {actual:?}, expected {want:?}"))
                        }
                    }
                }
            }
            "date" => {
                let text = input["text"].as_str().unwrap();
                let actual = GraphTime::date(text);
                let want = expected["date"]
                    .as_object()
                    .map(|_| date_of(&expected["date"]));
                if actual == want {
                    Ok(())
                } else {
                    Err(format!("date {text:?} gave {actual:?}, expected {want:?}"))
                }
            }
            "midnight" | "dateText" => {
                let date = date_of(&input["date"]);
                let (actual, label) = if input["fn"] == "midnight" {
                    (GraphTime::midnight(&date), "midnight")
                } else {
                    (GraphTime::date_text(&date), "dateText")
                };
                let want = expected["text"].as_str().unwrap();
                if actual == want {
                    Ok(())
                } else {
                    Err(format!(
                        "{label} {date:?} gave {actual:?}, expected {want:?}"
                    ))
                }
            }
            "instantText" => {
                let actual = GraphTime::instant_text(instant(input["instant"].as_i64().unwrap()));
                let want = expected["text"].as_str().unwrap();
                if actual == want {
                    Ok(())
                } else {
                    Err(format!("instantText gave {actual:?}, expected {want:?}"))
                }
            }
            other => Err(format!("unknown fn {other}")),
        };
        if let Err(message) = outcome {
            failures.push(format!("{name}: {message}"));
        }
    }
    assert_eq!(cases.len(), 300);
    assert_eq!(skipped, NOT_REPRESENTABLE.len());
    assert_no_failures("microsoft/graph-time", cases.len(), failures);
}
