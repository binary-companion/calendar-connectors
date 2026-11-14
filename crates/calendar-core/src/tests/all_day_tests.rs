use super::support::{iso, zone};
use crate::all_day::{AllDay, CalendarDate};
use crate::source_types::SourceError;

#[test]
fn canonical_one_day_in_new_york() {
    let (start, end) = AllDay::canonical(
        &CalendarDate::new(2026, 9, 18),
        &CalendarDate::new(2026, 9, 19),
        &zone("America/New_York"),
    )
    .unwrap();
    assert_eq!(start, iso("2026-09-18T04:00:00Z"));
    assert_eq!(end, iso("2026-09-19T04:00:00Z"));
}

#[test]
fn dates_round_trip_in_another_zone() {
    let tokyo = zone("Asia/Tokyo");
    let (start, end) = AllDay::canonical(
        &CalendarDate::new(2026, 9, 18),
        &CalendarDate::new(2026, 9, 20),
        &tokyo,
    )
    .unwrap();
    let (first, end_exclusive) = AllDay::dates(start, end, &tokyo);
    assert_eq!(first, CalendarDate::new(2026, 9, 18));
    assert_eq!(end_exclusive, CalendarDate::new(2026, 9, 20));
}

#[test]
fn start_of_day_when_midnight_does_not_exist() {
    // Sao Paulo skipped 00:00 on 2018-11-04 (clocks jumped to 01:00).
    let start =
        AllDay::start_of_day(&CalendarDate::new(2018, 11, 4), &zone("America/Sao_Paulo")).unwrap();
    assert_eq!(start, iso("2018-11-04T03:00:00Z"));
}

#[test]
fn adding_days_crosses_months_and_leap_days() {
    assert_eq!(
        CalendarDate::new(2026, 2, 28).adding_days(1),
        CalendarDate::new(2026, 3, 1)
    );
    assert_eq!(
        CalendarDate::new(2028, 2, 28).adding_days(1),
        CalendarDate::new(2028, 2, 29)
    );
    assert_eq!(
        AllDay::end_exclusive_after_last(&CalendarDate::new(2026, 12, 31)),
        CalendarDate::new(2027, 1, 1)
    );
}

#[test]
fn needs_permission_is_a_distinct_error() {
    assert_ne!(SourceError::NeedsPermission, SourceError::AuthExpired);
}

#[test]
fn calendar_dates_order_chronologically() {
    assert!(CalendarDate::new(2026, 9, 20) < CalendarDate::new(2026, 9, 21));
    assert!(CalendarDate::new(2026, 12, 31) < CalendarDate::new(2027, 1, 1));
    assert!(CalendarDate::new(2026, 9, 21) >= CalendarDate::new(2026, 9, 21));
    assert!(!(CalendarDate::new(2026, 9, 21) < CalendarDate::new(2026, 9, 21)));
}
