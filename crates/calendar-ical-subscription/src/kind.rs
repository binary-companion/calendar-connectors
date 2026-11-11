//! Port of `ICalSubscriptionKind.swift`.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use calendar_core::change_monitor::ChangeMonitor;
use calendar_core::connection::{
    AuthorizationInteraction, AuthorizationMethod, Connection, ConnectorKind, CredentialField,
    CredentialHelp, CredentialPromptHelp, CredentialStore, Platform, SyncStateStore,
};
use calendar_core::diagnostic_log::{
    DiagnosticField, DiagnosticLevel, DiagnosticLog, NullDiagnosticLog,
};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::HttpTransport;
use calendar_core::source_types::{CalendarSource, SourceError};
use calendar_core::time::{Now, Sleeper, Zone};
use calendar_core::url_string::UrlString;

use crate::feed_fetcher::{FeedFetcher, FeedResponse};
use crate::feed_location::FeedLocation;
use crate::feed_parser::{FeedParser, ParsedFeed};
use crate::retention_window::RetentionWindow;
use crate::source::{ICalSubscriptionSource, LinkProvider, SourceSettings};

/// A calendar subscription link (Meetup's Add to calendar links, a Google secret address, ...). One secret field: the
/// link. It is the credential, so it is stored only in the `CredentialStore`; the connection keeps just the host.
///
/// There is no default transport. The host supplies an [`HttpTransport`] and owes the library these settings, which the
/// Swift `makeDefaultTransport()` and `feedSessionConfiguration` applied (the full contract is in
/// `docs/host-http-contract.md`):
///
/// - **It must not follow redirects.** A 3xx answer must come back as the response: `FeedFetcher` follows redirects
///   itself, hop by hop, and refuses a downgrade from `https`, an address with a user name or password, and more than
///   five hops. A transport that follows them on its own bypasses those checks.
/// - **No cache.** The link is a credential, so nothing about a request may reach disk: no URL cache (it is keyed by the
///   full link and keeps the body), and the request must ignore any local cache.
/// - **No cookies.** Neither sent nor stored.
/// - **Nothing persistent.** The session is ephemeral.
/// - **Timeouts.** Swift set none of its own and ran on the ephemeral session defaults (60 seconds for a request, seven
///   days for the whole resource); a host should set finite ones.
/// - A cancelled request is reported as `Failure::Cancelled`; any other transport failure as
///   `Failure::Source(SourceError::Network(..))`. The fetcher replaces a message that repeats the link.
pub struct ICalSubscriptionKind {
    transport: Arc<dyn HttpTransport>,
    now: Now,
    sleep: Sleeper,
    poll_interval: f64,
    default_zone: Zone,
    retention: Option<RetentionWindow>,
    diagnostics: Arc<dyn DiagnosticLog>,
}

impl ICalSubscriptionKind {
    pub const KIND_ID: &'static str = "icalsub";
    const LINK_KEY: &'static str = "link";

    fn fields() -> Vec<CredentialField> {
        vec![CredentialField::new(Self::LINK_KEY, "iCal link").secret()]
    }

    /// Swift's `init(transport:now:sleep:pollInterval:defaultZone:retention:diagnostics:)`.
    ///
    /// **`transport` must not follow redirects**: it returns a 3xx answer as the response, because the feed fetcher
    /// follows redirects itself, hop by hop, and checks each hop (only `https`, no user name or password, at most five
    /// redirects, validators for the original link only). It must also keep no cache and no cookies and nothing on
    /// disk (the feed link is a credential); see the type's documentation and `docs/host-http-contract.md`. There is
    /// no default transport.
    ///
    /// `now`, `sleep` and `default_zone` are required too, because this library reads no clock, sleeps through the
    /// host's sleeper only and has no "current" zone. The poll interval (900 seconds), the retention window (none) and
    /// the diagnostics (none) keep the Swift defaults; change them with `with_poll_interval`, `with_retention` and
    /// `with_diagnostics`.
    pub fn new(
        transport: Arc<dyn HttpTransport>,
        now: Now,
        sleep: Sleeper,
        default_zone: Zone,
    ) -> ICalSubscriptionKind {
        ICalSubscriptionKind {
            transport,
            now,
            sleep,
            poll_interval: 900.0,
            default_zone,
            retention: None,
            diagnostics: Arc::new(NullDiagnosticLog),
        }
    }

    /// Seconds between checks, and how long a loaded feed is reused.
    pub fn with_poll_interval(mut self, seconds: f64) -> ICalSubscriptionKind {
        self.poll_interval = seconds;
        self
    }

    pub fn with_retention(mut self, retention: RetentionWindow) -> ICalSubscriptionKind {
        self.retention = Some(retention);
        self
    }

    pub fn with_diagnostics(mut self, diagnostics: Arc<dyn DiagnosticLog>) -> ICalSubscriptionKind {
        self.diagnostics = diagnostics;
        self
    }

    pub fn poll_interval(&self) -> f64 {
        self.poll_interval
    }

    /// Nothing is stored here: callers store the link only after the feed has loaded and parsed.
    async fn sign_in(
        &self,
        interaction: &dyn AuthorizationInteraction,
    ) -> Outcome<(UrlString, ParsedFeed)> {
        let values = interaction.prompt_credentials(&Self::fields()).await?;
        let result = self.load(&values).await;
        match &result {
            Ok(_) => self.record(
                DiagnosticLevel::Info,
                "signInAccepted",
                "feedLoaded".to_string(),
            ),
            Err(error) if *error != Failure::Cancelled => self.record(
                DiagnosticLevel::Warning,
                "signInRejected",
                Self::rejection(error).to_string(),
            ),
            Err(_) => {}
        }
        result
    }

    async fn load(&self, values: &BTreeMap<String, String>) -> Outcome<(UrlString, ParsedFeed)> {
        let url = FeedLocation::url_from(values.get(Self::LINK_KEY).map_or("", String::as_str))?;
        let fetcher = FeedFetcher::new(self.transport.clone())
            .with_diagnostics(self.diagnostics.clone(), self.now.clone());
        let FeedResponse::Body(data, _) = fetcher.fetch(&url, None).await? else {
            return Err(Failure::Source(SourceError::InvalidResponse(
                "the feed did not answer".to_string(),
            )));
        };
        let feed =
            FeedParser::parse_logged(&data, None, self.diagnostics.as_ref(), (self.now)(), true)?;
        Ok((url, feed))
    }

    fn record(&self, level: DiagnosticLevel, name: &str, reason: String) {
        self.diagnostics.record(
            level,
            "icalsub",
            name,
            vec![DiagnosticField::public_string("reason", reason)],
            (self.now)(),
        );
    }

    /// A fixed token for why sign-in failed; never the error's message, which a feed server could shape.
    fn rejection(error: &Failure) -> &'static str {
        match error {
            Failure::Source(SourceError::AuthExpired) => "authExpired",
            Failure::Source(SourceError::Network(_)) => "network",
            Failure::Source(SourceError::Server { .. }) => "server",
            Failure::Source(SourceError::InvalidResponse(message)) => {
                if message == FeedLocation::INVALID_MESSAGE || message.starts_with("enter the link")
                {
                    "invalidLink"
                } else {
                    "invalidResponse"
                }
            }
            _ => "other",
        }
    }
}

impl CredentialPromptHelp for ICalSubscriptionKind {
    fn credential_help(&self) -> Option<CredentialHelp> {
        Some(CredentialHelp {
            text: "Paste a calendar subscription link (webcal:// or https://). Do not use a downloaded .ics file: it will not update. \
                   In Meetup, open Your events, choose Add to calendar, and copy any link in the menu."
                .to_string(),
            link_title: Some("Open Meetup Your events".to_string()),
            url: UrlString::parse("https://www.meetup.com/your-events/"),
        })
    }
}

#[async_trait]
impl ConnectorKind for ICalSubscriptionKind {
    fn id(&self) -> String {
        Self::KIND_ID.to_string()
    }

    fn display_name(&self) -> String {
        "iCal link".to_string()
    }

    fn supported_platforms(&self) -> Platform {
        Platform::MACOS | Platform::IOS | Platform::LINUX | Platform::WINDOWS
    }

    fn authorization(&self) -> AuthorizationMethod {
        AuthorizationMethod::Password {
            fields: Self::fields(),
        }
    }

    async fn authorize(
        &self,
        interaction: &dyn AuthorizationInteraction,
        credentials: &dyn CredentialStore,
    ) -> Outcome<Connection> {
        let (url, feed) = self.sign_in(interaction).await?;
        let host = url.host().unwrap_or_default();
        let name = match &feed.name {
            Some(name) => format!("{name} ({host})"),
            None => host.clone(),
        };
        let connection = Connection::new(
            Self::KIND_ID,
            uuid::Uuid::new_v4().to_string().to_uppercase(),
            name,
        )
        .with_config(&[("host", host.as_str())]);
        credentials
            .set_secrets(
                BTreeMap::from([(Self::LINK_KEY.to_string(), url.as_str().to_string())]),
                &connection.connection_id,
            )
            .await?;
        Ok(connection)
    }

    /// Replaces the stored link (after the provider revoked or regenerated it). The old link stays if the new one
    /// fails.
    async fn reauthorize(
        &self,
        connection: &Connection,
        interaction: &dyn AuthorizationInteraction,
        credentials: &dyn CredentialStore,
    ) -> Outcome<Connection> {
        let (url, feed) = self.sign_in(interaction).await?;
        credentials
            .set_secrets(
                BTreeMap::from([(Self::LINK_KEY.to_string(), url.as_str().to_string())]),
                &connection.connection_id,
            )
            .await?;
        let mut updated = connection.clone();
        let host = url.host().unwrap_or_default();
        let old_host = connection.config.get("host").cloned().unwrap_or_default();
        updated.config.insert("host".to_string(), host.clone());
        if host != old_host {
            // Keep the name the feed had (the part before the old host); a link to another service names that service.
            let suffix = format!(" ({old_host})");
            let old_name = connection
                .display_name
                .strip_suffix(&suffix)
                .map(str::to_string);
            updated.display_name = match old_name.or(feed.name) {
                Some(name) => format!("{name} ({host})"),
                None => host,
            };
        }
        Ok(updated)
    }

    fn make_source(
        &self,
        connection: &Connection,
        credentials: Arc<dyn CredentialStore>,
        _sync_state: Arc<dyn SyncStateStore>,
    ) -> Outcome<Arc<dyn CalendarSource>> {
        let connection_id = connection.connection_id.clone();
        let link: LinkProvider = Arc::new(move || {
            let credentials = credentials.clone();
            let connection_id = connection_id.clone();
            Box::pin(async move {
                let secrets = credentials.secrets(&connection_id).await?;
                secrets
                    .and_then(|secrets| secrets.get(Self::LINK_KEY).cloned())
                    .and_then(|text| UrlString::parse(&text))
                    .ok_or(Failure::Source(SourceError::AuthExpired))
            })
        });
        let mut settings = SourceSettings::new(
            connection.clone(),
            link,
            self.transport.clone(),
            ChangeMonitor::new(self.poll_interval, 900.0, self.sleep.clone()),
            self.poll_interval.trunc(),
            self.now.clone(),
            self.default_zone.clone(),
        );
        settings.retention = self.retention;
        settings.diagnostics = self.diagnostics.clone();
        Ok(ICalSubscriptionSource::new(settings))
    }

    fn as_credential_prompt_help(&self) -> Option<&dyn CredentialPromptHelp> {
        Some(self)
    }
}
