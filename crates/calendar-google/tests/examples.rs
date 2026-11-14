//! Runs the example in `examples/register_kind.rs` as a test, so the code shown in `docs/api.md` is proven by `cargo test`.
#![allow(clippy::print_stdout, clippy::unwrap_used)]

#[path = "../examples/register_kind.rs"]
mod register_kind;

#[test]
fn the_example_runs() {
    register_kind::main();
}

/// `docs/api.md` shows this example verbatim; fail when the two drift apart.
#[test]
fn the_api_reference_shows_the_example_as_it_is() {
    let doc = include_str!("../../../docs/api.md");
    let source = include_str!("../examples/register_kind.rs");
    assert!(doc.contains(source.trim_end()));
}
