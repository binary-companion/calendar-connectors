//! Port of `TimeZoneResolver.swift`.

use std::collections::{BTreeMap, BTreeSet};

use calendar_core::all_day::AllDay;
use calendar_core::civil;
use calendar_core::time::Zone;
use calendar_core::windows_time_zones::WindowsTimeZones;
use jiff::Timestamp;

use crate::component::ICalComponent;
use crate::content_line::is_whitespace;
use crate::grapheme::{characters, drop_first, is, prefix, split};
use crate::ical_values::ICalDateValue;
use crate::zone_support::{is_daylight_saving, known_zones, next_transition};

/// Turns a `TZID` into a `Zone`: an IANA id; the IANA id inside a path-style id (`/mozilla.org/.../America/New_York`);
/// a Windows name; the file's own `VTIMEZONE` (its `X-LIC-LOCATION`, else the zone Foundation knows with the same
/// offsets and transition months, else a fixed offset). `None` when nothing fits.
#[derive(Clone, Debug, Default)]
pub struct TimeZoneResolver {
    definitions: BTreeMap<String, ICalComponent>,
}

impl TimeZoneResolver {
    /// Swift `init()`.
    pub fn new() -> TimeZoneResolver {
        TimeZoneResolver::default()
    }

    /// Swift `init(calendar:)`.
    pub fn from_calendar(calendar: &ICalComponent) -> TimeZoneResolver {
        let mut found = BTreeMap::new();
        for definition in calendar.components_named("VTIMEZONE") {
            if let Some(id) = definition.property("TZID") {
                found.insert(id.value.clone(), definition.clone());
            }
        }
        TimeZoneResolver { definitions: found }
    }

    /// Swift `zone(for:)`.
    pub fn zone_for(&self, tzid: &str) -> Option<Zone> {
        let id = tzid.trim_matches(is_whitespace);
        if let Some(zone) = Zone::named(id) {
            return Some(zone);
        }
        let parts = split(id, '/');
        if parts.len() > 1 {
            for start in 1..parts.len() {
                if let Some(zone) = Zone::named(&parts[start..].join("/")) {
                    return Some(zone);
                }
            }
        }
        if let Some(zone) = WindowsTimeZones::time_zone_for(id) {
            return Some(zone);
        }
        if let Some(definition) = self.definitions.get(id) {
            return TimeZoneResolver::zone_matching(definition);
        }
        None
    }

    /// The instant a value names. Floating times and dates are read in `zone`; an unknown TZID too.
    pub fn date(&self, value: &ICalDateValue, floating: &Zone) -> Option<Timestamp> {
        match value {
            ICalDateValue::Date(day) => AllDay::start_of_day(day, floating),
            ICalDateValue::Utc(instant) => Some(*instant),
            ICalDateValue::Local { local, tzid } => {
                let resolved = tzid
                    .as_deref()
                    .and_then(|tzid| self.zone_for(tzid))
                    .unwrap_or_else(|| floating.clone());
                civil::date_from(
                    local.date.year,
                    local.date.month,
                    local.date.day,
                    local.hour,
                    local.minute,
                    local.second,
                    &resolved,
                )
            }
        }
    }

    /// The zone to show a value in: its TZID's zone, UTC for a UTC value, `fallback` for floating times and dates.
    pub fn display_zone(&self, value: &ICalDateValue, fallback: &Zone) -> Zone {
        match value {
            ICalDateValue::Utc(_) => Zone::utc(),
            ICalDateValue::Local {
                tzid: Some(tzid), ..
            } => self.zone_for(tzid).unwrap_or_else(|| fallback.clone()),
            _ => fallback.clone(),
        }
    }

    // MARK: VTIMEZONE matching

    fn zone_matching(definition: &ICalComponent) -> Option<Zone> {
        if let Some(location) = definition.property("X-LIC-LOCATION")
            && let Some(zone) = Zone::named(&location.value)
        {
            return Some(zone);
        }
        let observances: Vec<&ICalComponent> = definition
            .components
            .iter()
            .filter(|c| c.name == "STANDARD" || c.name == "DAYLIGHT")
            .collect();
        // `max(by:)` keeps the first of equal elements.
        let latest = |kind: &str| -> Option<&ICalComponent> {
            fn dtstart(c: &ICalComponent) -> &str {
                c.property("DTSTART")
                    .map(|p| p.value.as_str())
                    .unwrap_or("")
            }
            let mut best: Option<&ICalComponent> = None;
            for observance in observances.iter().copied().filter(|c| c.name == kind) {
                best = match best {
                    Some(current) if dtstart(current) >= dtstart(observance) => Some(current),
                    _ => Some(observance),
                };
            }
            best
        };
        let standard = latest("STANDARD")?;
        let standard_offset =
            TimeZoneResolver::offset(standard.property("TZOFFSETTO").map(|p| p.value.as_str()))?;
        let daylight = latest("DAYLIGHT");
        let daylight_offset = daylight.and_then(|d| {
            TimeZoneResolver::offset(d.property("TZOFFSETTO").map(|p| p.value.as_str()))
        });
        let year = std::cmp::max(
            2000,
            prefix(
                standard
                    .property("DTSTART")
                    .map(|p| p.value.as_str())
                    .unwrap_or("2000"),
                4,
            )
            .parse::<i64>()
            .unwrap_or(2000),
        );
        let utc = Zone::utc();
        let january = civil::date_from(year, 1, 15, 12, 0, 0, &utc)?;
        let july = civil::date_from(year, 7, 15, 12, 0, 0, &utc)?;
        let mut wanted: BTreeSet<i64> = BTreeSet::new();
        wanted.insert(standard_offset);
        if let Some(offset) = daylight_offset {
            wanted.insert(offset);
        }
        let candidates: Vec<Zone> = known_zones()
            .filter(|zone| {
                let seen: BTreeSet<i64> = [
                    i64::from(zone.offset_seconds(january)),
                    i64::from(zone.offset_seconds(july)),
                ]
                .into_iter()
                .collect();
                seen == wanted
            })
            .collect();
        if let Some(daylight) = daylight
            && let Some(months) = TimeZoneResolver::transition_months_of_rules(daylight, standard)
        {
            let start = civil::date_from(year, 1, 1, 0, 0, 0, &utc)?;
            if let Some(found) = candidates.iter().find(|zone| {
                TimeZoneResolver::transition_months_of_zone(zone, start) == Some(months)
            }) {
                return Some(found.clone());
            }
        }
        candidates
            .into_iter()
            .next()
            .or_else(|| i32::try_from(standard_offset).ok().and_then(Zone::fixed))
    }

    /// `+HHMM`, `-HHMM` or `+HHMMSS` in seconds.
    pub(crate) fn offset(text: Option<&str>) -> Option<i64> {
        let text = text?;
        let chars: Vec<&str> = characters(text).collect();
        if !(chars.len() == 5 || chars.len() == 7) {
            return None;
        }
        let sign = chars[0];
        if !(is(sign, '+') || is(sign, '-')) {
            return None;
        }
        let digits = &chars[1..];
        if !digits
            .iter()
            .all(|c| c.len() == 1 && c.as_bytes()[0].is_ascii_digit())
        {
            return None;
        }
        let number = |from: usize, to: usize| digits[from..to].concat().parse::<i64>().ok();
        let hours = number(0, 2)?;
        let minutes = number(2, 4)?;
        let seconds = if digits.len() == 6 { number(4, 6)? } else { 0 };
        Some((if is(sign, '-') { -1 } else { 1 }) * (hours * 3600 + minutes * 60 + seconds))
    }

    /// The months daylight time starts and ends in, from the observances' `RRULE:...;BYMONTH=` (or their `DTSTART`).
    fn transition_months_of_rules(
        daylight: &ICalComponent,
        standard: &ICalComponent,
    ) -> Option<[i64; 2]> {
        fn month(observance: &ICalComponent) -> Option<i64> {
            if let Some(rule) = observance.property("RRULE")
                && let Some(part) = split(&rule.value, ';')
                    .into_iter()
                    .find(|part| part.to_uppercase().starts_with("BYMONTH="))
            {
                return drop_first(part, 8).parse().ok();
            }
            let dtstart = observance.property("DTSTART")?;
            prefix(drop_first(&dtstart.value, 4), 2).parse().ok()
        }
        Some([month(daylight)?, month(standard)?])
    }

    fn transition_months_of_zone(zone: &Zone, start: Timestamp) -> Option<[i64; 2]> {
        let first = next_transition(zone, start)?;
        let second = next_transition(zone, first)?;
        let (a, b) = if is_daylight_saving(zone, first) {
            (first, second)
        } else {
            (second, first)
        };
        let utc = Zone::utc();
        Some([
            civil::components(a, &utc).month,
            civil::components(b, &utc).month,
        ])
    }
}
