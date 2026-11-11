//! Review Focus 2 of the phase 1 plan: a parser given garbage returns an error and never panics. 2,000 pseudo-random
//! byte strings go through `ICalParser::parse`; 200 truncations of real samples and 2,000 damaged copies of them go
//! through the parser, `EventResource`, `EventReader` and `SeriesEditor`. (Not a port: Swift has no such test.)

use std::collections::BTreeSet;

use calendar_core::time::{DateInterval, Zone};

use super::fixtures::{WEEKLY_SERIES, la, la_time};
use crate::content_line::ICalParser;
use crate::event_reader::{EventReadContext, EventReader};
use crate::event_resource::EventResource;
use crate::series_editor::SeriesEditor;

/// The generator the vector exporter uses (conventions section 9), so the cases are the same on every machine.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

/// Pieces that make damaged samples interesting: separators, digits, date and duration letters, escapes, non-ASCII.
const PIECES: &[&str] = &[
    ";",
    ":",
    ",",
    "=",
    "\"",
    "\\",
    "\r\n",
    "\n",
    "\r",
    " ",
    "\t",
    "Z",
    "T",
    "P",
    "-",
    "+",
    "0",
    "9",
    "20260901T100000",
    "99991231T235959Z",
    "00000101",
    "\u{0301}",
    "é",
    "🙂",
    "\u{200D}",
    "\u{037E}",
    "BEGIN:VEVENT",
    "END:VEVENT",
    "RRULE:FREQ=DAILY;COUNT=99999999999999999999",
    "P99999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999D",
    "TZID=",
    "VALUE=DATE",
    "EXDATE",
    "RECURRENCE-ID",
    "DTSTART",
    "DURATION",
    "-P1W",
    "FREQ=YEARLY;BYMONTH=13",
];

fn context() -> EventReadContext {
    EventReadContext::new(
        "home",
        "g.ics",
        None,
        None,
        la(),
        BTreeSet::from(["mailto:me@icloud.test".to_string()]),
    )
}

fn window() -> DateInterval {
    DateInterval::new(la_time!(2026, 8, 1, 0), la_time!(2027, 1, 1, 0))
}

/// Everything a host does with a resource that parsed.
fn exercise(resource: &EventResource) {
    let context = context();
    let window = window();
    let _ = resource.uid();
    let _ = resource.serialized();
    let _ = EventReader::events_in(resource, &window, &context);
    let _ = EventReader::master_event_of(resource, &context);
    let _ = EventReader::series_of(resource, &context);
    let slot = la_time!(2026, 9, 22);
    let _ = EventReader::occurrence_in(resource, slot, &context);
    let ny = Zone::named("America/New_York").unwrap();
    let _ = SeriesEditor::override_in(resource, slot, &la());
    let mut copy = resource.clone();
    let _ = SeriesEditor::exclude(slot, &mut copy, &la());
    let mut copy = resource.clone();
    SeriesEditor::shift(&mut copy, 3600.0, &ny);
    let mut copy = resource.clone();
    SeriesEditor::shift(&mut copy, 90_000.0, &la());
    let mut copy = resource.clone();
    SeriesEditor::prune_unmatched(&mut copy, &la());
    let _ = SeriesEditor::split(resource, slot, "new", &la(), la_time!(2026, 9, 1));
}

fn parse_and_exercise(bytes: &[u8]) -> bool {
    match ICalParser::parse(bytes) {
        Ok(calendar) => {
            if let Ok(resource) = EventResource::new(calendar) {
                exercise(&resource);
                return true;
            }
            false
        }
        Err(_) => false,
    }
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Lcg(0x1CA1_C0DE_0008);
    for _ in 0..2000 {
        let length = rng.below(400);
        let bytes: Vec<u8> = (0..length)
            .map(|_| match rng.below(4) {
                0 => rng.below(256) as u8,
                1 => b"BEGINENDVCALDR:;,=\r\n \t\"\\"[rng.below(24)],
                _ => b"0123456789TZP-+"[rng.below(15)],
            })
            .collect();
        assert!(!parse_and_exercise(&bytes) || bytes.windows(6).any(|w| w == b"BEGIN:"));
        let _ = ICalParser::parse_text(&String::from_utf8_lossy(&bytes));
    }
}

#[test]
fn truncated_samples_never_panic() {
    let samples = [WEEKLY_SERIES.replace('\n', "\r\n"), "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:é🙂\r\nSUMMARY:caf\u{e9} \u{1F642}\r\nDTSTART;VALUE=DATE:20260910\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n".to_string()];
    let mut parsed_whole = 0;
    for sample in &samples {
        let bytes = sample.as_bytes();
        for step in 0..100 {
            let cut = bytes.len() * step / 100;
            parse_and_exercise(&bytes[..cut]);
        }
        if parse_and_exercise(bytes) {
            parsed_whole += 1;
        }
    }
    assert_eq!(parsed_whole, 2);
}

#[test]
fn damaged_samples_never_panic() {
    let mut rng = Lcg(0x1CA1_C0DE_0009);
    let original = WEEKLY_SERIES.replace('\n', "\r\n");
    let mut parsed = 0;
    for _ in 0..2000 {
        let mut text = original.clone().into_bytes();
        for _ in 0..1 + rng.below(3) {
            let at = rng.below(text.len());
            match rng.below(3) {
                0 => {
                    let piece = PIECES[rng.below(PIECES.len())].as_bytes();
                    text.splice(at..(at + 1).min(text.len()), piece.iter().copied());
                }
                1 => {
                    text.truncate(text.len().max(1) - 1);
                    text.insert(
                        at.min(text.len()),
                        PIECES[rng.below(PIECES.len())].as_bytes()[0],
                    );
                }
                _ => text[at] = rng.below(256) as u8,
            }
        }
        if parse_and_exercise(&text) {
            parsed += 1;
        }
    }
    // Enough damaged copies still parse that the readers, not only the parser, are exercised.
    assert!(parsed > 300, "{parsed} of 2000 parsed");
}
