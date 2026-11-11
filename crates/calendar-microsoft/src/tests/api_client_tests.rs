//! Port of `APIClientTests.swift`.

use calendar_core::failure::Failure;
use calendar_core::source_types::SourceError;
use calendar_test_support::fake_transport::FakeTransport;
use serde_json::json;

use super::support::{ClientHarness, graph_error, graph_error_message, ok, status, with_headers};
use crate::api_client::{GraphApiClient, GraphApiError, GraphError};
use crate::dtos::{GraphIDDTO, GraphListPage};

fn prefer(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_string()).collect()
}

fn source_failure(error: SourceError) -> GraphError {
    GraphError::Other(Failure::Source(error))
}

#[tokio::test]
async fn sends_bearer_token_and_ask_for_immutable_ids() {
    let h = ClientHarness::new().await;
    h.transport
        .route("/me/calendars", vec![ok(json!({"value": []}))]);
    h.client
        .get(
            &GraphApiClient::url("/me/calendars", &[]).unwrap(),
            &prefer(&["outlook.timezone=\"UTC\""]),
        )
        .await
        .unwrap();
    let request = h.transport.requests().into_iter().next().unwrap();
    assert_eq!(
        request.headers.get("Authorization").map(String::as_str),
        Some("Bearer at1")
    );
    assert_eq!(
        request.headers.get("Prefer").map(String::as_str),
        Some("IdType=\"ImmutableId\", outlook.timezone=\"UTC\"")
    );
}

#[test]
fn percent_encodes_ids_in_paths() {
    assert_eq!(
        GraphApiClient::event_path("AAMk=", "AQ/x+y"),
        "/me/calendars/AAMk%3D/events/AQ%2Fx%2By"
    );
}

#[tokio::test]
async fn maps_status_codes_to_provider_errors() {
    let h = ClientHarness::new().await;
    for (code, expected) in [
        (404, GraphApiError::NotFound),
        (403, GraphApiError::Forbidden),
        (410, GraphApiError::Gone),
        (409, GraphApiError::Conflict),
        (412, GraphApiError::PreconditionFailed),
    ] {
        let transport = std::sync::Arc::new(FakeTransport::new());
        transport.route("x", vec![status(json!({}), code)]);
        let client = GraphApiClient {
            transport,
            tokens: h.client.tokens.clone(),
            sleep: h.client.sleep.clone(),
        };
        let result = client
            .get(&GraphApiClient::url("/x", &[]).unwrap(), &[])
            .await;
        assert_eq!(
            result,
            Err(GraphError::Api(expected)),
            "status {code} did not map"
        );
    }
}

#[tokio::test]
async fn a_bad_request_carries_graphs_message_and_a_resync_code_means_gone() {
    let h = ClientHarness::new().await;
    h.transport.route(
        "bad",
        vec![graph_error_message(
            "ErrorInvalidRequest",
            "start after end",
            400,
        )],
    );
    h.transport
        .route("resync", vec![graph_error("syncStateNotFound", 400)]);
    assert_eq!(
        h.client
            .get(&GraphApiClient::url("/bad", &[]).unwrap(), &[])
            .await,
        Err(GraphError::Api(GraphApiError::BadRequest(
            "start after end".to_string()
        )))
    );
    assert_eq!(
        h.client
            .get(&GraphApiClient::url("/resync", &[]).unwrap(), &[])
            .await,
        Err(GraphError::Api(GraphApiError::Gone))
    );
}

#[tokio::test]
async fn a_401_refreshes_once_then_fails_as_auth_expired() {
    let h = ClientHarness::new().await;
    h.transport
        .route("me", vec![status(json!({}), 401), ok(json!({"ok": true}))]);
    h.client
        .get(&GraphApiClient::url("/me", &[]).unwrap(), &[])
        .await
        .unwrap();
    let tokens: Vec<Option<String>> = h
        .transport
        .requests()
        .iter()
        .map(|request| request.headers.get("Authorization").cloned())
        .collect();
    assert_eq!(
        tokens,
        [
            Some("Bearer at1".to_string()),
            Some("Bearer at2".to_string())
        ]
    );

    let again = ClientHarness::new().await;
    again.transport.route("me", vec![status(json!({}), 401)]);
    assert_eq!(
        again
            .client
            .get(&GraphApiClient::url("/me", &[]).unwrap(), &[])
            .await,
        Err(source_failure(SourceError::AuthExpired))
    );
}

#[tokio::test]
async fn throttling_honors_retry_after_then_gives_up() {
    let h = ClientHarness::new().await;
    h.transport.route(
        "me",
        vec![
            with_headers(json!({}), 429, &[("Retry-After", "7")]),
            ok(json!({"ok": true})),
        ],
    );
    h.client
        .get(&GraphApiClient::url("/me", &[]).unwrap(), &[])
        .await
        .unwrap();
    assert_eq!(h.sleeps.durations(), [7.0]);

    let stuck = ClientHarness::new().await;
    stuck.transport.route(
        "me",
        vec![with_headers(json!({}), 429, &[("Retry-After", "1")])],
    );
    assert_eq!(
        stuck
            .client
            .get(&GraphApiClient::url("/me", &[]).unwrap(), &[])
            .await,
        Err(source_failure(SourceError::RateLimited {
            retry_after: Some(1.0)
        }))
    );
    // The first try and three retries.
    assert_eq!(stuck.transport.requests().len(), 4);
}

#[tokio::test]
async fn a_service_unavailable_is_retried_then_reported_as_a_server_error() {
    let h = ClientHarness::new().await;
    h.transport.route("me", vec![status(json!({}), 503)]);
    assert_eq!(
        h.client
            .get(&GraphApiClient::url("/me", &[]).unwrap(), &[])
            .await,
        Err(source_failure(SourceError::Server { status: 503 }))
    );
}

#[tokio::test]
async fn five_hundred_is_a_server_error_without_retry() {
    let h = ClientHarness::new().await;
    h.transport.route("me", vec![status(json!({}), 500)]);
    assert_eq!(
        h.client
            .get(&GraphApiClient::url("/me", &[]).unwrap(), &[])
            .await,
        Err(source_failure(SourceError::Server { status: 500 }))
    );
    assert_eq!(h.transport.requests().len(), 1);
}

#[tokio::test]
async fn pages_follow_next_links_and_refuse_a_foreign_host() {
    let h = ClientHarness::new().await;
    h.transport.route(
        "/me/calendars",
        vec![
            ok(json!({"value": [{"id": "a"}], "@odata.nextLink": "https://graph.microsoft.com/v1.0/me/calendars?$skip=1"})),
            ok(json!({"value": [{"id": "b"}]})),
        ],
    );
    let mut ids: Vec<String> = Vec::new();
    h.client
        .pages::<GraphListPage<GraphIDDTO>, _>(
            &GraphApiClient::url("/me/calendars", &[]).unwrap(),
            &[],
            |page| {
                ids.extend(page.items().into_iter().filter_map(|item| item.id));
                Ok(())
            },
        )
        .await
        .unwrap();
    assert_eq!(ids, ["a", "b"]);

    let evil = ClientHarness::new().await;
    evil.transport.route(
        "/me/calendars",
        vec![ok(
            json!({"value": [], "@odata.nextLink": "https://evil.example/steal"}),
        )],
    );
    let result = evil
        .client
        .pages::<GraphListPage<GraphIDDTO>, _>(
            &GraphApiClient::url("/me/calendars", &[]).unwrap(),
            &[],
            |_| Ok(()),
        )
        .await;
    assert_eq!(
        result,
        Err(source_failure(SourceError::InvalidResponse(
            "unexpected link host".to_string()
        )))
    );
    assert!(evil.transport.requests_matching("evil.example").is_empty());
}

#[test]
fn a_malformed_item_is_dropped_but_still_counted() {
    let data = br#"{"value":[{"id":"a"},{"id":5},{"id":"c"}],"@odata.deltaLink":"https://graph.microsoft.com/v1.0/d"}"#;
    let page: GraphListPage<GraphIDDTO> = serde_json::from_slice(data).unwrap();
    assert_eq!(page.value.as_ref().map(Vec::len), Some(3));
    let ids: Vec<String> = page
        .items()
        .into_iter()
        .filter_map(|item| item.id)
        .collect();
    assert_eq!(ids, ["a", "c"]);
    assert_eq!(
        page.delta_link.as_deref(),
        Some("https://graph.microsoft.com/v1.0/d")
    );
}
