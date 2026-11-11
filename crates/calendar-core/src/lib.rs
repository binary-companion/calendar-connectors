//! Rust port of the Swift product `CalendarCore`: portable calendar model, time, errors and seams.
//!
//! This crate reads no clock, no environment and no network; time arrives as an argument or a `now` closure.

mod raw_enum;

pub mod all_day;
pub mod calendar_user_address;
pub mod civil;
pub mod conference_detector;
pub mod diagnostic_log;
pub mod failure;
pub mod files;
pub mod http;
pub mod join_url_policy;
pub mod json;
pub mod model;
pub mod recurrence;
pub mod source_types;
pub mod time;
pub mod url_string;
pub mod windows_time_zones;
pub mod write;

pub use failure::{Failure, Outcome};
pub use time::{DateInterval, Zone};
pub use url_string::UrlString;

#[cfg(test)]
mod tests;
