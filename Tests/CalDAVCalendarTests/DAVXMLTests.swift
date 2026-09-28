import CalendarCore
import Foundation
import Testing
@testable import CalDAVCalendar

/// Shaped like iCloud's answer to a depth-1 PROPFIND on a calendar home (scrubbed; uppercase prefixes).
let homeMultistatus = """
<?xml version="1.0" encoding="UTF-8"?>
<multistatus xmlns="DAV:">
 <response>
  <href>/123/calendars/home/</href>
  <propstat>
   <prop>
    <displayname>Home</displayname>
    <resourcetype><collection/><calendar xmlns="urn:ietf:params:xml:ns:caldav"/></resourcetype>
    <calendar-color xmlns="http://apple.com/ns/ical/">#FF2968FF</calendar-color>
    <getctag xmlns="http://calendarserver.org/ns/">HwoQEgwAAA</getctag>
    <sync-token>https://example.test/sync/1</sync-token>
   </prop>
   <status>HTTP/1.1 200 OK</status>
  </propstat>
  <propstat>
   <prop><calendar-timezone xmlns="urn:ietf:params:xml:ns:caldav"/></prop>
   <status>HTTP/1.1 404 Not Found</status>
  </propstat>
 </response>
</multistatus>
"""

@Test func parsesMultistatusWithFoundAndMissingProperties() throws {
    let result = try DAVXML.multistatus(Data(homeMultistatus.utf8))
    let response = try #require(result.responses.first)
    #expect(response.href == "/123/calendars/home/")
    #expect(response.properties[.displayName]?.trimmedText == "Home")
    #expect(response.properties[.calendarColor]?.trimmedText == "#FF2968FF")
    #expect(response.properties[.getCTag]?.trimmedText == "HwoQEgwAAA")
    #expect(response.properties[.syncToken]?.trimmedText == "https://example.test/sync/1")
    #expect(response.properties[.resourceType]?.child(DAV.caldav, "calendar") != nil)
    #expect(response.properties[.calendarTimeZone] == nil)   // 404 propstat is not a value
}

@Test func multistatusIgnoresPrefixes() throws {
    let prefixed = """
    <d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/">
      <d:response><d:href>/cal/a.ics</d:href>
        <d:propstat><d:prop><d:getetag>"7"</d:getetag><c:calendar-data>BEGIN:VCALENDAR</c:calendar-data></d:prop>
        <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
      <d:response><d:href>/cal/gone.ics</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response>
      <d:sync-token>tok-2</d:sync-token>
    </d:multistatus>
    """
    let result = try DAVXML.multistatus(Data(prefixed.utf8))
    #expect(result.responses.map(\.href) == ["/cal/a.ics", "/cal/gone.ics"])
    #expect(result.responses[0].properties[.getETag]?.trimmedText == "\"7\"")
    #expect(result.responses[0].properties[.calendarData]?.text == "BEGIN:VCALENDAR")
    #expect(result.responses[1].status == 404)
    #expect(result.syncToken == "tok-2")
}

@Test func rejectsNonXML() {
    #expect(throws: SourceError.self) { try DAVXML.multistatus(Data("<html><body>oops".utf8)) }
    #expect(throws: SourceError.self) { try DAVXML.multistatus(Data("<other xmlns=\"DAV:\"/>".utf8)) }
}

@Test func buildsRequestBodies() throws {
    let propfind = try XMLTree.parse(DAVXML.propfind([.displayName, .getCTag]))
    #expect(propfind.namespace == DAV.dav && propfind.name == "propfind")
    #expect(propfind.child(DAV.dav, "prop")?.child(DAV.calendarServer, "getctag") != nil)

    let start = Date(timeIntervalSince1970: 1_790_521_200)
    let query = try XMLTree.parse(DAVXML.calendarQuery(from: start, to: start.addingTimeInterval(86_400)))
    #expect(query.name == "calendar-query")
    let range = try #require(query.first(DAV.caldav, "time-range"))
    #expect(range.attributes["start"] == "20260927T150000Z" && range.attributes["end"] == "20260928T150000Z")
    #expect(query.first(DAV.caldav, "calendar-data") != nil && query.first(DAV.dav, "getetag") != nil)

    let byUID = try XMLTree.parse(DAVXML.calendarQuery(uid: "a<b&c"))
    #expect(byUID.first(DAV.caldav, "text-match")?.trimmedText == "a<b&c")

    let sync = try XMLTree.parse(DAVXML.syncCollection(token: nil))
    #expect(sync.name == "sync-collection" && sync.child(DAV.dav, "sync-token")?.trimmedText == "")
    #expect(sync.child(DAV.dav, "sync-level")?.trimmedText == "1")
}

@Test func recognizesAnInvalidSyncToken() {
    let body = "<error xmlns=\"DAV:\"><valid-sync-token/></error>"
    #expect(DAVXML.isInvalidSyncToken(HTTPResponse(status: 403, body: Data(body.utf8))))
    #expect(DAVXML.isInvalidSyncToken(HTTPResponse(status: 409, body: Data(body.utf8))))
    #expect(!DAVXML.isInvalidSyncToken(HTTPResponse(status: 403, body: Data("<error xmlns=\"DAV:\"><need-privileges/></error>".utf8))))
}
