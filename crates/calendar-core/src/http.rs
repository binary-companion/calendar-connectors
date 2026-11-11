//! The HTTP seam. Port of `HTTP.swift`; `URLSessionTransport` is host code and is not ported.

use std::collections::BTreeMap;

use async_trait::async_trait;

use crate::failure::Outcome;
use crate::url_string::UrlString;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRequest {
    pub url: UrlString,
    pub method: String,
    pub headers: BTreeMap<String, String>,
    pub body: Option<Vec<u8>>,
}

impl HttpRequest {
    /// `HTTPRequest(url:)`: a GET without headers or body; set the public fields for the rest.
    pub fn new(url: UrlString) -> HttpRequest {
        HttpRequest {
            url,
            method: "GET".to_string(),
            headers: BTreeMap::new(),
            body: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: i64,
    /// Header names lowercased.
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// Lowercases the header names; when two names differ only in case the last one (in the map's key order) wins.
    pub fn new(status: i64, headers: BTreeMap<String, String>, body: Vec<u8>) -> HttpResponse {
        let mut lowered = BTreeMap::new();
        for (name, value) in headers {
            lowered.insert(name.to_lowercase(), value);
        }
        HttpResponse {
            status,
            headers: lowered,
            body,
        }
    }

    /// The header called `name`, looked up case-insensitively.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_lowercase()).map(String::as_str)
    }
}

/// The seam through which every connector reaches the network. The host supplies the implementation.
///
/// Contract (pinned by the Swift `URLSessionTransport` tests, which are not ported because that type is host code):
/// the transport performs no caching, since sync tokens make every poll a new URL and a cache would only pile up
/// responses and keep authenticated bodies on disk; and where a connector says so (CalDAV checks where a redirect
/// points before sending credentials there), it must not follow redirects but return the 3xx response itself.
#[async_trait]
pub trait HttpTransport: Send + Sync {
    /// Returns any HTTP response (including 4xx/5xx). Fails with `SourceError::Network` for transport failures and
    /// `Failure::Cancelled` when the request was cancelled.
    async fn send(&self, request: HttpRequest) -> Outcome<HttpResponse>;
}
