#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod all_day_tests;
mod all_day_vectors;
mod calendar_user_address_tests;
mod change_monitor_tests;
mod conference_detector_tests;
mod conference_vectors;
mod connection_tests;
mod diagnostic_log_tests;
mod event_draft_tests;
mod event_patch_tests;
mod external_token_tests;
mod file_stores_tests;
mod files;
mod http_tests;
mod json_helpers;
mod model_tests;
mod patch_merge_tests;
mod persisted_format_tests;
mod recurrence_robustness;
mod recurrence_rule_tests;
mod recurrence_set_occurrences_tests;
mod recurrence_set_tests;
mod recurrence_vectors;
mod rule_expander_tests;
mod shared_connector_helpers_tests;
mod support;
mod time_vectors;
mod url_accessor_tests;
mod url_vectors;
mod windows_zone_vectors;
mod write_types_tests;

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
