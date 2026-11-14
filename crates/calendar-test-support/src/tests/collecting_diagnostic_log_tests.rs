//! Direct tests of the public `CollectingDiagnosticLog` (no Swift counterpart; see `fake_transport_tests`).

use calendar_core::diagnostic_log::{
    DiagnosticEvent, DiagnosticField, DiagnosticLevel, DiagnosticLog,
};

use super::support::at;
use crate::collecting_diagnostic_log::CollectingDiagnosticLog;

fn event(level: DiagnosticLevel, name: &str, fields: Vec<DiagnosticField>) -> DiagnosticEvent {
    DiagnosticEvent::new(level, "cat", name, fields, at(1_800_000_000))
}

#[test]
fn keeps_every_event_in_order() {
    let log = CollectingDiagnosticLog::new();
    log.log(event(DiagnosticLevel::Info, "a", vec![]));
    log.record(
        DiagnosticLevel::Error,
        "cat",
        "b",
        vec![],
        at(1_800_000_001),
    );
    let names: Vec<String> = log.events().into_iter().map(|e| e.name).collect();
    assert_eq!(names, ["a", "b"]);
    assert_eq!(log.events()[1].date, at(1_800_000_001));
}

#[test]
fn events_named_filters_by_name() {
    let log = CollectingDiagnosticLog::new();
    for name in ["x", "y", "x"] {
        log.log(event(DiagnosticLevel::Debug, name, vec![]));
    }
    assert_eq!(log.events_named("x").len(), 2);
    assert!(log.events_named("none").is_empty());
}

#[test]
fn clear_removes_everything() {
    let log = CollectingDiagnosticLog::new();
    log.log(event(DiagnosticLevel::Debug, "x", vec![]));
    log.clear();
    assert!(log.events().is_empty());
    assert_eq!(log.transcript(), "");
}

#[test]
fn transcript_includes_private_fields_unredacted() {
    let log = CollectingDiagnosticLog::new();
    log.log(event(
        DiagnosticLevel::Warning,
        "fetchFailed",
        vec![
            DiagnosticField::int("status", 500),
            DiagnosticField::string("uid", "secret value"),
            DiagnosticField::bool("retry", true),
        ],
    ));
    log.log(event(DiagnosticLevel::Notice, "done", vec![]));
    assert_eq!(
        log.transcript(),
        "cat fetchFailed warning status=500 uid=secret value retry=true\ncat done notice"
    );
}

#[test]
fn is_usable_through_a_shared_trait_object() {
    let log = std::sync::Arc::new(CollectingDiagnosticLog::new());
    let shared: std::sync::Arc<dyn DiagnosticLog> = log.clone();
    shared.log(event(DiagnosticLevel::Info, "x", vec![]));
    assert_eq!(log.events().len(), 1);
}
