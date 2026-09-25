import Foundation

/// Reads a calendar user address (an attendee's or organizer's `mailto:` URL or address) as an email. The one shared
/// parser, with no I/O, so every connector agrees. An address it cannot turn into an email (an opaque `urn:uuid:`
/// id, a CalDAV or iCloud principal URL, an Exchange directory path) is nil: the connector may still resolve it by
/// other means, but the library keeps no raw identifier.
public enum CalendarUserAddress {
    public static func email(from address: String?) -> String? {
        guard var text = address?.trimmingCharacters(in: .whitespacesAndNewlines), !text.isEmpty else { return nil }
        if text.lowercased().hasPrefix("mailto:") {
            text = String(text.dropFirst("mailto:".count))
            if let query = text.firstIndex(of: "?") { text = String(text[..<query]) }
            text = text.removingPercentEncoding ?? text
        } else if text.lowercased().hasPrefix("urn:") {
            // A URN whose last part is an email address: `urn:foo:bar:user@example.com`.
            text = String(text.split(separator: ":", omittingEmptySubsequences: true).last ?? "")
        }
        text = text.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return isEmail(text) ? text : nil
    }

    /// One `@` with something on both sides and no whitespace, control characters or `, ; < > : /`.
    private static func isEmail(_ text: String) -> Bool {
        let parts = text.split(separator: "@", omittingEmptySubsequences: false)
        guard parts.count == 2, !parts[0].isEmpty, !parts[1].isEmpty else { return false }
        let forbidden = CharacterSet.whitespacesAndNewlines.union(.controlCharacters).union(CharacterSet(charactersIn: ",;<>:/"))
        return text.unicodeScalars.allSatisfy { !forbidden.contains($0) }
    }
}
