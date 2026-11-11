//! Port of `OAuthConnections.swift`.

use std::collections::BTreeMap;
use std::future::Future;

use calendar_core::connection::{Connection, CredentialStore};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::source_types::SourceError;
use uuid::Uuid;

use crate::access_token_provider::AccessTokenProvider;

/// The authorize and reauthorize flows the OAuth connectors share. `sign_in` runs the provider's browser flow and
/// returns the account email and refresh token; nothing is persisted until it succeeds.
pub struct OAuthConnections;

impl OAuthConnections {
    pub async fn authorize<F, Fut>(
        kind_id: &str,
        credentials: &dyn CredentialStore,
        sign_in: F,
    ) -> Outcome<Connection>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Outcome<(String, String)>>,
    {
        let (email, refresh_token) = sign_in().await?;
        // `UUID().uuidString` is uppercase.
        let connection = Connection::new(
            kind_id,
            Uuid::new_v4().to_string().to_uppercase(),
            email.clone(),
        )
        .with_config(&[("email", &email)]);
        credentials
            .set_secrets(refresh_secrets(refresh_token), &connection.connection_id)
            .await?;
        Ok(connection)
    }

    pub async fn reauthorize<F, Fut>(
        connection: &Connection,
        credentials: &dyn CredentialStore,
        sign_in: F,
    ) -> Outcome<Connection>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Outcome<(String, String)>>,
    {
        let (email, refresh_token) = sign_in().await?;
        if connection.config.get("email") != Some(&email) {
            return Err(Failure::Source(SourceError::InvalidResponse(
                "signed in as a different account".to_string(),
            )));
        }
        credentials
            .set_secrets(refresh_secrets(refresh_token), &connection.connection_id)
            .await?;
        Ok(connection.clone())
    }
}

fn refresh_secrets(refresh_token: String) -> BTreeMap<String, String> {
    BTreeMap::from([(
        AccessTokenProvider::REFRESH_TOKEN_KEY.to_string(),
        refresh_token,
    )])
}
