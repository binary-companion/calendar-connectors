//! Port of `SeriesEditor.swift`.

use calendar_core::all_day::AllDay;
use calendar_core::recurrence::rule::RuleEnd;
use calendar_core::time::{DateInterval, Zone, add_seconds, seconds_between};
use calendar_core::write::types::{EventField, WriteError};
use jiff::Timestamp;

use crate::component::{ICalComponent, ICalParameter, ICalProperty};
use crate::event_reader::EventReader;
use crate::event_resource::EventResource;
use crate::event_writer::EventWriter;
use crate::ical_values::{ICalDateValue, ICalValues};
use crate::time_zone_resolver::TimeZoneResolver;

/// Edits a recurring resource in place. Every date it writes (`RECURRENCE-ID`, `EXDATE`, `RDATE`, `UNTIL`) uses the
/// master's own `DTSTART` form (the same `TZID`, `Z` or `VALUE=DATE`), so the server and other clients match them.
pub enum SeriesEditor {}

#[derive(Clone, Debug)]
pub(crate) struct Form {
    pub(crate) zone: Zone,
    pub(crate) is_all_day: bool,
    /// `None`: UTC with `Z` (or floating when `floating`).
    pub(crate) tzid: Option<String>,
    pub(crate) floating: bool,
}

impl Form {
    pub(crate) fn property(&self, name: &str, date: Timestamp) -> ICalProperty {
        if self.is_all_day {
            return ICalProperty::new(
                name,
                vec![ICalParameter::single("VALUE", "DATE")],
                ICalValues::date_text(&AllDay::date_of(date, &self.zone)),
            );
        }
        if let Some(tzid) = &self.tzid {
            return ICalProperty::new(
                name,
                vec![ICalParameter::single("TZID", tzid)],
                ICalValues::local_text(date, &self.zone),
            );
        }
        if self.floating {
            return ICalProperty::with_value(name, ICalValues::local_text(date, &self.zone));
        }
        ICalProperty::with_value(name, ICalValues::utc_text(date))
    }

    /// One property listing several dates (EXDATE, RDATE). `dates` is never empty.
    pub(crate) fn list_property(&self, name: &str, dates: &[Timestamp]) -> ICalProperty {
        let one = self.property(name, dates[0]);
        let values: Vec<String> = dates
            .iter()
            .map(|date| self.property(name, *date).value)
            .collect();
        ICalProperty::new(name, one.parameters, values.join(","))
    }
}

fn sorted(mut dates: Vec<Timestamp>) -> Vec<Timestamp> {
    dates.sort();
    dates
}

impl SeriesEditor {
    pub(crate) fn form_of(
        master: &ICalComponent,
        resolver: &TimeZoneResolver,
        calendar_zone: &Zone,
    ) -> Option<Form> {
        let start = master.property("DTSTART")?;
        let value = ICalValues::date_value(start)?;
        Some(match value {
            ICalDateValue::Date(_) => Form {
                zone: calendar_zone.clone(),
                is_all_day: true,
                tzid: None,
                floating: false,
            },
            ICalDateValue::Utc(_) => Form {
                zone: Zone::utc(),
                is_all_day: false,
                tzid: None,
                floating: false,
            },
            ICalDateValue::Local {
                tzid: Some(tzid), ..
            } => Form {
                zone: resolver
                    .zone_for(&tzid)
                    .unwrap_or_else(|| calendar_zone.clone()),
                is_all_day: false,
                tzid: Some(tzid),
                floating: false,
            },
            ICalDateValue::Local { tzid: None, .. } => Form {
                zone: calendar_zone.clone(),
                is_all_day: false,
                tzid: None,
                floating: true,
            },
        })
    }

    // MARK: Overrides and exclusions

    /// The override `VEVENT` for `slot`: the existing one, or a copy of the master moved to the slot. `None` when
    /// `slot` is not an occurrence. (Swift `override(in:at:calendarZone:)`.)
    pub fn override_in(
        resource: &EventResource,
        slot: Timestamp,
        calendar_zone: &Zone,
    ) -> Option<ICalComponent> {
        let resolver = resource.resolver();
        if let Some(existing) = resource
            .overrides()
            .into_iter()
            .find(|o| EventReader::recurrence_id_of(o, &resolver, calendar_zone) == Some(slot))
        {
            return Some(existing.clone());
        }
        let master = resource.master()?;
        let timing = EventReader::timing_of(master, &resolver, calendar_zone)?;
        let form = SeriesEditor::form_of(master, &resolver, calendar_zone)?;
        if !SeriesEditor::is_slot(slot, resource, calendar_zone) {
            return None;
        }
        let mut copy = master.clone();
        for name in ["RRULE", "RDATE", "EXDATE", "EXRULE"] {
            copy.remove_properties_named(name);
        }
        let end = if timing.is_all_day {
            AllDay::start_of_day(
                &AllDay::date_of(slot, &form.zone).adding_days(timing.all_day_length),
                &form.zone,
            )
            .unwrap_or(slot)
        } else {
            add_seconds(slot, seconds_between(timing.start, timing.end))
        };
        copy.set(form.property("DTSTART", slot));
        copy.set(form.property("DTEND", end));
        copy.remove_properties_named("DURATION");
        copy.set(form.property("RECURRENCE-ID", slot));
        Some(copy)
    }

    pub fn set_override(
        vevent: ICalComponent,
        slot: Timestamp,
        resource: &mut EventResource,
        calendar_zone: &Zone,
    ) {
        let resolver = resource.resolver();
        let mut events: Vec<ICalComponent> = resource
            .events()
            .into_iter()
            .filter(|e| {
                e.property("RECURRENCE-ID").is_none()
                    || EventReader::recurrence_id_of(e, &resolver, calendar_zone) != Some(slot)
            })
            .cloned()
            .collect();
        events.push(vevent);
        resource.set_events(events);
    }

    pub fn exclude(
        slot: Timestamp,
        resource: &mut EventResource,
        calendar_zone: &Zone,
    ) -> Result<(), WriteError> {
        let resolver = resource.resolver();
        if !SeriesEditor::is_slot(slot, resource, calendar_zone) {
            return Err(WriteError::NotFound);
        }
        let Some(mut master) = resource.master().cloned() else {
            return Err(WriteError::NotFound);
        };
        let Some(form) = SeriesEditor::form_of(&master, &resolver, calendar_zone) else {
            return Err(WriteError::NotFound);
        };
        let mut excluded =
            SeriesEditor::dates(&master.properties_named("EXDATE"), &resolver, &form);
        excluded.push(slot);
        master.set(form.list_property("EXDATE", &sorted(excluded)));
        let overrides: Vec<ICalComponent> = resource
            .overrides()
            .into_iter()
            .filter(|o| EventReader::recurrence_id_of(o, &resolver, calendar_zone) != Some(slot))
            .cloned()
            .collect();
        let mut events = vec![master];
        events.extend(overrides);
        resource.set_events(events);
        Ok(())
    }

    // MARK: Series-wide changes

    pub fn shift(resource: &mut EventResource, delta: f64, calendar_zone: &Zone) {
        let Some(mut master) = resource.master().cloned() else {
            return;
        };
        if delta == 0.0 {
            return;
        }
        let resolver = resource.resolver();
        let Some(form) = SeriesEditor::form_of(&master, &resolver, calendar_zone) else {
            return;
        };
        for name in ["EXDATE", "RDATE"] {
            let moved: Vec<Timestamp> =
                SeriesEditor::dates(&master.properties_named(name), &resolver, &form)
                    .into_iter()
                    .map(|date| SeriesEditor::moved(date, delta, &form))
                    .collect();
            master.remove_properties_named(name);
            if !moved.is_empty() {
                master.append(form.list_property(name, &sorted(moved)));
            }
        }
        let overrides: Vec<ICalComponent> = resource
            .overrides()
            .into_iter()
            .map(|vevent| {
                let Some(slot) = EventReader::recurrence_id_of(vevent, &resolver, calendar_zone)
                else {
                    return vevent.clone();
                };
                let mut copy = vevent.clone();
                let timing = EventReader::timing_of(vevent, &resolver, calendar_zone);
                copy.set(form.property("RECURRENCE-ID", SeriesEditor::moved(slot, delta, &form)));
                // An override still at its slot's time moves with the series; one the user moved keeps its own time.
                if let Some(timing) = timing
                    && timing.start == slot
                {
                    copy.set(
                        form.property("DTSTART", SeriesEditor::moved(timing.start, delta, &form)),
                    );
                    copy.set(form.property("DTEND", SeriesEditor::moved(timing.end, delta, &form)));
                }
                copy
            })
            .collect();
        let mut events = vec![master];
        events.extend(overrides);
        resource.set_events(events);
    }

    pub fn prune_unmatched(resource: &mut EventResource, calendar_zone: &Zone) {
        let Some(mut master) = resource.master().cloned() else {
            return;
        };
        let resolver = resource.resolver();
        let Some(form) = SeriesEditor::form_of(&master, &resolver, calendar_zone) else {
            return;
        };
        // Judge against the rule without exclusions, so an EXDATE still matching an occurrence stays.
        let mut bare = resource.clone();
        let mut bare_master = master.clone();
        bare_master.remove_properties_named("EXDATE");
        bare.set_events(vec![bare_master]);
        let kept: Vec<Timestamp> =
            SeriesEditor::dates(&master.properties_named("EXDATE"), &resolver, &form)
                .into_iter()
                .filter(|date| SeriesEditor::is_slot(*date, &bare, calendar_zone))
                .collect();
        master.remove_properties_named("EXDATE");
        if !kept.is_empty() {
            master.append(form.list_property("EXDATE", &sorted(kept)));
        }
        let overrides: Vec<ICalComponent> = resource
            .overrides()
            .into_iter()
            .filter(|vevent| {
                EventReader::recurrence_id_of(vevent, &resolver, calendar_zone)
                    .is_some_and(|slot| SeriesEditor::is_slot(slot, &bare, calendar_zone))
            })
            .cloned()
            .collect();
        let mut events = vec![master];
        events.extend(overrides);
        resource.set_events(events);
    }

    // MARK: Split

    pub fn split(
        resource: &EventResource,
        slot: Timestamp,
        new_uid: &str,
        calendar_zone: &Zone,
        now: Timestamp,
    ) -> Result<(EventResource, EventResource), WriteError> {
        let resolver = resource.resolver();
        let Some(master) = resource.master() else {
            return Err(WriteError::NotFound);
        };
        let Some(timing) = EventReader::timing_of(master, &resolver, calendar_zone) else {
            return Err(WriteError::NotFound);
        };
        let Some(form) = SeriesEditor::form_of(master, &resolver, calendar_zone) else {
            return Err(WriteError::NotFound);
        };
        let Some(set) =
            EventReader::recurrence_set_of(master, &form.zone, timing.is_all_day, &resolver)
        else {
            return Err(WriteError::NotFound);
        };
        if set.rules.len() > 1 || !set.unparsed.is_empty() {
            return Err(WriteError::Unsupported {
                fields: [EventField::Recurrence].into_iter().collect(),
            });
        }
        if !(slot > timing.start
            && SeriesEditor::is_slot_or_override(slot, resource, calendar_zone))
        {
            return Err(WriteError::NotFound);
        }

        let mut head_master = master.clone();
        let mut tail_master = master.clone();
        if let Some(rule) = set.rules.first() {
            let mut head_rule = rule.clone();
            let mut tail_rule = rule.clone();
            match rule.end {
                RuleEnd::Count(total) => {
                    let before =
                        set.rule_instance_count(timing.start, &form.zone, timing.is_all_day, slot);
                    if total - before < 1 {
                        return Err(WriteError::NotFound);
                    }
                    head_rule.end = RuleEnd::Count(before);
                    tail_rule.end = RuleEnd::Count(total - before);
                }
                RuleEnd::Until(_) | RuleEnd::Never => {
                    if timing.is_all_day {
                        // The previous calendar day, by date arithmetic: a 23-hour (spring-forward) day must not skip a date.
                        let previous = AllDay::date_of(slot, &form.zone).adding_days(-1);
                        head_rule.end = RuleEnd::Until(
                            AllDay::start_of_day(&previous, &form.zone)
                                .unwrap_or_else(|| add_seconds(slot, -86_400.0)),
                        );
                    } else {
                        head_rule.end = RuleEnd::Until(add_seconds(slot, -1.0));
                    }
                }
            }
            head_master.set(ICalProperty::with_value(
                "RRULE",
                head_rule.rrule_string(timing.is_all_day, Some(&form.zone)),
            ));
            tail_master.set(ICalProperty::with_value(
                "RRULE",
                tail_rule.rrule_string(timing.is_all_day, Some(&form.zone)),
            ));
        }
        for name in ["EXDATE", "RDATE"] {
            let all = SeriesEditor::dates(&master.properties_named(name), &resolver, &form);
            head_master.remove_properties_named(name);
            tail_master.remove_properties_named(name);
            let early: Vec<Timestamp> = all.iter().copied().filter(|d| *d < slot).collect();
            let late: Vec<Timestamp> = all.iter().copied().filter(|d| *d >= slot).collect();
            if !early.is_empty() {
                head_master.append(form.list_property(name, &sorted(early)));
            }
            if !late.is_empty() {
                tail_master.append(form.list_property(name, &sorted(late)));
            }
        }
        tail_master.set(ICalProperty::from_text("UID", new_uid, Vec::new()));
        let end: Timestamp = if timing.is_all_day {
            AllDay::start_of_day(
                &AllDay::date_of(slot, &form.zone).adding_days(timing.all_day_length),
                &form.zone,
            )
            .unwrap_or(slot)
        } else {
            add_seconds(slot, seconds_between(timing.start, timing.end))
        };
        tail_master.set(form.property("DTSTART", slot));
        tail_master.set(form.property("DTEND", end));
        tail_master.remove_properties_named("DURATION");
        tail_master.set(ICalProperty::with_value("SEQUENCE", "0"));
        tail_master.set(ICalProperty::with_value(
            "CREATED",
            ICalValues::utc_text(now),
        ));
        EventWriter::touch(&mut tail_master, now, false);
        EventWriter::touch(&mut head_master, now, true);

        let mut head_overrides: Vec<ICalComponent> = Vec::new();
        let mut tail_overrides: Vec<ICalComponent> = Vec::new();
        for vevent in resource.overrides() {
            let Some(id) = EventReader::recurrence_id_of(vevent, &resolver, calendar_zone) else {
                continue;
            };
            if id < slot {
                head_overrides.push(vevent.clone());
            } else {
                let mut moved = vevent.clone();
                moved.set(ICalProperty::from_text("UID", new_uid, Vec::new()));
                tail_overrides.push(moved);
            }
        }
        let mut head = resource.clone();
        head.set_events(std::iter::once(head_master).chain(head_overrides).collect());
        let mut tail = resource.clone();
        tail.set_events(std::iter::once(tail_master).chain(tail_overrides).collect());
        Ok((head, tail))
    }

    // MARK: Helpers

    /// `date` moved by `delta`; for an all-day form by whole days, so a daylight-saving change between the two dates
    /// cannot land the result on the evening before.
    pub(crate) fn moved(date: Timestamp, delta: f64, form: &Form) -> Timestamp {
        if !form.is_all_day {
            return add_seconds(date, delta);
        }
        let days = (delta / 86_400.0).round() as i64;
        AllDay::start_of_day(
            &AllDay::date_of(date, &form.zone).adding_days(days),
            &form.zone,
        )
        .unwrap_or_else(|| add_seconds(date, delta))
    }

    pub(crate) fn dates(
        properties: &[&ICalProperty],
        resolver: &TimeZoneResolver,
        form: &Form,
    ) -> Vec<Timestamp> {
        properties
            .iter()
            .flat_map(|p| ICalValues::date_values(p))
            .filter_map(|value| resolver.date(&value, &form.zone))
            .collect()
    }

    /// Whether `slot` is an occurrence of the master's set (EXDATEs applied).
    pub(crate) fn is_slot(slot: Timestamp, resource: &EventResource, calendar_zone: &Zone) -> bool {
        let resolver = resource.resolver();
        let Some(master) = resource.master() else {
            return false;
        };
        let Some(timing) = EventReader::timing_of(master, &resolver, calendar_zone) else {
            return false;
        };
        let Some(set) =
            EventReader::recurrence_set_of(master, &timing.zone, timing.is_all_day, &resolver)
        else {
            return false;
        };
        set.occurrences(
            timing.start,
            0.0,
            &timing.zone,
            timing.is_all_day,
            &DateInterval::new(slot, add_seconds(slot, 1.0)),
            2,
        )
        .starts
        .contains(&slot)
    }

    /// A slot of the set, or the slot of an existing override (a moved occurrence is still a split point).
    pub(crate) fn is_slot_or_override(
        slot: Timestamp,
        resource: &EventResource,
        calendar_zone: &Zone,
    ) -> bool {
        let resolver = resource.resolver();
        SeriesEditor::is_slot(slot, resource, calendar_zone)
            || resource
                .overrides()
                .into_iter()
                .any(|o| EventReader::recurrence_id_of(o, &resolver, calendar_zone) == Some(slot))
    }
}
