//! Port of `ICalValues.swift`: DATE, DATE-TIME and DURATION values.

use calendar_core::all_day::CalendarDate;
use calendar_core::civil;
use calendar_core::time::Zone;
use jiff::Timestamp;

use crate::component::ICalProperty;
use crate::content_line::is_whitespace;
use crate::grapheme::{characters, split};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LocalDateTime {
    pub date: CalendarDate,
    pub hour: i64,
    pub minute: i64,
    pub second: i64,
}

impl LocalDateTime {
    pub fn new(date: CalendarDate, hour: i64, minute: i64, second: i64) -> LocalDateTime {
        LocalDateTime {
            date,
            hour,
            minute,
            second,
        }
    }
}

/// A DATE or DATE-TIME value as written: a date, a UTC instant, or a wall-clock time in a named zone (`tzid`) or
/// floating (`tzid == None`, read in the calendar's zone).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ICalDateValue {
    Date(CalendarDate),
    Utc(Timestamp),
    Local {
        local: LocalDateTime,
        tzid: Option<String>,
    },
}

pub enum ICalValues {}

impl ICalValues {
    /// The property's first value, read with its `VALUE` and `TZID` parameters; `None` when it is malformed.
    pub fn date_value(property: &ICalProperty) -> Option<ICalDateValue> {
        ICalValues::date_values(property).into_iter().next()
    }

    /// Every value of a list property (`EXDATE`, `RDATE`); empty when any token is malformed or the values are periods.
    pub fn date_values(property: &ICalProperty) -> Vec<ICalDateValue> {
        let kind = property.parameter("VALUE").map(str::to_uppercase);
        if kind.as_deref() == Some("PERIOD") {
            return Vec::new();
        }
        let tzid = property.parameter("TZID");
        let mut values = Vec::new();
        for token in split(&property.value, ',') {
            match ICalValues::parse(token, kind.as_deref() == Some("DATE"), tzid) {
                Some(value) => values.push(value),
                None => return Vec::new(),
            }
        }
        values
    }

    pub(crate) fn parse(token: &str, is_date: bool, tzid: Option<&str>) -> Option<ICalDateValue> {
        let text = token.trim_matches(is_whitespace).to_uppercase();
        let chars: Vec<&str> = characters(&text).collect();
        let number = |from: usize, to: usize| -> Option<i64> {
            if chars.len() < to {
                return None;
            }
            let digits = &chars[from..to];
            if !digits
                .iter()
                .all(|c| c.len() == 1 && c.as_bytes()[0].is_ascii_digit())
            {
                return None;
            }
            digits.concat().parse::<i64>().ok()
        };
        let year = number(0, 4)?;
        let month = number(4, 6)?;
        let day = number(6, 8)?;
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        let date = CalendarDate::new(year, month, day);
        if chars.len() == 8 {
            return Some(ICalDateValue::Date(date));
        }
        if is_date || chars.len() < 15 || chars[8] != "T" {
            return None;
        }
        let hour = number(9, 11)?;
        let minute = number(11, 13)?;
        let second = number(13, 15)?;
        if !(hour < 24 && minute < 60 && second <= 60) {
            return None;
        }
        let local = LocalDateTime::new(date, hour, minute, second.min(59));
        if chars.len() == 16 && chars[15] == "Z" {
            let instant =
                civil::date_from(year, month, day, hour, minute, local.second, &Zone::utc())?;
            return Some(ICalDateValue::Utc(instant));
        }
        if chars.len() != 15 {
            return None;
        }
        Some(ICalDateValue::Local {
            local,
            tzid: tzid.map(str::to_string),
        })
    }

    /// RFC 5545 DURATION (`[+-]P[nW][nD][T[nH][nM][nS]]`) in seconds; `None` when malformed.
    pub fn duration(text: &str) -> Option<f64> {
        let upper = text.trim_matches(is_whitespace).to_uppercase();
        let mut chars = upper.chars().peekable();
        let mut sign = 1.0;
        match chars.peek() {
            Some('-') => {
                sign = -1.0;
                chars.next();
            }
            Some('+') => {
                chars.next();
            }
            _ => {}
        }
        if chars.next() != Some('P') {
            return None;
        }
        let mut total = 0.0;
        let mut number = String::new();
        let mut in_time = false;
        let mut saw_part = false;
        for c in chars {
            match c {
                'T' => {
                    if !number.is_empty() || in_time {
                        return None;
                    }
                    in_time = true;
                }
                '0'..='9' => number.push(c),
                'W' | 'D' | 'H' | 'M' | 'S' => {
                    let n: f64 = number.parse().ok()?;
                    let unit = match (c, in_time) {
                        ('W', false) => 604_800.0,
                        ('D', false) => 86_400.0,
                        ('H', true) => 3600.0,
                        ('M', true) => 60.0,
                        ('S', true) => 1.0,
                        _ => return None,
                    };
                    total += n * unit;
                    number.clear();
                    saw_part = true;
                }
                _ => return None,
            }
        }
        if !number.is_empty() || !saw_part {
            return None;
        }
        Some(sign * total)
    }

    pub fn duration_text(seconds: f64) -> String {
        let mut rest = seconds.abs().round() as i64;
        let sign = if seconds < 0.0 { "-" } else { "" };
        if rest == 0 {
            return "PT0S".to_string();
        }
        let days = rest / 86_400;
        rest %= 86_400;
        let hours = rest / 3600;
        rest %= 3600;
        let minutes = rest / 60;
        let secs = rest % 60;
        let mut text = format!("{sign}P");
        if days > 0 {
            text.push_str(&format!("{days}D"));
        }
        if hours + minutes + secs > 0 {
            text.push('T');
            if hours > 0 {
                text.push_str(&format!("{hours}H"));
            }
            if minutes > 0 {
                text.push_str(&format!("{minutes}M"));
            }
            if secs > 0 {
                text.push_str(&format!("{secs}S"));
            }
        }
        text
    }

    pub fn utc_text(date: Timestamp) -> String {
        ICalValues::local_text(date, &Zone::utc()) + "Z"
    }

    pub fn date_text(date: &CalendarDate) -> String {
        format!("{:04}{:02}{:02}", date.year, date.month, date.day)
    }

    pub fn local_text(date: Timestamp, zone: &Zone) -> String {
        let c = civil::components(date, zone);
        format!(
            "{:04}{:02}{:02}T{:02}{:02}{:02}",
            c.year, c.month, c.day, c.hour, c.minute, c.second
        )
    }
}
