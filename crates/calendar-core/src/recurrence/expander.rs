//! Expansion of a rule into its starts, on wall-clock time. Port of `RuleExpander.swift`.
//!
//! The day arithmetic (`days_between`, `days_in_month`, `week_start_of`, `week_number_of`) is ported literally,
//! integer division and remainder included, as the conventions require; nothing here uses `jiff` date arithmetic.
//! Only [`WallClock::date`] reads a zone for a wall time, and it does so through `civil::date_from`.

use std::collections::BTreeSet;

use jiff::Timestamp;

use super::rule::{Frequency, RecurrenceRule, RuleEnd, Weekday, WeekdayOccurrence};
use crate::all_day::{AllDay, CalendarDate};
use crate::civil;
use crate::time::{Zone, add_seconds, seconds_between};

/// The starts a rule generated, and whether a limit stopped it early.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleExpansion {
    pub starts: Vec<Timestamp>,
    /// True when `limit` or the iteration cap stopped the expansion before the rule's end or the bound.
    pub truncated: bool,
}

impl RecurrenceRule {
    /// The starts this rule generates from `anchor` (iCalendar `DTSTART`), in order. `anchor` is always the first start
    /// (RFC 5545 counts it as the first instance even when the rule would not generate it); then every generated start
    /// after it, until the rule's `COUNT` (which includes the anchor) or `UNTIL` (inclusive), the first start at or
    /// after `bound`, or `limit` starts. Expansion is on wall-clock time in `time_zone`, so a 09:00 rule stays at 09:00
    /// across daylight saving; a local time that does not exist moves forward by the gap and a repeated one uses its
    /// first occurrence. An all-day rule gives canonical all-day starts (`AllDay::start_of_day`).
    ///
    /// `skip_to` lets a caller that only needs starts from some instant on skip whole periods before it without
    /// generating them (a daily series from years ago stays cheap). It is ignored when the rule has a `COUNT`, which
    /// needs every earlier instance. The anchor is still returned first.
    ///
    /// Limitation: `BYWEEKNO` only matches days inside the period's own year (week 1 days in the previous December
    /// are not generated).
    pub fn instances(
        &self,
        anchor: Timestamp,
        time_zone: &Zone,
        is_all_day: bool,
        before: Timestamp,
        limit: i64,
        skip_to: Option<Timestamp>,
    ) -> RuleExpansion {
        RuleExpander::new(self, anchor, time_zone, is_all_day).run(before, limit.max(1), skip_to)
    }
}

struct RuleExpander<'a> {
    rule: &'a RecurrenceRule,
    anchor: Timestamp,
    zone: &'a Zone,
    is_all_day: bool,
    anchor_date: CalendarDate,
    anchor_hour: i64,
    anchor_minute: i64,
    anchor_second: i64,

    // BY-parts after RFC 5545 defaults (see `new`).
    by_month: BTreeSet<i64>,
    by_month_day: Vec<i64>,
    by_weekday: Vec<WeekdayOccurrence>,
    by_year_day: &'a [i64],
    by_week_no: &'a [i64],
    by_hour: &'a [i64],
    by_minute: &'a [i64],
    by_second: &'a [i64],
}

/// One period of the rule: the days it covers (daily and longer) or its first instant (sub-daily).
struct Period {
    start: Timestamp,
    days: Vec<CalendarDate>,
}

impl<'a> RuleExpander<'a> {
    const ITERATION_CAP: i64 = 200_000;

    fn new(
        rule: &'a RecurrenceRule,
        anchor: Timestamp,
        zone: &'a Zone,
        is_all_day: bool,
    ) -> RuleExpander<'a> {
        let c = civil::components(anchor, zone);
        let anchor_date = CalendarDate::new(c.year, c.month, c.day);

        let mut months: BTreeSet<i64> = rule.months.iter().copied().collect();
        let mut month_days = rule.month_days.clone();
        let mut weekdays = rule.weekdays.clone();
        // RFC 5545: parts the rule leaves out come from DTSTART (the same defaults as python-dateutil).
        if rule.week_numbers.is_empty()
            && rule.year_days.is_empty()
            && rule.month_days.is_empty()
            && rule.weekdays.is_empty()
        {
            match rule.frequency {
                Frequency::Yearly => {
                    if months.is_empty() {
                        months = BTreeSet::from([anchor_date.month]);
                    }
                    month_days = vec![anchor_date.day];
                }
                Frequency::Monthly => month_days = vec![anchor_date.day],
                Frequency::Weekly => {
                    weekdays = vec![WeekdayOccurrence::new(weekday_of(&anchor_date), None)];
                }
                _ => {}
            }
        }
        RuleExpander {
            rule,
            anchor,
            zone,
            is_all_day,
            anchor_date,
            anchor_hour: if is_all_day { 0 } else { c.hour },
            anchor_minute: if is_all_day { 0 } else { c.minute },
            anchor_second: if is_all_day { 0 } else { c.second },
            by_month: months,
            by_month_day: month_days,
            by_weekday: weekdays,
            by_year_day: &rule.year_days,
            by_week_no: &rule.week_numbers,
            by_hour: &rule.hours,
            by_minute: &rule.minutes,
            by_second: &rule.seconds,
        }
    }

    // MARK: Driver

    fn run(&self, bound: Timestamp, limit: i64, skip_to: Option<Timestamp>) -> RuleExpansion {
        if self.anchor >= bound {
            return RuleExpansion {
                starts: Vec::new(),
                truncated: false,
            };
        }
        let mut starts = vec![self.anchor];
        let mut produced: i64 = 1;
        let max_count = match self.rule.end {
            RuleEnd::Count(n) => Some(n),
            _ => None,
        };
        let until = match self.rule.end {
            RuleEnd::Until(date) => Some(date),
            _ => None,
        };
        if let Some(max_count) = max_count
            && produced >= max_count
        {
            return RuleExpansion {
                starts,
                truncated: false,
            };
        }
        if starts.len() as i64 >= limit {
            return RuleExpansion {
                starts,
                truncated: true,
            };
        }

        let mut index = if max_count.is_none() {
            self.first_index(skip_to)
        } else {
            0
        };
        let mut iterations: i64 = 0;
        loop {
            iterations += 1;
            if iterations > Self::ITERATION_CAP {
                return RuleExpansion {
                    starts,
                    truncated: true,
                };
            }
            let Some(period) = self.period(index) else {
                break;
            };
            if period.start >= bound {
                break;
            }
            if let Some(until) = until
                && period.start > until
            {
                break;
            }
            for candidate in self.candidates(&period) {
                if candidate <= self.anchor {
                    continue;
                }
                if let Some(until) = until
                    && candidate > until
                {
                    return RuleExpansion {
                        starts,
                        truncated: false,
                    };
                }
                if candidate >= bound {
                    return RuleExpansion {
                        starts,
                        truncated: false,
                    };
                }
                starts.push(candidate);
                produced += 1;
                if let Some(max_count) = max_count
                    && produced >= max_count
                {
                    return RuleExpansion {
                        starts,
                        truncated: false,
                    };
                }
                if starts.len() as i64 >= limit {
                    return RuleExpansion {
                        starts,
                        truncated: true,
                    };
                }
            }
            // Swift traps on overflow here; saturating keeps an absurd INTERVAL a finite loop (the period then
            // fails to resolve and the loop ends).
            index = index.saturating_add(self.rule.interval);
        }
        RuleExpansion {
            starts,
            truncated: false,
        }
    }

    // MARK: Periods

    /// The period `index` periods after the anchor's (a multiple of `interval`).
    ///
    /// A period whose first day is outside the range of a `Timestamp` (years beyond about 9999, reached only through
    /// an absurd `INTERVAL`) starts at `Timestamp::MAX`: Swift's `Date` still represents such a day and the period
    /// starts after every bound, which ends the loop. Swift's `startInstant` falls back to the anchor (`?? anchor`);
    /// here that would spin to the iteration cap and report `truncated`.
    fn period(&self, index: i64) -> Option<Period> {
        let beyond_range = Some(Period {
            start: Timestamp::MAX,
            days: Vec::new(),
        });
        match self.rule.frequency {
            Frequency::Yearly => {
                let Some(year) = self.anchor_date.year.checked_add(index) else {
                    return beyond_range;
                };
                let first = CalendarDate::new(year, 1, 1);
                let Some(start) = AllDay::start_of_day(&first, self.zone) else {
                    return beyond_range;
                };
                Some(Period {
                    start,
                    days: (0..days_in_year(year))
                        .map(|d| first.adding_days(d))
                        .collect(),
                })
            }
            Frequency::Monthly => {
                let Some(total) = (self.anchor_date.month - 1).checked_add(index) else {
                    return beyond_range;
                };
                // `Int(floor(Double(total) / 12))` is a floored division.
                let Some(year) = self.anchor_date.year.checked_add(total.div_euclid(12)) else {
                    return beyond_range;
                };
                let month = ((total % 12) + 12) % 12 + 1;
                let first = CalendarDate::new(year, month, 1);
                let Some(start) = AllDay::start_of_day(&first, self.zone) else {
                    return beyond_range;
                };
                Some(Period {
                    start,
                    days: (0..days_in_month(year, month))
                        .map(|d| first.adding_days(d))
                        .collect(),
                })
            }
            Frequency::Weekly => {
                let first = index.checked_mul(7).and_then(|days| {
                    self.week_start_of(&self.anchor_date)
                        .checked_adding_days(days)
                });
                let Some(first) = first else {
                    return beyond_range;
                };
                let Some(start) = AllDay::start_of_day(&first, self.zone) else {
                    return beyond_range;
                };
                Some(Period {
                    start,
                    days: (0..7).map(|d| first.adding_days(d)).collect(),
                })
            }
            Frequency::Daily => {
                let Some(day) = self.anchor_date.checked_adding_days(index) else {
                    return beyond_range;
                };
                let Some(start) = AllDay::start_of_day(&day, self.zone) else {
                    return beyond_range;
                };
                Some(Period {
                    start,
                    days: vec![day],
                })
            }
            Frequency::Hourly | Frequency::Minutely | Frequency::Secondly => {
                let unit = self.unit_seconds();
                let floored = self.floor_to_unit(self.anchor, unit);
                Some(Period {
                    start: add_seconds(floored, unit * index as f64),
                    days: Vec::new(),
                })
            }
        }
    }

    fn unit_seconds(&self) -> f64 {
        match self.rule.frequency {
            Frequency::Hourly => 3600.0,
            Frequency::Minutely => 60.0,
            _ => 1.0,
        }
    }

    /// With no COUNT, the first period worth generating for `skip_to`: a multiple of `interval`, one interval early for safety.
    fn first_index(&self, skip_to: Option<Timestamp>) -> i64 {
        let Some(skip_to) = skip_to else {
            return 0;
        };
        if skip_to <= self.anchor {
            return 0;
        }
        let target = CalendarDate::from_instant(skip_to, self.zone);
        let since_anchor = seconds_between(self.anchor, skip_to);
        let raw: i64 = match self.rule.frequency {
            Frequency::Yearly => target.year - self.anchor_date.year,
            Frequency::Monthly => {
                (target.year - self.anchor_date.year) * 12 + (target.month - self.anchor_date.month)
            }
            Frequency::Weekly => {
                days_between(
                    &self.week_start_of(&self.anchor_date),
                    &self.week_start_of(&target),
                ) / 7
            }
            Frequency::Daily => days_between(&self.anchor_date, &target),
            Frequency::Hourly => (since_anchor / 3600.0) as i64,
            Frequency::Minutely => (since_anchor / 60.0) as i64,
            Frequency::Secondly => since_anchor as i64,
        };
        let interval = self.rule.interval;
        ((raw / interval - 1).saturating_mul(interval)).max(0)
    }

    // MARK: Candidates

    fn candidates(&self, period: &Period) -> Vec<Timestamp> {
        let mut list: Vec<Timestamp> = Vec::new();
        match self.rule.frequency {
            Frequency::Hourly | Frequency::Minutely | Frequency::Secondly => {
                list = self.sub_daily_candidates(period.start);
            }
            _ => {
                for day in period.days.iter().filter(|day| self.day_matches(day)) {
                    if self.is_all_day {
                        if let Some(start) = AllDay::start_of_day(day, self.zone) {
                            list.push(start);
                        }
                        continue;
                    }
                    for h in or_anchor(self.by_hour, self.anchor_hour) {
                        for m in or_anchor(self.by_minute, self.anchor_minute) {
                            for s in or_anchor(self.by_second, self.anchor_second) {
                                if let Some(date) = WallClock::date(day, h, m, s, self.zone) {
                                    list.push(date);
                                }
                            }
                        }
                    }
                }
            }
        }
        list.sort();
        list.dedup();
        if self.rule.set_positions.is_empty() || list.is_empty() {
            return list;
        }
        let mut picked: Vec<Timestamp> = Vec::new();
        for position in &self.rule.set_positions {
            let i = if *position > 0 {
                position - 1
            } else {
                list.len() as i64 + position
            };
            if i >= 0 && (i as usize) < list.len() {
                picked.push(list[i as usize]);
            }
        }
        picked.sort();
        picked.dedup();
        picked
    }

    fn sub_daily_candidates(&self, period_start: Timestamp) -> Vec<Timestamp> {
        let unit = self.unit_seconds();
        let offsets: Vec<f64> = match self.rule.frequency {
            Frequency::Hourly => {
                let mut offsets = Vec::new();
                for m in or_anchor(self.by_minute, self.anchor_minute) {
                    for s in or_anchor(self.by_second, self.anchor_second) {
                        offsets.push((m * 60 + s) as f64);
                    }
                }
                offsets
            }
            Frequency::Minutely => or_anchor(self.by_second, self.anchor_second)
                .into_iter()
                .map(|s| s as f64)
                .collect(),
            _ => vec![0.0],
        };
        offsets
            .into_iter()
            .filter_map(|offset| {
                if offset >= unit {
                    return None;
                }
                let date = add_seconds(period_start, offset);
                let c = civil::components(date, self.zone);
                let day = CalendarDate::new(c.year, c.month, c.day);
                if !self.day_matches(&day) {
                    return None;
                }
                if !self.by_hour.is_empty() && !self.by_hour.contains(&c.hour) {
                    return None;
                }
                if self.rule.frequency != Frequency::Hourly
                    && !self.by_minute.is_empty()
                    && !self.by_minute.contains(&c.minute)
                {
                    return None;
                }
                if self.rule.frequency == Frequency::Secondly
                    && !self.by_second.is_empty()
                    && !self.by_second.contains(&c.second)
                {
                    return None;
                }
                Some(date)
            })
            .collect()
    }

    fn day_matches(&self, day: &CalendarDate) -> bool {
        if !self.by_month.is_empty() && !self.by_month.contains(&day.month) {
            return false;
        }
        if !self.by_week_no.is_empty() {
            let Some((week, weeks_in_year)) = self.week_number_of(day) else {
                return false;
            };
            if !self
                .by_week_no
                .iter()
                .any(|n| *n == week || *n == week - weeks_in_year - 1)
            {
                return false;
            }
        }
        if !self.by_year_day.is_empty() {
            let n = days_between(&CalendarDate::new(day.year, 1, 1), day) + 1;
            let total = days_in_year(day.year);
            if !self
                .by_year_day
                .iter()
                .any(|d| *d == n || *d == n - total - 1)
            {
                return false;
            }
        }
        if !self.by_month_day.is_empty() {
            let total = days_in_month(day.year, day.month);
            if !self
                .by_month_day
                .iter()
                .any(|d| *d == day.day || *d == day.day - total - 1)
            {
                return false;
            }
        }
        if !self.by_weekday.is_empty()
            && !self.by_weekday.iter().any(|w| self.weekday_matches(w, day))
        {
            return false;
        }
        true
    }

    /// A plain weekday matches any such day; an ordinal counts within the month (monthly rules, and yearly rules with
    /// `BYMONTH`) or within the year (other yearly rules). Other frequencies ignore the ordinal.
    fn weekday_matches(&self, occurrence: &WeekdayOccurrence, day: &CalendarDate) -> bool {
        if weekday_of(day) != occurrence.weekday {
            return false;
        }
        let Some(ordinal) = occurrence.ordinal else {
            return true;
        };
        let frequency = self.rule.frequency;
        let in_month = frequency == Frequency::Monthly
            || (frequency == Frequency::Yearly && !self.by_month.is_empty());
        if !(in_month || frequency == Frequency::Yearly) {
            return true;
        }
        let first = if in_month {
            CalendarDate::new(day.year, day.month, 1)
        } else {
            CalendarDate::new(day.year, 1, 1)
        };
        let length = if in_month {
            days_in_month(day.year, day.month)
        } else {
            days_in_year(day.year)
        };
        let offset = days_between(&first, day);
        let from_start = offset / 7 + 1;
        let from_end = -((length - 1 - offset) / 7 + 1);
        ordinal == from_start || ordinal == from_end
    }

    // MARK: Weeks

    /// The day on or before `day` that is the rule's week start.
    fn week_start_of(&self, day: &CalendarDate) -> CalendarDate {
        let back = (monday_first_index(weekday_of(day)) - monday_first_index(self.rule.week_start)
            + 7)
            % 7;
        day.adding_days(-back)
    }

    /// Week 1 is the first week (starting on `WKST`) with at least four days in the year. Returns the week number and
    /// the number of weeks in that year, or `None` for a day that belongs to the neighbouring year's weeks.
    fn week_number_of(&self, day: &CalendarDate) -> Option<(i64, i64)> {
        let first_week = self.week_start_of(&CalendarDate::new(day.year, 1, 4));
        let next_first_week = self.week_start_of(&CalendarDate::new(day.year + 1, 1, 4));
        if !(*day >= first_week && *day < next_first_week) {
            return None;
        }
        Some((
            days_between(&first_week, day) / 7 + 1,
            days_between(&first_week, &next_first_week) / 7,
        ))
    }

    // MARK: Date helpers

    fn floor_to_unit(&self, date: Timestamp, unit: f64) -> Timestamp {
        let c = civil::components(date, self.zone);
        let parts = if unit == 3600.0 {
            (c.year, c.month, c.day, c.hour, 0, 0)
        } else if unit == 60.0 {
            (c.year, c.month, c.day, c.hour, c.minute, 0)
        } else {
            (c.year, c.month, c.day, c.hour, c.minute, c.second)
        };
        civil::date_from(
            parts.0, parts.1, parts.2, parts.3, parts.4, parts.5, self.zone,
        )
        .unwrap_or(date)
    }
}

/// `byX.isEmpty ? [anchorX] : byX.sorted()`.
fn or_anchor(by: &[i64], anchor: i64) -> Vec<i64> {
    if by.is_empty() {
        return vec![anchor];
    }
    let mut sorted = by.to_vec();
    sorted.sort();
    sorted
}

fn monday_first_index(weekday: Weekday) -> i64 {
    match weekday {
        Weekday::Monday => 0,
        Weekday::Tuesday => 1,
        Weekday::Wednesday => 2,
        Weekday::Thursday => 3,
        Weekday::Friday => 4,
        Weekday::Saturday => 5,
        Weekday::Sunday => 6,
    }
}

/// Noon of `day` in UTC. `RuleExpander.weekday(of:)` and `daysBetween` build it with a force unwrap; a year outside
/// the range of a `Date` returns `None` here instead of trapping.
fn utc_noon(day: &CalendarDate) -> Option<Timestamp> {
    civil::date_from(day.year, day.month, day.day, 12, 0, 0, &Zone::utc())
}

fn weekday_of(day: &CalendarDate) -> Weekday {
    let Some(noon) = utc_noon(day) else {
        return Weekday::Monday; // Swift: `utc.date(from:)!` traps for a day outside the range of a `Date`
    };
    // Calendar weekday: 1 = Sunday ... 7 = Saturday.
    let index = civil::components(noon, &Zone::utc()).weekday;
    [
        Weekday::Sunday,
        Weekday::Monday,
        Weekday::Tuesday,
        Weekday::Wednesday,
        Weekday::Thursday,
        Weekday::Friday,
        Weekday::Saturday,
    ][(index - 1).clamp(0, 6) as usize]
}

fn days_between(a: &CalendarDate, b: &CalendarDate) -> i64 {
    let (Some(da), Some(db)) = (utc_noon(a), utc_noon(b)) else {
        return 0; // Swift: `utc.date(from:)!` traps for a day outside the range of a `Date`
    };
    // Both are noon UTC, so the difference is a whole number of days (Swift divides doubles and rounds).
    (db.as_second() - da.as_second()) / 86_400
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 => {
            if is_leap(year) {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn days_in_year(year: i64) -> i64 {
    if is_leap(year) { 366 } else { 365 }
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Wall-clock times in a zone, resolved the way iCalendar needs them: a time skipped by a daylight-saving jump moves
/// forward by the gap, and a time that happens twice uses the first. Computed from offsets rather than `Calendar`
/// matching policies so it behaves the same on swift-corelibs-foundation.
pub enum WallClock {}

impl WallClock {
    /// `WallClock.date(_:hour:minute:second:in:)`. `civil::date_from` implements this exact algorithm.
    pub fn date(
        day: &CalendarDate,
        hour: i64,
        minute: i64,
        second: i64,
        zone: &Zone,
    ) -> Option<Timestamp> {
        civil::date_from(day.year, day.month, day.day, hour, minute, second, zone)
    }
}

impl CalendarDate {
    /// Swift `CalendarDate.init(_ date: Date, calendar: Calendar)`: the day of `date` in `zone`.
    pub fn from_instant(date: Timestamp, zone: &Zone) -> CalendarDate {
        let c = civil::components(date, zone);
        CalendarDate::new(c.year, c.month, c.day)
    }

    /// Whole days from `self` to `other` (negative when `other` is earlier).
    pub fn days_to(&self, other: &CalendarDate) -> i64 {
        days_between(self, other)
    }
}
