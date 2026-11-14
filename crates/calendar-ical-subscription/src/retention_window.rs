//! Port of `RetentionWindow.swift`.

use calendar_core::time::{DateInterval, add_seconds};
use jiff::Timestamp;

/// How far from now a feed's events are kept. A UID group (one event, or a recurring series with its overrides) with no
/// occurrence overlapping `[now - days_back, now + days_ahead]` is dropped when the feed is read, and edits to it are not
/// reported as changes. A group with at least one occurrence in the window is kept whole. A day is 24 hours, so across a
/// daylight-saving change an edge can shift by an hour. Without a window (the library default) everything is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetentionWindow {
    /// Days before now that are kept. Never negative.
    pub days_back: i64,
    /// Days after now that are kept. Never negative.
    pub days_ahead: i64,
}

impl RetentionWindow {
    /// Negative values become 0, as in Swift.
    pub fn new(days_back: i64, days_ahead: i64) -> RetentionWindow {
        RetentionWindow {
            days_back: days_back.max(0),
            days_ahead: days_ahead.max(0),
        }
    }

    /// Swift `interval(at:)`.
    pub(crate) fn interval_at(&self, now: Timestamp) -> DateInterval {
        DateInterval::new(
            add_seconds(now, -(self.days_back as f64) * 86_400.0),
            add_seconds(now, (self.days_ahead as f64) * 86_400.0),
        )
    }
}
