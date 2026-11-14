//! Port of `GraphAPIClient.swift`: one request with the shared 401-refresh, throttling backoff and 5xx handling, the
//! paging walker and the account email.
//!
//! Swift throws `GraphAPIError` next to `SourceError`; a Rust `Outcome` carries only `Failure`, so the client returns
//! [`ApiResult`], whose error is [`GraphError`]: a provider outcome the caller handles, or any other failure.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::{HttpRequest, HttpResponse, HttpTransport};
use calendar_core::shared_connector_helpers::decode_response;
use calendar_core::source_types::SourceError;
use calendar_core::time::Sleeper;
use calendar_core::url_string::UrlString;
use calendar_oauth::AccessTokenSupplying;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::dtos::GraphPage;

/// Provider-specific outcomes that callers handle; never leaves the `calendar-microsoft` crate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GraphApiError {
    /// 410, or a 400 that says the delta token is no longer valid.
    Gone,
    /// 404.
    NotFound,
    /// 403.
    Forbidden,
    /// 409.
    Conflict,
    /// 412.
    PreconditionFailed,
    /// 400, with Graph's message.
    BadRequest(String),
}

impl GraphApiError {
    /// What a public `CalendarSource` method throws if it cannot handle the case itself.
    pub(crate) fn source_error(&self) -> SourceError {
        SourceError::InvalidResponse(format!("microsoft: {self}"))
    }
}

/// Swift's default description of the enum case, which the message above interpolates.
impl fmt::Display for GraphApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphApiError::Gone => f.write_str("gone"),
            GraphApiError::NotFound => f.write_str("notFound"),
            GraphApiError::Forbidden => f.write_str("forbidden"),
            GraphApiError::Conflict => f.write_str("conflict"),
            GraphApiError::PreconditionFailed => f.write_str("preconditionFailed"),
            GraphApiError::BadRequest(message) => write!(f, "badRequest({message:?})"),
        }
    }
}

/// What Swift's `throws` carries inside this module: a [`GraphApiError`] or anything else (`SourceError`,
/// `WriteError`, cancellation).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum GraphError {
    Api(GraphApiError),
    Other(Failure),
}

pub(crate) type ApiResult<T> = Result<T, GraphError>;

impl From<Failure> for GraphError {
    fn from(failure: Failure) -> GraphError {
        GraphError::Other(failure)
    }
}

impl From<GraphApiError> for GraphError {
    fn from(error: GraphApiError) -> GraphError {
        GraphError::Api(error)
    }
}

impl From<SourceError> for GraphError {
    fn from(error: SourceError) -> GraphError {
        GraphError::Other(Failure::Source(error))
    }
}

impl GraphError {
    /// A provider outcome the caller did not handle becomes a `SourceError` (Swift `error.sourceError`); anything else
    /// passes through.
    pub(crate) fn into_failure(self) -> Failure {
        match self {
            GraphError::Api(error) => Failure::Source(error.source_error()),
            GraphError::Other(failure) => failure,
        }
    }
}

/// A query item: name and value, in order.
pub(crate) type QueryItem = (String, String);

pub(crate) fn query_item(name: &str, value: &str) -> QueryItem {
    (name.to_string(), value.to_string())
}

#[derive(Clone)]
pub(crate) struct GraphApiClient {
    pub transport: Arc<dyn HttpTransport>,
    pub tokens: Arc<dyn AccessTokenSupplying>,
    pub sleep: Sleeper,
}

impl GraphApiClient {
    pub(crate) const BASE: &'static str = "https://graph.microsoft.com/v1.0";
    pub(crate) const HOST: &'static str = "graph.microsoft.com";
    const MAX_THROTTLE_RETRIES: u32 = 3;
    /// Error codes Graph uses for a delta token it no longer accepts.
    const RESYNC_CODES: [&'static str; 4] = [
        "syncStateNotFound",
        "ResyncRequired",
        "ErrorInvalidSyncStateData",
        "syncStateInvalid",
    ];

    /// ASCII-only allow-list: Swift's `CharacterSet.alphanumerics` also admits non-ASCII letters, which are not valid in
    /// `URLComponents.percentEncodedPath`. Graph ids contain `=`, `-` and `_`. Non-ASCII text is percent-encoded as
    /// UTF-8.
    pub(crate) fn percent_encode(text: &str) -> String {
        percent_encode_with(text, |b| {
            b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
        })
    }

    pub(crate) fn calendar_path(calendar_id: &str, tail: &str) -> String {
        format!("/me/calendars/{}{tail}", Self::percent_encode(calendar_id))
    }

    /// Events are addressed through their calendar, which also works for a shared calendar.
    pub(crate) fn event_path(calendar_id: &str, event_id: &str) -> String {
        Self::calendar_path(
            calendar_id,
            &format!("/events/{}", Self::percent_encode(event_id)),
        )
    }

    /// The request URL: the base, the percent-encoded `path`, and the query items encoded the way `URLComponents`
    /// encodes them (see [`query_encode`]). Swift force-unwraps the result; an address `UrlString` refuses is an error
    /// here.
    pub(crate) fn url(path: &str, query: &[QueryItem]) -> Outcome<UrlString> {
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
        UrlString::parse(&text).ok_or_else(|| {
            Failure::Source(SourceError::InvalidResponse(
                "microsoft: the request address is not valid".to_string(),
            ))
        })
    }

    pub(crate) async fn get(&self, url: &UrlString, prefer: &[String]) -> ApiResult<Vec<u8>> {
        self.send("GET", url, None, prefer, &BTreeMap::new()).await
    }

    /// One request with the shared 401-refresh, throttling backoff and 5xx handling. Every request asks for immutable
    /// ids (they survive a move between folders); `prefer` adds more preferences.
    pub(crate) async fn send(
        &self,
        method: &str,
        url: &UrlString,
        body: Option<Vec<u8>>,
        prefer: &[String],
        extra_headers: &BTreeMap<String, String>,
    ) -> ApiResult<Vec<u8>> {
        let mut retries: u32 = 0;
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
            let preferences: Vec<&str> = std::iter::once("IdType=\"ImmutableId\"")
                .chain(prefer.iter().map(String::as_str))
                .collect();
            headers.insert("Prefer".to_string(), preferences.join(", "));
            for (name, value) in extra_headers {
                headers.insert(name.clone(), value.clone());
            }
            let response = self
                .transport
                .send(HttpRequest {
                    url: url.clone(),
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
                    self.tokens.invalidate().await;
                }
                400 => {
                    let (code, message) = Self::error(&response);
                    if code
                        .as_deref()
                        .is_some_and(|code| Self::RESYNC_CODES.contains(&code))
                    {
                        return Err(GraphApiError::Gone.into());
                    }
                    return Err(GraphApiError::BadRequest(message).into());
                }
                403 => return Err(GraphApiError::Forbidden.into()),
                404 => return Err(GraphApiError::NotFound.into()),
                409 => return Err(GraphApiError::Conflict.into()),
                410 => return Err(GraphApiError::Gone.into()),
                412 => return Err(GraphApiError::PreconditionFailed.into()),
                429 | 503 | 504 => {
                    let retry_after = Self::retry_after(&response);
                    retries += 1;
                    if retries > Self::MAX_THROTTLE_RETRIES {
                        if response.status == 429 {
                            return Err(SourceError::RateLimited { retry_after }.into());
                        }
                        return Err(SourceError::Server {
                            status: response.status,
                        }
                        .into());
                    }
                    // Honor Retry-After exactly; otherwise exponential backoff with jitter.
                    let fallback = f64::from(1u32 << (retries - 1)) * jitter();
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

    /// Graph's error code and message, or no code and `HTTP <status>` when the body is not Graph's error shape.
    fn error(response: &HttpResponse) -> (Option<String>, String) {
        let inner = serde_json::from_slice::<ErrorBody>(&response.body)
            .ok()
            .and_then(|body| body.error);
        let code = inner.as_ref().and_then(|inner| inner.code.clone());
        let message = inner
            .and_then(|inner| inner.message)
            .unwrap_or_else(|| format!("HTTP {}", response.status));
        (code, message)
    }

    /// A link Graph sent must stay on Graph's own host over `https`: the bearer token is only ever sent there. The
    /// comparison is exact, as Swift's `url.scheme == "https"` and `url.host == host` are (`URL.scheme` and `.host`
    /// keep the case of the text).
    fn is_graph_link(url: &UrlString) -> bool {
        url.as_str().starts_with("https:") && url.host().as_deref() == Some(Self::HOST)
    }

    /// Walks every page from `first`, calling `handle` for each, following `@odata.nextLink`. A next link must stay on
    /// Graph's own host: the bearer token is only ever sent there. No page cap.
    pub(crate) async fn pages<Page, H>(
        &self,
        first: &UrlString,
        prefer: &[String],
        mut handle: H,
    ) -> ApiResult<()>
    where
        Page: GraphPage + DeserializeOwned,
        H: FnMut(&Page) -> ApiResult<()>,
    {
        let mut next: Option<UrlString> = Some(first.clone());
        while let Some(url) = next {
            if !Self::is_graph_link(&url) {
                return Err(
                    SourceError::InvalidResponse("unexpected link host".to_string()).into(),
                );
            }
            let page = decode_response::<Page>(&self.get(&url, prefer).await?)?;
            handle(&page)?;
            next = page.next_link().and_then(UrlString::parse);
        }
        Ok(())
    }

    /// `mail`, else `userPrincipalName`, lowercased: the address the account is known by. Both Microsoft kinds ask
    /// Graph for it after a sign-in, so a host is never trusted for the identity.
    pub(crate) async fn account_email(&self) -> Outcome<String> {
        let url = Self::url("/me", &[query_item("$select", "mail,userPrincipalName")])?;
        let data = self
            .get(&url, &[])
            .await
            .map_err(GraphError::into_failure)?;
        let me = decode_response::<MeDTO>(&data)?;
        let Some(address) = [me.mail, me.user_principal_name]
            .into_iter()
            .flatten()
            .find(|address| !address.is_empty())
        else {
            return Err(Failure::Source(SourceError::InvalidResponse(
                "the account has no email address".to_string(),
            )));
        };
        Ok(address.to_lowercase())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MeDTO {
    mail: Option<String>,
    user_principal_name: Option<String>,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: Option<ErrorInner>,
}

#[derive(Deserialize)]
struct ErrorInner {
    code: Option<String>,
    message: Option<String>,
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
