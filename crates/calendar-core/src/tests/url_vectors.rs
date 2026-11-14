use super::{assert_no_failures, vector_cases};
use crate::UrlString;

const URL_ACCEPT: &str = include_str!("../../../../vectors/core/url-accept.json");

#[test]
fn url_accept_vector() {
    let cases = vector_cases(URL_ACCEPT);
    let mut failures = Vec::new();
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let text = case["input"]["string"].as_str().unwrap();
        let accepted = case["expected"]["accepted"].as_bool().unwrap();
        let host = case["expected"]["host"].as_str().map(str::to_string);
        let scheme = case["expected"]["scheme"].as_str().map(str::to_string);
        let shown: String = text.chars().take(60).collect();
        match UrlString::parse(text) {
            None if accepted => {
                failures.push(format!("{name}: {shown:?} rejected, Foundation accepted"))
            }
            Some(_) if !accepted => {
                failures.push(format!("{name}: {shown:?} accepted, Foundation rejected"))
            }
            None => {}
            Some(url) => {
                if url.as_str() != text {
                    failures.push(format!("{name}: as_str changed the text"));
                }
                if url.host() != host {
                    failures.push(format!(
                        "{name}: {shown:?} host {:?} expected {host:?}",
                        url.host()
                    ));
                }
                if url.scheme() != scheme {
                    failures.push(format!(
                        "{name}: {shown:?} scheme {:?} expected {scheme:?}",
                        url.scheme()
                    ));
                }
            }
        }
    }
    assert_eq!(cases.len(), 300);
    assert_no_failures("core/url-accept", cases.len(), failures);
}

#[test]
fn as_str_keeps_the_original_text_and_parsed_normalizes() {
    let url = UrlString::parse("HTTPS://EXAMPLE.COM/Path").unwrap();
    assert_eq!(url.as_str(), "HTTPS://EXAMPLE.COM/Path");
    let parsed = url.parsed().unwrap();
    assert_eq!(parsed.as_str(), "https://example.com/Path");
    assert!(
        UrlString::parse("relative/path")
            .unwrap()
            .parsed()
            .is_none()
    );
}

#[test]
fn serde_round_trips_the_original_text() {
    let url = UrlString::parse("https://example.com").unwrap();
    let json = serde_json::to_string(&url).unwrap();
    assert_eq!(json, "\"https://example.com\"");
    let back: UrlString = serde_json::from_str(&json).unwrap();
    assert_eq!(back, url);
    assert!(serde_json::from_str::<UrlString>("\"\"").is_err());
}

#[test]
fn parse_never_panics_on_odd_input() {
    for text in [
        "\u{0}",
        "%",
        "://",
        ":",
        "[",
        "]]]",
        "https://[",
        "https://@",
        "https://:@:",
        "a:b:c",
        "\u{FFFF}",
        "http://%",
        "http://\u{FF0F}",
    ] {
        let _ = UrlString::parse(text);
    }
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let alphabet: Vec<char> = "ab:/@[]%?#\\ .\u{e9}\u{4e2d}0".chars().collect();
    for _ in 0..5000 {
        let mut s = String::new();
        for _ in 0..(seed % 24) {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            s.push(alphabet[(seed >> 33) as usize % alphabet.len()]);
        }
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        if let Some(url) = UrlString::parse(&s) {
            let _ = (url.scheme(), url.host(), url.parsed());
        }
    }
}
