//! Rust port of the Swift product `CalDAVCalendar`: CalDAV accounts (iCloud and any other server) as a read and write
//! calendar source, with sign-in, discovery (RFC 6764, RFC 4791), change detection by sync token (RFC 6578) or ctag,
//! and series editing including `.thisAndFollowing` splits.
//!
//! The account password is stored only in the host's `CredentialStore`, sent only as HTTP Basic to `https` hosts inside
//! the account's host base, and appears in no error, `Debug` output or diagnostic. The crate has no HTTP transport of
//! its own; the host supplies an `HttpTransport` that does not follow redirects (see [`ICloudConnectorKind`] and
//! `docs/host-http-contract.md`).
//!
//! Parsers return errors and never panic on any input (the XML reader also bounds nesting depth and never expands
//! entities). This crate reads no clock, no environment and no network.
//!
//! Public API: [`ICloudConnectorKind`] and [`CalDavConnectorKind`] (both `ConnectorKind`s, built with a transport, a
//! `now`, a sleeper and a default zone) and [`CalDavCalendarSource`], which the kinds build. Everything else is
//! internal, as in Swift.

mod account_config;
mod connector_kinds;
mod dav_url;
mod dav_xml;
mod discovery;
mod source;
mod source_series;
mod source_split;
mod source_sync;
mod source_write;
mod webdav_client;
mod xml_tree;

pub use connector_kinds::{CalDavConnectorKind, ICloudConnectorKind};
pub use source::CalDavCalendarSource;

#[cfg(test)]
mod tests;
