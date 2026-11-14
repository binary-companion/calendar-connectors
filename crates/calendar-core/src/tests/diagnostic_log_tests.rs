use std::sync::Arc;

use jiff::Timestamp;

use super::support::CollectingDiagnosticLog;
use crate::diagnostic_log::{
    DiagnosticEvent, DiagnosticField, DiagnosticFieldValue, DiagnosticLevel, DiagnosticLog,
    FanOutDiagnosticLog, NullDiagnosticLog, RingBufferDiagnosticLog,
};

fn epoch() -> Timestamp {
    Timestamp::from_second(1_800_000_000).unwrap()
}

fn event_with(name: &str, fields: Vec<DiagnosticField>, level: DiagnosticLevel) -> DiagnosticEvent {
    DiagnosticEvent::new(level, "test", name, fields, epoch())
}

fn event(name: &str) -> DiagnosticEvent {
    event_with(name, Vec::new(), DiagnosticLevel::Info)
}

fn names(events: &[DiagnosticEvent]) -> Vec<String> {
    events.iter().map(|e| e.name.clone()).collect()
}

#[test]
fn levels_are_ordered() {
    use DiagnosticLevel::*;
    let levels = [Debug, Info, Notice, Warning, Error];
    let mut sorted = levels;
    sorted.sort();
    assert!(levels == sorted && levels[0] < levels[4]);
}

#[test]
fn string_fields_are_private_by_default_and_numbers_are_not() {
    assert!(DiagnosticField::string("uid", "abc").is_private);
    assert!(!DiagnosticField::public_string("host", "www.example.test").is_private);
    assert!(!DiagnosticField::int("events", 12).is_private);
    assert!(!DiagnosticField::bool("ok", true).is_private);
    assert!(!DiagnosticField::double("seconds", 1.5).is_private);
    assert_eq!(
        DiagnosticField::int("events", 12).value,
        DiagnosticFieldValue::Int(12)
    );
}

#[test]
fn the_ring_buffer_keeps_the_last_events_oldest_first() {
    let log = RingBufferDiagnosticLog::new(3);
    for index in 1..=5 {
        log.log(event(&format!("e{index}")));
    }
    assert_eq!(names(&log.snapshot()), ["e3", "e4", "e5"]);
    log.clear();
    assert!(log.snapshot().is_empty());
}

#[test]
fn the_ring_buffer_defaults_to_five_hundred() {
    let log = RingBufferDiagnosticLog::with_default_capacity();
    for index in 0..520 {
        log.log(event(&format!("e{index}")));
    }
    let names = names(&log.snapshot());
    assert!(
        names.len() == 500 && names.first().unwrap() == "e20" && names.last().unwrap() == "e519"
    );
}

#[test]
fn the_ring_buffer_survives_concurrent_writers() {
    let log = Arc::new(RingBufferDiagnosticLog::new(100));
    std::thread::scope(|scope| {
        for task in 0..8 {
            let log = Arc::clone(&log);
            scope.spawn(move || {
                for index in 0..250 {
                    log.log(event(&format!("t{task}-{index}")));
                }
                let _ = log.snapshot();
            });
        }
    });
    assert_eq!(log.snapshot().len(), 100);
}

#[test]
fn rendering_hides_private_fields_unless_asked() {
    let log = RingBufferDiagnosticLog::with_default_capacity();
    log.log(event_with(
        "rruleUnreadable",
        vec![
            DiagnosticField::string("uid", "secret@example.test"),
            DiagnosticField::int("n", 2),
            DiagnosticField::public_string("host", "www.example.test"),
        ],
        DiagnosticLevel::Notice,
    ));
    let hidden = log.render(false);
    assert_eq!(
        hidden,
        "2027-01-15T08:00:00Z notice test.rruleUnreadable uid=<private> n=2 host=www.example.test"
    );
    assert!(!hidden.contains("secret@example.test"));
    assert!(log.render(true).contains("uid=secret@example.test"));
}

#[test]
fn rendering_is_one_line_per_event_and_quotes_awkward_strings() {
    let log = RingBufferDiagnosticLog::with_default_capacity();
    log.log(event_with(
        "a",
        vec![DiagnosticField::public_string("note", "two words\nnext")],
        DiagnosticLevel::Info,
    ));
    log.log(event("b"));
    let text = log.render(false);
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].ends_with("test.a note=\"two words\\nnext\""));
}

#[test]
fn fan_out_sends_to_every_log_and_null_discards() {
    let first = Arc::new(CollectingDiagnosticLog::default());
    let second = Arc::new(RingBufferDiagnosticLog::with_default_capacity());
    let fan = FanOutDiagnosticLog::new(vec![
        first.clone(),
        second.clone(),
        Arc::new(NullDiagnosticLog),
    ]);
    fan.log(event("x"));
    fan.record(
        DiagnosticLevel::Info,
        "test",
        "y",
        vec![DiagnosticField::int("n", 1)],
        epoch(),
    );
    assert!(names(&first.events()) == ["x", "y"] && names(&second.snapshot()) == ["x", "y"]);
    let events = first.events();
    assert_eq!(
        events[1].field("n").map(|f| f.value.clone()),
        Some(DiagnosticFieldValue::Int(1))
    );
    assert_eq!(events[1].date, epoch());
}

#[test]
fn double_fields_render_like_swifts_string_of_double() {
    // Expected texts were produced by `String(Double)` in Swift.
    let cases = [
        (1.5, "1.5"),
        (2.0, "2.0"),
        (0.0001, "0.0001"),
        (1e300, "1e+300"),
        (1e-5, "1e-05"),
        (1e16, "1e+16"),
        (1.5e-7, "1.5e-07"),
        (123456789012345680.0, "1.2345678901234568e+17"),
        (f64::NAN, "nan"),
        (f64::NEG_INFINITY, "-inf"),
    ];
    for (number, text) in cases {
        assert_eq!(DiagnosticField::double("x", number).unredacted_text(), text);
    }
}
