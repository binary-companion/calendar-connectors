//! Turns a source's cheap `check_for_changes()` into a `changes()` stream by polling. Port of
//! `ChangeMonitor.swift`.
//!
//! Swift's `Sleeper` typealias is `crate::time::Sleeper` (seconds); `defaultSleeper` (`Task.sleep`) is not ported,
//! because no crate except the engine calls `tokio::time` (conventions section 4).

use std::sync::Arc;

use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::failure::Failure;
use crate::source_types::{CalendarChange, PollingCalendarSource, SourceError};
use crate::time::Sleeper;

/// Turns a source's cheap `check_for_changes()` into a `changes()` stream by polling.
/// Sleeping goes through `sleep`, so tests drive it without real time. Use one subscriber per source
/// (each `changes` call starts its own poller).
#[derive(Clone)]
pub struct ChangeMonitor {
    /// Seconds between checks (Swift default 60).
    pub interval: f64,
    /// Seconds; the longest wait after repeated failures (Swift default 900).
    pub max_backoff: f64,
    sleep: Sleeper,
}

impl ChangeMonitor {
    /// Swift's `init(interval:maxBackoff:sleep:)`. There is no default sleeper (see the module comment), so `sleep`
    /// is required; [`ChangeMonitor::with_sleeper`] keeps the default intervals.
    pub fn new(interval: f64, max_backoff: f64, sleep: Sleeper) -> ChangeMonitor {
        ChangeMonitor {
            interval,
            max_backoff,
            sleep,
        }
    }

    /// The Swift defaults: 60 seconds between checks, at most 900 seconds of backoff.
    pub fn with_sleeper(sleep: Sleeper) -> ChangeMonitor {
        ChangeMonitor::new(60.0, 900.0, sleep)
    }

    /// Polling starts as soon as the stream is created, and stops when the consumer drops the receiver (Swift: when
    /// the consuming task is cancelled) or the stream finishes, e.g. on `AuthExpired`. Must be called inside a Tokio
    /// runtime; outside one the stream finishes at once.
    pub fn changes(
        &self,
        source: Arc<dyn PollingCalendarSource>,
    ) -> UnboundedReceiver<CalendarChange> {
        let (sender, receiver) = unbounded_channel();
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return receiver;
        };
        let interval = self.interval;
        let max_backoff = self.max_backoff;
        let sleep = Arc::clone(&self.sleep);
        runtime.spawn(async move {
            let mut failures: u32 = 0;
            loop {
                let checked = tokio::select! {
                    biased;
                    () = sender.closed() => break,
                    checked = source.check_for_changes() => checked,
                };
                let wait = match checked {
                    Ok(change) => {
                        if let Some(change) = change
                            && sender.send(change).is_err()
                        {
                            break;
                        }
                        failures = 0;
                        interval
                    }
                    Err(Failure::Cancelled) => break,
                    Err(Failure::Source(SourceError::AuthExpired)) => {
                        let _ = sender.send(CalendarChange::SourceFailed(SourceError::AuthExpired));
                        break;
                    }
                    Err(_) => {
                        failures = failures.saturating_add(1);
                        // Clamp the exponent before multiplying so a long outage cannot overflow.
                        let factor = f64::from(1u32 << failures.min(10));
                        max_backoff.min(interval * factor)
                    }
                };
                // A sleeper error is not a source failure: it just ends the monitor.
                let slept = tokio::select! {
                    biased;
                    () = sender.closed() => break,
                    slept = sleep(wait) => slept,
                };
                if slept.is_err() {
                    break;
                }
            }
        });
        receiver
    }
}
