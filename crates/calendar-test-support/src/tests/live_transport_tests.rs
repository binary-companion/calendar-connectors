//! The live tests' curl transport against a local server (no account and no outside network).

#![cfg(unix)]

use std::io::{Read, Write};
use std::net::TcpListener;

use calendar_core::failure::Failure;
use calendar_core::http::{HttpRequest, HttpTransport};
use calendar_core::source_types::SourceError;
use calendar_core::url_string::UrlString;

use crate::live::CurlTransport;

/// Answers one request with `reply` and returns what the server received (head and body, as text).
fn serve_once(reply: &'static str) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut received = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let count = stream.read(&mut chunk).unwrap();
            received.extend_from_slice(&chunk[..count]);
            let text = String::from_utf8_lossy(&received).into_owned();
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let length = head
                    .lines()
                    .find_map(|l| {
                        l.to_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if body.len() >= length {
                    break;
                }
            }
            if count == 0 {
                break;
            }
        }
        stream.write_all(reply.as_bytes()).unwrap();
        String::from_utf8_lossy(&received).into_owned()
    });
    (base, handle)
}

#[tokio::test(flavor = "current_thread")]
async fn sends_method_headers_and_body_and_returns_the_answer() {
    let (base, server) = serve_once(
        "HTTP/1.1 412 Precondition Failed\r\nRetry-After: 7\r\nETag: \"abc\"\r\nContent-Length: 2\r\nConnection: close\r\n\r\nno",
    );
    let mut request =
        HttpRequest::new(UrlString::parse(&format!("{base}/path?$select=id,start")).unwrap());
    request.method = "PATCH".to_string();
    request
        .headers
        .insert("Authorization".to_string(), "Bearer t0k".to_string());
    request.headers.insert(
        "Prefer".to_string(),
        r#"outlook.timezone="Pacific Standard Time", odata.maxpagesize=200"#.to_string(),
    );
    request.body = Some(b"{\"a\":1}".to_vec());
    let response = CurlTransport.send(request).await.unwrap();
    assert_eq!(response.status, 412);
    assert_eq!(response.header("retry-after"), Some("7"));
    assert_eq!(response.header("etag"), Some("\"abc\""));
    assert_eq!(response.body, b"no");
    let received = server.join().unwrap();
    assert!(
        received.starts_with("PATCH /path?$select=id,start HTTP/1.1"),
        "{received}"
    );
    assert!(received.contains("Authorization: Bearer t0k"));
    assert!(
        received
            .contains(r#"Prefer: outlook.timezone="Pacific Standard Time", odata.maxpagesize=200"#)
    );
    assert!(received.ends_with("{\"a\":1}"));
}

#[tokio::test(flavor = "current_thread")]
async fn does_not_follow_a_redirect() {
    let (base, server) = serve_once(
        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    );
    let request = HttpRequest::new(UrlString::parse(&format!("{base}/start")).unwrap());
    let response = CurlTransport.send(request).await.unwrap();
    server.join().unwrap();
    assert_eq!(response.status, 302);
    assert_eq!(
        response.header("location"),
        Some("http://127.0.0.1:1/elsewhere")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_refused_connection_is_a_network_error_without_the_address() {
    let free_port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("http://127.0.0.1:{free_port}/secret-path?token=abc");
    let outcome = CurlTransport
        .send(HttpRequest::new(UrlString::parse(&url).unwrap()))
        .await;
    match outcome {
        Err(Failure::Source(SourceError::Network(message))) => {
            assert!(
                !message.contains("secret-path") && !message.contains("token=abc"),
                "{message}"
            );
        }
        other => panic!("expected a network error, got {other:?}"),
    }
}
