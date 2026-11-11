//! Rust port of the Swift product `GoogleCalendar`: Google accounts as a read and write calendar source, with sign-in
//! (the OAuth browser flow with PKCE), change detection by Google's sync tokens, series reads and (in the write
//! modules) writes including `.thisAndFollowing` splits.
//!
//! The refresh token is stored only in the host's `CredentialStore` and appears in no error, `Debug` output or
//! diagnostic. Sign-in requests the two read-only scopes of [`GoogleConnectorKind::SCOPES`] and nothing wider. The
//! crate has no HTTP transport of its own; the host supplies an `HttpTransport` (see `docs/host-http-contract.md`).
//!
//! Parsers return errors and never panic on any input. This crate reads no clock, no environment and no network;
//! time and sleeping arrive as `Now` and `Sleeper`.
//!
//! Public API: [`GoogleConnectorKind`] with its [`GoogleOAuthConfig`], and [`GoogleCalendarSource`], which the kind
//! builds. Everything else is internal, as in Swift.

mod api_client;
mod connector_kind;
mod dtos;
mod event_mapper;
mod source;
mod source_series;
mod source_write;
mod write_mapper;

pub use connector_kind::{GoogleConnectorKind, GoogleOAuthConfig};
pub use source::GoogleCalendarSource;

#[cfg(test)]
mod tests;
