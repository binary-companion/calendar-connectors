//! Replays `vectors/ical/content-lines.json` (`ICalParser.unfold`, `parseLine`, `ICalSerializer.contentLine`).

use serde_json::Value;

use super::{assert_no_failures, vector_cases};
use crate::component::{ICalParameter, ICalProperty};
use crate::content_line::{ICalError, ICalParser, ICalSerializer};

const CONTENT_LINES: &str = include_str!("../../../../vectors/ical/content-lines.json");

fn parameters_of(value: &Value) -> Vec<ICalParameter> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            ICalParameter::new(
                p["name"].as_str().unwrap(),
                p["values"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect(),
            )
        })
        .collect()
}

#[test]
fn content_lines_vector() {
    let cases = vector_cases(CONTENT_LINES);
    let mut failures = Vec::new();
    let (mut unfold, mut parse_line, mut content_line) = (0, 0, 0);
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let input = &case["input"];
        let expected = &case["expected"];
        match input["fn"].as_str().unwrap() {
            "unfold" => {
                unfold += 1;
                let text = input["text"].as_str().unwrap();
                let actual = ICalParser::unfold(text);
                let want: Vec<String> = expected["lines"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|l| l.as_str().unwrap().to_string())
                    .collect();
                if actual != want {
                    failures.push(format!(
                        "{name}: unfold({text:?}) gave {actual:?}, expected {want:?}"
                    ));
                }
            }
            "parseLine" => {
                parse_line += 1;
                let line = input["line"].as_str().unwrap();
                match (
                    ICalParser::parse_line(line),
                    expected["ok"].as_bool().unwrap(),
                ) {
                    (Ok(property), true) => {
                        let want = ICalProperty::new(
                            expected["name"].as_str().unwrap(),
                            parameters_of(&expected["parameters"]),
                            expected["value"].as_str().unwrap(),
                        );
                        if property != want {
                            failures.push(format!(
                                "{name}: parseLine({line:?}) gave {property:?}, expected {want:?}"
                            ));
                        }
                    }
                    (Err(ICalError::Malformed(message)), false) => {
                        let want = expected["message"].as_str().unwrap();
                        if message != want {
                            failures.push(format!(
                                "{name}: parseLine({line:?}) message {message:?}, expected {want:?}"
                            ));
                        }
                    }
                    (actual, ok) => {
                        failures.push(format!(
                            "{name}: parseLine({line:?}) gave {actual:?}, expected ok={ok}"
                        ));
                    }
                }
            }
            "contentLine" => {
                content_line += 1;
                let property = ICalProperty::new(
                    input["name"].as_str().unwrap(),
                    parameters_of(&input["parameters"]),
                    input["value"].as_str().unwrap(),
                );
                let actual = ICalSerializer::content_line(&property);
                let want = expected["text"].as_str().unwrap();
                if actual != want {
                    failures.push(format!(
                        "{name}: contentLine gave {actual:?}, expected {want:?}"
                    ));
                }
            }
            other => failures.push(format!("{name}: unknown fn {other}")),
        }
    }
    assert_eq!(cases.len(), 300);
    assert_eq!((unfold, parse_line, content_line), (100, 100, 100));
    assert_no_failures("ical/content-lines", cases.len(), failures);
}
