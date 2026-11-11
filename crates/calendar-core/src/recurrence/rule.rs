//! An iCalendar recurrence rule (RFC 5545 `RRULE`). Port of `RecurrenceRule.swift`.

use std::collections::{BTreeMap, BTreeSet};

use jiff::Timestamp;

use super::swift_split;
use crate::all_day::CalendarDate;
use crate::civil;
use crate::raw_enum::raw_enum;
use crate::time::Zone;
use crate::write::types::{EventField, WriteError};

/// Text that is not a well-formed RFC 5545 recurrence rule. Reading a rule never throws for a rule that is merely
/// unusual (that is `RecurrenceRule::validate()`'s job when writing); only malformed text does.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RecurrenceParseError {
    #[error("{0}")]
    Malformed(String),
}

raw_enum! {
    pub enum Frequency {
        Secondly = "SECONDLY", Minutely = "MINUTELY", Hourly = "HOURLY",
        Daily = "DAILY", Weekly = "WEEKLY", Monthly = "MONTHLY", Yearly = "YEARLY"
    }
}

raw_enum! {
    pub enum Weekday {
        Monday = "MO", Tuesday = "TU", Wednesday = "WE", Thursday = "TH", Friday = "FR", Saturday = "SA",
        Sunday = "SU"
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WeekdayOccurrence {
    pub weekday: Weekday,
    /// "Second Tuesday" is 2, "last Friday" is -1. Reading accepts -53...53 (yearly rules count weeks of the
    /// year); a writable rule is limited to 1...5 or -5...-1 on monthly and yearly rules.
    pub ordinal: Option<i64>,
}

impl WeekdayOccurrence {
    pub fn new(weekday: Weekday, ordinal: Option<i64>) -> WeekdayOccurrence {
        WeekdayOccurrence { weekday, ordinal }
    }
}

/// How a rule ends (Swift `RecurrenceRule.End`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RuleEnd {
    Never,
    Count(i64),
    /// An instant. Rendered as a UTC date-time for timed events and as a date in the event's zone for all-day ones.
    Until(Timestamp),
}

/// A rule part the library does not model (an `X-` part or a future extension), kept in order.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UnrecognizedPart {
    pub name: String,
    pub value: String,
}

impl UnrecognizedPart {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> UnrecognizedPart {
        UnrecognizedPart {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// An iCalendar recurrence rule (RFC 5545 `RRULE`), used to read every provider's rules and to write the subset every
/// writer can express. `validate()` decides whether a rule can be written: what a writer cannot express is rejected,
/// not mangled. `unrecognized_parts` keeps `X-` parts and future extensions so a read rule can be written back
/// unchanged where a provider allows it. EXDATE and RDATE are not authorable through `EventPatch` (delete one
/// occurrence with `RecurrenceScope::ThisInstance`); they are read through `RecurrenceSet`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RecurrenceRule {
    pub frequency: Frequency,
    pub interval: i64,
    /// The day the week starts on (`WKST`); the iCalendar default is Monday.
    pub week_start: Weekday,
    pub weekdays: Vec<WeekdayOccurrence>,
    pub month_days: Vec<i64>,
    pub months: Vec<i64>,
    pub year_days: Vec<i64>,
    pub week_numbers: Vec<i64>,
    pub set_positions: Vec<i64>,
    pub hours: Vec<i64>,
    pub minutes: Vec<i64>,
    pub seconds: Vec<i64>,
    pub end: RuleEnd,
    pub unrecognized_parts: Vec<UnrecognizedPart>,
}

fn malformed<T>(message: String) -> Result<T, RecurrenceParseError> {
    Err(RecurrenceParseError::Malformed(message))
}

impl RecurrenceRule {
    /// The Swift memberwise `init(frequency:)` with every default: interval 1, week start Monday, no parts, no end.
    /// Set the other fields with struct update syntax.
    pub fn new(frequency: Frequency) -> RecurrenceRule {
        RecurrenceRule {
            frequency,
            interval: 1,
            week_start: Weekday::Monday,
            weekdays: Vec::new(),
            month_days: Vec::new(),
            months: Vec::new(),
            year_days: Vec::new(),
            week_numbers: Vec::new(),
            set_positions: Vec::new(),
            hours: Vec::new(),
            minutes: Vec::new(),
            seconds: Vec::new(),
            end: RuleEnd::Never,
            unrecognized_parts: Vec::new(),
        }
    }

    /// Whether every writer can store the rule: a daily-or-longer frequency and only the parts in the writable
    /// subset (`validate()` fails with `Unsupported` for the rest).
    pub fn is_writable_subset(&self) -> bool {
        !matches!(
            self.frequency,
            Frequency::Secondly | Frequency::Minutely | Frequency::Hourly
        ) && self.week_start == Weekday::Monday
            && self.year_days.is_empty()
            && self.week_numbers.is_empty()
            && self.set_positions.is_empty()
            && self.hours.is_empty()
            && self.minutes.is_empty()
            && self.seconds.is_empty()
            && self.unrecognized_parts.is_empty()
    }

    pub fn validate(&self) -> Result<(), WriteError> {
        let invalid = |message: &str| Err(WriteError::Invalid(message.to_string()));
        if self.interval < 1 {
            return invalid("recurrence interval must be at least 1");
        }
        if let RuleEnd::Count(n) = self.end
            && n < 1
        {
            return invalid("recurrence count must be at least 1");
        }
        if self.frequency == Frequency::Daily && !self.weekdays.is_empty() {
            return Err(unsupported_recurrence());
        }
        let monthly_or_yearly = matches!(self.frequency, Frequency::Monthly | Frequency::Yearly);
        for occurrence in &self.weekdays {
            let Some(ordinal) = occurrence.ordinal else {
                continue;
            };
            if !monthly_or_yearly {
                return invalid("an ordinal weekday needs a monthly or yearly rule");
            }
            if !((1..=5).contains(&ordinal) || (-5..=-1).contains(&ordinal)) {
                return invalid("weekday ordinal must be 1...5 or -5...-1");
            }
        }
        if !self.month_days.is_empty() {
            if !monthly_or_yearly {
                return invalid("month days apply to monthly and yearly rules");
            }
            if !self
                .month_days
                .iter()
                .all(|d| (1..=31).contains(d) || (-31..=-1).contains(d))
            {
                return invalid("month days must be 1...31 or -31...-1");
            }
        }
        if !self.months.is_empty() {
            if self.frequency != Frequency::Yearly {
                return invalid("months apply to yearly rules only");
            }
            if !self.months.iter().all(|m| (1..=12).contains(m)) {
                return invalid("months must be 1...12");
            }
        }
        // A rule read from a provider can be inspected and edited, but is written only when every writer can express it.
        if !self.is_writable_subset() {
            return Err(unsupported_recurrence());
        }
        Ok(())
    }

    // MARK: Parsing

    /// Swift `init(rrule:in:)`. Accepts an optional `RRULE:` prefix and every RFC 5545 rule part; parts the library
    /// does not model go to `unrecognized_parts`. A date-only `UNTIL` and a floating date-time `UNTIL` are read in
    /// `zone` (default UTC). Fails with `RecurrenceParseError::Malformed` for text that is not a well-formed rule
    /// (`INTERVAL=0`, `COUNT` with `UNTIL`, an unknown `FREQ`, an out-of-range value, a duplicate part). Whether a
    /// writer can store the rule is `validate()`'s job, so a rule is never refused at read time just because it is
    /// unusual.
    pub fn from_rrule(
        text: &str,
        zone: Option<&Zone>,
    ) -> Result<RecurrenceRule, RecurrenceParseError> {
        let trimmed = text.trim();
        let mut body = trimmed;
        if trimmed.to_uppercase().starts_with("RRULE:") {
            body = match trimmed.char_indices().nth(6) {
                Some((index, _)) => &trimmed[index..],
                None => "",
            };
        }
        let mut parts: BTreeMap<String, &str> = BTreeMap::new();
        let mut order: Vec<String> = Vec::new();
        for piece in swift_split(body, ';', None, true) {
            let pair = swift_split(piece, '=', Some(1), true);
            if pair.len() != 2 || pair[1].is_empty() {
                return malformed(format!("malformed RRULE part: {piece}"));
            }
            let key = pair[0].to_uppercase();
            if parts.contains_key(&key) {
                return malformed(format!("duplicate RRULE part: {key}"));
            }
            parts.insert(key.clone(), pair[1]);
            order.push(key);
        }
        let Some(freq_text) = parts.get("FREQ") else {
            return malformed("RRULE has no FREQ".to_string());
        };
        let Some(frequency) = Frequency::from_raw_value(&freq_text.to_uppercase()) else {
            return malformed(format!("unknown FREQ: {freq_text}"));
        };
        if parts.contains_key("COUNT") && parts.contains_key("UNTIL") {
            return malformed("RRULE has both COUNT and UNTIL".to_string());
        }

        let mut interval = 1;
        if let Some(text) = parts.get("INTERVAL") {
            match text.parse::<i64>() {
                Ok(value) if value >= 1 => interval = value,
                _ => return malformed(format!("bad INTERVAL: {text}")),
            }
        }
        let mut end = RuleEnd::Never;
        if let Some(text) = parts.get("COUNT") {
            match text.parse::<i64>() {
                Ok(value) if value >= 1 => end = RuleEnd::Count(value),
                _ => return malformed(format!("bad COUNT: {text}")),
            }
        }
        if let Some(text) = parts.get("UNTIL") {
            end = RuleEnd::Until(parse_until(text, zone)?);
        }
        let mut week_start = Weekday::Monday;
        if let Some(text) = parts.get("WKST") {
            match Weekday::from_raw_value(&text.to_uppercase()) {
                Some(day) => week_start = day,
                None => return malformed(format!("bad WKST: {text}")),
            }
        }
        let known: BTreeSet<&str> = [
            "FREQ",
            "INTERVAL",
            "COUNT",
            "UNTIL",
            "WKST",
            "BYDAY",
            "BYMONTHDAY",
            "BYMONTH",
            "BYYEARDAY",
            "BYWEEKNO",
            "BYSETPOS",
            "BYHOUR",
            "BYMINUTE",
            "BYSECOND",
        ]
        .into_iter()
        .collect();
        let unrecognized = order
            .iter()
            .filter(|name| !known.contains(name.as_str()))
            .map(|name| UnrecognizedPart::new(name.clone(), parts.get(name).copied().unwrap_or("")))
            .collect();

        // The arguments of the Swift initializer are evaluated in this order, so the first bad part wins.
        let weekdays = match parts.get("BYDAY") {
            Some(text) => swift_split(text, ',', None, false)
                .into_iter()
                .map(parse_weekday)
                .collect::<Result<Vec<_>, _>>()?,
            None => Vec::new(),
        };
        let month_days = ints(parts.get("BYMONTHDAY").copied(), "BYMONTHDAY", |v| {
            signed(v, 31)
        })?;
        let months = ints(parts.get("BYMONTH").copied(), "BYMONTH", |v| {
            (1..=12).contains(&v)
        })?;
        let year_days = ints(parts.get("BYYEARDAY").copied(), "BYYEARDAY", |v| {
            signed(v, 366)
        })?;
        let week_numbers = ints(parts.get("BYWEEKNO").copied(), "BYWEEKNO", |v| {
            signed(v, 53)
        })?;
        let set_positions = ints(parts.get("BYSETPOS").copied(), "BYSETPOS", |v| {
            signed(v, 366)
        })?;
        let hours = ints(parts.get("BYHOUR").copied(), "BYHOUR", |v| {
            (0..=23).contains(&v)
        })?;
        let minutes = ints(parts.get("BYMINUTE").copied(), "BYMINUTE", |v| {
            (0..=59).contains(&v)
        })?;
        let seconds = ints(parts.get("BYSECOND").copied(), "BYSECOND", |v| {
            (0..=60).contains(&v)
        })?;
        Ok(RecurrenceRule {
            frequency,
            interval,
            week_start,
            weekdays,
            month_days,
            months,
            year_days,
            week_numbers,
            set_positions,
            hours,
            minutes,
            seconds,
            end,
            unrecognized_parts: unrecognized,
        })
    }

    // MARK: Rendering

    /// The RRULE value without the `RRULE:` prefix, in a fixed order (unrecognized parts last, unchanged). Callers
    /// must `validate()` first when writing; this does not.
    pub fn rrule_string(&self, all_day: bool, zone: Option<&Zone>) -> String {
        let mut parts = vec![format!("FREQ={}", self.frequency.as_str())];
        if self.interval > 1 {
            parts.push(format!("INTERVAL={}", self.interval));
        }
        if !self.weekdays.is_empty() {
            let days: Vec<String> = self
                .weekdays
                .iter()
                .map(|w| {
                    format!(
                        "{}{}",
                        w.ordinal.map(|o| o.to_string()).unwrap_or_default(),
                        w.weekday.as_str()
                    )
                })
                .collect();
            parts.push(format!("BYDAY={}", days.join(",")));
        }
        let mut list = |name: &str, values: &[i64]| {
            if !values.is_empty() {
                let joined: Vec<String> = values.iter().map(i64::to_string).collect();
                parts.push(format!("{name}={}", joined.join(",")));
            }
        };
        list("BYMONTHDAY", &self.month_days);
        list("BYMONTH", &self.months);
        list("BYYEARDAY", &self.year_days);
        list("BYWEEKNO", &self.week_numbers);
        list("BYHOUR", &self.hours);
        list("BYMINUTE", &self.minutes);
        list("BYSECOND", &self.seconds);
        list("BYSETPOS", &self.set_positions);
        if self.week_start != Weekday::Monday {
            parts.push(format!("WKST={}", self.week_start.as_str()));
        }
        match self.end {
            RuleEnd::Never => {}
            RuleEnd::Count(n) => parts.push(format!("COUNT={n}")),
            RuleEnd::Until(date) => parts.push(format!(
                "UNTIL={}",
                RecurrenceRule::until_text(date, all_day, zone)
            )),
        }
        parts.extend(
            self.unrecognized_parts
                .iter()
                .map(|p| format!("{}={}", p.name, p.value)),
        );
        parts.join(";")
    }

    /// `yyyyMMdd` in `zone` for all-day events, `yyyyMMdd'T'HHmmss'Z'` in UTC for timed ones.
    pub fn until_text(date: Timestamp, all_day: bool, zone: Option<&Zone>) -> String {
        let utc = Zone::utc();
        let zone = if all_day { zone.unwrap_or(&utc) } else { &utc };
        let c = civil::components(date, zone);
        if all_day {
            return format!("{:04}{:02}{:02}", c.year, c.month, c.day);
        }
        format!(
            "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
            c.year, c.month, c.day, c.hour, c.minute, c.second
        )
    }

    pub fn date_text(date: &CalendarDate) -> String {
        format!("{:04}{:02}{:02}", date.year, date.month, date.day)
    }
}

fn unsupported_recurrence() -> WriteError {
    WriteError::Unsupported {
        fields: [EventField::Recurrence].into(),
    }
}

/// A non-zero value from -`limit` to `limit` (Swift avoids `abs`, which traps on `Int.min`).
fn signed(value: i64, limit: i64) -> bool {
    value != 0 && value >= -limit && value <= limit
}

fn ints(
    text: Option<&str>,
    name: &str,
    valid: impl Fn(i64) -> bool,
) -> Result<Vec<i64>, RecurrenceParseError> {
    let Some(text) = text else {
        return Ok(Vec::new());
    };
    swift_split(text, ',', None, false)
        .into_iter()
        .map(|piece| match piece.parse::<i64>() {
            Ok(value) if valid(value) => Ok(value),
            _ => malformed(format!("bad {name} value: {piece}")),
        })
        .collect()
}

fn parse_weekday(token: &str) -> Result<WeekdayOccurrence, RecurrenceParseError> {
    let upper: Vec<char> = token.to_uppercase().chars().collect();
    let bad = || malformed(format!("bad BYDAY value: {token}"));
    if upper.len() < 2 {
        return bad();
    }
    let split = upper.len() - 2;
    let suffix: String = upper[split..].iter().collect();
    let Some(weekday) = Weekday::from_raw_value(&suffix) else {
        return bad();
    };
    let prefix: String = upper[..split].iter().collect();
    if prefix.is_empty() {
        return Ok(WeekdayOccurrence::new(weekday, None));
    }
    match prefix.parse::<i64>() {
        Ok(ordinal) if signed(ordinal, 53) => Ok(WeekdayOccurrence::new(weekday, Some(ordinal))),
        _ => bad(),
    }
}

fn parse_until(text: &str, zone: Option<&Zone>) -> Result<Timestamp, RecurrenceParseError> {
    let chars: Vec<char> = text.to_uppercase().chars().collect();
    let bad = || malformed(format!("bad UNTIL: {text}"));
    // The ASCII-digit value of `chars[range]`.
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        let digits = chars.get(range)?;
        if !digits.iter().all(char::is_ascii_digit) {
            return None;
        }
        digits.iter().collect::<String>().parse().ok()
    };
    let utc = Zone::utc();
    let series_zone = zone.unwrap_or(&utc);
    if chars.len() == 8 {
        let (Some(year), Some(month), Some(day)) = (number(0..4), number(4..6), number(6..8))
        else {
            return bad();
        };
        let Some(date) = civil::date_from(year, month, day, 0, 0, 0, series_zone) else {
            return bad();
        };
        let back = civil::components(date, series_zone);
        if back.year != year || back.month != month || back.day != day {
            return bad();
        }
        return Ok(date);
    }
    let utc_form = chars.len() == 16 && chars[8] == 'T' && chars[15] == 'Z';
    let floating_form = chars.len() == 15 && chars[8] == 'T';
    if utc_form || floating_form {
        let in_zone = if utc_form { &utc } else { series_zone };
        let (Some(year), Some(month), Some(day)) = (number(0..4), number(4..6), number(6..8))
        else {
            return bad();
        };
        let (Some(hour), Some(minute), Some(second)) =
            (number(9..11), number(11..13), number(13..15))
        else {
            return bad();
        };
        let Some(date) = civil::date_from(year, month, day, hour, minute, second, in_zone) else {
            return bad();
        };
        let back = civil::components(date, in_zone);
        if back.year != year
            || back.month != month
            || back.day != day
            || back.hour != hour
            || back.minute != minute
            || back.second != second
        {
            return bad();
        }
        return Ok(date);
    }
    bad()
}
