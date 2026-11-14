use super::support::{iso, zone};
use crate::all_day::{AllDay, CalendarDate};
use crate::recurrence::rule::{Frequency, RecurrenceRule};
use crate::recurrence::set::RecurrenceSet;

#[test]
fn a_series_with_rule_exdate_and_rdate_is_read_with_every_date_form() {
    let lines = [
        "RRULE:FREQ=WEEKLY;BYDAY=MO",
        "EXDATE;TZID=America/New_York:20260921T100000,20260928T100000", // a named zone
        "EXDATE:20261005T140000Z",                                      // UTC
        "EXDATE:20261012T100000", // floating: read in the series zone
        "RDATE;TZID=America/New_York:20261001T090000",
    ];
    let set = RecurrenceSet::from_icalendar_lines(&lines, &zone("America/New_York"), false);
    assert!(set.rules.len() == 1 && set.rules[0].frequency == Frequency::Weekly);
    assert_eq!(
        set.excluded_dates,
        Some(vec![
            iso("2026-09-21T14:00:00Z"),
            iso("2026-09-28T14:00:00Z"),
            iso("2026-10-05T14:00:00Z"),
            iso("2026-10-12T14:00:00Z")
        ])
    );
    assert_eq!(set.extra_dates, Some(vec![iso("2026-10-01T13:00:00Z")]));
    assert!(set.unparsed.is_empty());
}

#[test]
fn an_all_day_series_reads_value_date_through_the_canonical_midnight() {
    let new_york = zone("America/New_York");
    let set = RecurrenceSet::from_icalendar_lines(
        &["RRULE:FREQ=DAILY", "EXDATE;VALUE=DATE:20260921,20260922"],
        &new_york,
        true,
    );
    let first = AllDay::start_of_day(&CalendarDate::new(2026, 9, 21), &new_york).unwrap();
    let second = AllDay::start_of_day(&CalendarDate::new(2026, 9, 22), &new_york).unwrap();
    assert_eq!(set.excluded_dates, Some(vec![first, second]));
}

#[test]
fn lines_the_library_does_not_model_are_kept_verbatim() {
    let lines = [
        "EXRULE:FREQ=WEEKLY",
        "RDATE;VALUE=PERIOD:19960403T020000Z/19960403T040000Z",
        "EXDATE;TZID=W. Europe Standard Time:20260921T100000", // a Windows zone name
        "RRULE:FREQ=NEVER",
        "X-CUSTOM:1",
        "RRULE:FREQ=DAILY",
    ];
    let set = RecurrenceSet::from_icalendar_lines(&lines, &zone("UTC"), false);
    assert_eq!(
        set.unparsed,
        [
            "EXRULE:FREQ=WEEKLY",
            "RDATE;VALUE=PERIOD:19960403T020000Z/19960403T040000Z",
            "EXDATE;TZID=W. Europe Standard Time:20260921T100000",
            "RRULE:FREQ=NEVER",
            "X-CUSTOM:1"
        ]
    );
    assert!(
        set.rules.len() == 1
            && set.excluded_dates == Some(vec![])
            && set.extra_dates == Some(vec![])
    );
}

#[test]
fn lines_round_trip() {
    let new_york = zone("America/New_York");
    let utc = zone("UTC");
    let lines = [
        "RRULE:FREQ=WEEKLY;BYDAY=MO",
        "RDATE;TZID=America/New_York:20261001T090000",
        "EXDATE;TZID=America/New_York:20260921T100000,20260928T100000",
        "EXRULE:FREQ=DAILY",
    ];
    let set = RecurrenceSet::from_icalendar_lines(&lines, &new_york, false);
    assert_eq!(set.icalendar_lines(&new_york, false), lines);
    let all_day = ["RRULE:FREQ=DAILY", "EXDATE;VALUE=DATE:20260921"];
    assert_eq!(
        RecurrenceSet::from_icalendar_lines(&all_day, &new_york, true)
            .icalendar_lines(&new_york, true),
        all_day
    );
    let in_utc = ["RRULE:FREQ=DAILY", "EXDATE:20260921T100000Z"];
    assert_eq!(
        RecurrenceSet::from_icalendar_lines(&in_utc, &utc, false).icalendar_lines(&utc, false),
        in_utc
    );
}

#[test]
fn a_source_that_cannot_list_extra_or_skipped_dates_uses_nil() {
    let set = RecurrenceSet {
        rules: vec![RecurrenceRule::new(Frequency::Daily)],
        extra_dates: None,
        excluded_dates: None,
        ..RecurrenceSet::default()
    };
    assert!(set.extra_dates.is_none() && set.excluded_dates.is_none());
    assert_eq!(
        set.icalendar_lines(&zone("UTC"), false),
        ["RRULE:FREQ=DAILY"]
    );
}
