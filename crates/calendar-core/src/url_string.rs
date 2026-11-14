//! `UrlString`: Swift's `URL` as the library uses it (conventions section 4). It keeps the original text, which
//! Foundation's `absoluteString` also does and `url::Url` (which normalizes) would not.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};

/// A link that Foundation's `URL(string:)` accepts. Accepted text is a URI reference whose syntax is checked as
/// Foundation checks it (`vectors/core/url-accept.json` is the pin): a scheme, if the text starts with one, must be
/// valid; the host must hold only characters a host can hold (spaces, brackets and bad percent-escapes are
/// refused); the port must be digits. Anything else, including characters that Foundation would escape in the path,
/// query or fragment, is accepted.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UrlString(String);

impl UrlString {
    /// `URL(string:)`: `None` where Foundation returns nil.
    pub fn parse(text: &str) -> Option<UrlString> {
        let parts = split(text)?;
        if let Some(authority) = parts.authority {
            let (_, host_port) = match authority.rfind('@') {
                Some(at) => (&authority[..at], &authority[at + 1..]),
                None => ("", authority),
            };
            host_of(host_port)?;
        }
        Some(UrlString(text.to_string()))
    }

    /// The text this link was made from, unchanged.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The link as `url::Url` reads it (normalized); `None` for a relative reference or text `url` refuses.
    pub fn parsed(&self) -> Option<url::Url> {
        url::Url::parse(&self.0).ok()
    }

    /// `URL.scheme`: the scheme exactly as written (`HTTPS` stays `HTTPS`; call sites that compare lowercase
    /// lowercase it themselves, as the Swift code does). `None` when the link has none.
    pub fn scheme(&self) -> Option<String> {
        split(&self.0)?.scheme.map(str::to_string)
    }

    /// `URL.path`: the path, percent-decoded (empty when the link has none).
    pub fn path(&self) -> String {
        split(&self.0).map_or_else(String::new, |parts| percent_decode_lenient(parts.path))
    }

    /// `URL.query`: the query as written (still percent-encoded); `None` when the link has no `?`.
    pub fn query(&self) -> Option<String> {
        split(&self.0)?.query.map(str::to_string)
    }

    /// `URLComponents.queryItems`: the query split at `&` into names and percent-decoded values (`None` as the
    /// value of an item without `=`). Empty when the link has no query.
    pub fn query_items(&self) -> Vec<(String, Option<String>)> {
        let Some(query) = split(&self.0).and_then(|parts| parts.query) else {
            return Vec::new();
        };
        query
            .split('&')
            .map(|item| match item.split_once('=') {
                Some((name, value)) => (
                    percent_decode_lenient(name),
                    Some(percent_decode_lenient(value)),
                ),
                None => (percent_decode_lenient(item), None),
            })
            .collect()
    }

    /// The value of the first query item called `name` (compared as written); `None` when there is no such item or
    /// it has no value. Matches `queryItems?.first { $0.name == name }?.value`.
    pub fn query_value(&self, name: &str) -> Option<String> {
        self.query_items()
            .into_iter()
            .find(|(item, _)| item == name)
            .and_then(|(_, value)| value)
    }

    /// `URL.user`: the percent-decoded user name; `None` when the link has no user info.
    pub fn user(&self) -> Option<String> {
        let userinfo = split(&self.0)?.userinfo?;
        let user = userinfo.split_once(':').map_or(userinfo, |(user, _)| user);
        Some(percent_decode_lenient(user))
    }

    /// `URL.password`: the percent-decoded password; `None` when the user info has none.
    pub fn password(&self) -> Option<String> {
        let userinfo = split(&self.0)?.userinfo?;
        userinfo
            .split_once(':')
            .map(|(_, password)| percent_decode_lenient(password))
    }

    /// `URL.host`: percent-decoded, Internationalized names in their ASCII (punycode) form, IPv6 literals without
    /// brackets; `None` when the link has no host or an empty one.
    pub fn host(&self) -> Option<String> {
        let parts = split(&self.0)?;
        let authority = parts.authority?;
        let host_port = authority
            .rfind('@')
            .map_or(authority, |at| &authority[at + 1..]);
        host_of(host_port).flatten()
    }
}

struct Parts<'a> {
    scheme: Option<&'a str>,
    /// The text between `//` and the next `/`, `?` or `#`.
    authority: Option<&'a str>,
    /// The user info part of the authority (before the last `@`).
    userinfo: Option<&'a str>,
    path: &'a str,
    query: Option<&'a str>,
}

/// Splits a URI reference per RFC 3986 section 3, refusing an invalid scheme.
fn split(text: &str) -> Option<Parts<'_>> {
    if text.is_empty() {
        return None;
    }
    let (scheme, rest) = match text.find([':', '/', '?', '#']) {
        Some(colon) if text.as_bytes()[colon] == b':' => {
            let candidate = &text[..colon];
            if !valid_scheme(candidate) {
                return None;
            }
            (Some(candidate), &text[colon + 1..])
        }
        _ => (None, text),
    };
    let (authority, after_authority) = match rest.strip_prefix("//") {
        Some(after) => {
            let end = after.find(['/', '?', '#']).unwrap_or(after.len());
            (Some(&after[..end]), &after[end..])
        }
        None => (None, rest),
    };
    let userinfo = authority.and_then(|a| a.rfind('@').map(|at| &a[..at]));
    let without_fragment = after_authority
        .find('#')
        .map_or(after_authority, |hash| &after_authority[..hash]);
    let (path, query) = match without_fragment.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (without_fragment, None),
    };
    Some(Parts {
        scheme,
        authority,
        userinfo,
        path,
        query,
    })
}

/// A scheme is a letter followed by letters, digits, `+`, `-` or `.`. Foundation also accepts an empty one
/// (`://example.com`).
fn valid_scheme(scheme: &str) -> bool {
    let mut chars = scheme.chars();
    match chars.next() {
        None => true,
        Some(first) => {
            first.is_ascii_alphabetic()
                && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        }
    }
}

/// Validates `host[:port]` and returns the host as `URL.host` reports it (`Some(None)`: valid, no host).
fn host_of(host_port: &str) -> Option<Option<String>> {
    let is_port = |p: &str| p.bytes().all(|b| b.is_ascii_digit());
    if let Some(inner) = host_port.strip_prefix('[') {
        let close = inner.find(']')?;
        let after = &inner[close + 1..];
        if !after.is_empty() && !is_port(after.strip_prefix(':')?) {
            return None;
        }
        // An IP literal is returned as written between the brackets, validated or not.
        let literal = &inner[..close];
        return Some((!literal.is_empty()).then(|| literal.to_string()));
    }
    let (raw_host, port) = host_port
        .split_once(':')
        .map_or((host_port, ""), |(h, p)| (h, p));
    if !is_port(port) {
        return None;
    }
    if raw_host.is_empty() {
        return Some(None);
    }
    let decoded = decode_host(raw_host)?;
    if decoded.is_ascii() {
        return Some(Some(decoded));
    }
    // A name with non-ASCII characters goes through IDNA to its ASCII form, or the whole link is refused.
    match url::Host::parse(&decoded).ok()? {
        url::Host::Domain(domain) => Some(Some(domain)),
        other => Some(Some(other.to_string())),
    }
}

/// Checks the characters of a host (unreserved, sub-delimiters, valid `%XX` escapes, non-ASCII) and percent-decodes it.
fn decode_host(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'%' => {
                let hi = hex(*bytes.get(i + 1)?)?;
                let lo = hex(*bytes.get(i + 2)?)?;
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
            0x80..=0xFF => {}
            b if b.is_ascii_alphanumeric() => {}
            b'-' | b'.' | b'_' | b'~' | b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+'
            | b',' | b';' | b'=' => {}
            _ => return None,
        }
        out.push(b);
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// Decodes every valid `%XX` escape; an invalid escape stays as written, and bytes that are not UTF-8 after decoding
/// leave the text unchanged (the nearest thing to Foundation keeping the raw component).
pub(crate) fn percent_decode_lenient(text: &str) -> String {
    percent_decode(text, false).unwrap_or_else(|| text.to_string())
}

/// `String.removingPercentEncoding`: `None` for a malformed escape or a result that is not UTF-8.
pub(crate) fn percent_decode_strict(text: &str) -> Option<String> {
    percent_decode(text, true)
}

fn percent_decode(text: &str, strict: bool) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let escaped = bytes
                .get(i + 1)
                .and_then(|h| hex(*h))
                .zip(bytes.get(i + 2).and_then(|l| hex(*l)));
            match escaped {
                Some((hi, lo)) => {
                    out.push(hi * 16 + lo);
                    i += 3;
                    continue;
                }
                None if strict => return None,
                None => {}
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

impl fmt::Debug for UrlString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for UrlString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A `URL` encodes as its string.
impl Serialize for UrlString {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

/// Decoding fails for text that `URL(string:)` would refuse, as Swift's `URL` decoding does.
impl<'de> Deserialize<'de> for UrlString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<UrlString, D::Error> {
        let text = String::deserialize(deserializer)?;
        UrlString::parse(&text).ok_or_else(|| D::Error::custom("invalid URL"))
    }
}
