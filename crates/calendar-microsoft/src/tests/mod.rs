#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

mod api_client_tests;
mod brokered_connector_kind_tests;
mod change_detection_tests;
mod connector_kind_tests;
mod graph_time_vectors;
mod mapper_tests;
mod persisted_format_tests;
mod recurrence_mapper_tests;
mod rust_additions_tests;
mod source_tests;
mod time_tests;

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
