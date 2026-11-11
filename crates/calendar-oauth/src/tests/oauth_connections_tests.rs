use std::collections::BTreeMap;

use calendar_core::connection::{Connection, CredentialStore, InMemoryCredentialStore};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::source_types::SourceError;

use crate::access_token_provider::AccessTokenProvider;
use crate::oauth_connections::OAuthConnections;

fn refresh_secrets(token: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(
        AccessTokenProvider::REFRESH_TOKEN_KEY.to_string(),
        token.to_string(),
    )])
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn authorize_stores_the_refresh_token_and_names_the_connection_after_the_email() {
    let store = InMemoryCredentialStore::new();
    let c = OAuthConnections::authorize("google", &store, || async {
        Ok(("me@x.com".to_string(), "r1".to_string()))
    })
    .await
    .unwrap();
    assert_eq!(c.kind_id, "google");
    assert_eq!(c.display_name, "me@x.com");
    assert_eq!(
        c.config,
        BTreeMap::from([("email".to_string(), "me@x.com".to_string())])
    );
    // `UUID().uuidString`: uppercase.
    assert_eq!(c.connection_id, c.connection_id.to_uppercase());
    assert_eq!(c.connection_id.len(), 36);
    assert_eq!(
        store.secrets(&c.connection_id).await.unwrap(),
        Some(refresh_secrets("r1"))
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn authorize_stores_nothing_when_sign_in_fails() {
    let store = InMemoryCredentialStore::new();
    let result = OAuthConnections::authorize("google", &store, || async {
        Err::<(String, String), Failure>(Failure::Source(SourceError::AuthExpired))
    })
    .await;
    assert_eq!(result, Err(Failure::Source(SourceError::AuthExpired)));
    assert!(store.is_empty());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn reauthorize_replaces_the_token_for_the_same_account() {
    let store = InMemoryCredentialStore::new();
    let c = Connection::new("google", "c1", "me@x.com").with_config(&[("email", "me@x.com")]);
    let out = OAuthConnections::reauthorize(&c, &store, || async {
        Ok(("me@x.com".to_string(), "r2".to_string()))
    })
    .await
    .unwrap();
    assert_eq!(out, c);
    assert_eq!(
        store.secrets("c1").await.unwrap(),
        Some(refresh_secrets("r2"))
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn reauthorize_rejects_a_different_account_without_storing() {
    let store = InMemoryCredentialStore::new();
    let c = Connection::new("google", "c1", "me@x.com").with_config(&[("email", "me@x.com")]);
    let result: Outcome<Connection> = OAuthConnections::reauthorize(&c, &store, || async {
        Ok(("other@x.com".to_string(), "r2".to_string()))
    })
    .await;
    assert_eq!(
        result,
        Err(Failure::Source(SourceError::InvalidResponse(
            "signed in as a different account".to_string()
        )))
    );
    assert_eq!(store.secrets("c1").await.unwrap(), None);
}
