use jiff::Timestamp;

use super::support::zone;
use crate::all_day::{AllDay, CalendarDate};
use crate::civil;
use crate::recurrence::rule::RecurrenceRule;
use crate::recurrence::set::RecurrenceSet;
use crate::time::{DateInterval, Zone};

fn ny() -> Zone {
    zone("America/New_York")
}

fn local(y: i64, mo: i64, d: i64, h: i64, mi: i64) -> Timestamp {
    civil::date_from(y, mo, d, h, mi, 0, &ny()).unwrap()
}

fn rule(text: &str) -> RecurrenceRule {
    RecurrenceRule::from_rrule(text, None).unwrap()
}

fn set_of(text: &str) -> RecurrenceSet {
    RecurrenceSet::new(vec![rule(text)])
}

const LIMIT: i64 = RecurrenceSet::DEFAULT_OCCURRENCE_LIMIT;

#[test]
fn window_keeps_only_overlapping_instances() {
    let set = set_of("FREQ=DAILY");
    let result = set.occurrences(
        local(2026, 9, 1, 9, 0),
        3600.0,
        &ny(),
        false,
        &DateInterval::new(local(2026, 9, 10, 9, 30), local(2026, 9, 12, 9, 0)),
        LIMIT,
    );
    // 10th 09:00-10:00 overlaps (started before the window), 11th inside, 12th starts at the end (excluded).
    assert_eq!(
        result.starts,
        vec![local(2026, 9, 10, 9, 0), local(2026, 9, 11, 9, 0)]
    );
    assert!(!result.truncated);
}

#[test]
fn exdates_remove_and_rdates_add() {
    let set = RecurrenceSet {
        extra_dates: Some(vec![local(2026, 9, 20, 9, 0)]),
        excluded_dates: Some(vec![local(2026, 9, 3, 9, 0)]),
        ..set_of("FREQ=DAILY;COUNT=5")
    };
    let result = set.occurrences(
        local(2026, 9, 1, 9, 0),
        1800.0,
        &ny(),
        false,
        &DateInterval::new(local(2026, 9, 1, 0, 0), local(2026, 10, 1, 9, 0)),
        LIMIT,
    );
    assert_eq!(
        result.starts,
        vec![
            local(2026, 9, 1, 9, 0),
            local(2026, 9, 2, 9, 0),
            local(2026, 9, 4, 9, 0),
            local(2026, 9, 5, 9, 0),
            local(2026, 9, 20, 9, 0)
        ]
    );
}

#[test]
fn all_day_exdate_matches_by_day() {
    let zone = ny();
    let day = |d: i64| AllDay::start_of_day(&CalendarDate::new(2026, 9, d), &zone).unwrap();
    let set = RecurrenceSet {
        excluded_dates: Some(vec![day(2)]),
        ..set_of("FREQ=DAILY;COUNT=3")
    };
    let result = set.occurrences(
        day(1),
        86_400.0,
        &zone,
        true,
        &DateInterval::new(day(1), day(10)),
        LIMIT,
    );
    assert_eq!(result.starts, vec![day(1), day(3)]);
}

#[test]
fn old_daily_series_still_shows_this_week() {
    let set = set_of("FREQ=DAILY");
    let result = set.occurrences(
        local(2015, 1, 1, 9, 0),
        1800.0,
        &ny(),
        false,
        &DateInterval::new(local(2026, 9, 27, 0, 0), local(2026, 10, 4, 0, 0)),
        LIMIT,
    );
    assert_eq!(result.starts.len(), 7);
    assert_eq!(result.starts.first(), Some(&local(2026, 9, 27, 9, 0)));
    assert!(!result.truncated);
}

#[test]
fn old_weekly_series_with_interval_still_shows_this_month() {
    let set = set_of("FREQ=WEEKLY;INTERVAL=2;BYDAY=TU");
    let anchor = local(2015, 1, 6, 9, 0); // a Tuesday
    let result = set.occurrences(
        anchor,
        1800.0,
        &ny(),
        false,
        &DateInterval::new(local(2026, 9, 1, 0, 0), local(2026, 10, 1, 0, 0)),
        LIMIT,
    );
    // Every second Tuesday from 2015-01-06: 2026-09-01 is 608 weeks later (even), so 09-01, 09-15 and 09-29.
    assert_eq!(
        result.starts,
        vec![
            local(2026, 9, 1, 9, 0),
            local(2026, 9, 15, 9, 0),
            local(2026, 9, 29, 9, 0)
        ]
    );
}

#[test]
fn large_count_series_still_shows_the_window() {
    // COUNT cannot skip ahead, so the years of instances before the window must not use up the limit.
    let set = set_of("FREQ=DAILY;COUNT=10000");
    let result = set.occurrences(
        local(2015, 1, 1, 9, 0),
        1800.0,
        &ny(),
        false,
        &DateInterval::new(local(2026, 9, 27, 0, 0), local(2026, 10, 4, 0, 0)),
        LIMIT,
    );
    assert_eq!(result.starts.len(), 7);
    assert!(!result.truncated);
}

#[test]
fn unreadable_rule_is_reported() {
    let set = RecurrenceSet::from_icalendar_lines(&["RRULE:FREQ=FORTNIGHTLY"], &ny(), false);
    let result = set.occurrences(
        local(2026, 9, 1, 9, 0),
        1800.0,
        &ny(),
        false,
        &DateInterval::new(local(2026, 9, 1, 0, 0), local(2026, 10, 1, 9, 0)),
        LIMIT,
    );
    assert!(result.has_unreadable_rule);
    assert_eq!(result.starts, vec![local(2026, 9, 1, 9, 0)]);
}

#[test]
fn limit_is_reported() {
    let set = set_of("FREQ=MINUTELY");
    let result = set.occurrences(
        local(2026, 9, 1, 9, 0),
        60.0,
        &ny(),
        false,
        &DateInterval::new(local(2026, 9, 1, 9, 0), local(2026, 9, 30, 9, 0)),
        100,
    );
    assert_eq!(result.starts.len(), 100);
    assert!(result.truncated);
}

#[test]
fn rule_instance_count_includes_anchor_and_excluded_dates() {
    let set = RecurrenceSet {
        excluded_dates: Some(vec![local(2026, 9, 8, 9, 0)]),
        ..set_of("FREQ=WEEKLY;COUNT=10")
    };
    // Instances before 09-22: 09-01, 09-08 (excluded but counted, as RFC 5545 COUNT does), 09-15.
    assert_eq!(
        set.rule_instance_count(
            local(2026, 9, 1, 9, 0),
            &ny(),
            false,
            local(2026, 9, 22, 9, 0)
        ),
        3
    );
}

#[test]
fn a_set_with_an_unparsed_rule_reports_it_as_unreadable() {
    let with = |line: &str| RecurrenceSet {
        unparsed: vec![line.to_string()],
        ..RecurrenceSet::default()
    };
    assert!(with("RRULE:INTERVAL=2").has_unreadable_rule());
    assert!(!with("EXRULE:FREQ=DAILY").has_unreadable_rule());
    assert!(!RecurrenceSet::default().has_unreadable_rule());
}
