import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import GoogleCalendar

private struct FakeSession: OAuthRedirectSession {
    let redirectURI = URL(string: "http://127.0.0.1:53211")!
    let recorder: SessionRecorder
    let mode: Mode
    enum Mode: Sendable { case ok, denied, wrongState }

    func authorize(at authorizationURL: URL) async throws -> URL {
        await recorder.opened(authorizationURL)
        let state = URLComponents(url: authorizationURL, resolvingAgainstBaseURL: false)!.queryItems!.first { $0.name == "state" }!.value!
        switch mode {
        case .ok: return URL(string: "http://127.0.0.1:53211/?code=the-code&state=\(state)")!
        case .denied: return URL(string: "http://127.0.0.1:53211/?error=access_denied&state=\(state)")!
        case .wrongState: return URL(string: "http://127.0.0.1:53211/?code=x&state=nope")!
        }
    }
    func close() async { await recorder.closed() }
}

private actor SessionRecorder {
    private(set) var authorizationURLs: [URL] = []
    private(set) var closeCount = 0
    func opened(_ url: URL) { authorizationURLs.append(url) }
    func closed() { closeCount += 1 }
}

private struct FakeInteraction: AuthorizationInteraction {
    let recorder = SessionRecorder()
    var mode: FakeSession.Mode = .ok
    func beginOAuthRedirect() async throws -> any OAuthRedirectSession { FakeSession(recorder: recorder, mode: mode) }
    func promptCredentials(_ fields: [CredentialField]) async throws -> [String: String] { [:] }
}

private func kind(_ transport: FakeTransport, now: TestNow = TestNow()) -> GoogleConnectorKind {
    GoogleConnectorKind(
        config: GoogleOAuthConfig(clientID: "cid", clientSecret: "csecret"),
        transport: transport, now: now.provider, sleep: { _ in }, pollInterval: .seconds(60))
}

private func routes(_ transport: FakeTransport, email: String = "me@x.com", refresh: String? = "rt-1") async {
    var token: [String: Any] = ["access_token": "at", "expires_in": 3600]
    if let refresh { token["refresh_token"] = refresh }
    await transport.route("oauth2.googleapis.com/token", [.json(token)])
    await transport.route("users/me/calendarList", [.json(["items": [["id": email, "primary": true, "accessRole": "owner"]]])])
}

@Test func authorizeRunsPKCEFlowStoresRefreshTokenAndReturnsAConnection() async throws {
    let transport = FakeTransport()
    await routes(transport)
    let store = InMemoryCredentialStore()
    let interaction = FakeInteraction()
    let connection = try await kind(transport).authorize(using: interaction, credentials: store)

    #expect(connection.kindID == "google" && connection.displayName == "me@x.com" && connection.config["email"] == "me@x.com")
    #expect(connection.sourceID == "google-\(connection.connectionID)")
    #expect(UUID(uuidString: connection.connectionID) != nil)
    let secrets = try #require(try await store.secrets(for: connection.connectionID))
    #expect(secrets[AccessTokenProvider.refreshTokenKey] == "rt-1")

    let url = try #require(await interaction.recorder.authorizationURLs.first)
    let q = Dictionary(uniqueKeysWithValues: URLComponents(url: url, resolvingAgainstBaseURL: false)!.queryItems!.map { ($0.name, $0.value ?? "") })
    #expect(q["client_id"] == "cid" && q["redirect_uri"] == "http://127.0.0.1:53211")
    #expect(q["access_type"] == "offline" && q["prompt"] == "consent" && q["code_challenge_method"] == "S256")
    #expect(q["scope"] == "https://www.googleapis.com/auth/calendar.events https://www.googleapis.com/auth/calendar.calendarlist.readonly")
    #expect(await interaction.recorder.closeCount == 1)

    let exchange = try #require(await transport.requests(matching: "oauth2.googleapis.com/token").first)
    let body = String(decoding: exchange.body ?? Data(), as: UTF8.self)
    #expect(body.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A53211") && body.contains("code=the-code"))
    #expect(body.contains("client_secret=csecret"))
}

@Test func authorizeWithoutARefreshTokenFailsAndStoresNothing() async throws {
    let transport = FakeTransport()
    await routes(transport, refresh: nil)
    let store = InMemoryCredentialStore()
    let interaction = FakeInteraction()
    await #expect(throws: SourceError.self) { try await kind(transport).authorize(using: interaction, credentials: store) }
    #expect(await store.isEmpty)
    #expect(await interaction.recorder.closeCount == 1)
}

@Test func authorizeFailureAfterTokenExchangeStoresNothing() async throws {
    let transport = FakeTransport()
    await transport.route("oauth2.googleapis.com/token", [.json(["access_token": "at", "expires_in": 3600, "refresh_token": "rt"])])
    await transport.route("users/me/calendarList", [.json([:], status: 500)])
    let store = InMemoryCredentialStore()
    await #expect(throws: SourceError.server(status: 500)) {
        try await kind(transport).authorize(using: FakeInteraction(), credentials: store)
    }
    // Nothing is keyed by an id the caller never received: the store is empty for every id we could have used.
    #expect(await store.isEmpty)
}

@Test func userDenyingAccessThrowsCancelledAndClosesTheSession() async throws {
    let transport = FakeTransport()
    await routes(transport)
    let interaction = FakeInteraction(mode: .denied)
    await #expect(throws: AuthorizationError.cancelled) {
        try await kind(transport).authorize(using: interaction, credentials: InMemoryCredentialStore())
    }
    #expect(await interaction.recorder.closeCount == 1)
    #expect(await transport.requests(matching: "oauth2.googleapis.com/token").isEmpty)
}

@Test func aMismatchedStateIsRejected() async throws {
    let transport = FakeTransport()
    await routes(transport)
    await #expect(throws: AuthorizationError.stateMismatch) {
        try await kind(transport).authorize(using: FakeInteraction(mode: .wrongState), credentials: InMemoryCredentialStore())
    }
}

@Test func reauthorizeKeepsTheConnectionIDAndReplacesSecrets() async throws {
    let transport = FakeTransport()
    await routes(transport, refresh: "rt-2")
    let store = InMemoryCredentialStore()
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt-old"], for: "keep-me")
    let existing = Connection(kindID: "google", connectionID: "keep-me", displayName: "me@x.com", config: ["email": "me@x.com"])
    let result = try await kind(transport).reauthorize(existing, using: FakeInteraction(), credentials: store)
    #expect(result == existing)
    #expect(try await store.secrets(for: "keep-me")?[AccessTokenProvider.refreshTokenKey] == "rt-2")
}

@Test func reauthorizeAsADifferentAccountIsRejectedAndKeepsOldSecrets() async throws {
    let transport = FakeTransport()
    await routes(transport, email: "other@x.com", refresh: "rt-2")
    let store = InMemoryCredentialStore()
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt-old"], for: "keep-me")
    let existing = Connection(kindID: "google", connectionID: "keep-me", displayName: "me@x.com", config: ["email": "me@x.com"])
    await #expect(throws: SourceError.invalidResponse("signed in as a different account")) {
        try await kind(transport).reauthorize(existing, using: FakeInteraction(), credentials: store)
    }
    #expect(try await store.secrets(for: "keep-me")?[AccessTokenProvider.refreshTokenKey] == "rt-old")
}

@Test func makeSourceRefreshesAndListsCalendarsWithoutPrompting() async throws {
    let transport = FakeTransport()
    await transport.route("oauth2.googleapis.com/token", [.json(["access_token": "fresh", "expires_in": 3600])])
    await transport.route("users/me/calendarList", [.json(["items": [["id": "me@x.com", "primary": true, "accessRole": "owner"]]])])
    let store = InMemoryCredentialStore()
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt"], for: "c9")
    let connection = Connection(kindID: "google", connectionID: "c9", displayName: "me@x.com", config: ["email": "me@x.com"])
    let source = try kind(transport).makeSource(for: connection, credentials: store, syncState: InMemorySyncStateStore())
    #expect(source.id == "google-c9")
    #expect(try await source.calendars().map(\.id) == ["me@x.com"])
    #expect(await transport.requests(matching: "calendarList").first?.headers["Authorization"] == "Bearer fresh")
}

@Test func kindMetadata() {
    let k = kind(FakeTransport())
    #expect(k.id == "google" && k.displayName == "Google")
    #expect(k.supportedPlatforms.contains(.macOS) && k.supportedPlatforms.contains(.linux))
    if case .oauth = k.authorization {} else { Issue.record("expected oauth") }
}

@Test func authorizeMapsProviderErrorsFromTheCalendarListToSourceError() async throws {
    let transport = FakeTransport()
    await routes(transport)
    await transport.route("users/me/calendarList", [.json(["error": ["code": 404]], status: 404)])
    let store = InMemoryCredentialStore()
    await #expect(throws: SourceError.self) { try await kind(transport).authorize(using: FakeInteraction(), credentials: store) }
    #expect(await store.isEmpty)
}

@Test func reauthorizeMapsProviderErrorsFromTheCalendarListToSourceError() async throws {
    let transport = FakeTransport()
    await routes(transport, refresh: "rt-2")
    await transport.route("users/me/calendarList", [.json(["error": ["code": 404]], status: 404)])
    let store = InMemoryCredentialStore()
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt-old"], for: "keep-me")
    let existing = Connection(kindID: "google", connectionID: "keep-me", displayName: "me@x.com", config: ["email": "me@x.com"])
    await #expect(throws: SourceError.self) { try await kind(transport).reauthorize(existing, using: FakeInteraction(), credentials: store) }
    #expect(try await store.secrets(for: "keep-me")?[AccessTokenProvider.refreshTokenKey] == "rt-old")
}
