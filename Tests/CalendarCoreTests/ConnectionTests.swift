import Foundation
import Testing
@testable import CalendarCore

@Test func connectionRoundTripsAndDerivesSourceID() throws {
    let c = Connection(kindID: "google", connectionID: "abc", displayName: "me@x.com", config: ["email": "me@x.com"])
    let data = try JSONEncoder().encode(c)
    #expect(try JSONDecoder().decode(Connection.self, from: data) == c)
    #expect(c.sourceID == "google-abc")
    #expect(c.id == "abc")
}

@Test func connectionDecodesWithUnknownKeys() throws {
    let json = #"{"kindID":"google","connectionID":"a","displayName":"d","config":{},"future":1}"#
    let c = try JSONDecoder().decode(Connection.self, from: Data(json.utf8))
    #expect(c.kindID == "google")
}

@Test func inMemoryCredentialStoreStoresAndRemoves() async throws {
    let store = InMemoryCredentialStore()
    #expect(try await store.secrets(for: "c1") == nil)
    try await store.setSecrets(["refresh_token": "r"], for: "c1")
    #expect(try await store.secrets(for: "c1") == ["refresh_token": "r"])
    try await store.removeSecrets(for: "c1")
    #expect(try await store.secrets(for: "c1") == nil)
}

@Test func inMemorySyncStateScopesTokensAndRemovesAll() async {
    let store = InMemorySyncStateStore()
    await store.setToken("t1", for: "c1", scope: "calA")
    await store.setToken("t2", for: "c1", scope: "calB")
    await store.setToken("t3", for: "c2", scope: "calA")
    #expect(await store.token(for: "c1", scope: "calA") == "t1")
    await store.setToken(nil, for: "c1", scope: "calA")
    #expect(await store.token(for: "c1", scope: "calA") == nil)
    await store.removeAll(for: "c1")
    #expect(await store.token(for: "c1", scope: "calB") == nil)
    #expect(await store.token(for: "c2", scope: "calA") == "t3")
}

private struct StubKind: ConnectorKind {
    let id: String
    let supportedPlatforms: Platform
    var displayName: String { id }
    var authorization: AuthorizationMethod { .oauth }
    func authorize(using: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        Connection(kindID: id, connectionID: "x", displayName: "x")
    }
    func reauthorize(_ connection: Connection, using: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection { connection }
    func makeSource(for connection: Connection, credentials: any CredentialStore, syncState: any SyncStateStore) throws -> any CalendarSource {
        throw SourceError.invalidResponse("stub")
    }
}

@Test func registryFiltersByPlatform() {
    var registry = ConnectorRegistry()
    registry.register(StubKind(id: "google", supportedPlatforms: [.macOS, .linux]))
    registry.register(StubKind(id: "eventkit", supportedPlatforms: [.macOS]))
    #expect(registry.kind(id: "google")?.id == "google")
    #expect(registry.kind(id: "missing") == nil)
    #expect(Set(registry.kinds(for: .macOS).map(\.id)) == ["google", "eventkit"])
    #expect(registry.kinds(for: .linux).map(\.id) == ["google"])
}
