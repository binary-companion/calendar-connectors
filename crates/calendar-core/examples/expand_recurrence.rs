//! Expands a weekly recurrence rule into the occurrences that fall in a window. No network, no clock.
//!
//! Run it with `cargo run -p calendar-core --example expand_recurrence`.

#![allow(clippy::print_stdout, clippy::expect_used)]

use calendar_core::recurrence::rule::RecurrenceRule;
use calendar_core::recurrence::set::RecurrenceSet;
use calendar_core::time::{DateInterval, Zone};
use jiff::Timestamp;

/// Runs the example (public so that the crate's test can call it).
pub fn main() {
    let zone = Zone::named("Europe/Berlin").expect("a zone every platform knows");
    let rule = RecurrenceRule::from_rrule("FREQ=WEEKLY;BYDAY=TU;COUNT=10", Some(&zone))
        .expect("a well-formed rule");
    let set = RecurrenceSet::new(vec![rule]);

    // The series starts on Tuesday 2026-09-01 09:00 Berlin time (07:00 UTC) and each occurrence lasts an hour.
    let anchor: Timestamp = "2026-09-01T07:00:00Z".parse().expect("a valid instant");
    let window = DateInterval::new(
        "2026-09-01T00:00:00Z".parse().expect("a valid instant"),
        "2026-09-30T00:00:00Z".parse().expect("a valid instant"),
    );

    let expansion = set.occurrences(
        anchor,
        3600.0,
        &zone,
        false,
        &window,
        RecurrenceSet::DEFAULT_OCCURRENCE_LIMIT,
    );
    assert!(!expansion.truncated && !expansion.has_unreadable_rule);
    // Tuesdays 1, 8, 15, 22 and 29 September; the daylight-saving offset is the same all month.
    assert_eq!(expansion.starts.len(), 5);
    for start in &expansion.starts {
        println!("{start}");
    }
}
