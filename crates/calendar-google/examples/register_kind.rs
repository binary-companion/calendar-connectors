//! Builds the Google connector kind and registers it. Sign-in and reading need a real Google account and a host that
//! can open a browser, so this example stops at what a host does at start-up; the sign-in flow is in
//! `docs/api.md`. Run it with `cargo run -p calendar-google --example register_kind`.

#![allow(clippy::print_stdout, clippy::expect_used)]

use std::sync::Arc;

use async_trait::async_trait;
use calendar_core::connection::{AuthorizationMethod, ConnectorRegistry, Platform};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::{HttpRequest, HttpResponse, HttpTransport};
use calendar_core::source_types::SourceError;
use calendar_core::time::{Now, Sleeper};
use calendar_google::{GoogleConnectorKind, GoogleOAuthConfig};

/// A host's transport wraps its HTTP client; this one has no network.
struct OfflineTransport;

#[async_trait]
impl HttpTransport for OfflineTransport {
    async fn send(&self, _request: HttpRequest) -> Outcome<HttpResponse> {
        Err(Failure::Source(SourceError::Network("offline".to_string())))
    }
}

/// Runs the example (public so that the crate's test can call it).
pub fn main() {
    // The OAuth client id and secret come from the host's own configuration, never from this library.
    let config = GoogleOAuthConfig::new("client-id.example", "client-secret.example");
    let now: Now = Arc::new(jiff::Timestamp::now);
    let sleep: Sleeper = Arc::new(|seconds| {
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_secs_f64(seconds)).await;
            Ok(())
        })
    });
    let kind = GoogleConnectorKind::new(config, Arc::new(OfflineTransport), now, sleep)
        .with_poll_interval(120.0);

    let mut registry = ConnectorRegistry::new();
    registry.register(Arc::new(kind));

    // A host offers the kinds that work on its platform.
    let offered = registry.kinds_for(Platform::current());
    println!("{} kind(s) available here", offered.len());
    let google = registry.kind("google").expect("registered above");
    assert!(matches!(google.authorization(), AuthorizationMethod::Oauth));
}
