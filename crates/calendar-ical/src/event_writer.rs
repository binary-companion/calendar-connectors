//! Port of `EventWriter.swift`.

use std::collections::BTreeSet;

use calendar_core::all_day::AllDay;
use calendar_core::model::{Availability, Visibility};
use calendar_core::time::Zone;
use calendar_core::write::event_draft::{EventDraft, EventTiming};
use calendar_core::write::event_patch::{EventPatch, FieldUpdate};
use calendar_core::write::types::{EventField, WriteError};
use jiff::Timestamp;

use crate::alarm_mapper::AlarmMapper;
use crate::attendee_mapper::AttendeeMapper;
use crate::component::{ICalComponent, ICalParameter, ICalProperty};
use crate::content_line::ICalError;
use crate::event_resource::EventResource;
use crate::ical_values::ICalValues;
use crate::time_zone_resolver::TimeZoneResolver;

pub enum EventWriter {}

impl EventWriter {
    pub const PRODUCT_ID: &'static str = "-//TimeTug//CalendarConnectors//EN";

    /// A new `VEVENT`. With attendees, `organizer_address` (the account's `mailto:` address) becomes the organizer and
    /// its own accepted attendee entry, as iCloud writes it. Fields are validated by the caller (`EventDraft::validate`
    /// and the source's capabilities). (Swift `vevent(from:uid:now:organizerAddress:)`.)
    pub fn vevent_from(
        draft: &EventDraft,
        uid: &str,
        now: Timestamp,
        organizer_address: Option<&str>,
    ) -> Result<ICalComponent, WriteError> {
        let mut vevent = ICalComponent::new(
            "VEVENT",
            vec![
                ICalProperty::from_text("UID", uid, Vec::new()),
                ICalProperty::with_value("DTSTAMP", ICalValues::utc_text(now)),
                ICalProperty::with_value("CREATED", ICalValues::utc_text(now)),
                ICalProperty::with_value("LAST-MODIFIED", ICalValues::utc_text(now)),
                ICalProperty::with_value("SEQUENCE", "0"),
                ICalProperty::from_text("SUMMARY", &draft.title, Vec::new()),
            ],
            Vec::new(),
        );
        vevent.set_text("DESCRIPTION", draft.notes.as_deref());
        vevent.set_text("LOCATION", draft.location.as_deref());
        EventWriter::set_timing(&draft.timing, &mut vevent);
        vevent.set(EventWriter::transparency(draft.availability));
        if let Some(value) = EventWriter::class_value(draft.visibility) {
            vevent.set(ICalProperty::with_value("CLASS", value));
        }
        if let Some(rule) = &draft.recurrence {
            let zone = draft.timing.time_zone.clone().unwrap_or_else(Zone::utc);
            vevent.set(ICalProperty::with_value(
                "RRULE",
                rule.rrule_string(draft.timing.is_all_day, Some(&zone)),
            ));
        }
        if !draft.attendees.is_empty() {
            if let Some(address) = organizer_address {
                vevent.set(AttendeeMapper::organizer_property(address));
                vevent.append(EventWriter::chair_attendee(address));
            }
            for attendee in &draft.attendees {
                vevent.append(AttendeeMapper::property_for(attendee));
            }
        }
        vevent.components = AlarmMapper::alarms_from(draft.reminders.as_deref().unwrap_or(&[]))?;
        Ok(vevent)
    }

    /// A resource holding `vevent` (and any overrides the caller adds later) with a `VTIMEZONE` for each zone.
    /// (Swift `resource(for:zones:from:through:)`.)
    pub fn resource_for(
        vevent: ICalComponent,
        zones: &[Zone],
        start: Timestamp,
        end: Timestamp,
    ) -> Result<EventResource, ICalError> {
        let mut resource = EventResource::new(ICalComponent::new(
            "VCALENDAR",
            vec![
                ICalProperty::with_value("VERSION", "2.0"),
                ICalProperty::from_text("PRODID", EventWriter::PRODUCT_ID, Vec::new()),
                ICalProperty::with_value("CALSCALE", "GREGORIAN"),
            ],
            vec![vevent],
        ))?;
        resource.ensure_time_zones(zones, start, end);
        Ok(resource)
    }

    /// A date property in the form the connector writes: `VALUE=DATE` for all-day, `Z` for UTC, else `TZID`.
    pub fn date_property(
        name: &str,
        date: Timestamp,
        zone: &Zone,
        is_all_day: bool,
    ) -> ICalProperty {
        if is_all_day {
            return ICalProperty::new(
                name,
                vec![ICalParameter::single("VALUE", "DATE")],
                ICalValues::date_text(&AllDay::date_of(date, zone)),
            );
        }
        if zone.identifier() == "UTC" || zone.identifier() == "GMT" {
            return ICalProperty::with_value(name, ICalValues::utc_text(date));
        }
        ICalProperty::new(
            name,
            vec![ICalParameter::single("TZID", zone.identifier())],
            ICalValues::local_text(date, zone),
        )
    }

    pub fn set_timing(timing: &EventTiming, vevent: &mut ICalComponent) {
        let zone = timing.time_zone.clone().unwrap_or_else(Zone::utc);
        vevent.set(EventWriter::date_property(
            "DTSTART",
            timing.start,
            &zone,
            timing.is_all_day,
        ));
        vevent.set(EventWriter::date_property(
            "DTEND",
            timing.end,
            &zone,
            timing.is_all_day,
        ));
        vevent.remove_properties_named("DURATION");
    }

    /// Applies a patch whose fields the caller has already checked against the capabilities. Clearing reminders (the
    /// calendar's defaults) cannot be expressed in CalDAV and fails with `.unsupported(fields: [.reminders])`. A
    /// generated or removed conference is refused by the caller before this is reached. `calendar_zone` is the zone a
    /// date `UNTIL` of an all-day series is written in.
    pub fn apply(
        patch: &EventPatch,
        vevent: &mut ICalComponent,
        now: Timestamp,
        organizer_address: Option<&str>,
        calendar_zone: Option<&Zone>,
    ) -> Result<(), WriteError> {
        if matches!(patch.reminders, FieldUpdate::Clear) {
            return Err(WriteError::Unsupported {
                fields: [EventField::Reminders].into_iter().collect(),
            });
        }
        if let Some(title) = &patch.title {
            vevent.set(ICalProperty::from_text("SUMMARY", title, Vec::new()));
        }
        match &patch.notes {
            FieldUpdate::Keep => {}
            FieldUpdate::Set(text) => vevent.set_text("DESCRIPTION", Some(text)),
            FieldUpdate::Clear => vevent.remove_properties_named("DESCRIPTION"),
        }
        match &patch.location {
            FieldUpdate::Keep => {}
            FieldUpdate::Set(text) => vevent.set_text("LOCATION", Some(text)),
            FieldUpdate::Clear => vevent.remove_properties_named("LOCATION"),
        }
        if let Some(timing) = &patch.timing {
            EventWriter::set_timing(timing, vevent);
        }
        if let Some(availability) = patch.availability {
            vevent.set(EventWriter::transparency(availability));
        }
        if let Some(visibility) = patch.visibility {
            match EventWriter::class_value(visibility) {
                Some(value) => vevent.set(ICalProperty::with_value("CLASS", value)),
                None => vevent.remove_properties_named("CLASS"),
            }
        }
        if let FieldUpdate::Set(reminders) = &patch.reminders {
            let mut components: Vec<ICalComponent> = vevent
                .components
                .iter()
                .filter(|component| component.name != "VALARM")
                .cloned()
                .collect();
            components.extend(AlarmMapper::alarms_from(reminders)?);
            vevent.components = components;
        }
        if let Some(changes) = &patch.attendees
            && !changes.is_empty()
        {
            let removed: BTreeSet<String> =
                changes.remove.iter().map(|e| e.to_lowercase()).collect();
            vevent.properties.retain(|property| {
                !(property.name == "ATTENDEE"
                    && AttendeeMapper::email_of(property)
                        .is_some_and(|email| removed.contains(&email)))
            });
            if vevent.property("ORGANIZER").is_none()
                && !changes.add.is_empty()
                && let Some(address) = organizer_address
            {
                vevent.set(AttendeeMapper::organizer_property(address));
                vevent.append(EventWriter::chair_attendee(address));
            }
            for draft in &changes.add {
                let index = vevent.properties.iter().position(|property| {
                    property.name == "ATTENDEE"
                        && AttendeeMapper::email_of(property).as_deref() == Some(draft.email())
                });
                if let Some(index) = index {
                    let mut existing = AttendeeMapper::property_for(draft);
                    let partstat = vevent.properties[index]
                        .parameter("PARTSTAT")
                        .unwrap_or("NEEDS-ACTION")
                        .to_string();
                    existing.set_parameter("PARTSTAT", Some(&partstat));
                    vevent.properties[index] = existing;
                } else {
                    vevent.append(AttendeeMapper::property_for(draft));
                }
            }
        }
        match &patch.recurrence {
            FieldUpdate::Keep => {}
            FieldUpdate::Set(rule) => {
                let start = vevent.property("DTSTART");
                let is_all_day = start
                    .and_then(|p| p.parameter("VALUE"))
                    .map(str::to_uppercase)
                    .as_deref()
                    == Some("DATE");
                // An all-day series has no TZID, and its date UNTIL is read in the calendar's zone (as `create` and
                // `SeriesEditor::split` write it), so write it in that zone too.
                let from_tzid = start
                    .and_then(|p| p.parameter("TZID"))
                    .and_then(|tzid| TimeZoneResolver::new().zone_for(tzid));
                let zone = from_tzid
                    .or_else(|| {
                        if is_all_day {
                            calendar_zone
                                .cloned()
                                .or_else(|| patch.timing.as_ref().and_then(|t| t.time_zone.clone()))
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(Zone::utc);
                vevent.set(ICalProperty::with_value(
                    "RRULE",
                    rule.rrule_string(is_all_day, Some(&zone)),
                ));
            }
            FieldUpdate::Clear => {
                for name in ["RRULE", "RDATE", "EXDATE", "EXRULE"] {
                    vevent.remove_properties_named(name);
                }
            }
        }
        let significant = patch.timing.is_some()
            || !matches!(patch.recurrence, FieldUpdate::Keep)
            || !patch.attendees.as_ref().is_none_or(|a| a.is_empty());
        EventWriter::touch(vevent, now, significant);
        Ok(())
    }

    /// Updates `DTSTAMP` and `LAST-MODIFIED`, and raises `SEQUENCE` for a change attendees must hear about.
    pub fn touch(vevent: &mut ICalComponent, now: Timestamp, bump_sequence: bool) {
        vevent.set(ICalProperty::with_value(
            "DTSTAMP",
            ICalValues::utc_text(now),
        ));
        vevent.set(ICalProperty::with_value(
            "LAST-MODIFIED",
            ICalValues::utc_text(now),
        ));
        if bump_sequence {
            let current = vevent
                .property("SEQUENCE")
                .and_then(|p| p.value.parse::<i64>().ok())
                .unwrap_or(0);
            vevent.set(ICalProperty::with_value(
                "SEQUENCE",
                (current + 1).to_string(),
            ));
        }
    }

    pub(crate) fn class_value(visibility: Visibility) -> Option<&'static str> {
        match visibility {
            Visibility::Default => None,
            Visibility::PublicEvent => Some("PUBLIC"),
            Visibility::PrivateEvent => Some("PRIVATE"),
            Visibility::Confidential => Some("CONFIDENTIAL"),
        }
    }

    fn transparency(availability: Availability) -> ICalProperty {
        let supported: BTreeSet<Availability> = [Availability::Busy, Availability::Free]
            .into_iter()
            .collect();
        ICalProperty::with_value(
            "TRANSP",
            if availability.closest_in(&supported) == Some(Availability::Free) {
                "TRANSPARENT"
            } else {
                "OPAQUE"
            },
        )
    }

    fn chair_attendee(address: &str) -> ICalProperty {
        ICalProperty::new(
            "ATTENDEE",
            vec![
                ICalParameter::single("PARTSTAT", "ACCEPTED"),
                ICalParameter::single("ROLE", "CHAIR"),
            ],
            address,
        )
    }
}
