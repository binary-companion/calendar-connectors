//! Not a port: Foundation's `URLComponents` behaviour that `FeedLocation` depends on and the Swift tests do not
//! pin, recorded by running the Swift `FeedLocation.url(from:)` (and `URL(string:relativeTo:)` for redirect
//! targets) on macOS with `swiftc` on 2026-11-10. `None` is a refusal. The same table also covers the
//! percent-encoding rules (a `%` that is not part of a valid escape is encoded, for every `%` of that component),
//! internationalised host names and the resource-name encoding of `FeedParser`.

use calendar_core::url_string::UrlString;

use crate::feed_fetcher::FeedFetcher;
use crate::feed_location::FeedLocation;
use crate::feed_parser::FeedParser;

const LOCATION_CASES: &[(&str, Option<&str>)] = &[
    (
        "https://example.test/feed with space",
        Some("https://example.test/feed%20with%20space"),
    ),
    (
        "HTTPS://Example.TEST/Feed?a=b c",
        Some("https://Example.TEST/Feed?a=b%20c"),
    ),
    (
        "WebCal://Example.test:8443/x#frag",
        Some("https://Example.test:8443/x#frag"),
    ),
    (
        "https://example.test/é/ü?x=é",
        Some("https://example.test/%C3%A9/%C3%BC?x=%C3%A9"),
    ),
    (
        "https://exämple.test/a",
        Some("https://xn--exmple-cua.test/a"),
    ),
    (
        "https://ÉXAMPLE.test/a",
        Some("https://xn--xample-9ua.test/a"),
    ),
    (
        "https://exämple.test:8080/ä?ö#ü",
        Some("https://xn--exmple-cua.test:8080/%C3%A4?%C3%B6#%C3%BC"),
    ),
    ("https:///x", None),
    ("https://", None),
    ("http://[::1]:8008/f", Some("http://[::1]:8008/f")),
    ("HTTP://LOCALHOST/x", Some("HTTP://LOCALHOST/x")),
    ("http://localhost./", None),
    ("https://me@example.test/", None),
    ("https://:pw@example.test/", None),
    (
        "https://example.test/%zz",
        Some("https://example.test/%25zz"),
    ),
    (
        "https://example.test/a%20b?q=%41",
        Some("https://example.test/a%20b?q=%41"),
    ),
    ("https://example.test", Some("https://example.test")),
    ("https://example.test?x=1", Some("https://example.test?x=1")),
    (
        "webcal://example.test/a b",
        Some("https://example.test/a%20b"),
    ),
    ("https://exa mple.test/", None),
    (
        "https://example.test:99999/x",
        Some("https://example.test:99999/x"),
    ),
    ("https://example.test:abc/x", None),
    ("https://[::1]/x", Some("https://[::1]/x")),
    (
        "http://127.0.0.1:8008/feed.ics",
        Some("http://127.0.0.1:8008/feed.ics"),
    ),
    (
        "https://example.test/a\u{0}b",
        Some("https://example.test/a%00b"),
    ),
    (
        "https://example.test/a|b",
        Some("https://example.test/a%7Cb"),
    ),
    (
        "https://example.test/a{b}",
        Some("https://example.test/a%7Bb%7D"),
    ),
    ("//example.test/a", None),
    ("https:example.test/a", None),
    ("https://example.test/#", Some("https://example.test/#")),
    (
        "https://example.test/%41%zz",
        Some("https://example.test/%2541%25zz"),
    ),
    (
        "https://example.test/%zz%41",
        Some("https://example.test/%25zz%2541"),
    ),
    (
        "https://example.test/a%41?q=%zz",
        Some("https://example.test/a%41?q=%25zz"),
    ),
    ("https://example.test/%4", Some("https://example.test/%254")),
    (
        "https://example.test/%4/%41",
        Some("https://example.test/%254/%2541"),
    ),
    (
        "https://example.test/%41/%4",
        Some("https://example.test/%2541/%254"),
    ),
    (
        "https://example.test/%%",
        Some("https://example.test/%25%25"),
    ),
    (
        "https://example.test/%25zz",
        Some("https://example.test/%25zz"),
    ),
    ("https://example.test/a%", Some("https://example.test/a%25")),
    ("https://example.test/%a", Some("https://example.test/%25a")),
    (
        "https://Example.test:443/a",
        Some("https://Example.test:443/a"),
    ),
    ("https://example.test:/a", Some("https://example.test:/a")),
    ("https://EXAMPLE.test./a", Some("https://EXAMPLE.test./a")),
    ("https://[::1]:80/a", Some("https://[::1]:80/a")),
    ("https://[::1/a", None),
    ("https://ex%41mple.test/a", Some("https://ex%41mple.test/a")),
    ("https://ex%zzmple.test/a", None),
    (
        "https://example.test/a?b#c#d",
        Some("https://example.test/a?b#c%23d"),
    ),
    (
        "https://example_test.org/a",
        Some("https://example_test.org/a"),
    ),
    ("https://exa*mple.test/", Some("https://exa*mple.test/")),
    ("https://a,b.test/", Some("https://a,b.test/")),
    ("https://1.2.3/", Some("https://1.2.3/")),
    ("https://example.test\\a", None),
    (
        "https://example.test/a\\b",
        Some("https://example.test/a%5Cb"),
    ),
    (
        "https://example.test/\u{7F}",
        Some("https://example.test/%7F"),
    ),
    (
        "https://example.test/\u{85}x",
        Some("https://example.test/%C2%85x"),
    ),
    (
        "https://example.test/😀",
        Some("https://example.test/%F0%9F%98%80"),
    ),
    (
        "https://example.test/e\u{301}",
        Some("https://example.test/e%CC%81"),
    ),
    (
        "https://example.test/a<b>c\"d^e`f[g]h",
        Some("https://example.test/a%3Cb%3Ec%22d%5Ee%60f%5Bg%5Dh"),
    ),
    (
        "https://example.test/?a<b>c\"d^e`f[g]h",
        Some("https://example.test/?a%3Cb%3Ec%22d%5Ee%60f%5Bg%5Dh"),
    ),
];

#[test]
fn feed_location_matches_foundation_on_what_the_swift_tests_do_not_pin() {
    let mut failures = Vec::new();
    for (input, expected) in LOCATION_CASES {
        let got = FeedLocation::url_from(input)
            .ok()
            .map(|url| url.as_str().to_string());
        if got.as_deref() != *expected {
            failures.push(format!("{input:?}: expected {expected:?}, got {got:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

const BASE: &str = "https://www.example.test/events/ical/42/PRIV/going";

const REDIRECT_CASES: &[(&str, Option<&str>)] = &[
    (
        "https://other.example.test/feed.ics",
        Some("https://other.example.test/feed.ics"),
    ),
    ("/relative", Some("https://www.example.test/relative")),
    (
        "relative",
        Some("https://www.example.test/events/ical/42/PRIV/relative"),
    ),
    (
        "../x?y=1",
        Some("https://www.example.test/events/ical/42/x?y=1"),
    ),
    ("//cdn.example.test/p", Some("https://cdn.example.test/p")),
    (
        "HTTPS://Other.Example.test/Feed",
        Some("HTTPS://Other.Example.test/Feed"),
    ),
    ("http://x/", Some("http://x/")),
    (
        "?q=1",
        Some("https://www.example.test/events/ical/42/PRIV/going?q=1"),
    ),
    (
        "#f",
        Some("https://www.example.test/events/ical/42/PRIV/going#f"),
    ),
    (
        "feed with space",
        Some("https://www.example.test/events/ical/42/PRIV/feed%20with%20space"),
    ),
    ("", None),
];

#[test]
fn redirect_targets_resolve_as_foundation_resolves_them() {
    let base = UrlString::parse(BASE).unwrap();
    let mut failures = Vec::new();
    for (location, expected) in REDIRECT_CASES {
        let got =
            FeedFetcher::resolve_redirect(location, &base).map(|url| url.as_str().to_string());
        if got.as_deref() != *expected {
            failures.push(format!("{location:?}: expected {expected:?}, got {got:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// `FeedParser.resourceName(for:)`: only ASCII letters and digits and `-._~@` stay; every other scalar is
/// percent-encoded as UTF-8 (probed: `addingPercentEncoding` encodes non-ASCII letters too).
#[test]
fn resource_names_encode_everything_but_ascii_alphanumerics_and_a_few_marks() {
    let cases = [
        ("a b#c/d@example.test", "a%20b%23c%2Fd@example.test"),
        ("é", "%C3%A9"),
        ("e\u{301}", "e%CC%81"),
        ("日本語", "%E6%97%A5%E6%9C%AC%E8%AA%9E"),
        ("😀", "%F0%9F%98%80"),
        ("a\u{0}b", "a%00b"),
        ("plain-._~@09AZaz", "plain-._~@09AZaz"),
    ];
    for (uid, expected) in cases {
        assert_eq!(FeedParser::resource_name(uid), expected, "{uid:?}");
    }
}

/// `FeedParser.fnv1a64`: the 64-bit FNV-1a of the UTF-8 bytes, as lower-case hex without padding.
#[test]
fn the_synthetic_uid_hash_is_fnv_1a() {
    assert_eq!(FeedParser::fnv1a_hex(""), "cbf29ce484222325");
    assert_eq!(FeedParser::fnv1a_hex("a"), "af63dc4c8601ec8c");
    assert_eq!(FeedParser::fnv1a_hex("foobar"), "85944171f73967e8");
    assert_eq!(FeedParser::fnv1a_hex("\u{0}"), "af63bd4c8601b7df");
    assert_eq!(FeedParser::fnv1a_hex("é"), "ac21707b7181e01");
    assert_eq!(
        FeedParser::fnv1a_hex("20260912T100000|No uid"),
        "e30cd19b4a62733d"
    );
}
