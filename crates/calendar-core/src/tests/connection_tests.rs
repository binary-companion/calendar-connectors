use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;

use crate::connection::{
    AuthorizationInteraction, AuthorizationMethod, Connection, ConnectorKind, ConnectorRegistry,
    CredentialStore, InMemoryCredentialStore, InMemorySyncStateStore, Platform, SyncStateStore,
};
use crate::failure::{Failure, Outcome};
use crate::source_types::{CalendarSource, SourceError};

use super::support::secrets;

#[test]
fn connection_round_trips_and_derives_source_id() {
    let c = Connection::new("google", "abc", "me@x.com").with_config(&[("email", "me@x.com")]);
    let data = serde_json::to_vec(&c).unwrap();
    assert_eq!(serde_json::from_slice::<Connection>(&data).unwrap(), c);
    assert_eq!(c.source_id(), "google-abc");
    assert_eq!(c.id(), "abc");
}

#[test]
fn connection_decodes_with_unknown_keys() {
    let json = r#"{"kindID":"google","connectionID":"a","displayName":"d","config":{},"future":1}"#;
    let c: Connection = serde_json::from_str(json).unwrap();
    assert_eq!(c.kind_id, "google");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn in_memory_credential_store_stores_and_removes() {
    let store = InMemoryCredentialStore::new();
    assert_eq!(store.secrets("c1").await.unwrap(), None);
    store
        .set_secrets(secrets(&[("refresh_token", "r")]), "c1")
        .await
        .unwrap();
    assert_eq!(
        store.secrets("c1").await.unwrap(),
        Some(secrets(&[("refresh_token", "r")]))
    );
    store.remove_secrets("c1").await.unwrap();
    assert_eq!(store.secrets("c1").await.unwrap(), None);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn in_memory_sync_state_scopes_tokens_and_removes_all() {
    let store = InMemorySyncStateStore::new();
    store.set_token(Some("t1"), "c1", "calA").await;
    store.set_token(Some("t2"), "c1", "calB").await;
    store.set_token(Some("t3"), "c2", "calA").await;
    assert_eq!(store.token("c1", "calA").await.as_deref(), Some("t1"));
    store.set_token(None, "c1", "calA").await;
    assert_eq!(store.token("c1", "calA").await, None);
    store.remove_all("c1").await;
    assert_eq!(store.token("c1", "calB").await, None);
    assert_eq!(store.token("c2", "calA").await.as_deref(), Some("t3"));
}

// The revision rule of `InMemoryCredentialStore` (`CredentialStore` doc comment): a refresh token is rotated only
// by the credential lifecycle that read the snapshot, and the revision survives the rotation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn update_refresh_token_requires_the_current_revision() {
    let store = InMemoryCredentialStore::new();
    assert!(
        !store
            .update_refresh_token("x", "c1", uuid::Uuid::new_v4())
            .await
            .unwrap()
    );
    store
        .set_secrets(secrets(&[("refresh_token", "r1"), ("other", "o")]), "c1")
        .await
        .unwrap();
    let snapshot = store.credential_snapshot("c1").await.unwrap().unwrap();
    assert!(
        !store
            .update_refresh_token("r2", "c1", uuid::Uuid::new_v4())
            .await
            .unwrap()
    );
    assert_eq!(
        store.secrets("c1").await.unwrap(),
        Some(secrets(&[("refresh_token", "r1"), ("other", "o")]))
    );
    assert!(
        store
            .update_refresh_token("r2", "c1", snapshot.revision)
            .await
            .unwrap()
    );
    let after = store.credential_snapshot("c1").await.unwrap().unwrap();
    assert_eq!(after.revision, snapshot.revision);
    assert_eq!(
        after.secrets,
        secrets(&[("refresh_token", "r2"), ("other", "o")])
    );
    // Setting secrets again starts a new lifecycle: the old revision no longer matches.
    store
        .set_secrets(secrets(&[("refresh_token", "r3")]), "c1")
        .await
        .unwrap();
    assert!(
        !store
            .update_refresh_token("r4", "c1", snapshot.revision)
            .await
            .unwrap()
    );
}

#[test]
fn platform_is_an_option_set() {
    assert!(Platform::MACOS.is_disjoint(Platform::LINUX));
    let both = Platform::MACOS | Platform::LINUX;
    assert!(!both.is_disjoint(Platform::LINUX));
    assert!(both.contains(Platform::MACOS) && !both.contains(Platform::WINDOWS));
    assert_eq!(Platform::MACOS.raw_value(), 1);
    assert_eq!(Platform::IOS.raw_value(), 2);
    assert_eq!(Platform::LINUX.raw_value(), 4);
    assert_eq!(Platform::WINDOWS.raw_value(), 8);
    assert!(Platform::empty().is_disjoint(both));
}

struct StubKind {
    id: String,
    supported_platforms: Platform,
}

impl StubKind {
    fn new(id: &str, supported_platforms: Platform) -> Arc<StubKind> {
        Arc::new(StubKind {
            id: id.to_string(),
            supported_platforms,
        })
    }
}

#[async_trait]
impl ConnectorKind for StubKind {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn display_name(&self) -> String {
        self.id.clone()
    }
    fn supported_platforms(&self) -> Platform {
        self.supported_platforms
    }
    fn authorization(&self) -> AuthorizationMethod {
        AuthorizationMethod::Oauth
    }
    async fn authorize(
        &self,
        _interaction: &dyn AuthorizationInteraction,
        _credentials: &dyn CredentialStore,
    ) -> Outcome<Connection> {
        Ok(Connection::new(self.id.clone(), "x", "x"))
    }
    async fn reauthorize(
        &self,
        connection: &Connection,
        _interaction: &dyn AuthorizationInteraction,
        _credentials: &dyn CredentialStore,
    ) -> Outcome<Connection> {
        Ok(connection.clone())
    }
    fn make_source(
        &self,
        _connection: &Connection,
        _credentials: Arc<dyn CredentialStore>,
        _sync_state: Arc<dyn SyncStateStore>,
    ) -> Outcome<Arc<dyn CalendarSource>> {
        Err(Failure::Source(SourceError::InvalidResponse(
            "stub".to_string(),
        )))
    }
}

#[test]
fn registry_filters_by_platform() {
    let mut registry = ConnectorRegistry::new();
    registry.register(StubKind::new("google", Platform::MACOS | Platform::LINUX));
    registry.register(StubKind::new("eventkit", Platform::MACOS));
    assert_eq!(
        registry.kind("google").map(|k| k.id()).as_deref(),
        Some("google")
    );
    assert!(registry.kind("missing").is_none());
    let mac: BTreeSet<String> = registry
        .kinds_for(Platform::MACOS)
        .iter()
        .map(|k| k.id())
        .collect();
    assert_eq!(
        mac,
        BTreeSet::from(["google".to_string(), "eventkit".to_string()])
    );
    let linux: Vec<String> = registry
        .kinds_for(Platform::LINUX)
        .iter()
        .map(|k| k.id())
        .collect();
    assert_eq!(linux, vec!["google".to_string()]);
    // Sorted by id; a later registration under the same id replaces the earlier one.
    let sorted: Vec<String> = registry
        .kinds_for(Platform::MACOS)
        .iter()
        .map(|k| k.id())
        .collect();
    assert_eq!(sorted, vec!["eventkit".to_string(), "google".to_string()]);
    registry.register(StubKind::new("google", Platform::WINDOWS));
    assert!(registry.kinds_for(Platform::LINUX).is_empty());
}
