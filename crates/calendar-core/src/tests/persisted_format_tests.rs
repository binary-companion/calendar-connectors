use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::support::TempDir;
use crate::connection::{Connection, SyncStateStore};
use crate::file_connection_store::{FileConnectionStore, LoadResult};
use crate::file_sync_state_store::FileSyncStateStore;

// Tests/PersistedFormats/<version> holds files that a release's own code wrote: the connection list and the sync
// state a host keeps on users' disks. They are add-only (AGENTS.md, "Backward compatibility"): the current code must
// read every one of them, with the same meaning, for as long as the major version lasts. When phase 3 removes the
// Swift tree they move to `fixtures/persisted-formats/` and this path changes with them.
const PERSISTED_FORMATS: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../Tests/PersistedFormats");

fn fixture(version: &str, name: &str) -> PathBuf {
    Path::new(PERSISTED_FORMATS).join(version).join(name)
}

/// A copy, so a store that writes can never touch the committed file.
fn persisted(version: &str, name: &str, dir: &TempDir) -> PathBuf {
    let copy = dir.path().join("copy").join(name);
    fs::create_dir_all(copy.parent().unwrap()).unwrap();
    fs::copy(fixture(version, name), &copy).unwrap();
    copy
}

fn connections_107() -> Vec<Connection> {
    vec![
        Connection::new(
            "google",
            "0B9C1A7E-6A55-4F0B-9E43-5C1F7D2A9B10",
            "alice@example.com",
        )
        .with_config(&[("email", "alice@example.com")]),
        Connection::new(
            "microsoft",
            "5D2E8F4A-1B3C-4D6E-8F90-A1B2C3D4E5F6",
            "Carol@Contoso.com",
        )
        .with_config(&[("email", "Carol@Contoso.com")]),
        Connection::new(
            "icloud",
            "9A8B7C6D-5E4F-4A3B-2C1D-0E9F8A7B6C5D",
            "alice@icloud.com",
        )
        .with_config(&[
            ("autoSchedule", "true"),
            (
                "homeURL",
                "https://p01-caldav.icloud.com/123456789/calendars/",
            ),
            (
                "principalURL",
                "https://p01-caldav.icloud.com/123456789/principal/",
            ),
            ("serverURL", "https://caldav.icloud.com"),
            (
                "userAddresses",
                "mailto:alice@icloud.com\nmailto:alice@me.com",
            ),
            ("username", "alice@icloud.com"),
        ]),
        Connection::new(
            "caldav",
            "1F2E3D4C-5B6A-4798-8A9B-0C1D2E3F4A5B",
            "bob@caldav.fastmail.com",
        )
        .with_config(&[
            ("autoSchedule", "false"),
            (
                "homeURL",
                "https://caldav.fastmail.com/dav/calendars/user/bob@fastmail.com/",
            ),
            (
                "principalURL",
                "https://caldav.fastmail.com/dav/principals/user/bob@fastmail.com/",
            ),
            ("serverURL", "https://caldav.fastmail.com/"),
            ("userAddresses", "mailto:bob@fastmail.com"),
            ("username", "bob"),
        ]),
        Connection::new(
            "icalsub",
            "C0FFEE00-1234-4567-89AB-CDEF01234567",
            "Team offsites",
        )
        .with_config(&[("host", "calendar.example.org")]),
    ]
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_connection_list_of107_loads() {
    let dir = TempDir::new("persisted-107-list");
    let store = FileConnectionStore::new(persisted("1.0.7", "connections.json", &dir));
    assert_eq!(store.load().await, LoadResult::Loaded(connections_107()));
    assert_eq!(
        connections_107()
            .first()
            .map(Connection::source_id)
            .as_deref(),
        Some("google-0B9C1A7E-6A55-4F0B-9E43-5C1F7D2A9B10")
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_connection_list_of107_stays_editable() {
    let dir = TempDir::new("persisted-107-edit");
    let url = persisted("1.0.7", "connections.json", &dir);
    let store = FileConnectionStore::new(url.clone());
    store
        .remove("C0FFEE00-1234-4567-89AB-CDEF01234567")
        .await
        .unwrap();
    let mut expected = connections_107();
    expected.pop();
    assert_eq!(
        FileConnectionStore::new(url).load().await,
        LoadResult::Loaded(expected)
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_sync_state_of107_loads() {
    let dir = TempDir::new("persisted-107-sync");
    let url = persisted("1.0.7", "sync-state.json", &dir);
    let store = FileSyncStateStore::new(url.clone());
    let google = "0B9C1A7E-6A55-4F0B-9E43-5C1F7D2A9B10";
    let microsoft = "5D2E8F4A-1B3C-4D6E-8F90-A1B2C3D4E5F6";
    let apple = "9A8B7C6D-5E4F-4A3B-2C1D-0E9F8A7B6C5D";
    assert_eq!(
        store.token(google, "_calendars").await.as_deref(),
        Some("alice@example.com\nfamily0123@group.calendar.google.com")
    );
    assert_eq!(
        store.token(google, "alice@example.com").await.as_deref(),
        Some("CPDAlvWDx70CEPDAlvWDx70CGAU=")
    );
    assert_eq!(
        store
            .token(google, "family0123@group.calendar.google.com")
            .await
            .as_deref(),
        Some("CKiZ1P6Dx70CEKiZ1P6Dx70CGAU=")
    );
    assert_eq!(
        store.token(microsoft, "_calendars").await.as_deref(),
        Some("AAMkAGI2TAAA=\nAAMkAGI2TBBB=")
    );
    assert!(
        store
            .token(microsoft, "AAMkAGI2TAAA=")
            .await
            .is_some_and(|t| t.starts_with("1790000000|https://graph.microsoft.com/"))
    );
    assert_eq!(
        store.token(apple, "calendar:home").await.as_deref(),
        Some("sync:https://p01-caldav.icloud.com/sync/30")
    );
    assert_eq!(
        store.token(apple, "calendar:shared-ro").await.as_deref(),
        Some("ctag:FT=-@RU=1")
    );
    // A write keeps everything else that was stored.
    store.remove_all(apple).await;
    let reread = FileSyncStateStore::new(url);
    assert_eq!(
        reread.token(google, "alice@example.com").await.as_deref(),
        Some("CPDAlvWDx70CEPDAlvWDx70CGAU=")
    );
    assert_eq!(reread.token(apple, "calendar:home").await, None);
}

// Release 1.2.0 added the brokered Microsoft kind (CalendarCore only sees a connection with `config` strings; the
// meaning is checked in the Microsoft crate's tests).
fn connections_120() -> Vec<Connection> {
    vec![
        Connection::new(
            "microsoft-brokered",
            "7E3A9C21-4B58-4D0F-A6E2-91C8B5D40F37",
            "dave@contoso.com",
        )
        .with_config(&[
            ("authMode", "external"),
            ("email", "dave@contoso.com"),
            (
                "externalAccountID",
                "00000000-0000-0000-9a1c-3e5f7b9d1c2e.9188040d-6c67-4c5b-b112-36a304b66dad",
            ),
        ]),
    ]
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_connection_list_of120_loads() {
    let dir = TempDir::new("persisted-120-list");
    let store = FileConnectionStore::new(persisted("1.2.0", "connections.json", &dir));
    assert_eq!(store.load().await, LoadResult::Loaded(connections_120()));
    assert_eq!(
        connections_120()[0].source_id(),
        "microsoft-brokered-7E3A9C21-4B58-4D0F-A6E2-91C8B5D40F37"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_sync_state_of120_loads() {
    let dir = TempDir::new("persisted-120-sync");
    let store = FileSyncStateStore::new(persisted("1.2.0", "sync-state.json", &dir));
    let id = "7E3A9C21-4B58-4D0F-A6E2-91C8B5D40F37";
    assert_eq!(
        store.token(id, "_calendars").await.as_deref(),
        Some("AAMkAGI2TCCC=\nAAMkAGI2TDDD=")
    );
    assert!(
        store
            .token(id, "AAMkAGI2TCCC=")
            .await
            .is_some_and(|t| t.starts_with("1800000000|https://graph.microsoft.com/"))
    );
    assert!(
        store
            .token(id, "AAMkAGI2TDDD=")
            .await
            .is_some_and(|t| t.ends_with("$skiptoken=Z4b9"))
    );
}

/// Every version directory under `Tests/PersistedFormats`, sorted, so a fixture added later is covered at once.
fn versions() -> Vec<String> {
    let mut found: Vec<String> = fs::read_dir(PERSISTED_FORMATS)
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

#[test]
fn the_fixture_versions_include_every_release_the_tests_above_name() {
    let found = versions();
    assert!(
        found.contains(&"1.0.7".to_string()) && found.contains(&"1.2.0".to_string()),
        "{found:?}"
    );
}

// Round trip per store over every fixture: load, save to a temporary file, reload, compare values.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_connection_store_round_trips_every_fixture() {
    let mut checked = 0;
    for version in versions() {
        let source = fixture(&version, "connections.json");
        if !source.exists() {
            continue;
        }
        let dir = TempDir::new("round-trip-connections");
        let loaded = match FileConnectionStore::new(persisted(&version, "connections.json", &dir))
            .load()
            .await
        {
            LoadResult::Loaded(list) => list,
            other => panic!("{version}: {other:?}"),
        };
        let target = dir.path().join("saved").join("connections.json");
        let store = FileConnectionStore::new(target.clone());
        store.save(&loaded).await.unwrap();
        assert_eq!(
            FileConnectionStore::new(target.clone()).load().await,
            LoadResult::Loaded(loaded.clone()),
            "{version}"
        );
        // The saved file means the same JSON value as the committed one (spacing and key order aside).
        let saved: Value = serde_json::from_slice(&fs::read(&target).unwrap()).unwrap();
        let original: Value = serde_json::from_slice(&fs::read(&source).unwrap()).unwrap();
        assert_eq!(saved, original, "{version}");
        checked += 1;
    }
    assert!(checked >= 2);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_sync_state_store_round_trips_every_fixture() {
    let mut checked = 0;
    for version in versions() {
        let source = fixture(&version, "sync-state.json");
        if !source.exists() {
            continue;
        }
        let dir = TempDir::new("round-trip-sync");
        let original: BTreeMap<String, BTreeMap<String, String>> =
            serde_json::from_slice(&fs::read(&source).unwrap()).unwrap();
        // Read every entry through the store.
        let from_fixture = FileSyncStateStore::new(persisted(&version, "sync-state.json", &dir));
        // Write every entry into a new file through a second store, then read it back through a third.
        let target = dir.path().join("saved").join("sync-state.json");
        let writer = FileSyncStateStore::new(target.clone());
        for (connection, scopes) in &original {
            for (scope, token) in scopes {
                assert_eq!(
                    from_fixture.token(connection, scope).await.as_ref(),
                    Some(token),
                    "{version}"
                );
                writer.set_token(Some(token), connection, scope).await;
            }
        }
        let reloaded = FileSyncStateStore::new(target.clone());
        for (connection, scopes) in &original {
            for (scope, token) in scopes {
                assert_eq!(
                    reloaded.token(connection, scope).await.as_ref(),
                    Some(token),
                    "{version}"
                );
            }
        }
        let saved: Value = serde_json::from_slice(&fs::read(&target).unwrap()).unwrap();
        let expected: Value = serde_json::from_slice(&fs::read(&source).unwrap()).unwrap();
        assert_eq!(saved, expected, "{version}");
        checked += 1;
    }
    assert!(checked >= 2);
}
