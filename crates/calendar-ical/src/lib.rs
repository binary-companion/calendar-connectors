//! Rust port of the Swift product `ICalendar`: the iCalendar content-line parser and serializer, value types, time
//! zone resolution, `VTIMEZONE` writing, and the readers and writers CalDAV uses for events and series.
//!
//! Parsers return errors and never panic on any input. This crate reads no clock, no environment and no network.

mod grapheme;
mod grapheme_tables;
mod swift_number;
mod zone_data;
mod zone_support;

pub mod alarm_mapper;
pub mod attendee_mapper;
pub mod component;
pub mod content_line;
pub mod event_reader;
pub mod event_resource;
pub mod event_writer;
pub mod ical_text;
pub mod ical_values;
pub mod series_editor;
pub mod time_zone_resolver;
pub mod vtimezone_writer;

pub use alarm_mapper::AlarmMapper;
pub use attendee_mapper::AttendeeMapper;
pub use component::{ICalComponent, ICalParameter, ICalProperty};
pub use content_line::{ICalError, ICalParser, ICalSerializer};
pub use event_reader::{EventReadContext, EventReader, EventTimingInfo};
pub use event_resource::EventResource;
pub use event_writer::EventWriter;
pub use ical_text::ICalText;
pub use ical_values::{ICalDateValue, ICalValues, LocalDateTime};
pub use series_editor::SeriesEditor;
pub use time_zone_resolver::TimeZoneResolver;
pub use vtimezone_writer::VTimeZoneWriter;

#[cfg(test)]
mod tests;
