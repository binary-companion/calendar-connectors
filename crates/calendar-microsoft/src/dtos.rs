//! Port of `GraphDTOs.swift`: the shapes of Microsoft Graph's JSON, family C of the conventions (provider payloads).
//!
//! Every Swift optional is an `Option` that is `None` when the field is missing or `null`; unknown fields are ignored;
//! a field of the wrong type fails the value that holds it, exactly as Swift's `Decodable` does (a bad item is then
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

/// A page of a Graph collection: the link to the next page.
pub(crate) trait GraphPage {
    fn next_link(&self) -> Option<&str>;
}

/// A Graph collection page. Items decode leniently; `delta_link` is present on the last page of a delta query.
#[derive(Clone, Debug, Deserialize)]
#[serde(bound(deserialize = "Item: DeserializeOwned"))]
pub(crate) struct GraphListPage<Item> {
    pub value: Option<Vec<LenientItem<Item>>>,
    #[serde(rename = "@odata.nextLink")]
    pub next_link: Option<String>,
    #[serde(rename = "@odata.deltaLink")]
    pub delta_link: Option<String>,
}

impl<Item: Clone> GraphListPage<Item> {
    pub(crate) fn items(&self) -> Vec<Item> {
        self.value
            .iter()
            .flatten()
            .filter_map(|item| item.value.clone())
            .collect()
    }
}

impl<Item> GraphPage for GraphListPage<Item> {
    fn next_link(&self) -> Option<&str> {
        self.next_link.as_deref()
    }
}

/// Only the id: a delta page is counted, never mapped.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct GraphIDDTO {
    /// Decoded as in Swift (a delta page is only counted); read by the tests.
    #[allow(dead_code)]
    pub id: Option<String>,
}

/// `{ "value": "..." }`, the shape of a single-property response such as `/me/mailboxSettings/timeZone`.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct GraphValueDTO {
    pub value: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct GraphEmailDTO {
    pub name: Option<String>,
    pub address: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GraphCalendarDTO {
    pub id: String,
    pub name: Option<String>,
    pub hex_color: Option<String>,
    pub is_default_calendar: Option<bool>,
    pub can_edit: Option<bool>,
    pub can_share: Option<bool>,
    pub can_view_private_items: Option<bool>,
    /// Decoded as in Swift; the mapper does not use it.
    #[allow(dead_code)]
    pub owner: Option<GraphEmailDTO>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GraphTimeDTO {
    pub date_time: Option<String>,
    pub time_zone: Option<String>,
}

/// Swift's nested `GraphAttendeeDTO.Status`.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct GraphAttendeeStatus {
    pub response: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GraphAttendeeDTO {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub status: Option<GraphAttendeeStatus>,
    pub email_address: Option<GraphEmailDTO>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GraphRecipientDTO {
    pub email_address: Option<GraphEmailDTO>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GraphBodyDTO {
    pub content_type: Option<String>,
    pub content: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GraphLocationDTO {
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GraphOnlineMeetingDTO {
    pub join_url: Option<String>,
}

/// Swift's nested `GraphRecurrenceDTO.Pattern`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GraphRecurrencePattern {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(default, deserialize_with = "optional_int")]
    pub interval: Option<i64>,
    #[serde(default, deserialize_with = "optional_int")]
    pub month: Option<i64>,
    #[serde(default, deserialize_with = "optional_int")]
    pub day_of_month: Option<i64>,
    pub days_of_week: Option<Vec<String>>,
    pub first_day_of_week: Option<String>,
    pub index: Option<String>,
}

/// Swift's nested `GraphRecurrenceDTO.Range`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GraphRecurrenceRange {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Decoded as in Swift; the mapper reads the range start from the event itself.
    #[allow(dead_code)]
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub recurrence_time_zone: Option<String>,
    #[serde(default, deserialize_with = "optional_int")]
    pub number_of_occurrences: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct GraphRecurrenceDTO {
    pub pattern: Option<GraphRecurrencePattern>,
    pub range: Option<GraphRecurrenceRange>,
}

/// Swift's nested `GraphEventDTO.ResponseStatus`.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct GraphResponseStatus {
    /// Decoded as in Swift; the mapper does not use it.
    #[allow(dead_code)]
    pub response: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GraphEventDTO {
    pub id: String,
    #[serde(rename = "iCalUId")]
    pub ical_uid: Option<String>,
    pub subject: Option<String>,
    pub body: Option<GraphBodyDTO>,
    pub location: Option<GraphLocationDTO>,
    pub start: Option<GraphTimeDTO>,
    pub end: Option<GraphTimeDTO>,
    pub is_all_day: Option<bool>,
    pub is_cancelled: Option<bool>,
    pub show_as: Option<String>,
    pub sensitivity: Option<String>,
    /// `singleInstance`, `occurrence`, `exception` or `seriesMaster`.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub series_master_id: Option<String>,
    pub original_start: Option<String>,
    pub original_start_time_zone: Option<String>,
    pub attendees: Option<Vec<GraphAttendeeDTO>>,
    pub organizer: Option<GraphRecipientDTO>,
    pub is_organizer: Option<bool>,
    #[allow(dead_code)]
    pub response_status: Option<GraphResponseStatus>,
    pub is_reminder_on: Option<bool>,
    #[serde(default, deserialize_with = "optional_int")]
    pub reminder_minutes_before_start: Option<i64>,
    #[allow(dead_code)]
    pub is_online_meeting: Option<bool>,
    pub online_meeting: Option<GraphOnlineMeetingDTO>,
    pub change_key: Option<String>,
    pub last_modified_date_time: Option<String>,
    pub created_date_time: Option<String>,
    pub web_link: Option<String>,
    pub recurrence: Option<GraphRecurrenceDTO>,
}
