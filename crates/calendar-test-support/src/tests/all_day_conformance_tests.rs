//! Port of `AllDayConformanceTests.swift`.

use calendar_core::model::CalendarEvent;
use calendar_core::time::Zone;

use super::support::{iso, zone};
use crate::all_day_conformance::AllDayConformance;

fn ny() -> Zone {
    zone("America/New_York")
}

fn event(start: &str, end: &str, zone: Zone, all_day: bool) -> CalendarEvent {
    CalendarEvent {
        time_zone: zone,
        is_all_day: all_day,
        ..CalendarEvent::new("e", "c", "T", iso(start), iso(end))
    }
}

#[test]
fn canonical_all_day_event_has_no_violations() {
    let e = event("2026-09-18T04:00:00Z", "2026-09-19T04:00:00Z", ny(), true);
    assert!(AllDayConformance::violations(&e).is_empty());
}

#[test]
fn end_at_end_of_day_is_a_violation() {
    let e = event("2026-09-18T04:00:00Z", "2026-09-19T03:59:59Z", ny(), true);
    assert!(!AllDayConformance::violations(&e).is_empty());
}

#[test]
fn end_equal_to_start_is_a_violation() {
    let e = event("2026-09-18T04:00:00Z", "2026-09-18T04:00:00Z", ny(), true);
    assert!(!AllDayConformance::violations(&e).is_empty());
}

#[test]
fn timed_events_are_not_checked() {
    let e = event("2026-09-18T10:15:00Z", "2026-09-18T10:15:00Z", ny(), false);
    assert!(AllDayConformance::violations(&e).is_empty());
}
