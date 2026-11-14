//! Checks beyond the Swift tests and the vectors, against what the Swift code answers (macOS 27, `en_US`): the
//! `Character` rules of the parser, TEXT escaping, line folding, DURATION and DATE-TIME values, `String(decoding:)` of
//! invalid UTF-8, `Double(String)`, the `VTIMEZONE` text of 30 zones and the zone chosen for 100 random `VTIMEZONE`
//! definitions. `swift_probe.json` is written by `scripts/dev/ical-tables/probe-main.swift` (copy it over
//! `Sources/VectorExporter/main.swift` in a scratch checkout of the last dual-language commit (docs/rust-port/README.md; the Swift package is gone from `v2`), build with `-Xswiftc -enable-testing`, run it with an output
//! path); the cases here are a fixed sample of its output. Strings are arrays of Unicode scalar values.

use calendar_core::time::Zone;
use jiff::Timestamp;
use serde_json::Value;

use super::assert_no_failures;
use crate::component::{ICalComponent, ICalParameter, ICalProperty};
use crate::content_line::{ICalError, ICalParser, ICalSerializer};
use crate::grapheme::{characters, drop_first, prefix};
use crate::ical_text::ICalText;
use crate::ical_values::{ICalDateValue, ICalValues};
use crate::swift_number::swift_double;
use crate::time_zone_resolver::TimeZoneResolver;
use crate::vtimezone_writer::VTimeZoneWriter;

const PROBE: &str = include_str!("swift_probe.json");

fn text(value: &Value) -> String {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|c| char::from_u32(u32::try_from(c.as_u64().unwrap()).unwrap()).unwrap())
        .collect()
}

fn texts(value: &Value) -> Vec<String> {
    value.as_array().unwrap().iter().map(text).collect()
}

fn probe() -> Value {
    serde_json::from_str(PROBE).unwrap()
}

#[test]
fn characters_escaping_and_prefixes_follow_swift() {
    let mut failures = Vec::new();
    let probe = probe();
    let cases = probe["graphemes"].as_array().unwrap();
    for case in cases {
        let input = text(&case["s"]);
        let clusters: Vec<String> = characters(&input).map(str::to_string).collect();
        if clusters != texts(&case["clusters"]) {
            failures.push(format!(
                "{input:?}: clusters {clusters:?}, expected {:?}",
                texts(&case["clusters"])
            ));
        }
        let escaped = ICalText::escape(&input);
        if escaped != text(&case["escape"]) {
            failures.push(format!(
                "{input:?}: escape {escaped:?}, expected {:?}",
                text(&case["escape"])
            ));
        }
        let unescaped = ICalText::unescape(&input);
        if unescaped != text(&case["unescape"]) {
            failures.push(format!(
                "{input:?}: unescape {unescaped:?}, expected {:?}",
                text(&case["unescape"])
            ));
        }
        if prefix(&input, 3) != text(&case["prefix3"]) {
            failures.push(format!("{input:?}: prefix(3) {:?}", prefix(&input, 3)));
        }
        if drop_first(&input, 2) != text(&case["dropFirst2"]) {
            failures.push(format!(
                "{input:?}: dropFirst(2) {:?}",
                drop_first(&input, 2)
            ));
        }
    }
    assert_no_failures("swift-probe graphemes", cases.len(), failures);
}

#[test]
fn unfolding_and_content_lines_follow_swift() {
    let mut failures = Vec::new();
    let probe = probe();
    let cases = probe["lines"].as_array().unwrap();
    for case in cases {
        let input = text(&case["s"]);
        let unfolded = ICalParser::unfold(&input);
        if unfolded != texts(&case["unfold"]) {
            failures.push(format!(
                "{input:?}: unfold {unfolded:?}, expected {:?}",
                texts(&case["unfold"])
            ));
        }
        let expected = &case["parse"];
        match ICalParser::parse_line(&input) {
            Ok(property) => {
                let parameters: Vec<(String, Vec<String>)> = expected["parameters"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .map(|p| (text(&p["name"]), texts(&p["values"])))
                            .collect()
                    })
                    .unwrap_or_default();
                let actual: Vec<(String, Vec<String>)> = property
                    .parameters
                    .iter()
                    .map(|p| (p.name.clone(), p.values.clone()))
                    .collect();
                if !(expected["ok"].as_bool() == Some(true)
                    && property.name == text(&expected["name"])
                    && property.value == text(&expected["value"])
                    && actual == parameters)
                {
                    failures.push(format!(
                        "{input:?}: parse_line gave {property:?}, expected {expected}"
                    ));
                }
            }
            Err(ICalError::Malformed(message)) => {
                if !(expected["ok"].as_bool() == Some(false)
                    && message == text(&expected["message"]))
                {
                    failures.push(format!(
                        "{input:?}: parse_line failed with {message:?}, expected {expected}"
                    ));
                }
            }
        }
    }
    assert_no_failures("swift-probe lines", cases.len(), failures);
}

#[test]
fn folding_counts_scalars_like_swift() {
    let mut failures = Vec::new();
    let probe = probe();
    let cases = probe["folds"].as_array().unwrap();
    for case in cases {
        let value = text(&case["value"]);
        let first = prefix(&value, 5).to_string();
        let property = ICalProperty::new(
            "x-name",
            vec![ICalParameter::new("cn", vec![first, "q;r".to_string()])],
            value.clone(),
        );
        let actual =
            ICalSerializer::serialize(&ICalComponent::new("vevent", vec![property], vec![]));
        if actual != text(&case["serialized"]) {
            failures.push(format!(
                "{value:?}: {actual:?}, expected {:?}",
                text(&case["serialized"])
            ));
        }
    }
    assert_no_failures("swift-probe folds", cases.len(), failures);
}

#[test]
fn durations_and_date_values_follow_swift() {
    let mut failures = Vec::new();
    let probe = probe();
    let durations = probe["durations"].as_array().unwrap();
    for case in durations {
        let input = text(&case["s"]);
        let actual = ICalValues::duration(&input).map(|d| d.to_bits().to_string());
        let expected = case["d"].as_str().map(str::to_string);
        if actual != expected {
            failures.push(format!(
                "duration({input:?}): {actual:?}, expected {expected:?}"
            ));
        }
    }
    let dates = probe["dates"].as_array().unwrap();
    for case in dates {
        let input = text(&case["s"]);
        let mut parameters = Vec::new();
        if let Some(kind) = case["kind"].as_str() {
            parameters.push(ICalParameter::single("VALUE", kind));
        }
        if let Some(tzid) = case["tzid"].as_str() {
            parameters.push(ICalParameter::single("TZID", tzid));
        }
        let property = ICalProperty::new("DTSTART", parameters, input.clone());
        let actual: Vec<String> = ICalValues::date_values(&property)
            .iter()
            .map(|value| match value {
                ICalDateValue::Date(d) => format!("date {}-{}-{}", d.year, d.month, d.day),
                ICalDateValue::Utc(t) => format!("utc {:?}", t.as_second() as f64),
                ICalDateValue::Local { local: l, tzid } => format!(
                    "local {}-{}-{} {}:{}:{} {}",
                    l.date.year,
                    l.date.month,
                    l.date.day,
                    l.hour,
                    l.minute,
                    l.second,
                    tzid.clone().unwrap_or_else(|| "-".to_string())
                ),
            })
            .collect();
        let expected: Vec<String> = case["values"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        if actual != expected {
            failures.push(format!(
                "date_values({input:?}, {}, {}): {actual:?}, expected {expected:?}",
                case["kind"], case["tzid"]
            ));
        }
    }
    assert_no_failures(
        "swift-probe durations and dates",
        durations.len() + dates.len(),
        failures,
    );
}

#[test]
fn invalid_utf8_becomes_replacement_characters_like_swift() {
    let mut failures = Vec::new();
    let probe = probe();
    let cases = probe["decode"].as_array().unwrap();
    for case in cases {
        let bytes: Vec<u8> = case["bytes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| u8::try_from(b.as_u64().unwrap()).unwrap())
            .collect();
        let actual = String::from_utf8_lossy(&bytes).to_string();
        if actual != text(&case["scalars"]) {
            failures.push(format!(
                "{bytes:02x?}: {actual:?}, expected {:?}",
                text(&case["scalars"])
            ));
        }
    }
    assert_no_failures("swift-probe decode", cases.len(), failures);
    // Through the parser: a stray byte in a value does not stop the parse, and there is no Latin-1 reading.
    let parsed = ICalParser::parse(b"BEGIN:VEVENT\r\nSUMMARY:caf\xE9\r\nEND:VEVENT\r\n").unwrap();
    assert_eq!(
        parsed.property("SUMMARY").map(|p| p.value.as_str()),
        Some("caf\u{FFFD}")
    );
}

#[test]
fn double_parsing_follows_swift() {
    let mut failures = Vec::new();
    let probe = probe();
    let cases = probe["doubles"].as_array().unwrap();
    for case in cases {
        let input = text(&case["s"]);
        let actual = swift_double(&input).map(|d| {
            if d.is_nan() {
                "nan".to_string()
            } else {
                d.to_bits().to_string()
            }
        });
        let expected = case["d"].as_str().map(str::to_string);
        if actual != expected {
            failures.push(format!(
                "Double({input:?}): {actual:?}, expected {expected:?}"
            ));
        }
    }
    assert_no_failures("swift-probe doubles", cases.len(), failures);
}

#[test]
fn vtimezone_text_is_byte_for_byte_swifts() {
    let mut failures = Vec::new();
    let probe = probe();
    let cases = probe["vtz"].as_array().unwrap();
    for case in cases {
        let id = case["zone"].as_str().unwrap();
        let Some(zone) = Zone::named(id) else {
            failures.push(format!("{id}: Zone::named is None"));
            continue;
        };
        let from = Timestamp::from_second(case["from"].as_f64().unwrap() as i64).unwrap();
        let through = Timestamp::from_second(case["through"].as_f64().unwrap() as i64).unwrap();
        let actual =
            VTimeZoneWriter::component(&zone, from, through).map(|c| ICalSerializer::serialize(&c));
        let expected = case["text"].as_array().map(|_| text(&case["text"]));
        if actual != expected {
            failures.push(format!("{id}: {actual:?}, expected {expected:?}"));
        }
    }
    assert_no_failures("swift-probe vtz", cases.len(), failures);
}

#[test]
fn zones_chosen_for_random_vtimezone_definitions_follow_swift() {
    let mut failures = Vec::new();
    let probe = probe();
    let cases = probe["resolver"].as_array().unwrap();
    for case in cases {
        let calendar = ICalParser::parse_text(case["calendar"].as_str().unwrap()).unwrap();
        let tzid = case["tzid"].as_str().unwrap();
        let actual = TimeZoneResolver::from_calendar(&calendar)
            .zone_for(tzid)
            .map(|z| z.identifier().to_string());
        let expected = case["zone"].as_str().map(str::to_string);
        if actual != expected {
            failures.push(format!(
                "{tzid}: {actual:?}, expected {expected:?}\n{}",
                case["calendar"].as_str().unwrap().replace("\r\n", " | ")
            ));
        }
    }
    assert_no_failures("swift-probe resolver", cases.len(), failures);
}
