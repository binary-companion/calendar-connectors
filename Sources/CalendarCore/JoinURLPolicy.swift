import Foundation

/// Schemes the app may open from calendar-supplied meeting data.
public enum JoinURLPolicy {
    public static func isAllowed(_ url: URL) -> Bool {
        guard let scheme = url.scheme?.lowercased(),
              let host = url.host?.lowercased(), !host.isEmpty,
              url.user == nil, url.password == nil else { return false }
        switch scheme {
        case "http", "https": return true
        case "zoommtg":
            return host == "zoom.us" || host == "zoom.com" ||
                host.hasSuffix(".zoom.us") || host.hasSuffix(".zoom.com")
        default: return false
        }
    }
}
