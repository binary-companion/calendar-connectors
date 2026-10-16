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
