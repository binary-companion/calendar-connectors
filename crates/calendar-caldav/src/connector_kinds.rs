//! Port of `CalDAVConnectorKinds.swift`: the iCloud and "Other CalDAV" connector kinds.
//!
//! There is no default transport. The host supplies an [`HttpTransport`] and owes the library the settings in
//! `docs/host-http-contract.md`; the first of them is that **the transport must not follow redirects**: it returns a
//! 3xx answer as the response, because [`crate::webdav_client::WebDavClient`] follows redirects itself, hop by hop, and
//! sends the Basic credentials only to `https` hosts inside the account's host base (or plain `http` to the loopback
//! host). Swift built its default with `URLSessionTransport(followsRedirects: false)`.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use calendar_core::change_monitor::ChangeMonitor;
use calendar_core::connection::{
    AuthorizationInteraction, AuthorizationMethod, Connection, ConnectorKind, CredentialField,
    CredentialHelp, CredentialPromptHelp, CredentialStore, Platform, SyncStateStore,
};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::HttpTransport;
use calendar_core::model::CalendarProvider;
use calendar_core::source_types::{CalendarSource, SourceError};
use calendar_core::time::{Now, Sleeper, Zone};
use calendar_core::url_string::UrlString;

use crate::account_config::CalDavAccountConfig;
use crate::dav_url::DavUrl;
use crate::discovery::CalDavDiscovery;
use crate::source::{CalDavCalendarSource, CalDavSourceSettings};
use crate::webdav_client::{CredentialProvider, WebDavClient, WebDavCredentials};

fn invalid(message: &str) -> Failure {
    Failure::Source(SourceError::InvalidResponse(message.to_string()))
}

/// What the iCloud and "Other CalDAV" kinds share: sign-in (prompt, discover, store), sign-in again, and the source.
pub(crate) struct CalDavAccountSetup {
    pub kind_id: &'static str,
    pub provider: CalendarProvider,
    pub fields: Vec<CredentialField>,
    /// iCloud's fixed server; `None` when the user enters one.
    pub fixed_server: Option<UrlString>,
    /// iCloud's host base (`icloud.com`, which covers the partition hosts); `None` means the entered server's host.
    pub fixed_host_base: Option<&'static str>,
    pub transport: Arc<dyn HttpTransport>,
    pub now: Now,
    pub sleep: Sleeper,
    /// Seconds between checks.
    pub poll_interval: f64,
    pub default_zone: Zone,
}

impl CalDavAccountSetup {
    fn host_base(&self, server: &UrlString) -> String {
        match self.fixed_host_base {
            Some(base) => base.to_string(),
            None => server.host().map(|h| h.to_lowercase()).unwrap_or_default(),
        }
    }

    async fn authorize(
        &self,
        interaction: &dyn AuthorizationInteraction,
        credentials: &dyn CredentialStore,
    ) -> Outcome<Connection> {
        let (account, secret) = self.sign_in(interaction).await?;
        let mut connection = Connection::new(
            self.kind_id,
            uuid::Uuid::new_v4().to_string().to_uppercase(),
            self.display_name(&account),
        );
        connection.config = account.config();
        credentials
            .set_secrets(Self::secrets_of(&secret), &connection.connection_id)
            .await?;
        Ok(connection)
    }

    async fn reauthorize(
        &self,
        connection: &Connection,
        interaction: &dyn AuthorizationInteraction,
        credentials: &dyn CredentialStore,
    ) -> Outcome<Connection> {
        let previous = CalDavAccountConfig::from_config(&connection.config)?;
        let (account, secret) = self.sign_in(interaction).await?;
        if DavUrl::path(&account.principal_url) != DavUrl::path(&previous.principal_url)
            || self.host_base(&account.server_url) != self.host_base(&previous.server_url)
        {
            return Err(invalid("signed in as a different account"));
        }
        credentials
            .set_secrets(Self::secrets_of(&secret), &connection.connection_id)
            .await?;
        let mut updated = connection.clone();
        updated.config = account.config();
        Ok(updated)
    }

    fn secrets_of(secret: &WebDavCredentials) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("username".to_string(), secret.username.clone()),
            ("password".to_string(), secret.password.clone()),
        ])
    }

    /// The source itself (the kinds hand it out as a `CalendarSource`).
    pub(crate) fn make_calendar_source(
        &self,
        connection: &Connection,
        credentials: Arc<dyn CredentialStore>,
        sync_state: Arc<dyn SyncStateStore>,
    ) -> Outcome<Arc<CalDavCalendarSource>> {
        let account = CalDavAccountConfig::from_config(&connection.config)?;
        let connection_id = connection.connection_id.clone();
        let provider: CredentialProvider = Arc::new(move || {
            let credentials = credentials.clone();
            let connection_id = connection_id.clone();
            Box::pin(async move {
                let secrets = credentials.secrets(&connection_id).await?;
                match secrets
                    .as_ref()
                    .and_then(|s| Some((s.get("username")?, s.get("password")?)))
                {
                    Some((username, password)) => Ok(WebDavCredentials {
                        username: username.clone(),
                        password: password.clone(),
                    }),
                    None => Err(Failure::Source(SourceError::AuthExpired)),
                }
            })
        });
        let client = WebDavClient::new(
            self.transport.clone(),
            &self.host_base(&account.server_url),
            provider,
        );
        Ok(CalDavCalendarSource::new(CalDavSourceSettings::new(
            connection.clone(),
            account,
            client,
            self.provider.clone(),
            sync_state,
            ChangeMonitor::new(self.poll_interval, 900.0, self.sleep.clone()),
            self.now.clone(),
            self.default_zone.clone(),
        )))
    }

    /// Nothing is stored here: callers store secrets only after discovery succeeds.
    async fn sign_in(
        &self,
        interaction: &dyn AuthorizationInteraction,
    ) -> Outcome<(CalDavAccountConfig, WebDavCredentials)> {
        let values = interaction.prompt_credentials(&self.fields).await?;
        let username = values
            .get("username")
            .map_or("", String::as_str)
            .trim()
            .to_string();
        let password = values.get("password").cloned().unwrap_or_default();
        if username.is_empty() || password.is_empty() {
            return Err(invalid("a user name and password are required"));
        }
        let server = match &self.fixed_server {
            Some(server) => server.clone(),
            None => Self::server_url_from(values.get("serverURL").map_or("", String::as_str))?,
        };
        let secret = WebDavCredentials {
            username: username.clone(),
            password,
        };
        let client = WebDavClient::new(
            self.transport.clone(),
            &self.host_base(&server),
            WebDavClient::fixed_credentials(secret.clone()),
        );
        let account = CalDavDiscovery { client }
            .discover(&server, &username)
            .await?;
        Ok((account, secret))
    }

    /// The address the user typed: `https://` is assumed when no scheme is given; plain `http` only to the loopback host.
    pub(crate) fn server_url_from(text: &str) -> Outcome<UrlString> {
        let mut trimmed = text.trim().to_string();
        if !trimmed.contains("://") {
            trimmed = format!("https://{trimmed}");
        }
        let url = UrlString::parse(&trimmed);
        let (Some(url), host, scheme) = (
            url.clone(),
            url.as_ref()
                .and_then(|u| u.host())
                .map(|h| h.to_lowercase())
                .filter(|h| !h.is_empty()),
            url.as_ref()
                .and_then(|u| u.scheme())
                .map(|s| s.to_lowercase()),
        ) else {
            return Err(invalid("the server address is not a valid URL"));
        };
        let (Some(host), Some(scheme)) = (host, scheme) else {
            return Err(invalid("the server address is not a valid URL"));
        };
        if !(scheme == "https"
            || (scheme == "http" && ["localhost", "127.0.0.1"].contains(&host.as_str())))
        {
            return Err(invalid("the server address must start with https://"));
        }
        if url.user().is_some() || url.password().is_some() {
            return Err(invalid(
                "enter the user name and password in their own fields, not in the server address",
            ));
        }
        Ok(url)
    }

    /// The Apple ID for iCloud; for another server the user name, with the host when the name is not an address.
    fn display_name(&self, account: &CalDavAccountConfig) -> String {
        if self.fixed_server.is_some() || account.username.contains('@') {
            return account.username.clone();
        }
        format!(
            "{}@{}",
            account.username,
            account.server_url.host().unwrap_or_default()
        )
    }
}

fn all_platforms() -> Platform {
    Platform::MACOS | Platform::IOS | Platform::LINUX | Platform::WINDOWS
}

/// iCloud through CalDAV: an Apple ID and an app-specific password; the server is fixed.
///
/// `transport` must not follow redirects (see the module documentation and `docs/host-http-contract.md`). `now`,
/// `sleep` and `default_zone` are required because this library reads no clock, sleeps through the host's sleeper only
/// and has no "current" zone (Swift defaulted them to `Date()`, `Task.sleep` and `TimeZone.current`). The poll interval
/// is 60 seconds; change it with [`ICloudConnectorKind::with_poll_interval`].
pub struct ICloudConnectorKind {
    setup: CalDavAccountSetup,
}

impl ICloudConnectorKind {
    pub const KIND_ID: &'static str = "icloud";
    pub const SERVER_URL: &'static str = "https://caldav.icloud.com";

    pub fn new(
        transport: Arc<dyn HttpTransport>,
        now: Now,
        sleep: Sleeper,
        default_zone: Zone,
    ) -> ICloudConnectorKind {
        ICloudConnectorKind {
            setup: CalDavAccountSetup {
                kind_id: Self::KIND_ID,
                provider: CalendarProvider::icloud(),
                fields: vec![
                    CredentialField::new("username", "Apple ID"),
                    CredentialField::new("password", "App-specific password").secret(),
                ],
                fixed_server: UrlString::parse(Self::SERVER_URL),
                fixed_host_base: Some("icloud.com"),
                transport,
                now,
                sleep,
                poll_interval: 60.0,
                default_zone,
            },
        }
    }

    /// Seconds between checks.
    #[must_use]
    pub fn with_poll_interval(mut self, seconds: f64) -> ICloudConnectorKind {
        self.setup.poll_interval = seconds;
        self
    }

    /// The source `make_source` hands out, as its own type (the live smoke test drives it directly).
    pub fn make_calendar_source(
        &self,
        connection: &Connection,
        credentials: Arc<dyn CredentialStore>,
        sync_state: Arc<dyn SyncStateStore>,
    ) -> Outcome<Arc<CalDavCalendarSource>> {
        self.setup
            .make_calendar_source(connection, credentials, sync_state)
    }
}

impl CredentialPromptHelp for ICloudConnectorKind {
    fn credential_help(&self) -> Option<CredentialHelp> {
        Some(CredentialHelp {
            text: "iCloud needs an app-specific password, not your Apple Account password. Create one at account.apple.com under Sign-In and Security."
                .to_string(),
            link_title: Some("Create an app-specific password".to_string()),
            url: UrlString::parse("https://account.apple.com/account/manage"),
        })
    }
}

#[async_trait]
impl ConnectorKind for ICloudConnectorKind {
    fn id(&self) -> String {
        Self::KIND_ID.to_string()
    }

    fn display_name(&self) -> String {
        "iCloud".to_string()
    }

    fn supported_platforms(&self) -> Platform {
        all_platforms()
    }

    fn authorization(&self) -> AuthorizationMethod {
        AuthorizationMethod::Password {
            fields: self.setup.fields.clone(),
        }
    }

    async fn authorize(
        &self,
        interaction: &dyn AuthorizationInteraction,
        credentials: &dyn CredentialStore,
    ) -> Outcome<Connection> {
        self.setup.authorize(interaction, credentials).await
    }

    async fn reauthorize(
        &self,
        connection: &Connection,
        interaction: &dyn AuthorizationInteraction,
        credentials: &dyn CredentialStore,
    ) -> Outcome<Connection> {
        self.setup
            .reauthorize(connection, interaction, credentials)
            .await
    }

    fn make_source(
        &self,
        connection: &Connection,
        credentials: Arc<dyn CredentialStore>,
        sync_state: Arc<dyn SyncStateStore>,
    ) -> Outcome<Arc<dyn CalendarSource>> {
        Ok(self
            .setup
            .make_calendar_source(connection, credentials, sync_state)?)
    }

    fn as_credential_prompt_help(&self) -> Option<&dyn CredentialPromptHelp> {
        Some(self)
    }
}

/// Any other CalDAV server (Fastmail, Nextcloud, ...): the user enters the server address, user name and password.
/// The constructor's arguments are those of [`ICloudConnectorKind::new`].
pub struct CalDavConnectorKind {
    setup: CalDavAccountSetup,
}

impl CalDavConnectorKind {
    pub const KIND_ID: &'static str = "caldav";

    pub fn new(
        transport: Arc<dyn HttpTransport>,
        now: Now,
        sleep: Sleeper,
        default_zone: Zone,
    ) -> CalDavConnectorKind {
        CalDavConnectorKind {
            setup: CalDavAccountSetup {
                kind_id: Self::KIND_ID,
                provider: CalendarProvider::cal_dav(),
                fields: vec![
                    CredentialField::new("serverURL", "Server address"),
                    CredentialField::new("username", "User name"),
                    CredentialField::new("password", "Password").secret(),
                ],
                fixed_server: None,
                fixed_host_base: None,
                transport,
                now,
                sleep,
                poll_interval: 60.0,
                default_zone,
            },
        }
    }

    /// Seconds between checks.
    #[must_use]
    pub fn with_poll_interval(mut self, seconds: f64) -> CalDavConnectorKind {
        self.setup.poll_interval = seconds;
        self
    }

    /// The source `make_source` hands out, as its own type.
    pub fn make_calendar_source(
        &self,
        connection: &Connection,
        credentials: Arc<dyn CredentialStore>,
        sync_state: Arc<dyn SyncStateStore>,
    ) -> Outcome<Arc<CalDavCalendarSource>> {
        self.setup
            .make_calendar_source(connection, credentials, sync_state)
    }
}

impl CredentialPromptHelp for CalDavConnectorKind {
    fn credential_help(&self) -> Option<CredentialHelp> {
        Some(CredentialHelp::new(
            "Use the CalDAV address your provider gives you, for example https://caldav.fastmail.com. Many providers need an app password here.",
        ))
    }
}

#[async_trait]
impl ConnectorKind for CalDavConnectorKind {
    fn id(&self) -> String {
        Self::KIND_ID.to_string()
    }

    fn display_name(&self) -> String {
        "Other CalDAV".to_string()
    }

    fn supported_platforms(&self) -> Platform {
        all_platforms()
    }

    fn authorization(&self) -> AuthorizationMethod {
        AuthorizationMethod::Password {
            fields: self.setup.fields.clone(),
        }
    }

    async fn authorize(
        &self,
        interaction: &dyn AuthorizationInteraction,
        credentials: &dyn CredentialStore,
    ) -> Outcome<Connection> {
        self.setup.authorize(interaction, credentials).await
    }

    async fn reauthorize(
        &self,
        connection: &Connection,
        interaction: &dyn AuthorizationInteraction,
        credentials: &dyn CredentialStore,
    ) -> Outcome<Connection> {
        self.setup
            .reauthorize(connection, interaction, credentials)
            .await
    }

    fn make_source(
        &self,
        connection: &Connection,
        credentials: Arc<dyn CredentialStore>,
        sync_state: Arc<dyn SyncStateStore>,
    ) -> Outcome<Arc<dyn CalendarSource>> {
        Ok(self
            .setup
            .make_calendar_source(connection, credentials, sync_state)?)
    }

    fn as_credential_prompt_help(&self) -> Option<&dyn CredentialPromptHelp> {
        Some(self)
    }
}
