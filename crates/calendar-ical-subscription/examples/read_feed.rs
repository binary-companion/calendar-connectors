//! Signs in to an iCalendar subscription link and reads its events, with a scripted host. This is the whole shape
//! of a connector integration: a host supplies HTTP, credential and sync-state storage and the sign-in interaction,
//! registers a connector kind, and reads events from the source the kind makes.
//!
//! The "network" here is a function that answers with a fixed feed, so the example needs no account and no
//! connection. Run it with `cargo run -p calendar-ical-subscription --example read_feed`.

#![allow(clippy::print_stdout, clippy::expect_used)]

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use calendar_core::connection::{
    AuthorizationInteraction, ConnectorRegistry, CredentialField, InMemoryCredentialStore,
    InMemorySyncStateStore, OAuthRedirectSession,
};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::{HttpRequest, HttpResponse, HttpTransport};
use calendar_core::source_types::SourceError;
use calendar_core::time::{DateInterval, Now, Sleeper, Zone};
use calendar_ical_subscription::ICalSubscriptionKind;
use jiff::Timestamp;

const FEED: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Example//Docs//EN\r\n\
    X-WR-CALNAME:Example meetups\r\nBEGIN:VEVENT\r\nUID:docs-1@example.test\r\nDTSTAMP:20260901T000000Z\r\n\
    DTSTART:20260910T190000Z\r\nDTEND:20260910T210000Z\r\nSUMMARY:Board games night\r\nEND:VEVENT\r\n\
    END:VCALENDAR\r\n";

/// The host's HTTP. A real host wraps its HTTP client here. It must return every status as a response, must not
/// follow redirects for this connector, and must not cache (see `docs/host-http-contract.md`).
struct FixedTransport;

#[async_trait]
impl HttpTransport for FixedTransport {
    async fn send(&self, request: HttpRequest) -> Outcome<HttpResponse> {
        if request.url.host().as_deref() == Some("calendar.example.test") {
            Ok(HttpResponse::new(
                200,
                BTreeMap::new(),
                FEED.as_bytes().to_vec(),
            ))
        } else {
            // A transport failure is a `Network` error; a cancelled request would be `Failure::Cancelled`.
            Err(Failure::Source(SourceError::Network(
                "no route".to_string(),
            )))
        }
    }
}

/// The host's sign-in UI. A real host shows `fields` and returns what the user typed.
struct ScriptedSignIn(&'static str);

#[async_trait]
impl AuthorizationInteraction for ScriptedSignIn {
    async fn begin_oauth_redirect(&self) -> Outcome<Box<dyn OAuthRedirectSession>> {
        Err(Failure::Cancelled) // this kind uses a password-style prompt, not an OAuth redirect
    }

    async fn prompt_credentials(
        &self,
        fields: &[CredentialField],
    ) -> Outcome<BTreeMap<String, String>> {
        let mut values = BTreeMap::new();
        for field in fields {
            if field.key == "link" {
                values.insert(field.key.clone(), self.0.to_string());
            }
        }
        Ok(values)
    }
}

/// Runs the example (public so that the crate's test can call it).
#[tokio::main(flavor = "current_thread")]
pub async fn main() -> Outcome<()> {
    // The library reads no clock and sleeps only through what the host passes.
    let now: Now = Arc::new(|| "2026-09-01T12:00:00Z".parse().expect("a valid instant"));
    let sleep: Sleeper = Arc::new(|_seconds| Box::pin(async { Ok(()) }));
    let kind = ICalSubscriptionKind::new(Arc::new(FixedTransport), now, sleep, Zone::utc());

    // Kinds are registered once; a host looks them up by `Connection::kind_id` later.
    let mut registry = ConnectorRegistry::new();
    registry.register(Arc::new(kind));
    let kind = registry
        .kind(ICalSubscriptionKind::KIND_ID)
        .expect("registered above");

    // Sign in: the secret (the link) goes to the credential store, the `Connection` is non-secret and the host
    // persists it.
    let credentials = Arc::new(InMemoryCredentialStore::new());
    let connection = kind
        .authorize(
            &ScriptedSignIn("webcal://calendar.example.test/feed/PRIVATE-TOKEN.ics"),
            credentials.as_ref(),
        )
        .await?;
    assert_eq!(
        connection.display_name,
        "Example meetups (calendar.example.test)"
    );

    // Make the source and read.
    let source = kind.make_source(
        &connection,
        credentials,
        Arc::new(InMemorySyncStateStore::new()),
    )?;
    let window = DateInterval::new(
        "2026-09-01T00:00:00Z"
            .parse::<Timestamp>()
            .expect("a valid instant"),
        "2026-10-01T00:00:00Z"
            .parse::<Timestamp>()
            .expect("a valid instant"),
    );
    let events = source.events_in(window).await?;
    assert_eq!(events.len(), 1);
    println!("{}: {}", events[0].start, events[0].title);

    // Errors: every fallible call returns `Outcome<T>`, a `Result` whose error is `Failure`. Match on it to decide
    // what to tell the user and whether to retry.
    let unreachable = ScriptedSignIn("https://elsewhere.example.test/feed.ics");
    match kind
        .authorize(&unreachable, &InMemoryCredentialStore::new())
        .await
    {
        Err(Failure::Source(SourceError::Network(_))) => println!("offline: try again later"),
        Err(Failure::Cancelled) => println!("the user cancelled"),
        Err(other) => println!("failed: {other}"),
        Ok(_) => unreachable!("the host has no route to that address"),
    }
    Ok(())
}
