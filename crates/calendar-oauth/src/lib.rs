//! Rust port of the Swift product `CalendarOAuth`: OAuth 2.0 with PKCE, token providers and the shared
//! authorize/reauthorize flows.
//!
//! `SHA256Hashing.swift` is not ported: the Swift seam existed only to avoid a crypto dependency, and `sha2` (pinned
//! by `vectors/oauth/pkce.json`) replaces it. This crate reads no clock, no environment and no network.

pub mod access_token_provider;
pub mod access_token_supplying;
pub mod external_access_token_provider;
pub mod oauth;
pub mod oauth_connections;
pub mod pkce;

pub use access_token_provider::{AccessTokenProvider, RefreshFunction};
pub use access_token_supplying::AccessTokenSupplying;
pub use external_access_token_provider::ExternalAccessTokenProvider;
pub use oauth::{
    AuthorizationCodeError, AuthorizationError, OAuthClient, OAuthConfig, OAuthTokens,
};
pub use oauth_connections::OAuthConnections;

#[cfg(test)]
mod tests;
