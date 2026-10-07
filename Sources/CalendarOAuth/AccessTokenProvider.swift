import CalendarCore
import Foundation

/// Hands out a valid access token, refreshing with the stored refresh token when it is about to expire.
/// Concurrent callers share one in-flight refresh. A rotated refresh token is written back to the store.
public actor AccessTokenProvider {
    public typealias RefreshFunction = @Sendable (String) async throws -> OAuthTokens
    public static let refreshTokenKey = "refresh_token"
    private static let safetyMargin: TimeInterval = 60

    private let connectionID: ConnectionID
    private let credentials: any CredentialStore
    private let refresh: RefreshFunction
    private let now: @Sendable () -> Date
    private var cached: OAuthTokens?
    private var boundRevision: UUID?
    private var inflight: Task<OAuthTokens, Error>?

    public init(
        connectionID: ConnectionID, credentials: any CredentialStore, refresh: @escaping RefreshFunction,
        now: @escaping @Sendable () -> Date, initial: OAuthTokens? = nil
    ) {
        self.connectionID = connectionID
        self.credentials = credentials
        self.refresh = refresh
        self.now = now
        self.cached = initial
    }

    public func accessToken() async throws -> String {
        if let cached, cached.expiresAt.timeIntervalSince(now()) > Self.safetyMargin {
            if let boundRevision {
                guard try await credentials.credentialSnapshot(for: connectionID)?.revision == boundRevision else {
                    self.cached = nil
                    self.boundRevision = nil
                    throw SourceError.authExpired
                }
            }
            return cached.accessToken
        }
        return try await refreshShared().accessToken
    }

    /// Drops the cached token (call after a 401) so the next `accessToken()` refreshes.
    public func invalidate() {
        cached = nil
        boundRevision = nil
    }

    private func refreshShared() async throws -> OAuthTokens {
        if let inflight { return try await inflight.value }
        // The task clears `inflight` itself when the refresh finishes, so a cancelled first caller
        // cannot let a second refresh start while this one is still running.
        let task = Task {
            do {
                let tokens = try await self.performRefresh()
                self.finishRefresh()
                return tokens
            } catch {
                self.finishRefresh()
                throw error
            }
        }
        inflight = task
        return try await task.value
    }

    private func finishRefresh() { inflight = nil }

    private func performRefresh() async throws -> OAuthTokens {
        guard let snapshot = try await credentials.credentialSnapshot(for: connectionID),
              let refreshToken = snapshot.secrets[Self.refreshTokenKey] else { throw SourceError.authExpired }
        let tokens = try await refresh(refreshToken)
        // Check and persist as one store operation: a removed or reauthorized account must not be restored
        // by an old in-flight refresh, even when its server returned a rotated token.
        guard try await credentials.updateRefreshToken(tokens.refreshToken ?? refreshToken,
                                                       for: connectionID, expectedRevision: snapshot.revision) else {
            throw SourceError.authExpired
        }
        boundRevision = snapshot.revision
        cached = tokens
        return tokens
    }
}
