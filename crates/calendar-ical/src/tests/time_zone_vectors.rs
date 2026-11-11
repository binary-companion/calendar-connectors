//! Replays `vectors/ical/timezones.json` (`TimeZoneResolver.zone(for:)`, `date(_:floating:)`, `displayZone`).

use calendar_core::all_day::CalendarDate;
use calendar_core::time::Zone;
use jiff::Timestamp;
use serde_json::Value;

use super::{assert_no_failures, vector_cases};
use crate::content_line::ICalParser;
use crate::ical_values::{ICalDateValue, LocalDateTime};
use crate::time_zone_resolver::TimeZoneResolver;

const TIME_ZONES: &str = include_str!("../../../../vectors/ical/timezones.json");

fn resolver_for(calendar: &Value) -> TimeZoneResolver {
    match calendar.as_str() {
        None => TimeZoneResolver::new(),
        Some(text) => TimeZoneResolver::from_calendar(&ICalParser::parse_text(text).unwrap()),
    }
}

fn day_of(value: &Value) -> CalendarDate {
    CalendarDate::new(
        value["year"].as_i64().unwrap(),
        value["month"].as_i64().unwrap(),
        value["day"].as_i64().unwrap(),
    )
}

fn value_of(value: &Value) -> ICalDateValue {
    match value["kind"].as_str().unwrap() {
        "date" => ICalDateValue::Date(day_of(&value["date"])),
        "utc" => ICalDateValue::Utc(
            Timestamp::from_millisecond(value["instant"].as_i64().unwrap()).unwrap(),
        ),
        "local" => {
            let local = &value["local"];
            ICalDateValue::Local {
                local: LocalDateTime::new(
                    day_of(&local["date"]),
                    local["hour"].as_i64().unwrap(),
                    local["minute"].as_i64().unwrap(),
                    local["second"].as_i64().unwrap(),
                ),
                tzid: value["tzid"].as_str().map(str::to_string),
            }
        }
        other => panic!("unknown value kind {other}"),
    }
}

#[test]
fn time_zones_vector() {
    let cases = vector_cases(TIME_ZONES);
    let mut failures = Vec::new();
    let (mut zones, mut dates, mut displays) = (0, 0, 0);
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let input = &case["input"];
        let expected = &case["expected"];
        let resolver = resolver_for(&input["calendar"]);
        match input["fn"].as_str().unwrap() {
            "zone" => {
                zones += 1;
                let tzid = input["tzid"].as_str().unwrap();
                let actual = resolver.zone_for(tzid).map(|z| z.identifier().to_string());
                let want = expected["zone"].as_str().map(str::to_string);
                if actual != want {
                    failures.push(format!(
                        "{name}: zone({tzid:?}) gave {actual:?}, expected {want:?}"
                    ));
                }
            }
            "date" => {
                dates += 1;
                let floating = Zone::named(input["floating"].as_str().unwrap()).unwrap();
                let value = value_of(&input["value"]);
                let actual = resolver.date(&value, &floating).map(|t| t.as_millisecond());
                let want = expected["instant"].as_i64();
                if actual != want {
                    failures.push(format!(
                        "{name}: date({value:?}) gave {actual:?}, expected {want:?}"
                    ));
                }
            }
            "displayZone" => {
                displays += 1;
                let fallback = Zone::named(input["fallback"].as_str().unwrap()).unwrap();
                let value = value_of(&input["value"]);
                let actual = resolver
                    .display_zone(&value, &fallback)
                    .identifier()
                    .to_string();
                let want = expected["zone"].as_str().unwrap();
                if actual != want {
                    failures.push(format!(
                        "{name}: displayZone({value:?}) gave {actual:?}, expected {want:?}"
                    ));
                }
            }
            other => failures.push(format!("{name}: unknown fn {other}")),
        }
    }
    assert_eq!(cases.len(), 150);
    assert_eq!((zones, dates, displays), (80, 50, 20));
    assert_no_failures("ical/timezones", cases.len(), failures);
}
