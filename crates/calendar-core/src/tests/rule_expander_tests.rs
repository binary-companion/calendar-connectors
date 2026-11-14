use jiff::Timestamp;

use super::support::{at, zone};
use crate::all_day::{AllDay, CalendarDate};
use crate::civil;
use crate::recurrence::rule::RecurrenceRule;
use crate::time::Zone;

fn ny() -> Zone {
    zone("America/New_York")
}

/// `local(y, mo, d, h, mi, s, zone:)`: `Calendar.date(from:)` in `zone`.
fn local_in(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64, zone: &Zone) -> Timestamp {
    civil::date_from(y, mo, d, h, mi, s, zone).unwrap()
}

fn local(y: i64, mo: i64, d: i64, h: i64, mi: i64) -> Timestamp {
    local_in(y, mo, d, h, mi, 0, &ny())
}

/// Local "yyyy-MM-dd HH:mm" texts, easy to compare with the RFC's listings.
fn texts(dates: &[Timestamp], zone: &Zone) -> Vec<String> {
    dates
        .iter()
        .map(|date| {
            let c = civil::components(*date, zone);
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}",
                c.year, c.month, c.day, c.hour, c.minute
            )
        })
        .collect()
}

fn expand_with(
    rule: &str,
    anchor: Timestamp,
    before: Timestamp,
    limit: i64,
    skip_to: Option<Timestamp>,
) -> Vec<String> {
    let zone = ny();
    let r = RecurrenceRule::from_rrule(rule, Some(&zone)).unwrap();
    texts(
        &r.instances(anchor, &zone, false, before, limit, skip_to)
            .starts,
        &zone,
    )
}

fn expand(rule: &str, anchor: Timestamp) -> Vec<String> {
    expand_with(rule, anchor, local(2050, 1, 1, 9, 0), 5000, None)
}

fn expand_before(rule: &str, anchor: Timestamp, before: Timestamp) -> Vec<String> {
    expand_with(rule, anchor, before, 5000, None)
}

fn days(list: &[String]) -> Vec<String> {
    list.iter()
        .map(|text| text.chars().take(10).collect())
        .collect()
}

#[test]
fn daily_for_ten_occurrences() {
    let expected: Vec<String> = (2..=11).map(|d| format!("1997-09-{d:02}")).collect();
    assert_eq!(
        days(&expand("FREQ=DAILY;COUNT=10", local(1997, 9, 2, 9, 0))),
        expected
    );
}

#[test]
fn daily_until_is_inclusive_and_counts_correctly() {
    let result = expand("FREQ=DAILY;UNTIL=19971224T000000Z", local(1997, 9, 2, 9, 0));
    assert_eq!(result.len(), 113);
    assert_eq!(result.last().map(String::as_str), Some("1997-12-23 09:00"));
}

#[test]
fn every_ten_days_five_times() {
    assert_eq!(
        days(&expand(
            "FREQ=DAILY;INTERVAL=10;COUNT=5",
            local(1997, 9, 2, 9, 0)
        )),
        [
            "1997-09-02",
            "1997-09-12",
            "1997-09-22",
            "1997-10-02",
            "1997-10-12"
        ]
    );
}

#[test]
fn every_day_in_january_for_three_years() {
    let result = expand(
        "FREQ=YEARLY;UNTIL=20000131T140000Z;BYMONTH=1;BYDAY=SU,MO,TU,WE,TH,FR,SA",
        local(1998, 1, 1, 9, 0),
    );
    assert_eq!(result.len(), 93);
    assert!(
        result
            .iter()
            .all(|text| text.chars().skip(5).collect::<String>().starts_with("01-"))
    );
}

#[test]
fn weekly_for_ten_occurrences() {
    assert_eq!(
        days(&expand("FREQ=WEEKLY;COUNT=10", local(1997, 9, 2, 9, 0))),
        [
            "1997-09-02",
            "1997-09-09",
            "1997-09-16",
            "1997-09-23",
            "1997-09-30",
            "1997-10-07",
            "1997-10-14",
            "1997-10-21",
            "1997-10-28",
            "1997-11-04"
        ]
    );
}

#[test]
fn weekly_tuesday_thursday_for_five_weeks() {
    assert_eq!(
        days(&expand(
            "FREQ=WEEKLY;UNTIL=19971007T000000Z;WKST=SU;BYDAY=TU,TH",
            local(1997, 9, 2, 9, 0)
        )),
        [
            "1997-09-02",
            "1997-09-04",
            "1997-09-09",
            "1997-09-11",
            "1997-09-16",
            "1997-09-18",
            "1997-09-23",
            "1997-09-25",
            "1997-09-30",
            "1997-10-02"
        ]
    );
}

#[test]
fn every_other_week_monday_wednesday_friday() {
    assert_eq!(
        days(&expand(
            "FREQ=WEEKLY;INTERVAL=2;UNTIL=19971224T000000Z;WKST=SU;BYDAY=MO,WE,FR",
            local(1997, 9, 1, 9, 0)
        )),
        [
            "1997-09-01",
            "1997-09-03",
            "1997-09-05",
            "1997-09-15",
            "1997-09-17",
            "1997-09-19",
            "1997-09-29",
            "1997-10-01",
            "1997-10-03",
            "1997-10-13",
            "1997-10-15",
            "1997-10-17",
            "1997-10-27",
            "1997-10-29",
            "1997-10-31",
            "1997-11-10",
            "1997-11-12",
            "1997-11-14",
            "1997-11-24",
            "1997-11-26",
            "1997-11-28",
            "1997-12-08",
            "1997-12-10",
            "1997-12-12",
            "1997-12-22"
        ]
    );
}

#[test]
fn week_start_changes_the_result() {
    assert_eq!(
        days(&expand(
            "FREQ=WEEKLY;INTERVAL=2;COUNT=4;BYDAY=TU,SU;WKST=MO",
            local(1997, 8, 5, 9, 0)
        )),
        ["1997-08-05", "1997-08-10", "1997-08-19", "1997-08-24"]
    );
    assert_eq!(
        days(&expand(
            "FREQ=WEEKLY;INTERVAL=2;COUNT=4;BYDAY=TU,SU;WKST=SU",
            local(1997, 8, 5, 9, 0)
        )),
        ["1997-08-05", "1997-08-17", "1997-08-19", "1997-08-31"]
    );
}

#[test]
fn monthly_first_friday() {
    assert_eq!(
        days(&expand(
            "FREQ=MONTHLY;COUNT=10;BYDAY=1FR",
            local(1997, 9, 5, 9, 0)
        )),
        [
            "1997-09-05",
            "1997-10-03",
            "1997-11-07",
            "1997-12-05",
            "1998-01-02",
            "1998-02-06",
            "1998-03-06",
            "1998-04-03",
            "1998-05-01",
            "1998-06-05"
        ]
    );
}

#[test]
fn monthly_second_to_last_monday() {
    assert_eq!(
        days(&expand(
            "FREQ=MONTHLY;COUNT=6;BYDAY=-2MO",
            local(1997, 9, 22, 9, 0)
        )),
        [
            "1997-09-22",
            "1997-10-20",
            "1997-11-17",
            "1997-12-22",
            "1998-01-19",
            "1998-02-16"
        ]
    );
}

#[test]
fn monthly_third_to_last_day() {
    assert_eq!(
        days(&expand(
            "FREQ=MONTHLY;COUNT=6;BYMONTHDAY=-3",
            local(1997, 9, 28, 9, 0)
        )),
        [
            "1997-09-28",
            "1997-10-29",
            "1997-11-28",
            "1997-12-29",
            "1998-01-29",
            "1998-02-26"
        ]
    );
}

#[test]
fn last_workday_of_the_month() {
    assert_eq!(
        days(&expand(
            "FREQ=MONTHLY;COUNT=7;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1",
            local(1997, 9, 30, 9, 0)
        )),
        [
            "1997-09-30",
            "1997-10-31",
            "1997-11-28",
            "1997-12-31",
            "1998-01-30",
            "1998-02-27",
            "1998-03-31"
        ]
    );
}

#[test]
fn third_tuesday_wednesday_or_thursday() {
    assert_eq!(
        days(&expand(
            "FREQ=MONTHLY;COUNT=3;BYDAY=TU,WE,TH;BYSETPOS=3",
            local(1997, 9, 4, 9, 0)
        )),
        ["1997-09-04", "1997-10-07", "1997-11-06"]
    );
}

#[test]
fn invalid_dates_are_skipped() {
    assert_eq!(
        days(&expand(
            "FREQ=MONTHLY;BYMONTHDAY=15,30;COUNT=5",
            local(2007, 1, 15, 9, 0)
        )),
        [
            "2007-01-15",
            "2007-01-30",
            "2007-02-15",
            "2007-03-15",
            "2007-03-30"
        ]
    );
}

#[test]
fn friday_the_13th() {
    // The RFC lists the rule with an EXDATE for the anchor; the rule alone starts with the anchor.
    let result = expand_before(
        "FREQ=MONTHLY;BYDAY=FR;BYMONTHDAY=13",
        local(1997, 9, 2, 9, 0),
        local(2000, 12, 31, 9, 0),
    );
    assert_eq!(
        days(&result[1..]),
        [
            "1998-02-13",
            "1998-03-13",
            "1998-11-13",
            "1999-08-13",
            "2000-10-13"
        ]
    );
}

#[test]
fn yearly_in_june_and_july() {
    assert_eq!(
        days(&expand(
            "FREQ=YEARLY;COUNT=10;BYMONTH=6,7",
            local(1997, 6, 10, 9, 0)
        )),
        [
            "1997-06-10",
            "1997-07-10",
            "1998-06-10",
            "1998-07-10",
            "1999-06-10",
            "1999-07-10",
            "2000-06-10",
            "2000-07-10",
            "2001-06-10",
            "2001-07-10"
        ]
    );
}

#[test]
fn monday_of_week_twenty() {
    assert_eq!(
        days(&expand_before(
            "FREQ=YEARLY;BYWEEKNO=20;BYDAY=MO",
            local(1997, 5, 12, 9, 0),
            local(2000, 1, 1, 9, 0)
        )),
        ["1997-05-12", "1998-05-11", "1999-05-17"]
    );
}

#[test]
fn every_third_year_on_year_days() {
    assert_eq!(
        days(&expand(
            "FREQ=YEARLY;INTERVAL=3;COUNT=10;BYYEARDAY=1,100,200",
            local(1997, 1, 1, 9, 0)
        )),
        [
            "1997-01-01",
            "1997-04-10",
            "1997-07-19",
            "2000-01-01",
            "2000-04-09",
            "2000-07-18",
            "2003-01-01",
            "2003-04-10",
            "2003-07-19",
            "2006-01-01"
        ]
    );
}

#[test]
fn every_thursday_in_march() {
    assert_eq!(
        days(&expand_before(
            "FREQ=YEARLY;BYMONTH=3;BYDAY=TH",
            local(1997, 3, 13, 9, 0),
            local(1999, 12, 31, 9, 0)
        )),
        [
            "1997-03-13",
            "1997-03-20",
            "1997-03-27",
            "1998-03-05",
            "1998-03-12",
            "1998-03-19",
            "1998-03-26",
            "1999-03-04",
            "1999-03-11",
            "1999-03-18",
            "1999-03-25"
        ]
    );
}

#[test]
fn twentieth_monday_of_the_year() {
    assert_eq!(
        days(&expand_before(
            "FREQ=YEARLY;BYDAY=20MO",
            local(1997, 5, 19, 9, 0),
            local(2000, 1, 1, 9, 0)
        )),
        ["1997-05-19", "1998-05-18", "1999-05-17"]
    );
}

#[test]
fn presidential_election_day() {
    assert_eq!(
        days(&expand_before(
            "FREQ=YEARLY;INTERVAL=4;BYMONTH=11;BYDAY=TU;BYMONTHDAY=2,3,4,5,6,7,8",
            local(1996, 11, 5, 9, 0),
            local(2005, 1, 1, 9, 0)
        )),
        ["1996-11-05", "2000-11-07", "2004-11-02"]
    );
}

#[test]
fn every_fifteen_minutes_six_times() {
    assert_eq!(
        expand("FREQ=MINUTELY;INTERVAL=15;COUNT=6", local(1997, 9, 2, 9, 0)),
        [
            "1997-09-02 09:00",
            "1997-09-02 09:15",
            "1997-09-02 09:30",
            "1997-09-02 09:45",
            "1997-09-02 10:00",
            "1997-09-02 10:15"
        ]
    );
}

#[test]
fn every_twenty_minutes_during_the_day() {
    let result = expand_before(
        "FREQ=DAILY;BYHOUR=9,10,11,12,13,14,15,16;BYMINUTE=0,20,40",
        local(1997, 9, 2, 9, 0),
        local(1997, 9, 3, 0, 0),
    );
    assert_eq!(result.len(), 24);
    assert_eq!(result.first().map(String::as_str), Some("1997-09-02 09:00"));
    assert_eq!(result.last().map(String::as_str), Some("1997-09-02 16:40"));
}

#[test]
fn keeps_wall_clock_across_dst() {
    let result = expand_before(
        "FREQ=DAILY;COUNT=3",
        local(2026, 3, 7, 9, 0),
        local(2027, 1, 1, 9, 0),
    );
    assert_eq!(
        result,
        ["2026-03-07 09:00", "2026-03-08 09:00", "2026-03-09 09:00"]
    );
}

#[test]
fn missing_local_time_moves_forward() {
    let result = expand_before(
        "FREQ=DAILY;COUNT=3",
        local(2026, 3, 7, 2, 30),
        local(2027, 1, 1, 9, 0),
    );
    assert_eq!(
        result,
        ["2026-03-07 02:30", "2026-03-08 03:30", "2026-03-09 02:30"]
    );
}

#[test]
fn repeated_local_time_uses_the_first() {
    let rule = RecurrenceRule::from_rrule("FREQ=DAILY;COUNT=2", None).unwrap();
    let starts = rule
        .instances(
            local(2026, 10, 31, 1, 30),
            &ny(),
            false,
            local(2027, 1, 1, 9, 0),
            10,
            None,
        )
        .starts;
    assert_eq!(starts[1], at(1_793_511_000)); // 2026-11-01 05:30Z = 01:30 EDT, not EST
}

#[test]
fn all_day_rules_produce_canonical_midnights() {
    let zone = ny();
    let rule = RecurrenceRule::from_rrule("FREQ=WEEKLY;COUNT=3", None).unwrap();
    let anchor = AllDay::start_of_day(&CalendarDate::new(2026, 3, 2), &zone).unwrap();
    let starts = rule
        .instances(anchor, &zone, true, local(2027, 1, 1, 9, 0), 10, None)
        .starts;
    let expected: Vec<Timestamp> = [2, 9, 16]
        .iter()
        .map(|d| AllDay::start_of_day(&CalendarDate::new(2026, 3, *d), &zone).unwrap())
        .collect();
    assert_eq!(starts, expected);
}

#[test]
fn leap_day_yearly_only_in_leap_years() {
    assert_eq!(
        days(&expand("FREQ=YEARLY;COUNT=3", local(2024, 2, 29, 9, 0))),
        ["2024-02-29", "2028-02-29", "2032-02-29"]
    );
}

#[test]
fn limit_truncates() {
    let rule = RecurrenceRule::from_rrule("FREQ=DAILY", None).unwrap();
    let result = rule.instances(
        local(2026, 1, 1, 9, 0),
        &ny(),
        false,
        local(2027, 1, 1, 9, 0),
        5,
        None,
    );
    assert_eq!(result.starts.len(), 5);
    assert!(result.truncated);
}

#[test]
fn anchor_at_or_after_bound_gives_nothing() {
    let rule = RecurrenceRule::from_rrule("FREQ=DAILY", None).unwrap();
    assert!(
        rule.instances(
            local(2026, 1, 1, 9, 0),
            &ny(),
            false,
            local(2026, 1, 1, 9, 0),
            5,
            None
        )
        .starts
        .is_empty()
    );
}

#[test]
fn skip_to_jumps_whole_periods_without_changing_results() {
    let rule = RecurrenceRule::from_rrule("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,TH", None).unwrap();
    let anchor = local(2015, 1, 5, 9, 0);
    let cut = local(2026, 9, 1, 9, 0);
    let before = local(2026, 10, 1, 9, 0);
    let full = rule
        .instances(anchor, &ny(), false, before, 100_000, None)
        .starts;
    let skipped = rule
        .instances(anchor, &ny(), false, before, 100_000, Some(cut))
        .starts;
    assert_eq!(skipped.first(), Some(&anchor));
    let after_cut = |list: &[Timestamp]| -> Vec<Timestamp> {
        list.iter().copied().filter(|d| *d >= cut).collect()
    };
    assert_eq!(after_cut(&skipped), after_cut(&full));
    assert!(skipped.len() < 20); // the years before the cut were not generated
}
