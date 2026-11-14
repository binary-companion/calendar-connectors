//! Port of `GraphRecurrenceMapper.swift`: Graph's `patternedRecurrence` to and from `RecurrenceRule`. Reading
//! understands every pattern Graph produces. Writing accepts only what Graph can express and returns
//! `WriteError::Unsupported { fields: [Recurrence] }` for the rest; it never mangles a rule.
//!
//! Swift keeps the recurrence as `[String: Any]`; here it is a `serde_json::Map`. Swift reads its numbers with
//! `as? Int`; [`int_of`] reads a JSON integer, or a number with no fraction, the same way.

use calendar_core::all_day::{AllDay, CalendarDate};
use calendar_core::civil;
use calendar_core::recurrence::rule::{
    Frequency, RecurrenceRule, RuleEnd, Weekday, WeekdayOccurrence,
};
use calendar_core::time::{Zone, add_seconds};
use calendar_core::windows_time_zones::WindowsTimeZones;
use calendar_core::write::types::{EventField, WriteError};
use serde_json::{Map, Value, json};

use crate::dtos::{GraphRecurrenceDTO, GraphRecurrenceRange};
use crate::graph_time::GraphTime;

pub(crate) type Json = Map<String, Value>;

pub(crate) enum GraphRecurrenceMapper {}

const WEEKDAYS: [(&str, Weekday); 7] = [
    ("monday", Weekday::Monday),
    ("tuesday", Weekday::Tuesday),
    ("wednesday", Weekday::Wednesday),
    ("thursday", Weekday::Thursday),
    ("friday", Weekday::Friday),
    ("saturday", Weekday::Saturday),
    ("sunday", Weekday::Sunday),
];

/// Graph's `index` is first to fourth or last (there is no fifth).
const INDEXES: [(&str, i64); 5] = [
    ("first", 1),
    ("second", 2),
    ("third", 3),
    ("fourth", 4),
    ("last", -1),
];

/// Calendar weekdays run Sunday = 1 ... Saturday = 7.
const NAMES: [&str; 7] = [
    "sunday",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
];

fn unsupported() -> WriteError {
    WriteError::Unsupported {
        fields: [EventField::Recurrence].into(),
    }
}

/// Swift's `value as? Int` on a value read from JSON: an integer, or a number with no fraction.
pub(crate) fn int_of(value: Option<&Value>) -> Option<i64> {
    let number = value?.as_number()?;
    if let Some(whole) = number.as_i64() {
        return Some(whole);
    }
    match number.as_f64() {
        Some(float) if float.fract() == 0.0 && float.abs() < 9.0e18 => Some(float as i64),
        _ => None,
    }
}

fn str_of<'a>(map: &'a Json, key: &str) -> Option<&'a str> {
    map.get(key).and_then(Value::as_str)
}

fn object_of<'a>(map: &'a Json, key: &str) -> Option<&'a Json> {
    map.get(key).and_then(Value::as_object)
}

fn weekday_named(name: &str) -> Option<Weekday> {
    let lowered = name.to_lowercase();
    WEEKDAYS
        .iter()
        .find(|(candidate, _)| *candidate == lowered)
        .map(|(_, day)| *day)
}

fn name_of(day: Weekday) -> &'static str {
    WEEKDAYS
        .iter()
        .find(|(_, candidate)| *candidate == day)
        .map_or("monday", |(name, _)| name)
}

/// Days from 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The inverse of [`days_from_civil`]: `(year, month, day)`.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

/// 1 = Sunday ... 7 = Saturday, as `Calendar.component(.weekday, from:)` numbers it.
fn weekday_number(days: i64) -> i64 {
    (days + 4).rem_euclid(7) + 1
}

fn days_in_month(year: i64, month: i64) -> i64 {
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

impl GraphRecurrenceMapper {
    // MARK: Reading

    /// `None` for a pattern this mapper does not know. `zone` is the event's zone, used when the range names none.
    pub(crate) fn rule(dto: &GraphRecurrenceDTO, zone: &Zone) -> Option<RecurrenceRule> {
        let pattern = dto.pattern.as_ref()?;
        let kind = pattern.kind.as_deref()?;
        let interval = pattern.interval.unwrap_or(1).max(1);
        let week_start = pattern
            .first_day_of_week
            .as_deref()
            .and_then(weekday_named)
            .unwrap_or(Weekday::Monday);
        let end = Self::end_of(dto.range.as_ref(), zone);
        let plain: Vec<WeekdayOccurrence> = pattern
            .days_of_week
            .iter()
            .flatten()
            .filter_map(|name| weekday_named(name))
            .map(|day| WeekdayOccurrence::new(day, None))
            .collect();
        let index = pattern.index.as_deref().map(str::to_lowercase);
        let ordinal = INDEXES
            .iter()
            .find(|(name, _)| Some(*name) == index.as_deref())
            .map(|(_, ordinal)| *ordinal);
        let counted: Vec<WeekdayOccurrence> = plain
            .iter()
            .map(|day| WeekdayOccurrence::new(day.weekday, ordinal))
            .collect();
        let make = |frequency: Frequency,
                    weekdays: Vec<WeekdayOccurrence>,
                    month_days: Vec<i64>,
                    months: Vec<i64>|
         -> RecurrenceRule {
            RecurrenceRule {
                interval,
                weekdays,
                month_days,
                months,
                end,
                week_start,
                ..RecurrenceRule::new(frequency)
            }
        };
        let day_of_month: Vec<i64> = pattern.day_of_month.into_iter().collect();
        let month: Vec<i64> = pattern.month.into_iter().collect();
        match kind {
            "daily" => Some(make(Frequency::Daily, Vec::new(), Vec::new(), Vec::new())),
            "weekly" => Some(make(Frequency::Weekly, plain, Vec::new(), Vec::new())),
            "absoluteMonthly" => Some(make(
                Frequency::Monthly,
                Vec::new(),
                day_of_month,
                Vec::new(),
            )),
            "relativeMonthly" => Some(make(Frequency::Monthly, counted, Vec::new(), Vec::new())),
            "absoluteYearly" => Some(make(Frequency::Yearly, Vec::new(), day_of_month, month)),
            "relativeYearly" => Some(make(Frequency::Yearly, counted, Vec::new(), month)),
            _ => None,
        }
    }

    fn end_of(range: Option<&GraphRecurrenceRange>, zone: &Zone) -> RuleEnd {
        match range.and_then(|range| range.kind.as_deref()) {
            Some("numbered") => RuleEnd::Count(
                range
                    .and_then(|range| range.number_of_occurrences)
                    .unwrap_or(1)
                    .max(1),
            ),
            Some("endDate") => {
                let range_zone = range
                    .and_then(|range| range.recurrence_time_zone.as_deref())
                    .and_then(WindowsTimeZones::time_zone_for)
                    .unwrap_or_else(|| zone.clone());
                let Some(last) = range
                    .and_then(|range| range.end_date.as_deref())
                    .and_then(GraphTime::date)
                else {
                    return RuleEnd::Never;
                };
                let Some(after) = AllDay::start_of_day(&last.adding_days(1), &range_zone) else {
                    return RuleEnd::Never;
                };
                // The end of the last day.
                RuleEnd::Until(add_seconds(after, -1.0))
            }
            _ => RuleEnd::Never,
        }
    }

    // MARK: Writing

    /// The `recurrence` object for an event whose first occurrence is on `start` (a date in `zone`).
    pub(crate) fn recurrence(
        rule: &RecurrenceRule,
        start: &CalendarDate,
        zone: &Zone,
    ) -> Result<Json, WriteError> {
        rule.validate()?;
        let mut pattern = Json::new();
        pattern.insert("interval".to_string(), json!(rule.interval));
        pattern.insert(
            "firstDayOfWeek".to_string(),
            json!(name_of(rule.week_start)),
        );
        match rule.frequency {
            Frequency::Daily => {
                pattern.insert("type".to_string(), json!("daily"));
            }
            Frequency::Weekly => {
                pattern.insert("type".to_string(), json!("weekly"));
                let days: Vec<&str> = if rule.weekdays.is_empty() {
                    vec![Self::weekday_name_of(start)?]
                } else {
                    rule.weekdays
                        .iter()
                        .map(|day| name_of(day.weekday))
                        .collect()
                };
                pattern.insert("daysOfWeek".to_string(), json!(days));
            }
            Frequency::Monthly => {
                if rule.weekdays.is_empty() {
                    let days = if rule.month_days.is_empty() {
                        vec![start.day]
                    } else {
                        rule.month_days.clone()
                    };
                    if days.len() != 1 || days[0] < 1 {
                        return Err(unsupported());
                    }
                    pattern.insert("type".to_string(), json!("absoluteMonthly"));
                    pattern.insert("dayOfMonth".to_string(), json!(days[0]));
                } else {
                    if !rule.month_days.is_empty() {
                        return Err(unsupported());
                    }
                    pattern.insert("type".to_string(), json!("relativeMonthly"));
                    Self::add_relative(&rule.weekdays, &mut pattern)?;
                }
            }
            Frequency::Yearly => {
                let months = if rule.months.is_empty() {
                    vec![start.month]
                } else {
                    rule.months.clone()
                };
                if months.len() != 1 {
                    return Err(unsupported());
                }
                pattern.insert("month".to_string(), json!(months[0]));
                if rule.weekdays.is_empty() {
                    let days = if rule.month_days.is_empty() {
                        vec![start.day]
                    } else {
                        rule.month_days.clone()
                    };
                    if days.len() != 1 || days[0] < 1 {
                        return Err(unsupported());
                    }
                    pattern.insert("type".to_string(), json!("absoluteYearly"));
                    pattern.insert("dayOfMonth".to_string(), json!(days[0]));
                } else {
                    if !rule.month_days.is_empty() {
                        return Err(unsupported());
                    }
                    pattern.insert("type".to_string(), json!("relativeYearly"));
                    Self::add_relative(&rule.weekdays, &mut pattern)?;
                }
            }
            Frequency::Secondly | Frequency::Minutely | Frequency::Hourly => {
                return Err(unsupported());
            }
        }
        let mut range = Json::new();
        range.insert("startDate".to_string(), json!(GraphTime::date_text(start)));
        range.insert(
            "recurrenceTimeZone".to_string(),
            json!(WindowsTimeZones::windows_name_for(zone).unwrap_or_else(|| "UTC".to_string())),
        );
        match rule.end {
            RuleEnd::Never => {
                range.insert("type".to_string(), json!("noEnd"));
            }
            RuleEnd::Count(n) => {
                range.insert("type".to_string(), json!("numbered"));
                range.insert("numberOfOccurrences".to_string(), json!(n));
            }
            RuleEnd::Until(date) => {
                range.insert("type".to_string(), json!("endDate"));
                range.insert(
                    "endDate".to_string(),
                    json!(GraphTime::date_text(&AllDay::date_of(date, zone))),
                );
            }
        }
        let mut result = Json::new();
        result.insert("pattern".to_string(), Value::Object(pattern));
        result.insert("range".to_string(), Value::Object(range));
        Ok(result)
    }

    /// Graph has one `index` for the whole pattern, so every weekday must carry the same ordinal, and it must be one
    /// Graph has (first to fourth, or last).
    fn add_relative(days: &[WeekdayOccurrence], pattern: &mut Json) -> Result<(), WriteError> {
        let Some(ordinal) = days.first().and_then(|day| day.ordinal) else {
            return Err(unsupported());
        };
        if !days.iter().all(|day| day.ordinal == Some(ordinal)) {
            return Err(unsupported());
        }
        let Some((name, _)) = INDEXES.iter().find(|(_, candidate)| *candidate == ordinal) else {
            return Err(unsupported());
        };
        pattern.insert("index".to_string(), json!(name));
        pattern.insert(
            "daysOfWeek".to_string(),
            json!(
                days.iter()
                    .map(|day| name_of(day.weekday))
                    .collect::<Vec<_>>()
            ),
        );
        Ok(())
    }

    fn weekday_name_of(date: &CalendarDate) -> Result<&'static str, WriteError> {
        let utc = Zone::utc();
        // Swift force-unwraps this date; a date outside the range of `Timestamp` is a rule Graph cannot take.
        let noon = civil::date_from(date.year, date.month, date.day, 12, 0, 0, &utc)
            .ok_or_else(unsupported)?;
        let index = civil::components(noon, &utc).weekday;
        Ok(NAMES[usize::try_from(index - 1).unwrap_or(0).min(6)])
    }

    // MARK: Splitting a series

    /// The number of occurrences of a `numbered` range; `None` for any other range.
    pub(crate) fn occurrence_count(recurrence: &Json) -> Option<i64> {
        let range = object_of(recurrence, "range");
        if range.and_then(|range| str_of(range, "type")) != Some("numbered") {
            return None;
        }
        int_of(range.and_then(|range| range.get("numberOfOccurrences")))
    }

    /// Counts the master's scheduled slots, including deleted and moved occurrences. Graph's instances endpoint
    /// reports current instances, so it cannot establish a numbered range's position after exceptions.
    pub(crate) fn remaining_count(
        recurrence: &Json,
        split: &CalendarDate,
    ) -> Result<i64, WriteError> {
        let Some(total) = Self::occurrence_count(recurrence).filter(|t| *t > 0 && *t <= 10_000)
        else {
            return Err(unsupported());
        };
        let range = object_of(recurrence, "range");
        let Some(start) = range
            .and_then(|range| str_of(range, "startDate"))
            .and_then(GraphTime::date)
        else {
            return Err(unsupported());
        };
        let Some(pattern) = object_of(recurrence, "pattern") else {
            return Err(unsupported());
        };
        let Some(kind) = str_of(pattern, "type") else {
            return Err(unsupported());
        };
        let Some(interval) = int_of(pattern.get("interval")).filter(|interval| *interval > 0)
        else {
            return Err(unsupported());
        };
        if start > *split {
            return Err(WriteError::Invalid(
                "the split precedes the series start".to_string(),
            ));
        }

        let utc = Zone::utc();
        let noon =
            |day: &CalendarDate| civil::date_from(day.year, day.month, day.day, 12, 0, 0, &utc);
        let (Some(origin), Some(target)) = (noon(&start), noon(split)) else {
            return Err(unsupported());
        };
        if AllDay::date_of(origin, &utc) != start || AllDay::date_of(target, &utc) != *split {
            return Err(unsupported());
        }
        let distance = (target.as_second() - origin.as_second()).div_euclid(86_400);
        // A finite scan avoids unbounded work on malformed Graph data. Older sparse series beyond this horizon are
        // rejected before the master is changed, rather than assigned an unsafe continuation count.
        if !(0..=36_600).contains(&distance) {
            return Err(unsupported());
        }
        // `daysOfWeek` is read with `as? [String]`: one element that is not a string makes the whole list absent.
        let days: Vec<String> = match pattern.get("daysOfWeek") {
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| item.as_str().map(str::to_string))
                .collect::<Option<Vec<String>>>()
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        if !days
            .iter()
            .all(|day| NAMES.contains(&day.to_lowercase().as_str()))
        {
            return Err(unsupported());
        }
        let origin_days = origin.as_second().div_euclid(86_400);
        let start_weekday = weekday_number(origin_days);
        let selected: Vec<String> = if kind == "weekly" && days.is_empty() {
            vec![NAMES[usize::try_from(start_weekday - 1).unwrap_or(0)].to_string()]
        } else {
            days.iter().map(|day| day.to_lowercase()).collect()
        };
        let first_day_name = str_of(pattern, "firstDayOfWeek")
            .unwrap_or("monday")
            .to_lowercase();
        let Some(first_day_index) = NAMES.iter().position(|name| *name == first_day_name) else {
            return Err(unsupported());
        };
        let week_offset = (start_weekday - 1 - first_day_index as i64 + 7) % 7;
        let index_name = str_of(pattern, "index").map(str::to_lowercase);
        let ordinal = INDEXES
            .iter()
            .find(|(name, _)| Some(*name) == index_name.as_deref())
            .map(|(_, ordinal)| *ordinal);
        let day_of_month = int_of(pattern.get("dayOfMonth"));
        let month_of = int_of(pattern.get("month"));
        if (kind == "relativeMonthly" || kind == "relativeYearly")
            && (ordinal.is_none() || selected.is_empty())
        {
            return Err(unsupported());
        }
        if (kind == "absoluteMonthly" || kind == "absoluteYearly")
            && day_of_month.is_none_or(|day| !(1..=31).contains(&day))
        {
            return Err(unsupported());
        }
        if (kind == "absoluteYearly" || kind == "relativeYearly")
            && month_of.is_none_or(|month| !(1..=12).contains(&month))
        {
            return Err(unsupported());
        }
        if ![
            "daily",
            "weekly",
            "absoluteMonthly",
            "relativeMonthly",
            "absoluteYearly",
            "relativeYearly",
        ]
        .contains(&kind)
        {
            return Err(unsupported());
        }

        let relative_match = |year: i64, month: i64, day: i64| -> bool {
            let Some(ordinal) = ordinal else {
                return false;
            };
            let first_of_month = days_from_civil(year, month, 1);
            let matches: Vec<i64> = (1..=days_in_month(year, month))
                .filter(|candidate| {
                    let name =
                        NAMES[usize::try_from(weekday_number(first_of_month + candidate - 1) - 1)
                            .unwrap_or(0)];
                    selected.iter().any(|chosen| chosen == name)
                })
                .collect();
            if matches.is_empty() {
                return false;
            }
            let selected_day = if ordinal == -1 {
                matches.last().copied()
            } else if ordinal > 0 && ordinal <= matches.len() as i64 {
                Some(matches[(ordinal - 1) as usize])
            } else {
                None
            };
            Some(day) == selected_day
        };
        let mut counted: i64 = 0;
        for offset in 0..=distance {
            let day_number = origin_days + offset;
            let (year, month, day) = civil_from_days(day_number);
            let weekday = weekday_number(day_number);
            let month_distance = (year - start.year) * 12 + month - start.month;
            let year_distance = year - start.year;
            let occurs = match kind {
                "daily" => offset % interval == 0,
                "weekly" => {
                    let week = (offset + week_offset) / 7;
                    week % interval == 0
                        && selected.iter().any(|chosen| {
                            chosen == NAMES[usize::try_from(weekday - 1).unwrap_or(0)]
                        })
                }
                "absoluteMonthly" => month_distance % interval == 0 && Some(day) == day_of_month,
                "relativeMonthly" => {
                    month_distance % interval == 0 && relative_match(year, month, day)
                }
                "absoluteYearly" => {
                    year_distance % interval == 0
                        && Some(month) == month_of
                        && Some(day) == day_of_month
                }
                _ => {
                    year_distance % interval == 0
                        && Some(month) == month_of
                        && relative_match(year, month, day)
                }
            };
            if occurs {
                counted += 1;
            }
            if offset == distance {
                if !occurs || counted > total {
                    return Err(WriteError::Invalid(
                        "the split is not a remaining recurrence slot".to_string(),
                    ));
                }
                return Ok(total - counted + 1);
            }
            if counted >= total {
                return Err(WriteError::Invalid(
                    "the split is beyond the numbered series".to_string(),
                ));
            }
        }
        Err(unsupported())
    }

    /// The same recurrence cut so its last occurrence is before `split`: the range becomes an `endDate` of the day
    /// before the split. Graph counts a series in days at the finest, so one occurrence a day is the most there is.
    pub(crate) fn truncated(recurrence: &Json, split: &CalendarDate) -> Json {
        let mut range = object_of(recurrence, "range").cloned().unwrap_or_default();
        range.insert("type".to_string(), json!("endDate"));
        range.insert(
            "endDate".to_string(),
            json!(GraphTime::date_text(&split.adding_days(-1))),
        );
        range.remove("numberOfOccurrences");
        let mut copy = recurrence.clone();
        copy.insert("range".to_string(), Value::Object(range));
        copy
    }

    /// The recurrence for the series that continues from `start`: the same pattern, starting there, with `remaining`
    /// occurrences when the original was counted.
    pub(crate) fn restarted(
        recurrence: &Json,
        start: &CalendarDate,
        remaining: Option<i64>,
    ) -> Json {
        let mut range = object_of(recurrence, "range").cloned().unwrap_or_default();
        range.insert("startDate".to_string(), json!(GraphTime::date_text(start)));
        if let Some(remaining) = remaining {
            range.insert("numberOfOccurrences".to_string(), json!(remaining));
        }
        let mut copy = recurrence.clone();
        copy.insert("range".to_string(), Value::Object(range));
        copy
    }
}
