import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import MicrosoftCalendar

// The Microsoft parts of Tests/PersistedFormats (see PersistedFormatTests in CalendarCoreTests): the stored delta
// positions ("<baseline seconds>|<link>") and the calendar-set key, as 1.0.7 wrote them.

private func persisted(_ version: String, _ name: String) throws -> Data {
    try Data(contentsOf: URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("PersistedFormats").appendingPathComponent(version).appendingPathComponent(name))
}

@Test func theMicrosoftStateOf107StillReads() throws {
    let connection = try #require(try JSONDecoder().decode([Connection].self, from: persisted("1.0.7", "connections.json"))
        .first { $0.kindID == MicrosoftConnectorKind.kindID })
    let kind = MicrosoftConnectorKind(config: MicrosoftOAuthConfig(clientID: "client"), transport: FakeTransport())
    let source = try kind.makeSource(for: connection, credentials: InMemoryCredentialStore(), syncState: InMemorySyncStateStore())
    #expect(source.id == "microsoft-5D2E8F4A-1B3C-4D6E-8F90-A1B2C3D4E5F6")
    #expect((source as? MicrosoftCalendarSource)?.accountEmail == "carol@contoso.com")

    let stored = try #require(try JSONDecoder().decode([String: [String: String]].self, from: persisted("1.0.7", "sync-state.json"))[
        connection.connectionID])
    #expect(stored[MicrosoftCalendarSource.calendarSetScope] == "AAMkAGI2TAAA=\nAAMkAGI2TBBB=")
    #expect(DeltaState(try #require(stored["AAMkAGI2TAAA="])) == DeltaState(
        baselineDate: Date(timeIntervalSince1970: 1_790_000_000),
        link: URL(string: "https://graph.microsoft.com/v1.0/me/calendars/AAMkAGI2TAAA=/calendarView/delta?$deltatoken=R0usmci39OQxqJrxK4")!))
    #expect(DeltaState(try #require(stored["AAMkAGI2TBBB="])) == DeltaState(
        baselineDate: Date(timeIntervalSince1970: 1_790_086_400),
        link: URL(string: "https://graph.microsoft.com/v1.0/me/calendars/AAMkAGI2TBBB=/calendarView/delta?$skiptoken=X9a2")!))
}

// Release 1.2.0 added the brokered Microsoft kind: a connection with no stored secret, whose account is the host's.
@Test func brokeredConnectionFromRelease120Decodes() async throws {
    let connections = try JSONDecoder().decode([Connection].self, from: persisted("1.2.0", "connections.json"))
    let connection = try #require(connections.first)
    #expect(connections.count == 1)
    #expect(connection.kindID == MicrosoftBrokeredConnectorKind.kindID && connection.kindID == "microsoft-brokered")
    #expect(connection.displayName == "dave@contoso.com")
    #expect(Set(connection.config.keys) == ["email", "authMode", "externalAccountID"])
    #expect(connection.config["authMode"] == "external" && connection.config["email"] == "dave@contoso.com")

    // It still opens through its kind, asks the host for tokens with the stored account and needs no credential.
    let source = FakeExternalTokenSource(tokens: [.success(ExternalAccessToken(accessToken: "tok", expiresAt: Date().addingTimeInterval(3600)))])
    let transport = FakeTransport()
    await transport.route("me/mailboxSettings/timeZone", [.json(["value": "UTC"])])
    await transport.route("me/calendars?", [.json(["value": [["id": "cal1", "name": "Calendar", "canEdit": false]]])])
    let kind = MicrosoftBrokeredConnectorKind(tokenSource: source, transport: transport, sleep: { _ in })
    let store = InMemoryCredentialStore()
    let opened = try kind.makeSource(for: connection, credentials: store, syncState: InMemorySyncStateStore())
    #expect(opened.id == "microsoft-brokered-7E3A9C21-4B58-4D0F-A6E2-91C8B5D40F37")
    #expect((opened as? MicrosoftCalendarSource)?.accountEmail == "dave@contoso.com")
    #expect(try await opened.calendars().map(\.id) == ["cal1"])
    #expect(await source.tokenCalls.first?.account.id == connection.config["externalAccountID"])
    #expect(await store.isEmpty)
}
