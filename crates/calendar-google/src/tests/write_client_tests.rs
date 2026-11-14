//! Port of `WriteClientTests.swift`.

use std::collections::BTreeMap;
use std::sync::Arc;

use calendar_core::connection::{CredentialStore, InMemoryCredentialStore};
use calendar_core::failure::Failure;
use calendar_core::source_types::SourceError;
use calendar_core::time::add_seconds;
use calendar_oauth::{AccessTokenProvider, OAuthTokens, RefreshFunction};
use calendar_test_support::fake_transport::{FakeTransport, TestNow};
use serde_json::json;

use super::support::{bare, google_error, instant_sleeper, ok};
use crate::api_client::{
    ApiResult, GoogleApiClient, GoogleApiError, GoogleError, GoogleRequestMode, query_item,
};

async fn make_client(transport: &Arc<FakeTransport>) -> GoogleApiClient {
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
    let now = TestNow::default();
    let clock = now.clone();
    let refresh: RefreshFunction = Arc::new(move |_| {
        let expires = add_seconds(clock.date(), 3600.0);
        Box::pin(async move { Ok(OAuthTokens::new("at", expires)) })
    });
    let provider = AccessTokenProvider::new("c1", store, refresh, now.provider(), None);
    GoogleApiClient {
        transport: transport.clone(),
        tokens: Arc::new(provider),
        sleep: instant_sleeper(),
    }
}

async fn write(client: &GoogleApiClient, method: &str, path: &str) -> ApiResult<Vec<u8>> {
    client
        .send(
            method,
            path,
            &[],
            None,
            &BTreeMap::new(),
            GoogleRequestMode::Write,
        )
        .await
}

fn source(error: SourceError) -> GoogleError {
    GoogleError::Other(Failure::Source(error))
}

#[tokio::test]
async fn send_posts_a_body_with_json_headers_and_extra_headers() {
    let transport = Arc::new(FakeTransport::new());
    transport.route("events", vec![ok(json!({"id": "x"}))]);
    let client = make_client(&transport).await;
    client
        .send(
            "PATCH",
            "/calendars/c/events/e",
            &[query_item("sendUpdates", "none")],
            Some(br#"{"summary":"S"}"#.to_vec()),
            &BTreeMap::from([("If-Match".to_string(), "e1".to_string())]),
            GoogleRequestMode::Write,
        )
        .await
        .unwrap();
    let request = transport.requests().last().cloned().unwrap();
    assert!(
        request.method == "PATCH"
            && request
                .url
                .as_str()
                .ends_with("/calendars/c/events/e?sendUpdates=none")
    );
    assert!(
        request.headers.get("Content-Type").map(String::as_str) == Some("application/json")
            && request.headers.get("If-Match").map(String::as_str) == Some("e1")
    );
    assert!(
        request.headers.get("Authorization").map(String::as_str) == Some("Bearer at")
            && request.body.as_deref() == Some(br#"{"summary":"S"}"#.as_slice())
    );
}

#[tokio::test]
async fn a_stale_if_match_surfaces_as_precondition_failed() {
    let transport = Arc::new(FakeTransport::new());
    transport.route("events", vec![google_error("conditionNotMet", "m", 412)]);
    let client = make_client(&transport).await;
    let error = write(&client, "PATCH", "/calendars/c/events/e")
        .await
        .unwrap_err();
    assert_eq!(error, GoogleError::Api(GoogleApiError::PreconditionFailed));
}

#[tokio::test]
async fn write_mode_maps_any_other_forbidden_reason_but_read_mode_keeps_its_behaviour() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(
        "events",
        vec![google_error("forbiddenForNonOrganizer", "m", 403)],
    );
    let client = make_client(&transport).await;
    let written = write(&client, "PATCH", "/calendars/c/events/e")
        .await
        .unwrap_err();
    assert_eq!(written, GoogleError::Api(GoogleApiError::Forbidden));
    let read = client.get("/calendars/c/events/e", &[]).await.unwrap_err();
    assert_eq!(
        read,
        source(SourceError::InvalidResponse(
            "HTTP 403: forbiddenForNonOrganizer".to_string()
        ))
    );
}

#[tokio::test]
async fn insufficient_permissions_is_still_auth_expired_in_write_mode() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(
        "events",
        vec![google_error("insufficientPermissions", "m", 403)],
    );
    let client = make_client(&transport).await;
    let error = write(&client, "POST", "/calendars/c/events")
        .await
        .unwrap_err();
    assert_eq!(error, source(SourceError::AuthExpired));
}

#[tokio::test]
async fn write_mode_maps_bad_request_with_its_message() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(
        "events",
        vec![google_error("invalid", "Invalid start time", 400)],
    );
    let client = make_client(&transport).await;
    let error = write(&client, "POST", "/calendars/c/events")
        .await
        .unwrap_err();
    assert_eq!(
        error,
        GoogleError::Api(GoogleApiError::BadRequest("Invalid start time".to_string()))
    );
}

#[tokio::test]
async fn gone_and_not_found_keep_their_meaning() {
    let transport = Arc::new(FakeTransport::new());
    transport.route("gone", vec![google_error("deleted", "m", 410)]);
    transport.route("missing", vec![google_error("notFound", "m", 404)]);
    let client = make_client(&transport).await;
    assert_eq!(
        client.get("/gone", &[]).await.unwrap_err(),
        GoogleError::Api(GoogleApiError::Gone)
    );
    assert_eq!(
        write(&client, "DELETE", "/missing").await.unwrap_err(),
        GoogleError::Api(GoogleApiError::NotFound)
    );
}

#[tokio::test]
async fn a_no_content_response_succeeds() {
    let transport = Arc::new(FakeTransport::new());
    transport.route("events", vec![bare(204)]);
    let client = make_client(&transport).await;
    let data = write(&client, "DELETE", "/calendars/c/events/e")
        .await
        .unwrap();
    assert!(data.is_empty());
}

#[test]
fn event_path_encodes_both_ids() {
    assert_eq!(
        GoogleApiClient::event_path("me@x.com", "a_b/c"),
        "/calendars/me%40x.com/events/a_b%2Fc"
    );
}

#[tokio::test]
async fn a_read_never_maps_a_412_to_precondition_failed() {
    let transport = Arc::new(FakeTransport::new());
    transport.route("events", vec![google_error("conditionNotMet", "m", 412)]);
    let client = make_client(&transport).await;
    let error = client.get("/calendars/c/events", &[]).await.unwrap_err();
    assert_eq!(
        error,
        source(SourceError::InvalidResponse("HTTP 412".to_string()))
    );
}

#[tokio::test]
async fn a_read_keeps_mapping_a_bad_request_to_a_generic_error() {
    let transport = Arc::new(FakeTransport::new());
    transport.route("events", vec![google_error("invalid", "m", 400)]);
    let client = make_client(&transport).await;
    let error = client.get("/calendars/c/events", &[]).await.unwrap_err();
    assert_eq!(
        error,
        source(SourceError::InvalidResponse("HTTP 400".to_string()))
    );
}

#[tokio::test]
async fn percent_encode_escapes_non_ascii_instead_of_trapping_in_the_url() {
    assert_eq!(GoogleApiClient::percent_encode("café"), "caf%C3%A9");
    let transport = Arc::new(FakeTransport::new());
    transport.route("events", vec![ok(json!({}))]);
    let client = make_client(&transport).await;
    write(
        &client,
        "GET",
        &GoogleApiClient::event_path("kalender-ü@x.de", "é"),
    )
    .await
    .unwrap();
    let request = transport.requests().last().cloned().unwrap();
    assert!(
        request
            .url
            .as_str()
            .ends_with("/calendars/kalender-%C3%BC%40x.de/events/%C3%A9")
    );
}

#[tokio::test]
async fn an_empty_query_leaves_no_trailing_question_mark() {
    let transport = Arc::new(FakeTransport::new());
    transport.route("events", vec![ok(json!({}))]);
    let client = make_client(&transport).await;
    write(&client, "DELETE", "/calendars/c/events/e")
        .await
        .unwrap();
    assert!(
        transport
            .requests()
            .last()
            .unwrap()
            .url
            .as_str()
            .ends_with("/events/e")
    );
}

#[tokio::test]
async fn a_rate_limited_write_is_retried_then_succeeds() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(
        "events",
        vec![
            google_error("rateLimitExceeded", "m", 403),
            ok(json!({"id": "x"})),
        ],
    );
    let client = make_client(&transport).await;
    client
        .send(
            "POST",
            "/calendars/c/events",
            &[],
            Some(b"{}".to_vec()),
            &BTreeMap::new(),
            GoogleRequestMode::Write,
        )
        .await
        .unwrap();
    assert_eq!(transport.requests().len(), 2);
}

#[tokio::test]
async fn a_write_401_refreshes_once_then_gives_up() {
    let transport = Arc::new(FakeTransport::new());
    transport.route("events", vec![bare(401)]);
    let client = make_client(&transport).await;
    let error = write(&client, "POST", "/calendars/c/events")
        .await
        .unwrap_err();
    assert_eq!(error, source(SourceError::AuthExpired));
    assert_eq!(transport.requests().len(), 2);
}

#[tokio::test]
async fn a_server_error_on_a_write_is_not_retried() {
    let transport = Arc::new(FakeTransport::new());
    transport.route("events", vec![bare(503)]);
    let client = make_client(&transport).await;
    let error = write(&client, "POST", "/calendars/c/events")
        .await
        .unwrap_err();
    assert_eq!(error, source(SourceError::Server { status: 503 }));
    assert_eq!(transport.requests().len(), 1);
}

#[tokio::test]
async fn a_read_classifies_a_403_by_its_first_error_only() {
    // The first error has no reason, the second says `forbidden`: a read keeps judging the first error alone.
    let transport = Arc::new(FakeTransport::new());
    transport.route(
        "events",
        vec![calendar_test_support::fake_transport::json_response(
            &json!({"error": {"errors": [{"message": "no reason"}, {"reason": "forbidden"}], "message": "m"}}),
            403,
        )],
    );
    let client = make_client(&transport).await;
    let error = client.get("/calendars/c/events", &[]).await.unwrap_err();
    assert_eq!(
        error,
        source(SourceError::InvalidResponse(
            "HTTP 403: unknown".to_string()
        ))
    );
}

#[tokio::test]
async fn a_write_409_is_a_typed_conflict_but_a_read_keeps_its_generic_error() {
    let transport = Arc::new(FakeTransport::new());
    transport.route("events", vec![google_error("duplicate", "m", 409)]);
    let client = make_client(&transport).await;
    let written = write(&client, "POST", "/calendars/c/events")
        .await
        .unwrap_err();
    assert_eq!(written, GoogleError::Api(GoogleApiError::Conflict));
    let read = client.get("/calendars/c/events", &[]).await.unwrap_err();
    assert_eq!(
        read,
        source(SourceError::InvalidResponse("HTTP 409".to_string()))
    );
}
