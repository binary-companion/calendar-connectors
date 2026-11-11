//! Port of `FakeGraph.swift`: a small stateful Graph server (one writable calendar `cal1`; events created, read,
//! patched, deleted and answered through the same URLs the source uses) and the writable-source conformance check
//! that runs against it. The request-shape tests use `FakeTransport` instead.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use calendar_core::change_monitor::ChangeMonitor;
use calendar_core::connection::{
    Connection, CredentialStore, InMemoryCredentialStore, InMemorySyncStateStore,
};
use calendar_core::failure::Outcome;
use calendar_core::http::{HttpRequest, HttpResponse, HttpTransport};
use calendar_core::time::{DateInterval, Zone, add_seconds};
use calendar_core::write::writable_source::WritableCalendarSource;
use calendar_oauth::{AccessTokenProvider, OAuthTokens, RefreshFunction};
use calendar_test_support::fake_transport::{TestNow, json_response};
use calendar_test_support::writable_source_conformance::WritableSourceConformance;
use jiff::Timestamp;
use serde_json::{Map, Value, json};

use super::support::{calendars_json, graph_error_message, instant_sleeper};
use crate::api_client::GraphApiClient;
use crate::graph_time::GraphTime;
use crate::source::MicrosoftCalendarSource;

type Json = Map<String, Value>;

#[derive(Default)]
struct State {
    requests: Vec<HttpRequest>,
    events: BTreeMap<String, Json>,
    counter: u64,
    /// When set, the next POST to create an event fails with this response (then the fake behaves normally again).
    fail_next_create: Option<HttpResponse>,
}

#[derive(Default)]
pub(super) struct FakeGraph {
    state: Mutex<State>,
}

fn graph_not_found() -> HttpResponse {
    graph_error_message("ErrorItemNotFound", "m", 404)
}

impl FakeGraph {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[allow(dead_code)]
    pub(super) fn requests(&self) -> Vec<HttpRequest> {
        self.lock().requests.clone()
    }

    fn create(state: &mut State, body: &Json) -> HttpResponse {
        if let Some(failure) = state.fail_next_create.take() {
            return failure;
        }
        if let Some(transaction) = body.get("transactionId").and_then(Value::as_str)
            && let Some(existing) = state.events.values().find(|event| {
                event.get("transactionId").and_then(Value::as_str) == Some(transaction)
            })
        {
            return json_response(&Value::Object(existing.clone()), 201);
        }
        state.counter += 1;
        let counter = state.counter;
        let mut event = body.clone();
        event.insert("id".to_string(), json!(format!("ev{counter}")));
        event.insert("iCalUId".to_string(), json!(format!("uid-{counter}")));
        event.insert("changeKey".to_string(), json!(format!("ck{counter}")));
        event.insert("isCancelled".to_string(), json!(false));
        event.insert(
            "type".to_string(),
            json!(
                if body.contains_key("recurrence") && !body["recurrence"].is_null() {
                    "seriesMaster"
                } else {
                    "singleInstance"
                }
            ),
        );
        state.events.insert(format!("ev{counter}"), event.clone());
        json_response(&Value::Object(event), 201)
    }

    fn patch(state: &mut State, id: &str, body: &Json) -> HttpResponse {
        let mut event = state.events.get(id).cloned().unwrap_or_default();
        for (key, value) in body {
            event.insert(key.clone(), value.clone());
        }
        state.counter += 1;
        event.insert(
            "changeKey".to_string(),
            json!(format!("ck{}", state.counter)),
        );
        state.events.insert(id.to_string(), event.clone());
        json_response(&Value::Object(event), 200)
    }

    fn act(state: &mut State, action: &str, id: &str) -> HttpResponse {
        let response = match action {
            "accept" => "accepted",
            "tentativelyAccept" => "tentativelyAccepted",
            "decline" => "declined",
            _ => return graph_error_message("BadRequest", "m", 400),
        };
        let mut event = state.events.get(id).cloned().unwrap_or_default();
        event.insert("responseStatus".to_string(), json!({"response": response}));
        let mut attendees: Vec<Value> = event
            .get("attendees")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for attendee in &mut attendees {
            let mine = attendee["emailAddress"]["address"].as_str() == Some("me@x.com");
            if mine {
                attendee["status"] = json!({"response": response});
            }
        }
        event.insert("attendees".to_string(), Value::Array(attendees));
        state.events.insert(id.to_string(), event);
        HttpResponse::new(202, Default::default(), Vec::new())
    }

    fn calendar_view(state: &State, request: &HttpRequest) -> HttpResponse {
        let items = request.url.query_items();
        let value = |name: &str| -> Option<String> {
            items
                .iter()
                .find(|(n, _)| n == name)
                .and_then(|(_, v)| v.clone())
        };
        let from = value("startDateTime").and_then(|t| GraphTime::parse_instant(&t));
        let to = value("endDateTime").and_then(|t| GraphTime::parse_instant(&t));
        let utc = Zone::utc();
        let read = |event: &Json, key: &str| -> Option<Timestamp> {
            event
                .get(key)
                .and_then(|time| time.get("dateTime"))
                .and_then(Value::as_str)
                .and_then(|text| GraphTime::parse(text, &utc))
        };
        let in_window: Vec<Value> = state
            .events
            .values()
            .filter(|event| {
                let (Some(start), Some(end)) = (read(event, "start"), read(event, "end")) else {
                    return false;
                };
                to.is_none_or(|to| start < to) && from.is_none_or(|from| end > from)
            })
            .map(|event| Value::Object(event.clone()))
            .collect();
        // `events` is keyed by id, so the values already come out sorted by id.
        json_response(&json!({"value": in_window}), 200)
    }
}

#[async_trait]
impl HttpTransport for FakeGraph {
    async fn send(&self, request: HttpRequest) -> Outcome<HttpResponse> {
        let mut state = self.lock();
        state.requests.push(request.clone());
        let path = request.url.path();
        let body: Json = request
            .body
            .as_deref()
            .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        if path.ends_with("/mailboxSettings/timeZone") {
            return Ok(json_response(&json!({"value": "UTC"}), 200));
        }
        if path == "/v1.0/me/calendars" {
            return Ok(json_response(&calendars_json(), 200));
        }
        let marker = "/v1.0/me/calendars/cal1/";
        let Some(tail) = path.strip_prefix(marker) else {
            return Ok(graph_not_found());
        };
        if tail == "calendarView" {
            return Ok(Self::calendar_view(&state, &request));
        }
        if tail == "events" && request.method == "POST" {
            return Ok(Self::create(&mut state, &body));
        }
        let Some(rest) = tail.strip_prefix("events/") else {
            return Ok(graph_not_found());
        };
        let parts: Vec<&str> = rest.split('/').filter(|part| !part.is_empty()).collect();
        let Some(id) = parts.first().map(|id| (*id).to_string()) else {
            return Ok(graph_not_found());
        };
        if !state.events.contains_key(&id) {
            return Ok(graph_not_found());
        }
        if parts.len() == 2 {
            return Ok(Self::act(&mut state, parts[1], &id));
        }
        Ok(match request.method.as_str() {
            "GET" => json_response(&Value::Object(state.events[&id].clone()), 200),
            "PATCH" => Self::patch(&mut state, &id, &body),
            "DELETE" => {
                state.events.remove(&id);
                HttpResponse::new(204, Default::default(), Vec::new())
            }
            _ => graph_error_message("BadRequest", "m", 400),
        })
    }
}

/// A source over `FakeGraph`.
pub(super) struct GraphServerHarness {
    #[allow(dead_code)]
    pub(super) graph: Arc<FakeGraph>,
    pub(super) source: Arc<MicrosoftCalendarSource>,
}

impl GraphServerHarness {
    pub(super) async fn new() -> GraphServerHarness {
        let graph = Arc::new(FakeGraph::default());
        let now = TestNow::default();
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
        let clock = now.clone();
        let counter = Arc::new(AtomicUsize::new(0));
        let refresh: RefreshFunction = Arc::new(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            let expires = add_seconds(clock.date(), 3600.0);
            Box::pin(async move { Ok(OAuthTokens::new("at", expires)) })
        });
        let provider = AccessTokenProvider::new(
            "c1",
            store as Arc<dyn CredentialStore>,
            refresh,
            now.provider(),
            None,
        );
        let api = GraphApiClient {
            transport: graph.clone(),
            tokens: Arc::new(provider),
            sleep: instant_sleeper(),
        };
        let connection =
            Connection::new("microsoft", "c1", "me@x.com").with_config(&[("email", "me@x.com")]);
        let source = MicrosoftCalendarSource::new(
            connection,
            api,
            Arc::new(InMemorySyncStateStore::new()),
            ChangeMonitor::with_sleeper(instant_sleeper()),
            now.provider(),
        );
        GraphServerHarness { graph, source }
    }
}

#[tokio::test]
async fn the_source_passes_the_writable_conformance_checks() {
    let h = GraphServerHarness::new().await;
    let start = Timestamp::from_second(1_790_000_000).unwrap();
    let window = DateInterval::new(start, add_seconds(start, 7.0 * 86_400.0));
    let source: &dyn WritableCalendarSource = h.source.as_ref();
    let problems = WritableSourceConformance::violations(source, "cal1", window).await;
    assert!(problems.is_empty(), "{problems:?}");
}
