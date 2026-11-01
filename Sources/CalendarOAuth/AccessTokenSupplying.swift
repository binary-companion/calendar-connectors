import Foundation

/// What an API client needs from a token provider: a valid access token, and a way to say that the last one was
/// rejected. Adopted by `AccessTokenProvider` (refresh token held by the library) and `ExternalAccessTokenProvider`
/// (tokens supplied by the host).
public protocol AccessTokenSupplying: Sendable {
    func accessToken() async throws -> String
    /// Drops the cached token (call after a 401) so the next `accessToken()` asks for a fresh one.
    func invalidate() async
}

extension AccessTokenProvider: AccessTokenSupplying {}
