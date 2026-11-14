//! Port of `FeedFetcherTests.swift`.

use std::sync::Arc;

use async_trait::async_trait;
use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::{HttpRequest, HttpResponse, HttpTransport};
use calendar_core::source_types::SourceError;
use calendar_core::url_string::UrlString;
use calendar_test_support::fake_transport::FakeTransport;

use super::fixtures::{
    PRIVATE_PATH, ThrowingTransport, feed_url, ok, ok_with, redirect, sample_feed, status,
};
use crate::feed_fetcher::{FeedFetcher, FeedResponse, FeedValidators};

fn body_of(response: &FeedResponse) -> Option<&Vec<u8>> {
    match response {
        FeedResponse::Body(data, _) => Some(data),
        FeedResponse::NotModified => None,
    }
}

fn fetcher(transport: &Arc<FakeTransport>) -> FeedFetcher {
    FeedFetcher::new(transport.clone())
}

fn validators(etag: &str, modified: &str) -> FeedValidators {
    FeedValidators {
        etag: Some(etag.to_string()),
        last_modified: Some(modified.to_string()),
    }
}

fn invalid(message: &str) -> Failure {
    Failure::Source(SourceError::InvalidResponse(message.to_string()))
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_successful_fetch_returns_the_body_and_its_validators() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(
        PRIVATE_PATH,
        vec![ok_with(
            &sample_feed(),
            &[
                ("ETag", "\"v1\""),
                ("Last-Modified", "Mon, 05 Oct 2026 10:00:00 GMT"),
            ],
        )],
    );
    let response = fetcher(&transport).fetch(&feed_url(), None).await.unwrap();
    let FeedResponse::Body(data, got) = response else {
        panic!("expected a body");
    };
    assert_eq!(String::from_utf8_lossy(&data), sample_feed());
    assert_eq!(got, validators("\"v1\"", "Mon, 05 Oct 2026 10:00:00 GMT"));
    let request = transport.requests().into_iter().next().unwrap();
    assert_eq!(request.method, "GET");
    assert!(request.headers["Accept"].starts_with("text/calendar"));
    assert!(!request.headers.contains_key("If-None-Match"));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn validators_make_the_request_conditional_and_a_304_means_unchanged() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(PRIVATE_PATH, vec![status(304)]);
    let response = fetcher(&transport)
        .fetch(
            &feed_url(),
            Some(&validators("\"v1\"", "Mon, 05 Oct 2026 10:00:00 GMT")),
        )
        .await
        .unwrap();
    assert!(matches!(response, FeedResponse::NotModified));
    let request = transport.requests().into_iter().next().unwrap();
    assert_eq!(request.headers["If-None-Match"], "\"v1\"");
    assert_eq!(
        request.headers["If-Modified-Since"],
        "Mon, 05 Oct 2026 10:00:00 GMT"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_stray_304_without_validators_is_an_invalid_response() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(PRIVATE_PATH, vec![status(304)]);
    let error = fetcher(&transport)
        .fetch(&feed_url(), None)
        .await
        .err()
        .unwrap();
    assert_eq!(error, invalid("the feed server answered 304"));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn gone_private_links_mean_the_link_needs_replacing() {
    for code in [401, 403, 404, 410] {
        let transport = Arc::new(FakeTransport::new());
        transport.route(PRIVATE_PATH, vec![status(code)]);
        let error = fetcher(&transport)
            .fetch(&feed_url(), None)
            .await
            .err()
            .unwrap();
        assert_eq!(error, Failure::Source(SourceError::AuthExpired), "{code}");
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn throttling_and_server_errors_are_server_errors() {
    for code in [429, 500, 503] {
        let transport = Arc::new(FakeTransport::new());
        transport.route(PRIVATE_PATH, vec![status(code)]);
        let error = fetcher(&transport)
            .fetch(&feed_url(), None)
            .await
            .err()
            .unwrap();
        assert_eq!(
            error,
            Failure::Source(SourceError::Server { status: code }),
            "{code}"
        );
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn other_statuses_are_invalid_responses() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(PRIVATE_PATH, vec![status(400)]);
    let error = fetcher(&transport)
        .fetch(&feed_url(), None)
        .await
        .err()
        .unwrap();
    assert_eq!(error, invalid("the feed server answered 400"));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn https_redirects_are_followed_including_relative_ones() {
    let transport = Arc::new(FakeTransport::new());
    transport.route("other.example.test", vec![ok(&sample_feed())]);
    transport.route(
        "/relative",
        vec![ok("BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n")],
    );
    transport.route(
        PRIVATE_PATH,
        vec![redirect(302, "https://other.example.test/feed.ics")],
    );
    let response = fetcher(&transport).fetch(&feed_url(), None).await.unwrap();
    assert!(body_of(&response).is_some());
    assert_eq!(transport.requests_matching("other.example.test").len(), 1);

    let relative = Arc::new(FakeTransport::new());
    relative.route(
        "/relative",
        vec![ok("BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n")],
    );
    relative.route(PRIVATE_PATH, vec![redirect(301, "/relative")]);
    let response = fetcher(&relative).fetch(&feed_url(), None).await.unwrap();
    assert!(body_of(&response).is_some());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn redirects_to_plain_http_or_another_scheme_are_refused() {
    for location in [
        "http://other.example.test/feed.ics",
        "ftp://other.example.test/feed.ics",
        "file:///tmp/a.ics",
    ] {
        let transport = Arc::new(FakeTransport::new());
        transport.route(PRIVATE_PATH, vec![redirect(302, location)]);
        let error = fetcher(&transport)
            .fetch(&feed_url(), None)
            .await
            .err()
            .unwrap();
        assert_eq!(
            error,
            invalid("the feed moved somewhere TimeTug will not follow"),
            "{location}"
        );
        assert_eq!(transport.requests().len(), 1);
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_redirect_to_an_address_with_a_user_name_or_password_is_refused() {
    for location in [
        "https://me:pw@other.example.test/feed.ics",
        "https://me@other.example.test/feed.ics",
    ] {
        let transport = Arc::new(FakeTransport::new());
        transport.route(PRIVATE_PATH, vec![redirect(302, location)]);
        let error = fetcher(&transport)
            .fetch(&feed_url(), None)
            .await
            .err()
            .unwrap();
        assert_eq!(
            error,
            invalid("the feed moved somewhere TimeTug will not follow"),
            "{location}"
        );
        let text = format!("{error} {error:?}");
        assert!(!text.contains("other.example.test") && !text.contains(PRIVATE_PATH));
        assert_eq!(transport.requests().len(), 1);
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_redirect_with_no_location_is_refused() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(PRIVATE_PATH, vec![status(302)]);
    let error = fetcher(&transport)
        .fetch(&feed_url(), None)
        .await
        .err()
        .unwrap();
    assert_eq!(
        error,
        invalid("the feed moved somewhere TimeTug will not follow")
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_redirect_loop_stops_after_five_redirects() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(PRIVATE_PATH, vec![redirect(302, feed_url().as_str())]);
    let error = fetcher(&transport)
        .fetch(&feed_url(), None)
        .await
        .err()
        .unwrap();
    assert_eq!(error, invalid("the feed redirected too many times"));
    assert_eq!(transport.requests().len(), FeedFetcher::MAX_REDIRECTS + 1);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_oversized_body_is_refused() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(
        PRIVATE_PATH,
        vec![HttpResponse::new(
            200,
            Default::default(),
            vec![0; FeedFetcher::MAX_BYTES + 1],
        )],
    );
    let error = fetcher(&transport)
        .fetch(&feed_url(), None)
        .await
        .err()
        .unwrap();
    assert_eq!(error, invalid("the feed is too large"));

    let exactly = Arc::new(FakeTransport::new());
    exactly.route(
        PRIVATE_PATH,
        vec![HttpResponse::new(
            200,
            Default::default(),
            vec![0; FeedFetcher::MAX_BYTES],
        )],
    );
    let response = fetcher(&exactly).fetch(&feed_url(), None).await.unwrap();
    assert_eq!(
        body_of(&response).map(Vec::len),
        Some(FeedFetcher::MAX_BYTES)
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn no_error_ever_contains_the_link() {
    for code in [301, 302, 400, 401, 403, 404, 410, 429, 500, 503] {
        let transport = Arc::new(FakeTransport::new());
        transport.route(PRIVATE_PATH, vec![status(code)]);
        if let Err(error) = fetcher(&transport).fetch(&feed_url(), None).await {
            let text = format!("{error} {error:?}");
            assert!(
                !text.contains(PRIVATE_PATH) && !text.contains("example.test/events"),
                "status {code}: {text}"
            );
        }
    }
}

struct EchoingTransport {
    message: String,
}

#[async_trait]
impl HttpTransport for EchoingTransport {
    async fn send(&self, _request: HttpRequest) -> Outcome<HttpResponse> {
        Err(Failure::Source(SourceError::Network(self.message.clone())))
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_transport_message_that_echoes_only_the_query_or_the_encoded_path_is_replaced() {
    let url = UrlString::parse(
        "https://www.example.test/my%20events/PRIVATE%20X/going?token=QUERY-SECRET-77",
    )
    .unwrap();
    let echoes = [
        "bad request ?token=QUERY-SECRET-77",
        "token=QUERY-SECRET-77",
        "failed at /my%20events/PRIVATE%20X/going",
        "failed at /my events/PRIVATE X/going",
    ];
    for echo in echoes {
        let transport = Arc::new(EchoingTransport {
            message: echo.to_string(),
        });
        let error = FeedFetcher::new(transport)
            .fetch(&url, None)
            .await
            .expect_err("expected a failure");
        assert_eq!(
            error,
            Failure::Source(SourceError::Network(
                "the feed could not be reached".to_string()
            )),
            "{echo}"
        );
    }
}

/// Swift's `URLError(.cancelled)` from any transport is a cancellation. Rust transports report a cancellation as
/// `Failure::Cancelled` (the `HttpTransport` contract), so that is what must pass through unchanged.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_cancelled_url_error_from_any_transport_is_a_cancellation() {
    let transport = Arc::new(ThrowingTransport {
        failure: Failure::Cancelled,
    });
    let error = FeedFetcher::new(transport)
        .fetch(&feed_url(), None)
        .await
        .err()
        .unwrap();
    assert_eq!(error, Failure::Cancelled);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_304_when_no_conditional_header_was_sent_is_an_invalid_response() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(PRIVATE_PATH, vec![status(304)]);
    let error = fetcher(&transport)
        .fetch(&feed_url(), Some(&FeedValidators::default()))
        .await
        .err()
        .unwrap();
    assert_eq!(error, invalid("the feed server answered 304"));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn validators_are_not_sent_to_a_redirect_target() {
    let transport = Arc::new(FakeTransport::new());
    transport.route("other.example.test", vec![status(304)]);
    transport.route(
        PRIVATE_PATH,
        vec![redirect(302, "https://other.example.test/feed.ics")],
    );
    let error = fetcher(&transport)
        .fetch(
            &feed_url(),
            Some(&validators("\"v1\"", "Mon, 05 Oct 2026 10:00:00 GMT")),
        )
        .await
        .err()
        .unwrap();
    assert_eq!(error, invalid("the feed server answered 304"));
    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].headers["If-None-Match"], "\"v1\"");
    assert!(
        !requests[1].headers.contains_key("If-None-Match")
            && !requests[1].headers.contains_key("If-Modified-Since")
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_chain_of_exactly_five_redirects_is_followed() {
    let transport = Arc::new(FakeTransport::new());
    let hop = redirect(302, feed_url().as_str());
    let mut responses = vec![hop; FeedFetcher::MAX_REDIRECTS];
    responses.push(ok(&sample_feed()));
    transport.route(PRIVATE_PATH, responses);
    let response = fetcher(&transport).fetch(&feed_url(), None).await.unwrap();
    assert!(body_of(&response).is_some());
    assert_eq!(transport.requests().len(), FeedFetcher::MAX_REDIRECTS + 1);
}
