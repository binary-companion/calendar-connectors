//! Tests with no Swift counterpart: conference detection and the URL helpers it uses never panic, whatever an event's
//! location, URL and notes hold. A panic here takes the host app down (the Swift binding of `detect_conferences` is
//! `try!`), so every shape of text a calendar can hold is tried, by hand and by a seeded generator.

use crate::UrlString;
use crate::conference_detector::ConferenceDetector;

/// Runs `text` through everything the detector does, as each of the three places it reads (location, URL, notes).
fn probe(text: &str) {
    let link = UrlString::parse(text);
    if let Some(link) = &link {
        let _ = (
            link.scheme(),
            link.host(),
            link.path(),
            link.query(),
            link.query_items(),
            link.query_value("url"),
            link.query_value("q"),
            link.user(),
            link.password(),
            link.parsed(),
        );
        let _ = ConferenceDetector::provider_of(link);
        let _ = ConferenceDetector::unwrap(text);
    }
    let _ = ConferenceDetector::conferences(&[], Some(text), link.as_ref(), Some(text));
    let _ = ConferenceDetector::conferences(&[], None, None, Some(text));
    let _ = ConferenceDetector::conferences(&[], Some(text), None, None);
}

#[test]
fn hand_picked_awkward_text_never_panics() {
    let long = "a".repeat(200_000);
    let many_schemes = "https://".repeat(5_000);
    let many_amps = "&amp;".repeat(5_000);
    let nested = "https://www.google.com/url?q=".repeat(50) + "https://zoom.us/j/1";
    let cases: Vec<&str> = vec![
        "",
        " ",
        "\0",
        "https://",
        "http://",
        "zoommtg://",
        "https:/",
        "https:",
        "HTTPS://ZOOM.US/J/1",
        "httpſ://zoom.us/j/1",
        "https://ſ",
        "https://zoom.us/j/1?pwd=%",
        "https://zoom.us/j/1?pwd=%E0%A4%A",
        "https://zoom.us/%",
        "https://zoom.us/%zz",
        "https://[::1",
        "https://[::1]:99999",
        "https://[]",
        "https://[",
        "https://]",
        "https://:80",
        "https://:",
        "https://@",
        "https://a@b@c/",
        "https://@@@/",
        "https://zoom.us:abc/",
        "https://zoom.us:/",
        "https://日本語.jp/x",
        "https://xn--/x",
        "https://xn--a.com",
        "https://\0",
        "https://exa mple",
        "https://\u{FFFD}.com",
        "https://a\u{200d}b.com",
        "https://☃.net",
        "https://\u{1F600}.com",
        "https://ab%00c.com",
        "https://%.com",
        "https://%E2%98%83.net/",
        "https://%FF.com",
        "https://a..b/",
        "https://./",
        "https://-/",
        "https://zoom.us/j/1\u{200b}",
        "https://zoom.us/j/1\u{2028}https://zoom.us/j/2",
        "https://zoom.us/j/1\u{e9}\u{301}",
        "\u{202e}https://zoom.us/j/1",
        "😀https://zoom.us/j/1😀",
        "e\u{301}e\u{301}https://zoom.us/j/1",
        "&amp;",
        "&amp;amp;",
        "https://zoom.us/j/1?a=1&amp;b=2",
        "https://safelinks.protection.outlook.com/?url=%25",
        "https://safelinks.protection.outlook.com/?url=https%3A%2F%2Fsafelinks.protection.outlook.com%2F%3Furl%3D%2525",
        "https://www.google.com/url?q=",
        "https://www.google.com/url?q=%",
        "https://www.google.com/url?url=https://www.google.com/url?url=https://zoom.us/j/1",
        "https://meet.google.com/\u{ffff}",
        "https://meet.google.com/abc-defg-hij.",
        "(https://meet.google.com/abc-defg-hij)",
        "<https://meet.google.com/abc-defg-hij>",
        "https://teams.microsoft.com/l/meetup-join/19%3ameeting_x%40thread.v2/0?context=%7b%22Tid%22%3a%22x%22%7d",
        "https://app.slack.com/huddle/T1/C1",
        "https://app.slack.com/huddle",
        "https://app.slack.com/",
        "mailto:a@b.c",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "data:text/html,<b>",
        "://example.com",
        "//example.com",
        "?",
        "#",
        "/",
        ":",
        "a:b:c:d",
        "https://zoom.us\t/j/1",
        "https://zoom.us\n/j/1",
        "http://\u{a0}",
        "http://\u{3000}x",
    ];
    for case in cases {
        probe(case);
    }
    for case in [long, many_schemes, many_amps, nested] {
        probe(&case);
    }
}

/// A small seeded generator (xorshift): the same corpus on every run, so a failure can be reproduced.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[(self.next() % items.len() as u64) as usize]
    }
}

#[test]
fn generated_text_never_panics() {
    let pieces: Vec<&str> = vec![
        "https://", "http://", "zoommtg://", "HTTPS://", "httpſ://", "zoom.us", "meet.google.com", "teams.microsoft.com",
        "safelinks.protection.outlook.com", "www.google.com", "app.slack.com", "/j/", "/huddle", "/url", "?", "&", "&amp;",
        "=", "url", "q", "%", "%2", "%25", "%E2%98", "%FF", "%00", "#", "@", ":", "::", "[", "]", "[::1]", ".", "..", "-",
        "/", "//", " ", "\n", "\t", "\u{a0}", "\u{2028}", "\u{3000}", "\u{200b}", "\u{200d}", "\u{301}", "\u{202e}", "\0",
        "<", ">", "\"", "'", "(", ")", ";", ",", "!", "日本", "é", "ſ", "😀", "xn--", "9", "123", "pwd", "a", "Z",
    ];
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..60_000 {
        let length = 1 + (rng.next() % 14) as usize;
        let mut text = String::new();
        for _ in 0..length {
            text.push_str(rng.pick(&pieces));
        }
        probe(&text);
    }
}
