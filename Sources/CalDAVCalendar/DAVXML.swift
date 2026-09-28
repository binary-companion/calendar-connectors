import CalendarCore
import Foundation
import ICalendar

enum DAV {
    static let dav = "DAV:"
    static let caldav = "urn:ietf:params:xml:ns:caldav"
    static let calendarServer = "http://calendarserver.org/ns/"
    static let apple = "http://apple.com/ns/ical/"
}

struct DAVProperty: Hashable, Sendable {
    var namespace: String
    var name: String

    static let currentUserPrincipal = DAVProperty(namespace: DAV.dav, name: "current-user-principal")
    static let calendarHomeSet = DAVProperty(namespace: DAV.caldav, name: "calendar-home-set")
    static let calendarUserAddressSet = DAVProperty(namespace: DAV.caldav, name: "calendar-user-address-set")
    static let scheduleInboxURL = DAVProperty(namespace: DAV.caldav, name: "schedule-inbox-URL")
    static let scheduleDefaultCalendarURL = DAVProperty(namespace: DAV.caldav, name: "schedule-default-calendar-URL")
    static let resourceType = DAVProperty(namespace: DAV.dav, name: "resourcetype")
    static let displayName = DAVProperty(namespace: DAV.dav, name: "displayname")
    static let supportedComponents = DAVProperty(namespace: DAV.caldav, name: "supported-calendar-component-set")
    static let privileges = DAVProperty(namespace: DAV.dav, name: "current-user-privilege-set")
    static let calendarTimeZone = DAVProperty(namespace: DAV.caldav, name: "calendar-timezone")
    static let calendarColor = DAVProperty(namespace: DAV.apple, name: "calendar-color")
    static let getCTag = DAVProperty(namespace: DAV.calendarServer, name: "getctag")
    static let syncToken = DAVProperty(namespace: DAV.dav, name: "sync-token")
    static let getETag = DAVProperty(namespace: DAV.dav, name: "getetag")
    static let calendarData = DAVProperty(namespace: DAV.caldav, name: "calendar-data")
}

struct DAVResponse: Sendable, Equatable {
    var href: String
    /// The response-level status (a `sync-collection` removal is 404); nil when the response has propstats.
    var status: Int?
    /// Properties from `200` propstats only.
    var properties: [DAVProperty: XMLTree]
}

struct Multistatus: Sendable, Equatable {
    var responses: [DAVResponse]
    var syncToken: String?
}

enum DAVXML {
    static func multistatus(_ data: Data) throws -> Multistatus {
        let root = try XMLTree.parse(data)
        guard root.namespace == DAV.dav, root.name == "multistatus" else {
            throw SourceError.invalidResponse("expected a multistatus body")
        }
        var responses: [DAVResponse] = []
        for response in root.children(DAV.dav, "response") {
            guard let href = response.child(DAV.dav, "href")?.trimmedText, !href.isEmpty else { continue }
            var properties: [DAVProperty: XMLTree] = [:]
            for propstat in response.children(DAV.dav, "propstat") where status(propstat.child(DAV.dav, "status")) == 200 {
                for property in propstat.child(DAV.dav, "prop")?.children ?? [] {
                    properties[DAVProperty(namespace: property.namespace, name: property.name)] = property
                }
            }
            responses.append(DAVResponse(href: href, status: status(response.child(DAV.dav, "status")), properties: properties))
        }
        return Multistatus(responses: responses, syncToken: root.child(DAV.dav, "sync-token")?.trimmedText)
    }

    /// `HTTP/1.1 200 OK` → 200.
    static func status(_ element: XMLTree?) -> Int? {
        guard let parts = element?.trimmedText.split(separator: " "), parts.count >= 2 else { return nil }
        return Int(parts[1])
    }

    static func propfind(_ properties: [DAVProperty]) -> Data {
        document("<d:propfind \(namespaces)><d:prop>\(properties.map(element).joined())</d:prop></d:propfind>")
    }

    static func calendarQuery(from start: Date, to end: Date) -> Data {
        document("""
        <c:calendar-query \(namespaces)><d:prop><d:getetag/><c:calendar-data/></d:prop>\
        <c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT">\
        <c:time-range start="\(ICalValues.utcText(start))" end="\(ICalValues.utcText(end))"/>\
        </c:comp-filter></c:comp-filter></c:filter></c:calendar-query>
        """)
    }

    static func calendarQuery(uid: String) -> Data {
        document("""
        <c:calendar-query \(namespaces)><d:prop><d:getetag/><c:calendar-data/></d:prop>\
        <c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT"><c:prop-filter name="UID">\
        <c:text-match collation="i;octet">\(escape(uid))</c:text-match>\
        </c:prop-filter></c:comp-filter></c:comp-filter></c:filter></c:calendar-query>
        """)
    }

    static func syncCollection(token: String?) -> Data {
        document("""
        <d:sync-collection \(namespaces)><d:sync-token>\(escape(token ?? ""))</d:sync-token><d:sync-level>1</d:sync-level>\
        <d:prop><d:getetag/></d:prop></d:sync-collection>
        """)
    }

    /// RFC 6578: a token the server no longer accepts is a 403 (or 409 on some servers) with `DAV:valid-sync-token`.
    static func isInvalidSyncToken(_ response: HTTPResponse) -> Bool {
        guard response.status == 403 || response.status == 409 else { return false }
        return (try? XMLTree.parse(response.body))?.first(DAV.dav, "valid-sync-token") != nil
    }

    private static let namespaces = "xmlns:d=\"DAV:\" xmlns:c=\"\(DAV.caldav)\" xmlns:cs=\"\(DAV.calendarServer)\" xmlns:a=\"\(DAV.apple)\""

    private static func element(_ property: DAVProperty) -> String {
        let prefix: String
        switch property.namespace {
        case DAV.dav: prefix = "d"
        case DAV.caldav: prefix = "c"
        case DAV.calendarServer: prefix = "cs"
        case DAV.apple: prefix = "a"
        default: return "<x:\(property.name) xmlns:x=\"\(escape(property.namespace))\"/>"
        }
        return "<\(prefix):\(property.name)/>"
    }

    private static func document(_ body: String) -> Data { Data(("<?xml version=\"1.0\" encoding=\"UTF-8\"?>" + body).utf8) }

    static func escape(_ text: String) -> String {
        text.replacingOccurrences(of: "&", with: "&amp;").replacingOccurrences(of: "<", with: "&lt;")
            .replacingOccurrences(of: ">", with: "&gt;").replacingOccurrences(of: "\"", with: "&quot;")
    }
}
