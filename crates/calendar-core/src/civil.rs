//! Gregorian calendar arithmetic that matches Foundation's `Calendar(identifier: .gregorian)` with a time zone
//! (conventions section 4). Pinned by `vectors/zones/*`.

use jiff::Timestamp;

use crate::time::Zone;

/// `Calendar.dateComponents` for year to second, plus the weekday (1 = Sunday).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Components {
    /// Year.
    pub year: i64,
    /// Month, 1 to 12.
    pub month: i64,
    /// Day of the month, starting at 1.
    pub day: i64,
    /// Hour, 0 to 23.
    pub hour: i64,
    /// Minute, 0 to 59.
    pub minute: i64,
    /// Second, 0 to 59.
    pub second: i64,
    /// 1 = Sunday ... 7 = Saturday, as Foundation numbers it.
    pub weekday: i64,
}

/// The calendar fields of `t` as seen in `zone`.
pub fn components(t: Timestamp, zone: &Zone) -> Components {
    let dt = zone.as_jiff().to_datetime(t);
    Components {
        year: i64::from(dt.year()),
        month: i64::from(dt.month()),
        day: i64::from(dt.day()),
        hour: i64::from(dt.hour()),
        minute: i64::from(dt.minute()),
        second: i64::from(dt.second()),
        weekday: i64::from(dt.weekday().to_sunday_one_offset()),
    }
}

/// `Calendar.startOfDay(for:)`: the first instant of the day of `t` in `zone`. A midnight skipped by a daylight-saving
/// jump moves forward by the gap.
pub fn start_of_day(t: Timestamp, zone: &Zone) -> Timestamp {
    let c = components(t, zone);
    date_from(c.year, c.month, c.day, 0, 0, 0, zone).unwrap_or(t)
}

/// `Calendar.date(from: DateComponents(...))` in `zone`. Fields out of range roll over as Foundation's lenient
/// calendar does (month 13 is January of the next year, hour 25 is 01:00 the next day). A wall time skipped by a
/// daylight-saving jump moves forward by the gap, and one that happens twice resolves to the first; the algorithm is
/// `WallClock.date` of `RuleExpander.swift`. `None` when the result is outside the range of `Timestamp`.
pub fn date_from(
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    zone: &Zone,
) -> Option<Timestamp> {
    if year.unsigned_abs() > 1_000_000 {
        return None;
    }
    let months = i128::from(year) * 12 + i128::from(month) - 1;
    let (y, m) = (months.div_euclid(12), months.rem_euclid(12) + 1);
    let day_number = days_from_civil(i64::try_from(y).ok()?, m as i64)? + i128::from(day) - 1;
    let wall = day_number * 86_400
        + i128::from(hour) * 3600
        + i128::from(minute) * 60
        + i128::from(second);
    let noon = day_number * 86_400 + 43_200;

    let offset_at = |seconds: i128| -> Option<i128> {
        let ts = Timestamp::from_second(i64::try_from(seconds).ok()?).ok()?;
        Some(i128::from(zone.offset_seconds(ts)))
    };
    let before = offset_at(noon - 86_400)?;
    let after = offset_at(noon + 86_400)?;
    let early = wall - before; // read with the offset in force before a transition that day
    let late = wall - after; // and with the offset after it
    let early_ok = offset_at(early)? == before;
    let late_ok = offset_at(late)? == after;
    let chosen = match (early_ok, late_ok) {
        (true, true) => early.min(late), // equal on an ordinary day; the first of a repeated time
        (true, false) => early,
        (false, true) => late,
        (false, false) => early, // a skipped time, moved forward by the gap
    };
    Timestamp::from_second(i64::try_from(chosen).ok()?).ok()
}

/// Days from 1970-01-01 to the first of `month` (1 to 12) of `year`, proleptic Gregorian.
fn days_from_civil(year: i64, month: i64) -> Option<i128> {
    let y = i128::from(year) - i128::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (i128::from(month) + 9) % 12; // March = 0
    let doy = (153 * mp + 2) / 5; // day of year of the 1st
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// `Calendar.Component` values `add` supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// One second.
    Second,
    /// One minute.
    Minute,
    /// One hour.
    Hour,
    /// One calendar day (not always 24 hours across a daylight-saving change).
    Day,
    /// One calendar month.
    Month,
    /// One calendar year.
    Year,
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// `Calendar.date(byAdding:value:to:)`. Seconds, minutes and hours are elapsed time. Days keep the wall-clock time of
/// day; months and years also keep it and clamp the day to the end of a shorter month (31 January plus one month is
/// the last day of February). A wall time skipped by a daylight-saving jump moves forward. `None` on overflow.
pub fn add(unit: Unit, n: i64, to: Timestamp, zone: &Zone) -> Option<Timestamp> {
    let elapsed = |per_unit: i128| -> Option<Timestamp> {
        let total = to.as_nanosecond().checked_add(
            i128::from(n)
                .checked_mul(per_unit)?
                .checked_mul(1_000_000_000)?,
        )?;
        Timestamp::from_nanosecond(total).ok()
    };
    match unit {
        Unit::Second => elapsed(1),
        Unit::Minute => elapsed(60),
        Unit::Hour => elapsed(3600),
        Unit::Day | Unit::Month | Unit::Year => {
            let c = components(to, zone);
            let sub_nanos = to.as_nanosecond().rem_euclid(1_000_000_000);
            let (year, month, day) = match unit {
                Unit::Day => (c.year, c.month, c.day.checked_add(n)?),
                _ => {
                    let months = if unit == Unit::Year {
                        n.checked_mul(12)?
                    } else {
                        n
                    };
                    let total =
                        i128::from(c.year) * 12 + i128::from(c.month) - 1 + i128::from(months);
                    let year = i64::try_from(total.div_euclid(12)).ok()?;
                    let month = total.rem_euclid(12) as i64 + 1;
                    (year, month, c.day.min(days_in_month(year, month)))
                }
            };
            let whole = date_from(year, month, day, c.hour, c.minute, c.second, zone)?;
            Timestamp::from_nanosecond(whole.as_nanosecond().checked_add(sub_nanos)?).ok()
        }
    }
}
