//! Port of `GraphWriteMapper.swift`: pure translation of the library's write types into Graph JSON. Nothing here does
//! I/O, so every rule is testable. Anything Graph cannot store is `WriteError::Unsupported`; nothing is silently
//! dropped.
//!
//! Swift's `[String: Any]` is [`Json`], a `serde_json::Map` (keys come out sorted, as `.sortedKeys` writes them).

use calendar_core::all_day::{AllDay, CalendarDate};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::model::{
    AttendeeRole, Availability, Reminder, ReminderType, ResponseStatus, Visibility,
};
use calendar_core::time::Zone;
use calendar_core::windows_time_zones::WindowsTimeZones;
use calendar_core::write::event_draft::{AttendeeDraft, EventDraft, EventTiming};
use calendar_core::write::event_patch::{AttendeeChanges, EventPatch, FieldUpdate};
use calendar_core::write::types::{ConferenceChange, ConferenceRequest, EventField, WriteError};
use serde_json::{Value, json};

use crate::graph_time::GraphTime;
use crate::recurrence_mapper::{GraphRecurrenceMapper, Json};

/// Where a series starts, for turning a recurrence rule into Graph's `patternedRecurrence`: the first occurrence's
/// date, read in the zone the rule is anchored to.
#[derive(Clone, Debug)]
pub(crate) struct RecurrenceAnchor {
    pub start: CalendarDate,
    pub zone: Zone,
}

impl RecurrenceAnchor {
    /// For validating a patch before the real anchor is known; the mapping errors do not depend on it.
    pub(crate) fn placeholder() -> RecurrenceAnchor {
        RecurrenceAnchor {
            start: CalendarDate::new(2000, 1, 3),
            zone: Zone::utc(),
        }
    }
}

pub(crate) enum GraphWriteMapper {}

fn unsupported(field: EventField) -> WriteError {
    WriteError::Unsupported {
        fields: [field].into(),
    }
}

impl GraphWriteMapper {
    /// The request body as JSON text with sorted keys. (Swift's `JSONSerialization` throws for an invalid object; a
    /// `serde_json::Value` cannot be one.)
    pub(crate) fn data(json: &Json) -> Outcome<Vec<u8>> {
        serde_json::to_vec(json).map_err(|_| {
            Failure::Write(WriteError::Invalid(
                "the request body is not valid JSON".to_string(),
            ))
        })
    }

    // MARK: Create

    pub(crate) fn create_body(draft: &EventDraft) -> Result<Json, WriteError> {
        draft.validate()?;
        let mut body = Json::new();
        body.insert("subject".to_string(), json!(draft.title));
        body.extend(Self::time_fields(&draft.timing)?);
        if let Some(notes) = &draft.notes {
            body.insert("body".to_string(), Value::Object(Self::text_body(notes)));
        }
        if let Some(location) = &draft.location {
            body.insert("location".to_string(), json!({"displayName": location}));
        }
        body.insert(
            "showAs".to_string(),
            json!(Self::show_as(draft.availability)),
        );
        body.insert(
            "sensitivity".to_string(),
            json!(Self::sensitivity(draft.visibility)),
        );
        if let Some(reminders) = &draft.reminders {
            body.extend(Self::reminder_fields(reminders)?);
        }
        if !draft.attendees.is_empty() {
            body.insert(
                "attendees".to_string(),
                Value::Array(
                    draft
                        .attendees
                        .iter()
                        .map(|attendee| Value::Object(Self::attendee(attendee)))
                        .collect(),
                ),
            );
        }
        if draft.conference == ConferenceRequest::Generate {
            body.extend(Self::teams_fields());
        }
        if let Some(rule) = &draft.recurrence {
            let zone = draft.timing.time_zone.clone().unwrap_or_else(Zone::utc);
            body.insert(
                "recurrence".to_string(),
                Value::Object(GraphRecurrenceMapper::recurrence(
                    rule,
                    &AllDay::date_of(draft.timing.start, &zone),
                    &zone,
                )?),
            );
        }
        Ok(body)
    }

    // MARK: Update

    /// The body of a PATCH: only the fields the patch touches. `current_attendees` is the event's attendee list as
    /// Graph returned it (Graph replaces the whole list, so an edit starts from it); `anchor` is where the series
    /// starts, used only when the patch sets a recurrence.
    pub(crate) fn patch_body(
        patch: &EventPatch,
        current_attendees: &[Json],
        anchor: &RecurrenceAnchor,
    ) -> Result<Json, WriteError> {
        let mut body = Json::new();
        if let Some(title) = &patch.title {
            body.insert("subject".to_string(), json!(title));
        }
        match &patch.notes {
            FieldUpdate::Keep => {}
            FieldUpdate::Set(notes) => {
                body.insert("body".to_string(), Value::Object(Self::text_body(notes)));
            }
            FieldUpdate::Clear => {
                body.insert("body".to_string(), Value::Object(Self::text_body("")));
            }
        }
        match &patch.location {
            FieldUpdate::Keep => {}
            FieldUpdate::Set(location) => {
                body.insert("location".to_string(), json!({"displayName": location}));
            }
            FieldUpdate::Clear => {
                body.insert("location".to_string(), json!({"displayName": ""}));
            }
        }
        if let Some(timing) = &patch.timing {
            timing.validate()?;
            body.extend(Self::time_fields(timing)?);
        }
        if let Some(availability) = patch.availability {
            body.insert("showAs".to_string(), json!(Self::show_as(availability)));
        }
        if let Some(visibility) = patch.visibility {
            body.insert(
                "sensitivity".to_string(),
                json!(Self::sensitivity(visibility)),
            );
        }
        match &patch.reminders {
            FieldUpdate::Keep => {}
            FieldUpdate::Set(reminders) => body.extend(Self::reminder_fields(reminders)?),
            // Graph has no "calendar default" to fall back to.
            FieldUpdate::Clear => return Err(unsupported(EventField::Reminders)),
        }
        if let Some(changes) = &patch.attendees
            && !changes.is_empty()
        {
            body.insert(
                "attendees".to_string(),
                Value::Array(
                    Self::attendee_list(current_attendees, changes)
                        .into_iter()
                        .map(Value::Object)
                        .collect(),
                ),
            );
        }
        match patch.conference {
            None => {}
            Some(ConferenceChange::Generate) => body.extend(Self::teams_fields()),
            Some(ConferenceChange::Remove) => {
                body.insert("isOnlineMeeting".to_string(), json!(false));
            }
        }
        match &patch.recurrence {
            FieldUpdate::Keep => {}
            FieldUpdate::Set(rule) => {
                body.insert(
                    "recurrence".to_string(),
                    Value::Object(GraphRecurrenceMapper::recurrence(
                        rule,
                        &anchor.start,
                        &anchor.zone,
                    )?),
                );
            }
            // Turning a series into a single event is not offered.
            FieldUpdate::Clear => return Err(unsupported(EventField::Recurrence)),
        }
        Ok(body)
    }

    // MARK: Respond

    /// The action segment of `POST .../events/{id}/<action>`.
    pub(crate) fn respond_action(response: ResponseStatus) -> Result<&'static str, WriteError> {
        match response {
            ResponseStatus::Accepted => Ok("accept"),
            ResponseStatus::Tentative => Ok("tentativelyAccept"),
            ResponseStatus::Declined => Ok("decline"),
            ResponseStatus::NeedsAction => Err(WriteError::Invalid(
                "a response must be accepted, tentative or declined".to_string(),
            )),
        }
    }

    // MARK: Pieces

    /// `start`, `end` and `isAllDay`. A timed event is written in its zone's Windows name; a zone with no Windows
    /// name is written in UTC with the same instants. An all-day event is written as midnight of its first day and of
    /// the day after its last, in the zone's Windows name (or UTC when there is none, which keeps the dates).
    pub(crate) fn time_fields(timing: &EventTiming) -> Result<Json, WriteError> {
        let mut fields = Json::new();
        if timing.is_all_day {
            let Some(zone) = &timing.time_zone else {
                return Err(WriteError::Invalid(
                    "an all-day event needs a time zone".to_string(),
                ));
            };
            let name =
                WindowsTimeZones::windows_name_for(zone).unwrap_or_else(|| "UTC".to_string());
            let first = AllDay::date_of(timing.start, zone);
            let after = AllDay::date_of(timing.end, zone);
            fields.insert("isAllDay".to_string(), json!(true));
            fields.insert(
                "start".to_string(),
                json!({"dateTime": GraphTime::midnight(&first), "timeZone": name}),
            );
            fields.insert(
                "end".to_string(),
                json!({"dateTime": GraphTime::midnight(&after), "timeZone": name}),
            );
            return Ok(fields);
        }
        let zone = timing.time_zone.clone().unwrap_or_else(Zone::utc);
        let (written_zone, name) = match WindowsTimeZones::windows_name_for(&zone) {
            Some(name) => (zone, name),
            None => (Zone::utc(), "UTC".to_string()),
        };
        fields.insert("isAllDay".to_string(), json!(false));
        fields.insert(
            "start".to_string(),
            json!({"dateTime": GraphTime::format(timing.start, &written_zone), "timeZone": name}),
        );
        fields.insert(
            "end".to_string(),
            json!({"dateTime": GraphTime::format(timing.end, &written_zone), "timeZone": name}),
        );
        Ok(fields)
    }

    pub(crate) fn text_body(text: &str) -> Json {
        let mut body = Json::new();
        body.insert("contentType".to_string(), json!("text"));
        body.insert("content".to_string(), json!(text));
        body
    }

    pub(crate) fn show_as(availability: Availability) -> &'static str {
        match availability {
            Availability::Busy => "busy",
            Availability::Free => "free",
            Availability::Tentative => "tentative",
            Availability::Unavailable => "oof",
        }
    }

    pub(crate) fn sensitivity(visibility: Visibility) -> &'static str {
        match visibility {
            Visibility::Default | Visibility::PublicEvent => "normal",
            Visibility::PrivateEvent => "private",
            Visibility::Confidential => "confidential",
        }
    }

    /// Graph keeps one reminder per event. An empty list turns it off; one plain on-screen reminder before the start
    /// turns it on; anything else (several, a repeat, an email, one counted from the end) is refused.
    pub(crate) fn reminder_fields(reminders: &[Reminder]) -> Result<Json, WriteError> {
        let mut fields = Json::new();
        if reminders.is_empty() {
            fields.insert("isReminderOn".to_string(), json!(false));
            return Ok(fields);
        }
        let [reminder] = reminders else {
            return Err(unsupported(EventField::Reminders));
        };
        let Some(minutes) = reminder.minutes_before().filter(|minutes| *minutes >= 0) else {
            return Err(unsupported(EventField::Reminders));
        };
        if reminder.kind != ReminderType::Display || reminder.repeat_count != 0 {
            return Err(unsupported(EventField::Reminders));
        }
        fields.insert("isReminderOn".to_string(), json!(true));
        fields.insert("reminderMinutesBeforeStart".to_string(), json!(minutes));
        Ok(fields)
    }

    /// Asks Graph for a Teams meeting. (Personal accounts may not support it; that comes back as an error.)
    pub(crate) fn teams_fields() -> Json {
        let mut fields = Json::new();
        fields.insert("isOnlineMeeting".to_string(), json!(true));
        fields.insert(
            "onlineMeetingProvider".to_string(),
            json!("teamsForBusiness"),
        );
        fields
    }

    fn role_name(role: AttendeeRole) -> &'static str {
        match role {
            AttendeeRole::Required => "required",
            AttendeeRole::Optional => "optional",
            AttendeeRole::Resource => "resource",
        }
    }

    pub(crate) fn attendee(draft: &AttendeeDraft) -> Json {
        let mut address = Json::new();
        address.insert("address".to_string(), json!(draft.email()));
        if let Some(name) = &draft.name {
            address.insert("name".to_string(), json!(name));
        }
        let mut item = Json::new();
        item.insert("emailAddress".to_string(), Value::Object(address));
        item.insert("type".to_string(), json!(Self::role_name(draft.role)));
        item
    }

    fn address_of(item: &Json) -> Option<String> {
        item.get("emailAddress")
            .and_then(|email| email.get("address"))
            .and_then(Value::as_str)
            .map(str::to_lowercase)
    }

    /// The attendee list after `changes`: the current people (their responses stay on the server), minus those
    /// removed, with each added person inserted or updated by email. Only the address, name and type are sent.
    pub(crate) fn attendee_list(current: &[Json], changes: &AttendeeChanges) -> Vec<Json> {
        let mut list: Vec<Json> = current
            .iter()
            .filter_map(|item| {
                let email = Self::address_of(item)?;
                if changes.remove.contains(&email) {
                    return None;
                }
                let mut kept = Json::new();
                kept.insert(
                    "emailAddress".to_string(),
                    item.get("emailAddress").cloned().unwrap_or(Value::Null),
                );
                kept.insert(
                    "type".to_string(),
                    item.get("type")
                        .cloned()
                        .unwrap_or_else(|| json!("required")),
                );
                Some(kept)
            })
            .collect();
        for draft in &changes.add {
            let existing = list
                .iter()
                .position(|item| Self::address_of(item).as_deref() == Some(draft.email()));
            match existing {
                Some(index) => {
                    list[index].insert("type".to_string(), json!(Self::role_name(draft.role)));
                    if let Some(name) = &draft.name
                        && let Some(Value::Object(email_address)) =
                            list[index].get_mut("emailAddress")
                    {
                        email_address.insert("name".to_string(), json!(name));
                    }
                }
                None => list.push(Self::attendee(draft)),
            }
        }
        list
    }

    // MARK: Splitting a series

    /// The fields of a series master that the series which continues after a split keeps: text, availability,
    /// sensitivity, reminder, guests and a Teams meeting. Times and recurrence are set by the caller.
    pub(crate) fn new_series_base(master: &Json) -> Json {
        let mut base = Json::new();
        for key in [
            "subject",
            "showAs",
            "sensitivity",
            "isReminderOn",
            "reminderMinutesBeforeStart",
        ] {
            if let Some(value) = master.get(key) {
                base.insert(key.to_string(), value.clone());
            }
        }
        if let Some(content) = master
            .get("body")
            .and_then(|body| body.get("content"))
            .and_then(Value::as_str)
            .filter(|content| !content.is_empty())
        {
            base.insert("body".to_string(), Value::Object(Self::text_body(content)));
        }
        if let Some(name) = master
            .get("location")
            .and_then(|location| location.get("displayName"))
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
        {
            base.insert("location".to_string(), json!({"displayName": name}));
        }
        // `master["attendees"] as? [[String: Any]]`: one element that is not an object makes the whole list absent.
        let attendees: Option<Vec<Json>> = master
            .get("attendees")
            .and_then(Value::as_array)
            .and_then(|list| list.iter().map(|item| item.as_object().cloned()).collect());
        if let Some(attendees) = attendees.filter(|attendees| !attendees.is_empty()) {
            base.insert(
                "attendees".to_string(),
                Value::Array(
                    Self::attendee_list(&attendees, &AttendeeChanges::default())
                        .into_iter()
                        .map(Value::Object)
                        .collect(),
                ),
            );
        }
        // A new meeting is created for the new series (the old link belongs to the old one).
        if master.get("isOnlineMeeting") == Some(&Value::Bool(true)) {
            base.extend(Self::teams_fields());
        }
        base
    }
}
