//! Port of `VTimeZoneWriter.swift`.

use calendar_core::civil;
use calendar_core::time::{Zone, add_seconds};
use jiff::Timestamp;

use crate::component::{ICalComponent, ICalProperty};
use crate::ical_values::ICalValues;
use crate::zone_support::{abbreviation, is_daylight_saving, next_transition};

/// Builds the `VTIMEZONE` a written event needs for its `TZID` (RFC 4791 requires one for each TZID used).
pub enum VTimeZoneWriter {}

impl VTimeZoneWriter {
    /// One observance per transition from a year before `start` through `end` (at most 400), or a single `STANDARD`
    /// observance for a zone without transitions. `None` for UTC, which is written with `Z` instead.
    pub fn component(zone: &Zone, start: Timestamp, end: Timestamp) -> Option<ICalComponent> {
        if zone.identifier() == "UTC" || zone.identifier() == "GMT" {
            return None;
        }
        let utc = Zone::utc();
        let mut observances: Vec<ICalComponent> = Vec::new();
        // The start of the year before `start`'s year, so a full prior year's transitions (including one that
        // falls before `start`'s own month, e.g. a March DST start when `start` is in June) are always included.
        let start_year = civil::components(start, &utc).year;
        let mut cursor = civil::date_from(start_year - 1, 1, 1, 0, 0, 0, &utc)
            .unwrap_or_else(|| add_seconds(start, -366.0 * 86_400.0));
        while observances.len() < 400 {
            let Some(transition) = next_transition(zone, cursor) else {
                break;
            };
            if transition > end {
                break;
            }
            let from = zone.offset_seconds(add_seconds(transition, -1.0));
            let to = zone.offset_seconds(transition);
            let mut properties = vec![
                // DTSTART is the transition's local time read with the offset before it.
                ICalProperty::with_value(
                    "DTSTART",
                    ICalValues::local_text(add_seconds(transition, f64::from(from)), &utc),
                ),
                ICalProperty::with_value(
                    "TZOFFSETFROM",
                    VTimeZoneWriter::offset_text(i64::from(from)),
                ),
                ICalProperty::with_value("TZOFFSETTO", VTimeZoneWriter::offset_text(i64::from(to))),
            ];
            properties.push(ICalProperty::from_text(
                "TZNAME",
                &abbreviation(zone, transition),
                Vec::new(),
            ));
            observances.push(ICalComponent::new(
                if is_daylight_saving(zone, transition) {
                    "DAYLIGHT"
                } else {
                    "STANDARD"
                },
                properties,
                Vec::new(),
            ));
            cursor = transition;
        }
        if observances.is_empty() {
            let offset = i64::from(zone.offset_seconds(start));
            observances = vec![ICalComponent::new(
                "STANDARD",
                vec![
                    ICalProperty::with_value("DTSTART", "19700101T000000"),
                    ICalProperty::with_value("TZOFFSETFROM", VTimeZoneWriter::offset_text(offset)),
                    ICalProperty::with_value("TZOFFSETTO", VTimeZoneWriter::offset_text(offset)),
                ],
                Vec::new(),
            )];
        }
        Some(ICalComponent::new(
            "VTIMEZONE",
            vec![ICalProperty::with_value("TZID", zone.identifier())],
            observances,
        ))
    }

    pub(crate) fn offset_text(seconds: i64) -> String {
        let sign = if seconds < 0 { "-" } else { "+" };
        let value = seconds.abs();
        let text = format!("{sign}{:02}{:02}", value / 3600, (value % 3600) / 60);
        if value % 60 == 0 {
            text
        } else {
            format!("{text}{:02}", value % 60)
        }
    }
}
