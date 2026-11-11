//! Port of `GoogleWriteMapper.swift`: pure conversion between the library's write types and Google's event JSON. No
//! I/O. Swift's `[String: Any]` is a `serde_json::Map` (keys sorted, `NSNull` is `Value::Null`).

use std::collections::BTreeSet;

use calendar_core::all_day::{AllDay, CalendarDate};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::model::{Availability, Reminder, ReminderType, ResponseStatus, Visibility};
use calendar_core::recurrence::rule::RecurrenceRule;
use calendar_core::source_types::SourceError;
use calendar_core::time::{Zone, add_seconds, seconds_between};
use calendar_core::write::event_draft::{AttendeeDraft, EventDraft, EventTiming};
use calendar_core::write::event_patch::{AttendeeChanges, EventPatch, FieldUpdate};
use calendar_core::write::types::{
    ConferenceChange, ConferenceRequest, EventField, NotifyPolicy, WriteError,
};
use jiff::Timestamp;
use serde_json::{Map, Value};

use crate::event_mapper::GoogleEventMapper;

/// Swift's `typealias JSON = [String: Any]`.
pub(crate) type Json = Map<String, Value>;

/// A request body plus whether the request needs `conferenceDataVersion=1`.
#[derive(Clone, Debug)]
pub(crate) struct Body {
    pub json: Json,
    pub needs_conference_version: bool,
}

pub(crate) enum GoogleWriteMapper {}

fn invalid(message: &str) -> Failure {
    Failure::Write(WriteError::Invalid(message.to_string()))
}

fn unsupported(field: EventField) -> Failure {
    Failure::Write(WriteError::Unsupported {
        fields: BTreeSet::from([field]),
    })
}

fn object(pairs: &[(&str, Value)]) -> Json {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect()
}

impl GoogleWriteMapper {
    /// Google's popup reminder range: zero minutes to four weeks.
    const REMINDER_MINUTES: std::ops::RangeInclusive<i64> = 0..=40320;
    /// Google's event dates run from year 1 to 9999 (1970-based seconds).
    const REPRESENTABLE_SECONDS: std::ops::RangeInclusive<f64> =
        -62_135_596_800.0..=253_402_300_799.0;

    pub(crate) fn send_updates(policy: NotifyPolicy) -> &'static str {
        match policy {
            NotifyPolicy::All => "all",
            NotifyPolicy::ExternalOnly => "externalOnly",
            NotifyPolicy::None => "none",
        }
    }

    /// The request body as JSON text with sorted keys. (Swift checks `isValidJSONObject` first, because
    /// `JSONSerialization` crashes on a value such as NaN; a `serde_json::Value` cannot hold one.)
    pub(crate) fn data(json: &Json) -> Outcome<Vec<u8>> {
        serde_json::to_vec(json).map_err(|_| invalid("the request body is not valid JSON"))
    }

    pub(crate) fn instant_text(date: Timestamp) -> String {
        GoogleEventMapper::instant_text(date)
    }

    fn day_text(date: &CalendarDate) -> String {
        format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
    }

    /// The zone name Google can resolve. A fixed-offset zone (`GMT+0500`) is not an IANA name, so it yields `None`; a
    /// timed event still carries its offset in `dateTime`. Foundation names the UTC zone "GMT"; the wire format is
    /// "UTC".
    fn zone_name(zone: Option<&Zone>) -> Option<String> {
        let identifier = zone?.identifier();
        if identifier.starts_with("GMT") && identifier.chars().count() > 3 {
            return None;
        }
        Some(if identifier == "GMT" {
            "UTC".to_string()
        } else {
            identifier.to_string()
        })
    }

    /// Google has two values (opaque, transparent); a value it cannot store is mapped to the closest one
    /// (`Availability::closest_in`), and `adjustments_compared_to` on the draft or patch reports the change.
    pub(crate) fn transparency(availability: Availability) -> &'static str {
        let closest = availability
            .closest_in(&GoogleEventMapper::supported_availabilities())
            .unwrap_or(Availability::Busy);
        if closest == Availability::Free {
            "transparent"
        } else {
            "opaque"
        }
    }

    /// Google cannot store times outside years 1 to 9999; an out-of-range date would render as a malformed string.
    fn require_representable(timing: &EventTiming) -> Outcome<()> {
        for date in [timing.start, timing.end] {
            let seconds = seconds_between(Timestamp::UNIX_EPOCH, date);
            if !Self::REPRESENTABLE_SECONDS.contains(&seconds) {
                return Err(invalid("times must fall in the years 1 to 9999"));
            }
        }
        Ok(())
    }

    /// All-day: `date` values with the exclusive end. Timed: `dateTime` (UTC) plus the zone name when there is one.
    pub(crate) fn time_json(timing: &EventTiming) -> (Json, Json) {
        if timing.is_all_day {
            let zone = timing.time_zone.clone().unwrap_or_else(Zone::utc);
            let (first, end_exclusive) = AllDay::dates(timing.start, timing.end, &zone);
            return (
                object(&[("date", Value::String(Self::day_text(&first)))]),
                object(&[("date", Value::String(Self::day_text(&end_exclusive)))]),
            );
        }
        let stamp = |date: Timestamp| -> Json {
            let mut json = object(&[("dateTime", Value::String(Self::instant_text(date)))]);
            if let Some(name) = Self::zone_name(timing.time_zone.as_ref()) {
                json.insert("timeZone".to_string(), Value::String(name));
            }
            json
        };
        (stamp(timing.start), stamp(timing.end))
    }

    /// For PATCH: the same, plus explicit nulls for the other form's keys, so an all-day event can become timed and
    /// back (Google keeps the old `date` or `dateTime` otherwise and rejects the mixture).
    fn patch_time_json(timing: &EventTiming) -> (Json, Json) {
        let (mut start, mut end) = Self::time_json(timing);
        let stale: &[&str] = if timing.is_all_day {
            &["dateTime", "timeZone"]
        } else {
            &["date"]
        };
        for key in stale {
            start.entry((*key).to_string()).or_insert(Value::Null);
            end.entry((*key).to_string()).or_insert(Value::Null);
        }
        (start, end)
    }

    fn visibility_text(visibility: Visibility) -> &'static str {
        match visibility {
            Visibility::Default => "default",
            Visibility::PublicEvent => "public",
            Visibility::PrivateEvent => "private",
            Visibility::Confidential => "confidential",
        }
    }

    /// Google reminders are minutes before the start, shown as a popup or sent as an email to the account owner; any
    /// other trigger or type is refused before a request is made ("rejected, not mangled").
    pub(crate) fn reminders_json(reminders: &[Reminder]) -> Outcome<Json> {
        if reminders.len() > 5 {
            return Err(invalid("Google allows at most 5 reminders"));
        }
        let mut overrides: Vec<Value> = Vec::new();
        for reminder in reminders {
            let method = match &reminder.kind {
                ReminderType::Display => "popup",
                ReminderType::Email { address: None } => "email",
                _ => return Err(unsupported(EventField::Reminders)),
            };
            let Some(minutes) = reminder
                .minutes_before()
                .filter(|_| reminder.repeat_count == 0)
            else {
                return Err(unsupported(EventField::Reminders));
            };
            if !Self::REMINDER_MINUTES.contains(&minutes) {
                return Err(invalid("reminder minutes must be between 0 and 40320"));
            }
            overrides.push(Value::Object(object(&[
                ("method", Value::String(method.to_string())),
                ("minutes", Value::from(minutes)),
            ])));
        }
        Ok(object(&[
            ("useDefault", Value::Bool(false)),
            ("overrides", Value::Array(overrides)),
        ]))
    }

    pub(crate) fn attendee_json(attendee: &AttendeeDraft) -> Json {
        let mut json = object(&[("email", Value::String(attendee.email().to_string()))]);
        if let Some(name) = &attendee.name {
            json.insert("displayName".to_string(), Value::String(name.clone()));
        }
        if attendee.role == calendar_core::model::AttendeeRole::Optional {
            json.insert("optional".to_string(), Value::Bool(true));
        }
        if attendee.role == calendar_core::model::AttendeeRole::Resource {
            json.insert("resource".to_string(), Value::Bool(true));
        }
        json
    }

    pub(crate) fn conference_request_json() -> Json {
        let request = object(&[
            (
                "requestId",
                Value::String(uuid::Uuid::new_v4().to_string().to_uppercase()),
            ),
            (
                "conferenceSolutionKey",
                Value::Object(object(&[(
                    "type",
                    Value::String("hangoutsMeet".to_string()),
                )])),
            ),
        ]);
        object(&[("createRequest", Value::Object(request))])
    }

    fn recurrence_lines(
        rule: &RecurrenceRule,
        all_day: bool,
        zone: Option<&Zone>,
    ) -> Outcome<Vec<String>> {
        rule.validate()?;
        if !all_day && Self::zone_name(zone).is_none() {
            return Err(invalid("recurring events need a time zone"));
        }
        Ok(vec![format!("RRULE:{}", rule.rrule_string(all_day, zone))])
    }

    fn lines_value(lines: Vec<String>) -> Value {
        Value::Array(lines.into_iter().map(Value::String).collect())
    }

    pub(crate) fn create_body(draft: &EventDraft) -> Outcome<Body> {
        draft.validate()?;
        Self::require_representable(&draft.timing)?;
        let mut json = object(&[("summary", Value::String(draft.title.clone()))]);
        if let Some(notes) = &draft.notes {
            json.insert("description".to_string(), Value::String(notes.clone()));
        }
        if let Some(location) = &draft.location {
            json.insert("location".to_string(), Value::String(location.clone()));
        }
        let (start, end) = Self::time_json(&draft.timing);
        json.insert("start".to_string(), Value::Object(start));
        json.insert("end".to_string(), Value::Object(end));
        json.insert(
            "transparency".to_string(),
            Value::String(Self::transparency(draft.availability).to_string()),
        );
        json.insert(
            "visibility".to_string(),
            Value::String(Self::visibility_text(draft.visibility).to_string()),
        );
        if let Some(uid) = draft.uid.as_ref().filter(|uid| !uid.is_empty()) {
            json.insert("iCalUID".to_string(), Value::String(uid.clone()));
        }
        if let Some(reminders) = &draft.reminders {
            json.insert(
                "reminders".to_string(),
                Value::Object(Self::reminders_json(reminders)?),
            );
        }
        if !draft.attendees.is_empty() {
            json.insert(
                "attendees".to_string(),
                Value::Array(
                    draft
                        .attendees
                        .iter()
                        .map(|a| Value::Object(Self::attendee_json(a)))
                        .collect(),
                ),
            );
        }
        if let Some(rule) = &draft.recurrence {
            json.insert(
                "recurrence".to_string(),
                Self::lines_value(Self::recurrence_lines(
                    rule,
                    draft.timing.is_all_day,
                    draft.timing.time_zone.as_ref(),
                )?),
            );
        }
        if draft.conference == ConferenceRequest::Generate {
            json.insert(
                "conferenceData".to_string(),
                Value::Object(Self::conference_request_json()),
            );
        }
        Ok(Body {
            json,
            needs_conference_version: draft.conference == ConferenceRequest::Generate,
        })
    }

    /// Only the touched fields; `Clear` is JSON `null`. `current_attendees` is required when the patch changes
    /// attendees (Google replaces the whole array on PATCH).
    pub(crate) fn patch_body(
        patch: &EventPatch,
        current_attendees: Option<&[Json]>,
    ) -> Outcome<Body> {
        let mut json = Json::new();
        if let Some(title) = &patch.title {
            json.insert("summary".to_string(), Value::String(title.clone()));
        }
        match &patch.notes {
            FieldUpdate::Keep => {}
            FieldUpdate::Set(value) => {
                json.insert("description".to_string(), Value::String(value.clone()));
            }
            FieldUpdate::Clear => {
                json.insert("description".to_string(), Value::Null);
            }
        }
        match &patch.location {
            FieldUpdate::Keep => {}
            FieldUpdate::Set(value) => {
                json.insert("location".to_string(), Value::String(value.clone()));
            }
            FieldUpdate::Clear => {
                json.insert("location".to_string(), Value::Null);
            }
        }
        if let Some(timing) = &patch.timing {
            timing.validate()?;
            Self::require_representable(timing)?;
            let (start, end) = Self::patch_time_json(timing);
            json.insert("start".to_string(), Value::Object(start));
            json.insert("end".to_string(), Value::Object(end));
        }
        if let Some(availability) = patch.availability {
            json.insert(
                "transparency".to_string(),
                Value::String(Self::transparency(availability).to_string()),
            );
        }
        if let Some(visibility) = patch.visibility {
            json.insert(
                "visibility".to_string(),
                Value::String(Self::visibility_text(visibility).to_string()),
            );
        }
        match &patch.reminders {
            FieldUpdate::Keep => {}
            FieldUpdate::Set(list) => {
                json.insert(
                    "reminders".to_string(),
                    Value::Object(Self::reminders_json(list)?),
                );
            }
            FieldUpdate::Clear => {
                json.insert(
                    "reminders".to_string(),
                    Value::Object(object(&[("useDefault", Value::Bool(true))])),
                );
            }
        }
        if let Some(changes) = patch.attendees.as_ref().filter(|c| !c.is_empty()) {
            let Some(current) = current_attendees else {
                return Err(invalid("attendee changes need the current attendees"));
            };
            json.insert(
                "attendees".to_string(),
                Value::Array(
                    Self::merge_attendees(current, changes)
                        .into_iter()
                        .map(Value::Object)
                        .collect(),
                ),
            );
        }
        match &patch.recurrence {
            FieldUpdate::Keep => {}
            FieldUpdate::Clear => {
                json.insert("recurrence".to_string(), Value::Null);
            }
            FieldUpdate::Set(rule) => {
                // The series uses the time the patch sends; only without one does it keep the base's zone.
                let base = patch.base();
                let all_day = patch
                    .timing
                    .as_ref()
                    .map(|t| t.is_all_day)
                    .or_else(|| base.map(|b| b.is_all_day))
                    .unwrap_or(false);
                let zone: Option<Zone> = match &patch.timing {
                    Some(timing) => timing.time_zone.clone(),
                    None => base.map(|b| b.time_zone.clone()),
                };
                json.insert(
                    "recurrence".to_string(),
                    Self::lines_value(Self::recurrence_lines(rule, all_day, zone.as_ref())?),
                );
            }
        }
        match patch.conference {
            None => {}
            Some(ConferenceChange::Generate) => {
                json.insert(
                    "conferenceData".to_string(),
                    Value::Object(Self::conference_request_json()),
                );
            }
            Some(ConferenceChange::Remove) => {
                json.insert("conferenceData".to_string(), Value::Null);
            }
        }
        Ok(Body {
            json,
            needs_conference_version: patch.conference.is_some(),
        })
    }

    fn normalized(email: &str) -> String {
        email.trim().to_lowercase()
    }

    fn email_of(attendee: &Json) -> String {
        Self::normalized(attendee.get("email").and_then(Value::as_str).unwrap_or(""))
    }

    /// Removes `changes.remove`, then upserts `changes.add` by email (case-insensitive). Existing entries keep every
    /// other key (responses, `self`, `organizer`), so other guests' responses survive.
    pub(crate) fn merge_attendees(current: &[Json], changes: &AttendeeChanges) -> Vec<Json> {
        let removed: BTreeSet<String> =
            changes.remove.iter().map(|e| Self::normalized(e)).collect();
        let mut result: Vec<Json> = current
            .iter()
            .filter(|a| !removed.contains(&Self::email_of(a)))
            .cloned()
            .collect();
        for draft in &changes.add {
            let wanted = Self::normalized(draft.email());
            if let Some(index) = result.iter().position(|a| Self::email_of(a) == wanted) {
                let mut merged = result[index].clone();
                if let Some(name) = &draft.name {
                    merged.insert("displayName".to_string(), Value::String(name.clone()));
                }
                if draft.role == calendar_core::model::AttendeeRole::Optional {
                    merged.insert("optional".to_string(), Value::Bool(true));
                } else {
                    merged.remove("optional");
                }
                if draft.role == calendar_core::model::AttendeeRole::Resource {
                    merged.insert("resource".to_string(), Value::Bool(true));
                } else {
                    merged.remove("resource");
                }
                result[index] = merged;
            } else {
                result.push(Self::attendee_json(draft));
            }
        }
        result
    }

    pub(crate) fn respond_attendees(
        current: &[Json],
        response: ResponseStatus,
    ) -> Outcome<Vec<Json>> {
        let text = match response {
            ResponseStatus::Accepted => "accepted",
            ResponseStatus::Tentative => "tentative",
            ResponseStatus::Declined => "declined",
            ResponseStatus::NeedsAction => return Err(invalid("cannot respond with needsAction")),
        };
        let Some(index) = current
            .iter()
            .position(|a| a.get("self") == Some(&Value::Bool(true)))
        else {
            return Err(invalid("you are not an attendee of this event"));
        };
        let mut result = current.to_vec();
        result[index].insert(
            "responseStatus".to_string(),
            Value::String(text.to_string()),
        );
        Ok(result)
    }
}

// MARK: Series splitting

impl GoogleWriteMapper {
    /// Fields a new series must not copy from the master: identity, bookkeeping, and the old conference (a new one is
    /// requested instead).
    pub(crate) const OUTPUT_ONLY_KEYS: [&'static str; 16] = [
        "id",
        "etag",
        "iCalUID",
        "htmlLink",
        "created",
        "updated",
        "sequence",
        "creator",
        "organizer",
        "recurringEventId",
        "originalStartTime",
        "conferenceData",
        "hangoutLink",
        "kind",
        "status",
        "recurrence",
    ];

    pub(crate) fn is_rrule(line: &str) -> bool {
        line.to_uppercase().starts_with("RRULE:")
    }

    fn parts_of(line: &str) -> Vec<String> {
        line.chars()
            .skip("RRULE:".chars().count())
            .collect::<String>()
            .split(';')
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// The `recurrence` lines with every RRULE cut off just before `split` (`COUNT` and `UNTIL` replaced by an `UNTIL`
    /// that is a date for all-day series and a UTC date-time, one second earlier, for timed ones). Other lines
    /// (EXDATE, RDATE) keep only the dates before `split`: later ones now belong to the new series (see
    /// `carried_over`), so the master must not keep them too (a later RDATE would show up twice, and survive a
    /// delete). A value that cannot be read stays with the master. Works on the raw text, so rules outside the
    /// authorable subset are fine.
    pub(crate) fn truncated(
        lines: &[String],
        split: Timestamp,
        all_day: bool,
        zone: Option<&Zone>,
    ) -> Vec<String> {
        let utc = Zone::utc();
        let until = if all_day {
            RecurrenceRule::date_text(&AllDay::date_of(split, zone.unwrap_or(&utc)).adding_days(-1))
        } else {
            RecurrenceRule::until_text(add_seconds(split, -1.0), false, None)
        };
        lines
            .iter()
            .filter_map(|line| {
                if !Self::is_rrule(line) {
                    if !Self::is_date_line(line) {
                        return Some(line.clone());
                    }
                    return Self::date_line(line, zone.unwrap_or(&utc), |date| {
                        date.is_none_or(|d| d < split)
                    });
                }
                let mut kept: Vec<String> = Self::parts_of(line)
                    .into_iter()
                    .filter(|part| {
                        let upper = part.to_uppercase();
                        !upper.starts_with("COUNT=") && !upper.starts_with("UNTIL=")
                    })
                    .collect();
                kept.push(format!("UNTIL={until}"));
                Some(format!("RRULE:{}", kept.join(";")))
            })
            .collect()
    }

    /// The `COUNT` of the first RRULE, if any.
    pub(crate) fn count_in(lines: &[String]) -> Option<i64> {
        for line in lines.iter().filter(|line| Self::is_rrule(line)) {
            for part in Self::parts_of(line) {
                if part.to_uppercase().starts_with("COUNT=") {
                    return swift_int(&part.chars().skip("COUNT=".len()).collect::<String>());
                }
            }
        }
        None
    }

    /// Google allows a single RRULE per event, so every RRULE line is the one rule.
    pub(crate) fn replacing_count(lines: &[String], count: i64) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                if !Self::is_rrule(line) {
                    return line.clone();
                }
                let parts: Vec<String> = Self::parts_of(line)
                    .into_iter()
                    .map(|part| {
                        if part.to_uppercase().starts_with("COUNT=") {
                            format!("COUNT={count}")
                        } else {
                            part
                        }
                    })
                    .collect();
                format!("RRULE:{}", parts.join(";"))
            })
            .collect()
    }

    pub(crate) fn rrule_lines(lines: &[String]) -> Vec<String> {
        lines
            .iter()
            .filter(|line| Self::is_rrule(line))
            .cloned()
            .collect()
    }

    /// The EXDATE and RDATE lines cut down to the dates at or after `split`, so deleted or added occurrences that
    /// belong to the new series follow it (`truncated` removes the same dates from the master). A line left with no
    /// values is dropped, and a value that cannot be read stays with the master. `zone` reads floating times.
    pub(crate) fn carried_over(lines: &[String], split: Timestamp, zone: &Zone) -> Vec<String> {
        lines
            .iter()
            .filter(|line| Self::is_date_line(line))
            .filter_map(|line| Self::date_line(line, zone, |date| date.is_some_and(|d| d >= split)))
            .collect()
    }

    fn is_date_line(line: &str) -> bool {
        let name: String = line
            .chars()
            .take_while(|c| *c != ';' && *c != ':')
            .collect::<String>()
            .to_uppercase();
        (name == "EXDATE" || name == "RDATE") && line.contains(':')
    }

    /// An EXDATE or RDATE `line` reduced to the values for which `keep` (given the value's instant, `None` if
    /// unreadable) is true; `None` when none remain.
    fn date_line(
        line: &str,
        zone: &Zone,
        keep: impl Fn(Option<Timestamp>) -> bool,
    ) -> Option<String> {
        let colon = line.find(':')?;
        let head = &line[..colon];
        let tzid = head
            .split(';')
            .filter(|part| !part.is_empty())
            .skip(1)
            .find_map(|param| {
                param
                    .to_uppercase()
                    .starts_with("TZID=")
                    .then(|| param.chars().skip("TZID=".len()).collect::<String>())
            });
        let line_zone = tzid.as_deref().and_then(Zone::named);
        let line_zone = line_zone.as_ref().unwrap_or(zone);
        let kept: Vec<&str> = line[colon + 1..]
            .split(',')
            .filter(|value| !value.is_empty())
            .filter(|value| keep(Self::ical_instant(value, line_zone)))
            .collect();
        if kept.is_empty() {
            None
        } else {
            Some(format!("{head}:{}", kept.join(",")))
        }
    }

    /// `yyyyMMdd` (start of that day in `zone`), `yyyyMMdd'T'HHmmss` (in `zone`) or the same with a trailing `Z`
    /// (UTC). A PERIOD value (`start/end`) reads as its start.
    fn ical_instant(text: &str, zone: &Zone) -> Option<Timestamp> {
        let first = text
            .split('/')
            .find(|piece| !piece.is_empty())
            .unwrap_or(text);
        let mut value = first
            .trim_matches(|c: char| c == '\t' || (c.is_whitespace() && c != '\n' && c != '\r'))
            .to_uppercase();
        let is_utc = value.ends_with('Z');
        if is_utc {
            value.pop();
        }
        let pieces: Vec<&str> = value.split('T').collect();
        let day = *pieces.first()?;
        let digits = |text: &str| -> Vec<char> { text.chars().collect() };
        let day_chars = digits(day);
        if day_chars.len() != 8 {
            return None;
        }
        let slice = |chars: &[char], from: usize, to: usize| -> Option<i64> {
            swift_int(&chars[from..to].iter().collect::<String>())
        };
        let year = slice(&day_chars, 0, 4)?;
        let month = slice(&day_chars, 4, 6)?;
        let day_of_month = slice(&day_chars, 6, 8)?;
        let utc = Zone::utc();
        let effective = if is_utc { &utc } else { zone };
        if pieces.len() == 1 {
            return AllDay::start_of_day(&CalendarDate::new(year, month, day_of_month), effective);
        }
        let time = digits(pieces[1]);
        if pieces.len() != 2 || time.len() != 6 {
            return None;
        }
        calendar_core::civil::date_from(
            year,
            month,
            day_of_month,
            slice(&time, 0, 2)?,
            slice(&time, 2, 4)?,
            slice(&time, 4, 6)?,
            effective,
        )
    }

    fn has_meet_link(json: &Json) -> bool {
        let key = json
            .get("conferenceData")
            .and_then(|c| c.get("conferenceSolution"))
            .and_then(|s| s.get("key"));
        key.and_then(|k| k.get("type")).and_then(Value::as_str) == Some("hangoutsMeet")
            || json.contains_key("hangoutLink")
    }

    /// The zone name for a fallback identifier, spelled as Google's wire format wants it. An identifier this platform
    /// does not know is passed through unchanged (it came from Google in the first place).
    fn wire_name(identifier: &str) -> Option<String> {
        match Zone::named(identifier) {
            None => Some(identifier.to_string()),
            Some(zone) => Self::zone_name(Some(&zone)),
        }
    }

    /// The days between two `date` values of an all-day master.
    fn day_count(first: &str, last: &str) -> Option<i64> {
        let day = |text: &str| -> Option<Timestamp> {
            let parts: Vec<i64> = text
                .split('-')
                .filter(|piece| !piece.is_empty())
                .filter_map(swift_int)
                .collect();
            if parts.len() != 3 {
                return None;
            }
            AllDay::start_of_day(
                &CalendarDate::new(parts[0], parts[1], parts[2]),
                &Zone::utc(),
            )
        };
        let (a, b) = (day(first)?, day(last)?);
        Some((seconds_between(a, b) / 86_400.0).round() as i64)
    }

    /// The start and end of the new series: the occurrence's original `slot` with the master's length, in the master's
    /// own form (`dateTime` with its zone name, or all-day `date`s), so following occurrences keep the series' time even
    /// when this one was moved. `fallback_zone` reads an all-day slot's day and names the zone when the master has
    /// none.
    fn series_times(
        master: &Json,
        slot: Timestamp,
        fallback_zone: Option<&str>,
    ) -> Outcome<(Json, Json)> {
        let unreadable = || {
            Failure::Source(SourceError::InvalidResponse(
                "google: unreadable event".to_string(),
            ))
        };
        let (Some(start), Some(end)) = (
            master.get("start").and_then(Value::as_object),
            master.get("end").and_then(Value::as_object),
        ) else {
            return Err(unreadable());
        };
        let fallback = fallback_zone.and_then(Zone::named);
        if let (Some(first), Some(last)) = (
            start.get("date").and_then(Value::as_str),
            end.get("date").and_then(Value::as_str),
        ) {
            let days = match Self::day_count(first, last) {
                Some(days) if days >= 1 => days,
                _ => return Err(unreadable()),
            };
            let day = AllDay::date_of(slot, fallback.as_ref().unwrap_or(&Zone::utc()));
            return Ok((
                object(&[("date", Value::String(Self::day_text(&day)))]),
                object(&[(
                    "date",
                    Value::String(Self::day_text(&day.adding_days(days))),
                )]),
            ));
        }
        let (Some(master_start), Some(master_end)) = (
            start
                .get("dateTime")
                .and_then(Value::as_str)
                .and_then(GoogleEventMapper::parse_instant),
            end.get("dateTime")
                .and_then(Value::as_str)
                .and_then(GoogleEventMapper::parse_instant),
        ) else {
            return Err(unreadable());
        };
        if master_end <= master_start {
            return Err(unreadable());
        }
        let zone = start
            .get("timeZone")
            .and_then(Value::as_str)
            .and_then(Zone::named)
            .or(fallback);
        Ok(Self::time_json(&EventTiming::new(
            slot,
            add_seconds(slot, seconds_between(master_start, master_end)),
            zone,
            false,
        )))
    }

    /// The insert body for the new series: the whole master resource (so unmodeled fields such as color, attachments
    /// and extended properties carry over) minus output-only fields, starting at the occurrence's original `slot` with
    /// the master's length (unless the patch sets the time), guests' responses reset, the patch applied, and a new
    /// Meet link requested when the master had one (unless the patch removes the conference).
    pub(crate) fn new_series_body(
        master: &Json,
        slot: Timestamp,
        patch: &EventPatch,
        recurrence: Vec<String>,
        fallback_zone: Option<&str>,
    ) -> Outcome<Body> {
        let mut json: Json = master
            .iter()
            .filter(|(key, _)| !Self::OUTPUT_ONLY_KEYS.contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        // `as? [JSON]`: only an array whose every element is an object.
        let attendees: Option<Vec<Json>> = json.get("attendees").and_then(|value| {
            value
                .as_array()?
                .iter()
                .map(|a| a.as_object().cloned())
                .collect()
        });
        if let Some(attendees) = &attendees {
            json.insert(
                "attendees".to_string(),
                Value::Array(
                    attendees
                        .iter()
                        .map(|attendee| {
                            let mut copy = attendee.clone();
                            copy.remove("responseStatus");
                            copy.remove("self");
                            copy.remove("organizer");
                            Value::Object(copy)
                        })
                        .collect(),
                ),
            );
        }
        let (start, end) = Self::series_times(master, slot, fallback_zone)?;
        json.insert("start".to_string(), Value::Object(start));
        json.insert("end".to_string(), Value::Object(end));
        let current: Vec<Json> = json
            .get("attendees")
            .and_then(|value| {
                value
                    .as_array()?
                    .iter()
                    .map(|a| a.as_object().cloned())
                    .collect()
            })
            .unwrap_or_default();
        let changes = Self::patch_body(patch, Some(&current))?;
        for (key, value) in changes.json {
            match value {
                Value::Null => {
                    json.remove(&key);
                }
                // A PATCH clears the other form of a date with a null; an insert has nothing to clear.
                Value::Object(nested) => {
                    json.insert(
                        key,
                        Value::Object(nested.into_iter().filter(|(_, v)| !v.is_null()).collect()),
                    );
                }
                other => {
                    json.insert(key, other);
                }
            }
        }
        // A recurring event's times need a zone name for its rule to be read in.
        for key in ["start", "end"] {
            if let Some(Value::Object(time)) = json.get(key)
                && time.contains_key("dateTime")
                && !time.contains_key("timeZone")
                && let Some(zone) = fallback_zone.and_then(Self::wire_name)
            {
                let mut time = time.clone();
                time.insert("timeZone".to_string(), Value::String(zone));
                json.insert(key.to_string(), Value::Object(time));
            }
        }
        if patch.recurrence == FieldUpdate::Keep {
            json.insert("recurrence".to_string(), Self::lines_value(recurrence));
        }
        let mut needs_conference_version = changes.needs_conference_version;
        if patch.conference.is_none() && Self::has_meet_link(master) {
            json.insert(
                "conferenceData".to_string(),
                Value::Object(Self::conference_request_json()),
            );
            needs_conference_version = true;
        }
        Ok(Body {
            json,
            needs_conference_version,
        })
    }
}

/// Swift's `Int(String)`: an optional sign and decimal digits.
fn swift_int(text: &str) -> Option<i64> {
    text.parse::<i64>().ok()
}
