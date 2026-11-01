import CalendarCore
import CalendarOAuth
import CalendarTestSupport
import Foundation
import Testing
@testable import MicrosoftCalendar

private struct HostFailure: Error, CustomStringConvertible {
    let description: String
}

private let t0 = Date(timeIntervalSince1970: 1_800_000_000)

private func token(_ text: String = "tok-1", expiresIn seconds: TimeInterval = 3600) -> ExternalAccessToken {
    ExternalAccessToken(accessToken: text, expiresAt: t0.addingTimeInterval(seconds))
}

private func signedIn(_ accountID: String = "acct-1", email: String? = nil, token text: String = "tok-1") -> Result<ExternalSignInResult, any Error> {
    .success(ExternalSignInResult(account: ExternalAccount(id: accountID, email: email), token: token(text)))
}

private func kind(_ source: FakeExternalTokenSource, _ transport: FakeTransport, now: TestNow = TestNow(t0)) -> MicrosoftBrokeredConnectorKind {
    MicrosoftBrokeredConnectorKind(tokenSource: source, transport: transport, now: now.provider, sleep: { _ in }, pollInterval: .seconds(60))
}

private func meRoute(_ transport: FakeTransport, _ me: [String: Any] = ["mail": "me@example.com", "userPrincipalName": "me@example.onmicrosoft.com"]) async {
    await transport.route("graph.microsoft.com/v1.0/me", [.json(me)])
}

private let stored = Connection(
    kindID: "microsoft-brokered", connectionID: "c1", displayName: "me@example.com",
    config: ["email": "me@example.com", "authMode": "external", "externalAccountID": "acct-1"])

/// Routes for a source's first sync: the time zone and the calendar list.
private func syncRoutes(_ transport: FakeTransport, calendars: [HTTPResponse]? = nil) async {
    await meRoute(transport)
    await transport.route("me/mailboxSettings/timeZone", [.json(["value": "UTC"])])
    await transport.route("me/calendars?", calendars ?? [.json(calendarsJSON(["cal1"]))])
}

private func unused() -> InMemoryCredentialStore { InMemoryCredentialStore() }

@Test func authorizeSignsInThenReadsMeAndBuildsTheConnection() async throws {
    let transport = FakeTransport()
    await meRoute(transport)
    let source = FakeExternalTokenSource(signIns: [signedIn("acct-1", token: "tok-signin")])
    let store = InMemoryCredentialStore()
    let connection = try await kind(source, transport).authorize(using: NoInteraction(), credentials: store)

    #expect(connection.kindID == "microsoft-brokered" && connection.displayName == "me@example.com")
    #expect(UUID(uuidString: connection.connectionID) != nil && connection.sourceID == "microsoft-brokered-\(connection.connectionID)")
    #expect(connection.config == ["email": "me@example.com", "authMode": "external", "externalAccountID": "acct-1"])
    #expect(await store.isEmpty)

    let calls = await source.signInCalls
    #expect(calls.count == 1 && calls[0].account == nil && calls[0].email == nil)
    #expect(calls[0].scopes == MicrosoftBrokeredConnectorKind.scopes)
    let me = try #require(await transport.requests(matching: "/me").first)
    #expect(me.headers["Authorization"] == "Bearer tok-signin")
    #expect(me.url.absoluteString.contains("select=mail,userPrincipalName") || me.url.absoluteString.contains("select=mail%2CuserPrincipalName"))
    #expect(await source.tokenCalls.isEmpty)   // the sign-in token serves /me: no second request
}

@Test func authorizeUsesUserPrincipalNameWhenMailIsMissing() async throws {
    let transport = FakeTransport()
    await meRoute(transport, ["userPrincipalName": "upn@example.com"])
    let connection = try await kind(FakeExternalTokenSource(signIns: [signedIn()]), transport)
        .authorize(using: NoInteraction(), credentials: unused())
    #expect(connection.config["email"] == "upn@example.com" && connection.displayName == "upn@example.com")

    let none = FakeTransport()
    await meRoute(none, ["mail": ""])
    await #expect(throws: SourceError.invalidResponse("the account has no email address")) {
        _ = try await kind(FakeExternalTokenSource(signIns: [signedIn()]), none).authorize(using: NoInteraction(), credentials: unused())
    }
}

@Test func authorizeLowercasesTheEmail() async throws {
    let transport = FakeTransport()
    await meRoute(transport, ["mail": "Me@Example.com"])
    let connection = try await kind(FakeExternalTokenSource(signIns: [signedIn()]), transport)
        .authorize(using: NoInteraction(), credentials: unused())
    #expect(connection.config["email"] == "me@example.com" && connection.displayName == "me@example.com")
}

@Test func authorizeIgnoresAnEmailTheHostClaims() async throws {
    let transport = FakeTransport()
    await meRoute(transport)
    let connection = try await kind(FakeExternalTokenSource(signIns: [signedIn(email: "other@evil.example")]), transport)
        .authorize(using: NoInteraction(), credentials: unused())
    #expect(connection.config["email"] == "me@example.com")
}

@Test func authorizeFailureStoresNothing() async throws {
    let store = InMemoryCredentialStore()
    // The sign-in throws.
    let transport = FakeTransport()
    await meRoute(transport)
    let failing = FakeExternalTokenSource(signIns: [.failure(SourceError.network("offline"))])
    await #expect(throws: SourceError.network("offline")) {
        _ = try await kind(failing, transport).authorize(using: NoInteraction(), credentials: store)
    }
    #expect(await store.isEmpty)
    // The sign-in works, /me answers 500.
    let broken = FakeTransport()
    await broken.route("graph.microsoft.com/v1.0/me", [.text("boom", status: 500)])
    await #expect(throws: SourceError.server(status: 500)) {
        _ = try await kind(FakeExternalTokenSource(signIns: [signedIn()]), broken).authorize(using: NoInteraction(), credentials: store)
    }
    #expect(await store.isEmpty)
}

@Test func aDismissedDialogIsCancellationAndOtherHostFailuresDoNotLeakTheirText() async throws {
    let transport = FakeTransport()
    await meRoute(transport)
    await #expect(throws: CancellationError.self) {
        _ = try await kind(FakeExternalTokenSource(signIns: [.failure(CancellationError())]), transport)
            .authorize(using: NoInteraction(), credentials: unused())
    }
    let leaky = FakeExternalTokenSource(signIns: [.failure(HostFailure(description: "boom for secret@example.com"))])
    await #expect(throws: SourceError.invalidResponse("the Windows account service failed")) {
        _ = try await kind(leaky, transport).authorize(using: NoInteraction(), credentials: unused())
    }
}

@Test func reauthorizeKeepsConnectionIDAndTakesNewExternalAccountID() async throws {
    let transport = FakeTransport()
    await meRoute(transport)
    let source = FakeExternalTokenSource(signIns: [signedIn("acct-2")])
    let result = try await kind(source, transport).reauthorize(stored, using: NoInteraction(), credentials: unused())
    #expect(result.connectionID == "c1" && result.kindID == "microsoft-brokered" && result.displayName == "me@example.com")
    #expect(result.config == ["email": "me@example.com", "authMode": "external", "externalAccountID": "acct-2"])
    let request = try #require(await source.signInCalls.first)
    #expect(request.account == ExternalAccount(id: "acct-1", email: "me@example.com", connectionID: "c1"))
    #expect(request.email == "me@example.com" && request.scopes == MicrosoftBrokeredConnectorKind.scopes)
}

@Test func reauthorizeWithADifferentEmailThrowsInvalidResponse() async throws {
    let transport = FakeTransport()
    await meRoute(transport, ["mail": "other@example.com"])
    await #expect(throws: SourceError.invalidResponse("signed in as a different account")) {
        _ = try await kind(FakeExternalTokenSource(signIns: [signedIn("acct-9")]), transport)
            .reauthorize(stored, using: NoInteraction(), credentials: unused())
    }
}

@Test func reauthorizeComparesTheEmailIgnoringCase() async throws {
    let transport = FakeTransport()
    await meRoute(transport, ["mail": "ME@Example.com"])
    let result = try await kind(FakeExternalTokenSource(signIns: [signedIn("acct-2")]), transport)
        .reauthorize(stored, using: NoInteraction(), credentials: unused())
    #expect(result.config["externalAccountID"] == "acct-2")
}

@Test func reauthorizeWorksWhenTheStoredAccountIDIsMissing() async throws {
    // makeSource asks the person to sign in again for such a connection, so that sign-in has to be possible.
    let incomplete = Connection(kindID: "microsoft-brokered", connectionID: "c1", displayName: "me@example.com",
                                config: ["email": "me@example.com", "authMode": "external"])
    let transport = FakeTransport()
    await meRoute(transport)
    let source = FakeExternalTokenSource(signIns: [signedIn("acct-3")])
    let result = try await kind(source, transport).reauthorize(incomplete, using: NoInteraction(), credentials: unused())
    #expect(result.config["externalAccountID"] == "acct-3")
    let request = try #require(await source.signInCalls.first)
    #expect(request.account == nil && request.email == "me@example.com")
}

@Test func reauthorizeWithoutAStoredEmailComparesWithTheDisplayName() async throws {
    // The display name is the email by construction; a record that lost its `email` key is still guarded.
    let noEmail = Connection(kindID: "microsoft-brokered", connectionID: "c1", displayName: "me@example.com",
                             config: ["authMode": "external", "externalAccountID": "acct-1"])
    let transport = FakeTransport()
    await meRoute(transport)
    let result = try await kind(FakeExternalTokenSource(signIns: [signedIn("acct-2")]), transport)
        .reauthorize(noEmail, using: NoInteraction(), credentials: unused())
    #expect(result.config["externalAccountID"] == "acct-2" && result.config["email"] == "me@example.com")

    let other = FakeTransport()
    await meRoute(other, ["mail": "other@example.com"])
    await #expect(throws: SourceError.invalidResponse("signed in as a different account")) {
        _ = try await kind(FakeExternalTokenSource(signIns: [signedIn("acct-2")]), other)
            .reauthorize(noEmail, using: NoInteraction(), credentials: unused())
    }
}

@Test func makeSourceWithoutExternalAccountIDThrowsIncompleteSettings() throws {
    for config in [["email": "me@example.com"], ["email": "me@example.com", "externalAccountID": ""]] {
        let connection = Connection(kindID: "microsoft-brokered", connectionID: "c1", displayName: "me@example.com", config: config)
        #expect(throws: SourceError.invalidResponse("the account settings are incomplete; sign in again")) {
            _ = try kind(FakeExternalTokenSource(), FakeTransport()).makeSource(
                for: connection, credentials: InMemoryCredentialStore(), syncState: InMemorySyncStateStore())
        }
    }
}

@Test func makeSourceReturnsAMicrosoftSourceThatAsksTheHostForTokens() async throws {
    let transport = FakeTransport()
    await syncRoutes(transport)
    let source = FakeExternalTokenSource(tokens: [.success(token("tok-silent"))])
    let result = try kind(source, transport).makeSource(for: stored, credentials: unused(), syncState: InMemorySyncStateStore())
    #expect(result is MicrosoftCalendarSource && result.id == "microsoft-brokered-c1")
    #expect(try await result.calendars().map(\.id) == ["cal1"])
    let calls = await source.tokenCalls
    #expect(calls == [.init(account: ExternalAccount(id: "acct-1", email: "me@example.com", connectionID: "c1"),
                            scopes: MicrosoftBrokeredConnectorKind.scopes, forceRefresh: false)])
    #expect(await transport.requests(matching: "me/calendars").allSatisfy { $0.headers["Authorization"] == "Bearer tok-silent" })
}

@Test func sourceRefreshesSilentlyNearExpiry() async throws {
    let transport = FakeTransport()
    await syncRoutes(transport)
    let now = TestNow(t0)
    let source = FakeExternalTokenSource(tokens: [.success(token("tok-1")), .success(token("tok-2", expiresIn: 7200))])
    let result = try kind(source, transport, now: now).makeSource(for: stored, credentials: unused(), syncState: InMemorySyncStateStore())
    _ = try await result.calendars()
    now.advance(3500)
    _ = try await result.calendars()
    #expect(await source.tokenCalls.count == 1)   // still cached 100 s before expiry
    now.advance(41)   // 3541 s
    _ = try await result.calendars()
    let calls = await source.tokenCalls
    #expect(calls.count == 2 && calls.allSatisfy { !$0.forceRefresh })
    #expect(await transport.requests(matching: "me/calendars").last?.headers["Authorization"] == "Bearer tok-2")
}

@Test func a401InvalidatesOnceThenForceRefreshesThenAuthExpired() async throws {
    let transport = FakeTransport()
    await syncRoutes(transport, calendars: [.json(["error": ["code": "InvalidAuthenticationToken"]], status: 401)])
    let source = FakeExternalTokenSource(tokens: [.success(token("tok-1")), .success(token("tok-2"))])
    let result = try kind(source, transport).makeSource(for: stored, credentials: unused(), syncState: InMemorySyncStateStore())
    await #expect(throws: SourceError.authExpired) { _ = try await result.calendars() }
    #expect(await source.tokenCalls.map(\.forceRefresh) == [false, true])
}

@Test func a401ThenAFreshTokenSucceeds() async throws {
    let transport = FakeTransport()
    await syncRoutes(transport, calendars: [.json([:], status: 401), .json(calendarsJSON(["cal1"]))])
    let source = FakeExternalTokenSource(tokens: [.success(token("tok-1")), .success(token("tok-2"))])
    let result = try kind(source, transport).makeSource(for: stored, credentials: unused(), syncState: InMemorySyncStateStore())
    #expect(try await result.calendars().map(\.id) == ["cal1"])
    #expect(await source.tokenCalls.map(\.forceRefresh) == [false, true])
}

@Test func hostAuthExpiredSurfacesAsAuthExpired() async throws {
    let transport = FakeTransport()
    await syncRoutes(transport)
    let source = FakeExternalTokenSource(tokens: [.failure(SourceError.authExpired)])
    let result = try kind(source, transport).makeSource(for: stored, credentials: unused(), syncState: InMemorySyncStateStore())
    await #expect(throws: SourceError.authExpired) { _ = try await result.calendars() }
    #expect(await transport.requests.isEmpty)   // no Graph request without a token
}

@Test func networkErrorPassesThrough() async throws {
    let transport = FakeTransport()
    await syncRoutes(transport)
    let source = FakeExternalTokenSource(tokens: [.failure(SourceError.network("offline"))])
    let result = try kind(source, transport).makeSource(for: stored, credentials: unused(), syncState: InMemorySyncStateStore())
    await #expect(throws: SourceError.network("offline")) { _ = try await result.calendars() }
}

@Test func otherHostFailureIsInvalidResponse() async throws {
    let transport = FakeTransport()
    await syncRoutes(transport)
    let source = FakeExternalTokenSource(tokens: [.failure(HostFailure(description: "no consent for me@example.com"))])
    let result = try kind(source, transport).makeSource(for: stored, credentials: unused(), syncState: InMemorySyncStateStore())
    await #expect(throws: SourceError.invalidResponse("the Windows account service failed")) { _ = try await result.calendars() }
}

@Test func scopesAreASubsetOfTheBrowserKindsScopesWithoutOfflineAccess() {
    let brokered = MicrosoftBrokeredConnectorKind.scopes
    #expect(Set(brokered).isSubset(of: Set(MicrosoftConnectorKind.scopes)))
    #expect(!brokered.contains("offline_access"))
    #expect(brokered == ["User.Read", "MailboxSettings.Read", "Calendars.Read", "Calendars.Read.Shared"])
}

@Test func scopesStayReadOnly() {
    #expect(MicrosoftBrokeredConnectorKind.scopes.allSatisfy { !$0.contains("ReadWrite") })
}

@Test func theBrokeredKindDescribesItself() {
    let kind = kind(FakeExternalTokenSource(), FakeTransport())
    #expect(kind.id == "microsoft-brokered" && MicrosoftBrokeredConnectorKind.kindID == "microsoft-brokered")
    #expect(kind.displayName == "Microsoft")
    if case .oauth = kind.authorization {} else { Issue.record("expected oauth") }
    #expect(kind.supportedPlatforms == [.macOS, .iOS, .linux, .windows])
}

@Test func identityFamilyIsMicrosoftForBothKinds() {
    let browser: any ConnectorKind = MicrosoftConnectorKind(config: MicrosoftOAuthConfig(clientID: "cid"), transport: FakeTransport())
    let brokered: any ConnectorKind = kind(FakeExternalTokenSource(), FakeTransport())
    #expect((browser as? AccountIdentityFamily)?.identityFamily == "microsoft")
    #expect((brokered as? AccountIdentityFamily)?.identityFamily == "microsoft")
}

/// The brokered kind never uses the browser interaction.
private struct NoInteraction: AuthorizationInteraction {
    func beginOAuthRedirect() async throws -> any OAuthRedirectSession {
        Issue.record("the brokered flow must not open a browser")
        throw SourceError.invalidResponse("unexpected")
    }
    func promptCredentials(_ fields: [CredentialField]) async throws -> [String: String] {
        Issue.record("the brokered flow must not prompt for credentials")
        return [:]
    }
}
