//! Port of `CalendarTestSupport/CollectingDiagnosticLog.swift`. (`calendar-core` keeps a private copy for its own
//! tests, because this crate cannot be a dev-dependency of `calendar-core`.)

use std::sync::{Mutex, MutexGuard, PoisonError};

use calendar_core::diagnostic_log::{DiagnosticEvent, DiagnosticLog};

/// A `DiagnosticLog` for tests: keeps every event it is given. Swift's `record(_:)` requirement is `log` here.
#[derive(Default)]
pub struct CollectingDiagnosticLog {
    recorded: Mutex<Vec<DiagnosticEvent>>,
}

impl CollectingDiagnosticLog {
    pub fn new() -> CollectingDiagnosticLog {
        CollectingDiagnosticLog::default()
    }

    fn lock(&self) -> MutexGuard<'_, Vec<DiagnosticEvent>> {
        self.recorded.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Swift `events`: everything recorded, in order.
    pub fn events(&self) -> Vec<DiagnosticEvent> {
        self.lock().clone()
    }

    /// Swift `events(named:)`.
    pub fn events_named(&self, name: &str) -> Vec<DiagnosticEvent> {
        self.lock()
            .iter()
            .filter(|event| event.name == name)
            .cloned()
            .collect()
    }

    pub fn clear(&self) {
        self.lock().clear();
    }

    /// Every event as text, private fields included: what a leak test searches. One line per event:
    /// `category name level field=value ...`.
    pub fn transcript(&self) -> String {
        self.lock()
            .iter()
            .map(|event| {
                let mut parts = vec![
                    event.category.clone(),
                    event.name.clone(),
                    event.level.label().to_string(),
                ];
                parts.extend(
                    event
                        .fields
                        .iter()
                        .map(|field| format!("{}={}", field.name, field.unredacted_text())),
                );
                parts.join(" ")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl DiagnosticLog for CollectingDiagnosticLog {
    fn log(&self, event: DiagnosticEvent) {
        self.lock().push(event);
    }
}
