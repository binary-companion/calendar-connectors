//! Rust port of the Swift product `MicrosoftCalendar`: Microsoft accounts (Exchange, Outlook.com, Microsoft 365) as a
//! calendar source through Microsoft Graph, with sign-in (the OAuth browser flow with PKCE, or tokens from the host's
//! own account service), change detection by Graph's delta links, series reads and (in the write modules) writes
//! including `.thisAndFollowing` splits.
//!
//! The refresh token of the browser kind is stored only in the host's `CredentialStore`; the brokered kind stores no
//! credential at all. Neither appears in any error, `Debug` output or diagnostic. Sign-in requests the read-only
//! scopes of [`MicrosoftConnectorKind::SCOPES`] and nothing wider (ADR 0017). The crate has no HTTP transport of its
//! own; the host supplies an `HttpTransport` (see `docs/host-http-contract.md`).
//!
//! Parsers return errors and never panic on any input. This crate reads no clock, no environment and no network;
//! time and sleeping arrive as `Now` and `Sleeper`.
//!
//! Public API: [`MicrosoftConnectorKind`] with its [`MicrosoftOAuthConfig`], [`MicrosoftBrokeredConnectorKind`], and
//! [`MicrosoftCalendarSource`], which the kinds build. Everything else is internal, as in Swift.

mod api_client;
mod brokered_connector_kind;
mod connector_kind;
mod dtos;
mod event_mapper;
mod graph_time;
mod recurrence_mapper;
mod source;
mod source_series;
mod source_sync;

pub use brokered_connector_kind::MicrosoftBrokeredConnectorKind;
pub use connector_kind::{MicrosoftConnectorKind, MicrosoftOAuthConfig};
pub use source::MicrosoftCalendarSource;

#[cfg(test)]
mod tests;
