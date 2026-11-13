//! Port of `CalendarTestSupport/AllDayConformance.swift`.

use calendar_core::all_day::AllDay;
use calendar_core::model::CalendarEvent;
use jiff::Timestamp;

/// Every connector's mapper tests run their all-day fixtures through this so no connector can emit a
/// provider-native form. An empty result means the event is canonical.
pub enum AllDayConformance {}

impl AllDayConformance {
    /// Returns the ways `event` breaks the canonical all-day form: start and end must each be the start of a day in the
    /// event's time zone, and end must be after start. Empty for a timed event or a conforming all-day event.
    pub fn violations(event: &CalendarEvent) -> Vec<String> {
        if !event.is_all_day {
            return Vec::new();
        }
        let zone = &event.time_zone;
        let is_midnight = |instant: Timestamp| {
            AllDay::start_of_day(&AllDay::date_of(instant, zone), zone) == Some(instant)
        };
        let mut found = Vec::new();
        if !is_midnight(event.start) {
            found.push(format!(
                "start is not the start of a day in {}",
                zone.identifier()
            ));
        }
        if !is_midnight(event.end) {
            found.push(format!(
                "end is not the start of a day in {}",
                zone.identifier()
            ));
        }
        if event.end <= event.start {
            found.push("end is not after start".to_string());
        }
        found
    }
}
