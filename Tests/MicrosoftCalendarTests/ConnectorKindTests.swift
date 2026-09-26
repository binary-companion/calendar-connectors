import CalendarCore
import CalendarOAuth
import CalendarTestSupport
import Foundation
import Testing
@testable import MicrosoftCalendar

private struct FakeSession: OAuthRedirectSession {
    let redirectURI = URL(string: "http://127.0.0.1:53211")!
    let recorder: SessionRecorder
    let mode: Mode
    enum Mode: Sendable { case ok, denied, wrongState }

    func authorize(at authorizationURL: URL) async throws -> URL {
        await recorder.opened(authorizationURL)
        let state = URLComponents(url: authorizationURL, resolvingAgainstBaseURL: false)!.queryItems!.first { $0.name == "state" }!.value!
        switch mode {
        case .ok: return URL(string: "http://localhost:53211/?code=the-code&state=\(state)")!
        case .denied: return URL(string: "http://localhost:53211/?error=access_denied&state=\(state)")!
        case .wrongState: return URL(string: "http://localhost:53211/?code=x&state=nope")!
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

private func kind(_ transport: FakeTransport, redirectHost: String? = "localhost", now: TestNow = TestNow()) -> MicrosoftConnectorKind {
    MicrosoftConnectorKind(
        config: MicrosoftOAuthConfig(clientID: "cid", redirectHost: redirectHost),
        transport: transport, now: now.provider, sleep: { _ in }, pollInterval: .seconds(60))
}

private func routes(_ transport: FakeTransport, me: [String: Any] = ["mail": "Me@X.com", "userPrincipalName": "me@x.onmicrosoft.com"], refresh: String? = "rt-1") async {
    var token: [String: Any] = ["access_token": "at", "expires_in": 3600]
    if let refresh { token["refresh_token"] = refresh }
    await transport.route("login.microsoftonline.com/common/oauth2/v2.0/token", [.json(token)])
    await transport.route("graph.microsoft.com/v1.0/me", [.json(me)])
}

private func query(_ url: URL) -> [String: String] {
    Dictionary(uniqueKeysWithValues: URLComponents(url: url, resolvingAgainstBaseURL: false)!.queryItems!.map { ($0.name, $0.value ?? "") })
}

@Test func authorizeRunsPKCEFlowStoresRefreshTokenAndReturnsAConnection() async throws {
    let transport = FakeTransport()
    await routes(transport)
    let store = InMemoryCredentialStore()
    let interaction = FakeInteraction()
    let connection = try await kind(transport).authorize(using: interaction, credentials: store)

    #expect(connection.kindID == "microsoft" && connection.displayName == "me@x.com" && connection.config["email"] == "me@x.com")
    #expect(connection.sourceID == "microsoft-\(connection.connectionID)" && UUID(uuidString: connection.connectionID) != nil)
    let secrets = try #require(try await store.secrets(for: connection.connectionID))
    #expect(secrets[AccessTokenProvider.refreshTokenKey] == "rt-1")

    let url = try #require(await interaction.recorder.authorizationURLs.first)
    #expect(url.host == "login.microsoftonline.com" && url.path == "/common/oauth2/v2.0/authorize")
    let q = query(url)
    #expect(q["client_id"] == "cid" && q["response_type"] == "code" && q["code_challenge_method"] == "S256")
    #expect(q["redirect_uri"] == "http://localhost:53211")   // the configured host, the session's port
    #expect(q["prompt"] == "select_account")
    #expect(q["scope"] == "offline_access User.Read MailboxSettings.Read Calendars.ReadWrite Calendars.ReadWrite.Shared")
    #expect(await interaction.recorder.closeCount == 1)

    let exchange = try #require(await transport.requests(matching: "oauth2/v2.0/token").first)
    let body = String(decoding: exchange.body ?? Data(), as: UTF8.self)
    #expect(body.contains("redirect_uri=http%3A%2F%2Flocalhost%3A53211") && body.contains("code=the-code") && body.contains("code_verifier="))
    #expect(!body.contains("client_secret"))   // a public client has none
}

@Test func theSessionsOwnHostIsUsedWhenNoRedirectHostIsConfigured() async throws {
    let transport = FakeTransport()
    await routes(transport)
    let interaction = FakeInteraction()
    _ = try await kind(transport, redirectHost: nil).authorize(using: interaction, credentials: InMemoryCredentialStore())
    #expect(query(try #require(await interaction.recorder.authorizationURLs.first))["redirect_uri"] == "http://127.0.0.1:53211")
}

@Test func theAccountIsKnownByMailElseUserPrincipalName() async throws {
    let transport = FakeTransport()
    await routes(transport, me: ["userPrincipalName": "Me@Outlook.com"])
    let connection = try await kind(transport).authorize(using: FakeInteraction(), credentials: InMemoryCredentialStore())
    #expect(connection.config["email"] == "me@outlook.com")

    let none = FakeTransport()
    await routes(none, me: ["mail": ""])
    await #expect(throws: SourceError.invalidResponse("the account has no email address")) {
        _ = try await kind(none).authorize(using: FakeInteraction(), credentials: InMemoryCredentialStore())
    }
}

@Test func authorizeWithoutARefreshTokenFailsAndStoresNothing() async throws {
    let transport = FakeTransport()
    await routes(transport, refresh: nil)
    let store = InMemoryCredentialStore()
    await #expect(throws: SourceError.invalidResponse("no refresh token returned")) {
        _ = try await kind(transport).authorize(using: FakeInteraction(), credentials: store)
    }
    #expect(await store.isEmpty)
}

@Test func aCancelledSignInThrowsCancelledAndClosesTheSession() async throws {
    let transport = FakeTransport()
    await routes(transport)
    let interaction = FakeInteraction(mode: .denied)
    await #expect(throws: AuthorizationError.cancelled) {
        _ = try await kind(transport).authorize(using: interaction, credentials: InMemoryCredentialStore())
    }
    #expect(await interaction.recorder.closeCount == 1)
    #expect(await transport.requests(matching: "oauth2/v2.0/token").isEmpty)
}

@Test func aWrongStateIsRejected() async throws {
    let transport = FakeTransport()
    await routes(transport)
    await #expect(throws: AuthorizationError.stateMismatch) {
        _ = try await kind(transport).authorize(using: FakeInteraction(mode: .wrongState), credentials: InMemoryCredentialStore())
    }
}

@Test func reauthorizeKeepsTheConnectionAndReplacesTheSecret() async throws {
    let transport = FakeTransport()
    await routes(transport, refresh: "rt-2")
    let store = InMemoryCredentialStore()
    let connection = Connection(kindID: "microsoft", connectionID: "c1", displayName: "me@x.com", config: ["email": "me@x.com"])
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "old"], for: "c1")
    let result = try await kind(transport).reauthorize(connection, using: FakeInteraction(), credentials: store)
    #expect(result == connection)
    #expect(try await store.secrets(for: "c1")?[AccessTokenProvider.refreshTokenKey] == "rt-2")
}

@Test func reauthorizeAsAnotherAccountFailsAndKeepsTheOldSecret() async throws {
    let transport = FakeTransport()
    await routes(transport, me: ["mail": "other@x.com"], refresh: "rt-2")
    let store = InMemoryCredentialStore()
    let connection = Connection(kindID: "microsoft", connectionID: "c1", displayName: "me@x.com", config: ["email": "me@x.com"])
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "old"], for: "c1")
    await #expect(throws: SourceError.invalidResponse("signed in as a different account")) {
        _ = try await kind(transport).reauthorize(connection, using: FakeInteraction(), credentials: store)
    }
    #expect(try await store.secrets(for: "c1")?[AccessTokenProvider.refreshTokenKey] == "old")
}

@Test func makeSourceRefreshesTokensWithoutASecretAndReturnsAMicrosoftSource() async throws {
    let transport = FakeTransport()
    await routes(transport)
    await transport.route("me/mailboxSettings/timeZone", [.json(["value": "UTC"])])
    await transport.route("me/calendars?", [.json(calendarsJSON(["cal1"]))])
    let store = InMemoryCredentialStore()
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt-1"], for: "c1")
    let connection = Connection(kindID: "microsoft", connectionID: "c1", displayName: "me@x.com", config: ["email": "me@x.com"])
    let source = try kind(transport).makeSource(for: connection, credentials: store, syncState: InMemorySyncStateStore())
    #expect(source is MicrosoftCalendarSource && source.id == "microsoft-c1")
    #expect(try await source.calendars().map(\.id) == ["cal1"])
    let refresh = try #require(await transport.requests(matching: "oauth2/v2.0/token").first)
    let body = String(decoding: refresh.body ?? Data(), as: UTF8.self)
    #expect(body.contains("grant_type=refresh_token") && body.contains("client_id=cid") && !body.contains("client_secret"))
}

@Test func theKindDescribesItself() {
    let kind = kind(FakeTransport())
    #expect(kind.id == "microsoft" && kind.displayName == "Microsoft")
    if case .oauth = kind.authorization {} else { Issue.record("expected oauth") }
    #expect(kind.supportedPlatforms.contains(.macOS))
}
