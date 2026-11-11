//! Port of `DiscoveryTests.swift`.

use std::collections::BTreeMap;
use std::sync::Arc;

use calendar_core::failure::Failure;
use calendar_core::http::{HttpResponse, HttpTransport};
use calendar_core::source_types::SourceError;
use calendar_test_support::fake_transport::FakeTransport;

use super::support::{FakeCalDavServer, fake_home_url, fake_server_url};
use crate::account_config::CalDavAccountConfig;
use crate::discovery::CalDavDiscovery;
use crate::webdav_client::{CredentialProvider, WebDavClient, WebDavCredentials};
use calendar_core::url_string::UrlString;

fn discovery_with(
    transport: Arc<dyn HttpTransport>,
    password: &str,
    base: &str,
) -> CalDavDiscovery {
    let password = password.to_string();
    let credentials: CredentialProvider = Arc::new(move || {
        let password = password.clone();
        Box::pin(async move {
            Ok(WebDavCredentials {
                username: "me@icloud.test".to_string(),
                password,
            })
        })
    });
    CalDavDiscovery {
        client: WebDavClient::new(transport, base, credentials),
    }
}

fn discovery(transport: Arc<dyn HttpTransport>) -> CalDavDiscovery {
    discovery_with(transport, "app-pass-1234", "icloud.com")
}

#[tokio::test]
async fn discovers_an_icloud_shaped_account() {
    let server = Arc::new(FakeCalDavServer::new());
    let account = discovery(server.clone())
        .discover(&fake_server_url(), "me@icloud.test")
        .await
        .unwrap();
    assert_eq!(
        account.principal_url.as_str(),
        "https://caldav.icloud.com/123/principal/"
    );
    assert_eq!(account.home_url, fake_home_url());
    assert_eq!(
        account.user_addresses,
        [
            "mailto:me@icloud.test",
            "urn:uuid:11111111-2222-3333-4444-555555555555"
        ]
    );
    assert!(account.auto_schedule);
    assert_eq!(account.username, "me@icloud.test");
    assert_eq!(account.server_url, fake_server_url());
    let methods: Vec<String> = server.log().into_iter().map(|r| r.method).collect();
    assert_eq!(methods, ["PROPFIND", "PROPFIND", "PROPFIND", "OPTIONS"]);
}

#[tokio::test]
async fn follows_a_partition_host_redirect() {
    let server = Arc::new(FakeCalDavServer::new());
    server
        .configure(|s| s.well_known_location = Some("https://p42-caldav.icloud.com/".to_string()));
    let account = discovery(server)
        .discover(&fake_server_url(), "me@icloud.test")
        .await
        .unwrap();
    assert_eq!(
        account.principal_url.host().as_deref(),
        Some("p42-caldav.icloud.com")
    );
    assert_eq!(
        account.home_url.as_str(),
        "https://p42-caldav.icloud.com/123/calendars/"
    );
}

#[tokio::test]
async fn refuses_a_redirect_to_a_foreign_host() {
    let server = Arc::new(FakeCalDavServer::new());
    server.configure(|s| s.well_known_location = Some("https://collector.evil.test/".to_string()));
    let error = discovery(server.clone())
        .discover(&fake_server_url(), "me@icloud.test")
        .await
        .unwrap_err();
    assert!(matches!(error, Failure::Source(_)));
    assert!(
        server
            .log()
            .iter()
            .all(|r| r.url.host().as_deref() != Some("collector.evil.test"))
    );
}

#[tokio::test]
async fn falls_back_to_the_server_url_without_well_known() {
    let server = Arc::new(FakeCalDavServer::new());
    server.configure(|s| s.well_known_location = None);
    let account = discovery(server)
        .discover(&fake_server_url(), "me@icloud.test")
        .await
        .unwrap();
    assert_eq!(account.home_url, fake_home_url());
}

#[tokio::test]
async fn wrong_password_is_auth_expired() {
    let error = discovery_with(Arc::new(FakeCalDavServer::new()), "wrong", "icloud.com")
        .discover(&fake_server_url(), "me@icloud.test")
        .await
        .unwrap_err();
    assert_eq!(error, Failure::Source(SourceError::AuthExpired));
}

#[tokio::test]
async fn no_auto_schedule_is_recorded() {
    let server = Arc::new(FakeCalDavServer::new());
    server.configure(|s| s.auto_schedule = false);
    let account = discovery(server)
        .discover(&fake_server_url(), "me@icloud.test")
        .await
        .unwrap();
    assert!(!account.auto_schedule);
}

#[tokio::test]
async fn missing_principal_is_invalid() {
    let transport = FakeTransport::new();
    transport.route(
        "caldav.icloud.com",
        vec![HttpResponse::new(
            207,
            BTreeMap::new(),
            b"<multistatus xmlns=\"DAV:\"/>".to_vec(),
        )],
    );
    let error = discovery(Arc::new(transport))
        .discover(&fake_server_url(), "me@icloud.test")
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        Failure::Source(SourceError::InvalidResponse(_))
    ));
}

#[test]
fn config_round_trips_and_rejects_missing_keys() {
    let account = CalDavAccountConfig::new(
        fake_server_url(),
        "me@icloud.test",
        UrlString::parse("https://caldav.icloud.com/123/principal/").unwrap(),
        fake_home_url(),
        vec![
            "mailto:me@icloud.test".to_string(),
            "urn:uuid:1".to_string(),
        ],
        true,
    );
    assert_eq!(
        CalDavAccountConfig::from_config(&account.config()).unwrap(),
        account
    );
    assert_eq!(
        account.config()["userAddresses"],
        "mailto:me@icloud.test\nurn:uuid:1"
    );
    assert_eq!(account.config()["autoSchedule"], "true");
    assert!(account.config().values().all(|v| !v.contains("app-pass")));
    let mut broken = account.config();
    broken.remove("homeURL");
    assert!(matches!(
        CalDavAccountConfig::from_config(&broken),
        Err(Failure::Source(_))
    ));
}
