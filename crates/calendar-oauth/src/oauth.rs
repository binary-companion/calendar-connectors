//! Port of `OAuth.swift`.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use calendar_core::UrlString;
use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::{HttpRequest, HttpTransport};
use calendar_core::source_types::SourceError;
use calendar_core::time::{Now, add_seconds};
use jiff::Timestamp;
use serde_json::Value;

/// Why a sign-in redirect could not be turned into an authorization code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AuthorizationError {
    /// The user declined or closed the sign-in.
    #[error("the sign-in was cancelled")]
    Cancelled,
    /// The redirect's `state` did not equal the value sent with the authorization request (possible forged redirect).
    #[error("the sign-in answered with a different state")]
    StateMismatch,
    /// The redirect carried no `code`, or an empty one.
    #[error("the sign-in answered without a code")]
    MissingCode,
}

/// What Swift's `authorizationCode(from:expectedState:)` can throw: an `AuthorizationError` or a `SourceError`.
/// Converting to a [`Failure`] (which has no case for `AuthorizationError`): `Cancelled` becomes
/// `Failure::Cancelled`, a state mismatch or a missing code becomes `Failure::Host` with the error's text, and a
/// `SourceError` stays one.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum AuthorizationCodeError {
    /// The redirect was rejected for an `AuthorizationError` reason.
    #[error(transparent)]
    Authorization(#[from] AuthorizationError),
    /// The redirect carried a provider error other than `access_denied`, reported as `SourceError::InvalidResponse`.
    #[error(transparent)]
    Source(#[from] SourceError),
}

impl From<AuthorizationCodeError> for Failure {
    fn from(error: AuthorizationCodeError) -> Failure {
        match error {
            AuthorizationCodeError::Authorization(AuthorizationError::Cancelled) => {
                Failure::Cancelled
            }
            AuthorizationCodeError::Authorization(other) => Failure::Host(other.to_string()),
            AuthorizationCodeError::Source(source) => Failure::Source(source),
        }
    }
}

/// Endpoints, client identity and scopes for one OAuth 2.0 authorization-code (PKCE) provider.
#[derive(Clone, PartialEq, Eq)]
pub struct OAuthConfig {
    /// URL of the provider's authorization endpoint, where the user signs in. Any query it has is replaced when
    /// building the sign-in URL.
    pub authorization_endpoint: UrlString,
    /// URL of the provider's token endpoint, used for code exchange and refresh.
    pub token_endpoint: UrlString,
    /// Public client id registered with the provider.
    pub client_id: String,
    /// For desktop clients this is not confidential, but the token endpoint still requires it. Supplied by the host app.
    pub client_secret: Option<String>,
    /// Scopes requested at sign-in, joined with spaces in the `scope` parameter.
    pub scopes: Vec<String>,
    /// Extra query parameters added to the authorization URL (for example `access_type` or `prompt`), in key order
    /// after the standard ones.
    pub extra_auth_params: BTreeMap<String, String>,
}

impl OAuthConfig {
    /// Swift's `init` with `clientSecret` nil and no extra parameters; set the public fields (or use struct update
    /// syntax) for the rest.
    pub fn new(
        authorization_endpoint: UrlString,
        token_endpoint: UrlString,
        client_id: impl Into<String>,
        scopes: Vec<String>,
    ) -> OAuthConfig {
        OAuthConfig {
            authorization_endpoint,
            token_endpoint,
            client_id: client_id.into(),
            client_secret: None,
            scopes,
            extra_auth_params: BTreeMap::new(),
        }
    }
}

/// Keeps the client secret out of logs and test failure output (Swift would print it).
impl fmt::Debug for OAuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthConfig")
            .field("authorization_endpoint", &self.authorization_endpoint)
            .field("token_endpoint", &self.token_endpoint)
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "<redacted>"),
            )
            .field("scopes", &self.scopes)
            .field("extra_auth_params", &self.extra_auth_params)
            .finish()
    }
}

/// The result of a token request. `Debug` redacts the token values.
#[derive(Clone, PartialEq, Eq)]
pub struct OAuthTokens {
    /// The bearer token sent with API requests.
    pub access_token: String,
    /// When the access token expires: the response's `expires_in` seconds from the time it was received (3600 s when
    /// the response gave none).
    pub expires_at: Timestamp,
    /// The refresh token, if the response contained one. Providers that rotate refresh tokens send a new one on
    /// refresh; `None` means keep using the old one.
    pub refresh_token: Option<String>,
}

impl OAuthTokens {
    /// Swift's `init` with `refreshToken` nil.
    pub fn new(access_token: impl Into<String>, expires_at: Timestamp) -> OAuthTokens {
        OAuthTokens {
            access_token: access_token.into(),
            expires_at,
            refresh_token: None,
        }
    }
}

/// Keeps the tokens out of logs and test failure output (Swift would print them).
impl fmt::Debug for OAuthTokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthTokens")
            .field("access_token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// Swift's `struct OAuthClient`; cloning shares the transport.
#[derive(Clone)]
pub struct OAuthClient {
    config: OAuthConfig,
    transport: Arc<dyn HttpTransport>,
    now: Now,
}

impl OAuthClient {
    /// Creates a client for `config` that sends token requests through `transport` and reads the current time from
    /// `now`.
    pub fn new(config: OAuthConfig, transport: Arc<dyn HttpTransport>, now: Now) -> OAuthClient {
        OAuthClient {
            config,
            transport,
            now,
        }
    }

    /// Builds the sign-in URL to open in the browser: the authorization endpoint with `client_id`, `redirect_uri`,
    /// `response_type=code`, the joined scopes, `state`, the PKCE `code_challenge` (method `S256`) and then
    /// `extra_auth_params`. A query already on the endpoint is replaced.
    pub fn authorization_url(
        &self,
        redirect_uri: &UrlString,
        state: &str,
        code_challenge: &str,
    ) -> UrlString {
        let scope = self.config.scopes.join(" ");
        let mut items = vec![
            ("client_id", self.config.client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("response_type", "code"),
            ("scope", scope.as_str()),
            ("state", state),
            ("code_challenge", code_challenge),
            ("code_challenge_method", "S256"),
        ];
        // `BTreeMap` iterates in key order, which is `sorted(by: { $0.key < $1.key })` for the ASCII names in use.
        for (name, value) in &self.config.extra_auth_params {
            items.push((name.as_str(), value.as_str()));
        }
        let query = items
            .iter()
            .map(|(name, value)| format!("{}={}", query_encode(name), query_encode(value)))
            .collect::<Vec<_>>()
            .join("&");
        // `URLComponents.queryItems = ...` replaces the endpoint's own query and keeps its fragment.
        let text = self.config.authorization_endpoint.as_str();
        let (before_fragment, fragment) = match text.find('#') {
            Some(at) => (&text[..at], &text[at..]),
            None => (text, ""),
        };
        let base = before_fragment
            .find('?')
            .map_or(before_fragment, |at| &before_fragment[..at]);
        // The endpoint was accepted and the query holds only characters a URL accepts; Swift force-unwraps here.
        UrlString::parse(&format!("{base}?{query}{fragment}"))
            .unwrap_or_else(|| self.config.authorization_endpoint.clone())
    }

    /// Extracts the authorization code from the redirect URL, validating `state`.
    pub fn authorization_code(
        &self,
        redirect: &UrlString,
        expected_state: &str,
    ) -> Result<String, AuthorizationCodeError> {
        if redirect.query_value("state").as_deref() != Some(expected_state) {
            return Err(AuthorizationError::StateMismatch.into());
        }
        if let Some(error) = redirect.query_value("error") {
            if error == "access_denied" {
                return Err(AuthorizationError::Cancelled.into());
            }
            return Err(SourceError::InvalidResponse(format!("oauth error: {error}")).into());
        }
        match redirect.query_value("code") {
            Some(code) if !code.is_empty() => Ok(code),
            _ => Err(AuthorizationError::MissingCode.into()),
        }
    }

    /// Exchanges the authorization `code` and PKCE `verifier` for tokens at the token endpoint. Fails with
    /// `SourceError::InvalidResponse` when the provider answers with an error or no access token.
    pub async fn exchange(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &UrlString,
    ) -> Outcome<OAuthTokens> {
        self.token(
            &[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("code_verifier", verifier),
                ("redirect_uri", redirect_uri.as_str()),
            ],
            false,
        )
        .await
    }

    /// Fails with `SourceError::AuthExpired` when the refresh token is no longer valid (`invalid_grant`).
    pub async fn refresh(&self, refresh_token: &str) -> Outcome<OAuthTokens> {
        self.token(
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ],
            true,
        )
        .await
    }

    async fn token(&self, params: &[(&str, &str)], is_refresh: bool) -> Outcome<OAuthTokens> {
        let mut fields: BTreeMap<&str, &str> = params.iter().copied().collect();
        fields.insert("client_id", &self.config.client_id);
        if let Some(secret) = &self.config.client_secret {
            fields.insert("client_secret", secret);
        }
        let body = fields
            .iter()
            .map(|(name, value)| format!("{}={}", form_encode(name), form_encode(value)))
            .collect::<Vec<_>>()
            .join("&");
        let mut request = HttpRequest::new(self.config.token_endpoint.clone());
        request.method = "POST".to_string();
        request.headers = BTreeMap::from([
            (
                "Content-Type".to_string(),
                "application/x-www-form-urlencoded".to_string(),
            ),
            ("Accept".to_string(), "application/json".to_string()),
        ]);
        request.body = Some(body.into_bytes());
        let response = self.transport.send(request).await?;
        let parsed = parse_token_response(&response.body);
        let access_token = parsed
            .as_ref()
            .and_then(|p| p.access_token.clone())
            .filter(|_| (200..300).contains(&response.status));
        let Some(access_token) = access_token else {
            let error = parsed
                .and_then(|p| p.error)
                .unwrap_or_else(|| format!("HTTP {}", response.status));
            // `interaction_required` is Microsoft's "sign in again" (a conditional-access or consent change).
            if is_refresh && (error == "invalid_grant" || error == "interaction_required") {
                return Err(Failure::Source(SourceError::AuthExpired));
            }
            // Only the `error` field is copied, as in Swift; `error_description` and the rest of the body are not.
            return Err(Failure::Source(SourceError::InvalidResponse(format!(
                "oauth error: {error}"
            ))));
        };
        let parsed = parsed.unwrap_or_default();
        Ok(OAuthTokens {
            access_token,
            expires_at: add_seconds((self.now)(), parsed.expires_in.unwrap_or(3600.0)),
            refresh_token: parsed.refresh_token,
        })
    }
}

/// Swift's private `TokenResponse` (decoded with `convertFromSnakeCase`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TokenResponse {
    pub(crate) access_token: Option<String>,
    pub(crate) expires_in: Option<f64>,
    pub(crate) refresh_token: Option<String>,
    pub(crate) error: Option<String>,
}

/// `try? decoder.decode(TokenResponse.self, from: data)`: `None` unless the body is a JSON object whose known keys,
/// when present and not null, have the right type. Unknown keys are ignored. Where `access_token` and `accessToken`
/// are both present the snake-case key wins (Swift's choice depends on dictionary order); a repeated key takes its
/// last value (Swift's first). Any bytes give a result; nothing panics.
pub(crate) fn parse_token_response(body: &[u8]) -> Option<TokenResponse> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let object = value.as_object()?;
    let field = |snake: &str, camel: &str| -> Option<&Value> {
        let present = |key: &str| object.get(key).filter(|v| !v.is_null());
        present(snake).or_else(|| present(camel))
    };
    let string = |snake: &str, camel: &str| -> Result<Option<String>, ()> {
        match field(snake, camel) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(()),
        }
    };
    let number = |snake: &str, camel: &str| -> Result<Option<f64>, ()> {
        match field(snake, camel) {
            None => Ok(None),
            Some(Value::Number(n)) => Ok(n.as_f64()),
            Some(_) => Err(()),
        }
    };
    Some(TokenResponse {
        access_token: string("access_token", "accessToken").ok()?,
        expires_in: number("expires_in", "expiresIn").ok()?,
        refresh_token: string("refresh_token", "refreshToken").ok()?,
        error: string("error", "error").ok()?,
    })
}

/// `addingPercentEncoding(withAllowedCharacters: alphanumerics + "-._~")` for the token request. Swift's
/// `alphanumerics` also lets non-ASCII letters through unescaped; here every non-ASCII character is percent-encoded
/// as UTF-8, which a form parser reads back identically (and which does not depend on the Unicode version of the
/// platform).
fn form_encode(text: &str) -> String {
    percent_encode(text, |b| {
        b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
    })
}

/// What `URLComponents` leaves unescaped in a query item's name or value (probed on macOS): letters, digits and
/// `! $ ' ( ) * + , - . / : ; ? @ _ ~`. Everything else, including space, `& = # % < > " [ \ ] ^ \` { | }` and
/// non-ASCII, is escaped.
fn query_encode(text: &str) -> String {
    percent_encode(text, |b| {
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

fn percent_encode(text: &str, allowed: impl Fn(u8) -> bool) -> String {
    let mut out = String::with_capacity(text.len());
    for &byte in text.as_bytes() {
        if allowed(byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
