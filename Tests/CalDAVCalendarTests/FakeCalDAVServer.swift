import CalendarCore
import CalendarTestSupport
import Foundation
@testable import CalDAVCalendar

/// An in-memory CalDAV server with iCloud-shaped paths, behind `HTTPTransport`. It ignores the host and answers by
/// method and path, keeps ETags, ctags and sync tokens, checks `If-Match`/`If-None-Match`, and can be told to fail a
/// request. A `calendar-query` returns every resource of the calendar (the source filters by the window itself).
actor FakeCalDAVServer: HTTPTransport {
    struct Resource: Sendable { var body: String; var etag: String; var revision: Int }

    struct Collection: Sendable {
        var displayName: String
        var color: String? = "#1BADF8FF"
        /// nil: the server does not say (all components).
        var components: [String]? = ["VEVENT"]
        var privileges: [String] = ["read", "write"]
        var timeZoneID: String? = "America/Los_Angeles"
        var subscribed = false
        var resources: [String: Resource] = [:]
        var removed: [String: Int] = [:]
        var revision = 1
    }

    private struct Failure { var method: String; var pathContains: String; var status: Int; var remaining: Int; var skip: Int }

    static let principalPath = "/123/principal/"
    static let homePath = "/123/calendars/"
    static let inboxPath = "/123/inbox/"

    var username = "me@icloud.test"
    var password = "app-pass-1234"
    var userAddresses = ["mailto:me@icloud.test", "urn:uuid:11111111-2222-3333-4444-555555555555"]
    var autoSchedule = true
    var supportsSync = true
    var sendsETagOnPut = true
    var defaultCalendar: String? = "home"
    /// Where `/.well-known/caldav` redirects; nil answers 404.
    var wellKnownLocation: String? = "/"
    var collections: [String: Collection] = ["home": Collection(displayName: "Home")]
    private var revision = 1
    private var oldestValidToken = 0
    private var failures: [Failure] = []
    private(set) var log: [HTTPRequest] = []

    func configure(_ change: @Sendable (isolated FakeCalDAVServer) -> Void) { change(self) }

    // MARK: Test helpers

    /// A change made by someone else (another client).
    func store(_ calendar: String, _ name: String, _ ics: String) {
        revision += 1
        collections[calendar, default: Collection(displayName: calendar)].resources[name] =
            Resource(body: Self.crlf(ics), etag: "\"e\(revision)\"", revision: revision)
        collections[calendar]!.removed[name] = nil
        collections[calendar]!.revision = revision
    }

    func remove(_ calendar: String, _ name: String) {
        revision += 1
        collections[calendar]?.resources[name] = nil
        collections[calendar]?.removed[name] = revision
        collections[calendar]?.revision = revision
    }

    func body(_ calendar: String, _ name: String) -> String? { collections[calendar]?.resources[name]?.body }
    func etag(_ calendar: String, _ name: String) -> String? { collections[calendar]?.resources[name]?.etag }
    func names(_ calendar: String) -> [String] { (collections[calendar]?.resources.keys).map { $0.sorted() } ?? [] }
    func requests(_ method: String) -> [HTTPRequest] { log.filter { $0.method == method } }
    func clearLog() { log = [] }

    /// After letting `after` matching requests through, the next `times` requests with this method whose path contains
    /// `pathContains` answer `status`.
    func fail(_ method: String, pathContains: String, status: Int, times: Int = 1, after: Int = 0) {
        failures.append(Failure(method: method, pathContains: pathContains, status: status, remaining: times, skip: after))
    }

    /// Every sync token issued so far stops being accepted.
    func expireSyncTokens() {
        revision += 1
        oldestValidToken = revision
        for key in collections.keys { collections[key]!.revision = revision }
    }

    static func crlf(_ text: String) -> String {
        text.replacingOccurrences(of: "\r\n", with: "\n").replacingOccurrences(of: "\n", with: "\r\n")
    }

    // MARK: HTTPTransport

    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        log.append(request)
        guard request.headers["Authorization"] == WebDAVClient.basicAuthorization(WebDAVCredentials(username: username, password: password))
        else { return HTTPResponse(status: 401) }
        let raw = (URLComponents(url: request.url, resolvingAgainstBaseURL: false)?.percentEncodedPath ?? "").removingPercentEncoding ?? ""
        let path = raw.isEmpty ? "/" : raw
        if let index = failures.firstIndex(where: { $0.method == request.method && path.contains($0.pathContains) && $0.remaining > 0 }) {
            if failures[index].skip > 0 {
                failures[index].skip -= 1
            } else {
                failures[index].remaining -= 1
                return HTTPResponse(status: failures[index].status)
            }
        }
        if path.hasSuffix("/.well-known/caldav") {
            guard let wellKnownLocation else { return HTTPResponse(status: 404) }
            return HTTPResponse(status: 301, headers: ["Location": wellKnownLocation])
        }
        if request.method == "OPTIONS" {
            return HTTPResponse(status: 200, headers: ["DAV": "1, 2, 3, access-control, calendar-access" + (autoSchedule ? ", calendar-auto-schedule" : "")])
        }
        switch (request.method, path) {
        case ("PROPFIND", "/"):
            return multistatus([response("/", ["<d:current-user-principal><d:href>\(Self.principalPath)</d:href></d:current-user-principal>"])])
        case ("PROPFIND", Self.principalPath):
            let addresses = userAddresses.map { "<d:href>\(DAVXML.escape($0))</d:href>" }.joined()
            return multistatus([response(Self.principalPath, [
                "<c:calendar-home-set><d:href>\(Self.homePath)</d:href></c:calendar-home-set>",
                "<c:calendar-user-address-set>\(addresses)</c:calendar-user-address-set>",
                "<c:schedule-inbox-URL><d:href>\(Self.inboxPath)</d:href></c:schedule-inbox-URL>",
            ])])
        case ("PROPFIND", Self.inboxPath):
            let props = defaultCalendar.map {
                ["<c:schedule-default-calendar-URL><d:href>\(Self.homePath)\($0)/</d:href></c:schedule-default-calendar-URL>"]
            } ?? []
            return multistatus([response(Self.inboxPath, props)])
        case ("PROPFIND", Self.homePath):
            return multistatus(homeResponses())
        default:
            break
        }
        guard path.hasPrefix(Self.homePath) else { return HTTPResponse(status: 404) }
        let parts = path.dropFirst(Self.homePath.count).split(separator: "/").map(String.init)
        guard let calendar = parts.first, collections[calendar] != nil else { return HTTPResponse(status: 404) }
        if parts.count == 1 {
            switch request.method {
            case "REPORT": return report(calendar, request)
            case "PROPFIND": return multistatus([collectionResponse(calendar)])
            default: return HTTPResponse(status: 405)
            }
        }
        let name = parts[1]
        let existing = collections[calendar]!.resources[name]
        switch request.method {
        case "GET":
            guard let existing else { return HTTPResponse(status: 404) }
            return HTTPResponse(status: 200, headers: ["ETag": existing.etag, "Content-Type": "text/calendar"], body: Data(existing.body.utf8))
        case "PUT":
            guard collections[calendar]!.privileges.contains("write") else { return HTTPResponse(status: 403) }
            if request.headers["If-None-Match"] == "*", existing != nil { return HTTPResponse(status: 412) }
            if let match = request.headers["If-Match"], match != existing?.etag { return HTTPResponse(status: 412) }
            revision += 1
            let etag = "\"e\(revision)\""
            collections[calendar]!.resources[name] = Resource(body: String(decoding: request.body ?? Data(), as: UTF8.self), etag: etag, revision: revision)
            collections[calendar]!.removed[name] = nil
            collections[calendar]!.revision = revision
            return HTTPResponse(status: existing == nil ? 201 : 204, headers: sendsETagOnPut ? ["ETag": etag] : [:])
        case "DELETE":
            guard collections[calendar]!.privileges.contains("write") else { return HTTPResponse(status: 403) }
            guard let existing else { return HTTPResponse(status: 404) }
            if let match = request.headers["If-Match"], match != existing.etag { return HTTPResponse(status: 412) }
            remove(calendar, name)
            return HTTPResponse(status: 204)
        default:
            return HTTPResponse(status: 405)
        }
    }

    // MARK: Bodies

    private func token(_ revision: Int) -> String { "https://fake.test/sync/\(revision)" }
    private func href(_ calendar: String, _ name: String? = nil) -> String { Self.homePath + calendar + "/" + (name ?? "") }

    private func response(_ href: String, _ props: [String]) -> String {
        "<d:response><d:href>\(href)</d:href><d:propstat><d:prop>\(props.joined())</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"
    }

    private func multistatus(_ responses: [String], syncToken: String? = nil) -> HTTPResponse {
        let token = syncToken.map { "<d:sync-token>\($0)</d:sync-token>" } ?? ""
        let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><d:multistatus xmlns:d=\"DAV:\" xmlns:c=\"\(DAV.caldav)\" "
            + "xmlns:cs=\"\(DAV.calendarServer)\" xmlns:a=\"\(DAV.apple)\">\(responses.joined())\(token)</d:multistatus>"
        return HTTPResponse(status: 207, headers: ["Content-Type": "application/xml"], body: Data(xml.utf8))
    }

    private func homeResponses() -> [String] {
        var responses = [
            response(Self.homePath, ["<d:resourcetype><d:collection/></d:resourcetype>"]),
            response(Self.inboxPath, ["<d:resourcetype><d:collection/><c:schedule-inbox/></d:resourcetype>"]),
            response(Self.homePath + "outbox/", ["<d:resourcetype><d:collection/><c:schedule-outbox/></d:resourcetype>"]),
            response(Self.homePath + "notification/", ["<d:resourcetype><d:collection/><cs:notification/></d:resourcetype>"]),
        ]
        responses += collections.keys.sorted().map(collectionResponse)
        return responses
    }

    private func collectionResponse(_ name: String) -> String {
        let c = collections[name]!
        var props = [
            "<d:resourcetype><d:collection/><c:calendar/>\(c.subscribed ? "<cs:subscribed/>" : "")</d:resourcetype>",
            "<d:displayname>\(DAVXML.escape(c.displayName))</d:displayname>",
            "<cs:getctag>ctag-\(c.revision)</cs:getctag>",
            "<d:current-user-privilege-set>\(c.privileges.map { "<d:privilege><d:\($0)/></d:privilege>" }.joined())</d:current-user-privilege-set>",
        ]
        if let color = c.color { props.append("<a:calendar-color>\(color)</a:calendar-color>") }
        if let components = c.components {
            props.append("<c:supported-calendar-component-set>\(components.map { "<c:comp name=\"\($0)\"/>" }.joined())</c:supported-calendar-component-set>")
        }
        if supportsSync { props.append("<d:sync-token>\(token(c.revision))</d:sync-token>") }
        if let zone = c.timeZoneID {
            let vcalendar = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTIMEZONE\r\nTZID:\(zone)\r\nEND:VTIMEZONE\r\nEND:VCALENDAR\r\n"
            props.append("<c:calendar-timezone>\(DAVXML.escape(vcalendar))</c:calendar-timezone>")
        }
        return response(href(name), props)
    }

    private func report(_ calendar: String, _ request: HTTPRequest) -> HTTPResponse {
        guard let body = request.body, let root = try? XMLTree.parse(body) else { return HTTPResponse(status: 400) }
        let c = collections[calendar]!
        if root.name == "sync-collection" {
            let invalid = HTTPResponse(status: 403, body: Data("<d:error xmlns:d=\"DAV:\"><d:valid-sync-token/></d:error>".utf8))
            guard supportsSync else { return HTTPResponse(status: 403) }
            var since = 0
            let text = root.child(DAV.dav, "sync-token")?.trimmedText ?? ""
            if !text.isEmpty {
                guard let value = text.split(separator: "/").last.flatMap({ Int($0) }), value >= oldestValidToken, value <= revision else { return invalid }
                since = value
            }
            var responses = c.resources.filter { $0.value.revision > since }.sorted { $0.key < $1.key }
                .map { response(href(calendar, $0.key), ["<d:getetag>\(DAVXML.escape($0.value.etag))</d:getetag>"]) }
            responses += c.removed.filter { $0.value > since }.keys.sorted()
                .map { "<d:response><d:href>\(href(calendar, $0))</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response>" }
            return multistatus(responses, syncToken: token(c.revision))
        }
        let uid = root.first(DAV.caldav, "text-match")?.trimmedText
        let matching = c.resources.filter { entry in
            guard let uid else { return true }
            return entry.value.body.components(separatedBy: "\r\n").contains("UID:" + uid)
        }.sorted { $0.key < $1.key }
        return multistatus(matching.map {
            response(href(calendar, $0.key), [
                "<d:getetag>\(DAVXML.escape($0.value.etag))</d:getetag>",
                "<c:calendar-data>\(DAVXML.escape($0.value.body))</c:calendar-data>",
            ])
        })
    }
}

/// Hands out `uuid-1`, `uuid-2`, ... so tests know the names and UIDs a write will use.
final class UUIDSequence: @unchecked Sendable {
    private let lock = NSLock()
    private var value = 0
    func next() -> String { lock.withLock { value += 1; return "uuid-\(value)" } }
}

let fakeServerURL = URL(string: "https://caldav.icloud.com")!
let fakeHomeURL = URL(string: "https://caldav.icloud.com/123/calendars/")!
