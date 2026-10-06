import CalendarCore
import Foundation

/// The link a user pasted, checked and normalised. A feed link is a credential, so nothing here puts it in a message.
enum FeedLocation {
    static let invalidMessage = "enter an iCal link starting with https:// or webcal://"
    private static let loopbackHosts: Set<String> = ["localhost", "127.0.0.1", "::1", "[::1]"]

    /// `webcal://` and `webcals://` become `https://`; `https://` is kept; `http://` is allowed only for the loopback
    /// host (a local test server). Anything else, a link with a user name or password included, is refused.
    static func url(from text: String) throws -> URL {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard var components = URLComponents(string: trimmed), let scheme = components.scheme?.lowercased(),
              let host = components.host?.lowercased(), !host.isEmpty else {
            throw SourceError.invalidResponse(invalidMessage)
        }
        guard components.user == nil, components.password == nil else {
            throw SourceError.invalidResponse("enter the link without a user name or password")
        }
        switch scheme {
        case "webcal", "webcals", "https": components.scheme = "https"
        case "http" where loopbackHosts.contains(host): break
        default: throw SourceError.invalidResponse(invalidMessage)
        }
        guard let url = components.url else { throw SourceError.invalidResponse(invalidMessage) }
        return url
    }
}
