use std::collections::BTreeSet;

use crate::pkce;

#[test]
fn pkce_challenge_matches_rfc7636_vector() {
    assert_eq!(
        pkce::challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
}

#[test]
fn pkce_random_string_uses_unreserved_alphabet_and_length() {
    let s = pkce::random_string(64).unwrap();
    assert_eq!(s.chars().count(), 64);
    let allowed: BTreeSet<char> =
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~"
            .chars()
            .collect();
    assert!(s.chars().all(|c| allowed.contains(&c)));
    assert_ne!(pkce::random_string(64).unwrap(), s);
}

/// The generator seam (Swift's `using generator:` overload): the same source gives the same text, and every one of
/// the 66 alphabet characters is reachable.
#[test]
fn pkce_random_string_using_a_generator_is_deterministic_and_covers_the_alphabet() {
    let mut counter: u64 = 0;
    let mut step = move || {
        counter = counter.wrapping_add(0x9E37_79B9_7F4A_7C15);
        counter
    };
    let first = pkce::random_string_using(2000, &mut step);
    let mut counter2: u64 = 0;
    let mut step2 = move || {
        counter2 = counter2.wrapping_add(0x9E37_79B9_7F4A_7C15);
        counter2
    };
    assert_eq!(first, pkce::random_string_using(2000, &mut step2));
    let seen: BTreeSet<char> = first.chars().collect();
    assert_eq!(seen.len(), 66);
}

#[test]
fn pkce_random_string_of_length_zero_is_empty() {
    assert_eq!(pkce::random_string(0).unwrap(), "");
}
