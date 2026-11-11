//! The write model: what a write needs to find an event, what it carries, and how concurrent edits are merged.

pub mod event_draft;
pub mod event_patch;
pub mod patch_merge;
pub mod types;
pub mod writable_source;
