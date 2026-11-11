#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod all_day_tests;
mod all_day_vectors;
mod calendar_user_address_tests;
mod conference_detector_tests;
mod conference_vectors;
mod diagnostic_log_tests;
mod files;
mod http_tests;
mod json_helpers;
mod model_tests;
mod support;
mod time_vectors;
mod url_accessor_tests;
mod url_vectors;
mod windows_zone_vectors;

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
