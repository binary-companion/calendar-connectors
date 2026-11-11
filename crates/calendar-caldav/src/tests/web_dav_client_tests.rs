//! Port of `WebDAVClientTests.swift`.

use std::collections::BTreeMap;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use calendar_core::failure::Failure;
use calendar_core::http::HttpResponse;
use calendar_core::source_types::SourceError;
use calendar_core::url_string::UrlString;
use calendar_test_support::fake_transport::FakeTransport;

use crate::webdav_client::{CredentialProvider, WebDavClient, WebDavCredentials};

fn secret() -> WebDavCredentials {
    WebDavCredentials {
        username: "me@icloud.test".to_string(),
        password: "pä:ss wörd".to_string(),
    }
}

fn client(transport: &Arc<FakeTransport>) -> WebDavClient {
    let credentials: CredentialProvider = Arc::new(|| Box::pin(async { Ok(secret()) }));
    WebDavClient::new(transport.clone(), "icloud.com", credentials)
}

fn url(text: &str) -> UrlString {
    UrlString::parse(text).unwrap()
}

fn response(status: i64) -> HttpResponse {
    HttpResponse::new(status, BTreeMap::new(), Vec::new())
}

fn response_with(status: i64, header: (&str, &str)) -> HttpResponse {
    HttpResponse::new(
        status,
        BTreeMap::from([(header.0.to_string(), header.1.to_string())]),
        Vec::new(),
    )
}

fn multistatus_empty() -> HttpResponse {
    HttpResponse::new(
        207,
        BTreeMap::new(),
        b"<multistatus xmlns=\"DAV:\"/>".to_vec(),
    )
}

#[tokio::test]
async fn basic_auth_encodes_utf8_and_colons() {
    assert_eq!(
        WebDavClient::basic_authorization(&secret()),
        "Basic ".to_string() + &STANDARD.encode("me@icloud.test:pä:ss wörd".as_bytes())
    );
    let transport = Arc::new(FakeTransport::new());
    transport.route("caldav.icloud.com", vec![response(401)]);
    let error = client(&transport)
        .send("PROPFIND", &url("https://caldav.icloud.com/"))
        .await
        .unwrap_err();
    assert_eq!(error, Failure::Source(SourceError::AuthExpired));
    // The password appears in neither the text nor the debug form of the error.
    assert!(!error.to_string().contains("wörd") && !format!("{error:?}").contains("wörd"));
    let sent = transport.requests();
    assert_eq!(
        sent.first().unwrap().headers.get("Authorization"),
        Some(&WebDavClient::basic_authorization(&secret()))
    );
    // And the credentials value does not print its password.
    assert!(!format!("{:?}", secret()).contains("wörd"));
}

#[test]
fn host_rule_matches_at_a_label_boundary() {
    let allowed = |text: &str, base: &str| WebDavClient::is_allowed(&url(text), base);
    assert!(allowed("https://caldav.icloud.com/", "icloud.com"));
    assert!(allowed("https://p42-caldav.icloud.com/x", "icloud.com"));
    assert!(allowed("https://ICLOUD.com/", "icloud.com"));
    assert!(!allowed("https://evilicloud.com/", "icloud.com"));
    assert!(!allowed("https://icloud.com.evil.test/", "icloud.com"));
    assert!(!allowed("http://caldav.icloud.com/", "icloud.com"));
    assert!(allowed("http://localhost:8008/", "localhost"));
    assert!(allowed("http://127.0.0.1:8008/", "127.0.0.1"));
    assert!(!allowed("ftp://caldav.icloud.com/", "icloud.com"));
}

#[tokio::test]
async fn follows_redirects_inside_the_base() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(
        "https://caldav.icloud.com/.well-known/caldav",
        vec![response_with(
            301,
            ("Location", "https://p42-caldav.icloud.com/"),
        )],
    );
    transport.route("https://p42-caldav.icloud.com/", vec![multistatus_empty()]);
    let headers = BTreeMap::from([("Depth".to_string(), "0".to_string())]);
    let reply = client(&transport)
        .send_with(
            "PROPFIND",
            &url("https://caldav.icloud.com/.well-known/caldav"),
            headers,
            Some(b"<x/>".to_vec()),
        )
        .await
        .unwrap();
    assert_eq!(reply.response.status, 207);
    assert_eq!(reply.url.as_str(), "https://p42-caldav.icloud.com/");
    let second = transport.requests().pop().unwrap();
    assert!(second.method == "PROPFIND" && second.body == Some(b"<x/>".to_vec()));
    assert_eq!(second.headers.get("Depth").map(String::as_str), Some("0"));
}

#[tokio::test]
async fn refuses_a_redirect_to_another_host_without_sending_credentials() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(
        "https://caldav.icloud.com/",
        vec![response_with(
            302,
            ("Location", "https://collector.evil.test/steal"),
        )],
    );
    let result = client(&transport)
        .send("GET", &url("https://caldav.icloud.com/"))
        .await;
    assert!(matches!(result, Err(Failure::Source(_))));
    assert!(transport.requests_matching("evil.test").is_empty());
}

#[tokio::test]
async fn refuses_a_downgrade_to_http() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(
        "https://caldav.icloud.com/",
        vec![response_with(
            301,
            ("Location", "http://caldav.icloud.com/"),
        )],
    );
    let result = client(&transport)
        .send("GET", &url("https://caldav.icloud.com/"))
        .await;
    assert!(matches!(result, Err(Failure::Source(_))));
    assert_eq!(transport.requests().len(), 1);
}

#[tokio::test]
async fn stops_after_five_redirects() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(
        "https://caldav.icloud.com/",
        vec![response_with(307, ("Location", "/again"))],
    );
    let result = client(&transport)
        .send("GET", &url("https://caldav.icloud.com/"))
        .await;
    assert!(matches!(result, Err(Failure::Source(_))));
    assert_eq!(transport.requests().len(), 6);
}

#[tokio::test]
async fn see_other_becomes_a_get() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(
        "https://caldav.icloud.com/a",
        vec![response_with(303, ("Location", "/b"))],
    );
    transport.route("https://caldav.icloud.com/b", vec![response(200)]);
    client(&transport)
        .send_with(
            "PUT",
            &url("https://caldav.icloud.com/a"),
            BTreeMap::new(),
            Some(b"x".to_vec()),
        )
        .await
        .unwrap();
    let last = transport.requests().pop().unwrap();
    assert!(last.method == "GET" && last.body.is_none());
}

#[tokio::test]
async fn maps_error_statuses() {
    let transport = Arc::new(FakeTransport::new());
    transport.route("/limited", vec![response_with(429, ("Retry-After", "7"))]);
    transport.route("/down", vec![response(503)]);
    transport.route("/broken", vec![response(500)]);
    transport.route("/full", vec![response(507)]);
    transport.route("/missing", vec![response(404)]);
    let c = client(&transport);
    let status = |path: &'static str| {
        let c = c.clone();
        async move {
            match c
                .send("GET", &url(&format!("https://caldav.icloud.com{path}")))
                .await
            {
                Ok(_) => None,
                Err(Failure::Source(error)) => Some(error),
                Err(other) => panic!("unexpected {other:?}"),
            }
        }
    };
    assert_eq!(
        status("/limited").await,
        Some(SourceError::RateLimited {
            retry_after: Some(7.0)
        })
    );
    assert_eq!(
        status("/down").await,
        Some(SourceError::RateLimited { retry_after: None })
    );
    assert_eq!(
        status("/broken").await,
        Some(SourceError::Server { status: 500 })
    );
    assert_eq!(
        status("/full").await,
        Some(SourceError::Server { status: 507 })
    );
    // Returned to the caller.
    assert_eq!(status("/missing").await, None);
}

#[tokio::test]
async fn refuses_to_send_credentials_over_http_or_to_another_host() {
    let transport = Arc::new(FakeTransport::new());
    assert!(
        client(&transport)
            .send("GET", &url("http://caldav.icloud.com/"))
            .await
            .is_err()
    );
    assert!(
        client(&transport)
            .send("GET", &url("https://example.test/"))
            .await
            .is_err()
    );
    assert!(transport.requests().is_empty());
}

#[test]
fn resolves_hrefs_and_refuses_foreign_ones() {
    let c = client(&Arc::new(FakeTransport::new()));
    let base = url("https://p42-caldav.icloud.com/123/calendars/");
    assert_eq!(
        c.resolve("/123/calendars/home/", &base).unwrap().as_str(),
        "https://p42-caldav.icloud.com/123/calendars/home/"
    );
    assert_eq!(
        c.resolve("home/a%20b.ics", &base).unwrap().as_str(),
        "https://p42-caldav.icloud.com/123/calendars/home/a%20b.ics"
    );
    assert_eq!(
        c.resolve("https://p07-caldav.icloud.com/123/principal/", &base)
            .unwrap()
            .host()
            .as_deref(),
        Some("p07-caldav.icloud.com")
    );
    assert!(c.resolve("https://evil.test/123/", &base).is_err());
}

/// The password reaches neither an error from a refused request nor a refused redirect.
#[tokio::test]
async fn no_error_carries_the_password() {
    let transport = Arc::new(FakeTransport::new());
    transport.route(
        "https://caldav.icloud.com/",
        vec![response_with(
            302,
            ("Location", "https://collector.evil.test/steal"),
        )],
    );
    let errors = [
        client(&transport)
            .send("GET", &url("https://caldav.icloud.com/"))
            .await
            .unwrap_err(),
        client(&transport)
            .send("GET", &url("http://caldav.icloud.com/"))
            .await
            .unwrap_err(),
        client(&transport)
            .send("GET", &url("https://other.test/"))
            .await
            .unwrap_err(),
    ];
    for error in errors {
        let shown = format!("{error} {error:?}");
        assert!(
            !shown.contains("wörd") && !shown.contains("pä:ss"),
            "{shown}"
        );
    }
}
