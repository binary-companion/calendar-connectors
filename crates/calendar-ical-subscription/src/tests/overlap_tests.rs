//! Port of `OverlapTests.swift`: overlapping fetches finishing in any order.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use calendar_core::failure::Outcome;
use calendar_core::http::{HttpRequest, HttpResponse, HttpTransport};
use calendar_core::source_types::{CalendarSource, PollingCalendarSource};
use calendar_core::time::{DateInterval, Zone};
use calendar_test_support::fake_transport::TestNow;
use tokio::task::{JoinHandle, yield_now};

use super::fixtures::{
    board_games, feed, feed_changed, ok, ok_with, sample_feed, september, source_settings, status,
    vevent,
};
use crate::retention_window::RetentionWindow;
use crate::source::ICalSubscriptionSource;

/// Answers each request, in the order they arrive, with the next response, but only once the test releases it. That
/// lets a test finish overlapping fetches in any order. (Swift parks the request on a continuation; the Rust gate
/// polls with `yield_now`, which a current-thread test runtime schedules fairly.)
struct GatedTransport {
    responses: Vec<HttpResponse>,
    state: Mutex<GateState>,
}

#[derive(Default)]
struct GateState {
    arrivals: usize,
    released: BTreeSet<usize>,
    requests: Vec<HttpRequest>,
}

impl GatedTransport {
    fn new(responses: Vec<HttpResponse>) -> Arc<GatedTransport> {
        Arc::new(GatedTransport {
            responses,
            state: Mutex::new(GateState::default()),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, GateState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn release(&self, index: usize) {
        self.lock().released.insert(index);
    }

    async fn wait_for_arrivals(&self, count: usize) {
        while self.lock().arrivals < count {
            yield_now().await;
        }
    }

    fn arrivals(&self) -> usize {
        self.lock().arrivals
    }

    fn requests(&self) -> Vec<HttpRequest> {
        self.lock().requests.clone()
    }
}

#[async_trait]
impl HttpTransport for GatedTransport {
    async fn send(&self, request: HttpRequest) -> Outcome<HttpResponse> {
        let index = {
            let mut state = self.lock();
            let index = state.arrivals;
            state.arrivals += 1;
            state.requests.push(request);
            index
        };
        let response = self.responses[index.min(self.responses.len() - 1)].clone();
        while !self.lock().released.contains(&index) {
            yield_now().await;
        }
        Ok(response)
    }
}

fn make_source(
    transport: &Arc<GatedTransport>,
    now: &TestNow,
    retention: Option<RetentionWindow>,
) -> Arc<ICalSubscriptionSource> {
    let mut settings = source_settings(
        transport.clone(),
        now.provider(),
        Zone::named("Europe/Berlin").unwrap(),
    );
    settings.connection.display_name = "My Meetups (www.example.test)".to_string();
    settings.retention = retention;
    ICalSubscriptionSource::new(settings)
}

type Check = JoinHandle<Outcome<Option<calendar_core::source_types::CalendarChange>>>;

fn check(source: &Arc<ICalSubscriptionSource>) -> Check {
    let source = source.clone();
    tokio::spawn(async move { source.check_for_changes().await })
}

async fn titles(source: &Arc<ICalSubscriptionSource>) -> Vec<String> {
    source
        .events_in(september())
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.title)
        .collect()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn overlapping_checks_of_the_same_change_report_it_once() {
    let old = ok(&sample_feed());
    let new = ok(&feed(&[board_games()]));
    let transport = GatedTransport::new(vec![old, new.clone(), new]);
    let source = make_source(&transport, &TestNow::default(), None);
    let baseline = check(&source);
    transport.wait_for_arrivals(1).await;
    transport.release(0);
    assert_eq!(baseline.await.unwrap().unwrap(), None);

    let first = check(&source);
    transport.wait_for_arrivals(2).await;
    let second = check(&source);
    transport.wait_for_arrivals(3).await;
    transport.release(2);
    transport.release(1);
    let results = [
        first.await.unwrap().unwrap(),
        second.await.unwrap().unwrap(),
    ];
    let reported: Vec<_> = results.into_iter().flatten().collect();
    assert_eq!(reported, vec![feed_changed()]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_older_fetch_that_finishes_last_does_not_replace_the_newer_feed() {
    let old = ok(&sample_feed());
    let new = ok(&feed(&[board_games()]));
    let transport = GatedTransport::new(vec![old, new]);
    let source = make_source(&transport, &TestNow::default(), None);
    let slow = {
        let source = source.clone();
        tokio::spawn(async move { source.events_in(september()).await })
    };
    transport.wait_for_arrivals(1).await;
    let fast = check(&source);
    transport.wait_for_arrivals(2).await;
    transport.release(1);
    fast.await.unwrap().unwrap();
    transport.release(0);
    slow.await.unwrap().unwrap();
    assert_eq!(titles(&source).await, ["Board games night"]);
    assert_eq!(transport.arrivals(), 2);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_change_seen_only_by_an_older_fetch_is_still_reported_once_without_going_backwards() {
    let old = ok(&sample_feed());
    let new = ok(&feed(&[board_games()]));
    // Check B starts after check A but the server answers it with the old feed; A's answer is the new one.
    let transport = GatedTransport::new(vec![old.clone(), new.clone(), old, new]);
    let source = make_source(&transport, &TestNow::default(), None);
    let baseline = check(&source);
    transport.wait_for_arrivals(1).await;
    transport.release(0);
    assert_eq!(baseline.await.unwrap().unwrap(), None);

    let older = check(&source);
    transport.wait_for_arrivals(2).await;
    let newer = check(&source);
    transport.wait_for_arrivals(3).await;
    transport.release(2);
    assert_eq!(newer.await.unwrap().unwrap(), None);
    transport.release(1);
    assert_eq!(older.await.unwrap().unwrap(), Some(feed_changed()));
    assert_eq!(titles(&source).await, ["Board games night"]);

    transport.release(3);
    assert_eq!(source.check_for_changes().await.unwrap(), None);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_older_fetch_that_finds_nothing_new_does_not_overwrite_newer_validators() {
    let versioned = |text: &str, etag: &str| ok_with(text, &[("ETag", etag)]);
    let first = versioned(&sample_feed(), "\"e1\"");
    let second = versioned(&feed(&[board_games()]), "\"e2\"");
    let transport = GatedTransport::new(vec![first.clone(), first, second.clone(), second]);
    let source = make_source(&transport, &TestNow::default(), None);
    let baseline = check(&source);
    transport.wait_for_arrivals(1).await;
    transport.release(0);
    baseline.await.unwrap().unwrap();

    let slow = check(&source);
    transport.wait_for_arrivals(2).await;
    let fast = check(&source);
    transport.wait_for_arrivals(3).await;
    transport.release(2);
    fast.await.unwrap().unwrap();
    transport.release(1);
    slow.await.unwrap().unwrap();

    transport.release(3);
    source.check_for_changes().await.unwrap();
    let last = transport.requests().pop().unwrap();
    assert_eq!(
        last.headers.get("If-None-Match").map(String::as_str),
        Some("\"e2\"")
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_older_fetch_finishing_late_does_not_put_an_older_body_back_after_a_newer_unchanged_one()
{
    let far = |title: &str, etag: Option<&str>| {
        let event = vevent(
            Some("far@example.test"),
            title,
            "20270125T100000",
            "20270125T110000",
            &[],
        );
        let text = feed(&[event]);
        match etag {
            Some(etag) => ok_with(&text, &[("ETag", etag)]),
            None => ok(&text),
        }
    };
    // The window (1 day back, 7 ahead of the start) leaves the far event out, so every edit to it is "unchanged".
    let now = TestNow::default();
    let transport = GatedTransport::new(vec![
        far("Far v0", Some("\"e0\"")),
        far("Far v1", None),
        far("Far v2", None),
        status(304),
    ]);
    let source = make_source(&transport, &now, Some(RetentionWindow::new(1, 7)));
    let baseline = check(&source);
    transport.wait_for_arrivals(1).await;
    transport.release(0);
    baseline.await.unwrap().unwrap();

    let older = check(&source);
    transport.wait_for_arrivals(2).await;
    let newer = check(&source);
    transport.wait_for_arrivals(3).await;
    transport.release(2);
    newer.await.unwrap().unwrap();
    transport.release(1);
    older.await.unwrap().unwrap();

    now.advance(5.0 * 86_400.0);
    transport.release(3);
    assert_eq!(
        source.check_for_changes().await.unwrap(),
        Some(feed_changed())
    );
    let window = DateInterval::new(
        now.date(),
        calendar_core::time::add_seconds(now.date(), 30.0 * 86_400.0),
    );
    let found: Vec<String> = source
        .events_in(window)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.title)
        .collect();
    assert_eq!(found, ["Far v2"]);
}
