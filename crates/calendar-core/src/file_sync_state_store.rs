//! A `SyncStateStore` backed by one JSON file. Port of `FileSyncStateStore.swift`.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::connection::{ConnectionId, SyncStateStore, lock};
use crate::files::write_atomic;

type State = BTreeMap<ConnectionId, BTreeMap<String, String>>;

/// A `SyncStateStore` backed by one JSON file. A missing or unreadable file means "no tokens", so a source does
/// a full bootstrap; write failures leave the in-memory state (and the next launch re-bootstraps).
pub struct FileSyncStateStore {
    path: PathBuf,
    /// `None` until first used (Swift's lazily loaded optional).
    storage: Mutex<Option<State>>,
}

impl FileSyncStateStore {
    /// Creates a store backed by the file at `path`; the file is read when first used.
    pub fn new(path: impl Into<PathBuf>) -> FileSyncStateStore {
        FileSyncStateStore {
            path: path.into(),
            storage: Mutex::new(None),
        }
    }

    /// The loaded state; reads the file on first use and falls back to empty (Swift's `try?` chain).
    fn current<'a>(&self, storage: &'a mut Option<State>) -> &'a mut State {
        storage.get_or_insert_with(|| {
            fs::read(&self.path)
                .ok()
                .and_then(|data| serde_json::from_slice::<State>(&data).ok())
                .unwrap_or_default()
        })
    }

    /// Writes the state; a failure is ignored, as Swift's `try?` does.
    fn commit(&self, all: &State) {
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(data) = serde_json::to_vec(all) {
            let _ = write_atomic(&self.path, &data);
        }
    }
}

#[async_trait]
impl SyncStateStore for FileSyncStateStore {
    async fn token(&self, connection_id: &str, scope: &str) -> Option<String> {
        let mut storage = lock(&self.storage);
        self.current(&mut storage)
            .get(connection_id)
            .and_then(|scopes| scopes.get(scope))
            .cloned()
    }

    async fn set_token(&self, token: Option<&str>, connection_id: &str, scope: &str) {
        let mut storage = lock(&self.storage);
        let all = self.current(&mut storage);
        let mut scopes = all.get(connection_id).cloned().unwrap_or_default();
        match token {
            Some(token) => {
                scopes.insert(scope.to_string(), token.to_string());
            }
            None => {
                scopes.remove(scope);
            }
        }
        if scopes.is_empty() {
            all.remove(connection_id);
        } else {
            all.insert(connection_id.to_string(), scopes);
        }
        self.commit(all);
    }

    async fn remove_all(&self, connection_id: &str) {
        let mut storage = lock(&self.storage);
        let all = self.current(&mut storage);
        all.remove(connection_id);
        self.commit(all);
    }
}
