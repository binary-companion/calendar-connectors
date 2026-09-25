import Foundation

/// Finds video-conference join links in event text, and is the one place that knows which hosts are conference
/// providers. Uses a known-provider allowlist so a "Join" button never opens something like a shared document.
public enum ConferenceDetector {
    struct Provider {
        let host: String
        var pathPrefix: String? = nil
        let provider: ConferenceProvider
    }

    /// Data, not logic: add providers here.
    static let providers: [Provider] = [
        Provider(host: "zoom.us", provider: .zoom), Provider(host: "zoom.com", provider: .zoom),
        Provider(host: "meet.google.com", provider: .meet),
        Provider(host: "teams.microsoft.com", provider: .teams), Provider(host: "teams.live.com", provider: .teams),
        Provider(host: "webex.com", provider: .webex),
        Provider(host: "gotomeet.me", provider: .goToMeeting), Provider(host: "gotomeeting.com", provider: .goToMeeting),
        Provider(host: "whereby.com", provider: .whereby),
        Provider(host: "meet.jit.si", provider: .jitsi),
        Provider(host: "app.slack.com", pathPrefix: "/huddle", provider: .slack),
    ]

    /// Hosts that only ever link back to a calendar's own web view of the event (Google's `htmlLink`, synced
    /// through both the direct connector and CalDAV), never a meeting join link, even though every Google event
    /// carries one whether or not it has a real conference. Guards only the raw-`url` fallback below; a redirect
    /// through one of these hosts to a real provider is still found by the text scan.
    static let nonConferenceEventLinkHosts: Set<String> = ["www.google.com", "calendar.google.com"]

    /// The provider a link belongs to; nil when its host is not on the allowlist.
    public static func provider(of url: URL) -> ConferenceProvider? {
        guard let scheme = url.scheme?.lowercased() else { return nil }
        if scheme == "zoommtg" {
            guard let host = url.host?.lowercased() else { return nil }
            let isZoom = host == "zoom.us" || host == "zoom.com" || host.hasSuffix(".zoom.us") || host.hasSuffix(".zoom.com")
            return isZoom ? .zoom : nil
        }
        guard scheme == "http" || scheme == "https", let host = url.host?.lowercased() else { return nil }
        return providers.first { entry in
            (host == entry.host || host.hasSuffix("." + entry.host))
                && (entry.pathPrefix.map { url.path == $0 || url.path.hasPrefix($0 + "/") } ?? true)
        }?.provider
    }

    /// Every conference link of an event, most likely first:
    /// 1. `structured` links, in the order the caller gives them;
    /// 2. every allowlisted link in `location`, then `url`, then `notes`, in order of appearance;
    /// 3. duplicates removed by `identity` (the first wins);
    /// 4. only if that leaves nothing: the event's own web `url` (not a calendar permalink), as `.eventURL` with
    ///    provider `.other` (the invite put it there on purpose).
    public static func conferences(
        structured: [ConferenceInfo] = [], location: String?, url: URL?, notes: String?
    ) -> [ConferenceInfo] {
        var result: [ConferenceInfo] = []
        var seen = Set<String>()
        func add(_ info: ConferenceInfo) { if seen.insert(info.identity).inserted { result.append(info) } }
        structured.forEach(add)
        let texts: [(String?, ConferenceOrigin)] = [(location, .location), (url?.absoluteString, .url), (notes, .notes)]
        for (text, origin) in texts {
            guard let text else { continue }
            for candidate in candidates(in: text) {
                if let link = unwrap(candidate), let provider = provider(of: link) {
                    add(ConferenceInfo(url: link, provider: provider, origin: origin))
                }
            }
        }
        if result.isEmpty, let url, let scheme = url.scheme?.lowercased(), scheme == "http" || scheme == "https",
           let host = url.host?.lowercased(), !nonConferenceEventLinkHosts.contains(host)
        {
            result.append(ConferenceInfo(url: url, provider: .other, origin: .eventURL))
        }
        return result
    }

    /// The first link `conferences(...)` finds, or nil.
    public static func detect(location: String?, url: URL?, notes: String?) -> URL? {
        conferences(location: location, url: url, notes: notes).first?.url
    }

    static func candidates(in text: String) -> [String] {
        let decoded = text.replacingOccurrences(of: "&amp;", with: "&")
        guard let regex = try? NSRegularExpression(pattern: #"(?:https?|zoommtg)://[^\s<>"'\)\]]+"#, options: [.caseInsensitive]) else {
            return []
        }
        let range = NSRange(decoded.startIndex..., in: decoded)
        return regex.matches(in: decoded, range: range).compactMap { match in
            Range(match.range, in: decoded).map {
                String(decoded[$0]).trimmingCharacters(in: CharacterSet(charactersIn: ".,;:!?"))
            }
        }
    }

    /// Unwraps Outlook SafeLinks and Google redirect URLs (up to a few layers).
    static func unwrap(_ raw: String) -> URL? {
        var current = URL(string: raw)
        for _ in 0..<3 {
            guard let url = current, let host = url.host?.lowercased() else { break }
            let inner: String?
            if host == "safelinks.protection.outlook.com" || host.hasSuffix(".safelinks.protection.outlook.com") {
                inner = queryValue(url, "url")
            } else if host == "www.google.com", url.path == "/url" {
                inner = queryValue(url, "q") ?? queryValue(url, "url")
            } else {
                break
            }
            guard let inner, let next = URL(string: inner) else { break }
            current = next
        }
        return current
    }

    private static func queryValue(_ url: URL, _ name: String) -> String? {
        URLComponents(url: url, resolvingAgainstBaseURL: false)?
            .queryItems?.first { $0.name == name }?.value
    }
}
