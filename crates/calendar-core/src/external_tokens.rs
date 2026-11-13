//! The seam for hosts that get access tokens from the operating system's own account service (Windows' Web Account
//! Manager, a mobile account broker) instead of running the OAuth browser flow. The library never sees a password
//! or a refresh token for such an account; it only asks the host for short-lived access tokens.
//! Port of `ExternalTokens.swift`.

use async_trait::async_trait;
use jiff::Timestamp;

use crate::connection::ConnectionId;
use crate::failure::Outcome;

/// An account the host's account service knows. `id` is the host's opaque identifier and is authoritative; `email`
/// and `connection_id` are informational (a host can use them to find the account when its id is not enough).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ExternalAccount {
    /// The host's opaque, authoritative account identifier.
    pub id: String,
    /// The account's email address, if the host knows it.
    pub email: Option<String>,
    /// The connection this account was created for, if known.
    pub connection_id: Option<ConnectionId>,
}

impl ExternalAccount {
    /// Swift's `init(id:email:connectionID:)` with `email` and `connectionID` left at nil.
    pub fn new(id: impl Into<String>) -> ExternalAccount {
        ExternalAccount {
            id: id.into(),
            email: None,
            connection_id: None,
        }
    }
}

/// An access token obtained from the host's account service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalAccessToken {
    /// The bearer token sent with API requests.
    pub access_token: String,
    /// When the token expires.
    pub expires_at: Timestamp,
}

impl ExternalAccessToken {
    /// Creates a token from its text and expiry.
    pub fn new(access_token: impl Into<String>, expires_at: Timestamp) -> ExternalAccessToken {
        ExternalAccessToken {
            access_token: access_token.into(),
            expires_at,
        }
    }
}

/// An interactive sign-in. `account` nil is a first sign-in (the host lets the person pick or confirm an account);
/// a given account is a reconnect, with `email` as a hint for the host's account picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalSignInRequest {
    /// The account being reconnected, or `None` for a first sign-in.
    pub account: Option<ExternalAccount>,
    /// A hint for the host's account picker.
    pub email: Option<String>,
    /// Scopes the access token must cover.
    pub scopes: Vec<String>,
}

impl ExternalSignInRequest {
    /// Swift's `init(account:email:scopes:)` with `account` and `email` left at nil.
    pub fn new(scopes: Vec<String>) -> ExternalSignInRequest {
        ExternalSignInRequest {
            account: None,
            email: None,
            scopes,
        }
    }
}

/// The outcome of an interactive sign-in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalSignInResult {
    /// The account that signed in.
    pub account: ExternalAccount,
    /// An access token for that account.
    pub token: ExternalAccessToken,
}

impl ExternalSignInResult {
    /// Creates a result from its parts.
    pub fn new(account: ExternalAccount, token: ExternalAccessToken) -> ExternalSignInResult {
        ExternalSignInResult { account, token }
    }
}

/// Implemented by the host. Errors follow the library's usual mapping: `SourceError::AuthExpired` when the person
/// must sign in again, `SourceError::Network` when offline, `Failure::Cancelled` when the person dismisses the
/// dialog.
#[async_trait]
pub trait ExternalTokenSource: Send + Sync {
    /// A token without any user interface. Fails with `SourceError::AuthExpired` when interaction is needed.
    async fn access_token(
        &self,
        account: &ExternalAccount,
        scopes: &[String],
        force_refresh: bool,
    ) -> Outcome<ExternalAccessToken>;
    /// A token with the host's own sign-in user interface.
    async fn sign_in(&self, request: &ExternalSignInRequest) -> Outcome<ExternalSignInResult>;
}

/// Adopted by a `ConnectorKind` whose accounts are the same people as another kind's (Microsoft in the browser and
/// Microsoft through the host's broker). Kinds with the same family and the same account name are one account.
/// Optional: hosts check with `ConnectorKind::as_identity_family` (Swift `as?`), and a kind without it is its own
/// family (its kind id).
pub trait AccountIdentityFamily {
    /// The identity family of this kind, shared with other kinds that sign in to the same accounts.
    fn identity_family(&self) -> String;
}
