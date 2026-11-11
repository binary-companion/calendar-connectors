#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod access_token_provider_tests;
mod external_access_token_provider_tests;
mod oauth_connections_tests;
mod oauth_tests;
mod pkce_tests;
mod pkce_vectors;
mod token_response_garbage_tests;

use calendar_core::UrlString;
use serde_json::Value;

/// `URL(string:)!`.
fn url(text: &str) -> UrlString {
    UrlString::parse(text).unwrap()
}

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

/// A pause of `turns` scheduler turns: the stand-in for the Swift tests' `Task.sleep(for: .milliseconds(30))`
/// (no crate calls `tokio::time`; on a current-thread runtime a fixed number of yields is deterministic).
async fn pause(turns: usize) {
    for _ in 0..turns {
        tokio::task::yield_now().await;
    }
}
