import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import CalDAVCalendar

// The CalDAV parts of Tests/PersistedFormats (see PersistedFormatTests in CalendarCoreTests): the account settings in
// `Connection.config` and the list signature and per-calendar markers in the sync state, as 1.0.7 wrote them.

private func persisted(_ version: String, _ name: String) throws -> Data {
    try Data(contentsOf: URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("PersistedFormats").appendingPathComponent(version).appendingPathComponent(name))
}

private func connection107(_ kindID: String) throws -> Connection {
    try #require(try JSONDecoder().decode([Connection].self, from: persisted("1.0.7", "connections.json")).first { $0.kindID == kindID })
}

@Test func theCalDAVAccountsOf107StillOpen() throws {
    let icloud = try connection107("icloud")
    #expect(try CalDAVAccountConfig(config: icloud.config) == CalDAVAccountConfig(
        serverURL: URL(string: "https://caldav.icloud.com")!, username: "alice@icloud.com",
        principalURL: URL(string: "https://p01-caldav.icloud.com/123456789/principal/")!,
        homeURL: URL(string: "https://p01-caldav.icloud.com/123456789/calendars/")!,
        userAddresses: ["mailto:alice@icloud.com", "mailto:alice@me.com"], autoSchedule: true))
    let other = try connection107("caldav")
    #expect(try CalDAVAccountConfig(config: other.config) == CalDAVAccountConfig(
        serverURL: URL(string: "https://caldav.fastmail.com/")!, username: "bob",
        principalURL: URL(string: "https://caldav.fastmail.com/dav/principals/user/bob@fastmail.com/")!,
        homeURL: URL(string: "https://caldav.fastmail.com/dav/calendars/user/bob@fastmail.com/")!,
        userAddresses: ["mailto:bob@fastmail.com"], autoSchedule: false))
    let sync = InMemorySyncStateStore(), store = InMemoryCredentialStore()
    #expect(try ICloudConnectorKind(transport: FakeCalDAVServer()).makeSource(for: icloud, credentials: store, syncState: sync).id
        == "icloud-9A8B7C6D-5E4F-4A3B-2C1D-0E9F8A7B6C5D")
    #expect(try CalDAVConnectorKind(transport: FakeCalDAVServer()).makeSource(for: other, credentials: store, syncState: sync).id
        == "caldav-1F2E3D4C-5B6A-4798-8A9B-0C1D2E3F4A5B")
}

/// The same calendars give the same signature and markers as 1.0.7 stored, so an update reports no change by itself.
@Test func theCalDAVSyncStateOf107MatchesTheSameCalendars() throws {
    let stored = try #require(try JSONDecoder().decode([String: [String: String]].self, from: persisted("1.0.7", "sync-state.json"))[
        "9A8B7C6D-5E4F-4A3B-2C1D-0E9F8A7B6C5D"])
    let zone = TimeZone(identifier: "Europe/Berlin")!
    let home = URL(string: "https://p01-caldav.icloud.com/123456789/calendars/")!
    let infos = [
        CalDAVCalendarInfo(
            descriptor: CalendarDescriptor(id: "home", title: "Home", service: .calDAV, colorHex: "#1BADF8",
                                           permissions: CalendarPermissions(canViewDetails: true, canEdit: true), isDefault: true, timeZone: zone),
            url: home.appendingPathComponent("home/"), ctag: "HwoQEgwAAA", syncToken: "https://p01-caldav.icloud.com/sync/30", zone: zone),
        CalDAVCalendarInfo(
            descriptor: CalendarDescriptor(id: "shared-ro", title: "Shared (read only)", service: .calDAV,
                                           permissions: CalendarPermissions(canViewDetails: true, canEdit: false), isDefault: false, timeZone: zone),
            url: home.appendingPathComponent("shared-ro/"), ctag: "FT=-@RU=1", syncToken: nil, zone: zone),
    ]
    #expect(stored[CalDAVCalendarSource.calendarsScope] == CalDAVCalendarSource.signature(infos))
    for info in infos {
        #expect(stored[CalDAVCalendarSource.calendarScope(info.descriptor.id)] == CalDAVCalendarSource.marker(info))
    }
}
