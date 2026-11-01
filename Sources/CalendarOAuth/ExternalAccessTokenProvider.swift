import CalendarCore
import Foundation

/// Hands out a valid access token from a host-supplied `ExternalTokenSource`. Behaves like `AccessTokenProvider`
/// where that applies: a cached token until 60 s before it expires, and concurrent callers share one in-flight
/// request. After `invalidate()` the next request passes `forceRefresh: true` (once).
///
/// Errors: `SourceError` and `CancellationError` pass through; anything else becomes
/// `SourceError.invalidResponse("the Windows account service failed")` without the underlying description, which
/// could carry account data.
public actor ExternalAccessTokenProvider: AccessTokenSupplying {
    private static let safetyMargin: TimeInterval = 60

    private let account: ExternalAccount
    private let scopes: [String]
    private let source: any ExternalTokenSource
    private let now: @Sendable () -> Date
    private var cached: ExternalAccessToken?
    private var forceNextRequest = false
    private var inflight: Task<ExternalAccessToken, Error>?
    /// Bumped by `invalidate()`: an answer to a request that started before it is returned to its callers but
    /// neither cached nor shared with later callers.
    private var generation = 0

    public init(
        account: ExternalAccount, scopes: [String], source: any ExternalTokenSource,
        now: @escaping @Sendable () -> Date
    ) {
        self.account = account
        self.scopes = scopes
        self.source = source
        self.now = now
    }

    public func accessToken() async throws -> String {
        if let cached, cached.expiresAt.timeIntervalSince(now()) > Self.safetyMargin {
            return cached.accessToken
        }
        return try await requestShared().accessToken
    }

    public func invalidate() {
        cached = nil
        forceNextRequest = true
        inflight = nil
        generation += 1
    }

    private func requestShared() async throws -> ExternalAccessToken {
        if let inflight { return try await inflight.value }
        let forceRefresh = forceNextRequest
        forceNextRequest = false
        let started = generation
        // The task clears `inflight` itself when it finishes, so a cancelled first caller cannot let a second
        // request start while this one is still running.
        let task = Task {
            do {
                let token = try await self.performRequest(forceRefresh: forceRefresh)
                self.finish(started, token)
                return token
            } catch {
                self.finish(started, nil)
                throw error
            }
        }
        inflight = task
        return try await task.value
    }

    private func finish(_ started: Int, _ token: ExternalAccessToken?) {
        guard started == generation else { return }
        inflight = nil
        if let token, token.expiresAt.timeIntervalSince(now()) > Self.safetyMargin { cached = token }
    }

    private func performRequest(forceRefresh: Bool) async throws -> ExternalAccessToken {
        do {
            return try await source.accessToken(for: account, scopes: scopes, forceRefresh: forceRefresh)
        } catch let error as SourceError {
            throw error
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            throw SourceError.invalidResponse("the Windows account service failed")
        }
    }
}
