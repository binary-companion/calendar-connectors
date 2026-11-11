//! Rust port of the Swift product `CalendarCore`: portable calendar model, time, errors and seams.
//!
//! This crate reads no clock, no environment and no network; time arrives as an argument or a `now` closure.

pub mod civil;
pub mod failure;
pub mod files;
pub mod json;
pub mod time;
pub mod url_string;

pub use failure::{Failure, Outcome};
pub use time::{DateInterval, Zone};
pub use url_string::UrlString;

#[cfg(test)]
mod tests;
