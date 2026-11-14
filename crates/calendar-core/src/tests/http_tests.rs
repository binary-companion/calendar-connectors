//! Not ported from Swift (`HTTP.swift` had no test of its own); pins the header rules of `HttpResponse` and
//! `HttpRequest` defaults that the connectors rely on.

use std::collections::BTreeMap;

use crate::UrlString;
use crate::http::{HttpRequest, HttpResponse};

#[test]
fn response_header_names_are_lowercased_and_lookup_is_case_insensitive() {
    let mut headers = BTreeMap::new();
    headers.insert("Content-Type".to_string(), "text/plain".to_string());
    headers.insert("Retry-After".to_string(), "5".to_string());
    let response = HttpResponse::new(200, headers, Vec::new());
    assert_eq!(
        response.headers.keys().cloned().collect::<Vec<_>>(),
        vec!["content-type".to_string(), "retry-after".to_string()]
    );
    assert_eq!(response.header("CONTENT-TYPE"), Some("text/plain"));
    assert_eq!(response.header("missing"), None);
}

#[test]
fn when_two_header_names_differ_only_in_case_the_last_wins() {
    let mut headers = BTreeMap::new();
    headers.insert("X-A".to_string(), "first".to_string());
    headers.insert("x-a".to_string(), "second".to_string());
    // `BTreeMap` iterates "X-A" before "x-a", so the lowercase spelling is the last one.
    let response = HttpResponse::new(200, headers, Vec::new());
    assert_eq!(response.header("x-a"), Some("second"));
}

#[test]
fn a_request_defaults_to_get_without_headers_or_body() {
    let request = HttpRequest::new(UrlString::parse("https://api.example.test/v1").unwrap());
    assert_eq!(request.method, "GET");
    assert!(request.headers.is_empty());
    assert_eq!(request.body, None);
}
