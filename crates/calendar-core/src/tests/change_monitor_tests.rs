use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::change_monitor::ChangeMonitor;
use crate::failure::{Failure, Outcome};
use crate::model::{CalendarDescriptor, CalendarEvent};
use crate::source_types::{
    CalendarChange, CalendarSource, PollingCalendarSource, SourceCapabilities, SourceError,
    SyncKind,
};
use crate::time::{DateInterval, Sleeper};

/// The scripted results; an exhausted script throws `CancellationError`, as in Swift.
struct Script {
    results: Mutex<VecDeque<Outcome<Option<CalendarChange>>>>,
    calls: AtomicUsize,
}

impl Script {
    fn new(results: Vec<Outcome<Option<CalendarChange>>>) -> Arc<Script> {
        Arc::new(Script {
            results: Mutex::new(results.into()),
            calls: AtomicUsize::new(0),
        })
    }
    fn next(&self) -> Outcome<Option<CalendarChange>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(Failure::Cancelled))
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

struct ScriptedSource {
    script: Arc<Script>,
}

#[async_trait]
impl CalendarSource for ScriptedSource {
    fn id(&self) -> String {
        "scripted".to_string()
    }
    fn display_name(&self) -> String {
        "Scripted".to_string()
    }
    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities {
            sync_kind: SyncKind::Token,
            ..SourceCapabilities::default()
        }
    }
    async fn calendars(&self) -> Outcome<Vec<CalendarDescriptor>> {
        Ok(Vec::new())
    }
    async fn events_in(&self, _interval: DateInterval) -> Outcome<Vec<CalendarEvent>> {
        Ok(Vec::new())
    }
    fn changes(&self) -> UnboundedReceiver<CalendarChange> {
        let (_sender, receiver) = unbounded_channel();
        receiver
    }
}

#[async_trait]
impl PollingCalendarSource for ScriptedSource {
    async fn check_for_changes(&self) -> Outcome<Option<CalendarChange>> {
        self.script.next()
    }
}

fn source(script: &Arc<Script>) -> Arc<dyn PollingCalendarSource> {
    Arc::new(ScriptedSource {
        script: Arc::clone(script),
    })
}

type SleepLog = Arc<Mutex<Vec<f64>>>;

fn recording_sleeper(log: &SleepLog) -> Sleeper {
    let log = Arc::clone(log);
    Arc::new(move |seconds| {
        log.lock().unwrap().push(seconds);
        Box::pin(async { Ok(()) })
    })
}

fn standard(sleeper: Sleeper) -> ChangeMonitor {
    ChangeMonitor::new(60.0, 900.0, sleeper)
}

async fn run(results: Vec<Outcome<Option<CalendarChange>>>) -> (Vec<CalendarChange>, Vec<f64>) {
    let log: SleepLog = Arc::default();
    let monitor = standard(recording_sleeper(&log));
    let script = Script::new(results);
    let mut stream = monitor.changes(source(&script));
    let mut changes = Vec::new();
    while let Some(change) = stream.recv().await {
        changes.push(change);
    }
    let sleeps = log.lock().unwrap().clone();
    (changes, sleeps)
}

fn events_changed(ids: &[&str]) -> CalendarChange {
    CalendarChange::EventsChanged {
        calendar_ids: Some(ids.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>()),
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn yields_changes_and_sleeps_the_interval_between_checks() {
    let result = run(vec![
        Ok(None),
        Ok(Some(events_changed(&["a"]))),
        Ok(Some(CalendarChange::CalendarsChanged)),
    ])
    .await;
    assert_eq!(
        result.0,
        vec![events_changed(&["a"]), CalendarChange::CalendarsChanged]
    );
    assert_eq!(result.1, vec![60.0, 60.0, 60.0]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn backs_off_exponentially_then_resets_on_success() {
    let boom = || Err(Failure::Source(SourceError::Network("down".to_string())));
    let result = run(vec![boom(), boom(), Ok(None), Ok(None)]).await;
    assert_eq!(result.1, vec![120.0, 240.0, 60.0, 60.0]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn backoff_is_capped_and_never_overflows() {
    let boom = || Err(Failure::Source(SourceError::Server { status: 503 }));
    let result = run((0..5000).map(|_| boom()).collect()).await;
    assert_eq!(result.1.len(), 5000);
    assert_eq!(result.1.iter().copied().fold(f64::MIN, f64::max), 900.0);
    assert_eq!(result.1.last().copied(), Some(900.0));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn auth_expired_yields_source_failed_and_finishes_without_sleeping() {
    let result = run(vec![Err(Failure::Source(SourceError::AuthExpired))]).await;
    assert_eq!(
        result.0,
        vec![CalendarChange::SourceFailed(SourceError::AuthExpired)]
    );
    assert!(result.1.is_empty());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn cancelling_the_consumer_stops_polling() {
    let script = Script::new((0..1_000_000).map(|_| Ok(None)).collect());
    // No real time: the sleeper only yields, so the poller interleaves with this test task.
    let sleeper: Sleeper = Arc::new(|_| {
        Box::pin(async {
            tokio::task::yield_now().await;
            Ok(())
        })
    });
    let monitor = ChangeMonitor::new(0.001, 1.0, sleeper);
    let stream = monitor.changes(source(&script));
    for _ in 0..30 {
        tokio::task::yield_now().await;
    }
    assert!(script.calls() > 0);
    // Dropping the receiver is the cancellation.
    drop(stream);
    let calls_at_stop = script.calls();
    for _ in 0..30 {
        tokio::task::yield_now().await;
    }
    assert!(script.calls() <= calls_at_stop + 1);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn sleeper_error_ends_the_monitor_without_counting_as_a_failure() {
    // Only the first sleep throws; if the error were treated as a polling failure the loop would retry.
    let script = Script::new(vec![Ok(None), Ok(None), Ok(None)]);
    let thrown = Arc::new(Mutex::new(false));
    let sleeper: Sleeper = Arc::new(move |_| {
        let mut thrown = thrown.lock().unwrap();
        let result = if *thrown {
            Ok(())
        } else {
            *thrown = true;
            Err(Failure::Host("sleeper boom".to_string()))
        };
        Box::pin(async move { result })
    });
    let monitor = ChangeMonitor::new(60.0, 900.0, sleeper);
    let mut stream = monitor.changes(source(&script));
    let mut changes = Vec::new();
    while let Some(change) = stream.recv().await {
        changes.push(change);
    }
    assert_eq!(script.calls(), 1);
    assert!(changes.is_empty());
}

#[test]
fn the_defaults_are_a_minute_and_fifteen_minutes() {
    let log: SleepLog = Arc::default();
    let monitor = ChangeMonitor::with_sleeper(recording_sleeper(&log));
    assert_eq!((monitor.interval, monitor.max_backoff), (60.0, 900.0));
}
