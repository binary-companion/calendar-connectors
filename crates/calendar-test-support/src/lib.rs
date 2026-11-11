//! Rust port of the Swift product `CalendarTestSupport`: fakes and conformance checks for tests.
//!
//! This is a normal library (other repositories' tests use it), so its public items are not `cfg(test)`. Its
//! conformance helpers return the violations they found; the tests that use them assert on the result.
//!
//! Ported so far: `AllDayConformance`, `FakeWritableSource`, `ProvidedFieldsConformance` and
//! `WritableSourceConformance`. `FakeTransport`, `FakeExternalTokenSource` and `CollectingDiagnosticLog` follow with
//! the tasks that need them.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod all_day_conformance;
pub mod fake_writable_source;
pub mod provided_fields_conformance;
pub mod writable_source_conformance;

use calendar_core::write::event_patch::EventPatch;

/// A hand-built patch (no base): `EventPatch::default()` with the fields `set` assigns. `EventPatch` keeps its `base`
/// private (Swift `private(set)`), so struct update syntax is not available outside `calendar-core`. Swift's
/// `EventPatch(title: ...)` is `hand_patch(|p| p.title = Some(...))`.
pub fn hand_patch(set: impl FnOnce(&mut EventPatch)) -> EventPatch {
    let mut patch = EventPatch::default();
    set(&mut patch);
    patch
}

#[cfg(test)]
mod tests;
