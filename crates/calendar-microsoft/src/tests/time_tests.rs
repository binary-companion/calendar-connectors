//! Port of `TimeTests.swift`: the Microsoft service constant, the Windows time-zone table (as the connector uses it)
//! and `GraphTime`.

use std::collections::BTreeSet;

use calendar_core::all_day::CalendarDate;
use calendar_core::model::CalendarService;
use calendar_core::time::Zone;
use calendar_core::windows_time_zones::WindowsTimeZones;
use jiff::Timestamp;

use crate::graph_time::GraphTime;

fn la() -> Zone {
    Zone::named("America/Los_Angeles").unwrap()
}

fn at(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).unwrap()
}

#[test]
fn microsoft_is_a_service() {
    assert_eq!(CalendarService::microsoft().raw_value(), "microsoft");
}

#[test]
fn windows_names_resolve_to_iana_zones() {
    assert_eq!(
        WindowsTimeZones::time_zone_for("Pacific Standard Time")
            .unwrap()
            .identifier(),
        "America/Los_Angeles"
    );
    assert_eq!(
        WindowsTimeZones::time_zone_for("UTC")
            .unwrap()
            .offset_seconds(at(0)),
        0
    );
    assert_eq!(
        WindowsTimeZones::time_zone_for("W. Europe Standard Time")
            .unwrap()
            .identifier(),
        "Europe/Berlin"
    );
}

#[test]
fn iana_names_pass_through_and_unknown_names_are_nil() {
    assert_eq!(
        WindowsTimeZones::time_zone_for("Europe/Paris")
            .unwrap()
            .identifier(),
        "Europe/Paris"
    );
    assert!(WindowsTimeZones::time_zone_for("Customized Time Zone").is_none());
    assert!(WindowsTimeZones::time_zone_for("").is_none());
}

#[test]
fn every_table_entry_resolves_on_this_platform() {
    // A failure names the entry; fix it by using a spelling this platform knows (see the aliases list).
    let mut missing: Vec<&str> = WindowsTimeZones::table_entries()
        .iter()
        .filter(|(_, iana)| Zone::named(iana).is_none())
        .map(|(name, _)| *name)
        .collect();
    missing.sort_unstable();
    assert!(missing.is_empty(), "unresolvable: {missing:?}");
}

#[test]
fn windows_name_for_a_zone() {
    assert_eq!(
        WindowsTimeZones::windows_name_for(&Zone::named("America/New_York").unwrap()).as_deref(),
        Some("Eastern Standard Time")
    );
    assert_eq!(
        WindowsTimeZones::windows_name_for(&Zone::named("UTC").unwrap()).as_deref(),
        Some("UTC")
    );
    assert_eq!(
        WindowsTimeZones::windows_name_for(&Zone::named("Asia/Kolkata").unwrap()).as_deref(),
        Some("India Standard Time")
    );
    assert_eq!(
        WindowsTimeZones::windows_name_for(&Zone::named("Pacific/Fakaofo").unwrap()),
        None
    );
}

#[test]
fn windows_names_are_one_to_one_with_iana_ids() {
    let entries = WindowsTimeZones::table_entries();
    let distinct: BTreeSet<&str> = entries.iter().map(|(_, iana)| *iana).collect();
    assert_eq!(distinct.len(), entries.len());
}

#[test]
fn parses_graph_local_date_times_in_a_zone() {
    let la = la();
    let date = GraphTime::parse("2026-09-25T10:00:00.0000000", &la);
    assert_eq!(date, Some(at(1_790_355_600))); // 17:00 UTC
    assert_eq!(GraphTime::parse("2026-09-25T10:00:00", &la), date);
    assert_eq!(GraphTime::parse("2026-13-25T10:00:00", &la), None);
    assert_eq!(GraphTime::parse("2026-02-30T10:00:00", &la), None);
    assert_eq!(GraphTime::parse("nonsense", &la), None);
}

#[test]
fn formats_and_reads_date_parts_and_instants() {
    let la = la();
    assert_eq!(
        GraphTime::format(at(1_790_355_600), &la),
        "2026-09-25T10:00:00"
    );
    assert_eq!(
        GraphTime::date("2026-09-25T00:00:00.0000000"),
        Some(CalendarDate::new(2026, 9, 25))
    );
    assert_eq!(
        GraphTime::midnight(&CalendarDate::new(2026, 9, 5)),
        "2026-09-05T00:00:00"
    );
    assert_eq!(
        GraphTime::date_text(&CalendarDate::new(2026, 9, 5)),
        "2026-09-05"
    );
    assert_eq!(
        GraphTime::instant_text(at(1_790_355_600)),
        "2026-09-25T17:00:00Z"
    );
}

#[test]
fn parses_instants_with_z_offset_and_fractions() {
    let expected = Some(at(1_790_355_600));
    assert_eq!(GraphTime::parse_instant("2026-09-25T17:00:00Z"), expected);
    assert_eq!(
        GraphTime::parse_instant("2026-09-25T17:00:00.1234567Z"),
        expected
    );
    assert_eq!(
        GraphTime::parse_instant("2026-09-25T10:00:00-07:00"),
        expected
    );
    assert_eq!(
        GraphTime::parse_instant("2026-09-25T22:30:00+05:30"),
        expected
    );
    // No offset: UTC.
    assert_eq!(
        GraphTime::parse_instant("2026-09-25T17:00:00.0000000"),
        expected
    );
    assert_eq!(GraphTime::parse_instant("garbage"), None);
}
