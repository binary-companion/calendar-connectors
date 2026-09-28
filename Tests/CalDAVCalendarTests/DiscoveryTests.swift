import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import CalDAVCalendar

private func discovery(_ transport: any HTTPTransport, password: String = "app-pass-1234", base: String = "icloud.com") -> CalDAVDiscovery {
    CalDAVDiscovery(client: WebDAVClient(transport: transport, hostBase: base,
                                         credentials: { WebDAVCredentials(username: "me@icloud.test", password: password) }))
}

@Test func discoversAnICloudShapedAccount() async throws {
    let server = FakeCalDAVServer()
    let account = try await discovery(server).discover(serverURL: fakeServerURL, username: "me@icloud.test")
    #expect(account.principalURL.absoluteString == "https://caldav.icloud.com/123/principal/")
    #expect(account.homeURL == fakeHomeURL)
    #expect(account.userAddresses == ["mailto:me@icloud.test", "urn:uuid:11111111-2222-3333-4444-555555555555"])
    #expect(account.autoSchedule)
    #expect(account.username == "me@icloud.test")
    #expect(account.serverURL == fakeServerURL)
    let methods = await server.log.map(\.method)
    #expect(methods == ["PROPFIND", "PROPFIND", "PROPFIND", "OPTIONS"])
}

@Test func followsAPartitionHostRedirect() async throws {
    let server = FakeCalDAVServer()
    await server.configure { $0.wellKnownLocation = "https://p42-caldav.icloud.com/" }
    let account = try await discovery(server).discover(serverURL: fakeServerURL, username: "me@icloud.test")
    #expect(account.principalURL.host == "p42-caldav.icloud.com")
    #expect(account.homeURL.absoluteString == "https://p42-caldav.icloud.com/123/calendars/")
}

@Test func refusesARedirectToAForeignHost() async throws {
    let server = FakeCalDAVServer()
    await server.configure { $0.wellKnownLocation = "https://collector.evil.test/" }
    await #expect(throws: SourceError.self) { try await discovery(server).discover(serverURL: fakeServerURL, username: "me@icloud.test") }
    #expect(await server.log.allSatisfy { $0.url.host != "collector.evil.test" })
}

@Test func fallsBackToTheServerURLWithoutWellKnown() async throws {
    let server = FakeCalDAVServer()
    await server.configure { $0.wellKnownLocation = nil }
    let account = try await discovery(server).discover(serverURL: fakeServerURL, username: "me@icloud.test")
    #expect(account.homeURL == fakeHomeURL)
}

@Test func wrongPasswordIsAuthExpired() async throws {
    await #expect(throws: SourceError.authExpired) {
        try await discovery(FakeCalDAVServer(), password: "wrong").discover(serverURL: fakeServerURL, username: "me@icloud.test")
    }
}

@Test func noAutoScheduleIsRecorded() async throws {
    let server = FakeCalDAVServer()
    await server.configure { $0.autoSchedule = false }
    #expect(try await discovery(server).discover(serverURL: fakeServerURL, username: "me@icloud.test").autoSchedule == false)
}

@Test func missingPrincipalIsInvalid() async throws {
    let transport = FakeTransport()
    await transport.route("caldav.icloud.com", [HTTPResponse(status: 207, body: Data("<multistatus xmlns=\"DAV:\"/>".utf8))])
    await #expect(throws: SourceError.self) { try await discovery(transport).discover(serverURL: fakeServerURL, username: "me@icloud.test") }
}

@Test func configRoundTripsAndRejectsMissingKeys() throws {
    let account = CalDAVAccountConfig(
        serverURL: fakeServerURL, username: "me@icloud.test", principalURL: URL(string: "https://caldav.icloud.com/123/principal/")!,
        homeURL: fakeHomeURL, userAddresses: ["mailto:me@icloud.test", "urn:uuid:1"], autoSchedule: true)
    #expect(try CalDAVAccountConfig(config: account.config) == account)
    #expect(account.config["userAddresses"] == "mailto:me@icloud.test\nurn:uuid:1")
    #expect(account.config["autoSchedule"] == "true")
    #expect(account.config.values.allSatisfy { !$0.contains("app-pass") })
    var broken = account.config
    broken["homeURL"] = nil
    #expect(throws: SourceError.self) { try CalDAVAccountConfig(config: broken) }
}
