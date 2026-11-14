use std::fs;
use std::path::PathBuf;

use super::support::TempDir;
use crate::connection::{Connection, SyncStateStore};
use crate::file_connection_store::{FileConnectionStore, FileStoreError, LoadResult};
use crate::file_sync_state_store::FileSyncStateStore;

/// A path in a directory that does not exist yet, as Swift's `tempURL`: `store.json` under a fresh UUID folder.
fn temp_url(dir: &TempDir, name: &str) -> PathBuf {
    dir.path().join("fresh").join(name)
}

fn alice() -> Connection {
    Connection::new("google", "c1", "alice@example.com")
        .with_config(&[("email", "alice@example.com")])
}

fn bob() -> Connection {
    Connection::new("google", "c2", "bob@example.com")
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn connection_store_missing_file_is_missing() {
    let dir = TempDir::new("store-missing");
    assert_eq!(
        FileConnectionStore::new(temp_url(&dir, "store.json"))
            .load()
            .await,
        LoadResult::Missing
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn connection_store_round_trips_add_replace_remove() {
    let dir = TempDir::new("store-add");
    let store = FileConnectionStore::new(temp_url(&dir, "store.json"));
    store.add(&alice()).await.unwrap();
    store.add(&bob()).await.unwrap();
    assert_eq!(store.connections().await, vec![alice(), bob()]);
    let mut renamed = alice();
    renamed.display_name = "alice2@example.com".to_string();
    store.add(&renamed).await.unwrap();
    assert_eq!(store.connections().await, vec![renamed.clone(), bob()]);
    store.remove("c1").await.unwrap();
    assert_eq!(store.load().await, LoadResult::Loaded(vec![bob()]));
    store.remove("c1").await.unwrap(); // idempotent
    assert_eq!(store.connections().await, vec![bob()]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn connection_store_unreadable_file_is_reported_and_never_overwritten() {
    let dir = TempDir::new("store-unreadable");
    let url = temp_url(&dir, "store.json");
    fs::create_dir_all(url.parent().unwrap()).unwrap();
    fs::write(&url, b"not json").unwrap();
    let store = FileConnectionStore::new(url.clone());
    assert_eq!(store.load().await, LoadResult::Unreadable);
    assert_eq!(store.add(&alice()).await, Err(FileStoreError::Unreadable));
    assert_eq!(store.remove("c1").await, Err(FileStoreError::Unreadable));
    assert_eq!(fs::read(&url).unwrap(), b"not json");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn sync_state_round_trips_across_instances_and_remove_all() {
    let dir = TempDir::new("sync-round-trip");
    let url = temp_url(&dir, "sync.json");
    let a = FileSyncStateStore::new(url.clone());
    a.set_token(Some("t1"), "c1", "cal-a").await;
    a.set_token(Some("t2"), "c1", "cal-b").await;
    a.set_token(Some("t3"), "c2", "cal-a").await;
    let b = FileSyncStateStore::new(url.clone());
    assert_eq!(b.token("c1", "cal-a").await.as_deref(), Some("t1"));
    b.set_token(None, "c1", "cal-a").await;
    assert_eq!(b.token("c1", "cal-a").await, None);
    b.remove_all("c1").await;
    assert_eq!(
        FileSyncStateStore::new(url.clone())
            .token("c1", "cal-b")
            .await,
        None
    );
    assert_eq!(
        FileSyncStateStore::new(url)
            .token("c2", "cal-a")
            .await
            .as_deref(),
        Some("t3")
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn sync_state_unreadable_file_means_no_tokens() {
    let dir = TempDir::new("sync-unreadable");
    let url = temp_url(&dir, "sync.json");
    fs::create_dir_all(url.parent().unwrap()).unwrap();
    fs::write(&url, b"###").unwrap();
    let store = FileSyncStateStore::new(url.clone());
    assert_eq!(store.token("c1", "x").await, None);
    store.set_token(Some("t"), "c1", "x").await;
    assert_eq!(
        FileSyncStateStore::new(url)
            .token("c1", "x")
            .await
            .as_deref(),
        Some("t")
    );
}

// Swift's `try?` on the write: a failure leaves the in-memory state and nothing else (the next launch re-bootstraps).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn sync_state_ignores_write_failures_and_keeps_memory() {
    let dir = TempDir::new("sync-write-failure");
    // The parent "directory" is a file, so neither creating it nor writing under it can work.
    let blocker = dir.path().join("blocker");
    fs::write(&blocker, b"x").unwrap();
    let url = blocker.join("sync.json");
    let store = FileSyncStateStore::new(url.clone());
    store.set_token(Some("t"), "c1", "x").await;
    assert_eq!(store.token("c1", "x").await.as_deref(), Some("t"));
    assert!(!url.exists());
    assert_eq!(FileSyncStateStore::new(url).token("c1", "x").await, None);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn connection_store_saves_pretty_printed_sorted_keys() {
    let dir = TempDir::new("store-format");
    let url = temp_url(&dir, "store.json");
    let store = FileConnectionStore::new(url.clone());
    store.save(&[alice()]).await.unwrap();
    let text = fs::read_to_string(&url).unwrap();
    let positions: Vec<usize> = [
        "\"config\"",
        "\"connectionID\"",
        "\"displayName\"",
        "\"kindID\"",
    ]
    .iter()
    .map(|key| text.find(key).unwrap())
    .collect();
    assert!(positions.windows(2).all(|w| w[0] < w[1]), "{text}");
    assert!(text.contains("\n  "), "pretty printed: {text}");
    // No temporary file is left behind.
    assert_eq!(fs::read_dir(url.parent().unwrap()).unwrap().count(), 1);
}
