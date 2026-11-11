//! Port of `ContentLineTests.swift`.

use crate::component::{ICalComponent, ICalProperty};
use crate::content_line::{ICalError, ICalParser, ICalSerializer};
use crate::ical_text::ICalText;

fn sample() -> String {
    r#"BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Example//EN
BEGIN:VEVENT
UID:abc-123
SUMMARY:Planning\, part 2\nwith notes
ATTENDEE;CN="Doe, Jane";ROLE=REQ-PARTICIPANT;DELEGATED-FROM="mailto:a@x.test","mailto:b@x.test":mailto:jane@x.test
URL:https://x.test:8443/a;b
X-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC
X-CUSTOM;X-FLAG=1:kept
BEGIN:VALARM
ACTION:DISPLAY
TRIGGER:-PT15M
END:VALARM
END:VEVENT
END:VCALENDAR
"#
    .replace('\n', "\r\n")
}

fn first_event(root: &ICalComponent) -> &ICalComponent {
    root.components_named("VEVENT")[0]
}

#[test]
fn parses_nested_components() {
    let root = ICalParser::parse_text(&sample()).unwrap();
    assert_eq!(root.name, "VCALENDAR");
    let event = first_event(&root);
    assert_eq!(
        event.property("UID").map(|p| p.value.as_str()),
        Some("abc-123")
    );
    assert_eq!(
        event.components_named("VALARM")[0]
            .property("TRIGGER")
            .map(|p| p.value.as_str()),
        Some("-PT15M")
    );
}

#[test]
fn parses_quoted_parameters_and_multiple_values() {
    let root = ICalParser::parse_text(&sample()).unwrap();
    let attendee = first_event(&root).property("ATTENDEE").unwrap();
    assert_eq!(attendee.parameter("CN"), Some("Doe, Jane"));
    let delegated = attendee
        .parameters
        .iter()
        .find(|p| p.name == "DELEGATED-FROM")
        .unwrap();
    assert_eq!(delegated.values, vec!["mailto:a@x.test", "mailto:b@x.test"]);
    assert_eq!(attendee.value, "mailto:jane@x.test");
}

#[test]
fn value_may_contain_colons_and_semicolons() {
    let root = ICalParser::parse_text(&sample()).unwrap();
    assert_eq!(
        first_event(&root).property("URL").map(|p| p.value.as_str()),
        Some("https://x.test:8443/a;b")
    );
}

#[test]
fn unescapes_text() {
    let root = ICalParser::parse_text(&sample()).unwrap();
    assert_eq!(
        first_event(&root)
            .property("SUMMARY")
            .map(|p| p.text())
            .as_deref(),
        Some("Planning, part 2\nwith notes")
    );
    assert_eq!(ICalText::escape("a;b,c\\d\ne"), "a\\;b\\,c\\\\d\\ne");
    assert_eq!(
        ICalText::unescape(&ICalText::escape("x;y,z\\\n")),
        "x;y,z\\\n"
    );
}

#[test]
fn folds_at_75_octets_without_splitting_utf8() {
    let long = "é🙂a".repeat(40);
    let component = ICalComponent::new(
        "VEVENT",
        vec![ICalProperty::from_text("SUMMARY", &long, Vec::new())],
        Vec::new(),
    );
    let text = ICalSerializer::serialize(&component);
    for line in text.split("\r\n") {
        assert!(line.len() <= 75, "{line:?}");
    }
    assert!(text.ends_with("END:VEVENT\r\n"));
    let parsed = ICalParser::parse_text(&text).unwrap();
    assert_eq!(parsed.property("SUMMARY").map(|p| p.text()), Some(long));
}

#[test]
fn unfolds_a_fold_that_splits_a_utf8_sequence() {
    // "é" is C3 A9; a careless server folds between the two bytes.
    let mut data = b"BEGIN:VEVENT\r\nSUMMARY:caf".to_vec();
    data.push(0xC3);
    data.extend_from_slice(b"\r\n ");
    data.push(0xA9);
    data.extend_from_slice(b"\r\nEND:VEVENT\r\n");
    let parsed = ICalParser::parse(&data).unwrap();
    assert_eq!(
        parsed.property("SUMMARY").map(|p| p.text()).as_deref(),
        Some("café")
    );
}

#[test]
fn parses_lf_only_and_missing_final_newline() {
    let text =
        "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\nSUMMARY:long\n  folded\nEND:VEVENT\nEND:VCALENDAR";
    let root = ICalParser::parse_text(text).unwrap();
    assert_eq!(
        first_event(&root)
            .property("SUMMARY")
            .map(|p| p.text())
            .as_deref(),
        Some("long folded")
    );
}

#[test]
fn round_trip_keeps_unknown_properties_and_parameters() {
    let first = ICalParser::parse_text(&sample()).unwrap();
    let again = ICalParser::parse_text(&ICalSerializer::serialize(&first)).unwrap();
    assert_eq!(again, first);
    let event = first_event(&again);
    assert_eq!(
        event.property("X-CUSTOM").unwrap().parameter("X-FLAG"),
        Some("1")
    );
    assert_eq!(
        event
            .property("X-APPLE-TRAVEL-ADVISORY-BEHAVIOR")
            .map(|p| p.value.as_str()),
        Some("AUTOMATIC")
    );
}

#[test]
fn rejects_unbalanced_and_unterminated_input() {
    for text in [
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nEND:VCALENDAR\r\n",
        "BEGIN:VCALENDAR\r\n",
        "SUMMARY:outside\r\n",
    ] {
        assert!(
            matches!(ICalParser::parse_text(text), Err(ICalError::Malformed(_))),
            "{text:?}"
        );
    }
}

#[test]
fn set_replaces_at_the_first_position() {
    let mut component = ICalComponent::new(
        "VEVENT",
        vec![
            ICalProperty::with_value("UID", "1"),
            ICalProperty::with_value("SUMMARY", "a"),
            ICalProperty::with_value("LOCATION", "x"),
            ICalProperty::with_value("SUMMARY", "b"),
        ],
        Vec::new(),
    );
    component.set(ICalProperty::with_value("summary", "c"));
    let names: Vec<&str> = component
        .properties
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(names, vec!["UID", "SUMMARY", "LOCATION"]);
    assert_eq!(
        component.property("SUMMARY").map(|p| p.value.as_str()),
        Some("c")
    );
    component.set_text("LOCATION", None);
    assert!(component.property("LOCATION").is_none());
}
