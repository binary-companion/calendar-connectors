//! Port of `GoogleDTOs.swift`: the shapes of Google Calendar's JSON, family C of the conventions (provider payloads).
//!
//! Every Swift optional is an `Option` that is `None` when the field is missing or `null`; unknown fields are ignored;
//! a field of the wrong type fails the value that holds it, exactly as Swift's `Decodable` does (a bad event is then
//! dropped by [`LenientItem`] instead of failing its page). Field names follow the Swift properties.

use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Deserializer};

/// Swift's `Int?` field: a JSON integer, or a number with no fraction (`10.0`), which Foundation's decoder also reads
/// as an `Int`. A string or a number with a fraction is an error.
fn optional_int<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<i64>, D::Error> {
    let Some(number) = Option::<serde_json::Number>::deserialize(deserializer)? else {
        return Ok(None);
    };
    if let Some(whole) = number.as_i64() {
        return Ok(Some(whole));
    }
    match number.as_f64() {
        Some(value) if value.fract() == 0.0 && value.abs() < 9.0e18 => Ok(Some(value as i64)),
        _ => Err(D::Error::custom("not an integer")),
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GoogleTimeDTO {
    pub date: Option<String>,
    pub date_time: Option<String>,
    pub time_zone: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GoogleAttendeeDTO {
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub response_status: Option<String>,
    pub optional: Option<bool>,
    pub resource: Option<bool>,
    pub organizer: Option<bool>,
    #[serde(rename = "self")]
    pub is_self: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GoogleOrganizerDTO {
    pub email: Option<String>,
    pub display_name: Option<String>,
    #[serde(rename = "self")]
    pub is_self: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GoogleConferenceEntryPoint {
    pub entry_point_type: Option<String>,
    pub uri: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct GoogleConferenceSolutionKey {
    #[serde(rename = "type")]
    pub kind: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct GoogleConferenceSolution {
    pub key: Option<GoogleConferenceSolutionKey>,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GoogleConferenceDTO {
    pub entry_points: Option<Vec<GoogleConferenceEntryPoint>>,
    pub conference_solution: Option<GoogleConferenceSolution>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct GoogleReminderOverride {
    pub method: Option<String>,
    #[serde(default, deserialize_with = "optional_int")]
    pub minutes: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GoogleRemindersDTO {
    pub use_default: Option<bool>,
    pub overrides: Option<Vec<GoogleReminderOverride>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GoogleEventDTO {
    pub id: String,
    #[serde(rename = "iCalUID")]
    pub ical_uid: Option<String>,
    pub status: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub html_link: Option<String>,
    pub etag: Option<String>,
    pub updated: Option<String>,
    pub created: Option<String>,
    pub hangout_link: Option<String>,
    pub transparency: Option<String>,
    pub visibility: Option<String>,
    pub event_type: Option<String>,
    pub recurring_event_id: Option<String>,
    /// Present on a series master only (reads expand series, so they never see it); the write path uses it.
    pub recurrence: Option<Vec<String>>,
    pub start: Option<GoogleTimeDTO>,
    pub end: Option<GoogleTimeDTO>,
    pub original_start_time: Option<GoogleTimeDTO>,
    pub attendees: Option<Vec<GoogleAttendeeDTO>>,
    pub organizer: Option<GoogleOrganizerDTO>,
    pub conference_data: Option<GoogleConferenceDTO>,
    pub reminders: Option<GoogleRemindersDTO>,
}

/// Decodes one array element without ever failing, so a single malformed item is dropped (`value == None`) instead of
/// failing the whole page.
#[derive(Clone, Debug)]
pub(crate) struct LenientItem<T> {
    pub value: Option<T>,
}

impl<'de, T: DeserializeOwned> Deserialize<'de> for LenientItem<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = serde_json::Value::deserialize(deserializer)?;
        Ok(LenientItem {
            value: serde_json::from_value(raw).ok(),
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GoogleEventsPageDTO {
    pub items: Option<Vec<LenientItem<GoogleEventDTO>>>,
    pub next_page_token: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct GoogleDefaultReminder {
    pub method: Option<String>,
    #[serde(default, deserialize_with = "optional_int")]
    pub minutes: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GoogleCalendarListEntryDTO {
    pub id: String,
    pub summary: Option<String>,
    pub summary_override: Option<String>,
    pub background_color: Option<String>,
    pub access_role: Option<String>,
    pub primary: Option<bool>,
    pub time_zone: Option<String>,
    pub hidden: Option<bool>,
    pub deleted: Option<bool>,
    pub default_reminders: Option<Vec<GoogleDefaultReminder>>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GoogleCalendarListPageDTO {
    pub items: Option<Vec<GoogleCalendarListEntryDTO>>,
    pub next_page_token: Option<String>,
}
