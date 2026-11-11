//! Connections, credential and sync-state stores, the connector-kind seam and the registry.
//! Port of `Connection.swift`.

use std::collections::BTreeMap;
use std::ops::BitOr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::external_tokens::AccountIdentityFamily;
use crate::failure::Outcome;
use crate::source_types::CalendarSource;
use crate::url_string::UrlString;

/// Swift's `ConnectionID` (`typealias ConnectionID = String`).
pub type ConnectionId = String;

/// Locks a `std` mutex and carries on with the data of a poisoned one: no code in this crate leaves the data it
/// guards half changed across a panic point.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Non-secret description of a connected account. The host app persists it (JSON, UserDefaults, ...).
///
/// The JSON keys are the Swift property names (`kindID`, `connectionID`, `displayName`, `config`); unknown keys are
/// ignored and `config` is required, as Swift's synthesized `Codable` has it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Connection {
    #[serde(rename = "kindID")]
    pub kind_id: String,
    #[serde(rename = "connectionID")]
    pub connection_id: ConnectionId,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub config: BTreeMap<String, String>,
}

impl Connection {
    /// Swift's memberwise `init` with `config` left at its default (empty); see [`Connection::with_config`].
    pub fn new(
        kind_id: impl Into<String>,
        connection_id: impl Into<String>,
        display_name: impl Into<String>,
    ) -> Connection {
        Connection {
            kind_id: kind_id.into(),
            connection_id: connection_id.into(),
            display_name: display_name.into(),
            config: BTreeMap::new(),
        }
    }

    /// Sets `config` from key/value pairs.
    #[must_use]
    pub fn with_config(mut self, pairs: &[(&str, &str)]) -> Connection {
        self.config = pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect();
        self
    }

    /// Swift's `Identifiable.id`.
    pub fn id(&self) -> &str {
        &self.connection_id
    }

    /// The `CalendarSource.id` for this connection. Every consumer must use this helper.
    pub fn source_id(&self) -> String {
        format!("{}-{}", self.kind_id, self.connection_id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CredentialField {
    pub key: String,
    pub label: String,
    pub is_secret: bool,
}

impl CredentialField {
    /// `isSecret` defaults to false in Swift.
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> CredentialField {
        CredentialField {
            key: key.into(),
            label: label.into(),
            is_secret: false,
        }
    }

    #[must_use]
    pub fn secret(mut self) -> CredentialField {
        self.is_secret = true;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AuthorizationMethod {
    Oauth,
    Password { fields: Vec<CredentialField> },
    System,
}

/// A short explanation a host shows next to a credential prompt (for example that iCloud needs an app-specific
/// password), with an optional link.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CredentialHelp {
    pub text: String,
    pub link_title: Option<String>,
    pub url: Option<UrlString>,
}

impl CredentialHelp {
    pub fn new(text: impl Into<String>) -> CredentialHelp {
        CredentialHelp {
            text: text.into(),
            link_title: None,
            url: None,
        }
    }
}

/// Adopted by a `.password` connector kind that has help to show with its fields. Optional: hosts check with
/// [`ConnectorKind::as_credential_prompt_help`] (Swift `as?`).
pub trait CredentialPromptHelp {
    fn credential_help(&self) -> Option<CredentialHelp>;
}

/// Swift's `OptionSet`: a set of platforms as bit flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Platform(u8);

impl Platform {
    pub const MACOS: Platform = Platform(1 << 0);
    pub const IOS: Platform = Platform(1 << 1);
    pub const LINUX: Platform = Platform(1 << 2);
    pub const WINDOWS: Platform = Platform(1 << 3);

    /// `init(rawValue:)`.
    pub fn from_raw_value(raw: u8) -> Platform {
        Platform(raw)
    }

    pub fn raw_value(self) -> u8 {
        self.0
    }

    /// The empty set (`[]`).
    pub fn empty() -> Platform {
        Platform(0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// `contains(_:)`: every member of `other` is in `self`.
    pub fn contains(self, other: Platform) -> bool {
        self.0 & other.0 == other.0
    }

    /// `isDisjoint(with:)`.
    pub fn is_disjoint(self, other: Platform) -> bool {
        self.0 & other.0 == 0
    }

    /// The platform this build runs on, from the compile-time target (no environment is read).
    pub fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::MACOS
        } else if cfg!(target_os = "ios") {
            Platform::IOS
        } else if cfg!(target_os = "linux") {
            Platform::LINUX
        } else if cfg!(target_os = "windows") {
            Platform::WINDOWS
        } else {
            Platform::empty()
        }
    }
}

impl BitOr for Platform {
    type Output = Platform;
    fn bitor(self, rhs: Platform) -> Platform {
        Platform(self.0 | rhs.0)
    }
}

/// A keyed map of secrets per connection (Google stores `["refresh_token": ...]`).
/// The host supplies the real store (TimeTug: Keychain). The library ships only an in-memory one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialSnapshot {
    pub secrets: BTreeMap<String, String>,
    pub revision: Uuid,
}

impl CredentialSnapshot {
    pub fn new(secrets: BTreeMap<String, String>, revision: Uuid) -> CredentialSnapshot {
        CredentialSnapshot { secrets, revision }
    }
}

#[async_trait]
pub trait CredentialStore: Send + Sync {
    async fn secrets(&self, connection_id: &str) -> Outcome<Option<BTreeMap<String, String>>>;
    async fn set_secrets(
        &self,
        secrets: BTreeMap<String, String>,
        connection_id: &str,
    ) -> Outcome<()>;
    async fn remove_secrets(&self, connection_id: &str) -> Outcome<()>;
    async fn credential_snapshot(&self, connection_id: &str)
    -> Outcome<Option<CredentialSnapshot>>;
    /// Only the same credential lifecycle may rotate its refresh token. The store must check and write atomically.
    async fn update_refresh_token(
        &self,
        token: &str,
        connection_id: &str,
        expected_revision: Uuid,
    ) -> Outcome<bool>;
}

#[async_trait]
pub trait SyncStateStore: Send + Sync {
    async fn token(&self, connection_id: &str, scope: &str) -> Option<String>;
    async fn set_token(&self, token: Option<&str>, connection_id: &str, scope: &str);
    async fn remove_all(&self, connection_id: &str);
}

/// Swift's `actor InMemoryCredentialStore`: a `std` mutex guards the map, and no method holds it across an `.await`.
#[derive(Default)]
pub struct InMemoryCredentialStore {
    storage: Mutex<BTreeMap<ConnectionId, CredentialSnapshot>>,
}

impl InMemoryCredentialStore {
    pub fn new() -> InMemoryCredentialStore {
        InMemoryCredentialStore::default()
    }

    pub fn is_empty(&self) -> bool {
        lock(&self.storage).is_empty()
    }
}

#[async_trait]
impl CredentialStore for InMemoryCredentialStore {
    async fn secrets(&self, connection_id: &str) -> Outcome<Option<BTreeMap<String, String>>> {
        Ok(lock(&self.storage)
            .get(connection_id)
            .map(|snapshot| snapshot.secrets.clone()))
    }

    async fn set_secrets(
        &self,
        secrets: BTreeMap<String, String>,
        connection_id: &str,
    ) -> Outcome<()> {
        lock(&self.storage).insert(
            connection_id.to_string(),
            CredentialSnapshot::new(secrets, Uuid::new_v4()),
        );
        Ok(())
    }

    async fn remove_secrets(&self, connection_id: &str) -> Outcome<()> {
        lock(&self.storage).remove(connection_id);
        Ok(())
    }

    async fn credential_snapshot(
        &self,
        connection_id: &str,
    ) -> Outcome<Option<CredentialSnapshot>> {
        Ok(lock(&self.storage).get(connection_id).cloned())
    }

    async fn update_refresh_token(
        &self,
        token: &str,
        connection_id: &str,
        expected_revision: Uuid,
    ) -> Outcome<bool> {
        let mut storage = lock(&self.storage);
        let Some(existing) = storage.get(connection_id) else {
            return Ok(false);
        };
        if existing.revision != expected_revision {
            return Ok(false);
        }
        let mut secrets = existing.secrets.clone();
        secrets.insert("refresh_token".to_string(), token.to_string());
        let revision = existing.revision;
        storage.insert(
            connection_id.to_string(),
            CredentialSnapshot::new(secrets, revision),
        );
        Ok(true)
    }
}

/// Swift's `actor InMemorySyncStateStore`.
#[derive(Default)]
pub struct InMemorySyncStateStore {
    storage: Mutex<BTreeMap<ConnectionId, BTreeMap<String, String>>>,
}

impl InMemorySyncStateStore {
    pub fn new() -> InMemorySyncStateStore {
        InMemorySyncStateStore::default()
    }
}

#[async_trait]
impl SyncStateStore for InMemorySyncStateStore {
    async fn token(&self, connection_id: &str, scope: &str) -> Option<String> {
        lock(&self.storage)
            .get(connection_id)
            .and_then(|scopes| scopes.get(scope))
            .cloned()
    }

    async fn set_token(&self, token: Option<&str>, connection_id: &str, scope: &str) {
        let mut storage = lock(&self.storage);
        let scopes = storage.entry(connection_id.to_string()).or_default();
        match token {
            Some(token) => {
                scopes.insert(scope.to_string(), token.to_string());
            }
            None => {
                scopes.remove(scope);
            }
        }
    }

    async fn remove_all(&self, connection_id: &str) {
        lock(&self.storage).remove(connection_id);
    }
}

/// One in-progress OAuth redirect (e.g. a loopback listener) owned by the host app.
#[async_trait]
pub trait OAuthRedirectSession: Send + Sync {
    /// The exact redirect URI to put in the authorization URL and the token exchange, e.g. `http://127.0.0.1:53211`.
    fn redirect_uri(&self) -> UrlString;
    /// Opens `authorization_url` for the user, waits for the redirect and returns the redirect URL received.
    async fn authorize(&self, authorization_url: &UrlString) -> Outcome<UrlString>;
    async fn close(&self);
}

/// Implemented by the host app: the only OS/UI-specific part of authorization.
#[async_trait]
pub trait AuthorizationInteraction: Send + Sync {
    async fn begin_oauth_redirect(&self) -> Outcome<Box<dyn OAuthRedirectSession>>;
    async fn prompt_credentials(
        &self,
        fields: &[CredentialField],
    ) -> Outcome<BTreeMap<String, String>>;
}

#[async_trait]
pub trait ConnectorKind: Send + Sync {
    fn id(&self) -> String;
    fn display_name(&self) -> String;
    fn supported_platforms(&self) -> Platform;
    fn authorization(&self) -> AuthorizationMethod;
    async fn authorize(
        &self,
        interaction: &dyn AuthorizationInteraction,
        credentials: &dyn CredentialStore,
    ) -> Outcome<Connection>;
    /// Re-runs sign-in for an existing connection (after `AuthExpired`). Keeps its `connection_id` (so `source_id`,
    /// calendar keys and sync state stay valid), replaces the secrets, and fails with
    /// `SourceError::InvalidResponse` if the user signs in as a different account.
    async fn reauthorize(
        &self,
        connection: &Connection,
        interaction: &dyn AuthorizationInteraction,
        credentials: &dyn CredentialStore,
    ) -> Outcome<Connection>;
    fn make_source(
        &self,
        connection: &Connection,
        credentials: Arc<dyn CredentialStore>,
        sync_state: Arc<dyn SyncStateStore>,
    ) -> Outcome<Arc<dyn CalendarSource>>;
    /// Swift's `as? AccountIdentityFamily`: `Some` when the kind belongs to an identity family. A kind without it is
    /// its own family (its kind id).
    fn as_identity_family(&self) -> Option<&dyn AccountIdentityFamily> {
        None
    }
    /// Swift's `as? CredentialPromptHelp`: `Some` when a `.password` kind has help to show with its fields.
    fn as_credential_prompt_help(&self) -> Option<&dyn CredentialPromptHelp> {
        None
    }
}

/// Swift's `struct ConnectorRegistry`; cloning shares the registered kinds.
#[derive(Clone, Default)]
pub struct ConnectorRegistry {
    kinds: BTreeMap<String, Arc<dyn ConnectorKind>>,
}

impl ConnectorRegistry {
    pub fn new() -> ConnectorRegistry {
        ConnectorRegistry::default()
    }

    /// Registers `kind` under its id, replacing an earlier kind with the same id.
    pub fn register(&mut self, kind: Arc<dyn ConnectorKind>) {
        self.kinds.insert(kind.id(), kind);
    }

    pub fn kind(&self, id: &str) -> Option<Arc<dyn ConnectorKind>> {
        self.kinds.get(id).cloned()
    }

    /// Kinds usable on `platform`, sorted by id.
    pub fn kinds_for(&self, platform: Platform) -> Vec<Arc<dyn ConnectorKind>> {
        self.kinds
            .values()
            .filter(|kind| !kind.supported_platforms().is_disjoint(platform))
            .cloned()
            .collect()
    }
}
