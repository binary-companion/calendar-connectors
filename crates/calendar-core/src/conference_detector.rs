//! Finds video-conference join links in event text. Port of `ConferenceDetector.swift`.

use std::collections::BTreeSet;

use crate::join_url_policy::JoinUrlPolicy;
use crate::model::{ConferenceInfo, ConferenceOrigin, ConferenceProvider};
use crate::url_string::UrlString;

/// Finds video-conference join links in event text, and is the one place that knows which hosts are conference
/// providers. Uses a known-provider allowlist so a "Join" button never opens something like a shared document.
pub enum ConferenceDetector {}

struct Provider {
    host: &'static str,
    path_prefix: Option<&'static str>,
    provider: ConferenceProvider,
}

const fn provider(host: &'static str, provider: ConferenceProvider) -> Provider {
    Provider {
        host,
        path_prefix: None,
        provider,
    }
}

/// Data, not logic: add providers here.
static PROVIDERS: &[Provider] = &[
    provider("zoom.us", ConferenceProvider::Zoom),
    provider("zoom.com", ConferenceProvider::Zoom),
    provider("meet.google.com", ConferenceProvider::Meet),
    provider("teams.microsoft.com", ConferenceProvider::Teams),
    provider("teams.live.com", ConferenceProvider::Teams),
    provider("webex.com", ConferenceProvider::Webex),
    provider("gotomeet.me", ConferenceProvider::GoToMeeting),
    provider("gotomeeting.com", ConferenceProvider::GoToMeeting),
    provider("whereby.com", ConferenceProvider::Whereby),
    provider("meet.jit.si", ConferenceProvider::Jitsi),
    Provider {
        host: "app.slack.com",
        path_prefix: Some("/huddle"),
        provider: ConferenceProvider::Slack,
    },
];

/// Hosts that only ever link back to a calendar's own web view of the event (Google's `htmlLink`, synced
/// through both the direct connector and CalDAV), never a meeting join link, even though every Google event
/// carries one whether or not it has a real conference. Guards only the raw-`url` fallback below; a redirect
/// through one of these hosts to a real provider is still found by the text scan.
const NON_CONFERENCE_EVENT_LINK_HOSTS: &[&str] = &["www.google.com", "calendar.google.com"];

fn is_zoom_host(host: &str) -> bool {
    host == "zoom.us"
        || host == "zoom.com"
        || host.ends_with(".zoom.us")
        || host.ends_with(".zoom.com")
}

impl ConferenceDetector {
    /// The provider a link belongs to; `None` when its host is not on the allowlist.
    pub fn provider_of(url: &UrlString) -> Option<ConferenceProvider> {
        if !JoinUrlPolicy::is_allowed(url) {
            return None;
        }
        let scheme = url.scheme()?.to_lowercase();
        if scheme == "zoommtg" {
            let host = url.host()?.to_lowercase();
            return is_zoom_host(&host).then_some(ConferenceProvider::Zoom);
        }
        if scheme != "http" && scheme != "https" {
            return None;
        }
        let host = url.host()?.to_lowercase();
        let path = url.path();
        PROVIDERS
            .iter()
            .find(|entry| {
                (host == entry.host || host.ends_with(&format!(".{}", entry.host)))
                    && entry.path_prefix.is_none_or(|prefix| {
                        path == prefix || path.starts_with(&format!("{prefix}/"))
                    })
            })
            .map(|entry| entry.provider)
    }

    /// Every conference link of an event, most likely first:
    /// 1. `structured` links, in the order the caller gives them;
    /// 2. every allowlisted link in `location`, then `url`, then `notes`, in order of appearance;
    /// 3. duplicates removed by `identity` (the first wins);
    /// 4. only if that leaves nothing: the event's own web `url` (not a calendar permalink), as `EventUrl` with
    ///    provider `Other` (the invite put it there on purpose).
    pub fn conferences(
        structured: &[ConferenceInfo],
        location: Option<&str>,
        url: Option<&UrlString>,
        notes: Option<&str>,
    ) -> Vec<ConferenceInfo> {
        let mut result: Vec<ConferenceInfo> = Vec::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut add = |info: ConferenceInfo| {
            if !JoinUrlPolicy::is_allowed(&info.url) {
                return;
            }
            if seen.insert(info.identity()) {
                result.push(info);
            }
        };
        for info in structured {
            add(info.clone());
        }
        let texts = [
            (location, ConferenceOrigin::Location),
            (url.map(UrlString::as_str), ConferenceOrigin::Url),
            (notes, ConferenceOrigin::Notes),
        ];
        for (text, origin) in texts {
            let Some(text) = text else { continue };
            for candidate in ConferenceDetector::candidates_in(text) {
                if let Some(link) = ConferenceDetector::unwrap(&candidate)
                    && let Some(provider) = ConferenceDetector::provider_of(&link)
                {
                    add(ConferenceInfo::new(link, provider, origin));
                }
            }
        }
        if result.is_empty()
            && let Some(url) = url
        {
            let scheme = url.scheme().map(|s| s.to_lowercase());
            let host = url.host().map(|h| h.to_lowercase());
            if JoinUrlPolicy::is_allowed(url)
                && matches!(scheme.as_deref(), Some("http" | "https"))
                && host
                    .as_deref()
                    .is_some_and(|h| !NON_CONFERENCE_EVENT_LINK_HOSTS.contains(&h))
            {
                result.push(ConferenceInfo::new(
                    url.clone(),
                    ConferenceProvider::Other,
                    ConferenceOrigin::EventUrl,
                ));
            }
        }
        result
    }

    /// The first link `conferences(...)` finds, or `None`.
    pub fn detect(
        location: Option<&str>,
        url: Option<&UrlString>,
        notes: Option<&str>,
    ) -> Option<UrlString> {
        ConferenceDetector::conferences(&[], location, url, notes)
            .into_iter()
            .next()
            .map(|info| info.url)
    }

    /// The links in `text`, matched as `(?:https?|zoommtg)://[^\s<>"'\)\]]+` (case-insensitive), after `&amp;` is
    /// replaced by `&`, with `. , ; : ! ?` trimmed from both ends. A hand-written scanner replaces the regular
    /// expression.
    pub(crate) fn candidates_in(text: &str) -> Vec<String> {
        let decoded = text.replace("&amp;", "&");
        let mut candidates = Vec::new();
        let mut position = 0;
        while position < decoded.len() {
            match match_link(&decoded, position) {
                Some(end) => {
                    candidates.push(
                        decoded[position..end]
                            .trim_matches(['.', ',', ';', ':', '!', '?'])
                            .to_string(),
                    );
                    position = end;
                }
                None => {
                    position += decoded[position..].chars().next().map_or(1, char::len_utf8);
                }
            }
        }
        candidates
    }

    /// Unwraps Outlook SafeLinks and Google redirect URLs (up to a few layers).
    pub(crate) fn unwrap(raw: &str) -> Option<UrlString> {
        let mut current = UrlString::parse(raw);
        for _ in 0..3 {
            let Some(url) = current.clone() else { break };
            let Some(host) = url.host().map(|h| h.to_lowercase()) else {
                break;
            };
            let inner = if host == "safelinks.protection.outlook.com"
                || host.ends_with(".safelinks.protection.outlook.com")
            {
                url.query_value("url")
            } else if host == "www.google.com" && url.path() == "/url" {
                url.query_value("q").or_else(|| url.query_value("url"))
            } else {
                break;
            };
            let Some(next) = inner.as_deref().and_then(UrlString::parse) else {
                break;
            };
            current = Some(next);
        }
        current
    }
}

/// The end of a link that starts at `start`, or `None` when none does.
fn match_link(text: &str, start: usize) -> Option<usize> {
    let rest = &text[start..];
    let after_scheme = ["https://", "http://", "zoommtg://"]
        .iter()
        .find_map(|scheme| {
            let candidate = rest.get(..scheme.len())?;
            candidate
                .eq_ignore_ascii_case(scheme)
                .then_some(scheme.len())
        })?;
    let body = &rest[after_scheme..];
    let length: usize = body
        .chars()
        .take_while(|c| !is_link_terminator(*c))
        .map(char::len_utf8)
        .sum();
    (length > 0).then_some(start + after_scheme + length)
}

/// The characters outside `[^\s<>"'\)\]]`: ICU's `\s` is `[\t\n\f\r\p{Z}]`.
fn is_link_terminator(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0c}' | '\r' | '<' | '>' | '"' | '\'' | ')' | ']'
    ) || matches!(
        c,
        '\u{20}' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
}
