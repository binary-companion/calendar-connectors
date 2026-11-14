//! Port of `ValuesTests.swift`.

use calendar_core::all_day::CalendarDate;
use calendar_core::time::Zone;

use super::fixtures::at;
use crate::component::{ICalParameter, ICalProperty};
use crate::ical_values::{ICalDateValue, ICalValues, LocalDateTime};

fn local(day: CalendarDate, hour: i64, tzid: Option<&str>) -> ICalDateValue {
    ICalDateValue::Local {
        local: LocalDateTime::new(day, hour, 0, 0),
        tzid: tzid.map(str::to_string),
    }
}

#[test]
fn parses_date_utc_zoned_and_floating_values() {
    let day = CalendarDate::new(2026, 9, 27);
    let date_property = ICalProperty::new(
        "DTSTART",
        vec![ICalParameter::single("VALUE", "DATE")],
        "20260927",
    );
    assert_eq!(
        ICalValues::date_value(&date_property),
        Some(ICalDateValue::Date(day))
    );
    assert_eq!(
        ICalValues::date_value(&ICalProperty::with_value("DTSTART", "20260927T150000Z")),
        Some(ICalDateValue::Utc(at(1_790_521_200)))
    );
    let berlin = ICalProperty::new(
        "DTSTART",
        vec![ICalParameter::single("TZID", "Europe/Berlin")],
        "20260927T170000",
    );
    assert_eq!(
        ICalValues::date_value(&berlin),
        Some(local(day, 17, Some("Europe/Berlin")))
    );
    assert_eq!(
        ICalValues::date_value(&ICalProperty::with_value("DTSTART", "20260927T170000")),
        Some(local(day, 17, None))
    );
    // An 8-digit value without VALUE=DATE is still a date (servers omit the parameter).
    assert_eq!(
        ICalValues::date_value(&ICalProperty::with_value("DTSTART", "20260927")),
        Some(ICalDateValue::Date(day))
    );
    assert_eq!(
        ICalValues::date_value(&ICalProperty::with_value("DTSTART", "2026-09-27")),
        None
    );
}

#[test]
fn parses_date_lists_and_rejects_periods() {
    let list = ICalProperty::new(
        "EXDATE",
        vec![ICalParameter::single("TZID", "America/New_York")],
        "20260901T090000,20260908T090000",
    );
    assert_eq!(ICalValues::date_values(&list).len(), 2);
    let period = ICalProperty::new(
        "RDATE",
        vec![ICalParameter::single("VALUE", "PERIOD")],
        "20260901T090000Z/PT1H",
    );
    assert!(ICalValues::date_values(&period).is_empty());
}

#[test]
fn parses_and_writes_durations() {
    assert_eq!(ICalValues::duration("PT15M"), Some(900.0));
    assert_eq!(ICalValues::duration("-PT15M"), Some(-900.0));
    assert_eq!(ICalValues::duration("P1DT2H3M4S"), Some(93_784.0));
    assert_eq!(ICalValues::duration("P2W"), Some(1_209_600.0));
    assert_eq!(ICalValues::duration("+P1D"), Some(86_400.0));
    assert_eq!(ICalValues::duration("PT"), None);
    assert_eq!(ICalValues::duration("15M"), None);
    assert_eq!(ICalValues::duration_text(-900.0), "-PT15M");
    assert_eq!(ICalValues::duration_text(93_784.0), "P1DT2H3M4S");
    assert_eq!(ICalValues::duration_text(0.0), "PT0S");
    assert_eq!(ICalValues::duration_text(86_400.0), "P1D");
}

#[test]
fn formats_texts() {
    let date = at(1_790_521_200); // 2026-09-27 15:00Z
    assert_eq!(ICalValues::utc_text(date), "20260927T150000Z");
    assert_eq!(
        ICalValues::local_text(date, &Zone::named("America/New_York").unwrap()),
        "20260927T110000"
    );
    assert_eq!(
        ICalValues::date_text(&CalendarDate::new(2026, 1, 5)),
        "20260105"
    );
}
