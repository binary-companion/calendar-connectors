//! Port of `SourceTests.swift`.

use std::sync::Arc;

use calendar_core::failure::Failure;
use calendar_core::model::{CalendarKind, CalendarProvider, CalendarService, UidScope};
use calendar_core::source_types::{
    CalendarChange, CalendarSource, PollingCalendarSource, SourceError, SyncKind,
};
use calendar_core::time::{Now, Zone};
use calendar_test_support::all_day_conformance::AllDayConformance;
use calendar_test_support::fake_transport::{FakeTransport, TestNow};
use calendar_test_support::provided_fields_conformance::ProvidedFieldsConformance;

use super::fixtures::{
    PRIVATE_PATH, board_games, feed, feed_changed, feed_ics, ok, ok_with, sample_feed, september,
    source_settings, status,
};
use crate::source::ICalSubscriptionSource;

fn berlin() -> Zone {
    Zone::named("Europe/Berlin").unwrap()
}

fn make_source(
    transport: &Arc<FakeTransport>,
    now: Now,
    max_age: f64,
) -> Arc<ICalSubscriptionSource> {
    let mut settings = source_settings(transport.clone(), now, berlin());
    settings.connection.display_name = "My Meetups (www.example.test)".to_string();
    settings.max_age = max_age;
    ICalSubscriptionSource::new(settings)
}

fn source_of(transport: &Arc<FakeTransport>) -> Arc<ICalSubscriptionSource> {
    make_source(transport, TestNow::default().provider(), 900.0)
}

fn serving(responses: Vec<calendar_core::http::HttpResponse>) -> Arc<FakeTransport> {
    let transport = FakeTransport::new();
    transport.route(PRIVATE_PATH, responses);
    Arc::new(transport)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_feed_is_one_read_only_subscribed_calendar() {
    let source = source_of(&serving(vec![ok(&sample_feed())]));
    let calendars = source.calendars().await.unwrap();
    let calendar = calendars.first().unwrap();
    assert!(calendars.len() == 1 && calendar.id == "feed" && calendar.title == "My Meetups");
    assert!(
        calendar.service == CalendarService::ical_subscription()
            && calendar.provider == Some(CalendarProvider::subscription())
            && calendar.kind == CalendarKind::Subscribed
    );
    assert!(!calendar.permissions.can_edit && calendar.permissions.can_view_details);
    assert_eq!(calendar.time_zone, Some(berlin()));
    assert_eq!(
        calendar.account_name.as_deref(),
        Some("My Meetups (www.example.test)")
    );
    let capabilities = source.capabilities();
    assert!(
        source.id() == "icalsub-c1"
            && !capabilities.can_write
            && capabilities.sync_kind == SyncKind::Token
    );
    assert!(ProvidedFieldsConformance::calendar_violations(calendar, &capabilities).is_empty());
    assert!(ProvidedFieldsConformance::source_violations(source.as_ref()).is_empty());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_calendar_name_falls_back_to_the_account_name() {
    let transport = serving(vec![ok(&feed_ics(&[board_games()], &[]))]);
    let calendar = source_of(&transport).calendars().await.unwrap().remove(0);
    assert_eq!(calendar.title, "My Meetups (www.example.test)");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn events_are_expanded_sorted_and_conform() {
    let source = source_of(&serving(vec![ok(&sample_feed())]));
    let events = source.events_in(september()).await.unwrap();
    assert!(events.len() == 6 && events.windows(2).all(|pair| pair[0].start <= pair[1].start));
    let titles: Vec<&str> = events.iter().take(3).map(|e| e.title.as_str()).collect();
    assert_eq!(titles, ["Weekly walk", "Weekly walk", "Board games night"]);
    assert!(events.iter().all(|e| e.calendar_id == "feed"
        && e.source_id.as_deref() == Some("icalsub-c1")
        && e.uid_scope == Some(UidScope::Global)));
    for event in &events {
        assert!(
            ProvidedFieldsConformance::event_violations(event, &source.capabilities()).is_empty(),
            "{}",
            event.title
        );
        assert!(
            AllDayConformance::violations(event).is_empty(),
            "{}",
            event.title
        );
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn floating_times_use_the_feeds_zone_else_the_default() {
    let floating: Vec<String> = [
        "BEGIN:VEVENT",
        "UID:float@example.test",
        "DTSTAMP:20260901T000000Z",
        "SUMMARY:Floating",
        "DTSTART:20260910T120000",
        "DTEND:20260910T130000",
        "END:VEVENT",
    ]
    .iter()
    .map(|line| line.to_string())
    .collect();
    let transport = serving(vec![ok(&feed_ics(
        std::slice::from_ref(&floating),
        &["X-WR-TIMEZONE:Asia/Tokyo"],
    ))]);
    let event = source_of(&transport)
        .events_in(september())
        .await
        .unwrap()
        .remove(0);
    let at = |zone: &str| {
        jiff::civil::date(2026, 9, 10)
            .at(12, 0, 0, 0)
            .to_zoned(Zone::named(zone).unwrap().as_jiff().clone())
            .unwrap()
            .timestamp()
    };
    assert_eq!(event.start, at("Asia/Tokyo"));

    let plain = serving(vec![ok(&feed_ics(&[floating], &[]))]);
    let fallback = source_of(&plain)
        .events_in(september())
        .await
        .unwrap()
        .remove(0);
    assert_eq!(fallback.start, at("Europe/Berlin"));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_parsed_feed_is_reused_until_it_is_older_than_the_maximum_age() {
    let transport = serving(vec![ok(&sample_feed())]);
    let now = TestNow::default();
    let source = make_source(&transport, now.provider(), 900.0);
    source.events_in(september()).await.unwrap();
    source.calendars().await.unwrap();
    source.events_in(september()).await.unwrap();
    assert_eq!(transport.requests_matching(PRIVATE_PATH).len(), 1);
    now.advance(901.0);
    source.events_in(september()).await.unwrap();
    assert_eq!(transport.requests_matching(PRIVATE_PATH).len(), 2);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_first_check_is_the_baseline_and_later_checks_report_only_real_changes() {
    let changed = feed(&[board_games()]);
    let transport = serving(vec![ok(&sample_feed()), ok(&sample_feed()), ok(&changed)]);
    let source = source_of(&transport);
    assert_eq!(source.check_for_changes().await.unwrap(), None);
    assert_eq!(source.check_for_changes().await.unwrap(), None);
    assert_eq!(
        source.check_for_changes().await.unwrap(),
        Some(feed_changed())
    );
    let titles: Vec<String> = source
        .events_in(september())
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.title)
        .collect();
    assert_eq!(titles, ["Board games night"]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_change_since_the_events_were_loaded_is_reported_by_the_first_check() {
    let transport = serving(vec![ok(&sample_feed()), ok(&feed(&[board_games()]))]);
    let source = source_of(&transport);
    assert_eq!(source.events_in(september()).await.unwrap().len(), 6);
    assert_eq!(
        source.check_for_changes().await.unwrap(),
        Some(feed_changed())
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn validators_let_an_unchanged_feed_answer_304() {
    let transport = serving(vec![
        ok_with(&sample_feed(), &[("ETag", "\"v1\"")]),
        status(304),
    ]);
    let source = source_of(&transport);
    assert_eq!(source.check_for_changes().await.unwrap(), None);
    assert_eq!(source.check_for_changes().await.unwrap(), None);
    let second = transport.requests_matching(PRIVATE_PATH).pop().unwrap();
    assert_eq!(second.headers["If-None-Match"], "\"v1\"");
    assert_eq!(source.events_in(september()).await.unwrap().len(), 6);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_revoked_link_is_auth_expired_everywhere() {
    let source = source_of(&serving(vec![status(404)]));
    let expired = Failure::Source(SourceError::AuthExpired);
    assert_eq!(
        source.events_in(september()).await.err(),
        Some(expired.clone())
    );
    assert_eq!(source.calendars().await.err(), Some(expired.clone()));
    assert_eq!(source.check_for_changes().await.err(), Some(expired));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_page_that_stops_being_a_calendar_keeps_the_last_good_feed() {
    let transport = serving(vec![ok(&sample_feed()), ok("<html>Please sign in</html>")]);
    let source = source_of(&transport);
    assert_eq!(source.check_for_changes().await.unwrap(), None);
    assert_eq!(
        source.check_for_changes().await.err(),
        Some(Failure::Source(SourceError::InvalidResponse(
            "that link did not return a calendar".to_string()
        )))
    );
    assert_eq!(source.events_in(september()).await.unwrap().len(), 6);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_changes_stream_reports_a_change_and_stops_on_auth_expired() {
    let transport = serving(vec![
        ok(&sample_feed()),
        ok(&feed(&[board_games()])),
        status(404),
    ]);
    let settings = source_settings(transport.clone(), TestNow::default().provider(), berlin());
    let source = ICalSubscriptionSource::new(settings);
    let mut stream = source.changes();
    assert_eq!(stream.recv().await, Some(feed_changed()));
    assert_eq!(
        stream.recv().await,
        Some(CalendarChange::SourceFailed(SourceError::AuthExpired))
    );
    assert_eq!(stream.recv().await, None);
}
