//! All-day dates and their canonical instants. Port of `AllDay.swift`.

use jiff::Timestamp;

use crate::civil::{self, Unit};
use crate::time::Zone;

/// A calendar day without a time or zone. Ordered chronologically (year, then month, then day).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CalendarDate {
    /// Year in the proleptic Gregorian calendar.
    pub year: i64,
    /// Month, 1 to 12.
    pub month: i64,
    /// Day of the month, starting at 1.
    pub day: i64,
}

impl CalendarDate {
    /// Creates a date; the values are not range checked.
    pub fn new(year: i64, month: i64, day: i64) -> CalendarDate {
        CalendarDate { year, month, day }
    }

    /// The day `days` after (or, when negative, before) this one. Computed from noon in UTC, as the Swift code does.
    /// Swift traps when the result is outside the range of a `Date`; this returns the date unchanged instead.
    pub fn adding_days(&self, days: i64) -> CalendarDate {
        self.checked_adding_days(days).unwrap_or(*self)
    }

    /// `adding_days`, or `None` when the result is outside the range of a `Timestamp` (about years -9999 to 9999).
    pub(crate) fn checked_adding_days(&self, days: i64) -> Option<CalendarDate> {
        let utc = Zone::utc();
        let base = civil::date_from(self.year, self.month, self.day, 12, 0, 0, &utc)?;
        let moved = civil::add(Unit::Day, days, base, &utc)?;
        let c = civil::components(moved, &utc);
        Some(CalendarDate::new(c.year, c.month, c.day))
    }
}

/// Conversions between all-day dates and the library's canonical instants: `start` is the start of the first
/// day in `zone`, `end` the start of the day after the last (exclusive). Connectors call this instead of doing
/// date math themselves.
pub enum AllDay {}

impl AllDay {
    /// The start of `date` in `zone`. Built from noon so a zone whose midnight does not exist that day still
    /// yields a valid instant (the first moment of the day).
    pub fn start_of_day(date: &CalendarDate, zone: &Zone) -> Option<Timestamp> {
        let noon = civil::date_from(date.year, date.month, date.day, 12, 0, 0, zone)?;
        Some(civil::start_of_day(noon, zone))
    }

    /// The calendar day that `instant` falls on in `zone`.
    pub fn date_of(instant: Timestamp, zone: &Zone) -> CalendarDate {
        let c = civil::components(instant, zone);
        CalendarDate::new(c.year, c.month, c.day)
    }

    /// `(start, end)` for the days `first` up to, not including, `end_exclusive`.
    pub fn canonical(
        first: &CalendarDate,
        end_exclusive: &CalendarDate,
        zone: &Zone,
    ) -> Option<(Timestamp, Timestamp)> {
        let start = AllDay::start_of_day(first, zone)?;
        let end = AllDay::start_of_day(end_exclusive, zone)?;
        Some((start, end))
    }

    /// The dates a canonical range covers, read in `zone`: `(first, end_exclusive)`.
    pub fn dates(start: Timestamp, end: Timestamp, zone: &Zone) -> (CalendarDate, CalendarDate) {
        (AllDay::date_of(start, zone), AllDay::date_of(end, zone))
    }

    /// For providers that report the last covered day: the exclusive end date.
    pub fn end_exclusive_after_last(last: &CalendarDate) -> CalendarDate {
        last.adding_days(1)
    }
}
