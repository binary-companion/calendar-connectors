//! Port of `WebDAVClient.swift`: HTTP for one CalDAV account.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::{HttpRequest, HttpResponse, HttpTransport};
use calendar_core::source_types::SourceError;
use calendar_core::url_string::UrlString;

use crate::dav_url::DavUrl;
use crate::dav_xml::{DavProperty, DavXml, Multistatus};

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct WebDavCredentials {
    pub username: String,
    pub password: String,
}

/// The password never prints.
impl fmt::Debug for WebDavCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebDavCredentials")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct WebDavReply {
    pub response: HttpResponse,
    /// The URL that answered, after redirects: relative hrefs in the body resolve against it.
    pub url: UrlString,
}

/// Read per request, so a sign-in again (new secrets in the store) reaches a source that already exists.
pub(crate) type CredentialProvider =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Outcome<WebDavCredentials>> + Send>> + Send + Sync>;

/// HTTP for one CalDAV account. Adds Basic auth to every request, only over HTTPS (plain HTTP only to the loopback
/// host) and only to hosts inside `host_base`, and follows redirects itself under the same rule. **The transport must
/// not follow redirects** (see `docs/host-http-contract.md`): it returns a 3xx answer as the response. The password
/// never appears in an error.
#[derive(Clone)]
pub(crate) struct WebDavClient {
    pub transport: Arc<dyn HttpTransport>,
    pub host_base: String,
    credentials: CredentialProvider,
}

fn invalid(message: impl Into<String>) -> Failure {
    Failure::Source(SourceError::InvalidResponse(message.into()))
}

impl WebDavClient {
    pub(crate) const MAX_REDIRECTS: usize = 5;
    const LOOPBACK: [&'static str; 2] = ["localhost", "127.0.0.1"];

    pub(crate) fn new(
        transport: Arc<dyn HttpTransport>,
        host_base: &str,
        credentials: CredentialProvider,
    ) -> WebDavClient {
        WebDavClient {
            transport,
            host_base: host_base.to_lowercase(),
            credentials,
        }
    }

    /// Credentials that never change (sign-in, before anything is stored).
    pub(crate) fn fixed_credentials(credentials: WebDavCredentials) -> CredentialProvider {
        Arc::new(move || {
            let credentials = credentials.clone();
            Box::pin(async move { Ok(credentials) })
        })
    }

    /// `https` to `host_base` or a subdomain of it (matched at a label boundary); `http` only to the loopback host.
    pub(crate) fn is_allowed(url: &UrlString, host_base: &str) -> bool {
        let (Some(host), Some(scheme)) = (url.host(), url.scheme()) else {
            return false;
        };
        let host = host.to_lowercase();
        let base = host_base.to_lowercase();
        if !(host == base || host.ends_with(&format!(".{base}"))) {
            return false;
        }
        match scheme.to_lowercase().as_str() {
            "https" => true,
            "http" => Self::LOOPBACK.contains(&host.as_str()),
            _ => false,
        }
    }

    pub(crate) fn basic_authorization(credentials: &WebDavCredentials) -> String {
        format!(
            "Basic {}",
            STANDARD
                .encode(format!("{}:{}", credentials.username, credentials.password).as_bytes())
        )
    }

    /// An href from a response, resolved against the URL that sent it. One that points outside `host_base` is refused.
    pub(crate) fn resolve(&self, href: &str, against: &UrlString) -> Outcome<UrlString> {
        match DavUrl::resolve(href, against) {
            Some(url) if Self::is_allowed(&url, &self.host_base) => Ok(url),
            _ => Err(invalid("the server pointed to another host")),
        }
    }

    /// Swift `send(_:_:headers:body:)` with no headers and no body.
    pub(crate) async fn send(&self, method: &str, url: &UrlString) -> Outcome<WebDavReply> {
        self.send_with(method, url, BTreeMap::new(), None).await
    }

    pub(crate) async fn send_with(
        &self,
        method: &str,
        url: &UrlString,
        headers: BTreeMap<String, String>,
        body: Option<Vec<u8>>,
    ) -> Outcome<WebDavReply> {
        let mut method = method.to_string();
        let mut url = url.clone();
        let mut body = body;
        let mut headers = headers;
        for _ in 0..=Self::MAX_REDIRECTS {
            if !Self::is_allowed(&url, &self.host_base) {
                return Err(invalid(format!(
                    "refused to send credentials to {}://{}",
                    url.scheme().as_deref().unwrap_or("?"),
                    url.host().as_deref().unwrap_or("?")
                )));
            }
            let mut request = HttpRequest {
                url: url.clone(),
                method: method.clone(),
                headers: headers.clone(),
                body: body.clone(),
            };
            let credentials = (self.credentials)().await?;
            request.headers.insert(
                "Authorization".to_string(),
                Self::basic_authorization(&credentials),
            );
            if body.is_some() && !request.headers.contains_key("Content-Type") {
                request.headers.insert(
                    "Content-Type".to_string(),
                    "application/xml; charset=utf-8".to_string(),
                );
            }
            let response = self.transport.send(request).await?;
            match response.status {
                301 | 302 | 303 | 307 | 308 => {
                    let next = response
                        .header("Location")
                        .and_then(|location| DavUrl::resolve(location, &url))
                        .ok_or_else(|| invalid("a redirect without a location"))?;
                    if !Self::is_allowed(&next, &self.host_base) {
                        return Err(invalid("the server redirected to another host"));
                    }
                    if response.status == 303 {
                        method = "GET".to_string();
                        body = None;
                        headers.remove("Content-Type");
                    }
                    url = next;
                }
                401 => return Err(SourceError::AuthExpired.into()),
                429 | 503 => {
                    return Err(SourceError::RateLimited {
                        retry_after: response
                            .header("Retry-After")
                            .and_then(|v| v.parse::<f64>().ok()),
                    }
                    .into());
                }
                500..=599 => {
                    return Err(SourceError::Server {
                        status: response.status,
                    }
                    .into());
                }
                _ => return Ok(WebDavReply { response, url }),
            }
        }
        Err(invalid("too many redirects"))
    }

    /// A `PROPFIND` that must answer 207; returns the multistatus and the URL that answered.
    pub(crate) async fn propfind(
        &self,
        url: &UrlString,
        depth: i64,
        properties: &[DavProperty],
    ) -> Outcome<(Multistatus, UrlString)> {
        let reply = self
            .send_with(
                "PROPFIND",
                url,
                BTreeMap::from([("Depth".to_string(), depth.to_string())]),
                Some(DavXml::propfind(properties)),
            )
            .await?;
        if reply.response.status != 207 {
            return Err(invalid(format!(
                "PROPFIND answered {}",
                reply.response.status
            )));
        }
        Ok((DavXml::multistatus(&reply.response.body)?, reply.url))
    }

    pub(crate) async fn report(
        &self,
        url: &UrlString,
        depth: i64,
        body: Vec<u8>,
    ) -> Outcome<WebDavReply> {
        self.send_with(
            "REPORT",
            url,
            BTreeMap::from([("Depth".to_string(), depth.to_string())]),
            Some(body),
        )
        .await
    }
}
