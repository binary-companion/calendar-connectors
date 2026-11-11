//! The parts of Foundation's `TimeZone` that `ICalendar` uses and `calendar_core::time::Zone` does not offer:
//! `knownTimeZoneIdentifiers`, `nextDaylightSavingTimeTransition(after:)`, `isDaylightSavingTime(for:)` and
//! `abbreviation(for:)`.

use calendar_core::civil;
use calendar_core::time::{Zone, add_seconds};
use jiff::Timestamp;

use crate::zone_data::{ABBREVIATIONS, KNOWN_IDENTIFIERS};

/// `TimeZone.knownTimeZoneIdentifiers.sorted().compactMap(TimeZone.init(identifier:))`. The list is the one Foundation
/// reports (`zone_data.rs`), not the zone database's own, so the same zone is picked on every platform.
pub(crate) fn known_zones() -> impl Iterator<Item = Zone> {
    KNOWN_IDENTIFIERS
        .iter()
        .filter_map(|identifier| Zone::named(identifier))
}

/// The most transitions looked at in one search; a zone database with only abbreviation changes ends long before.
const SEARCH_LIMIT: usize = 10_000;

/// `nextDaylightSavingTimeTransition(after:)`. Foundation reports every change of the total offset (a zone that
/// moved its standard time, such as Europe/Moscow in 2011, counts as well); a change of the abbreviation or the
/// daylight flag alone is not one.
pub(crate) fn next_transition(zone: &Zone, after: Timestamp) -> Option<Timestamp> {
    for transition in zone.as_jiff().following(after).take(SEARCH_LIMIT) {
        let at = transition.timestamp();
        if transition.offset().seconds() != zone.offset_seconds(add_seconds(at, -1.0)) {
            return Some(at);
        }
    }
    None
}

/// The zones whose current rules the main tzdata format writes with "negative" daylight time (standard time in summer,
/// daylight time in winter). Foundation's data is the rearguard form, where the larger offset is the daylight one,
/// and the zone database on Linux may be either; for these zones daylight time is read from the offset, so both agree.
/// Their standard offset is UTC.
const NEGATIVE_DST_ZONES: &[&str] = &[
    "Europe/Dublin",
    "Eire",
    "Africa/Casablanca",
    "Africa/El_Aaiun",
];

/// `isDaylightSavingTime(for:)`.
pub(crate) fn is_daylight_saving(zone: &Zone, at: Timestamp) -> bool {
    if NEGATIVE_DST_ZONES.contains(&zone.identifier()) {
        return zone.offset_seconds(at) > 0;
    }
    zone.as_jiff().to_offset_info(at).dst().is_dst()
}

/// `abbreviation(for:)` as Foundation answers it for the `en_US` locale: the common North American names where
/// Foundation has them, else `GMT+h` or `GMT+h:mm` (no abbreviation of the zone database is used). The names depend on
/// the locale in Swift; the table is what a Mac set to `en_US` reports. A zone that no longer observes daylight time
/// (Moscow, Sao Paulo, Tehran) is named for its present offset in every period, as Foundation does; "present" is
/// read from 2030, since this crate reads no clock.
pub(crate) fn abbreviation(zone: &Zone, at: Timestamp) -> String {
    let identifier = zone.identifier();
    if let Ok(index) = ABBREVIATIONS.binary_search_by(|(id, _, _)| (*id).cmp(identifier)) {
        let (_, standard, daylight) = ABBREVIATIONS[index];
        let name = if is_daylight_saving(zone, at) {
            daylight
        } else {
            standard
        };
        if !name.is_empty() {
            return name.to_string();
        }
    }
    // Foundation models Morocco's Ramadan clock change as daylight time; the zone database's forecast for 2030 does not.
    if identifier == "Africa/Casablanca" || identifier == "Africa/El_Aaiun" {
        return generic_abbreviation(zone.offset_seconds(at));
    }
    let winter = civil::date_from(2030, 1, 15, 12, 0, 0, &Zone::utc());
    let summer = civil::date_from(2030, 7, 15, 12, 0, 0, &Zone::utc());
    if let (Some(winter), Some(summer)) = (winter, summer) {
        let (current, other) = (zone.offset_seconds(winter), zone.offset_seconds(summer));
        if current == other {
            return generic_abbreviation(current);
        }
    }
    generic_abbreviation(zone.offset_seconds(at))
}

fn generic_abbreviation(seconds: i32) -> String {
    if seconds == 0 {
        return "GMT+0".to_string();
    }
    let sign = if seconds > 0 { '+' } else { '-' };
    let total = seconds.unsigned_abs();
    let (hours, minutes) = (total / 3600, (total % 3600) / 60);
    if minutes == 0 {
        format!("GMT{sign}{hours}")
    } else {
        format!("GMT{sign}{hours}:{minutes:02}")
    }
}
