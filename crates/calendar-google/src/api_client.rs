//! Port of `GoogleAPIClient.swift`: one request with the shared 401-refresh, rate-limit backoff and 5xx handling, the
//! paging walker and the calendar list.
//!
//! Swift throws `GoogleAPIError` next to `SourceError`; a Rust `Outcome` carries only `Failure`, so the client returns
//! [`ApiResult`], whose error is [`GoogleError`]: a provider outcome the caller handles, or any other failure.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::{HttpRequest, HttpResponse, HttpTransport};
use calendar_core::shared_connector_helpers::decode_response;
use calendar_core::source_types::SourceError;
use calendar_core::time::Sleeper;
use calendar_core::url_string::UrlString;
use calendar_oauth::AccessTokenProvider;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::dtos::{GoogleCalendarListEntryDTO, GoogleCalendarListPageDTO};

/// Provider-specific outcomes that callers handle; never leaves the `calendar-google` crate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GoogleApiError {
    /// 410: the sync token is no longer valid (or the event was already deleted).
    Gone,
    /// 404.
    NotFound,
    /// 403 without a rate-limit reason.
    Forbidden,
    /// 412 (write mode only): the `If-Match` version is stale.
    PreconditionFailed,
    /// 400 (write mode only), with Google's message.
    BadRequest(String),
    /// 409 (write mode only): e.g. an insert with a client-chosen id that already exists.
    Conflict,
}

impl GoogleApiError {
    /// What a public `CalendarSource` method throws if it cannot handle the case itself.
    pub(crate) fn source_error(&self) -> SourceError {
        SourceError::InvalidResponse(format!("google: {self}"))
    }
}

/// Swift's default description of the enum case, which the message above interpolates.
impl fmt::Display for GoogleApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GoogleApiError::Gone => f.write_str("gone"),
            GoogleApiError::NotFound => f.write_str("notFound"),
            GoogleApiError::Forbidden => f.write_str("forbidden"),
            GoogleApiError::PreconditionFailed => f.write_str("preconditionFailed"),
            GoogleApiError::BadRequest(message) => write!(f, "badRequest({message:?})"),
            GoogleApiError::Conflict => f.write_str("conflict"),
        }
    }
}

/// What Swift's `throws` carries inside this module: a [`GoogleApiError`] or anything else (`SourceError`,
/// `WriteError`, cancellation).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum GoogleError {
    Api(GoogleApiError),
    Other(Failure),
}

pub(crate) type ApiResult<T> = Result<T, GoogleError>;

impl From<Failure> for GoogleError {
    fn from(failure: Failure) -> GoogleError {
        GoogleError::Other(failure)
    }
}

impl From<GoogleApiError> for GoogleError {
    fn from(error: GoogleApiError) -> GoogleError {
        GoogleError::Api(error)
    }
}

impl From<SourceError> for GoogleError {
    fn from(error: SourceError) -> GoogleError {
        GoogleError::Other(Failure::Source(error))
    }
}

impl GoogleError {
    /// A provider outcome the caller did not handle becomes a `SourceError` (Swift `error.sourceError`); anything else
    /// passes through.
    pub(crate) fn into_failure(self) -> Failure {
        match self {
            GoogleError::Api(error) => Failure::Source(error.source_error()),
            GoogleError::Other(failure) => failure,
        }
    }
}

/// Reads keep the original error mapping. Writes additionally map a 400 to `BadRequest`, a 409 to `Conflict` and a 412
/// to `PreconditionFailed`, and treat every non-rate-limit 403 (other than `insufficientPermissions`) as `Forbidden`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GoogleRequestMode {
    Read,
    Write,
}

/// A query item: name and value, in order.
pub(crate) type QueryItem = (String, String);

pub(crate) fn query_item(name: &str, value: &str) -> QueryItem {
    (name.to_string(), value.to_string())
}

#[derive(Clone)]
pub(crate) struct GoogleApiClient {
    pub transport: Arc<dyn HttpTransport>,
    pub tokens: Arc<AccessTokenProvider>,
    pub sleep: Sleeper,
}

impl GoogleApiClient {
    pub(crate) const BASE: &'static str = "https://www.googleapis.com/calendar/v3";
    const MAX_RATE_LIMIT_RETRIES: u32 = 3;

    /// ASCII-only allow-list: Swift's `CharacterSet.alphanumerics` also admits non-ASCII letters, which are not valid in
    /// `URLComponents.percentEncodedPath` (setting one traps). Non-ASCII text is percent-encoded as UTF-8.
    pub(crate) fn percent_encode(text: &str) -> String {
        percent_encode_with(text, |b| {
            b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
        })
    }

    /// `tail` is appended as is; the calendar id is percent-encoded, e.g. `/calendars/me%40x.com/events`.
    pub(crate) fn calendar_path(calendar_id: &str, tail: &str) -> String {
        format!("/calendars/{}{tail}", Self::percent_encode(calendar_id))
    }

    pub(crate) fn event_path(calendar_id: &str, event_id: &str) -> String {
        Self::calendar_path(
            calendar_id,
            &format!("/events/{}", Self::percent_encode(event_id)),
        )
    }

    pub(crate) async fn get(&self, path: &str, query: &[QueryItem]) -> ApiResult<Vec<u8>> {
        self.send(
            "GET",
            path,
            query,
            None,
            &BTreeMap::new(),
            GoogleRequestMode::Read,
        )
        .await
    }

    /// The request URL: the base, the percent-encoded `path`, and the query items encoded the way `URLComponents`
    /// encodes them (see [`query_encode`]).
    pub(crate) fn url(path: &str, query: &[QueryItem]) -> Option<UrlString> {
        let mut text = format!("{}{path}", Self::BASE);
        if !query.is_empty() {
            text.push('?');
            text.push_str(
                &query
                    .iter()
                    .map(|(name, value)| format!("{}={}", query_encode(name), query_encode(value)))
                    .collect::<Vec<_>>()
                    .join("&"),
            );
        }
        UrlString::parse(&text)
    }

    /// One request with the shared 401-refresh, rate-limit backoff and 5xx handling. `path` is percent-encoded.
    pub(crate) async fn send(
        &self,
        method: &str,
        path: &str,
        query: &[QueryItem],
        body: Option<Vec<u8>>,
        extra_headers: &BTreeMap<String, String>,
        mode: GoogleRequestMode,
    ) -> ApiResult<Vec<u8>> {
        let mut rate_limit_retries: u32 = 0;
        let mut refreshed_after_401 = false;
        loop {
            let token = self.tokens.access_token().await?;
            let mut headers = BTreeMap::from([
                ("Authorization".to_string(), format!("Bearer {token}")),
                ("Accept".to_string(), "application/json".to_string()),
            ]);
            if body.is_some() {
                headers.insert("Content-Type".to_string(), "application/json".to_string());
            }
            for (name, value) in extra_headers {
                headers.insert(name.clone(), value.clone());
            }
            let url = Self::url(path, query).ok_or_else(|| {
                SourceError::InvalidResponse("google: the request address is not valid".to_string())
            })?;
            let response = self
                .transport
                .send(HttpRequest {
                    url,
                    method: method.to_string(),
                    headers,
                    body: body.clone(),
                })
                .await?;
            match response.status {
                200..=299 => return Ok(response.body),
                401 => {
                    if refreshed_after_401 {
                        return Err(SourceError::AuthExpired.into());
                    }
                    refreshed_after_401 = true;
                    self.tokens.invalidate();
                }
                400 if mode == GoogleRequestMode::Write => {
                    return Err(GoogleApiError::BadRequest(Self::message(&response)).into());
                }
                410 => return Err(GoogleApiError::Gone.into()),
                404 => return Err(GoogleApiError::NotFound.into()),
                409 if mode == GoogleRequestMode::Write => {
                    return Err(GoogleApiError::Conflict.into());
                }
                412 if mode == GoogleRequestMode::Write => {
                    return Err(GoogleApiError::PreconditionFailed.into());
                }
                403 | 429 => {
                    if !Self::is_rate_limit(&response) {
                        if response.status == 403 {
                            return Err(Self::classify_forbidden(&response, mode));
                        }
                        return Err(SourceError::InvalidResponse(format!(
                            "HTTP {}",
                            response.status
                        ))
                        .into());
                    }
                    let retry_after = Self::retry_after(&response);
                    rate_limit_retries += 1;
                    if rate_limit_retries > Self::MAX_RATE_LIMIT_RETRIES {
                        return Err(SourceError::RateLimited { retry_after }.into());
                    }
                    // Honor Retry-After exactly; otherwise exponential backoff with jitter so several clients
                    // de-synchronize.
                    let fallback = f64::from(1u32 << (rate_limit_retries - 1)) * jitter();
                    (self.sleep)(retry_after.unwrap_or(fallback)).await?;
                }
                status if status >= 500 => {
                    return Err(SourceError::Server { status }.into());
                }
                status => {
                    return Err(SourceError::InvalidResponse(format!("HTTP {status}")).into());
                }
            }
        }
    }

    /// `response.header("retry-after").flatMap(TimeInterval.init)`: a number of seconds. (Swift's `Double(String)` also
    /// reads hexadecimal floats; a Retry-After never uses them. A non-finite value is ignored here, where Swift would
    /// trap when it tried to sleep for it.)
    fn retry_after(response: &HttpResponse) -> Option<f64> {
        response
            .header("retry-after")
            .and_then(|text| text.parse::<f64>().ok())
            .filter(|seconds| seconds.is_finite())
    }

    fn message(response: &HttpResponse) -> String {
        serde_json::from_slice::<ErrorBody>(&response.body)
            .ok()
            .and_then(|body| body.error)
            .and_then(|inner| inner.message)
            .unwrap_or_else(|| format!("HTTP {}", response.status))
    }

    const QUOTA_REASONS: [&'static str; 3] = [
        "quotaExceeded",
        "calendarUsageLimitsExceeded",
        "dailyLimitExceeded",
    ];

    /// A non-rate-limit 403. Reads: only reason `forbidden` (one unreadable calendar) is skippable and the rest affect
    /// every calendar. Writes: a quota reason is a rate limit (retry later, not a permission problem); any other reason
    /// but `insufficientPermissions` is a permission problem on this event.
    fn classify_forbidden(response: &HttpResponse, mode: GoogleRequestMode) -> GoogleError {
        let errors = serde_json::from_slice::<ErrorBody>(&response.body)
            .ok()
            .and_then(|body| body.error)
            .and_then(|inner| inner.errors)
            .unwrap_or_default();
        // Only the write-mode quota check looks at every error; reads judge the first error's own reason, as they
        // always did.
        if mode == GoogleRequestMode::Write
            && errors.iter().any(|detail| {
                detail
                    .reason
                    .as_deref()
                    .is_some_and(|reason| Self::QUOTA_REASONS.contains(&reason))
            })
        {
            return SourceError::RateLimited {
                retry_after: Self::retry_after(response),
            }
            .into();
        }
        let first_reason = errors.first().and_then(|detail| detail.reason.as_deref());
        match first_reason {
            Some("forbidden") => GoogleApiError::Forbidden.into(),
            Some("insufficientPermissions") => SourceError::AuthExpired.into(),
            _ => {
                if mode == GoogleRequestMode::Write {
                    return GoogleApiError::Forbidden.into();
                }
                SourceError::InvalidResponse(format!(
                    "HTTP 403: {}",
                    first_reason.unwrap_or("unknown")
                ))
                .into()
            }
        }
    }

    fn is_rate_limit(response: &HttpResponse) -> bool {
        if response.status == 429 {
            return true;
        }
        let body = String::from_utf8_lossy(&response.body);
        body.contains("rateLimitExceeded") || body.contains("userRateLimitExceeded")
    }

    /// Walks every page, calling `handle` for each, until `next` returns `None`. No page cap.
    pub(crate) async fn pages<Page, N, H>(
        &self,
        path: &str,
        query: &[QueryItem],
        next: N,
        mut handle: H,
    ) -> ApiResult<()>
    where
        Page: DeserializeOwned,
        N: Fn(&Page) -> Option<String>,
        H: FnMut(&Page) -> ApiResult<()>,
    {
        let mut page_token: Option<String> = None;
        loop {
            let mut page_query = query.to_vec();
            if let Some(token) = &page_token {
                page_query.push(query_item("pageToken", token));
            }
            let page = decode_response::<Page>(&self.get(path, &page_query).await?)?;
            handle(&page)?;
            page_token = next(&page);
            if page_token.is_none() {
                return Ok(());
            }
        }
    }

    pub(crate) async fn calendar_list(&self) -> Outcome<Vec<GoogleCalendarListEntryDTO>> {
        let mut entries: Vec<GoogleCalendarListEntryDTO> = Vec::new();
        self.pages::<GoogleCalendarListPageDTO, _, _>(
            "/users/me/calendarList",
            &[
                query_item("showHidden", "false"),
                query_item("minAccessRole", "freeBusyReader"),
                query_item("maxResults", "250"),
                query_item(
                    "fields",
                    "nextPageToken,items(id,summary,summaryOverride,backgroundColor,accessRole,primary,timeZone,hidden,deleted,defaultReminders(method,minutes))",
                ),
            ],
            |page| page.next_page_token.clone(),
            |page| {
                entries.extend(page.items.iter().flatten().cloned());
                Ok(())
            },
        )
        .await
        .map_err(GoogleError::into_failure)?;
        Ok(entries)
    }
}

#[derive(Deserialize)]
struct ErrorBody {
    error: Option<ErrorInner>,
}

#[derive(Deserialize)]
struct ErrorInner {
    errors: Option<Vec<ErrorDetail>>,
    message: Option<String>,
}

#[derive(Deserialize)]
struct ErrorDetail {
    reason: Option<String>,
}

/// `Double.random(in: 0.5...1.0)` for the backoff. The jitter only spreads clients out, so a failing random source
/// falls back to the middle of the range instead of failing the request.
fn jitter() -> f64 {
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        return 0.75;
    }
    let unit = (u64::from_le_bytes(bytes) >> 11) as f64 / (1u64 << 53) as f64;
    0.5 + unit * 0.5
}

fn percent_encode_with(text: &str, allowed: impl Fn(u8) -> bool) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(text.len());
    for &byte in text.as_bytes() {
        if allowed(byte) {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0F)]));
        }
    }
    out
}

/// What `URLComponents` leaves unescaped in a query item's name or value (probed on macOS): letters, digits and
/// `! $ ' ( ) * + , - . / : ; ? @ _ ~`. Everything else, including space, `& = # % < > " [ \ ] ^ \` { | }` and
/// non-ASCII, is escaped.
pub(crate) fn query_encode(text: &str) -> String {
    percent_encode_with(text, |b| {
        b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'!' | b'$'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b'-'
                    | b'.'
                    | b'/'
                    | b':'
                    | b';'
                    | b'?'
                    | b'@'
                    | b'_'
                    | b'~'
            )
    })
}
