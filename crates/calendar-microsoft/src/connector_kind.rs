//! Port of `MicrosoftConnectorKind.swift`: the Microsoft connector kind (sign-in through the OAuth browser flow with
//! PKCE, and the source).
//!
//! There is no default transport. The host supplies an [`HttpTransport`] and owes the library the settings in
//! `docs/host-http-contract.md`. Swift's `hasher` parameter is gone with the `SHA256Hashing` seam (`sha2` replaces it).

use std::sync::Arc;

use async_trait::async_trait;
use calendar_core::change_monitor::ChangeMonitor;
use calendar_core::connection::{
    AuthorizationInteraction, AuthorizationMethod, Connection, ConnectorKind, CredentialStore,
    InMemoryCredentialStore, Platform, SyncStateStore,
};
use calendar_core::external_tokens::AccountIdentityFamily;
use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::HttpTransport;
use calendar_core::source_types::{CalendarSource, SourceError};
use calendar_core::time::{Now, Sleeper};
use calendar_core::url_string::UrlString;
use calendar_oauth::{
    AccessTokenProvider, OAuthClient, OAuthConfig, OAuthConnections, OAuthTokens, RefreshFunction,
    pkce,
};

use crate::api_client::GraphApiClient;
use crate::source::MicrosoftCalendarSource;

/// The host app's Microsoft Entra app registration (a public client: no secret). Supplied from git-ignored
/// configuration; never embedded here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MicrosoftOAuthConfig {
    pub client_id: String,
    /// The host name the redirect URI is sent with. The registration lists `http://localhost` (Microsoft ignores the
    /// port for a loopback redirect), while the loopback listener reports `127.0.0.1`; the browser reaches the
    /// listener either way. `None` keeps the session's own host.
    pub redirect_host: Option<String>,
}

impl MicrosoftOAuthConfig {
    /// Swift's `init(clientID:redirectHost:)` with the default redirect host, `localhost`.
    pub fn new(client_id: impl Into<String>) -> MicrosoftOAuthConfig {
        MicrosoftOAuthConfig {
            client_id: client_id.into(),
            redirect_host: Some("localhost".to_string()),
        }
    }

    /// Replaces the redirect host (`None` keeps the session's own).
    #[must_use]
    pub fn with_redirect_host(mut self, host: Option<&str>) -> MicrosoftOAuthConfig {
        self.redirect_host = host.map(str::to_string);
        self
    }
}

#[derive(Clone)]
pub struct MicrosoftConnectorKind {
    config: MicrosoftOAuthConfig,
    /// `None` only if `UrlString` refused one of the two constant OAuth endpoints (Swift force-unwraps them); sign-in
    /// and refresh then fail with `InvalidResponse` instead of the library panicking.
    oauth: Option<OAuthClient>,
    transport: Arc<dyn HttpTransport>,
    now: Now,
    sleep: Sleeper,
    /// Seconds between checks.
    poll_interval: f64,
}

impl MicrosoftConnectorKind {
    pub const KIND_ID: &'static str = "microsoft";
    /// `offline_access` returns the refresh token; `MailboxSettings.Read` is the account's time zone; `.Shared` reads
    /// the shared and delegated calendars the account can open. Read-only on purpose (ADR 0017): the host app only
    /// reads events, so the source's write calls get a 403 until the scopes are widened to `Calendars.ReadWrite` and
    /// `Calendars.ReadWrite.Shared` and the user signs in again. `scripts/ci/check-repository-contracts.py` pins them.
    pub const SCOPES: [&'static str; 5] = [
        "offline_access",
        "User.Read",
        "MailboxSettings.Read",
        "Calendars.Read",
        "Calendars.Read.Shared",
    ];

    /// Swift's `init` with `pollInterval` 60 seconds; change it with [`MicrosoftConnectorKind::with_poll_interval`].
    /// The host's `transport` must follow the rules of `docs/host-http-contract.md`; `sleep` waits between polls and
    /// between throttling retries.
    pub fn new(
        config: MicrosoftOAuthConfig,
        transport: Arc<dyn HttpTransport>,
        now: Now,
        sleep: Sleeper,
    ) -> MicrosoftConnectorKind {
        let oauth =
            UrlString::parse("https://login.microsoftonline.com/common/oauth2/v2.0/authorize")
                .zip(UrlString::parse(
                    "https://login.microsoftonline.com/common/oauth2/v2.0/token",
                ))
                .map(|(authorization_endpoint, token_endpoint)| {
                    let mut oauth_config = OAuthConfig::new(
                        authorization_endpoint,
                        token_endpoint,
                        config.client_id.clone(),
                        Self::SCOPES
                            .iter()
                            .map(|scope| (*scope).to_string())
                            .collect(),
                    );
                    // Without this a browser already signed in to one Microsoft account signs in as it without asking.
                    oauth_config
                        .extra_auth_params
                        .insert("prompt".to_string(), "select_account".to_string());
                    OAuthClient::new(oauth_config, transport.clone(), now.clone())
                });
        MicrosoftConnectorKind {
            config,
            oauth,
            transport,
            now,
            sleep,
            poll_interval: 60.0,
        }
    }

    #[must_use]
    pub fn with_poll_interval(mut self, seconds: f64) -> MicrosoftConnectorKind {
        self.poll_interval = seconds;
        self
    }

    fn oauth(&self) -> Outcome<&OAuthClient> {
        self.oauth.as_ref().ok_or_else(|| {
            Failure::Source(SourceError::InvalidResponse(
                "microsoft: the OAuth endpoints are not valid".to_string(),
            ))
        })
    }

    /// The redirect URI with the configured host in place of the session's.
    fn redirect_uri(&self, session_redirect: &UrlString) -> UrlString {
        match &self.config.redirect_host {
            Some(host) => replacing_host(session_redirect, host),
            None => session_redirect.clone(),
        }
    }

    /// Runs the browser flow and returns the account's email address and the refresh token. Nothing is persisted here:
    /// callers store secrets only after this succeeds.
    async fn sign_in(
        &self,
        interaction: &dyn AuthorizationInteraction,
    ) -> Outcome<(String, String)> {
        let oauth = self.oauth()?;
        let session = interaction.begin_oauth_redirect().await?;
        let exchanged: Outcome<OAuthTokens> = async {
            let redirect_uri = self.redirect_uri(&session.redirect_uri());
            let verifier = pkce::random_string(64)?;
            let state = pkce::random_string(32)?;
            let url = oauth.authorization_url(&redirect_uri, &state, &pkce::challenge(&verifier));
            let redirect = session.authorize(&url).await?;
            let code = oauth.authorization_code(&redirect, &state)?;
            oauth.exchange(&code, &verifier, &redirect_uri).await
        }
        .await;
        session.close().await;
        let tokens = exchanged?;

        let Some(refresh_token) = tokens.refresh_token.clone() else {
            return Err(Failure::Source(SourceError::InvalidResponse(
                "no refresh token returned".to_string(),
            )));
        };
        let refresh: RefreshFunction =
            Arc::new(|_| Box::pin(async { Err(Failure::Source(SourceError::AuthExpired)) }));
        let provider = AccessTokenProvider::new(
            "signin",
            Arc::new(InMemoryCredentialStore::new()),
            refresh,
            self.now.clone(),
            Some(tokens),
        );
        let api = GraphApiClient {
            transport: self.transport.clone(),
            tokens: Arc::new(provider),
            sleep: self.sleep.clone(),
        };
        Ok((api.account_email().await?, refresh_token))
    }

    /// `make_source`, with the concrete source type.
    pub(crate) fn make_microsoft_source(
        &self,
        connection: &Connection,
        credentials: Arc<dyn CredentialStore>,
        sync_state: Arc<dyn SyncStateStore>,
    ) -> Outcome<Arc<MicrosoftCalendarSource>> {
        let oauth = self.oauth()?.clone();
        let refresh: RefreshFunction = Arc::new(move |token: String| {
            let oauth = oauth.clone();
            Box::pin(async move { oauth.refresh(&token).await })
        });
        let provider = AccessTokenProvider::new(
            connection.connection_id.clone(),
            credentials,
            refresh,
            self.now.clone(),
            None,
        );
        let api = GraphApiClient {
            transport: self.transport.clone(),
            tokens: Arc::new(provider),
            sleep: self.sleep.clone(),
        };
        Ok(MicrosoftCalendarSource::new(
            connection.clone(),
            api,
            sync_state,
            ChangeMonitor::new(self.poll_interval, 900.0, self.sleep.clone()),
            self.now.clone(),
        ))
    }
}

/// `URLComponents.host = host`: the host of `url` replaced, the user, port, path, query and fragment kept as written.
/// Returns `url` unchanged when it has no authority or the result is not a valid address.
pub(crate) fn replacing_host(url: &UrlString, host: &str) -> UrlString {
    let text = url.as_str();
    let Some(scheme_end) = text.find("://") else {
        return url.clone();
    };
    let authority_start = scheme_end + 3;
    let authority_end = text[authority_start..]
        .find(['/', '?', '#'])
        .map_or(text.len(), |at| authority_start + at);
    let authority = &text[authority_start..authority_end];
    let (user, host_and_port) = match authority.rfind('@') {
        Some(at) => (&authority[..=at], &authority[at + 1..]),
        None => ("", authority),
    };
    let port = if host_and_port.starts_with('[') {
        host_and_port
            .find(']')
            .map(|close| &host_and_port[close + 1..])
            .filter(|rest| rest.starts_with(':'))
    } else {
        host_and_port
            .rfind(':')
            .map(|colon| &host_and_port[colon..])
    }
    .unwrap_or("");
    let rebuilt = format!(
        "{}{user}{host}{port}{}",
        &text[..authority_start],
        &text[authority_end..]
    );
    UrlString::parse(&rebuilt).unwrap_or_else(|| url.clone())
}

impl AccountIdentityFamily for MicrosoftConnectorKind {
    /// Shared with `MicrosoftBrokeredConnectorKind`: the same mailbox through either kind is one account.
    fn identity_family(&self) -> String {
        "microsoft".to_string()
    }
}

#[async_trait]
impl ConnectorKind for MicrosoftConnectorKind {
    fn id(&self) -> String {
        Self::KIND_ID.to_string()
    }

    fn display_name(&self) -> String {
        "Microsoft".to_string()
    }

    fn supported_platforms(&self) -> Platform {
        Platform::MACOS | Platform::IOS | Platform::LINUX | Platform::WINDOWS
    }

    fn authorization(&self) -> AuthorizationMethod {
        AuthorizationMethod::Oauth
    }

    async fn authorize(
        &self,
        interaction: &dyn AuthorizationInteraction,
        credentials: &dyn CredentialStore,
    ) -> Outcome<Connection> {
        OAuthConnections::authorize(Self::KIND_ID, credentials, || self.sign_in(interaction)).await
    }

    async fn reauthorize(
        &self,
        connection: &Connection,
        interaction: &dyn AuthorizationInteraction,
        credentials: &dyn CredentialStore,
    ) -> Outcome<Connection> {
        OAuthConnections::reauthorize(connection, credentials, || self.sign_in(interaction)).await
    }

    fn make_source(
        &self,
        connection: &Connection,
        credentials: Arc<dyn CredentialStore>,
        sync_state: Arc<dyn SyncStateStore>,
    ) -> Outcome<Arc<dyn CalendarSource>> {
        let source: Arc<dyn CalendarSource> =
            self.make_microsoft_source(connection, credentials, sync_state)?;
        Ok(source)
    }

    fn as_identity_family(&self) -> Option<&dyn AccountIdentityFamily> {
        Some(self)
    }
}
