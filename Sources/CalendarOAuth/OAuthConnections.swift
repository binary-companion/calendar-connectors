import CalendarCore
import Foundation

/// The authorize and reauthorize flows the OAuth connectors share. `signIn` runs the provider's browser flow and
/// returns the account email and refresh token; nothing is persisted until it succeeds.
public enum OAuthConnections {
    public static func authorize(
        kindID: String, credentials: any CredentialStore,
        signIn: () async throws -> (email: String, refreshToken: String)
    ) async throws -> Connection {
        let (email, refreshToken) = try await signIn()
        let connection = Connection(
            kindID: kindID, connectionID: UUID().uuidString, displayName: email, config: ["email": email])
        try await credentials.setSecrets([AccessTokenProvider.refreshTokenKey: refreshToken], for: connection.connectionID)
        return connection
    }

    public static func reauthorize(
        _ connection: Connection, credentials: any CredentialStore,
        signIn: () async throws -> (email: String, refreshToken: String)
    ) async throws -> Connection {
        let (email, refreshToken) = try await signIn()
        guard email == connection.config["email"] else {
            throw SourceError.invalidResponse("signed in as a different account")
        }
        try await credentials.setSecrets([AccessTokenProvider.refreshTokenKey: refreshToken], for: connection.connectionID)
        return connection
    }
}
