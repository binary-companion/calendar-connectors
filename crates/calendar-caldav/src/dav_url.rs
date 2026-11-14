//! The few `URL` operations the CalDAV code uses, with Foundation's results (probed on macOS): relative resolution that
//! keeps the text of the base, `path` and `lastPathComponent` (decoded; no trailing slash), and
//! `appendingPathComponent`. Not a Swift file: the Swift code calls Foundation, and `UrlString` (which keeps the
//! original text) has no resolution of its own.

use calendar_core::url_string::UrlString;

pub(crate) enum DavUrl {}

struct Parts<'a> {
    scheme: Option<&'a str>,
    authority: Option<&'a str>,
    path: &'a str,
    query: Option<&'a str>,
    fragment: Option<&'a str>,
}

fn split(text: &str) -> Parts<'_> {
    let (rest, fragment) = match text.split_once('#') {
        Some((rest, fragment)) => (rest, Some(fragment)),
        None => (text, None),
    };
    let (rest, query) = match rest.split_once('?') {
        Some((rest, query)) => (rest, Some(query)),
        None => (rest, None),
    };
    let mut scheme = None;
    let mut remainder = rest;
    if let Some(colon) = rest.find(':') {
        let candidate = &rest[..colon];
        let valid = candidate
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
            && candidate
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c));
        if valid {
            scheme = Some(candidate);
            remainder = &rest[colon + 1..];
        }
    }
    let (authority, path) = match remainder.strip_prefix("//") {
        Some(after) => {
            let end = after.find('/').unwrap_or(after.len());
            (Some(&after[..end]), &after[end..])
        }
        None => (None, remainder),
    };
    Parts {
        scheme,
        authority,
        path,
        query,
        fragment,
    }
}

fn compose(
    scheme: Option<&str>,
    authority: Option<&str>,
    path: &str,
    query: Option<&str>,
    fragment: Option<&str>,
) -> String {
    let mut text = String::new();
    if let Some(scheme) = scheme {
        text.push_str(scheme);
        text.push(':');
    }
    if let Some(authority) = authority {
        text.push_str("//");
        text.push_str(authority);
    }
    text.push_str(path);
    if let Some(query) = query {
        text.push('?');
        text.push_str(query);
    }
    if let Some(fragment) = fragment {
        text.push('#');
        text.push_str(fragment);
    }
    text
}

fn is_hex(byte: u8) -> bool {
    byte.is_ascii_hexdigit()
}

/// Escapes what a URL cannot hold as written (space, quotes, non-ASCII, a `%` not followed by two hex digits), as
/// Foundation does when it parses a reference.
fn escape_illegal(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    for (i, &byte) in bytes.iter().enumerate() {
        let keep = match byte {
            b'%' => i + 2 < bytes.len() && is_hex(bytes[i + 1]) && is_hex(bytes[i + 2]),
            b if b.is_ascii_alphanumeric() => true,
            b'-' | b'.' | b'_' | b'~' | b':' | b'/' | b'?' | b'#' | b'[' | b']' | b'@' | b'!'
            | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'=' => true,
            _ => false,
        };
        if keep {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() && is_hex(bytes[i + 1]) && is_hex(bytes[i + 2]) {
            let value = u8::from_str_radix(&text[i + 1..i + 3], 16).unwrap_or(b'%');
            out.push(value);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// RFC 3986 section 5.2.4.
fn remove_dot_segments(path: &str) -> String {
    let mut output: Vec<&str> = Vec::new();
    let absolute = path.starts_with('/');
    let segments: Vec<&str> = path.split('/').collect();
    let last = segments.len().saturating_sub(1);
    for (index, segment) in segments.iter().enumerate() {
        if index == 0 && absolute {
            continue;
        }
        match *segment {
            "." => {
                if index == last {
                    output.push("");
                }
            }
            ".." => {
                output.pop();
                if index == last {
                    output.push("");
                }
            }
            other => output.push(other),
        }
    }
    let joined = output.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

impl DavUrl {
    /// `URL(string: reference, relativeTo: base)?.absoluteURL`: `None` where Foundation returns nil (an empty or
    /// invalid reference, or a base without a scheme). The base's text (host case, port) is kept.
    pub(crate) fn resolve(reference: &str, base: &UrlString) -> Option<UrlString> {
        let escaped = escape_illegal(reference);
        let r = UrlString::parse(&escaped)?;
        let r = split(r.as_str());
        let b = split(base.as_str());
        let base_scheme = b.scheme?;
        let text = if r.scheme.is_some() {
            compose(r.scheme, r.authority, r.path, r.query, r.fragment)
        } else if r.authority.is_some() {
            compose(
                Some(base_scheme),
                r.authority,
                &remove_dot_segments(r.path),
                r.query,
                r.fragment,
            )
        } else if r.path.is_empty() {
            compose(
                Some(base_scheme),
                b.authority,
                b.path,
                r.query.or(b.query),
                r.fragment,
            )
        } else {
            let merged = if r.path.starts_with('/') {
                r.path.to_string()
            } else if b.authority.is_some() && b.path.is_empty() {
                format!("/{}", r.path)
            } else {
                match b.path.rfind('/') {
                    Some(slash) => format!("{}{}", &b.path[..=slash], r.path),
                    None => r.path.to_string(),
                }
            };
            compose(
                Some(base_scheme),
                b.authority,
                &remove_dot_segments(&merged),
                r.query,
                r.fragment,
            )
        };
        UrlString::parse(&text)
    }

    /// `URL.path`: decoded, without a trailing slash (the root stays `/`).
    pub(crate) fn path(url: &UrlString) -> String {
        let raw = split(url.as_str()).path;
        let trimmed = if raw.len() > 1 {
            raw.trim_end_matches('/')
        } else {
            raw
        };
        let trimmed = if trimmed.is_empty() && raw.starts_with('/') {
            "/"
        } else {
            trimmed
        };
        percent_decode(trimmed)
    }

    /// `URLComponents.percentEncodedPath.removingPercentEncoding`: decoded, trailing slash kept.
    #[cfg(test)]
    pub(crate) fn raw_decoded_path(url: &UrlString) -> String {
        percent_decode(split(url.as_str()).path)
    }

    /// `URL.lastPathComponent`: the last component of the encoded path (trailing slashes ignored), decoded; `/` for
    /// the root and empty for no path.
    pub(crate) fn last_path_component(url: &UrlString) -> String {
        let raw = split(url.as_str()).path;
        if raw.is_empty() {
            return String::new();
        }
        let trimmed = raw.trim_end_matches('/');
        if trimmed.is_empty() {
            return "/".to_string();
        }
        percent_decode(trimmed.rsplit('/').next().unwrap_or(trimmed))
    }

    /// `URL.appendingPathComponent(_:isDirectory:)`: the component is percent-encoded, joined with one slash, and the
    /// query and fragment stay at the end.
    pub(crate) fn appending_path_component(
        url: &UrlString,
        component: &str,
        is_directory: bool,
    ) -> UrlString {
        let parts = split(url.as_str());
        let mut path = parts.path.to_string();
        if !path.ends_with('/') {
            path.push('/');
        }
        for &byte in component.as_bytes() {
            let keep = byte.is_ascii_alphanumeric() || b"-._~!$&'()*+,/:;=@".contains(&byte);
            if keep {
                path.push(byte as char);
            } else {
                path.push_str(&format!("%{byte:02X}"));
            }
        }
        if is_directory && !path.ends_with('/') {
            path.push('/');
        }
        let text = compose(
            parts.scheme,
            parts.authority,
            &path,
            parts.query,
            parts.fragment,
        );
        UrlString::parse(&text).unwrap_or_else(|| url.clone())
    }
}
