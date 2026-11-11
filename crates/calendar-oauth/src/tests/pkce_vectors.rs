use super::{assert_no_failures, vector_cases};
use crate::pkce;

const PKCE: &str = include_str!("../../../../vectors/oauth/pkce.json");

#[test]
fn pkce_vector() {
    let cases = vector_cases(PKCE);
    assert!(cases.len() >= 3, "the vector file has cases");
    let mut failures = Vec::new();
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let verifier = case["input"]["verifier"].as_str().unwrap();
        let expected = case["expected"]["challenge"].as_str().unwrap();
        let actual = pkce::challenge(verifier);
        if actual != expected {
            failures.push(format!(
                "{name}: {verifier:?} gave {actual}, expected {expected}"
            ));
        }
    }
    assert_no_failures("oauth/pkce", cases.len(), failures);
}
