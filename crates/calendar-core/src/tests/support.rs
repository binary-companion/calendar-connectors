//! Helpers shared by the ported tests. `CollectingDiagnosticLog` is the part of `CalendarTestSupport` that
//! `DiagnosticLogTests` needs; `calendar-test-support` cannot be a dev-dependency of this crate (its types would be
//! a second copy of this crate's), so the in-crate tests carry their own copy and Task 6 ports the public one.

use std::sync::Mutex;

use jiff::Timestamp;

use std::collections::BTreeSet;
use std::fmt::Debug;

use crate::diagnostic_log::{DiagnosticEvent, DiagnosticLog};
use crate::failure::Failure;
use crate::model::Attendee;
use crate::time::Zone;
use crate::write::types::{EventField, WriteError};

/// `ISO8601DateFormatter().date(from:)` for the Z-suffixed form the Swift tests use.
pub(super) fn iso(text: &str) -> Timestamp {
    text.parse().unwrap()
}

/// `TimeZone(identifier:)!`.
pub(super) fn zone(identifier: &str) -> Zone {
    Zone::named(identifier).unwrap()
}

/// `Date(timeIntervalSince1970:)` for whole seconds.
pub(super) fn at(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).unwrap()
}

/// A `DiagnosticLog` for tests: keeps every event it is given.
#[derive(Default)]
pub(super) struct CollectingDiagnosticLog {
    recorded: Mutex<Vec<DiagnosticEvent>>,
}

impl DiagnosticLog for CollectingDiagnosticLog {
    fn log(&self, event: DiagnosticEvent) {
        self.recorded.lock().unwrap().push(event);
    }
}

impl CollectingDiagnosticLog {
    pub(super) fn events(&self) -> Vec<DiagnosticEvent> {
        self.recorded.lock().unwrap().clone()
    }
}

/// `TimeZone(identifier:)` as an `Option`.
pub(super) fn zone_or_none(identifier: &str) -> Option<Zone> {
    Zone::named(identifier)
}

/// A directory under the system temp dir, removed when dropped. Swift's tests make `temporaryDirectory/<UUID>/...`;
/// the process id plus a counter keep concurrent tests apart.
pub(super) struct TempDir(std::path::PathBuf);

impl TempDir {
    pub(super) fn new(tag: &str) -> TempDir {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut path = std::env::temp_dir();
        path.push(format!("calendar-core-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    pub(super) fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A `[String: String]` literal.
pub(super) fn secrets(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// `TestHelpers.swift`'s `expectWriteError`: fails unless `result` is exactly `Err(expected)`.
pub(super) fn expect_write_error<T: Debug, E: Into<Failure>>(
    expected: WriteError,
    result: Result<T, E>,
) {
    match result {
        Ok(value) => panic!("expected {expected:?}, but nothing was thrown (got {value:?})"),
        Err(error) => assert_eq!(error.into(), Failure::Write(expected)),
    }
}

/// A set of event fields.
pub(super) fn fields(list: &[EventField]) -> BTreeSet<EventField> {
    list.iter().copied().collect()
}

/// `Attendee(email:)`: a required attendee who has not answered.
pub(super) fn attendee(email: &str) -> Attendee {
    Attendee::new(
        None,
        Some(email),
        crate::model::AttendeeRole::Required,
        crate::model::ResponseStatus::NeedsAction,
        false,
        false,
    )
}

/// `Attendee(name:email:)`: a required attendee who has not answered.
pub(super) fn named_attendee(name: &str, email: Option<&str>) -> Attendee {
    Attendee::new(
        Some(name.to_string()),
        email,
        crate::model::AttendeeRole::Required,
        crate::model::ResponseStatus::NeedsAction,
        false,
        false,
    )
}

/// `EventPatch(...)` with the arguments given by `set`: a hand-built patch (no base). `EventPatch` keeps its `base`
/// private, so struct update syntax is not available outside its module.
pub(super) fn hand_patch(
    set: impl FnOnce(&mut crate::write::event_patch::EventPatch),
) -> crate::write::event_patch::EventPatch {
    let mut patch = crate::write::event_patch::EventPatch::default();
    set(&mut patch);
    patch
}
