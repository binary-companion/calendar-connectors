import Foundation
import Testing
@testable import CalendarCore

// Tests/PersistedFormats/<version> holds files that release's own code wrote: the connection list and the sync state
// a host keeps on users' disks. They are add-only (AGENTS.md, "Backward compatibility"): the current code must read
// every one of them, with the same meaning, for as long as the major version lasts.

private func persisted(_ version: String, _ name: String) throws -> URL {
    let source = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("PersistedFormats").appendingPathComponent(version).appendingPathComponent(name)
    // A copy, so a store that writes can never touch the committed file.
    let copy = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).appendingPathComponent(name)
    try FileManager.default.createDirectory(at: copy.deletingLastPathComponent(), withIntermediateDirectories: true)
    try FileManager.default.copyItem(at: source, to: copy)
    return copy
}

private let connections107 = [
    Connection(kindID: "google", connectionID: "0B9C1A7E-6A55-4F0B-9E43-5C1F7D2A9B10", displayName: "alice@example.com",
               config: ["email": "alice@example.com"]),
    Connection(kindID: "microsoft", connectionID: "5D2E8F4A-1B3C-4D6E-8F90-A1B2C3D4E5F6", displayName: "Carol@Contoso.com",
               config: ["email": "Carol@Contoso.com"]),
    Connection(kindID: "icloud", connectionID: "9A8B7C6D-5E4F-4A3B-2C1D-0E9F8A7B6C5D", displayName: "alice@icloud.com", config: [
        "autoSchedule": "true", "homeURL": "https://p01-caldav.icloud.com/123456789/calendars/",
        "principalURL": "https://p01-caldav.icloud.com/123456789/principal/", "serverURL": "https://caldav.icloud.com",
        "userAddresses": "mailto:alice@icloud.com\nmailto:alice@me.com", "username": "alice@icloud.com",
    ]),
    Connection(kindID: "caldav", connectionID: "1F2E3D4C-5B6A-4798-8A9B-0C1D2E3F4A5B", displayName: "bob@caldav.fastmail.com", config: [
        "autoSchedule": "false", "homeURL": "https://caldav.fastmail.com/dav/calendars/user/bob@fastmail.com/",
        "principalURL": "https://caldav.fastmail.com/dav/principals/user/bob@fastmail.com/", "serverURL": "https://caldav.fastmail.com/",
        "userAddresses": "mailto:bob@fastmail.com", "username": "bob",
    ]),
    Connection(kindID: "icalsub", connectionID: "C0FFEE00-1234-4567-89AB-CDEF01234567", displayName: "Team offsites",
               config: ["host": "calendar.example.org"]),
]

@Test func theConnectionListOf107Loads() async throws {
    let store = FileConnectionStore(url: try persisted("1.0.7", "connections.json"))
    #expect(await store.load() == .loaded(connections107))
    #expect(connections107.map(\.sourceID).first == "google-0B9C1A7E-6A55-4F0B-9E43-5C1F7D2A9B10")
}

@Test func theConnectionListOf107StaysEditable() async throws {
    let url = try persisted("1.0.7", "connections.json")
    let store = FileConnectionStore(url: url)
    try await store.remove(connectionID: "C0FFEE00-1234-4567-89AB-CDEF01234567")
    #expect(await FileConnectionStore(url: url).load() == .loaded(Array(connections107.dropLast())))
}

@Test func theSyncStateOf107Loads() async throws {
    let url = try persisted("1.0.7", "sync-state.json")
    let store = FileSyncStateStore(url: url)
    let google = "0B9C1A7E-6A55-4F0B-9E43-5C1F7D2A9B10", microsoft = "5D2E8F4A-1B3C-4D6E-8F90-A1B2C3D4E5F6"
    let apple = "9A8B7C6D-5E4F-4A3B-2C1D-0E9F8A7B6C5D"
    #expect(await store.token(for: google, scope: "_calendars") == "alice@example.com\nfamily0123@group.calendar.google.com")
    #expect(await store.token(for: google, scope: "alice@example.com") == "CPDAlvWDx70CEPDAlvWDx70CGAU=")
    #expect(await store.token(for: google, scope: "family0123@group.calendar.google.com") == "CKiZ1P6Dx70CEKiZ1P6Dx70CGAU=")
    #expect(await store.token(for: microsoft, scope: "_calendars") == "AAMkAGI2TAAA=\nAAMkAGI2TBBB=")
    #expect(await store.token(for: microsoft, scope: "AAMkAGI2TAAA=")?.hasPrefix("1790000000|https://graph.microsoft.com/") == true)
    #expect(await store.token(for: apple, scope: "calendar:home") == "sync:https://p01-caldav.icloud.com/sync/30")
    #expect(await store.token(for: apple, scope: "calendar:shared-ro") == "ctag:FT=-@RU=1")
    // A write keeps everything else that was stored.
    await store.removeAll(for: apple)
    let reread = FileSyncStateStore(url: url)
    #expect(await reread.token(for: google, scope: "alice@example.com") == "CPDAlvWDx70CEPDAlvWDx70CGAU=")
    #expect(await reread.token(for: apple, scope: "calendar:home") == nil)
}
