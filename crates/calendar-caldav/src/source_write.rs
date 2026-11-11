//! Port of `CalDAVCalendarSource+Write.swift`: create, update, delete and respond.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use calendar_core::all_day::{AllDay, CalendarDate};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::model::{CalendarEvent, ResponseStatus};
use calendar_core::recurrence::rule::{RecurrenceRule, RuleEnd};
use calendar_core::source_types::{CalendarSource, SourceError};
use calendar_core::time::{DateInterval, Zone, add_seconds};
use calendar_core::write::event_draft::{EventDraft, EventTiming};
use calendar_core::write::event_patch::{EventPatch, FieldUpdate};
use calendar_core::write::patch_merge::{Attempt, PatchMerge};
use calendar_core::write::types::{
    EventField, EventRef, NotifyPolicy, RecurrenceScope, WriteError, WriteValidation,
};
use calendar_core::write::writable_source::WritableCalendarSource;
use calendar_ical::{
    AlarmMapper, AttendeeMapper, EventReader, EventResource, EventTimingInfo, EventWriter,
    ICalComponent, ICalError, SeriesEditor,
};
use jiff::Timestamp;

use crate::dav_url::DavUrl;
use crate::dav_xml::DavXml;
use crate::source::CalDavCalendarSource;

pub(crate) struct FetchedResource {
    pub resource: EventResource,
    pub etag: String,
    pub url: calendar_core::url_string::UrlString,
    pub name: String,
}

pub(crate) enum PutOutcome {
    Stored {
        etag: String,
        resource: EventResource,
    },
    Stale,
}

pub(crate) fn invalid(message: impl Into<String>) -> Failure {
    Failure::Source(SourceError::InvalidResponse(message.into()))
}

/// Swift throws `ICalError` here, which no `SourceError` covers; the host sees it as its own failure text.
pub(crate) fn ical_failure(error: ICalError) -> Failure {
    Failure::Host(error.to_string())
}

fn unsupported(field: EventField) -> Failure {
    WriteError::Unsupported {
        fields: [field].into(),
    }
    .into()
}

/// Whole days from `a` to `b` (Foundation's `CalendarDate.days(to:)`).
fn days_between(a: &CalendarDate, b: &CalendarDate) -> i64 {
    fn days_from_civil(date: &CalendarDate) -> i64 {
        let (mut y, m, d) = (date.year, date.month, date.day);
        if m <= 2 {
            y -= 1;
        }
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }
    days_from_civil(b) - days_from_civil(a)
}

impl CalDavCalendarSource {
    pub(crate) const SERIES_MESSAGE: &'static str =
        "this is a recurring series; use .allInSeries or read the occurrence first";
    pub(crate) const SLOT_MESSAGE: &'static str = "needs the occurrence's original start";

    // MARK: Plumbing

    /// `name.ics` or `name.ics#20260915T170000Z`: the resource name and whether the id names one occurrence.
    pub(crate) fn resource_name_of(event_id: &str) -> (String, bool) {
        match event_id.find('#') {
            Some(hash) => (event_id[..hash].to_string(), true),
            None => (event_id.to_string(), false),
        }
    }

    /// How far ahead a written `VTIMEZONE` must reach: the rule's `UNTIL`, 20 years for an open or counted rule.
    pub(crate) fn series_end(start: Timestamp, rule: Option<&RecurrenceRule>) -> Timestamp {
        let Some(rule) = rule else {
            return start;
        };
        if let RuleEnd::Until(until) = rule.end {
            return until;
        }
        add_seconds(start, 20.0 * 366.0 * 86_400.0)
    }

    pub(crate) async fn fetch(&self, calendar_id: &str, name: &str) -> Outcome<FetchedResource> {
        let url = self.resource_url(calendar_id, name);
        let reply = self.client.send("GET", &url).await?;
        match reply.response.status {
            200 => {}
            404 | 410 => return Err(WriteError::NotFound.into()),
            403 => return Err(WriteError::Forbidden(None).into()),
            other => return Err(invalid(format!("GET answered {other}"))),
        }
        let etag = reply
            .response
            .header("ETag")
            .ok_or_else(|| invalid("the server sent no ETag"))?
            .to_string();
        let resource = EventResource::from_data(&reply.response.body)
            .map_err(|_| invalid("the event could not be read"))?;
        Ok(FetchedResource {
            resource,
            etag,
            url,
            name: name.to_string(),
        })
    }

    /// PUTs the whole resource; 412 is `Stale`. The ETag comes from the response, or from one GET when the server sends
    /// none (the returned resource is then the server's copy).
    pub(crate) async fn put(
        &self,
        resource: &EventResource,
        url: &calendar_core::url_string::UrlString,
        if_match: Option<&str>,
        if_none_match: bool,
    ) -> Outcome<PutOutcome> {
        let mut headers = BTreeMap::from([(
            "Content-Type".to_string(),
            "text/calendar; charset=utf-8".to_string(),
        )]);
        if let Some(if_match) = if_match {
            headers.insert("If-Match".to_string(), if_match.to_string());
        }
        if if_none_match {
            headers.insert("If-None-Match".to_string(), "*".to_string());
        }
        let reply = self
            .client
            .send_with("PUT", url, headers, Some(resource.serialized()))
            .await?;
        match reply.response.status {
            200 | 201 | 204 => {}
            412 => return Ok(PutOutcome::Stale),
            403 => return Err(WriteError::Forbidden(None).into()),
            404 | 410 => return Err(WriteError::NotFound.into()),
            other => return Err(invalid(format!("PUT answered {other}"))),
        }
        self.state.forget(url);
        if let Some(etag) = reply.response.header("ETag") {
            return Ok(PutOutcome::Stored {
                etag: etag.to_string(),
                resource: resource.clone(),
            });
        }
        let reread = self.client.send("GET", url).await?;
        let unreadable = || invalid("the stored event could not be read back");
        if reread.response.status != 200 {
            return Err(unreadable());
        }
        let etag = reread
            .response
            .header("ETag")
            .ok_or_else(unreadable)?
            .to_string();
        let copy = EventResource::from_data(&reread.response.body).map_err(|_| unreadable())?;
        Ok(PutOutcome::Stored {
            etag,
            resource: copy,
        })
    }

    /// The occurrence at `original_start`, or the master in its series form (a single event when it does not recur).
    pub(crate) async fn read_back(
        &self,
        resource: &EventResource,
        etag: &str,
        calendar_id: &str,
        name: &str,
        original_start: Option<Timestamp>,
    ) -> Outcome<CalendarEvent> {
        let zone = self.calendar_zone(calendar_id).await?;
        let context = self.context(calendar_id, &zone, name, Some(etag.to_string()));
        if let Some(original_start) = original_start
            && let Some(occurrence) = EventReader::occurrence_in(resource, original_start, &context)
        {
            return Ok(occurrence);
        }
        // A resource with overrides but no master (an invite to single occurrences) reads as its first override.
        let everything = DateInterval::new(Timestamp::MIN, Timestamp::MAX);
        EventReader::master_event_of(resource, &context)
            .or_else(|| {
                EventReader::events_in(resource, &everything, &context)
                    .into_iter()
                    .next()
            })
            .ok_or_else(|| WriteError::NotFound.into())
    }

    pub(crate) fn is_recurring(resource: &EventResource) -> bool {
        !resource.overrides().is_empty()
            || resource
                .master()
                .is_some_and(|m| m.property("RRULE").is_some() || m.property("RDATE").is_some())
    }

    pub(crate) fn master_start(
        resource: &EventResource,
        calendar_zone: &Zone,
    ) -> Option<Timestamp> {
        let master = resource.master()?;
        EventReader::timing_of(master, &resource.resolver(), calendar_zone).map(|t| t.start)
    }

    /// Whether the server would tell anyone but the account about a change: any attendee that is not the account, or an
    /// organizer that is not the account (an invite often lists only the account as an attendee, and an attendee's delete
    /// or exception is answered to the organizer). An organizer whose address cannot be read counts as someone else.
    pub(crate) fn tells_others(&self, resource: &EventResource) -> bool {
        let me = self.self_addresses();
        resource.events().into_iter().any(|vevent| {
            let (attendees, organizer) = AttendeeMapper::read(vevent, &me);
            attendees.iter().any(|a| !a.is_self) || organizer.is_some_and(|o| !o.is_self)
        })
    }

    /// Implicit scheduling tells attendees of every change and cannot be stopped (`controls_notifications == false`).
    pub(crate) fn require_notify(notify: NotifyPolicy, tells_others: bool) -> Outcome<()> {
        if notify != NotifyPolicy::All && tells_others {
            return Err(unsupported(EventField::Attendees));
        }
        Ok(())
    }

    /// The event a patch is judged against after a stale write: the occurrence the caller edited when it still exists,
    /// else the master.
    pub(crate) fn compared_event(
        &self,
        fresh: &FetchedResource,
        reference: &EventRef,
        scope: RecurrenceScope,
        calendar_zone: &Zone,
    ) -> Outcome<CalendarEvent> {
        let context = self.context(
            &reference.calendar_id,
            calendar_zone,
            &fresh.name,
            Some(fresh.etag.clone()),
        );
        if Self::is_recurring(&fresh.resource)
            && let Some(slot) = reference.original_start
        {
            let occurrence = EventReader::occurrence_in(&fresh.resource, slot, &context);
            if scope != RecurrenceScope::AllInSeries {
                return occurrence.ok_or_else(|| WriteError::NotFound.into());
            }
            // A whole-series edit is judged against the series master, so another client's change to the series shows up
            // even when the occurrence the caller read has its own values. The caller's base is an occurrence, so the
            // timing stays the occurrence's.
            if let Some(mut master) = EventReader::master_event_of(&fresh.resource, &context) {
                if let Some(occurrence) = occurrence {
                    master.start = occurrence.start;
                    master.end = occurrence.end;
                    master.time_zone = occurrence.time_zone;
                    master.is_all_day = occurrence.is_all_day;
                }
                return Ok(master);
            }
            // A resource of overrides only.
            return occurrence.ok_or_else(|| WriteError::NotFound.into());
        }
        // The id names an occurrence of a series that has since become a single event: that occurrence no longer exists.
        if Self::resource_name_of(&reference.event_id).1 && scope != RecurrenceScope::AllInSeries {
            return Err(WriteError::NotFound.into());
        }
        EventReader::master_event_of(&fresh.resource, &context)
            .ok_or_else(|| WriteError::NotFound.into())
    }

    // MARK: Create

    async fn create_event(
        &self,
        draft: &EventDraft,
        calendar_id: &str,
        notify: NotifyPolicy,
    ) -> Outcome<CalendarEvent> {
        draft.validate()?;
        WriteValidation::require_writable(&draft.used_fields(), &self.capabilities())?;
        Self::require_notify(notify, !draft.attendees.is_empty())?;
        let stamp = (self.now)();
        let uid = draft.uid.clone().unwrap_or_else(|| (self.make_uuid)());
        let vevent =
            EventWriter::vevent_from(draft, &uid, stamp, self.organizer_address().as_deref())?;
        let zone = draft.timing.time_zone.clone().unwrap_or_else(Zone::utc);
        let resource = EventWriter::resource_for(
            vevent,
            &[zone],
            draft.timing.start,
            Self::series_end(draft.timing.start, draft.recurrence.as_ref()),
        )
        .map_err(ical_failure)?;
        if let Some(uid) = &draft.uid {
            self.refuse_duplicate(uid, calendar_id).await?;
        }
        // A 412 means the name is taken: one more try with a new one.
        for _ in 0..2 {
            let name = format!("{}.ics", (self.make_uuid)());
            if let PutOutcome::Stored {
                etag,
                resource: copy,
            } = self
                .put(
                    &resource,
                    &self.resource_url(calendar_id, &name),
                    None,
                    true,
                )
                .await?
            {
                return self.read_back(&copy, &etag, calendar_id, &name, None).await;
            }
        }
        Err(invalid("the server refused every new event name"))
    }

    async fn refuse_duplicate(&self, uid: &str, calendar_id: &str) -> Outcome<()> {
        let reply = self
            .client
            .report(
                &self.calendar_url(calendar_id),
                1,
                DavXml::calendar_query_uid(uid),
            )
            .await?;
        match reply.response.status {
            207 => {}
            403 => return Err(WriteError::Forbidden(None).into()),
            404 | 410 => return Err(WriteError::NotFound.into()),
            other => return Err(invalid(format!("calendar-query answered {other}"))),
        }
        let zone = self.calendar_zone(calendar_id).await?;
        for response in DavXml::multistatus(&reply.response.body)?.responses {
            let Ok(url) = self.client.resolve(&response.href, &reply.url) else {
                continue;
            };
            let Some((resource, etag)) = self.calendar_object(&response, &url) else {
                continue;
            };
            if resource.uid().as_deref() != Some(uid) {
                continue;
            }
            let context = self.context(
                calendar_id,
                &zone,
                &DavUrl::last_path_component(&url),
                Some(etag),
            );
            let everything = DateInterval::new(Timestamp::MIN, Timestamp::MAX);
            if let Some(existing) =
                EventReader::master_event_of(&resource, &context).or_else(|| {
                    EventReader::events_in(&resource, &everything, &context)
                        .into_iter()
                        .next()
                })
            {
                return Err(WriteError::AlreadyExists(Box::new(existing)).into());
            }
        }
        Ok(())
    }

    // MARK: Update

    async fn update_event(
        &self,
        reference: &EventRef,
        patch: &EventPatch,
        scope: RecurrenceScope,
        _notify: NotifyPolicy,
    ) -> Outcome<CalendarEvent> {
        let notify = _notify;
        let (name, is_occurrence) = Self::resource_name_of(&reference.event_id);
        if patch.is_empty() {
            if let Some(base) = patch.base() {
                return Ok(base.clone());
            }
            let current = self.fetch(&reference.calendar_id, &name).await?;
            return self
                .read_back(
                    &current.resource,
                    &current.etag,
                    &reference.calendar_id,
                    &name,
                    if is_occurrence {
                        reference.original_start
                    } else {
                        None
                    },
                )
                .await;
        }
        WriteValidation::require_writable(&patch.touched_fields(), &self.capabilities())?;
        if let Some(timing) = &patch.timing {
            timing.validate()?;
        }
        if let FieldUpdate::Set(rule) = &patch.recurrence {
            rule.validate()?;
        }
        if matches!(patch.reminders, FieldUpdate::Clear) {
            return Err(unsupported(EventField::Reminders));
        }
        if let FieldUpdate::Set(reminders) = &patch.reminders {
            AlarmMapper::alarms_from(reminders)?;
        }
        Self::check_series_ref(reference, scope, is_occurrence, patch.timing.is_some())?;
        let zone = self.calendar_zone(&reference.calendar_id).await?;
        let url = self.resource_url(&reference.calendar_id, &name);
        // The first attempt may use the copy the caller just read, when its version is that copy's ETag.
        let cached: Mutex<Option<FetchedResource>> =
            Mutex::new(reference.version.as_ref().and_then(|version| {
                self.state
                    .resource(&url, version)
                    .map(|copy| FetchedResource {
                        resource: copy,
                        etag: version.clone(),
                        url: url.clone(),
                        name: name.clone(),
                    })
            }));
        let (this, zone, name, cached) = (self, &zone, name.as_str(), &cached);
        PatchMerge::apply(
            patch,
            reference.version.clone(),
            PatchMerge::DEFAULT_MAX_ATTEMPTS,
            move || async move {
                let fresh = this.fetch(&reference.calendar_id, name).await?;
                this.compared_event(&fresh, reference, scope, zone)
            },
            move |version: Option<String>| async move {
                let first = cached.lock().unwrap_or_else(PoisonError::into_inner).take();
                let current = match first {
                    Some(first) => first,
                    None => this.fetch(&reference.calendar_id, name).await?,
                };
                if let Some(version) = &version
                    && *version != current.etag
                {
                    return Ok(Attempt::Stale);
                }
                Self::require_notify(
                    notify,
                    this.tells_others(&current.resource)
                        || patch.attendees.as_ref().is_some_and(|a| !a.add.is_empty()),
                )?;
                if scope == RecurrenceScope::ThisAndFollowing
                    && is_occurrence
                    && Self::is_recurring(&current.resource)
                    && let Some(slot) = reference.original_start
                    && let Some(first) = Self::master_start(&current.resource, zone)
                    && slot > first
                {
                    return this
                        .split_series(&current, reference, patch, slot, zone)
                        .await;
                }
                let mut resource = current.resource.clone();
                let slot =
                    this.edit(&mut resource, reference, patch, scope, is_occurrence, zone)?;
                match this
                    .put(&resource, &current.url, Some(&current.etag), false)
                    .await?
                {
                    PutOutcome::Stale => Ok(Attempt::Stale),
                    PutOutcome::Stored {
                        etag,
                        resource: copy,
                    } => Ok(Attempt::Done(
                        this.read_back(&copy, &etag, &reference.calendar_id, name, slot)
                            .await?,
                    )),
                }
            },
        )
        .await
    }

    /// What can be refused from the ref alone, before any request. A ref with an occurrence id or a series id belongs
    /// to a series; one with neither is a single event, where the scope is ignored.
    pub(crate) fn check_series_ref(
        reference: &EventRef,
        scope: RecurrenceScope,
        is_occurrence: bool,
        moves_time: bool,
    ) -> Outcome<()> {
        if !is_occurrence && reference.series_id().is_none() {
            return Ok(());
        }
        if scope != RecurrenceScope::AllInSeries && !is_occurrence {
            return Err(WriteError::Invalid(Self::SERIES_MESSAGE.to_string()).into());
        }
        if scope != RecurrenceScope::AllInSeries && reference.original_start.is_none() {
            return Err(WriteError::Invalid(Self::SLOT_MESSAGE.to_string()).into());
        }
        // Moving the whole series from one occurrence needs that occurrence's slot to know how far it moved.
        if scope == RecurrenceScope::AllInSeries && moves_time && reference.original_start.is_none()
        {
            return Err(unsupported(EventField::Timing));
        }
        Ok(())
    }

    /// Applies the patch in place and returns the occurrence to read back (`None`: the master or single event).
    pub(crate) fn edit(
        &self,
        resource: &mut EventResource,
        reference: &EventRef,
        patch: &EventPatch,
        scope: RecurrenceScope,
        is_occurrence: bool,
        calendar_zone: &Zone,
    ) -> Outcome<Option<Timestamp>> {
        let result = self.edit_inner(
            resource,
            reference,
            patch,
            scope,
            is_occurrence,
            calendar_zone,
        );
        // Swift's `defer`: the time zone of a new timing gets its `VTIMEZONE`.
        if let Some(timing) = &patch.timing
            && let Some(zone) = &timing.time_zone
        {
            resource.ensure_time_zones(
                std::slice::from_ref(zone),
                timing.start,
                add_seconds(timing.start, 20.0 * 366.0 * 86_400.0),
            );
        }
        result
    }

    fn edit_inner(
        &self,
        resource: &mut EventResource,
        reference: &EventRef,
        patch: &EventPatch,
        scope: RecurrenceScope,
        is_occurrence: bool,
        calendar_zone: &Zone,
    ) -> Outcome<Option<Timestamp>> {
        let stamp = (self.now)();
        let organizer = self.organizer_address();
        if !Self::is_recurring(resource) {
            // The id names an occurrence of a series that has since become a single event (as in `delete`).
            if is_occurrence && scope != RecurrenceScope::AllInSeries {
                return Err(WriteError::NotFound.into());
            }
            let mut master = resource.master().cloned().ok_or(WriteError::NotFound)?;
            EventWriter::apply(
                patch,
                &mut master,
                stamp,
                organizer.as_deref(),
                Some(calendar_zone),
            )?;
            resource.set_events(vec![master]);
            return Ok(None);
        }
        if scope != RecurrenceScope::AllInSeries && !is_occurrence {
            return Err(WriteError::Invalid(Self::SERIES_MESSAGE.to_string()).into());
        }
        let slot_of_ref = reference.original_start;
        if scope != RecurrenceScope::AllInSeries && slot_of_ref.is_none() {
            return Err(WriteError::Invalid(Self::SLOT_MESSAGE.to_string()).into());
        }
        if scope == RecurrenceScope::ThisInstance {
            let slot =
                slot_of_ref.ok_or_else(|| WriteError::Invalid(Self::SLOT_MESSAGE.to_string()))?;
            if patch.recurrence != FieldUpdate::Keep {
                return Err(unsupported(EventField::Recurrence));
            }
            let mut over = SeriesEditor::override_in(resource, slot, calendar_zone)
                .ok_or(WriteError::NotFound)?;
            EventWriter::apply(
                patch,
                &mut over,
                stamp,
                organizer.as_deref(),
                Some(calendar_zone),
            )?;
            SeriesEditor::set_override(over, slot, resource, calendar_zone);
            return Ok(Some(slot));
        }
        if resource.master().is_none() {
            // A resource of overrides only (an invite to single occurrences): there is no series to edit, so a
            // whole-series edit applies to each occurrence. One timing cannot describe several occurrences.
            if scope != RecurrenceScope::AllInSeries || patch.recurrence != FieldUpdate::Keep {
                return Err(unsupported(EventField::Recurrence));
            }
            if patch.timing.is_some() && resource.overrides().len() > 1 {
                return Err(unsupported(EventField::Timing));
            }
            let mut events: Vec<ICalComponent> =
                resource.overrides().into_iter().cloned().collect();
            for event in &mut events {
                EventWriter::apply(
                    patch,
                    event,
                    stamp,
                    organizer.as_deref(),
                    Some(calendar_zone),
                )?;
            }
            resource.set_events(events);
            return Ok(None);
        }
        // .allInSeries, or .thisAndFollowing at the first occurrence.
        let mut master = resource.master().cloned().ok_or(WriteError::NotFound)?;
        let timing = EventReader::timing_of(&master, &resource.resolver(), calendar_zone)
            .ok_or(WriteError::NotFound)?;
        let mut master_patch = patch.clone();
        let mut delta = 0.0;
        if let Some(wanted) = &patch.timing {
            // The caller moved one occurrence; the series moves by the same amount, which needs that occurrence's slot.
            let slot = slot_of_ref.ok_or_else(|| unsupported(EventField::Timing))?;
            let has_exceptions =
                !resource.overrides().is_empty() || master.property("EXDATE").is_some();
            if wanted.is_all_day != timing.is_all_day && has_exceptions {
                return Err(unsupported(EventField::Timing));
            }
            let (moved, shift) = Self::series_timing(&timing, slot, wanted, calendar_zone);
            master_patch.timing = Some(moved);
            delta = shift;
        }
        EventWriter::apply(
            &master_patch,
            &mut master,
            stamp,
            organizer.as_deref(),
            Some(calendar_zone),
        )?;
        let mut events = vec![master];
        events.extend(resource.overrides().into_iter().cloned());
        resource.set_events(events);
        if delta != 0.0 {
            SeriesEditor::shift(resource, delta, calendar_zone);
        }
        if patch.recurrence != FieldUpdate::Keep {
            SeriesEditor::prune_unmatched(resource, calendar_zone);
        }
        Ok(None)
    }

    /// The master's new timing when the occurrence at `slot` moves to `wanted`: the same shift (whole days for all-day)
    /// and `wanted`'s length and zone.
    pub(crate) fn series_timing(
        master: &EventTimingInfo,
        slot: Timestamp,
        wanted: &EventTiming,
        calendar_zone: &Zone,
    ) -> (EventTiming, f64) {
        if !wanted.is_all_day {
            let delta = calendar_core::time::seconds_between(slot, wanted.start);
            let start = add_seconds(master.start, delta);
            let length = calendar_core::time::seconds_between(wanted.start, wanted.end);
            return (
                EventTiming::new(
                    start,
                    add_seconds(start, length),
                    wanted.time_zone.clone(),
                    false,
                ),
                delta,
            );
        }
        let zone = wanted
            .time_zone
            .clone()
            .unwrap_or_else(|| calendar_zone.clone());
        let days = days_between(
            &AllDay::date_of(slot, &zone),
            &AllDay::date_of(wanted.start, &zone),
        );
        let length = days_between(
            &AllDay::date_of(wanted.start, &zone),
            &AllDay::date_of(wanted.end, &zone),
        );
        let first = AllDay::date_of(master.start, &zone).adding_days(days);
        let start = AllDay::start_of_day(&first, &zone).unwrap_or(master.start);
        let end = AllDay::start_of_day(&first.adding_days(length), &zone).unwrap_or(start);
        (
            EventTiming::new(start, end, Some(zone), true),
            days as f64 * 86_400.0,
        )
    }

    // MARK: Delete

    async fn delete_event(
        &self,
        reference: &EventRef,
        scope: RecurrenceScope,
        notify: NotifyPolicy,
    ) -> Outcome<()> {
        let (name, is_occurrence) = Self::resource_name_of(&reference.event_id);
        Self::check_series_ref(reference, scope, is_occurrence, false)?;
        let zone = self.calendar_zone(&reference.calendar_id).await?;
        // An occurrence delete is idempotent and local to that occurrence, so a 412 (another occurrence changed) is
        // re-applied to the fresh copy. Every attempt judges the fresh copy again: who would be told, and whether the
        // occurrence still exists. From the second attempt on, a resource or occurrence that is gone is the goal reached.
        for attempt in 0..3 {
            let outcome = async {
                let current = self.fetch(&reference.calendar_id, &name).await?;
                self.delete_attempt(&current, reference, scope, is_occurrence, notify, &zone)
                    .await
            }
            .await;
            match outcome {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(Failure::Write(WriteError::NotFound)) if attempt > 0 => return Ok(()),
                Err(error) => return Err(error),
            }
        }
        Err(WriteError::Conflict {
            fields: [EventField::Recurrence].into(),
        }
        .into())
    }

    /// One delete against `current`: true when done, false when the server said the copy is stale.
    async fn delete_attempt(
        &self,
        current: &FetchedResource,
        reference: &EventRef,
        scope: RecurrenceScope,
        is_occurrence: bool,
        notify: NotifyPolicy,
        zone: &Zone,
    ) -> Outcome<bool> {
        Self::require_notify(notify, self.tells_others(&current.resource))?;
        let recurring = Self::is_recurring(&current.resource);
        // The id names an occurrence of a series that has since become a single event: that occurrence no longer exists.
        if is_occurrence && !recurring && scope != RecurrenceScope::AllInSeries {
            return Err(WriteError::NotFound.into());
        }
        let mut effective = if recurring {
            scope
        } else {
            RecurrenceScope::AllInSeries
        };
        if effective != RecurrenceScope::AllInSeries {
            if !is_occurrence {
                return Err(WriteError::Invalid(Self::SERIES_MESSAGE.to_string()).into());
            }
            let Some(slot) = reference.original_start else {
                return Err(WriteError::Invalid(Self::SLOT_MESSAGE.to_string()).into());
            };
            if effective == RecurrenceScope::ThisAndFollowing
                && let Some(first) = Self::master_start(&current.resource, zone)
                && slot <= first
            {
                effective = RecurrenceScope::AllInSeries;
            }
        }
        if effective == RecurrenceScope::AllInSeries {
            // Last writer wins, like the other connectors' deletes.
            return self.remove(&current.url, None).await;
        }
        let slot = reference
            .original_start
            .ok_or_else(|| WriteError::Invalid(Self::SLOT_MESSAGE.to_string()))?;
        let mut resource = current.resource.clone();
        if resource.master().is_none() {
            // A resource of overrides only: drop the occurrence (and, for this and following, the later ones).
            let resolver = resource.resolver();
            let remaining: Vec<ICalComponent> = resource
                .overrides()
                .into_iter()
                .filter(
                    |vevent| match EventReader::recurrence_id_of(vevent, &resolver, zone) {
                        None => true,
                        Some(id) => {
                            if effective == RecurrenceScope::ThisInstance {
                                id != slot
                            } else {
                                id < slot
                            }
                        }
                    },
                )
                .cloned()
                .collect();
            if remaining.len() == resource.overrides().len() {
                return Err(WriteError::NotFound.into());
            }
            if remaining.is_empty() {
                return self.remove(&current.url, Some(&current.etag)).await;
            }
            resource.set_events(remaining);
        } else if effective == RecurrenceScope::ThisInstance {
            SeriesEditor::exclude(slot, &mut resource, zone)?;
            if let Some(mut master) = resource.master().cloned() {
                EventWriter::touch(&mut master, (self.now)(), true);
                let mut events = vec![master];
                events.extend(resource.overrides().into_iter().cloned());
                resource.set_events(events);
            }
        } else {
            resource =
                SeriesEditor::split(&resource, slot, &(self.make_uuid)(), zone, (self.now)())?.0;
        }
        Ok(matches!(
            self.put(&resource, &current.url, Some(&current.etag), false)
                .await?,
            PutOutcome::Stored { .. }
        ))
    }

    /// DELETE; false on a 412 (only when `if_match` was sent).
    async fn remove(
        &self,
        url: &calendar_core::url_string::UrlString,
        if_match: Option<&str>,
    ) -> Outcome<bool> {
        let headers: BTreeMap<String, String> = if_match
            .map(|m| BTreeMap::from([("If-Match".to_string(), m.to_string())]))
            .unwrap_or_default();
        let reply = self.client.send_with("DELETE", url, headers, None).await?;
        match reply.response.status {
            200 | 204 => {
                self.state.forget(url);
                Ok(true)
            }
            412 => Ok(false),
            404 | 410 => Err(WriteError::NotFound.into()),
            403 => Err(WriteError::Forbidden(None).into()),
            other => Err(invalid(format!("DELETE answered {other}"))),
        }
    }

    // MARK: Respond

    async fn respond_event(
        &self,
        reference: &EventRef,
        response: ResponseStatus,
        scope: RecurrenceScope,
        notify: NotifyPolicy,
    ) -> Outcome<CalendarEvent> {
        if response == ResponseStatus::NeedsAction {
            return Err(WriteError::Invalid(
                "a response must be accepted, tentative or declined".to_string(),
            )
            .into());
        }
        if !self.capabilities().can_respond_to_invite {
            return Err(unsupported(EventField::Attendees));
        }
        // The organizer is always told of an answer.
        if notify != NotifyPolicy::All {
            return Err(unsupported(EventField::Attendees));
        }
        let (name, is_occurrence) = Self::resource_name_of(&reference.event_id);
        // An attendee cannot split the organizer's series.
        if scope == RecurrenceScope::ThisAndFollowing
            && (is_occurrence || reference.series_id().is_some())
        {
            return Err(unsupported(EventField::Attendees));
        }
        let zone = self.calendar_zone(&reference.calendar_id).await?;
        let (this, zone, name) = (self, &zone, name.as_str());
        let empty = EventPatch::default();
        PatchMerge::apply(
            &empty,
            reference.version.clone(),
            PatchMerge::DEFAULT_MAX_ATTEMPTS,
            move || async move {
                let fresh = this.fetch(&reference.calendar_id, name).await?;
                this.compared_event(&fresh, reference, scope, zone)
            },
            move |version: Option<String>| async move {
                let current = this.fetch(&reference.calendar_id, name).await?;
                if let Some(version) = &version
                    && *version != current.etag
                {
                    return Ok(Attempt::Stale);
                }
                let mut resource = current.resource.clone();
                let slot = this.set_response(
                    response,
                    &mut resource,
                    reference,
                    scope,
                    is_occurrence,
                    zone,
                )?;
                match this
                    .put(&resource, &current.url, Some(&current.etag), false)
                    .await?
                {
                    PutOutcome::Stale => Ok(Attempt::Stale),
                    PutOutcome::Stored {
                        etag,
                        resource: copy,
                    } => Ok(Attempt::Done(
                        this.read_back(&copy, &etag, &reference.calendar_id, name, slot)
                            .await?,
                    )),
                }
            },
        )
        .await
    }

    fn set_response(
        &self,
        response: ResponseStatus,
        resource: &mut EventResource,
        reference: &EventRef,
        scope: RecurrenceScope,
        is_occurrence: bool,
        calendar_zone: &Zone,
    ) -> Outcome<Option<Timestamp>> {
        let stamp = (self.now)();
        let me = self.self_addresses();
        if !Self::is_recurring(resource) {
            // The id names an occurrence of a series that has since become a single event (as in `delete`).
            if is_occurrence && scope != RecurrenceScope::AllInSeries {
                return Err(WriteError::NotFound.into());
            }
            let mut master = resource
                .master()
                .cloned()
                .filter(|_| true)
                .ok_or_else(|| unsupported(EventField::Attendees))?;
            if !AttendeeMapper::set_response(response, &mut master, &me) {
                return Err(unsupported(EventField::Attendees));
            }
            EventWriter::touch(&mut master, stamp, false);
            resource.set_events(vec![master]);
            return Ok(None);
        }
        match scope {
            RecurrenceScope::ThisAndFollowing => Err(unsupported(EventField::Attendees)),
            RecurrenceScope::ThisInstance => {
                if !is_occurrence {
                    return Err(WriteError::Invalid(Self::SERIES_MESSAGE.to_string()).into());
                }
                let slot = reference
                    .original_start
                    .ok_or_else(|| WriteError::Invalid(Self::SLOT_MESSAGE.to_string()))?;
                let mut over = SeriesEditor::override_in(resource, slot, calendar_zone)
                    .ok_or(WriteError::NotFound)?;
                if !AttendeeMapper::set_response(response, &mut over, &me) {
                    return Err(unsupported(EventField::Attendees));
                }
                EventWriter::touch(&mut over, stamp, false);
                SeriesEditor::set_override(over, slot, resource, calendar_zone);
                Ok(Some(slot))
            }
            RecurrenceScope::AllInSeries => {
                // The answer is for the whole series: the master and every override.
                let mut events: Vec<ICalComponent> =
                    resource.events().into_iter().cloned().collect();
                let mut found = false;
                for event in &mut events {
                    if AttendeeMapper::set_response(response, event, &me) {
                        EventWriter::touch(event, stamp, false);
                        found = true;
                    }
                }
                if !found {
                    return Err(unsupported(EventField::Attendees));
                }
                resource.set_events(events);
                Ok(None)
            }
        }
    }
}

#[async_trait]
impl WritableCalendarSource for CalDavCalendarSource {
    async fn create(
        &self,
        draft: &EventDraft,
        calendar_id: &str,
        notify: NotifyPolicy,
    ) -> Outcome<CalendarEvent> {
        self.create_event(draft, calendar_id, notify).await
    }

    async fn update(
        &self,
        reference: &EventRef,
        patch: &EventPatch,
        scope: RecurrenceScope,
        notify: NotifyPolicy,
    ) -> Outcome<CalendarEvent> {
        self.update_event(reference, patch, scope, notify).await
    }

    async fn delete(
        &self,
        reference: &EventRef,
        scope: RecurrenceScope,
        notify: NotifyPolicy,
    ) -> Outcome<()> {
        self.delete_event(reference, scope, notify).await
    }

    async fn respond(
        &self,
        reference: &EventRef,
        response: ResponseStatus,
        scope: RecurrenceScope,
        notify: NotifyPolicy,
    ) -> Outcome<CalendarEvent> {
        self.respond_event(reference, response, scope, notify).await
    }
}
