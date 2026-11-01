import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import CalendarOAuth

/// Counts silent requests and records each `forceRefresh`. Tokens are numbered `ext-1`, `ext-2`, ... and live for
/// `lifetime` seconds on the test clock; `failure` makes every request throw.
private actor CountingSource: ExternalTokenSource {
    private(set) var forceRefreshFlags: [Bool] = []
    private(set) var scopesSeen: [[String]] = []
    private(set) var accountsSeen: [ExternalAccount] = []
    private let now: TestNow
    private let lifetime: TimeInterval
    private let delay: Duration
    private var failure: (any Error)?

    init(now: TestNow, lifetime: TimeInterval = 3600, delay: Duration = .milliseconds(30), failure: (any Error)? = nil) {
        self.now = now
        self.lifetime = lifetime
        self.delay = delay
        self.failure = failure
    }

    var calls: Int { forceRefreshFlags.count }
    func setFailure(_ error: (any Error)?) { failure = error }

    func accessToken(for account: ExternalAccount, scopes: [String], forceRefresh: Bool) async throws -> ExternalAccessToken {
        forceRefreshFlags.append(forceRefresh)
        scopesSeen.append(scopes)
        accountsSeen.append(account)
        let number = forceRefreshFlags.count
        try await Task.sleep(for: delay)
        if let failure { throw failure }
        return ExternalAccessToken(accessToken: "ext-\(number)", expiresAt: now.date.addingTimeInterval(lifetime))
    }

    func signIn(_ request: ExternalSignInRequest) async throws -> ExternalSignInResult {
        throw SourceError.invalidResponse("the silent provider never signs in")
    }
}

private struct HostFailure: Error, CustomStringConvertible {
    let description: String
}

private let account = ExternalAccount(id: "acct-1", email: "me@example.com", connectionID: "c1")
private let scopes = ["User.Read", "Calendars.Read"]

private func provider(_ source: CountingSource, now: TestNow) -> ExternalAccessTokenProvider {
    ExternalAccessTokenProvider(account: account, scopes: scopes, source: source, now: now.provider)
}

@Test func firstCallRequestsATokenWithForceRefreshFalse() async throws {
    let now = TestNow()
    let source = CountingSource(now: now)
    let token = try await provider(source, now: now).accessToken()
    #expect(token == "ext-1")
    #expect(await source.forceRefreshFlags == [false])
    #expect(await source.scopesSeen == [scopes])
    #expect(await source.accountsSeen == [account])
}

@Test func cachedTokenIsServedUntilSixtySecondsBeforeExpiry() async throws {
    let now = TestNow()
    let source = CountingSource(now: now)
    let provider = provider(source, now: now)
    #expect(try await provider.accessToken() == "ext-1")
    now.advance(3539)
    #expect(try await provider.accessToken() == "ext-1")
    #expect(await source.calls == 1)
    now.advance(2)   // 3541 s: inside the 60 s safety margin
    #expect(try await provider.accessToken() == "ext-2")
    #expect(await source.calls == 2)
    #expect(await source.forceRefreshFlags == [false, false])
}

@Test func concurrentCallersShareOneInFlightRequest() async throws {
    let now = TestNow()
    let source = CountingSource(now: now)
    let provider = provider(source, now: now)
    let tokens = try await withThrowingTaskGroup(of: String.self) { group in
        for _ in 0..<10 { group.addTask { try await provider.accessToken() } }
        return try await group.reduce(into: [String]()) { $0.append($1) }
    }
    #expect(Set(tokens) == ["ext-1"] && tokens.count == 10)
    #expect(await source.calls == 1)
}

@Test func invalidateMakesTheNextRequestForceRefreshOnce() async throws {
    let now = TestNow()
    let source = CountingSource(now: now, lifetime: 120)
    let provider = provider(source, now: now)
    #expect(try await provider.accessToken() == "ext-1")
    await provider.invalidate()
    #expect(try await provider.accessToken() == "ext-2")
    #expect(try await provider.accessToken() == "ext-2")   // cached again
    now.advance(100)
    #expect(try await provider.accessToken() == "ext-3")   // expiry refresh: not forced
    #expect(await source.forceRefreshFlags == [false, true, false])
}

@Test func invalidateWhileARequestIsInFlightDoesNotReuseOrCacheItsAnswer() async throws {
    let now = TestNow()
    let source = CountingSource(now: now, delay: .milliseconds(80))
    let provider = provider(source, now: now)
    async let stale = provider.accessToken()
    try await Task.sleep(for: .milliseconds(20))
    await provider.invalidate()
    let fresh = try await provider.accessToken()
    _ = try await stale
    #expect(fresh == "ext-2")
    #expect(await source.forceRefreshFlags == [false, true])
    #expect(try await provider.accessToken() == "ext-2")   // the answer cached is the post-invalidate one
}

@Test func hostAuthExpiredPassesThrough() async throws {
    let now = TestNow()
    let source = CountingSource(now: now, failure: SourceError.authExpired)
    await #expect(throws: SourceError.authExpired) { _ = try await provider(source, now: now).accessToken() }
}

@Test func hostNetworkErrorPassesThrough() async throws {
    let now = TestNow()
    let source = CountingSource(now: now, failure: SourceError.network("offline"))
    await #expect(throws: SourceError.network("offline")) { _ = try await provider(source, now: now).accessToken() }
}

@Test func aFailedRequestIsNotCachedAndTheNextCallAsksAgain() async throws {
    let now = TestNow()
    let source = CountingSource(now: now, failure: SourceError.network("offline"))
    let provider = provider(source, now: now)
    await #expect(throws: SourceError.network("offline")) { _ = try await provider.accessToken() }
    await source.setFailure(nil)
    #expect(try await provider.accessToken() == "ext-2")
    #expect(await source.calls == 2)
}

@Test func cancellationPassesThrough() async throws {
    let now = TestNow()
    let source = CountingSource(now: now, failure: CancellationError())
    await #expect(throws: CancellationError.self) { _ = try await provider(source, now: now).accessToken() }
}

@Test func unknownErrorBecomesInvalidResponseWithoutItsMessage() async throws {
    let now = TestNow()
    let source = CountingSource(now: now, failure: HostFailure(description: "no consent for secret@example.com (acct-1, token-xyz)"))
    do {
        _ = try await provider(source, now: now).accessToken()
        Issue.record("expected a throw")
    } catch let error as SourceError {
        #expect(error == .invalidResponse("the Windows account service failed"))
        let text = String(describing: error)
        #expect(!text.contains("secret@example.com") && !text.contains("token-xyz"))
    }
}

@Test func alreadyExpiredTokenFromTheHostIsNotCachedAcrossCalls() async throws {
    let now = TestNow()
    let source = CountingSource(now: now, lifetime: -10)
    let provider = provider(source, now: now)
    #expect(try await provider.accessToken() == "ext-1")   // one call asks the host once: no loop
    #expect(await source.calls == 1)
    #expect(try await provider.accessToken() == "ext-2")   // the next call asks again
    #expect(await source.calls == 2)
}

@Test func theExistingProviderAdoptsTheSupplyingProtocol() async throws {
    let now = TestNow()
    let store = InMemoryCredentialStore()
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt"], for: "c1")
    let real = AccessTokenProvider(
        connectionID: "c1", credentials: store,
        refresh: { _ in OAuthTokens(accessToken: "at", expiresAt: now.date.addingTimeInterval(3600), refreshToken: nil) },
        now: now.provider)
    let supplying: any AccessTokenSupplying = real
    #expect(try await supplying.accessToken() == "at")
    await supplying.invalidate()
}
