//! The occurrences of a series that overlap a window. Port of `RecurrenceSet+Occurrences.swift`.

use std::collections::BTreeSet;

use jiff::Timestamp;

use super::is_swift_whitespace;
use super::set::RecurrenceSet;
use crate::all_day::{AllDay, CalendarDate};
use crate::recurrence::rule::RuleEnd;
use crate::time::{DateInterval, Zone, add_seconds};

/// The occurrences of a series that overlap a window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecurrenceExpansion {
    /// Original starts (the instants the rules put the occurrences at), sorted and without duplicates.
    pub starts: Vec<Timestamp>,
    /// True when the limit stopped the expansion early.
    pub truncated: bool,
    /// True when an `RRULE` line could not be read (it sits in `unparsed`): the starts then hold only the anchor, and a
    /// caller should show just the first occurrence and the series' exceptions.
    pub has_unreadable_rule: bool,
}

/// What identifies an occurrence when comparing starts: the calendar day for an all-day series, else the instant.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Day(CalendarDate),
    Instant(Timestamp),
}

impl RecurrenceSet {
    /// Swift's default `limit` of `occurrences(...)`.
    pub const DEFAULT_OCCURRENCE_LIMIT: i64 = 5000;

    /// True when an `RRULE` line could not be read (it sits in `unparsed`).
    pub fn has_unreadable_rule(&self) -> bool {
        self.unparsed.iter().any(|line| {
            line.trim_matches(is_swift_whitespace)
                .to_uppercase()
                .starts_with("RRULE")
        })
    }

    /// Every occurrence whose span (`start ..< start + duration`) overlaps `window`: the rules' instances from `anchor`
    /// plus `extra_dates`, minus `excluded_dates` (all-day series compare calendar days in `time_zone`). At most `limit`
    /// starts. Expansion skips whole periods before the window, so an old series stays cheap.
    pub fn occurrences(
        &self,
        anchor: Timestamp,
        duration: f64,
        time_zone: &Zone,
        is_all_day: bool,
        window: &DateInterval,
        limit: i64,
    ) -> RecurrenceExpansion {
        let unreadable = self.has_unreadable_rule();
        let span = if duration >= 0.0 { duration } else { 0.0 };
        let overlaps = |start: Timestamp| -> bool {
            if span == 0.0 {
                start >= window.start && start < window.end
            } else {
                start < window.end && add_seconds(start, span) > window.start
            }
        };
        if unreadable {
            return RecurrenceExpansion {
                starts: if overlaps(anchor) {
                    vec![anchor]
                } else {
                    Vec::new()
                },
                truncated: false,
                has_unreadable_rule: true,
            };
        }
        let mut candidates: Vec<Timestamp> = vec![anchor];
        let mut truncated = false;
        let skip_to = add_seconds(window.start, -span);
        for rule in &self.rules {
            // A COUNT rule cannot skip, so its instances before the window must not use up the limit; COUNT itself
            // bounds it (and the iteration cap guards the rest). Otherwise skipping leaves only a few early instances.
            let rule_limit = if matches!(rule.end, RuleEnd::Count(_)) {
                i64::MAX / 2
            } else {
                limit.saturating_add(64)
            };
            let expansion = rule.instances(
                anchor,
                time_zone,
                is_all_day,
                window.end,
                rule_limit,
                Some(skip_to),
            );
            candidates.extend(expansion.starts);
            truncated = truncated || expansion.truncated;
        }
        if let Some(extra) = &self.extra_dates {
            candidates.extend(extra.iter().copied());
        }
        let key = |date: Timestamp| -> Key {
            if is_all_day {
                Key::Day(AllDay::date_of(date, time_zone))
            } else {
                Key::Instant(date)
            }
        };
        let excluded_keys: BTreeSet<Key> = self
            .excluded_dates
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|d| key(*d))
            .collect();
        let mut seen: BTreeSet<Key> = BTreeSet::new();
        let mut result: Vec<Timestamp> = Vec::new();
        candidates.sort();
        for start in candidates {
            if overlaps(start) && !excluded_keys.contains(&key(start)) && seen.insert(key(start)) {
                result.push(start);
            }
        }
        if result.len() as i64 > limit {
            result.truncate(limit.max(0) as usize);
            truncated = true;
        }
        RecurrenceExpansion {
            starts: result,
            truncated,
            has_unreadable_rule: false,
        }
    }

    /// How many instances the first rule generates strictly before `date`, the anchor included and excluded dates
    /// counted (RFC 5545 `COUNT` counts generated instances before `EXDATE` removes any). Used to split a `COUNT` rule.
    pub fn rule_instance_count(
        &self,
        anchor: Timestamp,
        time_zone: &Zone,
        is_all_day: bool,
        before: Timestamp,
    ) -> i64 {
        let Some(rule) = self.rules.first() else {
            return i64::from(anchor < before);
        };
        rule.instances(anchor, time_zone, is_all_day, before, i64::MAX / 2, None)
            .starts
            .len() as i64
    }
}
