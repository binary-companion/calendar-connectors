//! Port of `FeedLocation.swift`: the link a user pasted, checked and normalised. A feed link is a credential, so
//! nothing here puts it in a message.
//!
//! Swift reads the text with `URLComponents`, which (on current Foundation) percent-encodes the characters a path,
//! query or fragment may not hold and turns an international host name into its ASCII form. `url_from` reproduces
//! that by hand; the cases the Swift tests do not pin were probed on macOS and are in
//! `tests/feed_location_probe_tests.rs`.

use calendar_core::source_types::SourceError;
use calendar_core::url_string::UrlString;

/// The link a user pasted, checked and normalised.
pub enum FeedLocation {}

const LOOPBACK_HOSTS: [&str; 4] = ["localhost", "127.0.0.1", "::1", "[::1]"];

impl FeedLocation {
    pub const INVALID_MESSAGE: &'static str =
        "enter an iCal link starting with https:// or webcal://";

    /// `webcal://` and `webcals://` become `https://`; `https://` is kept; `http://` is allowed only for the loopback
    /// host (a local test server). Anything else, a link with a user name or password included, is refused.
    pub fn url_from(text: &str) -> Result<UrlString, SourceError> {
        let invalid = || SourceError::InvalidResponse(FeedLocation::INVALID_MESSAGE.to_string());
        let trimmed = text.trim();
        let parts = Parts::split(trimmed).ok_or_else(invalid)?;
        let candidate = parts.assemble(parts.scheme).ok_or_else(invalid)?;
        let url = UrlString::parse(&candidate).ok_or_else(invalid)?;
        let host = url.host().ok_or_else(invalid)?.to_lowercase();
        if host.is_empty() {
            return Err(invalid());
        }
        if parts.userinfo.is_some() {
            return Err(SourceError::InvalidResponse(
                "enter the link without a user name or password".to_string(),
            ));
        }
        let scheme = parts.scheme.to_lowercase();
        match scheme.as_str() {
            "webcal" | "webcals" | "https" => {
                let rewritten = parts.assemble("https").ok_or_else(invalid)?;
                UrlString::parse(&rewritten).ok_or_else(invalid)
            }
            "http" if LOOPBACK_HOSTS.contains(&host.as_str()) => Ok(url),
            _ => Err(invalid()),
        }
    }
}

/// The text of a link cut into its parts, not yet encoded.
struct Parts<'a> {
    scheme: &'a str,
    userinfo: Option<&'a str>,
    /// With the brackets of an IPv6 literal.
    host: &'a str,
    /// The text after the `:`, digits only; may be empty.
    port: Option<&'a str>,
    path: &'a str,
    query: Option<&'a str>,
    fragment: Option<&'a str>,
}

impl<'a> Parts<'a> {
    /// `None` when `URLComponents` would give no scheme or no host, or would refuse the text.
    fn split(text: &'a str) -> Option<Parts<'a>> {
        let colon = text.find(':')?;
        let scheme = &text[..colon];
        let mut chars = scheme.chars();
        if !chars.next()?.is_ascii_alphabetic()
            || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            return None;
        }
        let rest = text[colon + 1..].strip_prefix("//")?;
        let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(authority_end);
        let (tail, fragment) = match tail.split_once('#') {
            Some((before, fragment)) => (before, Some(fragment)),
            None => (tail, None),
        };
        let (path, query) = match tail.split_once('?') {
            Some((path, query)) => (path, Some(query)),
            None => (tail, None),
        };
        let (userinfo, host_port) = match authority.rfind('@') {
            Some(at) => (Some(&authority[..at]), &authority[at + 1..]),
            None => (None, authority),
        };
        let (host, port) = if host_port.starts_with('[') {
            let close = host_port.find(']')?;
            let after = &host_port[close + 1..];
            let port = match after.strip_prefix(':') {
                Some(port) => Some(port),
                None if after.is_empty() => None,
                None => return None,
            };
            (&host_port[..=close], port)
        } else {
            match host_port.rfind(':') {
                Some(at) => (&host_port[..at], Some(&host_port[at + 1..])),
                None => (host_port, None),
            }
        };
        if host.is_empty() || port.is_some_and(|p| !p.bytes().all(|b| b.is_ascii_digit())) {
            return None;
        }
        Some(Parts {
            scheme,
            userinfo,
            host,
            port,
            path,
            query,
            fragment,
        })
    }

    /// The link with `scheme`, the host in its ASCII form and path, query and fragment percent-encoded; `None` when
    /// the host cannot be converted.
    fn assemble(&self, scheme: &str) -> Option<String> {
        let mut out = String::new();
        out.push_str(scheme);
        out.push_str("://");
        if let Some(userinfo) = self.userinfo {
            out.push_str(userinfo);
            out.push('@');
        }
        if self.host.is_ascii() {
            out.push_str(self.host);
        } else {
            out.push_str(&ascii_host(self.host)?);
        }
        if let Some(port) = self.port {
            out.push(':');
            out.push_str(port);
        }
        out.push_str(&encode(self.path, false));
        if let Some(query) = self.query {
            out.push('?');
            out.push_str(&encode(query, false));
        }
        if let Some(fragment) = self.fragment {
            out.push('#');
            out.push_str(&encode(fragment, true));
        }
        Some(out)
    }
}

/// An international host name in its ASCII (punycode) form, as `url` converts it.
fn ascii_host(host: &str) -> Option<String> {
    let url = url::Url::parse(&format!("https://{host}/")).ok()?;
    url.host_str().map(str::to_string)
}

/// Percent-encodes what `URLComponents` encodes in a path, query or fragment: controls, space, `"<>[\]^`{|}` and
/// non-ASCII (as UTF-8), plus `#` in a fragment. A `%` stays only when every `%` of the component starts a valid
/// escape; otherwise every `%` of the component is encoded (probed).
fn encode(component: &str, is_fragment: bool) -> String {
    let bytes = component.as_bytes();
    let escapes_valid = bytes.iter().enumerate().all(|(i, &b)| {
        b != b'%'
            || (bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit))
    });
    let mut out = String::with_capacity(component.len());
    for &b in bytes {
        let keep = match b {
            b'%' => escapes_valid,
            b'#' => !is_fragment,
            0..=0x20
            | b'"'
            | b'<'
            | b'>'
            | b'['
            | b'\\'
            | b']'
            | b'^'
            | b'`'
            | b'{'
            | b'|'
            | b'}'
            | 0x7F..=0xFF => false,
            _ => true,
        };
        if keep {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
