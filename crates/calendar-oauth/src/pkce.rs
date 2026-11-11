//! Port of `PKCE.swift`. `SHA256Hashing.swift` is not ported: `sha2` replaces the Swift seam, and
//! `vectors/oauth/pkce.json` pins the result.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use calendar_core::failure::{Failure, Outcome};
use sha2::{Digest, Sha256};

const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";

/// Swift's `randomString(length:using:)`: each character is drawn uniformly from the 66 unreserved characters of RFC
/// 3986, using `next_u64` as the source of randomness (the Swift `RandomNumberGenerator`). Values that would make
/// the choice uneven are skipped.
pub fn random_string_using(length: usize, next_u64: &mut dyn FnMut() -> u64) -> String {
    let size = ALPHABET.len() as u64;
    // The largest multiple of the alphabet size: draws at or above it are skipped, so every character is equally
    // likely.
    let limit = (u64::MAX / size) * size;
    let mut out = String::with_capacity(length);
    while out.len() < length {
        let draw = next_u64();
        if draw < limit {
            out.push(char::from(ALPHABET[(draw % size) as usize]));
        }
    }
    out
}

/// Swift's `randomString(length:)` (default 64): drawn from the operating system's random source through
/// `getrandom`. Fails with `Failure::Host` if the system has no random source; a weaker one is never substituted,
/// because the text is a PKCE verifier or an OAuth `state`.
pub fn random_string(length: usize) -> Outcome<String> {
    let mut failed = false;
    let mut next = || {
        let mut bytes = [0u8; 8];
        if getrandom::fill(&mut bytes).is_err() {
            failed = true;
            // Any value below the limit ends the loop quickly; the result is discarded.
            return 0;
        }
        u64::from_le_bytes(bytes)
    };
    let text = random_string_using(length, &mut next);
    if failed {
        return Err(Failure::Host(
            "the system random number generator failed".to_string(),
        ));
    }
    Ok(text)
}

/// base64url(SHA-256(verifier)) without padding (RFC 7636, method S256).
pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
