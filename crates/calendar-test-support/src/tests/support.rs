//! Helpers shared by the ported tests (`TestHelpers.swift`). `CalendarCoreTests` that use `CalendarTestSupport` live
//! in this crate rather than in `calendar-core`: a dev-dependency from `calendar-core` on this crate would link two
//! copies of `calendar-core`, and the fakes' types would not be the ones the tests construct.

use std::fmt::Debug;

use calendar_core::Failure;
use calendar_core::time::Zone;
use calendar_core::write::types::WriteError;
use jiff::Timestamp;

/// `ISO8601DateFormatter().date(from:)` for the Z-suffixed form the Swift tests use.
pub(super) fn iso(text: &str) -> Timestamp {
    text.parse().unwrap()
}

/// `Date(timeIntervalSince1970:)` for whole seconds.
pub(super) fn at(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).unwrap()
}

/// `TimeZone(identifier:)!`.
pub(super) fn zone(identifier: &str) -> Zone {
    Zone::named(identifier).unwrap()
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
