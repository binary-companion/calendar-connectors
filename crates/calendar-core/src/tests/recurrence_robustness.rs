//! Tests with no Swift counterpart: the shared `split` helper against Swift's results (probed with the Swift
//! toolchain), and the rule that parsers never panic.

use super::support::zone;
use crate::recurrence::rule::RecurrenceRule;
use crate::recurrence::set::RecurrenceSet;
use crate::recurrence::swift_split;

#[test]
fn swift_split_matches_the_stdlib() {
    assert_eq!(swift_split("=A=B", '=', Some(1), true), ["A", "B"]);
    assert_eq!(swift_split("A=", '=', Some(1), true), ["A"]);
    assert_eq!(swift_split("A==B", '=', Some(1), true), ["A", "=B"]);
    assert!(swift_split("==", '=', Some(1), true).is_empty());
    assert_eq!(swift_split("", ',', None, false), [""]);
    assert_eq!(swift_split(",", ',', None, false), ["", ""]);
    assert_eq!(swift_split("a,,b", ',', None, true), ["a", "b"]);
    assert_eq!(swift_split("=A=B=C", '=', Some(1), true), ["A", "B=C"]);
}

#[test]
fn a_leading_equals_makes_the_next_pair_the_part() {
    // Swift: "=X=Y" splits to ["X", "Y"], so the empty name is skipped and X-less part X=Y is read.
    let rule = RecurrenceRule::from_rrule("FREQ=DAILY;=X=Y", None).unwrap();
    assert_eq!(rule.unrecognized_parts.len(), 1);
    assert_eq!(
        (
            rule.unrecognized_parts[0].name.as_str(),
            rule.unrecognized_parts[0].value.as_str()
        ),
        ("X", "Y")
    );
}

/// 2,000 pseudo-random strings from a fixed-seed generator, plus every prefix of real rules.
fn odd_texts() -> Vec<String> {
    let alphabet: Vec<char> = "FREQ=DAILYWEKMONTHSUC;,-+0123456789T:Z \n\u{0662}\u{301}ßİ"
        .chars()
        .collect();
    let mut state: u64 = 0x1234_5678_9ABC_DEF1;
    let mut next = move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as usize
    };
    let mut texts: Vec<String> = (0..2000)
        .map(|_| {
            (0..next() % 40)
                .map(|_| alphabet[next() % alphabet.len()])
                .collect()
        })
        .collect();
    for real in [
        "RRULE:FREQ=YEARLY;INTERVAL=2;BYMONTH=1,7;BYWEEKNO=20;BYYEARDAY=1,-1;BYMONTHDAY=15;BYDAY=-1SU;UNTIL=20261005T150000Z;WKST=SU",
        "EXDATE;TZID=America/New_York:20260921T100000,20260928T100000",
    ] {
        for end in 0..=real.len() {
            if real.is_char_boundary(end) {
                texts.push(real[..end].to_string());
            }
        }
    }
    texts
}

#[test]
fn parsers_never_panic_on_odd_text() {
    let new_york = zone("America/New_York");
    for text in odd_texts() {
        let _ = RecurrenceRule::from_rrule(&text, None);
        let _ = RecurrenceRule::from_rrule(&text, Some(&new_york));
        let set = RecurrenceSet::from_icalendar_lines(&[text.as_str()], &new_york, false);
        let _ = set.icalendar_lines(&new_york, false);
        let _ = RecurrenceSet::from_icalendar_lines(&[text.as_str()], &new_york, true);
        let _ = set.has_unreadable_rule();
    }
}

#[test]
fn absurd_numbers_neither_panic_nor_hang() {
    let new_york = zone("America/New_York");
    let anchor = crate::civil::date_from(2026, 1, 1, 9, 0, 0, &new_york).unwrap();
    let before = crate::civil::date_from(2030, 1, 1, 9, 0, 0, &new_york).unwrap();
    for text in [
        "FREQ=YEARLY;INTERVAL=9223372036854775807",
        "FREQ=MONTHLY;INTERVAL=9223372036854775807",
        "FREQ=WEEKLY;INTERVAL=9223372036854775807",
        "FREQ=DAILY;INTERVAL=9223372036854775807",
        "FREQ=HOURLY;INTERVAL=9223372036854775807",
        "FREQ=SECONDLY;INTERVAL=9223372036854775807;COUNT=9223372036854775807",
        "FREQ=DAILY;COUNT=9223372036854775807",
    ] {
        let rule = RecurrenceRule::from_rrule(text, None).unwrap();
        let _ = rule.instances(anchor, &new_york, false, before, 100, Some(before));
        let _ = rule.instances(anchor, &new_york, false, before, 100, None);
    }
}
