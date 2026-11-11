//! Direct tests of `FakeTransport` and `TestNow` (no Swift counterpart: the Swift tests of this target are the ones
//! that use the fakes).

use std::collections::BTreeMap;

use calendar_core::failure::Failure;
use calendar_core::http::{HttpRequest, HttpTransport};
use calendar_core::source_types::SourceError;
use calendar_core::url_string::UrlString;
use serde_json::json;

use super::support::at;
use crate::fake_transport::{
    FakeTransport, TestNow, json_response, json_response_with_headers, text_response,
};

fn request(url: &str) -> HttpRequest {
    HttpRequest::new(UrlString::parse(url).unwrap())
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn unmatched_request_is_404_with_the_url() {
    let transport = FakeTransport::new();
    let response = transport
        .send(request("https://example.test/a"))
        .await
        .unwrap();
    assert_eq!(response.status, 404);
    assert_eq!(
        response.body,
        b"no route for https://example.test/a".to_vec()
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn most_recent_matching_route_answers() {
    let transport = FakeTransport::new();
    transport.route("example.test", vec![text_response("general", 200)]);
    transport.route(
        "example.test/specific",
        vec![text_response("specific", 201)],
    );
    let specific = transport
        .send(request("https://example.test/specific/1"))
        .await
        .unwrap();
    assert_eq!(
        (specific.status, specific.body),
        (201, b"specific".to_vec())
    );
    let general = transport
        .send(request("https://example.test/other"))
        .await
        .unwrap();
    assert_eq!((general.status, general.body), (200, b"general".to_vec()));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn responses_are_consumed_in_order_and_the_last_repeats() {
    let transport = FakeTransport::new();
    transport.route(
        "feed",
        vec![
            text_response("one", 200),
            text_response("two", 500),
            text_response("three", 200),
        ],
    );
    let mut seen = Vec::new();
    for _ in 0..5 {
        let response = transport
            .send(request("https://x.test/feed"))
            .await
            .unwrap();
        seen.push((response.status, String::from_utf8(response.body).unwrap()));
    }
    let expected: Vec<(i64, String)> = [
        (200, "one"),
        (500, "two"),
        (200, "three"),
        (200, "three"),
        (200, "three"),
    ]
    .iter()
    .map(|(s, b)| (*s, b.to_string()))
    .collect();
    assert_eq!(seen, expected);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn scripted_errors_are_returned_and_consumed_like_responses() {
    let transport = FakeTransport::new();
    transport.route_outcomes(
        "feed",
        vec![
            Err(Failure::Source(SourceError::Network("offline".to_string()))),
            Err(Failure::Cancelled),
            Ok(text_response("fine", 200)),
        ],
    );
    let url = "https://x.test/feed";
    assert_eq!(
        transport.send(request(url)).await.unwrap_err(),
        Failure::Source(SourceError::Network("offline".to_string()))
    );
    assert_eq!(
        transport.send(request(url)).await.unwrap_err(),
        Failure::Cancelled
    );
    assert_eq!(transport.send(request(url)).await.unwrap().status, 200);
    assert_eq!(transport.send(request(url)).await.unwrap().status, 200);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_empty_route_does_not_panic_and_answers_404() {
    let transport = FakeTransport::new();
    transport.route("feed", vec![]);
    let response = transport
        .send(request("https://x.test/feed"))
        .await
        .unwrap();
    assert_eq!(response.status, 404);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn requests_are_recorded_in_order_with_their_content() {
    let transport = FakeTransport::new();
    transport.route("a", vec![text_response("", 200)]);
    let mut post = request("https://x.test/a/post");
    post.method = "POST".to_string();
    post.headers
        .insert("Authorization".to_string(), "Bearer t".to_string());
    post.body = Some(b"payload".to_vec());
    transport.send(request("https://x.test/a/1")).await.unwrap();
    transport.send(post.clone()).await.unwrap();
    transport.send(request("https://x.test/zzz")).await.unwrap();

    let all = transport.requests();
    assert_eq!(all.len(), 3);
    assert_eq!(all[1], post);
    let matching = transport.requests_matching("/a/");
    assert_eq!(matching.len(), 2);
    assert_eq!(matching[0].url.as_str(), "https://x.test/a/1");
    assert_eq!(matching[1].method, "POST");
    assert!(transport.requests_matching("nothing").is_empty());
}

#[test]
fn json_response_serializes_the_value() {
    let response = json_response(&json!({"a": [1, 2], "b": "x"}), 201);
    assert_eq!(response.status, 201);
    assert!(response.headers.is_empty());
    let parsed: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(parsed, json!({"a": [1, 2], "b": "x"}));
}

#[test]
fn json_response_with_headers_lowercases_the_names() {
    let response = json_response_with_headers(&json!([]), 200, &[("Retry-After", "30")]);
    let expected: BTreeMap<String, String> = [("retry-after".to_string(), "30".to_string())].into();
    assert_eq!(response.headers, expected);
    assert_eq!(response.header("RETRY-AFTER"), Some("30"));
}

#[test]
fn text_response_has_the_text_as_utf8_body() {
    let response = text_response("héllo", 200);
    assert_eq!(response.status, 200);
    assert_eq!(response.body, "héllo".as_bytes().to_vec());
}

#[test]
fn test_now_starts_at_the_swift_default_and_advances() {
    let now = TestNow::default();
    assert_eq!(now.date().as_second(), 1_800_000_000);
    now.advance(90.5);
    assert_eq!(now.date().as_millisecond(), 1_800_000_090_500);
}

#[test]
fn test_now_provider_follows_the_clock_and_clones_share_it() {
    let now = TestNow::new(at(100));
    let provider = now.provider();
    assert_eq!(provider().as_second(), 100);
    let alias = now.clone();
    alias.advance(60.0);
    assert_eq!(provider().as_second(), 160);
    assert_eq!(now.date().as_second(), 160);
}
