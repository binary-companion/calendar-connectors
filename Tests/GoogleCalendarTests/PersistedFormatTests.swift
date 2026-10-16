import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import GoogleCalendar

// The Google connection of Tests/PersistedFormats (see PersistedFormatTests in CalendarCoreTests), as 1.0.7 wrote it.

private func persisted(_ version: String, _ name: String) throws -> Data {
    try Data(contentsOf: URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("PersistedFormats").appendingPathComponent(version).appendingPathComponent(name))
}

@Test func theGoogleConnectionOf107StillOpens() throws {
    let connection = try #require(try JSONDecoder().decode([Connection].self, from: persisted("1.0.7", "connections.json"))
        .first { $0.kindID == GoogleConnectorKind.kindID })
    let kind = GoogleConnectorKind(config: GoogleOAuthConfig(clientID: "client", clientSecret: "client-secret"), transport: FakeTransport())
    let source = try kind.makeSource(for: connection, credentials: InMemoryCredentialStore(), syncState: InMemorySyncStateStore())
    #expect(source.id == "google-0B9C1A7E-6A55-4F0B-9E43-5C1F7D2A9B10")
    let stored = try JSONDecoder().decode([String: [String: String]].self, from: persisted("1.0.7", "sync-state.json"))
    #expect(stored[connection.connectionID]?[GoogleCalendarSource.calendarSetScope] == "alice@example.com\nfamily0123@group.calendar.google.com")
}
