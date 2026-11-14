#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod fixtures;

mod alarm_mapper_tests;
mod content_line_tests;
mod content_line_vectors;
mod event_reader_tests;
mod event_writer_tests;
mod series_editor_tests;
mod time_zone_tests;
mod time_zone_vectors;
mod values_tests;

use serde_json::Value;

/// The `cases` array of a golden vector file (conventions section 9).
fn vector_cases(text: &str) -> Vec<Value> {
    let file: Value = serde_json::from_str(text).expect("vector file is JSON");
    file["cases"].as_array().expect("cases array").clone()
}

/// Fails with every failing case name at once, not only the first.
fn assert_no_failures(vector: &str, total: usize, failures: Vec<String>) {
    assert!(
        failures.is_empty(),
        "{vector}: {} of {total} cases failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
mod garbage_input_tests;
mod swift_probe_tests;
