//! Port of `MicrosoftBrokeredConnectorKind.swift`: Microsoft accounts whose access tokens come from the host
//! (Windows' Web Account Manager, a mobile account broker) instead of the library's browser flow. The library stores
//! no credential for such an account: the host's account service owns the password, MFA, policy and the refresh
//! token, and the library asks for short-lived access tokens. The host decides whether to offer the kind by
//! registering it with an `ExternalTokenSource`.
//!
//! A separate kind next to `MicrosoftConnectorKind` (not a mode of it), so nothing about the browser kind changes.

use std::sync::Arc;

use async_trait::async_trait;
use calendar_core::change_monitor::ChangeMonitor;
use calendar_core::connection::{
    AuthorizationInteraction, AuthorizationMethod, Connection, ConnectorKind, CredentialStore,
    Platform, SyncStateStore,
};
use calendar_core::external_tokens::{
    AccountIdentityFamily, ExternalAccount, ExternalSignInRequest, ExternalSignInResult,
    ExternalTokenSource,
};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::HttpTransport;
use calendar_core::source_types::{CalendarSource, SourceError};
use calendar_core::time::{Now, Sleeper};
use calendar_oauth::{AccessTokenSupplying, ExternalAccessTokenProvider};
use uuid::Uuid;

use crate::api_client::GraphApiClient;
use crate::connector_kind::MicrosoftConnectorKind;
use crate::source::MicrosoftCalendarSource;

#[derive(Clone)]
pub struct MicrosoftBrokeredConnectorKind {
    token_source: Arc<dyn ExternalTokenSource>,
    transport: Arc<dyn HttpTransport>,
    now: Now,
    sleep: Sleeper,
    /// Seconds between checks.
    poll_interval: f64,
}

impl MicrosoftBrokeredConnectorKind {
    pub const KIND_ID: &'static str = "microsoft-brokered";

    /// `Connection.config` keys this kind writes, besides `email`.
    const AUTH_MODE_KEY: &'static str = "authMode";
    const AUTH_MODE_EXTERNAL: &'static str = "external";
    const EXTERNAL_ACCOUNT_ID_KEY: &'static str = "externalAccountID";

    /// `MicrosoftConnectorKind::SCOPES` without `offline_access`: the broker keeps the refresh token itself.
    /// Read-only on purpose (ADR 0017), like the browser kind.
    pub fn scopes() -> Vec<String> {
        MicrosoftConnectorKind::SCOPES
            .iter()
            .filter(|scope| **scope != "offline_access")
            .map(|scope| (*scope).to_string())
            .collect()
    }

    /// Swift's `init` with `pollInterval` 60 seconds; change it with
    /// [`MicrosoftBrokeredConnectorKind::with_poll_interval`].
    pub fn new(
        token_source: Arc<dyn ExternalTokenSource>,
        transport: Arc<dyn HttpTransport>,
        now: Now,
        sleep: Sleeper,
    ) -> MicrosoftBrokeredConnectorKind {
        MicrosoftBrokeredConnectorKind {
            token_source,
            transport,
            now,
            sleep,
            poll_interval: 60.0,
        }
    }

    #[must_use]
    pub fn with_poll_interval(mut self, seconds: f64) -> MicrosoftBrokeredConnectorKind {
        self.poll_interval = seconds;
        self
    }

    /// `make_source`, with the concrete source type.
    pub(crate) fn make_microsoft_source(
        &self,
        connection: &Connection,
        _credentials: Arc<dyn CredentialStore>,
        sync_state: Arc<dyn SyncStateStore>,
    ) -> Outcome<Arc<MicrosoftCalendarSource>> {
        let Some(account_id) = connection
            .config
            .get(Self::EXTERNAL_ACCOUNT_ID_KEY)
            .filter(|id| !id.is_empty())
        else {
            return Err(Failure::Source(SourceError::InvalidResponse(
                "the account settings are incomplete; sign in again".to_string(),
            )));
        };
        let provider = ExternalAccessTokenProvider::new(
            ExternalAccount {
                id: account_id.clone(),
                email: connection.config.get("email").cloned(),
                connection_id: Some(connection.connection_id.clone()),
            },
            Self::scopes(),
            self.token_source.clone(),
            self.now.clone(),
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

    /// Runs the host's sign-in, then asks Graph who signed in: the email comes from Graph, never from the host.
    async fn sign_in(&self, request: ExternalSignInRequest) -> Outcome<(String, String)> {
        let result: ExternalSignInResult = match self.token_source.sign_in(&request).await {
            Ok(result) => result,
            Err(error @ Failure::Source(_)) => return Err(error),
            Err(Failure::Cancelled) => return Err(Failure::Cancelled),
            // Not the error's own description: it could carry account data.
            Err(_) => {
                return Err(Failure::Source(SourceError::InvalidResponse(
                    "the Windows account service failed".to_string(),
                )));
            }
        };
        let api = GraphApiClient {
            transport: self.transport.clone(),
            tokens: Arc::new(SignInToken(result.token.access_token.clone())),
            sleep: self.sleep.clone(),
        };
        Ok((api.account_email().await?, result.account.id))
    }
}

/// The token a sign-in returned, good for the one `/me` request after it. It cannot refresh: a 401 is `AuthExpired`.
struct SignInToken(String);

#[async_trait]
impl AccessTokenSupplying for SignInToken {
    async fn access_token(&self) -> Outcome<String> {
        Ok(self.0.clone())
    }

    async fn invalidate(&self) {}
}

impl AccountIdentityFamily for MicrosoftBrokeredConnectorKind {
    /// The same mailbox through the browser kind and through this one is one account.
    fn identity_family(&self) -> String {
        "microsoft".to_string()
    }
}

#[async_trait]
impl ConnectorKind for MicrosoftBrokeredConnectorKind {
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

    /// The host shows its own sign-in; `interaction` and `credentials` are not used. Nothing is persisted here, and
    /// nothing is stored on failure.
    async fn authorize(
        &self,
        _interaction: &dyn AuthorizationInteraction,
        _credentials: &dyn CredentialStore,
    ) -> Outcome<Connection> {
        let (email, account_id) = self
            .sign_in(ExternalSignInRequest::new(Self::scopes()))
            .await?;
        // `UUID().uuidString` is uppercase.
        Ok(Connection::new(
            Self::KIND_ID,
            Uuid::new_v4().to_string().to_uppercase(),
            email.clone(),
        )
        .with_config(&[
            ("email", &email),
            (Self::AUTH_MODE_KEY, Self::AUTH_MODE_EXTERNAL),
            (Self::EXTERNAL_ACCOUNT_ID_KEY, &account_id),
        ]))
    }

    async fn reauthorize(
        &self,
        connection: &Connection,
        _interaction: &dyn AuthorizationInteraction,
        _credentials: &dyn CredentialStore,
    ) -> Outcome<Connection> {
        let stored_email = connection.config.get("email").cloned();
        // A connection without an account id (see `make_source`) is reconnected without one: the person picks.
        let account = connection
            .config
            .get(Self::EXTERNAL_ACCOUNT_ID_KEY)
            .filter(|id| !id.is_empty())
            .map(|id| ExternalAccount {
                id: id.clone(),
                email: stored_email.clone(),
                connection_id: Some(connection.connection_id.clone()),
            });
        let (email, account_id) = self
            .sign_in(ExternalSignInRequest {
                account,
                email: stored_email.clone(),
                scopes: Self::scopes(),
            })
            .await?;
        // The display name is the email by construction, so a record that lost its `email` key is still guarded.
        let expected = stored_email.unwrap_or_else(|| connection.display_name.clone());
        if email != expected.to_lowercase() {
            return Err(Failure::Source(SourceError::InvalidResponse(
                "signed in as a different account".to_string(),
            )));
        }
        let mut result = connection.clone();
        result.config.insert("email".to_string(), email);
        result.config.insert(
            Self::AUTH_MODE_KEY.to_string(),
            Self::AUTH_MODE_EXTERNAL.to_string(),
        );
        result
            .config
            .insert(Self::EXTERNAL_ACCOUNT_ID_KEY.to_string(), account_id);
        Ok(result)
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
