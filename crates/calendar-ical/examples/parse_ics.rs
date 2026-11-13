//! Parses iCalendar text into a component tree, reads two properties and writes the text back.
//!
//! Run it with `cargo run -p calendar-ical --example parse_ics`.

#![allow(clippy::print_stdout, clippy::expect_used)]

use calendar_ical::content_line::{ICalParser, ICalSerializer};

/// Runs the example (public so that the crate's test can call it).
pub fn main() {
    let text = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Example//Docs//EN\r\n\
                BEGIN:VEVENT\r\nUID:docs-1@example.test\r\nDTSTAMP:20260901T000000Z\r\n\
                DTSTART:20260910T190000Z\r\nDTEND:20260910T210000Z\r\nSUMMARY:Board games night\r\n\
                END:VEVENT\r\nEND:VCALENDAR\r\n";

    // A parser returns an error for malformed text, never a panic.
    let calendar = ICalParser::parse_text(text).expect("well-formed iCalendar");
    let events = calendar.components_named("VEVENT");
    assert_eq!(events.len(), 1);
    let summary = events[0].property("SUMMARY").map(|p| p.text());
    assert_eq!(summary.as_deref(), Some("Board games night"));
    println!("{}", summary.unwrap_or_default());

    let round_trip = ICalSerializer::serialize(&calendar);
    assert!(round_trip.contains("SUMMARY:Board games night"));

    let broken = ICalParser::parse_text("BEGIN:VCALENDAR\r\nthis line has no colon\r\n");
    assert!(broken.is_err());
}
