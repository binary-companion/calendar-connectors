//! A series' repetition: rules, extra and skipped dates. Port of `RecurrenceSet.swift`.

use jiff::Timestamp;

use super::rule::RecurrenceRule;
use super::swift_split;
use crate::all_day::{AllDay, CalendarDate};
use crate::civil;
use crate::time::Zone;

/// Everything that makes up a series' repetition: its rules and the extra and skipped dates. Read from a provider's
/// iCalendar lines (`RRULE`, `EXDATE`, `RDATE`) and written back unchanged. Connectors whose server expands occurrences
/// (Google, Microsoft, EventKit) never expand; a connector that reads raw iCalendar (CalDAV) expands with
/// [`RecurrenceSet::occurrences`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RecurrenceSet {
    pub rules: Vec<RecurrenceRule>,
    /// Extra occurrences (RDATE). `None` means the source cannot say (EventKit); `Some([])` means there are none.
    pub extra_dates: Option<Vec<Timestamp>>,
    /// Skipped occurrences (EXDATE). `None` means the source cannot say (EventKit); `Some([])` means there are none.
    pub excluded_dates: Option<Vec<Timestamp>>,
    /// Whole lines the library does not model (`EXRULE`, `RDATE;VALUE=PERIOD`, a rule that is malformed, dates in a
    /// zone it cannot resolve such as a Windows zone name), kept verbatim so a write does not lose them.
    pub unparsed: Vec<String>,
}

impl Default for RecurrenceSet {
    /// The Swift initializer's defaults: no rules, no extra or skipped dates (empty, not unknown), nothing unparsed.
    fn default() -> RecurrenceSet {
        RecurrenceSet {
            rules: Vec::new(),
            extra_dates: Some(Vec::new()),
            excluded_dates: Some(Vec::new()),
            unparsed: Vec::new(),
        }
    }
}

impl RecurrenceSet {
    /// A set of `rules` with the other fields at their defaults.
    pub fn new(rules: Vec<RecurrenceRule>) -> RecurrenceSet {
        RecurrenceSet {
            rules,
            ..RecurrenceSet::default()
        }
    }

    // MARK: Reading

    /// Swift `init(iCalendarLines:timeZone:isAllDay:)`. Reads a provider's recurrence lines. A read never fails:
    /// anything it does not understand goes to `unparsed`. `time_zone` and `is_all_day` come from the series' first
    /// event: floating date-times and date-only values are read in that zone (all-day dates through `AllDay`, so they
    /// land on the canonical midnight).
    pub fn from_icalendar_lines<S: AsRef<str>>(
        lines: &[S],
        time_zone: &Zone,
        is_all_day: bool,
    ) -> RecurrenceSet {
        let mut rules: Vec<RecurrenceRule> = Vec::new();
        let mut extra: Vec<Timestamp> = Vec::new();
        let mut excluded: Vec<Timestamp> = Vec::new();
        let mut unparsed: Vec<String> = Vec::new();
        for line in lines {
            let line = line.as_ref();
            let trimmed = line.trim();
            let Some(colon) = trimmed.find(':') else {
                unparsed.push(line.to_string());
                continue;
            };
            let head = swift_split(&trimmed[..colon], ';', None, true);
            let value = &trimmed[colon + 1..];
            let name = head.first().map(|n| n.to_uppercase());
            match name.as_deref() {
                Some("RRULE") => match RecurrenceRule::from_rrule(value, Some(time_zone)) {
                    Ok(rule) => rules.push(rule),
                    Err(_) => unparsed.push(line.to_string()),
                },
                Some(kind @ ("EXDATE" | "RDATE")) => {
                    match parse_dates(&head[1..], value, time_zone, is_all_day) {
                        Some(dates) => {
                            if kind == "EXDATE" {
                                excluded.extend(dates);
                            } else {
                                extra.extend(dates);
                            }
                        }
                        None => unparsed.push(line.to_string()),
                    }
                }
                _ => unparsed.push(line.to_string()),
            }
        }
        RecurrenceSet {
            rules,
            extra_dates: Some(extra),
            excluded_dates: Some(excluded),
            unparsed,
        }
    }

    // MARK: Writing

    /// The lines to store: one `RRULE` per rule, then `RDATE` and `EXDATE` (a date-time list per zone, `VALUE=DATE`
    /// for all-day series), then the unparsed lines, unchanged.
    pub fn icalendar_lines(&self, time_zone: &Zone, is_all_day: bool) -> Vec<String> {
        let mut lines: Vec<String> = self
            .rules
            .iter()
            .map(|rule| format!("RRULE:{}", rule.rrule_string(is_all_day, Some(time_zone))))
            .collect();
        if let Some(extra) = self.extra_dates.as_ref().filter(|d| !d.is_empty()) {
            lines.push(line("RDATE", extra, time_zone, is_all_day));
        }
        if let Some(excluded) = self.excluded_dates.as_ref().filter(|d| !d.is_empty()) {
            lines.push(line("EXDATE", excluded, time_zone, is_all_day));
        }
        lines.extend(self.unparsed.iter().cloned());
        lines
    }
}

/// `None` when the value is a period, has an unknown zone, or is malformed.
fn parse_dates(
    params: &[&str],
    value: &str,
    zone: &Zone,
    is_all_day: bool,
) -> Option<Vec<Timestamp>> {
    let _ = is_all_day; // Swift passes it and does not read it
    let mut tzid: Option<Zone> = None;
    let mut is_date = false;
    for param in params {
        let pair = swift_split(param, '=', Some(1), true);
        if pair.len() != 2 {
            return None;
        }
        match pair[0].to_uppercase().as_str() {
            "TZID" => tzid = Some(Zone::named(pair[1])?),
            "VALUE" => match pair[1].to_uppercase().as_str() {
                "DATE" => is_date = true,
                "DATE-TIME" => {}
                _ => return None, // PERIOD
            },
            _ => {}
        }
    }
    let mut dates: Vec<Timestamp> = Vec::new();
    for token in swift_split(value, ',', None, true) {
        dates.push(parse_one(token, tzid.as_ref(), zone, is_date)?);
    }
    if dates.is_empty() { None } else { Some(dates) }
}

fn parse_one(text: &str, tzid: Option<&Zone>, zone: &Zone, is_date: bool) -> Option<Timestamp> {
    let chars: Vec<char> = text.to_uppercase().chars().collect();
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        let digits = chars.get(range)?;
        if !digits.iter().all(char::is_ascii_digit) {
            return None;
        }
        digits.iter().collect::<String>().parse().ok()
    };
    let (year, month, day) = (number(0..4)?, number(4..6)?, number(6..8)?);
    if is_date || chars.len() == 8 {
        if chars.len() != 8 {
            return None;
        }
        return AllDay::start_of_day(&CalendarDate::new(year, month, day), zone);
    }
    if chars.len() < 15 || chars[8] != 'T' {
        return None;
    }
    let (hour, minute, second) = (number(9..11)?, number(11..13)?, number(13..15)?);
    let is_utc = chars.len() == 16 && chars[15] == 'Z';
    if !(chars.len() == 15 || is_utc) {
        return None;
    }
    let utc = Zone::utc();
    let in_zone = if is_utc { &utc } else { tzid.unwrap_or(zone) };
    let date = civil::date_from(year, month, day, hour, minute, second, in_zone)?;
    let back = civil::components(date, in_zone);
    if back.year != year
        || back.month != month
        || back.day != day
        || back.hour != hour
        || back.minute != minute
        || back.second != second
    {
        return None;
    }
    Some(date)
}

fn line(name: &str, dates: &[Timestamp], zone: &Zone, is_all_day: bool) -> String {
    if is_all_day {
        let days: Vec<String> = dates
            .iter()
            .map(|date| RecurrenceRule::date_text(&AllDay::date_of(*date, zone)))
            .collect();
        return format!("{name};VALUE=DATE:{}", days.join(","));
    }
    let is_utc = zone.identifier() == "GMT" || zone.identifier() == "UTC";
    let texts: Vec<String> = dates
        .iter()
        .map(|date| {
            let c = civil::components(*date, zone);
            format!(
                "{:04}{:02}{:02}T{:02}{:02}{:02}{}",
                c.year,
                c.month,
                c.day,
                c.hour,
                c.minute,
                c.second,
                if is_utc { "Z" } else { "" }
            )
        })
        .collect();
    let prefix = if is_utc {
        format!("{name}:")
    } else {
        format!("{name};TZID={}:", zone.identifier())
    };
    format!("{prefix}{}", texts.join(","))
}

/// A recurring series as its own object: the first occurrence's anchor time and everything that repeats it.
/// Instances stay parented to a series through `SeriesInfo`; the series owns the rule.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CalendarSeries {
    pub series_id: String,
    pub calendar_id: String,
    /// The rule's anchor time (iCalendar `DTSTART`); an all-day series uses the canonical all-day form.
    pub start: Timestamp,
    pub time_zone: Zone,
    pub is_all_day: bool,
    pub recurrence: RecurrenceSet,
}

impl CalendarSeries {
    pub fn new(
        series_id: impl Into<String>,
        calendar_id: impl Into<String>,
        start: Timestamp,
        time_zone: Zone,
        is_all_day: bool,
        recurrence: RecurrenceSet,
    ) -> CalendarSeries {
        CalendarSeries {
            series_id: series_id.into(),
            calendar_id: calendar_id.into(),
            start,
            time_zone,
            is_all_day,
            recurrence,
        }
    }
}
