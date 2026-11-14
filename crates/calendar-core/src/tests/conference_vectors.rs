use serde_json::{Value, json};

use super::{assert_no_failures, vector_cases};
use crate::UrlString;
use crate::conference_detector::ConferenceDetector;
use crate::join_url_policy::JoinUrlPolicy;
use crate::model::{ConferenceInfo, ConferenceOrigin, ConferenceProvider};

const CONFERENCE: &str = include_str!("../../../../vectors/core/conference.json");

fn text(value: &Value) -> Option<&str> {
    value.as_str()
}

fn info_json(info: &ConferenceInfo) -> Value {
    json!({"url": info.url.as_str(), "provider": info.provider.as_str(), "origin": info.origin.as_str()})
}

#[test]
fn conference_vector() {
    let cases = vector_cases(CONFERENCE);
    let mut failures = Vec::new();
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let input = &case["input"];
        let expected = &case["expected"];
        let link = text(&input["url"]).and_then(UrlString::parse);
        let actual: Value = match input["fn"].as_str().unwrap() {
            "provider" => {
                let url = link.as_ref().unwrap();
                json!({"provider": ConferenceDetector::provider_of(url).map(|p| p.as_str())})
            }
            "isAllowed" => json!({"allowed": JoinUrlPolicy::is_allowed(link.as_ref().unwrap())}),
            "detect" => {
                let found = ConferenceDetector::detect(
                    text(&input["location"]),
                    link.as_ref(),
                    text(&input["notes"]),
                );
                json!({"url": found.as_ref().map(UrlString::as_str)})
            }
            "conferences" => {
                let structured: Vec<ConferenceInfo> = input["structured"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|item| {
                        ConferenceInfo::new(
                            UrlString::parse(item["url"].as_str().unwrap()).unwrap(),
                            ConferenceProvider::from_raw_value(item["provider"].as_str().unwrap())
                                .unwrap(),
                            ConferenceOrigin::from_raw_value(item["origin"].as_str().unwrap())
                                .unwrap(),
                        )
                    })
                    .collect();
                let list = ConferenceDetector::conferences(
                    &structured,
                    text(&input["location"]),
                    link.as_ref(),
                    text(&input["notes"]),
                );
                json!({"conferences": list.iter().map(info_json).collect::<Vec<_>>()})
            }
            other => {
                failures.push(format!("{name}: unknown fn {other}"));
                continue;
            }
        };
        if &actual != expected {
            failures.push(format!(
                "{name}: input {input} gave {actual}, expected {expected}"
            ));
        }
    }
    assert_eq!(cases.len(), 400);
    assert_no_failures("core/conference", cases.len(), failures);
}
