//! Port of `CalendarTestSupport/FakeTransport.swift`: `FakeTransport`, the response builders that Swift adds to
//! `HTTPResponse` as static functions, and `TestNow`.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use calendar_core::failure::Outcome;
use calendar_core::http::{HttpRequest, HttpResponse, HttpTransport};
use calendar_core::time::{Now, add_seconds};
use jiff::Timestamp;

struct Route {
    matching: String,
    outcomes: Vec<Outcome<HttpResponse>>,
}

#[derive(Default)]
struct State {
    routes: Vec<Route>,
    requests: Vec<HttpRequest>,
}

/// Routes requests by URL substring. The MOST RECENTLY registered route whose text is contained in the request URL
/// answers (so register general routes first and specific ones after); it consumes its responses in order and
/// repeats the last one. Unmatched requests get 404 with the body `no route for <url>`.
///
/// Differences from the Swift actor: the methods are plain (not `async`), because the state is behind a mutex that is
/// never held across an `.await`; [`FakeTransport::route_outcomes`] also scripts transport errors (the Swift fake
/// only scripts responses, and Swift tests that need an error write their own transport); a route registered with no
/// responses answers 404 where Swift would trap on `responses[0]`.
#[derive(Default)]
pub struct FakeTransport {
    state: Mutex<State>,
}

impl FakeTransport {
    pub fn new() -> FakeTransport {
        FakeTransport::default()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Swift `route(_:_:)`: registers `responses` for URLs containing `matching`.
    pub fn route(&self, matching: &str, responses: Vec<HttpResponse>) {
        self.route_outcomes(matching, responses.into_iter().map(Ok).collect());
    }

    /// Like [`FakeTransport::route`], but each scripted answer may be a failure (a network error, a cancellation).
    pub fn route_outcomes(&self, matching: &str, outcomes: Vec<Outcome<HttpResponse>>) {
        self.lock().routes.push(Route {
            matching: matching.to_string(),
            outcomes,
        });
    }

    /// Swift `requests`: every request received, in order.
    pub fn requests(&self) -> Vec<HttpRequest> {
        self.lock().requests.clone()
    }

    /// Swift `requests(matching:)`: the requests whose URL contains `text`.
    pub fn requests_matching(&self, text: &str) -> Vec<HttpRequest> {
        self.lock()
            .requests
            .iter()
            .filter(|request| request.url.as_str().contains(text))
            .cloned()
            .collect()
    }
}

#[async_trait]
impl HttpTransport for FakeTransport {
    async fn send(&self, request: HttpRequest) -> Outcome<HttpResponse> {
        let mut state = self.lock();
        let url = request.url.as_str().to_string();
        state.requests.push(request);
        for route in state.routes.iter_mut().rev() {
            if url.contains(&route.matching) {
                match route.outcomes.len() {
                    // Swift traps on `responses[0]`; the nearest answer is the no-route 404, not an older route.
                    0 => break,
                    1 => return route.outcomes[0].clone(),
                    _ => return route.outcomes.remove(0),
                }
            }
        }
        Ok(HttpResponse::new(
            404,
            Default::default(),
            format!("no route for {url}").into_bytes(),
        ))
    }
}

/// Swift `HTTPResponse.json(_:status:)` with no headers. `value` is serialized with `serde_json`.
pub fn json_response(value: &serde_json::Value, status: i64) -> HttpResponse {
    json_response_with_headers(value, status, &[])
}

/// Swift `HTTPResponse.json(_:status:headers:)`.
pub fn json_response_with_headers(
    value: &serde_json::Value,
    status: i64,
    headers: &[(&str, &str)],
) -> HttpResponse {
    let body = serde_json::to_vec(value).unwrap_or_default();
    HttpResponse::new(
        status,
        headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect(),
        body,
    )
}

/// Swift `HTTPResponse.text(_:status:)`.
pub fn text_response(text: &str, status: i64) -> HttpResponse {
    HttpResponse::new(status, Default::default(), text.as_bytes().to_vec())
}

/// A controllable clock for tests: [`TestNow::provider`] is the [`Now`] closure the library takes. Clones share the
/// same clock.
#[derive(Clone)]
pub struct TestNow {
    value: Arc<Mutex<Timestamp>>,
}

impl TestNow {
    /// Swift `init(_ start:)`.
    pub fn new(start: Timestamp) -> TestNow {
        TestNow {
            value: Arc::new(Mutex::new(start)),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Timestamp> {
        self.value.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Swift `date`.
    pub fn date(&self) -> Timestamp {
        *self.lock()
    }

    /// Swift `advance(_:)`: moves the clock forward by `seconds`.
    pub fn advance(&self, seconds: f64) {
        let mut value = self.lock();
        *value = add_seconds(*value, seconds);
    }

    /// Swift `provider`: a closure that reads this clock.
    pub fn provider(&self) -> Now {
        let clock = self.clone();
        Arc::new(move || clock.date())
    }
}

impl Default for TestNow {
    /// Swift's default start, `Date(timeIntervalSince1970: 1_800_000_000)`.
    fn default() -> TestNow {
        TestNow::new(Timestamp::from_second(1_800_000_000).unwrap())
    }
}
