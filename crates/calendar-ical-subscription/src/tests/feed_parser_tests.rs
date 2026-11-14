//! Port of `FeedParserTests.swift`.

use std::collections::BTreeSet;

use calendar_core::model::CalendarEvent;
use calendar_core::source_types::SourceError;
use calendar_core::time::{DateInterval, Zone};
use calendar_core::url_string::UrlString;
use calendar_ical::{EventReadContext, EventReader};
use calendar_test_support::all_day_conformance::AllDayConformance;

use super::fixtures::{
    board_games, feed, feed_ics, pacific, pt, sample_feed, september, vevent, weekly_walk,
};
use crate::feed_parser::{FeedParser, ParsedFeed};

fn events(feed: &ParsedFeed, window: &DateInterval) -> Vec<CalendarEvent> {
    let mut all: Vec<CalendarEvent> = feed
        .resources
        .iter()
        .flat_map(|item| {
            EventReader::events_in(
                &item.resource,
                window,
                &EventReadContext::new(
                    "feed",
                    item.name.clone(),
                    None,
                    Some("icalsub-c1".to_string()),
                    pacific(),
                    BTreeSet::new(),
                ),
            )
        })
        .collect();
    all.sort_by_cached_key(|event| (event.start, event.id()));
    all
}

#[test]
fn groups_events_by_uid_and_names_the_calendar() {
    let parsed = FeedParser::parse(sample_feed().as_bytes()).unwrap();
    assert_eq!(parsed.name.as_deref(), Some("My Meetups"));
    let names: Vec<&str> = parsed.resources.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        ["event_board@example.test", "event_walk@example.test"]
    );
    let all = events(&parsed, &september());
    assert_eq!(all.len(), 6);
    let titles: Vec<&str> = all.iter().take(3).map(|e| e.title.as_str()).collect();
    assert_eq!(titles, ["Weekly walk", "Weekly walk", "Board games night"]);
    assert_eq!(all[2].location.as_deref(), Some("Cafe"));
    assert_eq!(
        all[2].url,
        UrlString::parse("https://www.example.test/events/1/")
    );
}

#[test]
fn overrides_stay_with_their_series() {
    let moved = vevent(
        Some("event_walk@example.test"),
        "Weekly walk (moved)",
        "20260916T140000",
        "20260916T150000",
        &["RECURRENCE-ID;TZID=America/Los_Angeles:20260915T100000"],
    );
    let parsed = FeedParser::parse(feed(&[weekly_walk(), moved]).as_bytes()).unwrap();
    assert_eq!(parsed.resources.len(), 1);
    let all = events(&parsed, &september());
    assert_eq!(all.len(), 5);
    assert!(
        all.iter()
            .any(|e| e.title == "Weekly walk (moved)" && e.start == pt(2026, 9, 16, 14, 0))
    );
    assert!(!all.iter().any(|e| e.start == pt(2026, 9, 15, 10, 0)));
}

#[test]
fn the_feed_carries_its_own_time_zone_and_colour() {
    let header = [
        "X-WR-CALNAME:Walks",
        "X-WR-TIMEZONE:Europe/Berlin",
        "X-APPLE-CALENDAR-COLOR:#12ab34ff",
    ];
    let parsed = FeedParser::parse(feed_ics(&[board_games()], &header).as_bytes()).unwrap();
    assert_eq!(parsed.name.as_deref(), Some("Walks"));
    assert_eq!(parsed.time_zone, Zone::named("Europe/Berlin"));
    assert_eq!(parsed.color_hex.as_deref(), Some("#12ab34"));
}

#[test]
fn a_feed_with_no_events_is_valid() {
    let parsed = FeedParser::parse(feed(&[]).as_bytes()).unwrap();
    assert!(parsed.resources.is_empty());
    assert_eq!(parsed.name.as_deref(), Some("My Meetups"));
}

#[test]
fn a_page_that_is_not_a_calendar_is_refused() {
    for body in [
        "<html><body>Please sign in</body></html>",
        "",
        "BEGIN:VEVENT\r\nEND:VEVENT\r\n",
        "not a calendar at all",
    ] {
        assert_eq!(
            FeedParser::parse(body.as_bytes()).err(),
            Some(SourceError::InvalidResponse(
                "that link did not return a calendar".to_string()
            )),
            "{}",
            body.chars().take(20).collect::<String>()
        );
    }
}

#[test]
fn events_without_a_uid_get_a_stable_synthetic_one() {
    let bare = vevent(None, "No uid", "20260912T100000", "20260912T110000", &[]);
    let first = FeedParser::parse(feed(&[bare.clone(), board_games()]).as_bytes()).unwrap();
    let second = FeedParser::parse(feed(&[bare, board_games()]).as_bytes()).unwrap();
    let names = |parsed: &ParsedFeed| -> Vec<String> {
        parsed.resources.iter().map(|r| r.name.clone()).collect()
    };
    assert_eq!(names(&first), names(&second));
    assert!(first.resources[0].name.starts_with("feed-") && first.resources.len() == 2);
    assert!(
        events(&first, &september())
            .iter()
            .any(|e| e.title == "No uid")
    );
}

#[test]
fn resource_names_never_contain_the_separator_used_in_event_ids() {
    let odd = vevent(
        Some("a b#c/d@example.test"),
        "Odd",
        "20260912T100000",
        "20260912T110000",
        &["RRULE:FREQ=WEEKLY"],
    );
    let parsed = FeedParser::parse(feed(&[odd]).as_bytes()).unwrap();
    let name = parsed.resources.first().unwrap().name.clone();
    assert!(!name.contains('#') && !name.contains('/') && !name.contains(' '));
    let all = events(&parsed, &september());
    assert!(!all.is_empty());
    assert!(
        all.iter()
            .all(|e| e.event_id.starts_with(&format!("{name}#")))
    );
}

#[test]
fn all_day_events_and_fields_conform() {
    let all_day: Vec<String> = [
        "BEGIN:VEVENT",
        "UID:allday@example.test",
        "DTSTAMP:20260901T000000Z",
        "SUMMARY:Festival",
        "DTSTART;VALUE=DATE:20260920",
        "DTEND;VALUE=DATE:20260922",
        "END:VEVENT",
    ]
    .iter()
    .map(|line| line.to_string())
    .collect();
    let parsed =
        FeedParser::parse(feed(&[all_day, board_games(), weekly_walk()]).as_bytes()).unwrap();
    let all = events(&parsed, &september());
    let festival = all.iter().find(|e| e.title == "Festival").unwrap();
    assert!(festival.is_all_day);
    for event in &all {
        assert!(
            AllDayConformance::violations(event).is_empty(),
            "{}",
            event.title
        );
    }
}
