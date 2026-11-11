//! Persists the non-secret `Connection` list as JSON. Port of `FileConnectionStore.swift`.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::Value;

use crate::connection::{Connection, lock};
use crate::files::write_atomic;

/// What the store can fail with. Swift's `FileStoreError` has only `unreadable`; the other Swift errors a save can
/// throw are Foundation's (directory creation, the write), which are `Io` here.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FileStoreError {
    /// The file exists but cannot be read; it is left untouched so the user's data is never overwritten.
    #[error("the file exists but cannot be read")]
    Unreadable,
    /// Creating the directory or writing the file failed.
    #[error("could not write the file: {0}")]
    Io(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadResult {
    Loaded(Vec<Connection>),
    Missing,
    Unreadable,
}

/// Persists the non-secret `Connection` list as JSON. Portable; the host chooses the file location.
///
/// Swift's actor serializes the read-modify-write of `add` and `remove`; a `std` mutex does that here, and no method
/// holds it across an `.await`.
pub struct FileConnectionStore {
    path: PathBuf,
    gate: Mutex<()>,
}

impl FileConnectionStore {
    pub fn new(path: impl Into<PathBuf>) -> FileConnectionStore {
        FileConnectionStore {
            path: path.into(),
            gate: Mutex::new(()),
        }
    }

    pub async fn load(&self) -> LoadResult {
        let _gate = lock(&self.gate);
        self.load_locked()
    }

    /// The stored connections; empty when the file is missing or unreadable (check `load()` to tell them apart).
    pub async fn connections(&self) -> Vec<Connection> {
        match self.load().await {
            LoadResult::Loaded(list) => list,
            LoadResult::Missing | LoadResult::Unreadable => Vec::new(),
        }
    }

    pub async fn save(&self, connections: &[Connection]) -> Result<(), FileStoreError> {
        let _gate = lock(&self.gate);
        self.save_locked(connections)
    }

    /// Adds, or replaces the entry with the same `connection_id`.
    pub async fn add(&self, connection: &Connection) -> Result<(), FileStoreError> {
        let _gate = lock(&self.gate);
        let mut list = self.writable()?;
        match list
            .iter()
            .position(|existing| existing.connection_id == connection.connection_id)
        {
            Some(index) => list[index] = connection.clone(),
            None => list.push(connection.clone()),
        }
        self.save_locked(&list)
    }

    /// Removes a connection; a no-op when it is not stored.
    pub async fn remove(&self, connection_id: &str) -> Result<(), FileStoreError> {
        let _gate = lock(&self.gate);
        let list = self.writable()?;
        if !list.iter().any(|c| c.connection_id == connection_id) {
            return Ok(());
        }
        let kept: Vec<Connection> = list
            .into_iter()
            .filter(|c| c.connection_id != connection_id)
            .collect();
        self.save_locked(&kept)
    }

    fn load_locked(&self) -> LoadResult {
        if !self.path.exists() {
            return LoadResult::Missing;
        }
        let Ok(data) = fs::read(&self.path) else {
            return LoadResult::Unreadable;
        };
        match serde_json::from_slice::<Vec<Connection>>(&data) {
            Ok(list) => LoadResult::Loaded(list),
            Err(_) => LoadResult::Unreadable,
        }
    }

    /// Pretty-printed with sorted keys: the keys sort because `serde_json::Value` objects are ordered maps.
    fn save_locked(&self, connections: &[Connection]) -> Result<(), FileStoreError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| FileStoreError::Io(e.to_string()))?;
        }
        let value: Value =
            serde_json::to_value(connections).map_err(|e| FileStoreError::Io(e.to_string()))?;
        let data =
            serde_json::to_vec_pretty(&value).map_err(|e| FileStoreError::Io(e.to_string()))?;
        write_atomic(&self.path, &data).map_err(|e| FileStoreError::Io(e.to_string()))
    }

    fn writable(&self) -> Result<Vec<Connection>, FileStoreError> {
        match self.load_locked() {
            LoadResult::Loaded(list) => Ok(list),
            LoadResult::Missing => Ok(Vec::new()),
            LoadResult::Unreadable => Err(FileStoreError::Unreadable),
        }
    }
}
