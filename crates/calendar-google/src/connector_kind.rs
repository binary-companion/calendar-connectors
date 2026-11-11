//! Port of `GoogleConnectorKind.swift`: the Google connector kind (sign-in through the OAuth browser flow with PKCE,
//! and the source).
//!
//! There is no default transport. The host supplies an [`HttpTransport`] and owes the library the settings in
//! `docs/host-http-contract.md`. Swift's `hasher` parameter is gone with the `SHA256Hashing` seam (`sha2` replaces it).

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use calendar_core::change_monitor::ChangeMonitor;
use calendar_core::connection::{
    AuthorizationInteraction, AuthorizationMethod, Connection, ConnectorKind, CredentialStore,
    InMemoryCredentialStore, Platform, SyncStateStore,
};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::HttpTransport;
use calendar_core::source_types::{CalendarSource, SourceError};
use calendar_core::time::{Now, Sleeper};
use calendar_core::url_string::UrlString;
use calendar_oauth::{
    AccessTokenProvider, OAuthClient, OAuthConfig, OAuthConnections, OAuthTokens, RefreshFunction,
    pkce,
};

use crate::api_client::GoogleApiClient;
use crate::source::GoogleCalendarSource;

/// The host app's Google Cloud OAuth client (type Desktop). Supplied from git-ignored configuration; never embedded
/// here. `Debug` hides the secret.
#[derive(Clone, PartialEq, Eq)]
pub struct GoogleOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
}

impl GoogleOAuthConfig {
    pub fn new(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> GoogleOAuthConfig {
        GoogleOAuthConfig {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
        }
    }
}

impl fmt::Debug for GoogleOAuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GoogleOAuthConfig")
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .finish()
    }
}

#[derive(Clone)]
pub struct GoogleConnectorKind {
    /// `None` only if `UrlString` refused one of the two constant OAuth endpoints (Swift force-unwraps them); sign-in
    /// and refresh then fail with `InvalidResponse` instead of the library panicking.
    oauth: Option<OAuthClient>,
    transport: Arc<dyn HttpTransport>,
    now: Now,
    sleep: Sleeper,
    /// Seconds between checks.
    poll_interval: f64,
}

impl GoogleConnectorKind {
    pub const KIND_ID: &'static str = "google";
    /// Read-only, as in Swift (`scripts/ci/check-repository-contracts.py` pins them).
    pub const SCOPES: [&'static str; 2] = [
        "https://www.googleapis.com/auth/calendar.events.readonly",
        "https://www.googleapis.com/auth/calendar.calendarlist.readonly",
    ];

    /// Swift's `init` with `pollInterval` 60 seconds; change it with [`GoogleConnectorKind::with_poll_interval`]. The
    /// host's `transport` must follow the rules of `docs/host-http-contract.md`; `sleep` waits between polls and
    /// between rate-limit retries.
    pub fn new(
        config: GoogleOAuthConfig,
        transport: Arc<dyn HttpTransport>,
        now: Now,
        sleep: Sleeper,
    ) -> GoogleConnectorKind {
        let oauth = UrlString::parse("https://accounts.google.com/o/oauth2/v2/auth")
            .zip(UrlString::parse("https://oauth2.googleapis.com/token"))
            .map(|(authorization_endpoint, token_endpoint)| {
                let mut oauth_config = OAuthConfig::new(
                    authorization_endpoint,
                    token_endpoint,
                    config.client_id,
                    Self::SCOPES
                        .iter()
                        .map(|scope| (*scope).to_string())
                        .collect(),
                );
                oauth_config.client_secret = Some(config.client_secret);
                oauth_config
                    .extra_auth_params
                    .insert("access_type".to_string(), "offline".to_string());
                oauth_config
                    .extra_auth_params
                    .insert("prompt".to_string(), "consent".to_string());
                OAuthClient::new(oauth_config, transport.clone(), now.clone())
            });
        GoogleConnectorKind {
            oauth,
            transport,
            now,
            sleep,
            poll_interval: 60.0,
        }
    }

    pub fn with_poll_interval(mut self, seconds: f64) -> GoogleConnectorKind {
        self.poll_interval = seconds;
        self
    }

    fn oauth(&self) -> Outcome<&OAuthClient> {
        self.oauth.as_ref().ok_or_else(|| {
            Failure::Source(SourceError::InvalidResponse(
                "google: the OAuth endpoints are not valid".to_string(),
            ))
        })
    }

    /// Runs the browser flow and returns the account email (the primary calendar id) and the refresh token.
    /// Nothing is persisted here: callers store secrets only after this succeeds.
    async fn sign_in(
        &self,
        interaction: &dyn AuthorizationInteraction,
    ) -> Outcome<(String, String)> {
        let oauth = self.oauth()?;
        let session = interaction.begin_oauth_redirect().await?;
        let exchanged: Outcome<OAuthTokens> = async {
            let redirect_uri = session.redirect_uri();
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
        let api = GoogleApiClient {
            transport: self.transport.clone(),
            tokens: Arc::new(provider),
            sleep: self.sleep.clone(),
        };
        let Some(primary) = api
            .calendar_list()
            .await?
            .into_iter()
            .find(|entry| entry.primary == Some(true))
        else {
            return Err(Failure::Source(SourceError::InvalidResponse(
                "no primary calendar".to_string(),
            )));
        };
        Ok((primary.id.to_lowercase(), refresh_token))
    }
}

#[async_trait]
impl ConnectorKind for GoogleConnectorKind {
    fn id(&self) -> String {
        Self::KIND_ID.to_string()
    }

    fn display_name(&self) -> String {
        "Google".to_string()
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
        let api = GoogleApiClient {
            transport: self.transport.clone(),
            tokens: Arc::new(provider),
            sleep: self.sleep.clone(),
        };
        Ok(GoogleCalendarSource::new(
            connection.clone(),
            api,
            sync_state,
            ChangeMonitor::new(self.poll_interval, 900.0, self.sleep.clone()),
        ))
    }
}
