//! The error type every fallible operation of the connector library returns (conventions section 4, `throws`).
//!
//! `SourceError` lives in `source_types`; `WriteError` is defined with the write model and adds the `Write` variant.

use crate::source_types::SourceError;

/// What a Swift `throws` function threw: a connector's own error, a cancellation, or a host failure.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum Failure {
    /// A connector's own error.
    #[error(transparent)]
    Source(#[from] SourceError),
    /// Swift's `CancellationError`.
    #[error("cancelled")]
    Cancelled,
    /// A failure reported by the host (a transport, a credential store, a user interaction) as text.
    #[error("{0}")]
    Host(String),
}

/// The result of an operation that can fail with a [`Failure`].
pub type Outcome<T> = Result<T, Failure>;
