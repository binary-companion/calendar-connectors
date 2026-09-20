import Foundation
import Testing
@testable import CalendarCore

private func tempURL(_ name: String = "store.json") -> URL {
    FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).appendingPathComponent(name)
}
private let alice = Connection(kindID: "google", connectionID: "c1", displayName: "alice@example.com", config: ["email": "alice@example.com"])
private let bob = Connection(kindID: "google", connectionID: "c2", displayName: "bob@example.com")

@Test func connectionStoreMissingFileIsMissing() async {
    #expect(await FileConnectionStore(url: tempURL()).load() == .missing)
}

@Test func connectionStoreRoundTripsAddReplaceRemove() async throws {
    let store = FileConnectionStore(url: tempURL())
    try await store.add(alice)
    try await store.add(bob)
    #expect(await store.connections() == [alice, bob])
    var renamed = alice; renamed.displayName = "alice2@example.com"
    try await store.add(renamed)
    #expect(await store.connections() == [renamed, bob])
    try await store.remove(connectionID: "c1")
    #expect(await store.load() == .loaded([bob]))
    try await store.remove(connectionID: "c1")   // idempotent
    #expect(await store.connections() == [bob])
}

@Test func connectionStoreUnreadableFileIsReportedAndNeverOverwritten() async throws {
    let url = tempURL()
    try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
    try Data("not json".utf8).write(to: url)
    let store = FileConnectionStore(url: url)
    #expect(await store.load() == .unreadable)
    await #expect(throws: FileStoreError.unreadable) { try await store.add(alice) }
    #expect(try Data(contentsOf: url) == Data("not json".utf8))
}

@Test func syncStateRoundTripsAcrossInstancesAndRemoveAll() async {
    let url = tempURL("sync.json")
    let a = FileSyncStateStore(url: url)
    await a.setToken("t1", for: "c1", scope: "cal-a")
    await a.setToken("t2", for: "c1", scope: "cal-b")
    await a.setToken("t3", for: "c2", scope: "cal-a")
    let b = FileSyncStateStore(url: url)
    #expect(await b.token(for: "c1", scope: "cal-a") == "t1")
    await b.setToken(nil, for: "c1", scope: "cal-a")
    #expect(await b.token(for: "c1", scope: "cal-a") == nil)
    await b.removeAll(for: "c1")
    #expect(await FileSyncStateStore(url: url).token(for: "c1", scope: "cal-b") == nil)
    #expect(await FileSyncStateStore(url: url).token(for: "c2", scope: "cal-a") == "t3")
}

@Test func syncStateUnreadableFileMeansNoTokens() async throws {
    let url = tempURL("sync.json")
    try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
    try Data("###".utf8).write(to: url)
    let store = FileSyncStateStore(url: url)
    #expect(await store.token(for: "c1", scope: "x") == nil)
    await store.setToken("t", for: "c1", scope: "x")
    #expect(await FileSyncStateStore(url: url).token(for: "c1", scope: "x") == "t")
}
