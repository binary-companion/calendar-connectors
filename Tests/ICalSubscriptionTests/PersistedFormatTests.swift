import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import ICalSubscription

// The iCal-link connection of Tests/PersistedFormats (see PersistedFormatTests in CalendarCoreTests), as 1.0.7 wrote it.

private func persisted(_ version: String, _ name: String) throws -> Data {
    try Data(contentsOf: URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("PersistedFormats").appendingPathComponent(version).appendingPathComponent(name))
}

@Test func theICalLinkConnectionOf107StillOpens() throws {
    let connection = try #require(try JSONDecoder().decode([Connection].self, from: persisted("1.0.7", "connections.json"))
        .first { $0.kindID == ICalSubscriptionKind.kindID })
    #expect(connection.config["host"] == "calendar.example.org")
    let source = try ICalSubscriptionKind(transport: FakeTransport())
        .makeSource(for: connection, credentials: InMemoryCredentialStore(), syncState: InMemorySyncStateStore())
    #expect(source.id == "icalsub-C0FFEE00-1234-4567-89AB-CDEF01234567")
}
