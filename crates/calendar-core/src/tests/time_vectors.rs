use jiff::Timestamp;

use super::{assert_no_failures, vector_cases};
use crate::civil::{self, Unit};
use crate::time::{DateInterval, Zone, add_seconds, seconds_between};

const OFFSETS: &str = include_str!("../../../../vectors/zones/offsets.json");
const KNOWN: &str = include_str!("../../../../vectors/zones/known.json");
const WALL_CLOCK: &str = include_str!("../../../../vectors/zones/wall-clock.json");

fn ts(ms: i64) -> Timestamp {
    Timestamp::from_millisecond(ms).unwrap()
}

#[test]
fn offsets_vector() {
    let cases = vector_cases(OFFSETS);
    let mut failures = Vec::new();
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let zone_id = case["input"]["zone"].as_str().unwrap();
        let instant = case["input"]["instant"].as_i64().unwrap();
        let expected = case["expected"]["offsetSeconds"].as_i64().unwrap();
        match Zone::named(zone_id) {
            None => failures.push(format!("{name}: zone {zone_id} not accepted")),
            Some(zone) => {
                let got = i64::from(zone.offset_seconds(ts(instant)));
                if got != expected {
                    failures.push(format!(
                        "{name}: {zone_id} at {instant}: got {got}, expected {expected}"
                    ));
                }
            }
        }
    }
    assert_eq!(cases.len(), 2400);
    assert_no_failures("zones/offsets", cases.len(), failures);
}

#[test]
fn known_vector() {
    let cases = vector_cases(KNOWN);
    let mut failures = Vec::new();
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let input = case["input"]["identifier"].as_str().unwrap();
        let accepted = case["expected"]["accepted"].as_bool().unwrap();
        let identifier = case["expected"]["identifier"].as_str();
        let got = Zone::named(input);
        if got.is_some() != accepted {
            failures.push(format!(
                "{name}: {input:?}: accepted {} expected {accepted}",
                got.is_some()
            ));
        } else if let Some(zone) = got {
            let spelled = zone.identifier();
            if Some(spelled) != identifier {
                failures.push(format!(
                    "{name}: {input:?}: identifier {spelled:?} expected {identifier:?}"
                ));
            }
        }
    }
    assert_eq!(cases.len(), 483);
    assert_no_failures("zones/known", cases.len(), failures);
}

/// `recurrence_vectors` replays this vector through `WallClock`; replaying it here pins `civil::date_from` directly.
#[test]
fn wall_clock_vector_through_date_from() {
    let cases = vector_cases(WALL_CLOCK);
    let mut failures = Vec::new();
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let input = &case["input"];
        let zone = Zone::named(input["zone"].as_str().unwrap()).unwrap();
        let get = |v: &serde_json::Value| v.as_i64().unwrap();
        let got = civil::date_from(
            get(&input["date"]["year"]),
            get(&input["date"]["month"]),
            get(&input["date"]["day"]),
            get(&input["hour"]),
            get(&input["minute"]),
            get(&input["second"]),
            &zone,
        )
        .map(|t| t.as_millisecond());
        let expected = case["expected"]["instant"].as_i64();
        if got != expected {
            failures.push(format!("{name}: got {got:?}, expected {expected:?}"));
        }
    }
    assert_no_failures("zones/wall-clock", cases.len(), failures);
}

#[test]
fn fixed_zones() {
    assert_eq!(Zone::utc().identifier(), "GMT");
    let z = Zone::fixed(3600).unwrap();
    assert_eq!(z.identifier(), "GMT+0100");
    assert_eq!(z.offset_seconds(ts(0)), 3600);
    let z = Zone::fixed(-19800).unwrap();
    assert_eq!(z.identifier(), "GMT-0530");
    assert_eq!(Zone::fixed(0).unwrap(), Zone::utc());
    assert!(Zone::fixed(18 * 3600 + 1).is_none());
    assert!(Zone::fixed(-(18 * 3600) - 1).is_none());
    assert!(Zone::fixed(18 * 3600).is_some());
}

/// `TimeZone(secondsFromGMT:).identifier` for offsets with seconds, read from Foundation: whole minutes, rounded half
/// up. Found by `ical/timezones.json` (the `+013045` observance), which is not a multiple of a minute.
#[test]
fn fixed_zone_identifiers_round_to_the_minute() {
    for (seconds, identifier) in [
        (1, "GMT"),
        (29, "GMT"),
        (30, "GMT+0001"),
        (89, "GMT+0001"),
        (90, "GMT+0002"),
        (3599, "GMT+0100"),
        (3630, "GMT+0101"),
        (5429, "GMT+0130"),
        (5430, "GMT+0131"),
        (5445, "GMT+0131"),
        (-29, "GMT"),
        (-30, "GMT-0001"),
        (-5445, "GMT-0131"),
        (-5429, "GMT-0130"),
        (64800, "GMT+1800"),
        (-64800, "GMT-1800"),
    ] {
        let zone = Zone::fixed(seconds).unwrap();
        assert_eq!(zone.identifier(), identifier, "{seconds} s");
        assert_eq!(zone.offset_seconds(ts(0)), seconds, "{seconds} s");
    }
}

#[test]
fn zone_equality_ordering_hashing_use_identifier() {
    use std::collections::HashSet;
    let a = Zone::named("America/New_York").unwrap();
    let b = Zone::named("America/New_York").unwrap();
    let c = Zone::named("Europe/Paris").unwrap();
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert!(a < c);
    let set: HashSet<Zone> = [a, b, c].into_iter().collect();
    assert_eq!(set.len(), 2);
}

#[test]
fn date_interval_semantics() {
    let a = DateInterval::new(ts(0), ts(10_000));
    assert_eq!(a.duration(), 10.0);
    assert!(a.contains(ts(0)));
    assert!(a.contains(ts(10_000)));
    assert!(!a.contains(ts(10_001)));
    assert!(!a.contains(ts(-1)));
    let touching = DateInterval::new(ts(10_000), ts(20_000));
    assert!(a.intersects(&touching));
    assert!(touching.intersects(&a));
    let apart = DateInterval::new(ts(10_001), ts(20_000));
    assert!(!a.intersects(&apart));
}

#[test]
fn add_seconds_and_seconds_between() {
    assert_eq!(add_seconds(ts(0), 1.5), ts(1500));
    assert_eq!(add_seconds(ts(0), -0.25), ts(-250));
    // Rounded to the nearest nanosecond.
    assert_eq!(add_seconds(ts(0), 1e-9 * 1.4).as_nanosecond(), 1);
    assert_eq!(add_seconds(ts(0), 1e-9 * 1.6).as_nanosecond(), 2);
    // Non-finite and out-of-range input never panics.
    assert_eq!(add_seconds(ts(5), f64::NAN), ts(5));
    assert_eq!(add_seconds(ts(5), f64::INFINITY), Timestamp::MAX);
    assert_eq!(add_seconds(ts(5), f64::NEG_INFINITY), Timestamp::MIN);
    assert_eq!(add_seconds(ts(5), 1e300), Timestamp::MAX);
    assert_eq!(seconds_between(ts(1000), ts(3500)), 2.5);
    assert_eq!(seconds_between(ts(3500), ts(1000)), -2.5);
}

fn zone(id: &str) -> Zone {
    Zone::named(id).unwrap()
}

fn civil_ms(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64, id: &str) -> i64 {
    civil::date_from(y, mo, d, h, mi, s, &zone(id))
        .unwrap()
        .as_millisecond()
}

#[test]
fn components_and_weekday() {
    // 2026-03-08 is a Sunday; 12:34:56 UTC.
    let t = ts(1_772_973_296_000);
    let c = civil::components(t, &Zone::utc());
    assert_eq!(
        (
            c.year, c.month, c.day, c.hour, c.minute, c.second, c.weekday
        ),
        (2026, 3, 8, 12, 34, 56, 1)
    );
    let tokyo = civil::components(t, &zone("Asia/Tokyo"));
    assert_eq!((tokyo.day, tokyo.hour, tokyo.weekday), (8, 21, 1));
    let fiji_next_day = civil::components(t, &zone("Pacific/Kiritimati"));
    assert_eq!((fiji_next_day.day, fiji_next_day.weekday), (9, 2));
}

#[test]
fn start_of_day_follows_the_zone() {
    let ny = zone("America/New_York");
    // 2026-03-08 15:00 UTC is 11:00 EDT; the day began at 05:00 UTC (EST, before the jump at 02:00).
    let t = ts(1_772_982_000_000);
    assert_eq!(
        civil::start_of_day(t, &ny).as_millisecond(),
        civil_ms(2026, 3, 8, 0, 0, 0, "America/New_York")
    );
    assert_eq!(
        civil::start_of_day(t, &ny).as_millisecond(),
        1_772_946_000_000
    );
    // A midnight skipped by a jump: Sao Paulo (old rules) is not needed; Havana 2024-03-10 jumps at 00:00 -> 01:00.
    let havana = zone("America/Havana");
    let noon = civil_ms(2024, 3, 10, 12, 0, 0, "America/Havana");
    assert_eq!(
        civil::start_of_day(ts(noon), &havana).as_millisecond(),
        civil_ms(2024, 3, 10, 1, 0, 0, "America/Havana")
    );
}

#[test]
fn date_from_resolves_gaps_forward_and_overlaps_to_the_first() {
    // New York 2026-03-08: 02:30 does not exist, moves forward to 03:30 EDT = 07:30 UTC.
    assert_eq!(
        civil_ms(2026, 3, 8, 2, 30, 0, "America/New_York"),
        1_772_955_000_000
    );
    // 2026-11-01: 01:30 happens twice; the first (EDT, 05:30 UTC) is used.
    assert_eq!(
        civil_ms(2026, 11, 1, 1, 30, 0, "America/New_York"),
        1_793_511_000_000
    );
    // Lord Howe has a 30 minute step (02:00 -> 02:30 on 2026-10-04): 02:15 is skipped and moves forward by the gap,
    // to the same instant as 02:45; 02:30 itself is the first real instant after the jump.
    let skipped = civil_ms(2026, 10, 4, 2, 15, 0, "Australia/Lord_Howe");
    assert_eq!(
        skipped,
        civil_ms(2026, 10, 4, 2, 45, 0, "Australia/Lord_Howe")
    );
    assert_eq!(
        civil_ms(2026, 10, 4, 2, 30, 0, "Australia/Lord_Howe")
            - civil_ms(2026, 10, 4, 1, 59, 0, "Australia/Lord_Howe"),
        60_000
    );
    assert!(civil::date_from(2026, 2, 30, 0, 0, 0, &Zone::utc()).is_some());
    // Out of range or absurd input is None, never a panic.
    assert!(civil::date_from(i64::MAX, 1, 1, 0, 0, 0, &Zone::utc()).is_none());
    assert!(civil::date_from(2026, i64::MIN, 1, 0, 0, 0, &Zone::utc()).is_none());
    assert!(civil::date_from(1, 1, i64::MAX, 0, 0, 0, &Zone::utc()).is_none());
    assert!(civil::date_from(1, 1, 1, i64::MAX, i64::MAX, i64::MAX, &Zone::utc()).is_none());
}

#[test]
fn date_from_rolls_over_out_of_range_components_like_foundation() {
    let utc = Zone::utc();
    let a = civil::date_from(2026, 13, 1, 0, 0, 0, &utc).unwrap();
    let b = civil::date_from(2027, 1, 1, 0, 0, 0, &utc).unwrap();
    assert_eq!(a, b);
    let c = civil::date_from(2026, 1, 32, 25, 0, 0, &utc).unwrap();
    let d = civil::date_from(2026, 2, 2, 1, 0, 0, &utc).unwrap();
    assert_eq!(c, d);
}

#[test]
fn add_month_clamps_to_month_end() {
    let utc = Zone::utc();
    let jan31 = civil::date_from(2026, 1, 31, 9, 0, 0, &utc).unwrap();
    let feb = civil::add(Unit::Month, 1, jan31, &utc).unwrap();
    assert_eq!(feb, civil::date_from(2026, 2, 28, 9, 0, 0, &utc).unwrap());
    let leap = civil::add(
        Unit::Month,
        1,
        civil::date_from(2024, 1, 31, 9, 0, 0, &utc).unwrap(),
        &utc,
    )
    .unwrap();
    assert_eq!(leap, civil::date_from(2024, 2, 29, 9, 0, 0, &utc).unwrap());
    let back = civil::add(Unit::Month, -2, jan31, &utc).unwrap();
    assert_eq!(back, civil::date_from(2025, 11, 30, 9, 0, 0, &utc).unwrap());
    let year = civil::add(
        Unit::Year,
        1,
        civil::date_from(2024, 2, 29, 9, 0, 0, &utc).unwrap(),
        &utc,
    )
    .unwrap();
    assert_eq!(year, civil::date_from(2025, 2, 28, 9, 0, 0, &utc).unwrap());
}

#[test]
fn add_day_keeps_wall_time_across_dst_and_hour_is_elapsed_time() {
    let ny = zone("America/New_York");
    let before = civil::date_from(2026, 3, 7, 9, 0, 0, &ny).unwrap();
    let next = civil::add(Unit::Day, 1, before, &ny).unwrap();
    assert_eq!(next, civil::date_from(2026, 3, 8, 9, 0, 0, &ny).unwrap());
    assert_eq!(seconds_between(before, next), 23.0 * 3600.0);
    // A day that lands in the gap moves forward.
    let two_thirty = civil::date_from(2026, 3, 7, 2, 30, 0, &ny).unwrap();
    let moved = civil::add(Unit::Day, 1, two_thirty, &ny).unwrap();
    assert_eq!(moved.as_millisecond(), 1_772_955_000_000);
    // Hours, minutes and seconds are elapsed time.
    let h = civil::add(Unit::Hour, 24, before, &ny).unwrap();
    assert_eq!(seconds_between(before, h), 24.0 * 3600.0);
    assert_eq!(
        civil::add(Unit::Minute, 90, before, &ny).unwrap(),
        add_seconds(before, 5400.0)
    );
    assert_eq!(
        civil::add(Unit::Second, -5, before, &ny).unwrap(),
        add_seconds(before, -5.0)
    );
    // Overflow is None.
    assert!(civil::add(Unit::Year, i64::MAX, before, &ny).is_none());
    assert!(civil::add(Unit::Day, i64::MIN, before, &ny).is_none());
    assert!(civil::add(Unit::Hour, i64::MAX, before, &ny).is_none());
}
