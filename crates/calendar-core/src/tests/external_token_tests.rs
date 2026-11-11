use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;

use super::support::at;
use crate::connection::{
    AuthorizationInteraction, AuthorizationMethod, Connection, ConnectorKind, CredentialStore,
    Platform, SyncStateStore,
};
use crate::external_tokens::{
    AccountIdentityFamily, ExternalAccessToken, ExternalAccount, ExternalSignInRequest,
    ExternalSignInResult,
};
use crate::failure::{Failure, Outcome};
use crate::source_types::{CalendarSource, SourceError};
use crate::time::add_seconds;

struct StubKind;

#[async_trait]
impl ConnectorKind for StubKind {
    fn id(&self) -> String {
        "stub".to_string()
    }
    fn display_name(&self) -> String {
        "Stub".to_string()
    }
    fn supported_platforms(&self) -> Platform {
        Platform::WINDOWS
    }
    fn authorization(&self) -> AuthorizationMethod {
        AuthorizationMethod::System
    }
    async fn authorize(
        &self,
        _interaction: &dyn AuthorizationInteraction,
        _credentials: &dyn CredentialStore,
    ) -> Outcome<Connection> {
        Err(Failure::Source(SourceError::NotFound))
    }
    async fn reauthorize(
        &self,
        connection: &Connection,
        _interaction: &dyn AuthorizationInteraction,
        _credentials: &dyn CredentialStore,
    ) -> Outcome<Connection> {
        Ok(connection.clone())
    }
    fn make_source(
        &self,
        _connection: &Connection,
        _credentials: Arc<dyn CredentialStore>,
        _sync_state: Arc<dyn SyncStateStore>,
    ) -> Outcome<Arc<dyn CalendarSource>> {
        Err(Failure::Source(SourceError::NotFound))
    }
}

struct FamilyKind;

impl AccountIdentityFamily for FamilyKind {
    fn identity_family(&self) -> String {
        "family".to_string()
    }
}

#[async_trait]
impl ConnectorKind for FamilyKind {
    fn id(&self) -> String {
        "family-kind".to_string()
    }
    fn display_name(&self) -> String {
        "Family".to_string()
    }
    fn supported_platforms(&self) -> Platform {
        Platform::WINDOWS
    }
    fn authorization(&self) -> AuthorizationMethod {
        AuthorizationMethod::System
    }
    async fn authorize(
        &self,
        _interaction: &dyn AuthorizationInteraction,
        _credentials: &dyn CredentialStore,
    ) -> Outcome<Connection> {
        Err(Failure::Source(SourceError::NotFound))
    }
    async fn reauthorize(
        &self,
        connection: &Connection,
        _interaction: &dyn AuthorizationInteraction,
        _credentials: &dyn CredentialStore,
    ) -> Outcome<Connection> {
        Ok(connection.clone())
    }
    fn make_source(
        &self,
        _connection: &Connection,
        _credentials: Arc<dyn CredentialStore>,
        _sync_state: Arc<dyn SyncStateStore>,
    ) -> Outcome<Arc<dyn CalendarSource>> {
        Err(Failure::Source(SourceError::NotFound))
    }
    fn as_identity_family(&self) -> Option<&dyn AccountIdentityFamily> {
        Some(self)
    }
}

#[test]
fn external_account_init_defaults_optional_fields_to_nil() {
    let account = ExternalAccount::new("acct-1");
    assert_eq!(account.id, "acct-1");
    assert_eq!(account.email, None);
    assert_eq!(account.connection_id, None);
    let full = ExternalAccount {
        email: Some("me@example.com".to_string()),
        connection_id: Some("c1".to_string()),
        ..ExternalAccount::new("acct-1")
    };
    assert!(
        full.email.as_deref() == Some("me@example.com")
            && full.connection_id.as_deref() == Some("c1")
    );
    assert_ne!(full, account);
    let set: HashSet<ExternalAccount> =
        HashSet::from([account.clone(), ExternalAccount::new("acct-1")]);
    assert_eq!(set.len(), 1);
}

#[test]
fn sign_in_request_without_account_is_a_first_sign_in() {
    let request = ExternalSignInRequest::new(vec!["User.Read".to_string()]);
    assert_eq!(request.account, None);
    assert_eq!(request.email, None);
    assert_eq!(request.scopes, vec!["User.Read".to_string()]);
    let again = ExternalSignInRequest {
        account: Some(ExternalAccount::new("a")),
        email: Some("me@example.com".to_string()),
        scopes: vec!["User.Read".to_string()],
    };
    assert!(
        again.account.as_ref().map(|a| a.id.as_str()) == Some("a")
            && again.email.as_deref() == Some("me@example.com")
    );
}

#[test]
fn access_token_and_sign_in_result_carry_their_fields() {
    let expiry = at(1_800_000_000);
    let token = ExternalAccessToken::new("t", expiry);
    assert_eq!(token, ExternalAccessToken::new("t", expiry));
    assert_ne!(
        token,
        ExternalAccessToken::new("t", add_seconds(expiry, 1.0))
    );
    let result = ExternalSignInResult::new(ExternalAccount::new("a"), token.clone());
    assert!(result.account.id == "a" && result.token == token);
}

#[test]
fn kind_without_identity_family_yields_nil_on_cast() {
    let kind: Arc<dyn ConnectorKind> = Arc::new(StubKind);
    assert!(kind.as_identity_family().is_none());
    let family: Arc<dyn ConnectorKind> = Arc::new(FamilyKind);
    assert_eq!(
        family.as_identity_family().map(|f| f.identity_family()),
        Some("family".to_string())
    );
}
