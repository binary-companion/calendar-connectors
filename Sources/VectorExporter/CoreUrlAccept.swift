import Foundation

// core/url-accept.json
//
// Entry point: URL(string:) with absoluteString, host and scheme (Foundation's URL, the type the library parses
// every link with; the Rust UrlString::parse accepts exactly what it accepts).
//
//   input:    { "string": "<text>" }
//   expected: { "accepted": <Bool>, "absoluteString": "<text>" | null, "host": "<text>" | null, "scheme": "<text>" | null }
//
// 300 strings: valid URLs, a missing scheme, spaces and other characters that must be escaped, non-ASCII hosts and
// paths, percent-encoding (valid and malformed), trailing dots, ports, user info, IPv4 and IPv6 literals,
// control characters, very long strings, and the schemes the library handles (http, https, zoommtg, file, data,
// javascript, mailto). "host" is URL.host (nil when none) and "scheme" is URL.scheme. The values are
// the platform's: the file depends on the Foundation of the macOS that ran the exporter.

func exportCoreUrlAccept() -> [VectorCase] {
    var rng = LCG(state: 0x0021_ACC3_0008)
    var strings: [String] = [
        "https://example.com", "https://example.com/", "http://example.com/a/b?c=d#e", "HTTPS://EXAMPLE.COM/Path", "https://Example.COM:8443/x", "http://example.com:80/",
        "https://example.com./", "https://example.com../", "https://.example.com/", "https://exa mple.com/", "https://example.com/a b", "https://example.com/a%20b",
        "https://example.com/a%2", "https://example.com/a%zz", "https://example.com/%", "https://example.com/?q=a b", "https://example.com/?q=%E2%9C%93",
        "https://example.com/\u{00E9}", "https://example.com/\u{4E2D}\u{6587}", "https://example.com/?q=\u{00E9}", "https://example.com/#\u{00E9}",
        "https://m\u{00FC}nchen.de/", "https://\u{4E2D}\u{6587}.example/", "https://xn--mnchen-3ya.de/", "https://\u{1F642}.example/", "https://EXAMPLE.com\u{00E9}/",
        "example.com", "www.example.com/path", "//example.com/x", "/relative/path", "relative", "", " ", "https://", "https:", "https:/", "https:/example.com", "https:example.com",
        "http://", "http:///x", "https:///missing-host", "://example.com", "1http://example.com", "ht tp://example.com", "h+t.t-p://example.com",
        "mailto:a@example.test", "tel:+15555550100", "javascript:alert(1)", "data:text/html,hello", "file:///tmp/payload", "file://localhost/tmp/x", "custom://example.com/run",
        "zoommtg://zoom.us/join?confno=123", "zoommtg://acme.zoom.us/join", "ZOOMMTG://ZOOM.US/join", "zoommtg:///join", "ftp://example.com/x",
        "https://user:pass@example.com/", "https://user@example.com/", "https://:pass@example.com/", "https://user:@example.com/", "https://us er@example.com/",
        "https://127.0.0.1/", "https://127.0.0.1:99999/", "https://256.256.256.256/", "https://[::1]/", "https://[::1]:8080/x", "https://[2001:db8::1]/", "https://[not-ipv6]/",
        "https://example.com:/", "https://example.com:abc/", "https://example.com:-1/", "https://example.com:0/", "https://example.com:65536/",
        "https://example.com/\t", "https://example.com/\n", "https://example.com/\u{0000}", "https://exa\tmple.com/", "\thttps://example.com/", "https://example.com/ ",
        "https://example.com/a\\b", "https:\\\\example.com\\x", "https://example.com/a|b", "https://example.com/a^b", "https://example.com/{x}", "https://example.com/<x>",
        "https://example.com/\"x\"", "https://example.com/a`b", "https://exa_mple.com/", "https://exa-mple.com/", "https://-example.com/", "https://example-.com/",
        "https://example.com?x=1", "https://example.com#frag", "https://example.com/#", "https://example.com/?", "https://example.com/??a?b", "https://example.com/##a",
        "https://example.com/a/../b", "https://example.com/./b", "https://example.com//b", "https://example.com/%2e%2e/b", "https://example.com/a;b=c;d", "https://x.test:8443/a;b",
        "https://acme.zoom.us/j/123456789?pwd=abc", "https://meet.google.com/abc-defg-hij", "https://teams.microsoft.com/l/meetup-join/19%3ameeting?a=1&b=2",
        "https://www.google.com/url?q=https://acme.zoom.us/j/1&sa=D", "https://nam02.safelinks.protection.outlook.com/?url=https%3A%2F%2Fteams.microsoft.com%2Fl%2Fmeetup-join%2F0&data=05",
        "https://example.com/" + String(repeating: "a", count: 3000), "https://" + String(repeating: "a", count: 300) + ".example/", "https://example.com/?" + String(repeating: "q=1&", count: 200),
        "https://example.com/\u{FF0E}", "https://example\u{3002}com/", "https://example.com\u{FF0F}x", "https://\u{FF45}xample.com/", "https://example.com/\u{202E}x", "https://example.com/\u{200B}",
        "http://0x7f.1/", "http://2130706433/", "http://example.com:80:80/", "http://[::1", "http://::1/", "http://exa[mple.com/", "http://exa]mple.com/",
        "geo:37.786971,-122.399677", "sms:+15555550100", "x-apple.systempreferences:", "webcal://example.com/cal.ics", "webcals://example.com/cal.ics", "HTTP://EXAMPLE.COM",
    ]
    // Generated: a scheme, a host and a path, each from a pool with a few oddities.
    let schemes = ["http", "https", "HTTPS", "zoommtg", "webcal", "x-custom", "h", "1x", "a+b", "a.b"]
    let hosts = ["example.com", "Example.COM", "a.b.example.com", "example.com.", "exa mple.com", "m\u{00FC}nchen.example", "xn--mnchen-3ya.example", "127.0.0.1", "[::1]",
                 "example.com:8080", "u:p@example.com", "", "-bad-.example", "ex%41mple.com", "ex%zzmple.com", "example.com%2F"]
    let paths = ["", "/", "/a", "/a/b/", "/a b", "/a%20b", "/a%2", "/\u{00E9}", "/\u{1F642}", "/a?b=c", "/a?b=c d", "/a#f", "/a#f g", "/%E4%B8%AD", "/..", "/a/./b", "/;x=1", "/a\\b"]
    while strings.count < 300 {
        strings.append("\(rng.pick(schemes))://\(rng.pick(hosts))\(rng.pick(paths))")
    }
    return strings.enumerated().map { index, text in
        let url = URL(string: text)
        return VectorCase(
            name: "url-accept-\(fourDigits(index + 1))", input: ["string": text],
            expected: ["accepted": url != nil, "absoluteString": orNull(url?.absoluteString), "host": orNull(url?.host), "scheme": orNull(url?.scheme)])
    }
}
