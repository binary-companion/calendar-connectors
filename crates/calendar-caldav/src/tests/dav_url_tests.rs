//! New in the Rust port: `DavUrl` gives the results Foundation's `URL` gave in a probe on macOS (the Swift code calls
//! Foundation directly).

use calendar_core::url_string::UrlString;

use crate::dav_url::DavUrl;

fn url(text: &str) -> UrlString {
    UrlString::parse(text).unwrap()
}

#[test]
fn relative_references_resolve_like_foundation_and_keep_the_bases_text() {
    let base = url("https://P42-caldav.icloud.com:443/123/calendars/?x=1#f");
    for (reference, expected) in [
        (
            "/123/calendars/home/",
            Some("https://P42-caldav.icloud.com:443/123/calendars/home/"),
        ),
        (
            "home/a%20b.ics",
            Some("https://P42-caldav.icloud.com:443/123/calendars/home/a%20b.ics"),
        ),
        (
            "home/a b.ics",
            Some("https://P42-caldav.icloud.com:443/123/calendars/home/a%20b.ics"),
        ),
        ("../x/y", Some("https://P42-caldav.icloud.com:443/123/x/y")),
        (
            "./z",
            Some("https://P42-caldav.icloud.com:443/123/calendars/z"),
        ),
        ("//other.test/p", Some("https://other.test/p")),
        (
            "https://p07-caldav.icloud.com/123/principal/",
            Some("https://p07-caldav.icloud.com/123/principal/"),
        ),
        (
            "?q=2",
            Some("https://P42-caldav.icloud.com:443/123/calendars/?q=2"),
        ),
        (
            "#frag",
            Some("https://P42-caldav.icloud.com:443/123/calendars/?x=1#frag"),
        ),
        ("", None),
        ("/a/../b/./c", Some("https://P42-caldav.icloud.com:443/b/c")),
        (
            "/é/ü.ics",
            Some("https://P42-caldav.icloud.com:443/%C3%A9/%C3%BC.ics"),
        ),
        ("/a%2Fb", Some("https://P42-caldav.icloud.com:443/a%2Fb")),
        (
            "home/%zz",
            Some("https://P42-caldav.icloud.com:443/123/calendars/home/%25zz"),
        ),
    ] {
        let got = DavUrl::resolve(reference, &base);
        assert_eq!(
            got.as_ref().map(UrlString::as_str),
            expected,
            "{reference:?}"
        );
    }
}

#[test]
fn path_and_last_component_are_decoded_without_the_trailing_slash() {
    for (text, path, last) in [
        ("https://x.test/a/b/", "/a/b", "b"),
        ("https://x.test/a/b%20c.ics", "/a/b c.ics", "b c.ics"),
        ("https://x.test/", "/", "/"),
        ("https://x.test", "", ""),
        ("https://x.test/a//", "/a", "a"),
        (
            "https://x.test/%E2%82%AC.ics",
            "/\u{20AC}.ics",
            "\u{20AC}.ics",
        ),
        ("https://x.test/a%2Fb.ics", "/a/b.ics", "a/b.ics"),
    ] {
        assert_eq!(DavUrl::path(&url(text)), path, "{text}");
        assert_eq!(DavUrl::last_path_component(&url(text)), last, "{text}");
    }
}

#[test]
fn appending_a_path_component_encodes_it_and_keeps_the_query() {
    for (base, component, directory, expected) in [
        (
            "https://caldav.icloud.com",
            ".well-known/caldav",
            false,
            "https://caldav.icloud.com/.well-known/caldav",
        ),
        (
            "https://x.test/dav",
            ".well-known/caldav",
            false,
            "https://x.test/dav/.well-known/caldav",
        ),
        (
            "https://x.test/dav/?a=b#f",
            ".well-known/caldav",
            false,
            "https://x.test/dav/.well-known/caldav?a=b#f",
        ),
        ("https://x.test", "home", true, "https://x.test/home/"),
        (
            "https://x.test/123/calendars/",
            "my cal",
            true,
            "https://x.test/123/calendars/my%20cal/",
        ),
        (
            "https://x.test/123/calendars/",
            "100%",
            true,
            "https://x.test/123/calendars/100%25/",
        ),
        (
            "https://x.test/123/calendars/",
            "x?y",
            true,
            "https://x.test/123/calendars/x%3Fy/",
        ),
        (
            "https://x.test/123/calendars/",
            "a:b",
            false,
            "https://x.test/123/calendars/a:b",
        ),
    ] {
        assert_eq!(
            DavUrl::appending_path_component(&url(base), component, directory).as_str(),
            expected,
            "{base} + {component}"
        );
    }
}
