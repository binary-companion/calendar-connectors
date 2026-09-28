import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import CalDAVCalendar

private let secret = WebDAVCredentials(username: "me@icloud.test", password: "pä:ss wörd")

private func client(_ transport: FakeTransport, base: String = "icloud.com") -> WebDAVClient {
    WebDAVClient(transport: transport, hostBase: base, credentials: { secret })
}

@Test func basicAuthEncodesUTF8AndColons() async throws {
    #expect(WebDAVClient.basicAuthorization(secret) == "Basic " + Data("me@icloud.test:pä:ss wörd".utf8).base64EncodedString())
    let transport = FakeTransport()
    await transport.route("caldav.icloud.com", [HTTPResponse(status: 401)])
    do {
        _ = try await client(transport).send("PROPFIND", URL(string: "https://caldav.icloud.com/")!)
        Issue.record("expected authExpired")
    } catch let error as SourceError {
        #expect(error == .authExpired)
        #expect(!String(describing: error).contains("wörd"))
    }
    let sent = await transport.requests
    #expect(sent.first?.headers["Authorization"] == WebDAVClient.basicAuthorization(secret))
}

@Test func hostRuleMatchesAtALabelBoundary() {
    #expect(WebDAVClient.isAllowed(URL(string: "https://caldav.icloud.com/")!, hostBase: "icloud.com"))
    #expect(WebDAVClient.isAllowed(URL(string: "https://p42-caldav.icloud.com/x")!, hostBase: "icloud.com"))
    #expect(WebDAVClient.isAllowed(URL(string: "https://ICLOUD.com/")!, hostBase: "icloud.com"))
    #expect(!WebDAVClient.isAllowed(URL(string: "https://evilicloud.com/")!, hostBase: "icloud.com"))
    #expect(!WebDAVClient.isAllowed(URL(string: "https://icloud.com.evil.test/")!, hostBase: "icloud.com"))
    #expect(!WebDAVClient.isAllowed(URL(string: "http://caldav.icloud.com/")!, hostBase: "icloud.com"))
    #expect(WebDAVClient.isAllowed(URL(string: "http://localhost:8008/")!, hostBase: "localhost"))
    #expect(WebDAVClient.isAllowed(URL(string: "http://127.0.0.1:8008/")!, hostBase: "127.0.0.1"))
    #expect(!WebDAVClient.isAllowed(URL(string: "ftp://caldav.icloud.com/")!, hostBase: "icloud.com"))
}

@Test func followsRedirectsInsideTheBase() async throws {
    let transport = FakeTransport()
    await transport.route("https://caldav.icloud.com/.well-known/caldav", [HTTPResponse(status: 301, headers: ["Location": "https://p42-caldav.icloud.com/"])])
    await transport.route("https://p42-caldav.icloud.com/", [HTTPResponse(status: 207, body: Data("<multistatus xmlns=\"DAV:\"/>".utf8))])
    let reply = try await client(transport).send("PROPFIND", URL(string: "https://caldav.icloud.com/.well-known/caldav")!,
                                                 headers: ["Depth": "0"], body: Data("<x/>".utf8))
    #expect(reply.response.status == 207)
    #expect(reply.url.absoluteString == "https://p42-caldav.icloud.com/")
    let second = try #require(await transport.requests.last)
    #expect(second.method == "PROPFIND" && second.body == Data("<x/>".utf8) && second.headers["Depth"] == "0")
}

@Test func refusesARedirectToAnotherHostWithoutSendingCredentials() async throws {
    let transport = FakeTransport()
    await transport.route("https://caldav.icloud.com/", [HTTPResponse(status: 302, headers: ["Location": "https://collector.evil.test/steal"])])
    await #expect(throws: SourceError.self) { try await client(transport).send("GET", URL(string: "https://caldav.icloud.com/")!) }
    #expect(await transport.requests(matching: "evil.test").isEmpty)
}

@Test func refusesADowngradeToHTTP() async throws {
    let transport = FakeTransport()
    await transport.route("https://caldav.icloud.com/", [HTTPResponse(status: 301, headers: ["Location": "http://caldav.icloud.com/"])])
    await #expect(throws: SourceError.self) { try await client(transport).send("GET", URL(string: "https://caldav.icloud.com/")!) }
    #expect(await transport.requests.count == 1)
}

@Test func stopsAfterFiveRedirects() async throws {
    let transport = FakeTransport()
    await transport.route("https://caldav.icloud.com/", [HTTPResponse(status: 307, headers: ["Location": "/again"])])
    await #expect(throws: SourceError.self) { try await client(transport).send("GET", URL(string: "https://caldav.icloud.com/")!) }
    #expect(await transport.requests.count == 6)
}

@Test func seeOtherBecomesAGet() async throws {
    let transport = FakeTransport()
    await transport.route("https://caldav.icloud.com/a", [HTTPResponse(status: 303, headers: ["Location": "/b"])])
    await transport.route("https://caldav.icloud.com/b", [HTTPResponse(status: 200)])
    _ = try await client(transport).send("PUT", URL(string: "https://caldav.icloud.com/a")!, body: Data("x".utf8))
    let last = try #require(await transport.requests.last)
    #expect(last.method == "GET" && last.body == nil)
}

@Test func mapsErrorStatuses() async throws {
    let transport = FakeTransport()
    await transport.route("/limited", [HTTPResponse(status: 429, headers: ["Retry-After": "7"])])
    await transport.route("/down", [HTTPResponse(status: 503)])
    await transport.route("/broken", [HTTPResponse(status: 500)])
    await transport.route("/full", [HTTPResponse(status: 507)])
    await transport.route("/missing", [HTTPResponse(status: 404)])
    let c = client(transport)
    func status(_ path: String) async -> SourceError? {
        do { _ = try await c.send("GET", URL(string: "https://caldav.icloud.com" + path)!); return nil } catch { return error as? SourceError }
    }
    #expect(await status("/limited") == .rateLimited(retryAfter: 7))
    #expect(await status("/down") == .rateLimited(retryAfter: nil))
    #expect(await status("/broken") == .server(status: 500))
    #expect(await status("/full") == .server(status: 507))
    #expect(await status("/missing") == nil)   // returned to the caller
}

@Test func refusesToSendCredentialsOverHTTPOrToAnotherHost() async throws {
    let transport = FakeTransport()
    await #expect(throws: SourceError.self) { try await client(transport).send("GET", URL(string: "http://caldav.icloud.com/")!) }
    await #expect(throws: SourceError.self) { try await client(transport).send("GET", URL(string: "https://example.test/")!) }
    #expect(await transport.requests.isEmpty)
}

@Test func resolvesHrefsAndRefusesForeignOnes() throws {
    let c = client(FakeTransport())
    let base = URL(string: "https://p42-caldav.icloud.com/123/calendars/")!
    #expect(try c.resolve("/123/calendars/home/", against: base).absoluteString == "https://p42-caldav.icloud.com/123/calendars/home/")
    #expect(try c.resolve("home/a%20b.ics", against: base).absoluteString == "https://p42-caldav.icloud.com/123/calendars/home/a%20b.ics")
    #expect(try c.resolve("https://p07-caldav.icloud.com/123/principal/", against: base).host == "p07-caldav.icloud.com")
    #expect(throws: SourceError.self) { try c.resolve("https://evil.test/123/", against: base) }
}
