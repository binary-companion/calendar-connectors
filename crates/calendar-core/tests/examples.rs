//! Runs the example in `examples/expand_recurrence.rs` as a test, so the code shown in `docs/api.md` is proven by `cargo test`.
#![allow(clippy::print_stdout, clippy::unwrap_used)]

#[path = "../examples/expand_recurrence.rs"]
mod expand_recurrence;

#[test]
fn the_example_runs() {
    expand_recurrence::main();
}

/// `docs/api.md` shows this example verbatim; fail when the two drift apart.
#[test]
fn the_api_reference_shows_the_example_as_it_is() {
    let doc = include_str!("../../../docs/api.md");
    let source = include_str!("../examples/expand_recurrence.rs");
    assert!(doc.contains(source.trim_end()));
}
