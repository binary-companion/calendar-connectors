//! Port of `SeriesEditorTests.swift`.

use calendar_core::civil;
use calendar_core::recurrence::rule::RuleEnd;
use calendar_core::time::{DateInterval, Zone, add_seconds};
use calendar_core::write::types::{EventField, WriteError};
use jiff::Timestamp;

use super::fixtures::{WEEKLY_SERIES, at, context, context_named, la, la_time, resource};
use crate::component::{ICalParameter, ICalProperty};
use crate::event_reader::{EventReadContext, EventReader};
use crate::event_resource::EventResource;
use crate::event_writer::EventWriter;
use calendar_core::write::event_draft::EventTiming;

fn now() -> Timestamp {
    at(1_790_000_000)
}

fn window() -> DateInterval {
    DateInterval::new(la_time!(2026, 8, 1, 0), la_time!(2027, 1, 1, 0))
}

fn starts(r: &EventResource) -> Vec<Timestamp> {
    EventReader::events_in(r, &window(), &context())
        .iter()
        .map(|e| e.start)
        .collect()
}

fn value<'a>(component: &'a crate::component::ICalComponent, name: &str) -> Option<&'a str> {
    component.property(name).map(|p| p.value.as_str())
}

fn master_value(r: &EventResource, name: &str) -> Option<String> {
    r.master()
        .and_then(|m| m.property(name))
        .map(|p| p.value.clone())
}

#[test]
fn override_for_a_slot_copies_the_master() {
    let r = resource(WEEKLY_SERIES);
    let copy =
        crate::series_editor::SeriesEditor::override_in(&r, la_time!(2026, 9, 22), &la()).unwrap();
    assert_eq!(value(&copy, "RECURRENCE-ID"), Some("20260922T100000"));
    assert_eq!(
        copy.property("RECURRENCE-ID").unwrap().parameter("TZID"),
        Some("America/Los_Angeles")
    );
    assert_eq!(value(&copy, "DTSTART"), Some("20260922T100000"));
    assert_eq!(value(&copy, "DTEND"), Some("20260922T103000"));
    assert!(copy.property("RRULE").is_none() && copy.property("EXDATE").is_none());
    assert_eq!(
        copy.property("SUMMARY").map(|p| p.text()).as_deref(),
        Some("Team sync")
    );
    let existing =
        crate::series_editor::SeriesEditor::override_in(&r, la_time!(2026, 9, 15), &la()).unwrap();
    assert_eq!(
        existing.property("SUMMARY").map(|p| p.text()).as_deref(),
        Some("Team sync (moved)")
    );
    assert!(
        crate::series_editor::SeriesEditor::override_in(&r, la_time!(2026, 9, 23), &la()).is_none()
    );
}

#[test]
fn exclude_adds_an_exdate_and_drops_the_override() {
    use crate::series_editor::SeriesEditor;
    let mut r = resource(WEEKLY_SERIES);
    SeriesEditor::exclude(la_time!(2026, 9, 15), &mut r, &la()).unwrap();
    SeriesEditor::exclude(la_time!(2026, 9, 22), &mut r, &la()).unwrap();
    assert!(r.overrides().is_empty());
    let kept: Vec<Timestamp> = starts(&r)
        .into_iter()
        .filter(|s| *s < la_time!(2026, 10, 1))
        .collect();
    assert_eq!(kept, vec![la_time!(2026, 9, 1), la_time!(2026, 9, 29)]);
    assert_eq!(
        SeriesEditor::exclude(la_time!(2026, 9, 23), &mut r, &la()),
        Err(WriteError::NotFound)
    );
}

#[test]
fn shift_moves_slots_exdates_and_unmoved_overrides() {
    use crate::series_editor::SeriesEditor;
    let mut r = resource(WEEKLY_SERIES);
    // Move the master one hour later, as an all-in-series timing change does, then shift the rest.
    let mut master = r.master().unwrap().clone();
    EventWriter::set_timing(
        &EventTiming::new(
            la_time!(2026, 9, 1, 11),
            la_time!(2026, 9, 1, 11, 30),
            Some(la()),
            false,
        ),
        &mut master,
    );
    let mut events = vec![master];
    events.extend(r.overrides().into_iter().cloned());
    r.set_events(events);
    SeriesEditor::shift(&mut r, 3600.0, &la());
    let override_event = r.overrides()[0].clone();
    assert_eq!(
        value(&override_event, "RECURRENCE-ID"),
        Some("20260915T110000")
    );
    assert_eq!(value(&override_event, "DTSTART"), Some("20260916T140000")); // it had its own time: kept
    assert_eq!(
        master_value(&r, "EXDATE").as_deref(),
        Some("20260908T110000")
    );
    let kept: Vec<Timestamp> = starts(&r)
        .into_iter()
        .filter(|s| *s < la_time!(2026, 10, 1))
        .collect();
    assert_eq!(
        kept,
        vec![
            la_time!(2026, 9, 1, 11),
            la_time!(2026, 9, 16, 14),
            la_time!(2026, 9, 22, 11),
            la_time!(2026, 9, 29, 11)
        ]
    );
}

#[test]
fn all_day_shift_moves_by_whole_days_across_dst() {
    use crate::series_editor::SeriesEditor;
    // A weekly all-day series from Friday 2026-10-30; moving it one day later crosses the 2026-11-01 DST change.
    let mut r = resource(
        "BEGIN:VCALENDAR
VERSION:2.0
BEGIN:VEVENT
UID:days
DTSTART;VALUE=DATE:20261030
DTEND;VALUE=DATE:20261031
RRULE:FREQ=WEEKLY
EXDATE;VALUE=DATE:20261106
END:VEVENT
END:VCALENDAR",
    );
    let mut master = r.master().unwrap().clone();
    master.set(ICalProperty::new(
        "DTSTART",
        vec![ICalParameter::single("VALUE", "DATE")],
        "20261031",
    ));
    master.set(ICalProperty::new(
        "DTEND",
        vec![ICalParameter::single("VALUE", "DATE")],
        "20261101",
    ));
    r.set_events(vec![master]);
    // 25 hours between the two local midnights in Los Angeles: still one day.
    SeriesEditor::shift(&mut r, 90_000.0, &la());
    assert_eq!(master_value(&r, "EXDATE").as_deref(), Some("20261107"));
}

#[test]
fn prune_drops_what_no_longer_matches() {
    use crate::series_editor::SeriesEditor;
    let mut r = resource(WEEKLY_SERIES);
    let mut master = r.master().unwrap().clone();
    master.set(ICalProperty::with_value(
        "RRULE",
        "FREQ=WEEKLY;BYDAY=TU;INTERVAL=3",
    )); // 1st, 22nd: the 8th and 15th are gone
    let mut events = vec![master];
    events.extend(r.overrides().into_iter().cloned());
    r.set_events(events);
    SeriesEditor::prune_unmatched(&mut r, &la());
    assert!(r.overrides().is_empty());
    assert!(master_value(&r, "EXDATE").is_none());
}

#[test]
fn split_until_rule() {
    use crate::series_editor::SeriesEditor;
    let text = WEEKLY_SERIES.replace(
        "RRULE:FREQ=WEEKLY;BYDAY=TU",
        "RRULE:FREQ=WEEKLY;BYDAY=TU;UNTIL=20261231T235959Z",
    );
    let (head, tail) = SeriesEditor::split(
        &resource(&text),
        la_time!(2026, 9, 22),
        "new-uid",
        &la(),
        now(),
    )
    .unwrap();
    assert_eq!(
        starts(&head),
        vec![la_time!(2026, 9, 1), la_time!(2026, 9, 16, 14)]
    );
    assert!(
        master_value(&head, "RRULE")
            .unwrap()
            .contains("UNTIL=20260922T165959Z")
    );
    assert_eq!(tail.uid().as_deref(), Some("new-uid"));
    assert!(
        master_value(&tail, "RRULE")
            .unwrap()
            .contains("UNTIL=20261231T235959Z")
    );
    assert_eq!(
        master_value(&tail, "DTSTART").as_deref(),
        Some("20260922T100000")
    );
    assert_eq!(starts(&tail).first().copied(), Some(la_time!(2026, 9, 22)));
    assert_eq!(starts(&tail).last().copied(), Some(la_time!(2026, 12, 29)));
    let _ = RuleEnd::Never;
}

#[test]
fn split_count_rule_keeps_exact_counts() {
    use crate::series_editor::SeriesEditor;
    let text = WEEKLY_SERIES.replace(
        "RRULE:FREQ=WEEKLY;BYDAY=TU",
        "RRULE:FREQ=WEEKLY;BYDAY=TU;COUNT=6",
    );
    let (head, tail) = SeriesEditor::split(
        &resource(&text),
        la_time!(2026, 9, 22),
        "new-uid",
        &la(),
        now(),
    )
    .unwrap();
    // Before the 22nd the rule generated the 1st, 8th (excluded, still counted) and 15th (moved): COUNT=3 stays, 3 move.
    assert_eq!(
        master_value(&head, "RRULE").as_deref(),
        Some("FREQ=WEEKLY;BYDAY=TU;COUNT=3")
    );
    assert_eq!(
        master_value(&tail, "RRULE").as_deref(),
        Some("FREQ=WEEKLY;BYDAY=TU;COUNT=3")
    );
    assert_eq!(
        starts(&tail),
        vec![
            la_time!(2026, 9, 22),
            la_time!(2026, 9, 29),
            la_time!(2026, 10, 6)
        ]
    );
    assert!(!master_value(&head, "RRULE").unwrap().contains("UNTIL"));
}

#[test]
fn split_open_rule_stays_open_and_moves_later_exceptions() {
    use crate::series_editor::SeriesEditor;
    let text = WEEKLY_SERIES.replace(
        "EXDATE;TZID=America/Los_Angeles:20260908T100000",
        "EXDATE;TZID=America/Los_Angeles:20260908T100000,20261006T100000",
    );
    let (head, tail) = SeriesEditor::split(
        &resource(&text),
        la_time!(2026, 9, 15),
        "new-uid",
        &la(),
        now(),
    )
    .unwrap();
    assert!(head.overrides().is_empty()); // the 15th's override moved
    assert_eq!(
        master_value(&head, "EXDATE").as_deref(),
        Some("20260908T100000")
    );
    assert_eq!(tail.overrides().len(), 1);
    assert_eq!(
        tail.overrides()[0]
            .property("UID")
            .map(|p| p.text())
            .as_deref(),
        Some("new-uid")
    ); // re-stamped
    assert_eq!(
        master_value(&tail, "EXDATE").as_deref(),
        Some("20261006T100000")
    );
    assert_eq!(
        master_value(&tail, "RRULE").as_deref(),
        Some("FREQ=WEEKLY;BYDAY=TU")
    );
    let first_three: Vec<Timestamp> = starts(&tail).into_iter().take(3).collect();
    assert_eq!(
        first_three,
        vec![
            la_time!(2026, 9, 16, 14),
            la_time!(2026, 9, 22),
            la_time!(2026, 9, 29)
        ]
    );
}

#[test]
fn split_at_a_non_slot_is_not_found_and_multiple_rules_are_unsupported() {
    use crate::series_editor::SeriesEditor;
    assert_eq!(
        SeriesEditor::split(
            &resource(WEEKLY_SERIES),
            la_time!(2026, 9, 23),
            "x",
            &la(),
            now()
        )
        .map(|_| ()),
        Err(WriteError::NotFound)
    );
    let two_rules = WEEKLY_SERIES.replace(
        "RRULE:FREQ=WEEKLY;BYDAY=TU",
        "RRULE:FREQ=WEEKLY;BYDAY=TU\nRRULE:FREQ=MONTHLY",
    );
    assert_eq!(
        SeriesEditor::split(
            &resource(&two_rules),
            la_time!(2026, 9, 22),
            "x",
            &la(),
            now()
        )
        .map(|_| ()),
        Err(WriteError::Unsupported {
            fields: [EventField::Recurrence].into_iter().collect()
        })
    );
}

fn all_day_daily_series(rrule: &str) -> String {
    format!(
        "BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Test//EN
BEGIN:VEVENT
UID:ALLDAY-UID
DTSTAMP:20260301T000000Z
SUMMARY:Daily all-day
DTSTART;VALUE=DATE:20260305
DTEND;VALUE=DATE:20260306
{rrule}
END:VEVENT
END:VCALENDAR"
    )
}

#[test]
fn all_day_split_after_a_spring_forward_day_keeps_the_day_before_the_slot() {
    use crate::series_editor::SeriesEditor;
    let ny = Zone::named("America/New_York").unwrap();
    let mar9 = civil::date_from(2026, 3, 9, 0, 0, 0, &ny).unwrap();
    let mar8 = civil::date_from(2026, 3, 8, 0, 0, 0, &ny).unwrap();
    let r = resource(&all_day_daily_series("RRULE:FREQ=DAILY"));
    let (head, tail) = SeriesEditor::split(&r, mar9, "new-uid", &ny, now()).unwrap();
    // Mar 8 is 23 hours long in New York; the previous day is Mar 8, not Mar 7.
    assert_eq!(
        master_value(&head, "RRULE").as_deref(),
        Some("FREQ=DAILY;UNTIL=20260308")
    );
    let ctx = EventReadContext::new(
        "home",
        "a.ics",
        Some("\"e\"".to_string()),
        Some("s".to_string()),
        ny.clone(),
        Default::default(),
    );
    let head_starts: Vec<Timestamp> = EventReader::events_in(
        &head,
        &DateInterval::new(
            add_seconds(mar8, -86_400.0),
            add_seconds(mar9, 86_400.0 * 3.0),
        ),
        &ctx,
    )
    .iter()
    .map(|e| e.start)
    .collect();
    assert!(head_starts.contains(&mar8));
    assert!(!head_starts.contains(&mar9));
    assert_eq!(master_value(&tail, "DTSTART").as_deref(), Some("20260309"));
}

#[test]
fn timed_split_across_the_november_change_keeps_the_head_strictly_before_the_slot() {
    use crate::series_editor::SeriesEditor;
    let text = WEEKLY_SERIES
        .replace("RRULE:FREQ=WEEKLY;BYDAY=TU", "RRULE:FREQ=WEEKLY;BYDAY=SU")
        .replace("EXDATE;TZID=America/Los_Angeles:20260908T100000\n", "")
        .replace("20260901T100000", "20261018T013000")
        .replace("20260901T103000", "20261018T020000");
    let r = resource(&text);
    let oct25 = la_time!(2026, 10, 25, 1, 30);
    let nov1 = la_time!(2026, 11, 1, 1, 30);
    let nov8 = la_time!(2026, 11, 8, 1, 30);
    let wide = DateInterval::new(la_time!(2026, 10, 1, 0), la_time!(2026, 12, 1, 0));
    for slot in [nov1, nov8] {
        let (head, tail) = SeriesEditor::split(&r, slot, "new-uid", &la(), now()).unwrap();
        let h: Vec<Timestamp> = EventReader::events_in(&head, &wide, &context())
            .iter()
            .map(|e| e.start)
            .collect();
        let t: Vec<Timestamp> = EventReader::events_in(&tail, &wide, &context())
            .iter()
            .map(|e| e.start)
            .collect();
        assert!(
            h.contains(&oct25) && h.iter().all(|s| *s < slot),
            "head for {slot}"
        );
        assert_eq!(t.first().copied(), Some(slot), "tail for {slot}");
    }
    let _ = context_named;
}
