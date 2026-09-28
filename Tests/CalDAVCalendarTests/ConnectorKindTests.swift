import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import CalDAVCalendar

private let good = ["username": "  me@icloud.test ", "password": "app-pass-1234"]

@Test func iCloudKindDescribesItself() throws {
    let kind = ICloudConnectorKind(transport: FakeCalDAVServer())
    #expect(kind.id == "icloud" && kind.displayName == "iCloud")
    guard case .password(let fields) = kind.authorization else { Issue.record("not a password kind"); return }
    #expect(fields.map(\.key) == ["username", "password"] && fields.map(\.isSecret) == [false, true])
    #expect(fields.map(\.label) == ["Apple ID", "App-specific password"])
    #expect(kind.credentialHelp?.url != nil)
    #expect(kind.supportedPlatforms.contains(.linux))
}

@Test func iCloudSignInStoresConfigAndSecrets() async throws {
    let server = FakeCalDAVServer()
    let store = InMemoryCredentialStore()
    let connection = try await ICloudConnectorKind(transport: server).authorize(using: StubInteraction(good), credentials: store)
    #expect(connection.kindID == "icloud" && connection.displayName == "me@icloud.test")
    let account = try CalDAVAccountConfig(config: connection.config)
    #expect(account.username == "me@icloud.test" && account.homeURL == fakeHomeURL && account.autoSchedule)
    #expect(!connection.config.values.contains { $0.contains("app-pass") })
    #expect(try await store.secrets(for: connection.connectionID) == ["username": "me@icloud.test", "password": "app-pass-1234"])
}

@Test func failedSignInStoresNothing() async throws {
    let store = InMemoryCredentialStore()
    await #expect(throws: SourceError.authExpired) {
        try await ICloudConnectorKind(transport: FakeCalDAVServer())
            .authorize(using: StubInteraction(["username": "me@icloud.test", "password": "wrong"]), credentials: store)
    }
    #expect(await store.isEmpty)
}

@Test func caldavKindValidatesTheServerAddress() async throws {
    #expect(try CalDAVAccountSetup.serverURL(from: " caldav.example.test/dav ").absoluteString == "https://caldav.example.test/dav")
    #expect(try CalDAVAccountSetup.serverURL(from: "http://localhost:8008").absoluteString == "http://localhost:8008")
    #expect(throws: SourceError.self) { try CalDAVAccountSetup.serverURL(from: "http://caldav.example.test") }
    #expect(throws: SourceError.self) { try CalDAVAccountSetup.serverURL(from: "https://") }
    #expect(throws: SourceError.self) { try CalDAVAccountSetup.serverURL(from: "https://me:secret@caldav.example.test/dav/") }
    let server = FakeCalDAVServer()
    let interaction = StubInteraction(["serverURL": "http://caldav.example.test", "username": "me", "password": "p"])
    await #expect(throws: SourceError.self) {
        try await CalDAVConnectorKind(transport: server).authorize(using: interaction, credentials: InMemoryCredentialStore())
    }
    #expect(await server.log.isEmpty)
}

@Test func caldavKindSignsInToTheEnteredServer() async throws {
    let server = FakeCalDAVServer()
    await server.configure { $0.username = "me"; $0.password = "p" }
    let kind = CalDAVConnectorKind(transport: server)
    guard case .password(let fields) = kind.authorization else { Issue.record("not a password kind"); return }
    #expect(fields.map(\.key) == ["serverURL", "username", "password"])
    let connection = try await kind.authorize(
        using: StubInteraction(["serverURL": "https://caldav.example.test", "username": "me", "password": "p"]), credentials: InMemoryCredentialStore())
    #expect(connection.kindID == "caldav" && connection.displayName == "me@caldav.example.test")
    #expect(try CalDAVAccountConfig(config: connection.config).homeURL.host == "caldav.example.test")
}

@Test func reauthorizeKeepsTheConnectionAndRefusesAnotherAccount() async throws {
    let server = FakeCalDAVServer()
    let store = InMemoryCredentialStore()
    let kind = ICloudConnectorKind(transport: server)
    let connection = try await kind.authorize(using: StubInteraction(good), credentials: store)
    await server.configure { $0.password = "new-pass" }
    let again = try await kind.reauthorize(connection, using: StubInteraction(["username": "me@icloud.test", "password": "new-pass"]), credentials: store)
    #expect(again.connectionID == connection.connectionID)
    #expect(try await store.secrets(for: connection.connectionID)?["password"] == "new-pass")

    var other = connection
    other.config["principalURL"] = "https://caldav.icloud.com/999/principal/"
    await #expect(throws: SourceError.invalidResponse("signed in as a different account")) {
        try await kind.reauthorize(other, using: StubInteraction(["username": "me@icloud.test", "password": "new-pass"]), credentials: store)
    }
}

@Test func makeSourceReadsSecretsFromTheStore() async throws {
    let server = FakeCalDAVServer()
    let store = InMemoryCredentialStore()
    let kind = ICloudConnectorKind(transport: server)
    let connection = try await kind.authorize(using: StubInteraction(good), credentials: store)
    let source = try kind.makeSource(for: connection, credentials: store, syncState: InMemorySyncStateStore())
    #expect(source.id == "icloud-\(connection.connectionID)")
    #expect(source is CalDAVCalendarSource)
    #expect(try await source.calendars().map(\.id) == ["home"])
    try await store.removeSecrets(for: connection.connectionID)
    await #expect(throws: SourceError.authExpired) { try await source.calendars() }
    var broken = connection
    broken.config = [:]
    #expect(throws: SourceError.self) { try kind.makeSource(for: broken, credentials: store, syncState: InMemorySyncStateStore()) }
}
