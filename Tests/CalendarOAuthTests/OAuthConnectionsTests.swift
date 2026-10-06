import Foundation
import Testing
import CalendarCore
@testable import CalendarOAuth

@Test func authorizeStoresTheRefreshTokenAndNamesTheConnectionAfterTheEmail() async throws {
    let store = InMemoryCredentialStore()
    let c = try await OAuthConnections.authorize(kindID: "google", credentials: store) { ("me@x.com", "r1") }
    #expect(c.kindID == "google" && c.displayName == "me@x.com" && c.config == ["email": "me@x.com"])
    #expect(try await store.secrets(for: c.connectionID) == [AccessTokenProvider.refreshTokenKey: "r1"])
}

@Test func authorizeStoresNothingWhenSignInFails() async {
    let store = InMemoryCredentialStore()
    await #expect(throws: SourceError.authExpired) {
        _ = try await OAuthConnections.authorize(kindID: "google", credentials: store) { throw SourceError.authExpired }
    }
}

@Test func reauthorizeReplacesTheTokenForTheSameAccount() async throws {
    let store = InMemoryCredentialStore()
    let c = Connection(kindID: "google", connectionID: "c1", displayName: "me@x.com", config: ["email": "me@x.com"])
    let out = try await OAuthConnections.reauthorize(c, credentials: store) { ("me@x.com", "r2") }
    #expect(out == c)
    #expect(try await store.secrets(for: "c1") == [AccessTokenProvider.refreshTokenKey: "r2"])
}

@Test func reauthorizeRejectsADifferentAccountWithoutStoring() async throws {
    let store = InMemoryCredentialStore()
    let c = Connection(kindID: "google", connectionID: "c1", displayName: "me@x.com", config: ["email": "me@x.com"])
    await #expect(throws: SourceError.invalidResponse("signed in as a different account")) {
        _ = try await OAuthConnections.reauthorize(c, credentials: store) { ("other@x.com", "r2") }
    }
    #expect(try await store.secrets(for: "c1") == nil)
}
