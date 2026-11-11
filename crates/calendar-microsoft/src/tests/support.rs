//! Shared helpers of the Microsoft tests: Swift's `Harness.swift` (the client and source harnesses, the error builder,
//! the calendars JSON) and the event builder of `MapperTests.swift`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use calendar_core::change_monitor::ChangeMonitor;
use calendar_core::connection::{
    Connection, CredentialStore, InMemoryCredentialStore, InMemorySyncStateStore, SyncStateStore,
};
use calendar_core::http::{HttpRequest, HttpResponse, HttpTransport};
use calendar_core::time::{DateInterval, Sleeper};
use calendar_oauth::{AccessTokenProvider, OAuthTokens, RefreshFunction};
use calendar_test_support::fake_transport::{
    FakeTransport, TestNow, json_response, json_response_with_headers, text_response,
};
use jiff::Timestamp;
use serde_json::{Map, Value, json};

use crate::api_client::GraphApiClient;
use crate::dtos::GraphEventDTO;
use crate::source::MicrosoftCalendarSource;

pub(super) fn ok(value: Value) -> HttpResponse {
    json_response(&value, 200)
}

pub(super) fn status(value: Value, status: i64) -> HttpResponse {
    json_response(&value, status)
}

pub(super) fn with_headers(value: Value, status: i64, headers: &[(&str, &str)]) -> HttpResponse {
    json_response_with_headers(&value, status, headers)
}

pub(super) fn text(text: &str, status: i64) -> HttpResponse {
    text_response(text, status)
}

/// Graph's error body, message "m".
pub(super) fn graph_error(code: &str, status: i64) -> HttpResponse {
    graph_error_message(code, "m", status)
}

pub(super) fn graph_error_message(code: &str, message: &str, status: i64) -> HttpResponse {
    json_response(
        &json!({"error": {"code": code, "message": message}}),
        status,
    )
}

/// Swift's `bodyJSON`: the request body as a JSON object (empty when it is not one).
pub(super) fn body_json(request: &HttpRequest) -> Map<String, Value> {
    request
        .body
        .as_deref()
        .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
}

/// Swift's `calendarsJSON`.
pub(super) fn calendars_json() -> Value {
    json!({"value": [
        {"id": "cal1", "name": "Calendar", "hexColor": "#112233", "isDefaultCalendar": true, "canEdit": true,
         "canShare": true, "canViewPrivateItems": true, "owner": {"name": "Me", "address": "me@x.com"}},
        {"id": "cal2", "name": "Team", "isDefaultCalendar": false, "canEdit": false, "canShare": false,
         "canViewPrivateItems": false, "owner": {"name": "Boss", "address": "boss@x.com"}},
    ]})
}

/// A calendars response with these calendar ids (all read-only).
pub(super) fn calendars_json_ids(ids: &[&str]) -> Value {
    json!({"value": ids.iter().map(|id| json!({"id": id, "name": id, "canEdit": false})).collect::<Vec<_>>()})
}

/// A Graph event as `calendarView` returns it with `Prefer: outlook.timezone="Pacific Standard Time"`; `extra`
/// (a JSON object) overrides or adds keys.
pub(super) fn graph_event(id: &str, extra: Value) -> Value {
    let mut json = json!({
        "id": id, "iCalUId": format!("uid-{id}"), "subject": "Standup", "type": "singleInstance", "isAllDay": false,
        "isCancelled": false,
        "start": {"dateTime": "2026-09-25T10:00:00.0000000", "timeZone": "Pacific Standard Time"},
        "end": {"dateTime": "2026-09-25T10:30:00.0000000", "timeZone": "Pacific Standard Time"},
        "originalStartTimeZone": "Pacific Standard Time", "showAs": "busy", "sensitivity": "normal",
        "changeKey": "ck1", "lastModifiedDateTime": "2026-09-24T12:00:00.1234567Z",
        "createdDateTime": "2026-09-01T08:00:00Z",
        "webLink": "https://outlook.office.com/calendar/item/x", "isReminderOn": true, "reminderMinutesBeforeStart": 10,
        "body": {"contentType": "text", "content": "Agenda"}, "location": {"displayName": "Room 1"},
        "organizer": {"emailAddress": {"name": "Boss", "address": "boss@x.com"}}, "isOrganizer": false,
        "attendees": [
            {"type": "required", "status": {"response": "accepted"}, "emailAddress": {"name": "Me", "address": "Me@X.com"}},
            {"type": "optional", "status": {"response": "tentativelyAccepted"}, "emailAddress": {"name": "Opt", "address": "opt@x.com"}},
            {"type": "resource", "status": {"response": "none"}, "emailAddress": {"name": "Room", "address": "room@x.com"}},
        ],
    });
    if let (Some(target), Some(extra)) = (json.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            target.insert(key.clone(), value.clone());
        }
    }
    json
}

pub(super) fn decode_event(json: Value) -> GraphEventDTO {
    serde_json::from_value(json).unwrap()
}

/// Swift's `SleepRecorder`: the seconds each sleep was asked for.
#[derive(Default)]
pub(super) struct SleepRecorder {
    durations: Mutex<Vec<f64>>,
}

impl SleepRecorder {
    pub(super) fn durations(&self) -> Vec<f64> {
        self.durations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn sleeper(self: &Arc<Self>) -> Sleeper {
        let recorder = self.clone();
        Arc::new(move |seconds| {
            recorder
                .durations
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(seconds);
            Box::pin(async { Ok(()) })
        })
    }
}

/// A sleeper that returns at once and records nothing.
pub(super) fn instant_sleeper() -> Sleeper {
    Arc::new(|_| Box::pin(async { Ok(()) }))
}

/// A sleeper that only yields to the runtime, so a polling monitor on a single-threaded runtime lets other tasks run.
pub(super) fn yielding_sleeper() -> Sleeper {
    Arc::new(|_| {
        Box::pin(async {
            tokio::task::yield_now().await;
            Ok(())
        })
    })
}

/// A token provider that always has a valid access token ("at1", "at2", ... for each refresh).
fn counting_provider(
    store: &Arc<InMemoryCredentialStore>,
    now: &TestNow,
) -> Arc<AccessTokenProvider> {
    let counter = Arc::new(AtomicUsize::new(0));
    let clock = now.clone();
    let refresh: RefreshFunction = Arc::new(move |_| {
        let number = counter.fetch_add(1, Ordering::SeqCst) + 1;
        let expires = calendar_core::time::add_seconds(clock.date(), 3600.0);
        Box::pin(async move { Ok(OAuthTokens::new(format!("at{number}"), expires)) })
    });
    Arc::new(AccessTokenProvider::new(
        "c1",
        store.clone() as Arc<dyn CredentialStore>,
        refresh,
        now.provider(),
        None,
    ))
}

async fn store_with_refresh_token() -> Arc<InMemoryCredentialStore> {
    let store = Arc::new(InMemoryCredentialStore::new());
    store
        .set_secrets(
            [(
                AccessTokenProvider::REFRESH_TOKEN_KEY.to_string(),
                "rt".to_string(),
            )]
            .into(),
            "c1",
        )
        .await
        .unwrap();
    store
}

/// Swift's `ClientHarness`.
#[allow(dead_code)]
pub(super) struct ClientHarness {
    pub(super) transport: Arc<FakeTransport>,
    pub(super) now: TestNow,
    pub(super) store: Arc<InMemoryCredentialStore>,
    pub(super) sleeps: Arc<SleepRecorder>,
    pub(super) client: GraphApiClient,
}

impl ClientHarness {
    pub(super) async fn new() -> ClientHarness {
        let transport = Arc::new(FakeTransport::new());
        let now = TestNow::default();
        let store = store_with_refresh_token().await;
        let sleeps = Arc::new(SleepRecorder::default());
        let provider = counting_provider(&store, &now);
        let client = GraphApiClient {
            transport: transport.clone(),
            tokens: provider,
            sleep: sleeps.sleeper(),
        };
        ClientHarness {
            transport,
            now,
            store,
            sleeps,
            client,
        }
    }
}

/// Swift's `SourceHarness`: a source over a fake Graph. `mailbox_zone` is what `/me/mailboxSettings/timeZone` answers.
#[allow(dead_code)]
pub(super) struct SourceHarness {
    pub(super) transport: Arc<FakeTransport>,
    pub(super) now: TestNow,
    pub(super) sync: Arc<InMemorySyncStateStore>,
    pub(super) store: Arc<InMemoryCredentialStore>,
    pub(super) source: Arc<MicrosoftCalendarSource>,
    pub(super) sleeps: Arc<SleepRecorder>,
}

#[allow(clippy::type_complexity)]
impl SourceHarness {
    pub(super) async fn new() -> SourceHarness {
        SourceHarness::build(
            calendars_json(),
            "Pacific Standard Time",
            Arc::new(InMemorySyncStateStore::new()),
            None,
        )
        .await
    }

    pub(super) async fn with_calendars(calendars: Value) -> SourceHarness {
        SourceHarness::build(
            calendars,
            "Pacific Standard Time",
            Arc::new(InMemorySyncStateStore::new()),
            None,
        )
        .await
    }

    pub(super) async fn with_mailbox_zone(zone: &str) -> SourceHarness {
        SourceHarness::build(
            calendars_json(),
            zone,
            Arc::new(InMemorySyncStateStore::new()),
            None,
        )
        .await
    }

    /// Pass a shared `sync` to simulate a relaunch.
    pub(super) async fn with_sync(
        calendars: Value,
        sync: Arc<InMemorySyncStateStore>,
    ) -> SourceHarness {
        SourceHarness::build(calendars, "Pacific Standard Time", sync, None).await
    }

    /// `wrap` puts a decorator in front of the transport (which still records every request that reaches it).
    pub(super) async fn wrapped(
        calendars: Value,
        wrap: impl FnOnce(Arc<FakeTransport>) -> Arc<dyn HttpTransport> + 'static,
    ) -> SourceHarness {
        SourceHarness::build(
            calendars,
            "Pacific Standard Time",
            Arc::new(InMemorySyncStateStore::new()),
            Some(Box::new(wrap)),
        )
        .await
    }

    async fn build(
        calendars: Value,
        mailbox_zone: &str,
        sync: Arc<InMemorySyncStateStore>,
        wrap: Option<Box<dyn FnOnce(Arc<FakeTransport>) -> Arc<dyn HttpTransport>>>,
    ) -> SourceHarness {
        let transport = Arc::new(FakeTransport::new());
        let now = TestNow::default();
        let store = store_with_refresh_token().await;
        let sleeps = Arc::new(SleepRecorder::default());
        let connection =
            Connection::new("microsoft", "c1", "me@x.com").with_config(&[("email", "me@x.com")]);
        let provider = counting_provider(&store, &now);
        let sent: Arc<dyn HttpTransport> = match wrap {
            Some(wrap) => wrap(transport.clone()),
            None => transport.clone(),
        };
        let api = GraphApiClient {
            transport: sent,
            tokens: provider,
            sleep: sleeps.sleeper(),
        };
        let source = MicrosoftCalendarSource::new(
            connection,
            api,
            sync.clone() as Arc<dyn SyncStateStore>,
            ChangeMonitor::with_sleeper(yielding_sleeper()),
            now.provider(),
        );
        transport.route(
            "me/mailboxSettings/timeZone",
            vec![ok(json!({"value": mailbox_zone}))],
        );
        transport.route("me/calendars?", vec![ok(calendars)]);
        SourceHarness {
            transport,
            now,
            sync,
            store,
            source,
            sleeps,
        }
    }
}

/// `DateInterval(start: Date(timeIntervalSince1970: 1_790_000_000), end: ...1_790_086_400)`.
pub(super) fn day_window() -> DateInterval {
    DateInterval::new(
        Timestamp::from_second(1_790_000_000).unwrap(),
        Timestamp::from_second(1_790_086_400).unwrap(),
    )
}

/// Swift's `DateInterval(start: .now, duration: 3600)`, anchored at a fixed instant.
pub(super) fn one_hour_window() -> DateInterval {
    let start = Timestamp::from_second(1_790_000_000).unwrap();
    DateInterval::new(start, calendar_core::time::add_seconds(start, 3600.0))
}

/// Swift's `expectWriteError`: the result is exactly this `WriteError`.
pub(super) fn expect_write_error<T: std::fmt::Debug>(
    expected: calendar_core::write::types::WriteError,
    result: calendar_core::failure::Outcome<T>,
) {
    assert_eq!(
        result.err(),
        Some(calendar_core::failure::Failure::Write(expected))
    );
}
