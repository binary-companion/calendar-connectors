import CalendarCore
import Foundation

/// RFC 6764 and RFC 4791 discovery: `/.well-known/caldav` (or the server URL itself), the current user's principal,
/// its calendar home and addresses, and whether the server schedules invitations.
struct CalDAVDiscovery: Sendable {
    let client: WebDAVClient

    func discover(serverURL: URL, username: String) async throws -> CalDAVAccountConfig {
        let principal = try await principalURL(serverURL: serverURL)
        let (status, answeredBy) = try await client.propfind(principal, depth: 0, [.calendarHomeSet, .calendarUserAddressSet])
        let properties = status.responses.first?.properties ?? [:]
        guard let homeHref = properties[.calendarHomeSet]?.child(DAV.dav, "href")?.trimmedText, !homeHref.isEmpty else {
            throw SourceError.invalidResponse("the server did not name a calendar home")
        }
        var home = try client.resolve(homeHref, against: answeredBy)
        if !home.absoluteString.hasSuffix("/") { home = URL(string: home.absoluteString + "/") ?? home }
        let addresses = (properties[.calendarUserAddressSet]?.children(DAV.dav, "href") ?? [])
            .map { $0.trimmedText.lowercased() }.filter { !$0.isEmpty }
        let options = try await client.send("OPTIONS", home)
        let dav = options.response.header("DAV") ?? ""
        let autoSchedule = dav.split(separator: ",").contains { $0.trimmingCharacters(in: .whitespaces).lowercased() == "calendar-auto-schedule" }
        return CalDAVAccountConfig(serverURL: serverURL, username: username, principalURL: principal, homeURL: home,
                                   userAddresses: addresses, autoSchedule: autoSchedule)
    }

    /// The well-known URL first; a server without it (404, 405, 501) is asked at the URL the user gave.
    private func principalURL(serverURL: URL) async throws -> URL {
        let wellKnown = serverURL.appendingPathComponent(".well-known/caldav")
        for candidate in [wellKnown, serverURL] {
            let reply = try await client.send("PROPFIND", candidate, headers: ["Depth": "0"], body: DAVXML.propfind([.currentUserPrincipal]))
            switch reply.response.status {
            case 207:
                let status = try DAVXML.multistatus(reply.response.body)
                guard let href = status.responses.first?.properties[.currentUserPrincipal]?.child(DAV.dav, "href")?.trimmedText,
                      !href.isEmpty else {
                    throw SourceError.invalidResponse("the server did not name a principal")
                }
                return try client.resolve(href, against: reply.url)
            case let status where candidate == wellKnown && [404, 405, 501].contains(status):
                continue
            default:
                throw SourceError.invalidResponse("the server answered \(reply.response.status) to discovery")
            }
        }
        throw SourceError.invalidResponse("the server did not name a principal")
    }
}
