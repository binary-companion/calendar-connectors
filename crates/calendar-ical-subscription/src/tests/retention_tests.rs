//! Port of `RetentionTests.swift`. TestNow starts at 2027-01-15 00:00 Pacific, so the window is Jan 14 00:00 to
//! Jan 22 00:00 Pacific.

use std::collections::BTreeSet;
use std::sync::Arc;

use calendar_core::connection::ConnectorKind;
use calendar_core::connection::{InMemoryCredentialStore, InMemorySyncStateStore};
use calendar_core::http::HttpResponse;
use calendar_core::source_types::{CalendarSource, PollingCalendarSource};
use calendar_core::time::DateInterval;
use calendar_test_support::fake_transport::{FakeTransport, TestNow};

use super::fixtures::{
    PRIVATE_PATH, StubInteraction, feed, feed_changed, feed_url, immediate_sleeper, ok, ok_with,
    pacific, pt, source_settings, status, vevent,
};
use crate::kind::ICalSubscriptionKind;
use crate::retention_window::RetentionWindow;
use crate::source::ICalSubscriptionSource;

fn window() -> RetentionWindow {
    RetentionWindow::new(1, 7)
}

fn everything() -> DateInterval {
    DateInterval::new(pt(2026, 8, 1, 0, 0), pt(2027, 12, 1, 0, 0))
}

fn dashed(title: &str) -> String {
    title.replace(' ', "-")
}

fn one_off(title: &str, start: &str, end: &str) -> Vec<String> {
    vevent(
        Some(&format!("{}@example.test", dashed(title))),
        title,
        start,
        end,
        &[],
    )
}

fn all_day(title: &str, first: &str, next: &str) -> Vec<String> {
    vec![
        "BEGIN:VEVENT".to_string(),
        format!("UID:{}@example.test", dashed(title)),
        "DTSTAMP:20260901T000000Z".to_string(),
        format!("SUMMARY:{title}"),
        format!("DTSTART;VALUE=DATE:{first}"),
        format!("DTEND;VALUE=DATE:{next}"),
        "END:VEVENT".to_string(),
    ]
}

fn far_past() -> Vec<String> {
    one_off("Far past", "20260901T100000", "20260901T110000")
}
fn far_future() -> Vec<String> {
    one_off("Far future", "20270301T100000", "20270301T110000")
}
fn in_window() -> Vec<String> {
    one_off("In window", "20270118T100000", "20270118T110000")
}
fn coming_up() -> Vec<String> {
    one_off("Coming up", "20270125T100000", "20270125T110000")
}
fn old_weekly() -> Vec<String> {
    vevent(
        Some("walk@example.test"),
        "Old weekly",
        "20260901T100000",
        "20260901T110000",
        &["RRULE:FREQ=WEEKLY;BYDAY=TU"],
    )
}

fn make_source(
    transport: &Arc<FakeTransport>,
    now: &TestNow,
    retention: Option<RetentionWindow>,
) -> Arc<ICalSubscriptionSource> {
    let mut settings = source_settings(transport.clone(), now.provider(), pacific());
    settings.retention = retention;
    ICalSubscriptionSource::new(settings)
}

fn serve(transport: &FakeTransport, responses: Vec<HttpResponse>) {
    transport.route(PRIVATE_PATH, responses);
}

fn ics(events: &[Vec<String>]) -> HttpResponse {
    ok(&feed(events))
}

fn ics_etag(events: &[Vec<String>], etag: &str) -> HttpResponse {
    ok_with(&feed(events), &[("ETag", etag)])
}

async fn titles(source: &Arc<ICalSubscriptionSource>) -> BTreeSet<String> {
    source
        .events_in(everything())
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.title)
        .collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|item| item.to_string()).collect()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_window_drops_far_one_offs_and_keeps_in_window_and_recurring_events() {
    let transport = Arc::new(FakeTransport::new());
    serve(
        &transport,
        vec![ics(&[far_past(), far_future(), in_window(), old_weekly()])],
    );
    let found = titles(&make_source(
        &transport,
        &TestNow::default(),
        Some(window()),
    ))
    .await;
    assert_eq!(found, set(&["In window", "Old weekly"]));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn all_day_events_at_the_edges_of_the_window() {
    let transport = Arc::new(FakeTransport::new());
    serve(
        &transport,
        vec![ics(&[
            all_day("Ends as the window starts", "20270113", "20270114"),
            all_day("Last day before the window", "20270114", "20270115"),
            all_day("Last day inside", "20270121", "20270122"),
            all_day("Starts as the window ends", "20270122", "20270123"),
        ])],
    );
    let found = titles(&make_source(
        &transport,
        &TestNow::default(),
        Some(window()),
    ))
    .await;
    assert_eq!(
        found,
        set(&["Last day before the window", "Last day inside"])
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn no_window_keeps_everything() {
    let transport = Arc::new(FakeTransport::new());
    serve(
        &transport,
        vec![ics(&[far_past(), far_future(), in_window()])],
    );
    let found = titles(&make_source(&transport, &TestNow::default(), None)).await;
    assert_eq!(found, set(&["Far past", "Far future", "In window"]));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_edit_outside_the_window_is_not_a_change_but_one_inside_is() {
    let edited_far = one_off("Far future renamed", "20270301T100000", "20270301T110000");
    let edited_in = one_off("In window renamed", "20270118T100000", "20270118T110000");
    let transport = Arc::new(FakeTransport::new());
    serve(
        &transport,
        vec![
            ics(&[far_future(), in_window()]),
            ics(&[edited_far.clone(), in_window()]),
            ics(&[edited_far.clone(), in_window()]),
            ics(&[edited_far, edited_in]),
        ],
    );
    let source = make_source(&transport, &TestNow::default(), Some(window()));
    assert_eq!(source.check_for_changes().await.unwrap(), None);
    assert_eq!(source.check_for_changes().await.unwrap(), None);
    assert_eq!(source.check_for_changes().await.unwrap(), None);
    assert_eq!(
        source.check_for_changes().await.unwrap(),
        Some(feed_changed())
    );
    assert_eq!(titles(&source).await, set(&["In window renamed"]));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_event_that_comes_into_the_window_as_time_passes_is_reported_and_readable() {
    let now = TestNow::default();
    let transport = Arc::new(FakeTransport::new());
    serve(&transport, vec![ics(&[coming_up(), in_window()])]);
    let source = make_source(&transport, &now, Some(window()));
    assert_eq!(source.check_for_changes().await.unwrap(), None);
    assert_eq!(titles(&source).await, set(&["In window"]));
    now.advance(5.0 * 86_400.0);
    assert_eq!(
        source.check_for_changes().await.unwrap(),
        Some(feed_changed())
    );
    assert!(titles(&source).await.contains("Coming up"));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_not_modified_answer_still_brings_an_event_into_the_window() {
    let now = TestNow::default();
    let transport = Arc::new(FakeTransport::new());
    serve(
        &transport,
        vec![ics_etag(&[coming_up()], "\"v1\""), status(304)],
    );
    let source = make_source(&transport, &now, Some(window()));
    assert_eq!(source.check_for_changes().await.unwrap(), None);
    now.advance(5.0 * 86_400.0);
    assert_eq!(
        source.check_for_changes().await.unwrap(),
        Some(feed_changed())
    );
    let second = transport.requests_matching(PRIVATE_PATH).pop().unwrap();
    assert_eq!(second.headers["If-None-Match"], "\"v1\"");
    assert!(titles(&source).await.contains("Coming up"));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_edit_outside_the_window_still_updates_what_is_kept_for_later() {
    // The edited far event comes into the window later; the 304 must re-read the edited version, not the first one.
    let now = TestNow::default();
    let edited = one_off("Coming up edited", "20270125T100000", "20270125T110000");
    let transport = Arc::new(FakeTransport::new());
    serve(
        &transport,
        vec![
            ics_etag(&[coming_up()], "\"v1\""),
            ics_etag(&[edited], "\"v2\""),
            status(304),
        ],
    );
    let source = make_source(&transport, &now, Some(window()));
    assert_eq!(source.check_for_changes().await.unwrap(), None);
    assert_eq!(source.check_for_changes().await.unwrap(), None);
    now.advance(5.0 * 86_400.0);
    assert_eq!(
        source.check_for_changes().await.unwrap(),
        Some(feed_changed())
    );
    assert_eq!(titles(&source).await, set(&["Coming up edited"]));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_feed_with_everything_outside_the_window_is_a_valid_sign_in_and_the_kind_passes_the_window_on()
 {
    let transport = Arc::new(FakeTransport::new());
    serve(&transport, vec![ics(&[far_past(), far_future()])]);
    let store = Arc::new(InMemoryCredentialStore::new());
    let kind = ICalSubscriptionKind::new(
        transport.clone(),
        TestNow::default().provider(),
        immediate_sleeper(),
        pacific(),
    )
    .with_retention(window());
    let connection = kind
        .authorize(&StubInteraction::link(feed_url().as_str()), store.as_ref())
        .await
        .unwrap();
    assert_eq!(connection.display_name, "My Meetups (www.example.test)");
    let source = kind
        .make_source(&connection, store, Arc::new(InMemorySyncStateStore::new()))
        .unwrap();
    assert!(source.events_in(everything()).await.unwrap().is_empty());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_feed_that_restamps_itself_on_every_request_is_not_a_change() {
    let stamped = |stamp: &str, modified: &str| {
        let event = vevent(
            Some("in@example.test"),
            "In window",
            "20270118T100000",
            "20270118T110000",
            &[
                &format!("LAST-MODIFIED:{modified}"),
                &format!("CREATED:{modified}"),
            ],
        );
        let text = feed(&[event])
            .replace("DTSTAMP:20260901T000000Z", &format!("DTSTAMP:{stamp}"))
            .replace(
                "PRODID:-//Example//Feed 1.0//EN",
                &format!("PRODID:-//Example//Feed {stamp}//EN"),
            );
        ok(&text)
    };
    let transport = Arc::new(FakeTransport::new());
    serve(
        &transport,
        vec![
            stamped("20261001T000000Z", "20260901T000000Z"),
            stamped("20261002T000000Z", "20260901T000000Z"),
            stamped("20261003T000000Z", "20261003T000000Z"),
        ],
    );
    let source = make_source(&transport, &TestNow::default(), Some(window()));
    assert_eq!(source.check_for_changes().await.unwrap(), None);
    assert_eq!(source.check_for_changes().await.unwrap(), None);
    assert_eq!(source.check_for_changes().await.unwrap(), None);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_real_edit_still_reports_when_only_the_stamps_also_moved() {
    let before = vevent(
        Some("in@example.test"),
        "In window",
        "20270118T100000",
        "20270118T110000",
        &[],
    );
    let after = vevent(
        Some("in@example.test"),
        "In window",
        "20270118T110000",
        "20270118T120000",
        &["LAST-MODIFIED:20261003T000000Z"],
    );
    let transport = Arc::new(FakeTransport::new());
    serve(&transport, vec![ics(&[before]), ics(&[after])]);
    let source = make_source(&transport, &TestNow::default(), Some(window()));
    assert_eq!(source.check_for_changes().await.unwrap(), None);
    assert_eq!(
        source.check_for_changes().await.unwrap(),
        Some(feed_changed())
    );
}

// MARK: recurring groups are kept only when an occurrence overlaps the window

fn series(title: &str, start: &str, rule: &str, extra: &[&str]) -> Vec<String> {
    let mut lines = vec![format!("RRULE:{rule}")];
    lines.extend(extra.iter().map(|line| line.to_string()));
    let extra: Vec<&str> = lines.iter().map(String::as_str).collect();
    vevent(
        Some(&format!("{}@example.test", dashed(title))),
        title,
        start,
        &format!("{}110000", &start[..9]),
        &extra,
    )
}

fn override_of(title: &str, slot: &str, start: &str, new_title: &str) -> Vec<String> {
    vevent(
        Some(&format!("{}@example.test", dashed(title))),
        new_title,
        start,
        &format!("{}110000", &start[..9]),
        &[&format!("RECURRENCE-ID;TZID=America/Los_Angeles:{slot}")],
    )
}

async fn read_titles(events: &[Vec<String>]) -> BTreeSet<String> {
    let transport = Arc::new(FakeTransport::new());
    serve(&transport, vec![ics(events)]);
    titles(&make_source(
        &transport,
        &TestNow::default(),
        Some(window()),
    ))
    .await
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_weekly_series_with_an_old_start_and_an_occurrence_in_the_window_is_kept() {
    assert_eq!(read_titles(&[old_weekly()]).await, set(&["Old weekly"]));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_series_that_ended_before_the_window_is_dropped() {
    let until_date = series(
        "Ended by date",
        "20260901T100000",
        "FREQ=WEEKLY;UNTIL=20261201T000000Z",
        &[],
    );
    let by_count = series(
        "Ended by count",
        "20260901T100000",
        "FREQ=WEEKLY;COUNT=3",
        &[],
    );
    assert_eq!(
        read_titles(&[until_date, by_count, in_window()]).await,
        set(&["In window"])
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_series_that_starts_after_the_window_is_dropped() {
    let later = series("Later series", "20270301T100000", "FREQ=WEEKLY", &[]);
    assert_eq!(
        read_titles(&[later, in_window()]).await,
        set(&["In window"])
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_unbounded_daily_series_from_many_years_ago_is_cheap() {
    // Expansion skips to the window, so a decade of daily instances is not walked.
    let transport = Arc::new(FakeTransport::new());
    serve(
        &transport,
        vec![ics(&[series(
            "Daily forever",
            "20170101T100000",
            "FREQ=DAILY",
            &[],
        )])],
    );
    let found = titles(&make_source(
        &transport,
        &TestNow::default(),
        Some(window()),
    ))
    .await;
    assert_eq!(found, set(&["Daily forever"]));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_series_whose_only_in_window_occurrence_is_an_override_moved_in_is_kept() {
    let mut master = series(
        "Moved in series",
        "20261001T100000",
        "FREQ=WEEKLY;COUNT=2",
        &[],
    );
    let moved = override_of(
        "Moved in series",
        "20261008T100000",
        "20270118T100000",
        "Moved in",
    );
    master.extend(moved);
    // The whole group is kept, so its out-of-window occurrence is readable too.
    assert_eq!(
        read_titles(&[master]).await,
        set(&["Moved in", "Moved in series"])
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_override_moved_out_of_the_window_drops_the_group_when_nothing_else_is_inside() {
    // Occurrences Jan 12 and Jan 19; the Jan 19 slot (the only one in the window) is moved to March.
    let mut master = series(
        "Moved out series",
        "20270112T100000",
        "FREQ=WEEKLY;COUNT=2",
        &[],
    );
    let moved = override_of(
        "Moved out series",
        "20270119T100000",
        "20270301T100000",
        "Moved out",
    );
    master.extend(moved);
    assert!(read_titles(&[master]).await.is_empty());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_exdate_that_removes_the_only_in_window_occurrence_drops_the_group() {
    let master = series(
        "Excluded series",
        "20270112T100000",
        "FREQ=WEEKLY;COUNT=2",
        &["EXDATE;TZID=America/Los_Angeles:20270119T100000"],
    );
    assert!(read_titles(&[master]).await.is_empty());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_edit_to_a_dropped_series_is_not_a_change() {
    let transport = Arc::new(FakeTransport::new());
    serve(
        &transport,
        vec![
            ics(&[
                series("Later series", "20270301T100000", "FREQ=WEEKLY", &[]),
                in_window(),
            ]),
            ics(&[
                series(
                    "Later series renamed",
                    "20270301T100000",
                    "FREQ=WEEKLY",
                    &[],
                ),
                in_window(),
            ]),
        ],
    );
    let source = make_source(&transport, &TestNow::default(), Some(window()));
    assert_eq!(source.check_for_changes().await.unwrap(), None);
    assert_eq!(source.check_for_changes().await.unwrap(), None);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_series_whose_first_occurrence_comes_into_the_window_is_picked_up() {
    let later = series("Starts soon", "20270125T100000", "FREQ=WEEKLY", &[]);
    for use_not_modified in [false, true] {
        let now = TestNow::default();
        let transport = Arc::new(FakeTransport::new());
        let second = if use_not_modified {
            status(304)
        } else {
            ics_etag(std::slice::from_ref(&later), "\"v1\"")
        };
        serve(
            &transport,
            vec![ics_etag(std::slice::from_ref(&later), "\"v1\""), second],
        );
        let source = make_source(&transport, &now, Some(window()));
        assert_eq!(source.check_for_changes().await.unwrap(), None);
        assert!(titles(&source).await.is_empty());
        now.advance(5.0 * 86_400.0);
        assert_eq!(
            source.check_for_changes().await.unwrap(),
            Some(feed_changed()),
            "304: {use_not_modified}"
        );
        assert_eq!(titles(&source).await, set(&["Starts soon"]));
    }
}
