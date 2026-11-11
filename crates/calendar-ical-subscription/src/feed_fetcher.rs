//! Port of `FeedFetcher.swift`.

use std::collections::BTreeMap;
use std::sync::Arc;

use calendar_core::diagnostic_log::{
    DiagnosticField, DiagnosticLevel, DiagnosticLog, NullDiagnosticLog,
};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::{HttpRequest, HttpResponse, HttpTransport};
use calendar_core::source_types::SourceError;
use calendar_core::time::{Now, seconds_between};
use calendar_core::url_string::UrlString;
use jiff::Timestamp;

/// What a server said about a version of the feed, sent back on the next request so an unchanged feed costs no body.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FeedValidators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl FeedValidators {
    /// True when the server gave at least one validator (`etag != nil || lastModified != nil`).
    pub(crate) fn has_any(&self) -> bool {
        self.etag.is_some() || self.last_modified.is_some()
    }
}

#[derive(Clone)]
pub(crate) enum FeedResponse {
    NotModified,
    Body(Vec<u8>, FeedValidators),
}

/// Prints the length of the body, never the body: a server can echo the link into the page it sends (Swift's `Data`
/// describes itself by its length too).
impl std::fmt::Debug for FeedResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FeedResponse::NotModified => f.write_str("NotModified"),
            FeedResponse::Body(data, validators) => f
                .debug_tuple("Body")
                .field(&format_args!("{} bytes", data.len()))
                .field(validators)
                .finish(),
        }
    }
}

/// Downloads a feed.
///
/// **The transport must not follow redirects itself**: it returns a 3xx response as it is. Every hop is checked here:
/// only `https`, at most [`FeedFetcher::MAX_REDIRECTS`], no user name or password in the target, and validators are
/// sent to the original link only. A transport that follows redirects on its own would bypass those checks and send the
/// request (whose address is a credential) to places this code never saw. The settings the Swift `URLSessionTransport`
/// was built with (`followsRedirects: false`, an ephemeral session with no URL cache and no cookies) are the host's
/// obligation; see `docs/host-http-contract.md`.
///
/// No message built here contains the link: it is a credential (ADR 0018).
#[derive(Clone)]
pub(crate) struct FeedFetcher {
    transport: Arc<dyn HttpTransport>,
    diagnostics: Arc<dyn DiagnosticLog>,
    /// Times a fetch and dates its diagnostics; without it the duration is left out.
    now: Option<Now>,
}

/// What a fetch did so far, for the diagnostics only. Never holds the link.
struct Trace {
    status: Option<i64>,
    hops: i64,
    conditional: bool,
    reason: &'static str,
}

const UNREACHABLE: &str = "the feed could not be reached";
const REFUSED_REDIRECT: &str = "the feed moved somewhere TimeTug will not follow";

impl FeedFetcher {
    pub(crate) const MAX_BYTES: usize = 10 * 1024 * 1024;
    pub(crate) const MAX_REDIRECTS: usize = 5;

    /// A fetcher that records nothing. See the type's documentation for what `transport` must do.
    pub(crate) fn new(transport: Arc<dyn HttpTransport>) -> FeedFetcher {
        FeedFetcher {
            transport,
            diagnostics: Arc::new(NullDiagnosticLog),
            now: None,
        }
    }

    /// Swift sets `diagnostics` and `now` as two properties; the library has no clock of its own, so the diagnostics
    /// come with the `now` that dates them (and times the fetch).
    pub(crate) fn with_diagnostics(
        mut self,
        diagnostics: Arc<dyn DiagnosticLog>,
        now: Now,
    ) -> FeedFetcher {
        self.diagnostics = diagnostics;
        self.now = Some(now);
        self
    }

    fn date(&self) -> Timestamp {
        self.now.as_ref().map_or(Timestamp::UNIX_EPOCH, |now| now())
    }

    pub(crate) async fn fetch(
        &self,
        url: &UrlString,
        validators: Option<&FeedValidators>,
    ) -> Outcome<FeedResponse> {
        let started = self.now.as_ref().map(|now| now());
        let mut trace = Trace {
            status: None,
            hops: 0,
            conditional: false,
            reason: "network",
        };
        match self.run(url, validators, &mut trace).await {
            Ok(response) => {
                let mut fields = vec![
                    DiagnosticField::int("status", trace.status.unwrap_or(0)),
                    DiagnosticField::bool("conditional", trace.conditional),
                    DiagnosticField::int("hops", trace.hops),
                ];
                match &response {
                    FeedResponse::NotModified => {
                        fields.insert(2, DiagnosticField::bool("notModified", true));
                        fields.push(DiagnosticField::int("bytes", 0));
                    }
                    FeedResponse::Body(data, _) => {
                        fields.insert(2, DiagnosticField::bool("notModified", false));
                        fields.push(DiagnosticField::int("bytes", data.len() as i64));
                    }
                }
                if let (Some(started), Some(now)) = (started, &self.now) {
                    let milliseconds = (seconds_between(started, now()) * 1000.0) as i64;
                    fields.push(DiagnosticField::int("ms", milliseconds.max(0)));
                }
                self.diagnostics.record(
                    DiagnosticLevel::Info,
                    "icalsub",
                    "fetchCompleted",
                    fields,
                    self.date(),
                );
                Ok(response)
            }
            Err(error) => {
                if error != Failure::Cancelled {
                    let mut fields = vec![DiagnosticField::public_string("reason", trace.reason)];
                    if let Some(status) = trace.status {
                        fields.push(DiagnosticField::int("status", status));
                    }
                    self.diagnostics.record(
                        DiagnosticLevel::Warning,
                        "icalsub",
                        "fetchFailed",
                        fields,
                        self.date(),
                    );
                }
                Err(error)
            }
        }
    }

    async fn run(
        &self,
        url: &UrlString,
        validators: Option<&FeedValidators>,
        trace: &mut Trace,
    ) -> Outcome<FeedResponse> {
        let mut current = url.clone();
        for hop in 0..=Self::MAX_REDIRECTS {
            let mut headers = BTreeMap::from([(
                "Accept".to_string(),
                "text/calendar, text/plain;q=0.5, */*;q=0.1".to_string(),
            )]);
            // Validators describe the version the original link served; a redirect target never validated it.
            if hop == 0 {
                if let Some(etag) = validators.and_then(|v| v.etag.as_ref()) {
                    headers.insert("If-None-Match".to_string(), etag.clone());
                }
                if let Some(modified) = validators.and_then(|v| v.last_modified.as_ref()) {
                    headers.insert("If-Modified-Since".to_string(), modified.clone());
                }
            }
            let conditional =
                headers.contains_key("If-None-Match") || headers.contains_key("If-Modified-Since");
            trace.hops = hop as i64;
            trace.conditional = if hop == 0 {
                conditional
            } else {
                trace.conditional
            };
            trace.reason = "network";
            trace.status = None;
            let mut request = HttpRequest::new(current.clone());
            request.headers = headers;
            let response = self.send(request).await?;
            trace.status = Some(response.status);
            match response.status {
                200 => {
                    if response.body.len() > Self::MAX_BYTES {
                        trace.reason = "tooLarge";
                        return Err(invalid("the feed is too large"));
                    }
                    let received = FeedValidators {
                        etag: response.header("etag").map(str::to_string),
                        last_modified: response.header("last-modified").map(str::to_string),
                    };
                    return Ok(FeedResponse::Body(response.body, received));
                }
                304 if conditional => return Ok(FeedResponse::NotModified),
                301 | 302 | 303 | 307 | 308 => {
                    let next = response
                        .header("location")
                        .and_then(|location| Self::resolve_redirect(location, &current))
                        .filter(|next| {
                            next.scheme().is_some_and(|s| s.to_lowercase() == "https")
                                && next.user().is_none()
                                && next.password().is_none()
                        });
                    let Some(next) = next else {
                        trace.reason = "redirectRefused";
                        return Err(invalid(REFUSED_REDIRECT));
                    };
                    current = next;
                }
                401 | 403 | 404 | 410 => {
                    trace.reason = "authExpired";
                    return Err(Failure::Source(SourceError::AuthExpired));
                }
                429 | 500..=599 => {
                    trace.reason = "server";
                    return Err(Failure::Source(SourceError::Server {
                        status: response.status,
                    }));
                }
                other => {
                    trace.reason = "invalidResponse";
                    return Err(invalid(&format!("the feed server answered {other}")));
                }
            }
        }
        trace.reason = "tooManyRedirects";
        Err(invalid("the feed redirected too many times"))
    }

    /// `URL(string: location, relativeTo: current)?.absoluteURL`: an absolute `location` as it is, a relative one
    /// resolved against `base` (RFC 3986). `None` when Foundation would give no URL.
    pub(crate) fn resolve_redirect(location: &str, base: &UrlString) -> Option<UrlString> {
        if location.is_empty() {
            return None;
        }
        let reference = UrlString::parse(location)?;
        if reference.scheme().is_some() {
            return Some(reference);
        }
        let joined = base.parsed()?.join(location).ok()?;
        UrlString::parse(joined.as_str())
    }

    /// The transport's failure, without the link. `SourceError::Network` is the transport's contract, but a message
    /// (or a stray host failure, the Rust form of Swift's `URLError`) can carry the address, and the address is a
    /// credential.
    async fn send(&self, request: HttpRequest) -> Outcome<HttpResponse> {
        let url = request.url.clone();
        match self.transport.send(request).await {
            Ok(response) => Ok(response),
            Err(Failure::Cancelled) => Err(Failure::Cancelled),
            Err(Failure::Source(SourceError::Network(message))) => {
                let leaks = secrets_of(&url)
                    .iter()
                    .any(|secret| secret.chars().count() > 1 && message.contains(secret.as_str()));
                Err(Failure::Source(SourceError::Network(
                    if leaks { UNREACHABLE } else { &message }.to_string(),
                )))
            }
            Err(Failure::Source(error)) => Err(Failure::Source(error)),
            Err(_) => Err(Failure::Source(SourceError::Network(
                UNREACHABLE.to_string(),
            ))),
        }
    }
}

fn invalid(message: &str) -> Failure {
    Failure::Source(SourceError::InvalidResponse(message.to_string()))
}

/// The pieces of a link a message must not repeat: the whole link, the path as `URL.path` reads it (decoded) and as
/// `URLComponents.percentEncodedPath` reads it, the query decoded and as written.
fn secrets_of(url: &UrlString) -> Vec<String> {
    let text = url.as_str();
    let mut secrets = vec![text.to_string(), url.path()];
    if let Some(path) = encoded_path(text) {
        secrets.push(path.to_string());
    }
    if let Some(query) = url.query() {
        secrets.push(percent_decode(&query));
        secrets.push(query);
    }
    secrets
}

/// The path of `scheme://authority/path?query#fragment` as written, between the authority and the `?` or `#`.
fn encoded_path(text: &str) -> Option<&str> {
    let after_scheme = text.split_once("://")?.1;
    let start = after_scheme.find(['/', '?', '#'])?;
    let from_path = &after_scheme[start..];
    let end = from_path.find(['?', '#']).unwrap_or(from_path.len());
    Some(&from_path[..end])
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| char::from(b).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(high), Some(low)) = (
                bytes.get(i + 1).and_then(|b| hex(*b)),
                bytes.get(i + 2).and_then(|b| hex(*b)),
            )
        {
            out.push((high * 16 + low) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
