//! Shared helpers of the Google tests: the Swift `Harness` (`SourceTests.swift`), its JSON builders and the response
//! builders of `calendar-test-support`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use calendar_core::change_monitor::ChangeMonitor;
use calendar_core::connection::{
    Connection, CredentialStore, InMemoryCredentialStore, InMemorySyncStateStore, SyncStateStore,
};
use calendar_core::failure::Failure;
use calendar_core::http::{HttpRequest, HttpResponse, HttpTransport};
use calendar_core::time::{DateInterval, Sleeper};
use calendar_core::write::types::WriteError;
use calendar_oauth::{AccessTokenProvider, OAuthTokens, RefreshFunction};
use calendar_test_support::fake_transport::{
    FakeTransport, TestNow, json_response, json_response_with_headers, text_response,
};
use jiff::Timestamp;
use serde_json::{Value, json};

use crate::api_client::GoogleApiClient;
use crate::source::GoogleCalendarSource;

/// An instant from RFC 3339 text, read by `jiff` (independent of the code under test).
pub(super) fn instant(text: &str) -> Timestamp {
    text.parse().unwrap()
}

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

/// Swift's `calendarListJSON`.
pub(super) fn calendar_list_json() -> Value {
    json!({"items": [
        {"id": "me@x.com", "summary": "me@x.com", "primary": true, "accessRole": "owner", "timeZone": "UTC", "backgroundColor": "#112233"},
        {"id": "team@group.calendar.google.com", "summary": "Team", "accessRole": "reader", "timeZone": "UTC"},
        {"id": "hidden", "hidden": true},
    ]})
}

/// A calendarList response with these calendar ids (all readers, UTC).
pub(super) fn list_json(ids: &[&str]) -> Value {
    json!({"items": ids.iter().map(|id| json!({"id": id, "accessRole": "reader", "timeZone": "UTC"})).collect::<Vec<_>>()})
}

pub(super) fn event_json(id: &str, start: &str) -> Value {
    event_json_titled(id, start, "E")
}

pub(super) fn event_json_titled(id: &str, start: &str, summary: &str) -> Value {
    json!({"id": id, "summary": summary, "start": {"dateTime": start}, "end": {"dateTime": start}})
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

#[allow(dead_code)]
pub(super) struct Harness {
    pub(super) transport: Arc<FakeTransport>,
    pub(super) now: TestNow,
    pub(super) sync: Arc<InMemorySyncStateStore>,
    pub(super) store: Arc<InMemoryCredentialStore>,
    pub(super) source: Arc<GoogleCalendarSource>,
    pub(super) sleeps: Arc<SleepRecorder>,
}

impl Harness {
    pub(super) async fn new() -> Harness {
        Harness::build(
            calendar_list_json(),
            Arc::new(InMemorySyncStateStore::new()),
            None,
        )
        .await
    }

    /// `list` is what the account's calendar list returns.
    pub(super) async fn with_list(list: Value) -> Harness {
        Harness::build(list, Arc::new(InMemorySyncStateStore::new()), None).await
    }

    /// Pass a shared `sync` to simulate a relaunch.
    pub(super) async fn with_sync(list: Value, sync: Arc<InMemorySyncStateStore>) -> Harness {
        Harness::build(list, sync, None).await
    }

    /// `wrap` puts a decorator in front of the transport (which still records every request that reaches it).
    pub(super) async fn wrapped(
        list: Value,
        wrap: impl FnOnce(Arc<FakeTransport>) -> Arc<dyn HttpTransport> + 'static,
    ) -> Harness {
        Harness::build(
            list,
            Arc::new(InMemorySyncStateStore::new()),
            Some(Box::new(wrap)),
        )
        .await
    }

    #[allow(clippy::type_complexity)]
    async fn build(
        list: Value,
        sync: Arc<InMemorySyncStateStore>,
        wrap: Option<Box<dyn FnOnce(Arc<FakeTransport>) -> Arc<dyn HttpTransport>>>,
    ) -> Harness {
        let transport = Arc::new(FakeTransport::new());
        let now = TestNow::default();
        let store = Arc::new(InMemoryCredentialStore::new());
        let sleeps = Arc::new(SleepRecorder::default());
        let connection =
            Connection::new("google", "c1", "me@x.com").with_config(&[("email", "me@x.com")]);
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
        let counter = Arc::new(AtomicUsize::new(0));
        let clock = now.clone();
        let refresh: RefreshFunction = Arc::new(move |_| {
            let number = counter.fetch_add(1, Ordering::SeqCst) + 1;
            let expires = calendar_core::time::add_seconds(clock.date(), 3600.0);
            Box::pin(async move { Ok(OAuthTokens::new(format!("at{number}"), expires)) })
        });
        let provider = AccessTokenProvider::new(
            "c1",
            store.clone() as Arc<dyn CredentialStore>,
            refresh,
            now.provider(),
            None,
        );
        let sent: Arc<dyn HttpTransport> = match wrap {
            Some(wrap) => wrap(transport.clone()),
            None => transport.clone(),
        };
        let api = GoogleApiClient {
            transport: sent,
            tokens: Arc::new(provider),
            sleep: sleeps.sleeper(),
        };
        let source = GoogleCalendarSource::new(
            connection,
            api,
            sync.clone() as Arc<dyn SyncStateStore>,
            ChangeMonitor::with_sleeper(yielding_sleeper()),
        );
        transport.route("users/me/calendarList", vec![ok(list)]);
        Harness {
            transport,
            now,
            sync,
            store,
            source,
            sleeps,
        }
    }
}

/// Swift's `DateInterval(start: .now, duration: 3600)`, anchored at a fixed instant.
pub(super) fn one_hour_window() -> DateInterval {
    let start = Timestamp::from_second(1_790_000_000).unwrap();
    DateInterval::new(start, calendar_core::time::add_seconds(start, 3600.0))
}

/// Asserts that a write failed with exactly `expected` (Swift's `expectWriteError`).
pub(super) fn assert_write_error<T: std::fmt::Debug>(
    result: Result<T, Failure>,
    expected: WriteError,
) {
    assert_eq!(result.unwrap_err(), Failure::Write(expected));
}

/// A minimal Google event resource with the default etag `e1` and summary `Standup`; `extra` overrides or adds keys.
pub(super) fn google_event(id: &str, extra: Value) -> Value {
    google_event_full(id, "e1", "Standup", extra)
}

pub(super) fn google_event_full(id: &str, etag: &str, summary: &str, extra: Value) -> Value {
    let mut json = json!({
        "id": id, "etag": etag, "summary": summary,
        "start": {"dateTime": "2026-09-21T10:00:00Z", "timeZone": "UTC"},
        "end": {"dateTime": "2026-09-21T10:30:00Z", "timeZone": "UTC"},
    });
    if let Value::Object(map) = extra {
        for (key, value) in map {
            json[key] = value;
        }
    }
    json
}

/// The request body parsed as a JSON object (empty when it is not one).
pub(super) fn body_json(request: &HttpRequest) -> Value {
    request
        .body
        .as_deref()
        .and_then(|body| serde_json::from_slice::<Value>(body).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

pub(super) fn google_error(reason: &str, message: &str, status: i64) -> HttpResponse {
    json_response(
        &json!({"error": {"errors": [{"reason": reason}], "message": message}}),
        status,
    )
}

pub(super) fn google_error_with_headers(
    reason: &str,
    status: i64,
    headers: &[(&str, &str)],
) -> HttpResponse {
    json_response_with_headers(
        &json!({"error": {"errors": [{"reason": reason}], "message": "m"}}),
        status,
        headers,
    )
}

/// A bare response with a status and no body (Swift's `HTTPResponse(status:)`).
pub(super) fn bare(status: i64) -> HttpResponse {
    HttpResponse::new(status, Default::default(), Vec::new())
}

/// Swift's `Intercept`: answers `hook`'s outcome instead of the routed one when it returns one; the routed transport
/// still records the request.
pub(super) struct Intercept {
    pub(super) inner: Arc<FakeTransport>,
    #[allow(clippy::type_complexity)]
    pub(super) hook: Box<
        dyn Fn(&HttpRequest) -> Option<calendar_core::failure::Outcome<HttpResponse>> + Send + Sync,
    >,
}

#[async_trait::async_trait]
impl HttpTransport for Intercept {
    async fn send(&self, request: HttpRequest) -> calendar_core::failure::Outcome<HttpResponse> {
        let routed = self.inner.send(request.clone()).await;
        match (self.hook)(&request) {
            Some(answer) => answer,
            None => routed,
        }
    }
}
