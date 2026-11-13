//! Port of `EventReader.swift`.

use std::collections::BTreeSet;

use calendar_core::all_day::AllDay;
use calendar_core::conference_detector::ConferenceDetector;
use calendar_core::model::{
    Availability, CalendarEvent, EventStatus, SeriesInfo, UidScope, Visibility,
};
use calendar_core::recurrence::rule::RecurrenceRule;
use calendar_core::recurrence::set::{CalendarSeries, RecurrenceSet};
use calendar_core::time::{DateInterval, Zone, add_seconds, seconds_between};
use calendar_core::url_string::UrlString;
use jiff::Timestamp;

use crate::alarm_mapper::AlarmMapper;
use crate::attendee_mapper::AttendeeMapper;
use crate::component::ICalComponent;
use crate::content_line::ICalSerializer;
use crate::event_resource::EventResource;
use crate::ical_values::{ICalDateValue, ICalValues};
use crate::time_zone_resolver::TimeZoneResolver;

/// What the reader needs to know about the calendar an event resource came from.
#[derive(Clone, Debug)]
pub struct EventReadContext {
    /// Id of the calendar the events belong to; copied to `CalendarEvent::calendar_id`.
    pub calendar_id: String,
    /// The resource's last path segment; the series id and the base of every event id.
    pub resource_name: String,
    /// The server's entity tag for the resource, if known; copied to the events.
    pub etag: Option<String>,
    /// Id of the source (connection) the events come from, if known; copied to the events.
    pub source_id: Option<String>,
    /// Floating times and all-day dates are read in this zone.
    pub calendar_zone: Zone,
    /// The account's own calendar-user addresses, lowercased (`mailto:...`, `urn:uuid:...`, principal URLs).
    pub self_addresses: BTreeSet<String>,
}

impl EventReadContext {
    /// Creates a context from its parts.
    pub fn new(
        calendar_id: impl Into<String>,
        resource_name: impl Into<String>,
        etag: Option<String>,
        source_id: Option<String>,
        calendar_zone: Zone,
        self_addresses: BTreeSet<String>,
    ) -> EventReadContext {
        EventReadContext {
            calendar_id: calendar_id.into(),
            resource_name: resource_name.into(),
            etag,
            source_id,
            calendar_zone,
            self_addresses,
        }
    }
}

/// When a `VEVENT` happens. All-day events keep their length in days so an occurrence across a daylight-saving change
/// still ends at midnight.
#[derive(Clone, Debug, PartialEq)]
pub struct EventTimingInfo {
    /// Start instant of the first occurrence. For an all-day event this is the start of its first day in `zone`.
    pub start: Timestamp,
    /// End instant of the first occurrence. For an all-day event this is the start of the day after its last day (end
    /// exclusive).
    pub end: Timestamp,
    /// Time zone the event is shown in: its `TZID`, UTC for a `Z` time, or the calendar's zone for floating and all-day
    /// events.
    pub zone: Zone,
    /// True for a `VALUE=DATE` event.
    pub is_all_day: bool,
    /// Days covered by an all-day event (end exclusive); 0 for timed events.
    pub all_day_length: i64,
}

/// Reads `VEVENT`s into `CalendarEvent`s and expands recurring series.
pub enum EventReader {}

impl EventReader {
    /// Largest number of occurrences produced for one recurring series in one expansion.
    pub const INSTANCE_LIMIT: i64 = 5000;

    /// The id of an event: `resource_name` for a single event or a series master, and `resource_name#` plus the
    /// occurrence's original start (`yyyymmdd` for all-day, UTC date-time otherwise) for an occurrence of a series.
    pub fn event_id(
        resource_name: &str,
        original_start: Option<Timestamp>,
        is_all_day: bool,
        zone: &Zone,
    ) -> String {
        let Some(original_start) = original_start else {
            return resource_name.to_string();
        };
        let suffix = if is_all_day {
            ICalValues::date_text(&AllDay::date_of(original_start, zone))
        } else {
            ICalValues::utc_text(original_start)
        };
        format!("{resource_name}#{suffix}")
    }

    /// Every event of the resource overlapping `window`: expanded occurrences of the master, overrides in place of their
    /// slots (an override moved into the window counts, its slot does not), or the single event. (Swift
    /// `events(in:overlapping:context:)`.)
    pub fn events_in(
        resource: &EventResource,
        window: &DateInterval,
        context: &EventReadContext,
    ) -> Vec<CalendarEvent> {
        let resolver = resource.resolver();
        let mut result: Vec<CalendarEvent> = Vec::new();
        let mut overridden: BTreeSet<Timestamp> = BTreeSet::new();
        for vevent in resource.overrides() {
            let Some(slot) =
                EventReader::recurrence_id_of(vevent, &resolver, &context.calendar_zone)
            else {
                continue;
            };
            let Some(timing) = EventReader::timing_of(vevent, &resolver, &context.calendar_zone)
            else {
                continue;
            };
            overridden.insert(slot);
            if EventReader::overlaps(timing.start, timing.end, window) {
                result.push(EventReader::map(vevent, &timing, Some(slot), true, context));
            }
        }
        if let Some(master) = resource.master()
            && let Some(timing) = EventReader::timing_of(master, &resolver, &context.calendar_zone)
        {
            if let Some(set) =
                EventReader::recurrence_set_of(master, &timing.zone, timing.is_all_day, &resolver)
            {
                let expansion = set.occurrences(
                    timing.start,
                    seconds_between(timing.start, timing.end),
                    &timing.zone,
                    timing.is_all_day,
                    window,
                    EventReader::INSTANCE_LIMIT,
                );
                for start in expansion.starts {
                    if overridden.contains(&start) {
                        continue;
                    }
                    result.push(EventReader::map(
                        master,
                        &EventReader::moved(&timing, start),
                        Some(start),
                        true,
                        context,
                    ));
                }
            } else if EventReader::overlaps(timing.start, timing.end, window) {
                result.push(EventReader::map(master, &timing, None, false, context));
            }
        }
        result.sort_by(|a, b| (a.start, &a.event_id).cmp(&(b.start, &b.event_id)));
        result
    }

    /// The series master in its series form (`event_id` and `series_id` = the resource name, `original_start` = its
    /// start), or the single event for a resource that does not recur. `None` when the resource has no master.
    /// (Swift `masterEvent(of:context:)`.)
    pub fn master_event_of(
        resource: &EventResource,
        context: &EventReadContext,
    ) -> Option<CalendarEvent> {
        let resolver = resource.resolver();
        let master = resource.master()?;
        let timing = EventReader::timing_of(master, &resolver, &context.calendar_zone)?;
        let recurring =
            EventReader::recurrence_set_of(master, &timing.zone, timing.is_all_day, &resolver)
                .is_some();
        let mut event = EventReader::map(
            master,
            &timing,
            if recurring { Some(timing.start) } else { None },
            recurring,
            context,
        );
        event.event_id = context.resource_name.clone();
        Some(event)
    }

    /// The occurrence whose slot is `original_start`: its override, or the master's instance there. `None` when that
    /// slot is excluded or is not an occurrence. (Swift `occurrence(in:originalStart:context:)`.)
    pub fn occurrence_in(
        resource: &EventResource,
        original_start: Timestamp,
        context: &EventReadContext,
    ) -> Option<CalendarEvent> {
        let resolver = resource.resolver();
        for vevent in resource.overrides() {
            if EventReader::recurrence_id_of(vevent, &resolver, &context.calendar_zone)
                != Some(original_start)
            {
                continue;
            }
            let timing = EventReader::timing_of(vevent, &resolver, &context.calendar_zone)?;
            return Some(EventReader::map(
                vevent,
                &timing,
                Some(original_start),
                true,
                context,
            ));
        }
        let master = resource.master()?;
        let timing = EventReader::timing_of(master, &resolver, &context.calendar_zone)?;
        let set =
            EventReader::recurrence_set_of(master, &timing.zone, timing.is_all_day, &resolver)?;
        let probe = DateInterval::new(original_start, add_seconds(original_start, 1.0));
        let found = set.occurrences(
            timing.start,
            0.0,
            &timing.zone,
            timing.is_all_day,
            &probe,
            2,
        );
        if !found.starts.contains(&original_start) {
            return None;
        }
        Some(EventReader::map(
            master,
            &EventReader::moved(&timing, original_start),
            Some(original_start),
            true,
            context,
        ))
    }

    /// Swift `series(of:context:)`.
    pub fn series_of(
        resource: &EventResource,
        context: &EventReadContext,
    ) -> Option<CalendarSeries> {
        let resolver = resource.resolver();
        let master = resource.master()?;
        let timing = EventReader::timing_of(master, &resolver, &context.calendar_zone)?;
        let set =
            EventReader::recurrence_set_of(master, &timing.zone, timing.is_all_day, &resolver)?;
        Some(CalendarSeries::new(
            context.resource_name.clone(),
            context.calendar_id.clone(),
            timing.start,
            timing.zone,
            timing.is_all_day,
            set,
        ))
    }

    // MARK: Parts

    /// Swift `timing(of:resolver:calendarZone:)`.
    pub fn timing_of(
        vevent: &ICalComponent,
        resolver: &TimeZoneResolver,
        calendar_zone: &Zone,
    ) -> Option<EventTimingInfo> {
        let start_property = vevent.property("DTSTART")?;
        let start_value = ICalValues::date_value(start_property)?;
        if let ICalDateValue::Date(first) = &start_value {
            let mut length: i64 = 1;
            if let Some(end_property) = vevent.property("DTEND")
                && let Some(ICalDateValue::Date(last)) = ICalValues::date_value(end_property)
            {
                length = std::cmp::max(1, first.days_to(&last));
            } else if let Some(duration) = vevent
                .property("DURATION")
                .and_then(|p| ICalValues::duration(&p.value))
                && duration >= 86_400.0
            {
                length = (duration / 86_400.0) as i64;
            }
            let (start, end) = AllDay::canonical(first, &first.adding_days(length), calendar_zone)?;
            return Some(EventTimingInfo {
                start,
                end,
                zone: calendar_zone.clone(),
                is_all_day: true,
                all_day_length: length,
            });
        }
        let start = resolver.date(&start_value, calendar_zone)?;
        let mut end = start;
        if let Some(end_property) = vevent.property("DTEND")
            && let Some(value) = ICalValues::date_value(end_property)
            && let Some(date) = resolver.date(&value, calendar_zone)
            && date >= start
        {
            end = date;
        } else if let Some(duration) = vevent
            .property("DURATION")
            .and_then(|p| ICalValues::duration(&p.value))
            && duration >= 0.0
        {
            end = add_seconds(start, duration);
        }
        Some(EventTimingInfo {
            start,
            end,
            zone: resolver.display_zone(&start_value, calendar_zone),
            is_all_day: false,
            all_day_length: 0,
        })
    }

    /// Swift `recurrenceID(of:resolver:calendarZone:)`.
    pub fn recurrence_id_of(
        vevent: &ICalComponent,
        resolver: &TimeZoneResolver,
        calendar_zone: &Zone,
    ) -> Option<Timestamp> {
        let property = vevent.property("RECURRENCE-ID")?;
        let value = ICalValues::date_value(property)?;
        resolver.date(&value, calendar_zone)
    }

    /// The master's `RRULE`, `RDATE` and `EXDATE`, with dates resolved by this file's zones; `None` when it does not
    /// recur. An `RRULE` that cannot be read is kept in `unparsed` (so the set reports `has_unreadable_rule`). (Swift
    /// `recurrenceSet(of:zone:isAllDay:resolver:)`.)
    pub fn recurrence_set_of(
        master: &ICalComponent,
        zone: &Zone,
        _is_all_day: bool,
        resolver: &TimeZoneResolver,
    ) -> Option<RecurrenceSet> {
        let rule_properties = master.properties_named("RRULE");
        let extra = master.properties_named("RDATE");
        if rule_properties.is_empty() && extra.is_empty() {
            return None;
        }
        let mut rules: Vec<RecurrenceRule> = Vec::new();
        let mut unparsed: Vec<String> = Vec::new();
        for property in rule_properties {
            match RecurrenceRule::from_rrule(&property.value, Some(zone)) {
                Ok(rule) => rules.push(rule),
                Err(_) => unparsed.push(ICalSerializer::content_line(property)),
            }
        }
        let dates = |properties: Vec<&crate::component::ICalProperty>| -> Vec<Timestamp> {
            properties
                .into_iter()
                .flat_map(ICalValues::date_values)
                .filter_map(|value| resolver.date(&value, zone))
                .collect()
        };
        Some(RecurrenceSet {
            rules,
            extra_dates: Some(dates(extra)),
            excluded_dates: Some(dates(master.properties_named("EXDATE"))),
            unparsed,
        })
    }

    fn moved(timing: &EventTimingInfo, start: Timestamp) -> EventTimingInfo {
        let mut copy = timing.clone();
        copy.start = start;
        if timing.is_all_day {
            let first = AllDay::date_of(start, &timing.zone);
            copy.end =
                AllDay::start_of_day(&first.adding_days(timing.all_day_length), &timing.zone)
                    .unwrap_or_else(|| add_seconds(start, timing.all_day_length as f64 * 86_400.0));
        } else {
            copy.end = add_seconds(start, seconds_between(timing.start, timing.end));
        }
        copy
    }

    fn overlaps(start: Timestamp, end: Timestamp, window: &DateInterval) -> bool {
        if end > start {
            start < window.end && end > window.start
        } else {
            start >= window.start && start < window.end
        }
    }

    fn map(
        vevent: &ICalComponent,
        timing: &EventTimingInfo,
        original_start: Option<Timestamp>,
        recurring: bool,
        context: &EventReadContext,
    ) -> CalendarEvent {
        let (attendees, organizer) = AttendeeMapper::read(vevent, &context.self_addresses);
        let notes = vevent.property("DESCRIPTION").map(|p| p.text());
        let location = vevent.property("LOCATION").map(|p| p.text());
        let url = vevent
            .property("URL")
            .and_then(|p| UrlString::parse(&p.value));
        let status = match vevent
            .property("STATUS")
            .map(|p| p.value.to_uppercase())
            .as_deref()
        {
            Some("CANCELLED") => EventStatus::Cancelled,
            Some("TENTATIVE") => EventStatus::Tentative,
            _ => EventStatus::Confirmed,
        };
        let visibility = match vevent
            .property("CLASS")
            .map(|p| p.value.to_uppercase())
            .as_deref()
        {
            Some("PUBLIC") => Visibility::PublicEvent,
            Some("PRIVATE") => Visibility::PrivateEvent,
            Some("CONFIDENTIAL") => Visibility::Confidential,
            _ => Visibility::Default,
        };
        let utc = |name: &str| -> Option<Timestamp> {
            let property = vevent.property(name)?;
            match ICalValues::date_value(property) {
                Some(ICalDateValue::Utc(date)) => Some(date),
                _ => None,
            }
        };
        let participation = AttendeeMapper::participation(
            &attendees,
            organizer.as_ref(),
            !context.self_addresses.is_empty(),
        );
        let conferences = ConferenceDetector::conferences(
            &[],
            location.as_deref(),
            url.as_ref(),
            notes.as_deref(),
        );
        let mut event = CalendarEvent::new(
            EventReader::event_id(
                &context.resource_name,
                if recurring { original_start } else { None },
                timing.is_all_day,
                &timing.zone,
            ),
            context.calendar_id.clone(),
            vevent
                .property("SUMMARY")
                .map(|p| p.text())
                .unwrap_or_default(),
            timing.start,
            timing.end,
        );
        event.uid = vevent.property("UID").map(|p| p.text());
        event.uid_scope = Some(UidScope::Global);
        event.notes = notes.filter(|n| !n.is_empty());
        event.location = location.filter(|l| !l.is_empty());
        event.time_zone = timing.zone.clone();
        event.is_all_day = timing.is_all_day;
        event.status = status;
        event.availability = Some(
            if vevent
                .property("TRANSP")
                .map(|p| p.value.to_uppercase())
                .as_deref()
                == Some("TRANSPARENT")
            {
                Availability::Free
            } else {
                Availability::Busy
            },
        );
        event.visibility = Some(visibility);
        event.series = Some(if recurring {
            SeriesInfo::Occurrence {
                series_id: context.resource_name.clone(),
                original_start,
            }
        } else {
            SeriesInfo::NotRecurring
        });
        event.attendees = attendees;
        event.organizer = organizer;
        event.conferences = conferences;
        event.reminders = Some(AlarmMapper::reminders_from(
            vevent.components_named("VALARM"),
        ));
        event.url = url;
        event.version = context.etag.clone();
        event.last_modified = utc("LAST-MODIFIED");
        event.created = utc("CREATED");
        event.participation = participation;
        event.source_id = context.source_id.clone();
        event
    }
}
