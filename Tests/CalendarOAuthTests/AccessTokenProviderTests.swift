import CalendarTestSupport
import Foundation
import Testing
@testable import CalendarOAuth
import CalendarCore

private actor Refresher {
    private(set) var calls = 0
    private(set) var seen: [String] = []
    let now: TestNow
    let rotate: Bool
    init(now: TestNow, rotate: Bool = false) { self.now = now; self.rotate = rotate }

    func refresh(_ token: String) async throws -> OAuthTokens {
        calls += 1
        seen.append(token)
        try await Task.sleep(for: .milliseconds(30))
        return OAuthTokens(
            accessToken: "at\(calls)", expiresAt: now.date.addingTimeInterval(3600),
            refreshToken: rotate ? "rt\(calls)" : nil)
    }
}

private func makeProvider(
    now: TestNow, refresher: Refresher, store: InMemoryCredentialStore, initial: OAuthTokens? = nil
) -> AccessTokenProvider {
    AccessTokenProvider(
        connectionID: "c1", credentials: store, refresh: { try await refresher.refresh($0) },
        now: now.provider, initial: initial)
}

@Test func cachesUntilNearExpiry() async throws {
    let now = TestNow()
    let store = InMemoryCredentialStore()
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt0"], for: "c1")
    let refresher = Refresher(now: now)
    let provider = makeProvider(now: now, refresher: refresher, store: store)
    #expect(try await provider.accessToken() == "at1")
    #expect(try await provider.accessToken() == "at1")
    #expect(await refresher.calls == 1)
    now.advance(3600 - 30) // inside the 60 s safety margin
    #expect(try await provider.accessToken() == "at2")
    #expect(await refresher.calls == 2)
}

@Test func concurrentCallersShareOneRefresh() async throws {
    let now = TestNow()
    let store = InMemoryCredentialStore()
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt0"], for: "c1")
    let refresher = Refresher(now: now)
    let provider = makeProvider(now: now, refresher: refresher, store: store)
    let tokens = try await withThrowingTaskGroup(of: String.self) { group in
        for _ in 0..<10 { group.addTask { try await provider.accessToken() } }
        return try await group.reduce(into: [String]()) { $0.append($1) }
    }
    #expect(Set(tokens) == ["at1"])
    #expect(await refresher.calls == 1)
}

@Test func rotatedRefreshTokenIsPersisted() async throws {
    let now = TestNow()
    let store = InMemoryCredentialStore()
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt0", "other": "keep"], for: "c1")
    let refresher = Refresher(now: now, rotate: true)
    let provider = makeProvider(now: now, refresher: refresher, store: store)
    _ = try await provider.accessToken()
    let secrets = try #require(try await store.secrets(for: "c1"))
    #expect(secrets[AccessTokenProvider.refreshTokenKey] == "rt1")
    #expect(secrets["other"] == "keep")
    #expect(await refresher.seen == ["rt0"])
}

@Test func missingRefreshTokenIsAuthExpired() async {
    let now = TestNow()
    let provider = makeProvider(now: now, refresher: Refresher(now: now), store: InMemoryCredentialStore())
    await #expect(throws: SourceError.authExpired) { try await provider.accessToken() }
}

@Test func invalidateForcesARefresh() async throws {
    let now = TestNow()
    let store = InMemoryCredentialStore()
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt0"], for: "c1")
    let refresher = Refresher(now: now)
    let provider = makeProvider(now: now, refresher: refresher, store: store)
    _ = try await provider.accessToken()
    await provider.invalidate()
    #expect(try await provider.accessToken() == "at2")
}

@Test func initialTokensAreUsedWithoutTouchingTheStore() async throws {
    let now = TestNow()
    let refresher = Refresher(now: now)
    let initial = OAuthTokens(accessToken: "seed", expiresAt: now.date.addingTimeInterval(3600))
    let provider = makeProvider(now: now, refresher: refresher, store: InMemoryCredentialStore(), initial: initial)
    #expect(try await provider.accessToken() == "seed")
    #expect(await refresher.calls == 0)
}

@Test func cancellingTheFirstCallerKeepsSingleFlight() async throws {
    let now = TestNow()
    let store = InMemoryCredentialStore()
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt0"], for: "c1")
    let refresher = Refresher(now: now)
    let provider = makeProvider(now: now, refresher: refresher, store: store)
    let first = Task { try await provider.accessToken() }
    for _ in 0..<200 where await refresher.calls == 0 { try await Task.sleep(for: .milliseconds(1)) }
    #expect(await refresher.calls == 1)
    first.cancel()
    let token = try await provider.accessToken()
    #expect(token == "at1")
    #expect(await refresher.calls == 1)
    _ = try? await first.value
}

@Test func rotationDoesNotOverwriteEntriesWrittenDuringRefresh() async throws {
    let now = TestNow()
    let store = InMemoryCredentialStore()
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt0", "other": "keep"], for: "c1")
    let provider = AccessTokenProvider(
        connectionID: "c1", credentials: store,
        refresh: { _ in
            try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt0", "other": "keep", "other2": "x"], for: "c1")
            return OAuthTokens(accessToken: "at", expiresAt: now.date.addingTimeInterval(3600), refreshToken: "rt1")
        },
        now: now.provider)
    _ = try await provider.accessToken()
    let secrets = try #require(try await store.secrets(for: "c1"))
    #expect(secrets[AccessTokenProvider.refreshTokenKey] == "rt1")
    #expect(secrets["other"] == "keep")
    #expect(secrets["other2"] == "x")
}
