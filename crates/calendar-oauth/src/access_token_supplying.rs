//! Port of `AccessTokenSupplying.swift`.

use async_trait::async_trait;
use calendar_core::failure::Outcome;

use crate::access_token_provider::AccessTokenProvider;

/// What an API client needs from a token provider: a valid access token, and a way to say that the last one was
/// rejected. Adopted by `AccessTokenProvider` (refresh token held by the library) and `ExternalAccessTokenProvider`
/// (tokens supplied by the host).
#[async_trait]
pub trait AccessTokenSupplying: Send + Sync {
    async fn access_token(&self) -> Outcome<String>;
    /// Drops the cached token (call after a 401) so the next `access_token()` asks for a fresh one.
    async fn invalidate(&self);
}

#[async_trait]
impl AccessTokenSupplying for AccessTokenProvider {
    async fn access_token(&self) -> Outcome<String> {
        AccessTokenProvider::access_token(self).await
    }

    async fn invalidate(&self) {
        AccessTokenProvider::invalidate(self);
    }
}
