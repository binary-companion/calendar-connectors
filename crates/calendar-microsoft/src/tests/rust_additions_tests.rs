//! Tests with no Swift counterpart: the "tolerates unknown and missing fields" tests of the plan's review focus 1
//! (events, calendars, `/me` and the mailbox time-zone response), the exact request URLs (probed against
//! `URLComponents` on macOS), the backoff, the parsers on garbage input, and the redirect-host rewrite.

use std::sync::Arc;

use calendar_core::connection::{ConnectorKind, InMemoryCredentialStore, InMemorySyncStateStore};
use calendar_core::failure::Failure;
use calendar_core::model::{
    Availability, CalendarDescriptor, CalendarService, EventKind, EventStatus, Visibility,
};
use calendar_core::source_types::{CalendarSource, SourceError};
use calendar_core::time::Zone;
use calendar_test_support::fake_transport::{FakeTransport, TestNow};
use jiff::Timestamp;
use serde_json::json;

use super::support::{
    ClientHarness, SourceHarness, calendars_json_ids, decode_event, graph_event, instant_sleeper,
    ok, one_hour_window, status, with_headers,
};
use crate::api_client::{GraphApiClient, QueryItem, query_item};
use crate::dtos::{
    GraphCalendarDTO, GraphEventDTO, GraphIDDTO, GraphListPage, GraphRecurrenceDTO, GraphValueDTO,
};
use crate::event_mapper::GraphEventMapper;
use crate::graph_time::GraphTime;
use crate::recurrence_mapper::GraphRecurrenceMapper;
use crate::source_sync::DeltaState;
use crate::{MicrosoftConnectorKind, MicrosoftOAuthConfig};

fn calendar() -> CalendarDescriptor {
    CalendarDescriptor::new("c", "C", CalendarService::microsoft())
}

// Review focus 1: events.

#[test]
fn an_event_with_unknown_enum_strings_extra_fields_and_nulls_still_maps() {
    let dto: GraphEventDTO = serde_json::from_value(json!({
        "id": "x", "@odata.etag": "W/abc", "futureField": {"a": [1, 2, 3]}, "type": "somethingNew",
        "showAs": "somethingNew", "sensitivity": "secret", "subject": null, "body": null, "location": {"displayName": null},
        "iCalUId": null, "changeKey": null, "webLink": null, "recurrence": null, "originalStart": null,
        "start": {"dateTime": "2026-09-21T10:00:00.0000000", "extra": true, "timeZone": "Not/AZone"},
        "end": {"dateTime": "2026-09-21T11:00:00.0000000"},
        "attendees": [{"type": "futureType", "status": {"response": "futureResponse"}, "emailAddress": {"address": "a@x.com", "x": 1}}],
        "organizer": {"emailAddress": {"address": null}}, "onlineMeeting": {"joinUrl": null},
        "isReminderOn": true, "reminderMinutesBeforeStart": null,
    }))
    .unwrap();
    let e = GraphEventMapper::map(&dto, &calendar(), None, None).unwrap();
    assert_eq!(e.title, "(No title)");
    assert_eq!(e.status, EventStatus::Confirmed);
    assert_eq!(e.kind, Some(EventKind::Standard));
    assert_eq!(e.availability, Some(Availability::Busy));
    assert_eq!(e.visibility, Some(Visibility::Default));
    // An unknown zone name falls back to the calendar's zone (UTC here).
    assert!(e.time_zone.identifier() == "UTC" || e.time_zone.identifier() == "GMT");
    assert_eq!(e.attendees.len(), 1);
    assert_eq!(
        e.attendees[0].role,
        calendar_core::model::AttendeeRole::Required
    );
    assert_eq!(
        e.attendees[0].response,
        calendar_core::model::ResponseStatus::NeedsAction
    );
    // A reminder that is on with no minutes is the 15-minute default.
    assert_eq!(e.reminders.as_ref().map(Vec::len), Some(1));
    assert!(e.conferences.is_empty());
    assert!(e.organizer.is_some_and(|o| o.email().is_none()));
    assert!(e.location.is_none() && e.notes.is_none() && e.uid.is_none() && e.url.is_none());
}

#[test]
fn an_event_with_only_an_id_and_times_maps_with_every_optional_missing() {
    let dto: GraphEventDTO = serde_json::from_str(
        r#"{"id":"min","start":{"dateTime":"2026-09-21T10:00:00"},"end":{"dateTime":"2026-09-21T11:00:00"}}"#,
    )
    .unwrap();
    let e = GraphEventMapper::map(&dto, &calendar(), None, None).unwrap();
    assert!(e.notes.is_none() && e.location.is_none() && e.uid.is_none() && e.url.is_none());
    assert!(e.attendees.is_empty() && e.organizer.is_none() && e.conferences.is_empty());
    assert_eq!(e.reminders, Some(Vec::new()));
    assert!(e.version.is_none() && e.last_modified.is_none() && e.created.is_none());
}

#[test]
fn an_event_without_an_id_is_dropped_by_the_page_decoder() {
    let page: GraphListPage<GraphEventDTO> = serde_json::from_value(
        json!({"value": [{"subject": "no id"}, {"id": "ok", "start": null, "end": null}]}),
    )
    .unwrap();
    assert_eq!(page.value.as_ref().map(Vec::len), Some(2));
    assert_eq!(page.items().len(), 1);
}

#[test]
fn a_page_without_value_or_with_null_value_decodes() {
    for text in [
        r#"{}"#,
        r#"{"value":null}"#,
        r#"{"value":[],"unknown":1,"@odata.context":"x"}"#,
    ] {
        let page: GraphListPage<GraphIDDTO> = serde_json::from_str(text).unwrap();
        assert!(page.items().is_empty(), "{text}");
        assert!(
            page.next_link.is_none() && page.delta_link.is_none(),
            "{text}"
        );
    }
}

// Review focus 1: calendars.

#[test]
fn a_calendar_with_unknown_and_missing_fields_still_maps() {
    let dto: GraphCalendarDTO = serde_json::from_value(json!({
        "id": "c@x.com", "@odata.etag": "e", "color": "auto", "hexColor": "red", "isRemovable": true,
        "allowedOnlineMeetingProviders": ["teamsForBusiness"], "defaultOnlineMeetingProvider": "teamsForBusiness",
        "name": null, "canEdit": null, "owner": {"name": null, "address": null},
    }))
    .unwrap();
    let d = GraphEventMapper::descriptor(&dto, None, &Zone::utc());
    // The title falls back to the id; a colour that is not hex reads as none; a missing flag reads as false.
    assert_eq!(d.title, "c@x.com");
    assert!(d.color_hex().is_none());
    assert!(!d.permissions.can_edit && d.permissions.can_view_details);
    assert_eq!(d.is_default, Some(false));
}

#[tokio::test]
async fn a_calendar_list_with_extra_and_missing_fields_and_a_null_value_page_is_accepted() {
    let h = SourceHarness::with_calendars(json!({"@odata.context": "x", "value": [
        {"id": "only-an-id"}, {"id": "b", "name": null, "hexColor": null, "nothing": 1}, {"name": "no id"},
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
    let empty = SourceHarness::with_calendars(json!({"value": null})).await;
    assert!(empty.source.calendars().await.unwrap().is_empty());
    let none = SourceHarness::with_calendars(json!({})).await;
    assert!(none.source.calendars().await.unwrap().is_empty());
}

// Review focus 1: /me.

async fn account_email_for(me: serde_json::Value) -> Result<String, Failure> {
    let h = ClientHarness::new().await;
    h.transport.route("v1.0/me", vec![ok(me)]);
    h.client.account_email().await
}

#[tokio::test]
async fn me_with_unknown_and_missing_fields_still_names_the_account() {
    assert_eq!(
        account_email_for(json!({"@odata.context": "x", "id": "abc", "mail": null, "userPrincipalName": "Me@X.com", "businessPhones": []})).await,
        Ok("me@x.com".to_string())
    );
    assert_eq!(
        account_email_for(json!({"mail": "A@B.com"})).await,
        Ok("a@b.com".to_string())
    );
    let none = Err(Failure::Source(SourceError::InvalidResponse(
        "the account has no email address".to_string(),
    )));
    assert_eq!(account_email_for(json!({})).await, none);
    assert_eq!(
        account_email_for(json!({"mail": null, "userPrincipalName": null})).await,
        none
    );
    assert_eq!(
        account_email_for(json!({"mail": "", "userPrincipalName": ""})).await,
        none
    );
}

#[tokio::test]
async fn me_with_a_field_of_the_wrong_type_is_an_invalid_response() {
    assert_eq!(
        account_email_for(json!({"mail": 5})).await,
        Err(Failure::Source(SourceError::InvalidResponse(
            "could not decode MeDTO".to_string()
        )))
    );
}

// Review focus 1: the mailbox time-zone response.

async fn zone_for(response: serde_json::Value) -> Result<String, Failure> {
    let h = SourceHarness::new().await;
    h.transport
        .route("me/mailboxSettings/timeZone", vec![ok(response)]);
    h.source.calendars().await.map(|calendars| {
        calendars[0]
            .time_zone
            .as_ref()
            .unwrap()
            .identifier()
            .to_string()
    })
}

#[tokio::test]
async fn a_mailbox_zone_response_with_unknown_and_missing_fields_still_reads() {
    assert_eq!(
        zone_for(json!({"@odata.context": "x", "value": "W. Europe Standard Time", "extra": [1]}))
            .await,
        Ok("Europe/Berlin".to_string())
    );
    // A missing, null or unknown value is UTC.
    for response in [
        json!({}),
        json!({"value": null}),
        json!({"value": "Atlantis Standard Time"}),
    ] {
        let zone = zone_for(response.clone()).await.unwrap();
        assert!(zone == "UTC" || zone == "GMT", "{response}: {zone}");
    }
    // A value of the wrong type fails the request, as `Decodable` does: it is not a definite "no zone".
    assert_eq!(
        zone_for(json!({"value": 5})).await,
        Err(Failure::Source(SourceError::InvalidResponse(
            "could not decode GraphValueDTO".to_string()
        )))
    );
}

#[test]
fn the_value_dto_ignores_unknown_fields() {
    let dto: GraphValueDTO = serde_json::from_str(r#"{"value":"UTC","x":1}"#).unwrap();
    assert_eq!(dto.value.as_deref(), Some("UTC"));
}

// The token response is decoded by calendar-oauth; read here through the Microsoft kind.

#[tokio::test]
async fn a_token_response_with_unknown_and_missing_fields_still_refreshes() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(
        "login.microsoftonline.com/common/oauth2/v2.0/token",
        vec![ok(json!({
            "access_token": "tok", "token_type": "Bearer", "scope": "a b", "id_token": "x.y.z", "ext_expires_in": 3599,
            "future": [1], "refresh_token": null,
        }))],
    );
    transport.route(
        "me/mailboxSettings/timeZone",
        vec![ok(json!({"value": "UTC"}))],
    );
    transport.route("me/calendars?", vec![ok(calendars_json_ids(&["cal1"]))]);
    let store = Arc::new(InMemoryCredentialStore::new());
    calendar_core::connection::CredentialStore::set_secrets(
        store.as_ref(),
        [(
            calendar_oauth::AccessTokenProvider::REFRESH_TOKEN_KEY.to_string(),
            "rt".to_string(),
        )]
        .into(),
        "c1",
    )
    .await
    .unwrap();
    let kind = MicrosoftConnectorKind::new(
        MicrosoftOAuthConfig::new("cid"),
        transport.clone(),
        TestNow::default().provider(),
        instant_sleeper(),
    );
    let connection = calendar_core::connection::Connection::new("microsoft", "c1", "me@x.com");
    let source = kind
        .make_source(&connection, store, Arc::new(InMemorySyncStateStore::new()))
        .unwrap();
    // `expires_in` is missing: the token is good for an hour, so the second call needs no second refresh.
    assert_eq!(source.calendars().await.unwrap().len(), 1);
    assert_eq!(source.calendars().await.unwrap().len(), 1);
    assert_eq!(transport.requests_matching("oauth2/v2.0/token").len(), 1);
    assert_eq!(
        transport.requests_matching("me/calendars?")[0]
            .headers
            .get("Authorization")
            .map(String::as_str),
        Some("Bearer tok")
    );
}

// Request URLs, compared as whole strings with what Foundation's `URLComponents` produced (probed on macOS).

#[test]
fn request_urls_match_what_url_components_builds() {
    let events = GraphApiClient::url(
        &GraphApiClient::calendar_path("AAMk=", "/calendarView"),
        &[
            query_item("startDateTime", "2026-09-25T17:00:00Z"),
            query_item("$top", "100"),
            query_item("$select", "id,iCalUId,subject"),
        ],
    )
    .unwrap();
    assert_eq!(
        events.as_str(),
        "https://graph.microsoft.com/v1.0/me/calendars/AAMk%3D/calendarView?startDateTime=2026-09-25T17:00:00Z&$top=100&$select=id,iCalUId,subject"
    );
    let filter = GraphApiClient::url(
        "/me/calendars/c/events",
        &[
            query_item("$filter", "iCalUId eq 'a''b&c#d%é+'"),
            query_item("$top", "50"),
        ],
    )
    .unwrap();
    assert_eq!(
        filter.as_str(),
        "https://graph.microsoft.com/v1.0/me/calendars/c/events?$filter=iCalUId%20eq%20'a''b%26c%23d%25%C3%A9+'&$top=50"
    );
    assert_eq!(
        GraphApiClient::url("/me", &[query_item("$select", "mail,userPrincipalName")])
            .unwrap()
            .as_str(),
        "https://graph.microsoft.com/v1.0/me?$select=mail,userPrincipalName"
    );
    let none: [QueryItem; 0] = [];
    assert_eq!(
        GraphApiClient::url("/me/calendars", &none)
            .unwrap()
            .as_str(),
        "https://graph.microsoft.com/v1.0/me/calendars"
    );
}

#[tokio::test]
async fn the_events_request_carries_the_window_and_the_select_list() {
    let h = SourceHarness::with_calendars(calendars_json_ids(&["cal1"])).await;
    h.transport
        .route("cal1/calendarView", vec![ok(json!({"value": []}))]);
    let window = one_hour_window();
    h.source.events_in(window).await.unwrap();
    let request = h
        .transport
        .requests_matching("/calendarView")
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(
        request.url.as_str(),
        format!(
            "https://graph.microsoft.com/v1.0/me/calendars/cal1/calendarView?startDateTime={}&endDateTime={}&$top=100&$select={}",
            GraphTime::instant_text(window.start),
            GraphTime::instant_text(window.end),
            GraphEventMapper::EVENT_SELECT
        )
    );
    assert_eq!(request.method, "GET");
    assert_eq!(
        request.headers.get("Accept").map(String::as_str),
        Some("application/json")
    );
}

// The backoff.

#[tokio::test]
async fn a_throttle_without_retry_after_backs_off_exponentially_with_jitter() {
    let h = ClientHarness::new().await;
    h.transport.route(
        "me",
        vec![
            status(json!({}), 503),
            status(json!({}), 503),
            status(json!({}), 503),
            ok(json!({"ok": true})),
        ],
    );
    h.client
        .get(&GraphApiClient::url("/me", &[]).unwrap(), &[])
        .await
        .unwrap();
    let sleeps = h.sleeps.durations();
    assert_eq!(sleeps.len(), 3);
    for (index, seconds) in sleeps.iter().enumerate() {
        let base = f64::from(1u32 << index);
        assert!(
            (base * 0.5..=base).contains(seconds),
            "retry {index}: {seconds}"
        );
    }
}

#[tokio::test]
async fn a_retry_after_that_is_not_a_finite_number_is_ignored() {
    for text in ["soon", "inf", "NaN", ""] {
        let h = ClientHarness::new().await;
        h.transport.route(
            "me",
            vec![
                with_headers(json!({}), 429, &[("Retry-After", text)]),
                ok(json!({"ok": true})),
            ],
        );
        h.client
            .get(&GraphApiClient::url("/me", &[]).unwrap(), &[])
            .await
            .unwrap();
        let sleeps = h.sleeps.durations();
        assert!(
            sleeps.len() == 1 && sleeps[0].is_finite() && (0.5..=1.0).contains(&sleeps[0]),
            "{text:?}: {sleeps:?}"
        );
    }
}

#[tokio::test]
async fn a_link_on_a_look_alike_host_or_scheme_is_refused() {
    for link in [
        "http://graph.microsoft.com/v1.0/x",
        "https://graph.microsoft.com.evil.example/v1.0/x",
        "https://evil.example/graph.microsoft.com",
        "https://Graph.Microsoft.com/v1.0/x",
        "HTTPS://graph.microsoft.com/v1.0/x",
    ] {
        let h = ClientHarness::new().await;
        h.transport.route(
            "/me/calendars",
            vec![ok(json!({"value": [], "@odata.nextLink": link}))],
        );
        let result = h
            .client
            .pages::<GraphListPage<GraphIDDTO>, _>(
                &GraphApiClient::url("/me/calendars", &[]).unwrap(),
                &[],
                |_| Ok(()),
            )
            .await;
        assert!(result.is_err(), "{link}");
        assert_eq!(h.transport.requests().len(), 1, "{link}");
    }
}

// The delta position.

#[test]
fn a_delta_position_with_a_non_finite_baseline_is_not_a_position() {
    for stored in [
        "nan|https://x/y",
        "inf|https://x/y",
        "-inf|https://x/y",
        "1e999|https://x/y",
        "|https://x/y",
    ] {
        assert!(DeltaState::parse(stored).is_none(), "{stored}");
    }
    let state = DeltaState::parse("1790000000.9|https://graph.microsoft.com/v1.0/x").unwrap();
    // `Int(timeIntervalSince1970)` truncates.
    assert!(state.stored().starts_with("1790000000|"));
}

// The redirect host.

#[test]
fn the_redirect_host_replaces_only_the_host() {
    use calendar_core::url_string::UrlString;
    let swap = |text: &str, host: &str| -> String {
        crate::connector_kind::replacing_host(&UrlString::parse(text).unwrap(), host)
            .as_str()
            .to_string()
    };
    assert_eq!(
        swap("http://127.0.0.1:53211", "localhost"),
        "http://localhost:53211"
    );
    assert_eq!(
        swap("http://127.0.0.1:53211/cb?x=1", "localhost"),
        "http://localhost:53211/cb?x=1"
    );
    assert_eq!(swap("http://127.0.0.1/", "localhost"), "http://localhost/");
    assert_eq!(
        swap("http://[::1]:5000/", "localhost"),
        "http://localhost:5000/"
    );
}

// Garbage input: an error or a value, never a panic.

#[test]
fn garbage_and_truncated_json_never_panic_the_decoders() {
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let zone = Zone::named("America/Los_Angeles").unwrap();
    for _ in 0..2000 {
        let length = (next() % 120) as usize;
        let bytes: Vec<u8> = (0..length).map(|_| (next() & 0xFF) as u8).collect();
        let _ = serde_json::from_slice::<GraphEventDTO>(&bytes);
        let _ = serde_json::from_slice::<GraphListPage<GraphEventDTO>>(&bytes);
        let _ = serde_json::from_slice::<GraphCalendarDTO>(&bytes);
        let _ = serde_json::from_slice::<GraphRecurrenceDTO>(&bytes);
        let text = String::from_utf8_lossy(&bytes).to_string();
        let _ = GraphTime::parse(&text, &zone);
        let _ = GraphTime::parse_instant(&text);
        let _ = GraphTime::date(&text);
        let _ = DeltaState::parse(&text);
    }
    let sample = br#"{"id":"e","type":"seriesMaster","start":{"dateTime":"2026-09-21T10:00:00.0000000","timeZone":"Pacific Standard Time"},"attendees":[{"emailAddress":{"address":"a@x.com"}}],"recurrence":{"pattern":{"type":"weekly","interval":1,"daysOfWeek":["friday"]},"range":{"type":"numbered","numberOfOccurrences":8}}}"#;
    for cut in 0..sample.len() {
        let _ = serde_json::from_slice::<GraphEventDTO>(&sample[..cut]);
    }
}

#[test]
fn graph_time_never_panics_on_multi_byte_or_short_text() {
    let zone = Zone::named("Europe/Berlin").unwrap();
    for text in [
        "",
        "é",
        "２０２６-09-25T10:00:00",
        "2026-09-25T10:00:0é",
        "2026-09-25é10:00:00",
        "😀😀😀😀😀😀😀😀😀😀😀",
        "2026-09-25T10:00:00😀",
        "9999-12-31T23:59:59",
        "0000-01-01T00:00:00",
        "-001-01-01T00:00:00",
        "2026-09-25T10:00:00+0",
        "2026-09-25T10:00:00+0é:00",
    ] {
        let _ = GraphTime::parse(text, &zone);
        let _ = GraphTime::parse_instant(text);
        let _ = GraphTime::date(text);
    }
}

#[test]
fn a_recurrence_with_garbage_values_never_panics_the_mapper() {
    let weird = [
        json!({"pattern": {"type": "weekly", "interval": -5, "daysOfWeek": ["nonsday", ""], "firstDayOfWeek": "x"}, "range": {"type": "endDate", "endDate": "9999-12-31", "recurrenceTimeZone": "Nope"}}),
        json!({"pattern": {"type": "absoluteMonthly", "interval": 0, "dayOfMonth": 99}, "range": {"type": "numbered", "numberOfOccurrences": -3}}),
        json!({"pattern": {"type": "relativeYearly", "index": "fifth", "month": 13}, "range": {"type": "endDate", "endDate": "junk"}}),
        json!({"pattern": {"type": "daily"}}),
        json!({"range": {"type": "numbered"}}),
    ];
    for value in weird {
        let dto: GraphRecurrenceDTO = serde_json::from_value(value).unwrap();
        let _ = GraphRecurrenceMapper::rule(&dto, &Zone::utc());
    }
    let _ = Timestamp::UNIX_EPOCH;
    let _ = decode_event(graph_event("x", json!({})));
}
