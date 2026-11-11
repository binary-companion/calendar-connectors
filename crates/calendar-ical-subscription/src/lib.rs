//! Rust port of the Swift product `ICalSubscription`: a read-only calendar fed by a subscription link
//! (`webcal://` or `https://`), such as Meetup's "Add to calendar" links or a Google secret address.
//!
//! The feed link is a credential (ADR 0018): it is stored only in the host's `CredentialStore`, and no error, diagnostic
//! or `Debug` output of this crate contains it. The crate has no HTTP transport of its own; the host supplies an
//! `HttpTransport` that does not follow redirects (see [`ICalSubscriptionKind`] and `docs/host-http-contract.md`).
//!
//! Parsers return errors and never panic on any input. This crate reads no clock, no environment and no network.
//!
//! Public API: [`ICalSubscriptionKind`] (a `ConnectorKind`), [`RetentionWindow`], and [`ICalSubscriptionSource`], which
//! the kind builds. Everything else is internal, as in Swift.

mod feed_fetcher;
mod feed_location;
mod feed_parser;
mod kind;
mod retention_window;
mod source;

pub use kind::ICalSubscriptionKind;
pub use retention_window::RetentionWindow;
pub use source::ICalSubscriptionSource;

#[cfg(test)]
mod tests;
