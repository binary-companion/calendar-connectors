//! Tests with no Swift counterpart: the "tolerates unknown and missing fields" tests of the plan's review focus 1 (the
//! provider responses the Swift tests never showed), the exact request URLs, the instant parser and formatter, and the
//! garbage-input check for the JSON decoders.

use std::sync::Arc;

use calendar_core::connection::{
    Connection, ConnectorKind, InMemoryCredentialStore, InMemorySyncStateStore,
};
use calendar_core::model::{Availability, EventKind, EventStatus, Visibility};
use calendar_core::source_types::CalendarSource;
use calendar_core::time::Zone;
use calendar_oauth::AccessTokenProvider;
use calendar_test_support::fake_transport::{FakeTransport, TestNow};
use serde_json::json;

use super::support::{Harness, instant, instant_sleeper, list_json, ok, one_hour_window};
use crate::api_client::{GoogleApiClient, query_item};
use crate::dtos::{GoogleCalendarListEntryDTO, GoogleEventDTO, GoogleEventsPageDTO};
use crate::event_mapper::GoogleEventMapper;
use crate::{GoogleConnectorKind, GoogleOAuthConfig};

// Review focus 1: events.

#[test]
fn an_event_with_unknown_enum_strings_extra_fields_and_nulls_still_maps() {
    let dto: GoogleEventDTO = serde_json::from_value(json!({
        "id": "x", "kind": "calendar#event", "futureField": {"a": [1, 2, 3]}, "sequence": 7,
        "status": "somethingNew", "eventType": "mystery", "transparency": "translucent", "visibility": "secret",
        "summary": null, "description": null, "location": null, "etag": null, "recurrence": null,
        "start": {"dateTime": "2026-09-21T10:00:00Z", "extra": true, "timeZone": "Not/AZone"},
        "end": {"dateTime": "2026-09-21T11:00:00Z"},
        "attendees": [{"email": "a@x.com", "responseStatus": "futureStatus", "additionalGuests": 2}],
        "organizer": {"email": null},
        "conferenceData": {"entryPoints": [{"entryPointType": "audio"}, {"entryPointType": "video"}], "conferenceSolution": {"key": {}}},
        "reminders": {"useDefault": false, "overrides": [{"method": null, "minutes": 15}, {"method": "popup"}]},
    }))
    .unwrap();
    let calendar = calendar_core::model::CalendarDescriptor::new(
        "c",
        "C",
        calendar_core::model::CalendarService::google(),
    );
    let e = GoogleEventMapper::map(&dto, &calendar, None).unwrap();
    assert_eq!(e.title, "(No title)");
    assert_eq!(e.status, EventStatus::Confirmed);
    assert_eq!(e.kind, Some(EventKind::Other));
    assert_eq!(e.availability, Some(Availability::Busy));
    assert_eq!(e.visibility, Some(Visibility::Default));
    // An unknown zone name falls back to the calendar's zone (UTC here).
    assert!(e.time_zone.identifier() == "UTC" || e.time_zone.identifier() == "GMT");
    assert_eq!(e.attendees.len(), 1);
    assert_eq!(e.reminders.as_ref().map(Vec::len), Some(1));
    assert!(e.conferences.is_empty());
    assert!(e.organizer.is_some_and(|o| o.email().is_none()));
}

#[test]
fn an_event_with_only_an_id_and_times_maps_with_every_optional_missing() {
    let dto: GoogleEventDTO = serde_json::from_str(
        r#"{"id":"min","start":{"dateTime":"2026-09-21T10:00:00Z"},"end":{"dateTime":"2026-09-21T11:00:00Z"}}"#,
    )
    .unwrap();
    let calendar = calendar_core::model::CalendarDescriptor::new(
        "c",
        "C",
        calendar_core::model::CalendarService::google(),
    );
    let e = GoogleEventMapper::map(&dto, &calendar, None).unwrap();
    assert!(e.notes.is_none() && e.location.is_none() && e.uid.is_none() && e.url.is_none());
    assert!(e.attendees.is_empty() && e.organizer.is_none() && e.conferences.is_empty());
    assert_eq!(e.reminders, Some(Vec::new()));
}

#[test]
fn a_page_without_items_or_with_null_items_decodes() {
    for text in [r#"{}"#, r#"{"items":null}"#, r#"{"items":[],"unknown":1}"#] {
        let page: GoogleEventsPageDTO = serde_json::from_str(text).unwrap();
        assert!(page.items.as_ref().is_none_or(Vec::is_empty), "{text}");
        assert!(page.next_page_token.is_none(), "{text}");
    }
}

// Review focus 1: calendars.

#[test]
fn a_calendar_with_unknown_and_missing_fields_still_maps() {
    let dto: GoogleCalendarListEntryDTO = serde_json::from_value(json!({
        "id": "c@x.com", "kind": "calendar#calendarListEntry", "etag": "e", "accessRole": "someFutureRole",
        "timeZone": "Not/AZone", "backgroundColor": "red", "foregroundColor": "#fff", "selected": true,
        "defaultReminders": [{"method": "sms"}, {"minutes": 5}, {"method": "popup", "minutes": 10.0}],
        "notificationSettings": {"notifications": []}, "summary": null,
    }))
    .unwrap();
    let d = GoogleEventMapper::descriptor(&dto, None).unwrap();
    // The title falls back to the id; an unknown role reads as a reader; a zone name that does not exist, as no zone;
    // a color that is not hex, as none; a reminder without minutes is dropped, one with 10.0 minutes is kept.
    assert_eq!(d.title, "c@x.com");
    assert!(!d.permissions.can_edit && d.permissions.can_view_details);
    assert!(d.time_zone.is_none());
    assert!(d.color_hex().is_none());
    assert_eq!(d.default_reminders.as_ref().map(Vec::len), Some(2));
}

#[tokio::test]
async fn a_calendar_list_with_extra_and_missing_fields_and_a_null_items_page_is_accepted() {
    let h = Harness::with_list(json!({"kind": "calendar#calendarList", "etag": "x", "items": [
        {"id": "only-an-id"}, {"id": "b", "hidden": null, "primary": null, "deleted": false, "nothing": 1},
    ]}))
    .await;
    let ids: Vec<String> = h
        .source
        .calendars()
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(ids, ["only-an-id", "b"]);
    let empty = Harness::with_list(json!({"items": null})).await;
    assert!(empty.source.calendars().await.unwrap().is_empty());
    let none = Harness::with_list(json!({})).await;
    assert!(none.source.calendars().await.unwrap().is_empty());
}

// Review focus 1: the token response (decoded by calendar-oauth, read here through the Google kind).

#[tokio::test]
async fn a_token_response_with_unknown_and_missing_fields_still_refreshes() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(
        "oauth2.googleapis.com/token",
        vec![ok(json!({
            "access_token": "tok", "token_type": "Bearer", "scope": "a b", "id_token": "x.y.z", "future": [1],
            "refresh_token": null,
        }))],
    );
    transport.route("users/me/calendarList", vec![ok(list_json(&["me@x.com"]))]);
    let store = Arc::new(InMemoryCredentialStore::new());
    calendar_core::connection::CredentialStore::set_secrets(
        store.as_ref(),
        [(
            AccessTokenProvider::REFRESH_TOKEN_KEY.to_string(),
            "rt".to_string(),
        )]
        .into(),
        "c1",
    )
    .await
    .unwrap();
    let kind = GoogleConnectorKind::new(
        GoogleOAuthConfig::new("cid", "csecret"),
        transport.clone(),
        TestNow::default().provider(),
        instant_sleeper(),
    );
    let connection = Connection::new("google", "c1", "me@x.com");
    let source = kind
        .make_source(&connection, store, Arc::new(InMemorySyncStateStore::new()))
        .unwrap();
    // `expires_in` is missing: the token is good for an hour, so the second call needs no second refresh.
    assert_eq!(source.calendars().await.unwrap().len(), 1);
    assert_eq!(source.calendars().await.unwrap().len(), 1);
    assert_eq!(
        transport
            .requests_matching("oauth2.googleapis.com/token")
            .len(),
        1
    );
    assert_eq!(
        transport.requests_matching("calendarList")[0]
            .headers
            .get("Authorization")
            .map(String::as_str),
        Some("Bearer tok")
    );
}

// Request URLs, compared as whole strings with what Foundation's `URLComponents` produced (probed on macOS).

#[test]
fn request_urls_match_what_url_components_builds() {
    let events = GoogleApiClient::url(
        &GoogleApiClient::calendar_path("me@x.com", "/events"),
        &[
            query_item("singleEvents", "true"),
            query_item(
                "timeMin",
                &GoogleEventMapper::instant_text(instant("2026-09-21T14:13:20Z")),
            ),
            query_item(
                "timeMax",
                &GoogleEventMapper::instant_text(instant("2026-09-22T14:13:20Z")),
            ),
            query_item("fields", "nextPageToken,items(id,iCalUID,updated)"),
        ],
    )
    .unwrap();
    assert_eq!(
        events.as_str(),
        "https://www.googleapis.com/calendar/v3/calendars/me%40x.com/events?singleEvents=true&timeMin=2026-09-21T14:13:20Z&timeMax=2026-09-22T14:13:20Z&fields=nextPageToken,items(id,iCalUID,updated)"
    );
    let awkward = GoogleApiClient::url(
        &GoogleApiClient::event_path("Zoë & Co/#1 +x%@group.calendar.google.com", "ev id_1~é"),
        &[
            query_item("syncToken", "CPj=+/ a&b#c%d é😀;?:@!$'()*,~"),
            query_item("pageToken", "A[1]{2}|\\^`<>\""),
        ],
    )
    .unwrap();
    assert_eq!(
        awkward.as_str(),
        "https://www.googleapis.com/calendar/v3/calendars/Zo%C3%AB%20%26%20Co%2F%231%20%2Bx%25%40group.calendar.google.com/events/ev%20id_1~%C3%A9?syncToken=CPj%3D+/%20a%26b%23c%25d%20%C3%A9%F0%9F%98%80;?:@!$'()*,~&pageToken=A%5B1%5D%7B2%7D%7C%5C%5E%60%3C%3E%22"
    );
    assert_eq!(
        GoogleApiClient::url("/users/me/calendarList", &[])
            .unwrap()
            .as_str(),
        "https://www.googleapis.com/calendar/v3/users/me/calendarList"
    );
}

#[tokio::test]
async fn the_events_request_carries_the_window_and_the_fields_mask() {
    let h = Harness::with_list(list_json(&["me@x.com"])).await;
    h.transport.route(
        "calendars/me%40x.com/events",
        vec![ok(json!({"items": []}))],
    );
    let window = one_hour_window();
    h.source.events_in(window).await.unwrap();
    let request = h
        .transport
        .requests_matching("/events")
        .into_iter()
        .next()
        .unwrap();
    let expected_fields = crate::source::GoogleCalendarSource::EVENT_FIELDS;
    assert_eq!(
        request.url.as_str(),
        format!(
            "https://www.googleapis.com/calendar/v3/calendars/me%40x.com/events?singleEvents=true&timeMin={}&timeMax={}&maxResults=250&showDeleted=false&fields={expected_fields}",
            GoogleEventMapper::instant_text(window.start),
            GoogleEventMapper::instant_text(window.end),
        )
    );
    assert_eq!(request.method, "GET");
    assert_eq!(
        request.headers.get("Accept").map(String::as_str),
        Some("application/json")
    );
}

// The instant parser and formatter.

#[test]
fn instants_parse_with_and_without_fractions_and_offsets() {
    let parse = GoogleEventMapper::parse_instant;
    assert_eq!(
        parse("2026-09-21T10:00:00Z"),
        Some(instant("2026-09-21T10:00:00Z"))
    );
    assert_eq!(
        parse("2026-09-21T10:00:00.500Z"),
        Some(instant("2026-09-21T10:00:00.5Z"))
    );
    assert_eq!(
        parse("2026-09-21T10:00:00.5Z"),
        Some(instant("2026-09-21T10:00:00.5Z"))
    );
    // Foundation keeps milliseconds.
    assert_eq!(
        parse("2026-09-21T10:00:00.1239999Z"),
        Some(instant("2026-09-21T10:00:00.123Z"))
    );
    assert_eq!(
        parse("2026-09-21T10:00:00-07:00"),
        Some(instant("2026-09-21T17:00:00Z"))
    );
    assert_eq!(
        parse("2026-09-21T10:00:00+0530"),
        Some(instant("2026-09-21T04:30:00Z"))
    );
    assert_eq!(
        parse("2026-09-21T10:00:00+01"),
        Some(instant("2026-09-21T09:00:00Z"))
    );
    assert_eq!(
        parse("2026-09-21T10:00:00.250+02:00"),
        Some(instant("2026-09-21T08:00:00.25Z"))
    );
    assert_eq!(
        parse("2024-02-29T00:00:00Z"),
        Some(instant("2024-02-29T00:00:00Z"))
    );
}

#[test]
fn malformed_instants_are_none() {
    for text in [
        "",
        "garbage",
        "2026-09-21",
        "2026-09-21T10:00:00",
        "2026-09-21T10:00Z",
        "2026-09-21 10:00:00Z",
        "2026-13-01T00:00:00Z",
        "2026-00-10T00:00:00Z",
        "2026-02-30T10:00:00Z",
        "2026-09-21T24:00:00Z",
        "2026-09-21T10:60:00Z",
        "2026-09-21T10:00:60Z",
        "2026-09-21T10:00:00.Z",
        "2026-09-21T10:00:00Zjunk",
        "0000-01-01T00:00:00Z",
        "2026-09-21T10:00:00+25:00",
        "2026-09-21T10:00:00+05:3",
        "２０２６-09-21T10:00:00Z",
        "2026-09-21T10:00:00+05:30:15",
    ] {
        assert_eq!(GoogleEventMapper::parse_instant(text), None, "{text:?}");
    }
}

#[test]
fn instants_format_as_whole_second_utc_text_rounding_down() {
    let text = GoogleEventMapper::instant_text;
    assert_eq!(text(jiff::Timestamp::UNIX_EPOCH), "1970-01-01T00:00:00Z");
    assert_eq!(
        text(instant("2026-09-21T10:00:00.999Z")),
        "2026-09-21T10:00:00Z"
    );
    assert_eq!(
        text(instant("1969-12-31T23:59:59.5Z")),
        "1969-12-31T23:59:59Z"
    );
    // `jiff::Timestamp` ends on 9999-12-30T22:00:00Z, a little before the end of year 9999.
    assert_eq!(
        text(instant("9999-12-30T21:59:59Z")),
        "9999-12-30T21:59:59Z"
    );
    assert_eq!(
        text(instant("2024-02-29T12:00:00Z")),
        "2024-02-29T12:00:00Z"
    );
    for sample in [
        "2026-09-21T10:00:00Z",
        "2000-01-01T00:00:00Z",
        "1999-12-31T23:59:59Z",
    ] {
        assert_eq!(
            GoogleEventMapper::parse_instant(&text(instant(sample))),
            Some(instant(sample))
        );
    }
}

#[test]
fn an_all_day_date_that_is_not_three_numbers_is_dropped() {
    let calendar = Zone::utc();
    for date in ["2026-09", "2026-09-20-01", "garbage", ""] {
        let dto: GoogleEventDTO = serde_json::from_value(json!({
            "id": "a", "start": {"date": date}, "end": {"date": "2026-09-21"},
        }))
        .unwrap();
        assert!(
            GoogleEventMapper::resolve(dto.start.as_ref(), &calendar).is_none(),
            "{date:?}"
        );
    }
}

// Garbage input to the decoders and the instant parser: an error or a value, never a panic.

#[test]
fn garbage_and_truncated_json_never_panic_the_decoders() {
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..2000 {
        let length = (next() % 120) as usize;
        let bytes: Vec<u8> = (0..length).map(|_| (next() & 0xFF) as u8).collect();
        let _ = serde_json::from_slice::<GoogleEventDTO>(&bytes);
        let _ = serde_json::from_slice::<GoogleEventsPageDTO>(&bytes);
        let _ = serde_json::from_slice::<GoogleCalendarListEntryDTO>(&bytes);
        let _ = GoogleEventMapper::parse_instant(&String::from_utf8_lossy(&bytes));
    }
    let sample = br#"{"id":"e","start":{"dateTime":"2026-09-21T10:00:00-07:00","timeZone":"America/Los_Angeles"},"attendees":[{"email":"a@x.com","self":true}],"reminders":{"overrides":[{"method":"popup","minutes":10}]}}"#;
    for cut in 0..sample.len() {
        let _ = serde_json::from_slice::<GoogleEventDTO>(&sample[..cut]);
    }
}
