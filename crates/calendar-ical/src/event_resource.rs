//! Port of `EventResource.swift`.

use calendar_core::time::Zone;
use jiff::Timestamp;

use crate::component::ICalComponent;
use crate::content_line::{ICalError, ICalParser, ICalSerializer};
use crate::time_zone_resolver::TimeZoneResolver;
use crate::vtimezone_writer::VTimeZoneWriter;

/// One CalDAV calendar object resource: a `VCALENDAR` whose `VEVENT`s share one `UID` (the series master, if any, and
/// its `RECURRENCE-ID` overrides), plus its `VTIMEZONE`s and anything else the file holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventResource {
    /// The `VCALENDAR` component; always named `VCALENDAR`, with at least one `VEVENT` when built through `new`.
    pub calendar: ICalComponent,
}

impl EventResource {
    /// Wraps a parsed calendar. Fails with `ICalError::Malformed` when it is not a `VCALENDAR` or holds no `VEVENT`.
    pub fn new(calendar: ICalComponent) -> Result<EventResource, ICalError> {
        if calendar.name != "VCALENDAR" {
            return Err(ICalError::Malformed("not a VCALENDAR".to_string()));
        }
        if calendar.components_named("VEVENT").is_empty() {
            return Err(ICalError::Malformed("no VEVENT".to_string()));
        }
        Ok(EventResource { calendar })
    }

    /// Swift `init(data:)`.
    pub fn from_data(data: &[u8]) -> Result<EventResource, ICalError> {
        EventResource::new(ICalParser::parse(data)?)
    }

    /// Every `VEVENT` in file order: the master (if any) and the overrides.
    pub fn events(&self) -> Vec<&ICalComponent> {
        self.calendar.components_named("VEVENT")
    }

    /// The `VEVENT` without `RECURRENCE-ID`: the single event or the series master. `None` when the resource holds only
    /// overrides.
    pub fn master(&self) -> Option<&ICalComponent> {
        self.events()
            .into_iter()
            .find(|event| event.property("RECURRENCE-ID").is_none())
    }

    /// The `VEVENT`s with a `RECURRENCE-ID` (single-occurrence overrides of a series).
    pub fn overrides(&self) -> Vec<&ICalComponent> {
        self.events()
            .into_iter()
            .filter(|event| event.property("RECURRENCE-ID").is_some())
            .collect()
    }

    /// The `UID` of the first `VEVENT`, unescaped, or `None` without one.
    pub fn uid(&self) -> Option<String> {
        self.events().first()?.property("UID").map(|p| p.text())
    }

    /// A time-zone resolver built from the resource's `VTIMEZONE` components.
    pub fn resolver(&self) -> TimeZoneResolver {
        TimeZoneResolver::from_calendar(&self.calendar)
    }

    /// The resource as iCalendar bytes (CRLF line ends, folded lines).
    pub fn serialized(&self) -> Vec<u8> {
        ICalSerializer::serialize(&self.calendar).into_bytes()
    }

    /// Replaces the `VEVENT`s, keeping every other component where it was.
    pub fn set_events(&mut self, events: Vec<ICalComponent>) {
        let mut components: Vec<ICalComponent> = self
            .calendar
            .components
            .iter()
            .filter(|component| component.name != "VEVENT")
            .cloned()
            .collect();
        components.extend(events);
        self.calendar.components = components;
    }

    /// Adds a `VTIMEZONE` for each zone that has none yet (UTC needs none).
    pub fn ensure_time_zones(&mut self, zones: &[Zone], start: Timestamp, end: Timestamp) {
        let present: Vec<String> = self
            .calendar
            .components_named("VTIMEZONE")
            .iter()
            .filter_map(|component| component.property("TZID").map(|p| p.value.clone()))
            .collect();
        let mut added: Vec<ICalComponent> = Vec::new();
        for zone in zones {
            let identifier = zone.identifier();
            let already_added = added.iter().any(|component| {
                component
                    .property("TZID")
                    .is_some_and(|p| p.value == identifier)
            });
            if !present.iter().any(|id| id == identifier)
                && !already_added
                && let Some(definition) = VTimeZoneWriter::component(zone, start, end)
            {
                added.push(definition);
            }
        }
        added.append(&mut self.calendar.components);
        self.calendar.components = added;
    }
}
