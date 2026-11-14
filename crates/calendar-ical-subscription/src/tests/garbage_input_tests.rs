//! Review Focus 2 of the phase 1 plan: a parser given garbage returns an error and never panics. 2,000 pseudo-random
//! byte strings go through `FeedParser::parse`, 200 truncations of a real sample (cut at every sixth byte of the
//! sample, so each cut falls somewhere else) and 2,000 damaged copies of it go through the parser with and without a
//! retention window, and every feed that parses is then read through the shared `EventReader`. (Not a port: Swift has
//! no such test.)

use std::collections::BTreeSet;

use calendar_core::time::DateInterval;
use calendar_ical::{EventReadContext, EventReader};

use super::fixtures::{pacific, pt, sample_feed};
use crate::feed_fetcher::FeedFetcher;
use crate::feed_parser::{FeedParser, ParsedFeed};

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
    "BEGIN:VEVENT",
    "END:VEVENT",
    "BEGIN:VCALENDAR",
    "END:VCALENDAR",
    "BEGIN:VTIMEZONE",
    "END:VTIMEZONE",
    "UID:",
    "RRULE:FREQ=DAILY",
    "RRULE:FREQ=",
    "RRULE:INTERVAL=0",
    "DTSTART:",
    "DTSTART;TZID=Nowhere/Land:20260901T100000",
    "RECURRENCE-ID:20260901T100000",
    "X-WR-TIMEZONE:Europe/Berlin",
    "X-WR-TIMEZONE:Not/AZone",
    "X-APPLE-CALENDAR-COLOR:#12345678",
    "COLOR:",
    "EXDATE:",
    "RDATE:",
];

fn read(feed: &ParsedFeed) {
    let window = DateInterval::new(pt(2026, 8, 1, 0, 0), pt(2026, 12, 1, 0, 0));
    for item in &feed.resources {
        let context = EventReadContext::new(
            "feed",
            item.name.clone(),
            None,
            None,
            pacific(),
            BTreeSet::new(),
        );
        let _ = EventReader::events_in(&item.resource, &window, &context);
    }
    let _ = feed.fingerprint();
}

fn parse_all_ways(bytes: &[u8]) {
    for retention in [
        None,
        Some((
            DateInterval::new(pt(2026, 9, 1, 0, 0), pt(2026, 9, 30, 0, 0)),
            pacific(),
        )),
    ] {
        if let Ok(feed) = FeedParser::parse_with_retention(bytes, retention) {
            read(&feed);
        }
    }
}

#[test]
fn random_bytes_never_panic_the_feed_parser() {
    let mut random = Lcg(0x5EED);
    for _ in 0..2_000 {
        let length = random.below(400);
        let bytes: Vec<u8> = (0..length).map(|_| random.below(256) as u8).collect();
        parse_all_ways(&bytes);
    }
}

#[test]
fn truncated_real_samples_never_panic_the_feed_parser() {
    let sample = sample_feed().into_bytes();
    let step = (sample.len() / 200).max(1);
    let mut cuts = 0;
    for end in (0..sample.len()).step_by(step) {
        parse_all_ways(&sample[..end]);
        cuts += 1;
    }
    assert!(cuts >= 200, "{cuts} truncations");
}

#[test]
fn damaged_real_samples_never_panic_the_feed_parser() {
    let sample = sample_feed();
    let mut random = Lcg(0xFEED);
    for _ in 0..2_000 {
        let mut bytes = sample.clone().into_bytes();
        for _ in 0..(1 + random.below(4)) {
            let at = random.below(bytes.len());
            match random.below(3) {
                0 => {
                    bytes.splice(at..at, PIECES[random.below(PIECES.len())].bytes());
                }
                1 => {
                    let end = (at + 1 + random.below(30)).min(bytes.len());
                    bytes.drain(at..end);
                }
                _ => bytes[at] = random.below(256) as u8,
            }
        }
        parse_all_ways(&bytes);
    }
}

#[test]
fn a_feed_that_is_html_is_an_error_not_a_panic() {
    let html = "<!DOCTYPE html><html><head><title>Sign in</title></head><body><form>\
                <input name=\"password\"></form></body></html>";
    assert!(FeedParser::parse(html.as_bytes()).is_err());
    assert!(FeedParser::parse(&[0xFF, 0xFE, 0x00, 0x42]).is_err());
    assert!(FeedParser::parse(&vec![b'A'; FeedFetcher::MAX_BYTES / 20]).is_err());
}
