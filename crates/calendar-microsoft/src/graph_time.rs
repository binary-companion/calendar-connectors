//! Port of `GraphTime.swift`: Graph's date-time text (`2026-09-25T10:00:00.0000000`, no offset; the zone travels beside
//! it) and ISO 8601 instants with a `Z` or an offset. Pure parsing and formatting, no clock.
//!
//! Swift's `Character` operations (`prefix`, `count`, `suffix`) work on user-perceived characters; this port works on
//! `char`s. The two differ only when a combining mark follows the characters the code looks at, which Graph never
//! sends; [`GraphTime::date`] refuses that one case the way Swift's grapheme counting does, and
//! `graph_time_never_panics_on_multi_byte_or_short_text` pins that nothing panics.

use calendar_core::all_day::CalendarDate;
use calendar_core::civil;
use calendar_core::time::{Zone, add_seconds};
use jiff::Timestamp;

pub(crate) enum GraphTime {}

/// Characters that join the character before them into one Swift `Character` (combining marks, the zero-width joiner,
/// variation selectors, emoji modifiers).
fn extends_previous(c: char) -> bool {
    matches!(
        u32::from(c),
        0x0300..=0x036F
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x20D0..=0x20FF
            | 0xFE00..=0xFE0F
            | 0xFE20..=0xFE2F
            | 0x200D
            | 0x1F3FB..=0x1F3FF
    )
}

impl GraphTime {
    /// `dateTime` read as local time in `zone`. A fractional part is ignored. `None` for text that is not a date-time.
    pub(crate) fn parse(text: &str, zone: &Zone) -> Option<Timestamp> {
        let c = text.as_bytes();
        if c.len() < 19
            || c[4] != b'-'
            || c[7] != b'-'
            || !(c[10] == b'T' || c[10] == b' ')
            || c[13] != b':'
            || c[16] != b':'
        {
            return None;
        }
        let number = |range: std::ops::Range<usize>| -> Option<i64> {
            let mut value: i64 = 0;
            for i in range {
                if !c[i].is_ascii_digit() {
                    return None;
                }
                value = value * 10 + i64::from(c[i] - b'0');
            }
            Some(value)
        };
        let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
        let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
        let date = civil::date_from(year, month, day, hour, minute, second, zone)?;
        // A month or day out of range would roll over into another date; refuse it.
        let back = civil::components(date, zone);
        if back.year != year || back.month != month || back.day != day {
            return None;
        }
        Some(date)
    }

    /// The date part of `dateTime` (`2026-09-25T00:00:00.0000000` gives 2026-09-25).
    pub(crate) fn date(text: &str) -> Option<CalendarDate> {
        let mut chars = text.chars();
        let prefix: String = chars.by_ref().take(10).collect();
        if prefix.chars().count() < 10 {
            return None;
        }
        // A combining mark right after the tenth character would be part of the tenth Swift `Character`.
        if chars.next().is_some_and(extends_previous) {
            return None;
        }
        let parts: Vec<i64> = prefix
            .split('-')
            .filter(|piece| !piece.is_empty())
            .filter_map(|piece| piece.parse::<i64>().ok())
            .collect();
        if parts.len() != 3 {
            return None;
        }
        Some(CalendarDate::new(parts[0], parts[1], parts[2]))
    }

    /// `yyyy-MM-dd'T'HH:mm:ss` in `zone`.
    pub(crate) fn format(date: Timestamp, zone: &Zone) -> String {
        let c = civil::components(date, zone);
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            c.year, c.month, c.day, c.hour, c.minute, c.second
        )
    }

    /// `yyyy-MM-dd'T'00:00:00` for a calendar day. (Used by the write path.)
    #[allow(dead_code)]
    pub(crate) fn midnight(date: &CalendarDate) -> String {
        format!(
            "{:04}-{:02}-{:02}T00:00:00",
            date.year, date.month, date.day
        )
    }

    /// `yyyy-MM-dd` for a calendar day (a recurrence range's `startDate` and `endDate`). (Used by the write path.)
    #[allow(dead_code)]
    pub(crate) fn date_text(date: &CalendarDate) -> String {
        format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
    }

    /// An instant with a `Z` or `+hh:mm` offset (`lastModifiedDateTime`, `originalStart`), with any fractional part.
    /// Text with no offset is read as UTC.
    pub(crate) fn parse_instant(text: &str) -> Option<Timestamp> {
        let mut body: Vec<char> = text.chars().collect();
        let mut offset: f64 = 0.0;
        if matches!(body.last(), Some('Z' | 'z')) {
            body.pop();
        } else if body.len() >= 6 && matches!(body[body.len() - 6], '+' | '-') {
            // Only an offset that follows the time part counts (a date's own hyphens are earlier).
            let tail = &body[body.len() - 6..];
            let rest: String = tail[1..].iter().collect();
            let parts: Vec<i64> = rest
                .split(':')
                .filter(|piece| !piece.is_empty())
                .filter_map(|piece| piece.parse::<i64>().ok())
                .collect();
            if tail[3] == ':' && parts.len() == 2 && body.len() > 19 {
                let sign = if tail[0] == '-' { -1.0 } else { 1.0 };
                // Swift traps on an overflow here; text that large is not an offset.
                let seconds = parts[0]
                    .checked_mul(3600)?
                    .checked_add(parts[1].checked_mul(60)?)?;
                offset = seconds as f64 * sign;
                body.truncate(body.len() - 6);
            }
        }
        let body: String = body.into_iter().collect();
        let local = Self::parse(&body, &Zone::utc())?;
        Some(add_seconds(local, -offset))
    }

    /// `yyyy-MM-dd'T'HH:mm:ss'Z'` in UTC (query parameters such as `startDateTime`).
    pub(crate) fn instant_text(date: Timestamp) -> String {
        format!("{}Z", Self::format(date, &Zone::utc()))
    }
}
