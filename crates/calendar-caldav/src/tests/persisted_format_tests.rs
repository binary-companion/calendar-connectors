//! Port of `PersistedFormatTests.swift` (CalDAVCalendarTests): the CalDAV parts of `Tests/PersistedFormats`, the
//! account settings in `Connection.config` and the list signature and per-calendar markers in the sync state, as
//! 1.0.7 wrote them.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use calendar_core::connection::{
    Connection, ConnectorKind, InMemoryCredentialStore, InMemorySyncStateStore,
};
use calendar_core::model::{CalendarDescriptor, CalendarPermissions, CalendarService};
use calendar_core::time::Zone;
use calendar_core::url_string::UrlString;

use super::support::{FakeCalDavServer, harness_default_zone};
use crate::account_config::CalDavAccountConfig;
use crate::connector_kinds::{CalDavConnectorKind, ICloudConnectorKind};
use crate::source::{CalDavCalendarInfo, CalDavCalendarSource};

const PERSISTED_FORMATS: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../Tests/PersistedFormats");

fn persisted(version: &str, name: &str) -> Vec<u8> {
    std::fs::read(Path::new(PERSISTED_FORMATS).join(version).join(name)).unwrap()
}

fn connection_107(kind_id: &str) -> Connection {
    let connections: Vec<Connection> =
        serde_json::from_slice(&persisted("1.0.7", "connections.json")).unwrap();
    connections
        .into_iter()
        .find(|c| c.kind_id == kind_id)
        .unwrap()
}

fn url(text: &str) -> UrlString {
    UrlString::parse(text).unwrap()
}

#[test]
fn the_caldav_accounts_of_107_still_open() {
    let icloud = connection_107("icloud");
    assert_eq!(
        CalDavAccountConfig::from_config(&icloud.config).unwrap(),
        CalDavAccountConfig::new(
            url("https://caldav.icloud.com"),
            "alice@icloud.com",
            url("https://p01-caldav.icloud.com/123456789/principal/"),
            url("https://p01-caldav.icloud.com/123456789/calendars/"),
            vec![
                "mailto:alice@icloud.com".to_string(),
                "mailto:alice@me.com".to_string()
            ],
            true
        )
    );
    let other = connection_107("caldav");
    assert_eq!(
        CalDavAccountConfig::from_config(&other.config).unwrap(),
        CalDavAccountConfig::new(
            url("https://caldav.fastmail.com/"),
            "bob",
            url("https://caldav.fastmail.com/dav/principals/user/bob@fastmail.com/"),
            url("https://caldav.fastmail.com/dav/calendars/user/bob@fastmail.com/"),
            vec!["mailto:bob@fastmail.com".to_string()],
            false
        )
    );
    let sync = Arc::new(InMemorySyncStateStore::new());
    let store = Arc::new(InMemoryCredentialStore::new());
    let icloud_kind = ICloudConnectorKind::new(
        Arc::new(FakeCalDavServer::new()),
        Arc::new(|| jiff::Timestamp::UNIX_EPOCH),
        super::support::instant_sleeper(),
        harness_default_zone(),
    );
    assert_eq!(
        icloud_kind
            .make_source(&icloud, store.clone(), sync.clone())
            .unwrap()
            .id(),
        "icloud-9A8B7C6D-5E4F-4A3B-2C1D-0E9F8A7B6C5D"
    );
    let caldav_kind = CalDavConnectorKind::new(
        Arc::new(FakeCalDavServer::new()),
        Arc::new(|| jiff::Timestamp::UNIX_EPOCH),
        super::support::instant_sleeper(),
        harness_default_zone(),
    );
    assert_eq!(
        caldav_kind.make_source(&other, store, sync).unwrap().id(),
        "caldav-1F2E3D4C-5B6A-4798-8A9B-0C1D2E3F4A5B"
    );
}

fn info(
    descriptor: CalendarDescriptor,
    path: &str,
    ctag: &str,
    sync_token: Option<&str>,
    zone: &Zone,
) -> CalDavCalendarInfo {
    CalDavCalendarInfo {
        descriptor,
        url: url(path),
        ctag: Some(ctag.to_string()),
        sync_token: sync_token.map(str::to_string),
        zone: zone.clone(),
    }
}

/// The same calendars give the same signature and markers as 1.0.7 stored, so an update reports no change by itself.
#[test]
fn the_caldav_sync_state_of_107_matches_the_same_calendars() {
    let all: BTreeMap<String, BTreeMap<String, String>> =
        serde_json::from_slice(&persisted("1.0.7", "sync-state.json")).unwrap();
    let stored = &all["9A8B7C6D-5E4F-4A3B-2C1D-0E9F8A7B6C5D"];
    let zone = Zone::named("Europe/Berlin").unwrap();
    let mut home = CalendarDescriptor::new("home", "Home", CalendarService::cal_dav())
        .with_color_hex(Some("#1BADF8"));
    home.permissions = CalendarPermissions {
        can_view_details: true,
        can_edit: true,
        ..CalendarPermissions::default()
    };
    home.is_default = Some(true);
    home.time_zone = Some(zone.clone());
    let mut shared = CalendarDescriptor::new(
        "shared-ro",
        "Shared (read only)",
        CalendarService::cal_dav(),
    );
    shared.permissions = CalendarPermissions {
        can_view_details: true,
        can_edit: false,
        ..CalendarPermissions::default()
    };
    shared.is_default = Some(false);
    shared.time_zone = Some(zone.clone());
    let infos = vec![
        info(
            home,
            "https://p01-caldav.icloud.com/123456789/calendars/home/",
            "HwoQEgwAAA",
            Some("https://p01-caldav.icloud.com/sync/30"),
            &zone,
        ),
        info(
            shared,
            "https://p01-caldav.icloud.com/123456789/calendars/shared-ro/",
            "FT=-@RU=1",
            None,
            &zone,
        ),
    ];
    assert_eq!(
        stored[CalDavCalendarSource::CALENDARS_SCOPE],
        CalDavCalendarSource::signature(&infos)
    );
    for info in &infos {
        assert_eq!(
            stored[&CalDavCalendarSource::calendar_scope(&info.descriptor.id)],
            CalDavCalendarSource::marker(info)
        );
    }
}
