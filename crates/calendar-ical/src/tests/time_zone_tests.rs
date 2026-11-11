//! Port of `TimeZoneTests.swift`.

use calendar_core::all_day::{AllDay, CalendarDate};
use calendar_core::civil;
use calendar_core::time::Zone;
use jiff::Timestamp;

use super::fixtures::at;
use crate::component::{ICalComponent, ICalProperty};
use crate::ical_values::{ICalDateValue, LocalDateTime};
use crate::time_zone_resolver::TimeZoneResolver;
use crate::vtimezone_writer::VTimeZoneWriter;

fn instant(y: i64, m: i64, d: i64) -> Timestamp {
    civil::date_from(y, m, d, 12, 0, 0, &Zone::named("UTC").unwrap()).unwrap()
}

fn same_offsets(a: Option<&Zone>, b: &Zone, year: i64) -> bool {
    let Some(a) = a else {
        return false;
    };
    [
        instant(year, 1, 15),
        instant(year, 3, 20),
        instant(year, 7, 15),
        instant(year, 11, 15),
    ]
    .iter()
    .all(|t| a.offset_seconds(*t) == b.offset_seconds(*t))
}

fn zone(identifier: &str) -> Zone {
    Zone::named(identifier).unwrap()
}

fn calendar_of(definition: ICalComponent) -> ICalComponent {
    ICalComponent::new("VCALENDAR", Vec::new(), vec![definition])
}

#[test]
fn resolves_windows_mozilla_and_custom_zones() {
    let resolver = TimeZoneResolver::new();
    let new_york = zone("America/New_York");
    assert!(same_offsets(
        resolver.zone_for("America/New_York").as_ref(),
        &new_york,
        2026
    ));
    assert!(same_offsets(
        resolver.zone_for("Eastern Standard Time").as_ref(),
        &new_york,
        2026
    ));
    assert!(same_offsets(
        resolver
            .zone_for("/mozilla.org/20050126_1/America/New_York")
            .as_ref(),
        &new_york,
        2026
    ));
    assert!(resolver.zone_for("Nowhere/Special").is_none());

    // A custom TZID defined only by its VTIMEZONE (generated here from Berlin's rules, then renamed).
    let berlin = zone("Europe/Berlin");
    let mut definition =
        VTimeZoneWriter::component(&berlin, instant(2026, 1, 1), instant(2027, 12, 31)).unwrap();
    definition.set(ICalProperty::with_value("TZID", "My Office Zone"));
    let calendar = calendar_of(definition);
    assert!(same_offsets(
        TimeZoneResolver::from_calendar(&calendar)
            .zone_for("My Office Zone")
            .as_ref(),
        &berlin,
        2026
    ));
}

#[test]
fn fixed_offset_fallback_for_a_zone_without_daylight_time() {
    let definition = ICalComponent::new(
        "VTIMEZONE",
        vec![ICalProperty::with_value("TZID", "Custom +0530")],
        vec![ICalComponent::new(
            "STANDARD",
            vec![
                ICalProperty::with_value("DTSTART", "19700101T000000"),
                ICalProperty::with_value("TZOFFSETFROM", "+0530"),
                ICalProperty::with_value("TZOFFSETTO", "+0530"),
            ],
            Vec::new(),
        )],
    );
    let found = TimeZoneResolver::from_calendar(&calendar_of(definition)).zone_for("Custom +0530");
    assert_eq!(
        found.map(|z| z.offset_seconds(instant(2026, 7, 1))),
        Some(19_800)
    );
}

#[test]
fn x_lic_location_wins() {
    let definition = ICalComponent::new(
        "VTIMEZONE",
        vec![
            ICalProperty::with_value("TZID", "Weird"),
            ICalProperty::with_value("X-LIC-LOCATION", "Asia/Tokyo"),
        ],
        Vec::new(),
    );
    assert_eq!(
        TimeZoneResolver::from_calendar(&calendar_of(definition))
            .zone_for("Weird")
            .map(|z| z.identifier().to_string())
            .as_deref(),
        Some("Asia/Tokyo")
    );
}

#[test]
fn resolves_values_with_the_right_zone() {
    let resolver = TimeZoneResolver::new();
    let floating = zone("America/Los_Angeles");
    let local = LocalDateTime::new(CalendarDate::new(2026, 9, 27), 9, 0, 0);
    let berlin = ICalDateValue::Local {
        local,
        tzid: Some("Europe/Berlin".to_string()),
    };
    let unzoned = ICalDateValue::Local { local, tzid: None };
    assert_eq!(resolver.date(&berlin, &floating), Some(at(1_790_492_400)));
    assert_eq!(resolver.date(&unzoned, &floating), Some(at(1_790_524_800)));
    let day = CalendarDate::new(2026, 9, 27);
    assert_eq!(
        resolver.date(&ICalDateValue::Date(day), &floating),
        AllDay::start_of_day(&day, &floating)
    );
    assert_eq!(
        resolver.display_zone(&berlin, &floating).identifier(),
        "Europe/Berlin"
    );
    let utc_identifier = resolver
        .display_zone(&ICalDateValue::Utc(at(0)), &floating)
        .identifier()
        .to_string();
    assert!(utc_identifier == "UTC" || utc_identifier == "GMT");
    assert_eq!(resolver.display_zone(&unzoned, &floating), floating);
}

#[test]
fn vtimezone_covers_every_transition_in_the_range() {
    let new_york = zone("America/New_York");
    let component =
        VTimeZoneWriter::component(&new_york, instant(2026, 6, 1), instant(2046, 6, 1)).unwrap();
    assert_eq!(
        component.property("TZID").map(|p| p.value.as_str()),
        Some("America/New_York")
    );
    let daylight = component.components_named("DAYLIGHT");
    let standard = component.components_named("STANDARD");
    // One DST start and one end per year from a year before the start through the end: 2025 ... 2046.
    assert!(daylight.len() >= 21 && standard.len() >= 21);
    assert!(daylight.iter().all(|d| {
        d.property("TZOFFSETTO").map(|p| p.value.as_str()) == Some("-0400")
            && d.property("TZOFFSETFROM").map(|p| p.value.as_str()) == Some("-0500")
    }));
    assert_eq!(
        daylight[0].property("DTSTART").map(|p| p.value.as_str()),
        Some("20250309T020000")
    );
    assert!(
        VTimeZoneWriter::component(&zone("UTC"), instant(2026, 1, 1), instant(2027, 1, 1))
            .is_none()
    );
}

#[test]
fn zone_without_transitions_gets_one_standard_observance() {
    let tokyo = zone("Asia/Tokyo");
    let component =
        VTimeZoneWriter::component(&tokyo, instant(2026, 1, 1), instant(2027, 1, 1)).unwrap();
    let standard = component.components_named("STANDARD");
    assert_eq!(standard.len(), 1);
    assert_eq!(
        standard[0].property("TZOFFSETTO").map(|p| p.value.as_str()),
        Some("+0900")
    );
    assert!(component.components_named("DAYLIGHT").is_empty());
}
