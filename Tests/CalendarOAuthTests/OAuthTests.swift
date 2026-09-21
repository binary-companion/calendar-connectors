import CalendarTestSupport
import Foundation
import Testing
@testable import CalendarOAuth
import CalendarCore

private let config = OAuthConfig(
    authorizationEndpoint: URL(string: "https://accounts.example.com/auth")!,
    tokenEndpoint: URL(string: "https://oauth.example.com/token")!,
    clientID: "client-1", clientSecret: "shh",
    scopes: ["scope.a", "scope.b"], extraAuthParams: ["access_type": "offline", "prompt": "consent"])

private func client(_ transport: FakeTransport, now: TestNow = TestNow()) -> OAuthClient {
    OAuthClient(config: config, transport: transport, now: now.provider)
}

private func form(_ request: HTTPRequest) -> [String: String] {
    let body = String(decoding: request.body ?? Data(), as: UTF8.self)
    var out: [String: String] = [:]
    for pair in body.split(separator: "&") {
        let parts = pair.split(separator: "=", maxSplits: 1).map(String.init)
        out[parts[0].removingPercentEncoding ?? parts[0]] = (parts.count > 1 ? parts[1] : "").removingPercentEncoding
    }
    return out
}

@Test func authorizationURLCarriesPKCEScopesAndExtras() throws {
    let url = client(FakeTransport()).authorizationURL(
        redirectURI: URL(string: "http://127.0.0.1:5000")!, state: "st", codeChallenge: "ch")
    let items = Dictionary(uniqueKeysWithValues: URLComponents(url: url, resolvingAgainstBaseURL: false)!.queryItems!.map { ($0.name, $0.value ?? "") })
    #expect(url.host == "accounts.example.com")
    #expect(items["client_id"] == "client-1")
    #expect(items["redirect_uri"] == "http://127.0.0.1:5000")
    #expect(items["response_type"] == "code")
    #expect(items["scope"] == "scope.a scope.b")
    #expect(items["state"] == "st")
    #expect(items["code_challenge"] == "ch")
    #expect(items["code_challenge_method"] == "S256")
    #expect(items["access_type"] == "offline")
    #expect(items["prompt"] == "consent")
}

@Test func authorizationCodeValidatesStateAndErrors() throws {
    let c = client(FakeTransport())
    #expect(try c.authorizationCode(from: URL(string: "http://127.0.0.1:5000/?code=abc&state=st")!, expectedState: "st") == "abc")
    #expect(throws: AuthorizationError.stateMismatch) {
        try c.authorizationCode(from: URL(string: "http://127.0.0.1:5000/?code=abc&state=other")!, expectedState: "st")
    }
    #expect(throws: AuthorizationError.cancelled) {
        try c.authorizationCode(from: URL(string: "http://127.0.0.1:5000/?error=access_denied&state=st")!, expectedState: "st")
    }
    #expect(throws: AuthorizationError.missingCode) {
        try c.authorizationCode(from: URL(string: "http://127.0.0.1:5000/?state=st")!, expectedState: "st")
    }
    #expect(throws: SourceError.invalidResponse("oauth error: server_error")) {
        try c.authorizationCode(from: URL(string: "http://127.0.0.1:5000/?error=server_error&state=st")!, expectedState: "st")
    }
}

@Test func authorizationCodeValidatesStateBeforeErrorHandling() throws {
    let c = client(FakeTransport())
    // Forged state with access_denied error should throw stateMismatch, not cancelled
    #expect(throws: AuthorizationError.stateMismatch) {
        try c.authorizationCode(from: URL(string: "http://127.0.0.1:5000/?error=access_denied&state=forged")!, expectedState: "st")
    }
}

@Test func authorizationCodeValidatesStateBeforeMissingState() throws {
    let c = client(FakeTransport())
    // Missing state with access_denied error should throw stateMismatch, not cancelled
    #expect(throws: AuthorizationError.stateMismatch) {
        try c.authorizationCode(from: URL(string: "http://127.0.0.1:5000/?error=access_denied")!, expectedState: "st")
    }
}

@Test func exchangePostsFormAndParsesTokens() async throws {
    let transport = FakeTransport()
    await transport.route("oauth.example.com/token", [.json(["access_token": "at", "expires_in": 3600, "refresh_token": "rt"])])
    let now = TestNow()
    let tokens = try await client(transport, now: now).exchange(
        code: "the+code/1", verifier: "ver", redirectURI: URL(string: "http://127.0.0.1:5000")!)
    #expect(tokens.accessToken == "at")
    #expect(tokens.refreshToken == "rt")
    #expect(tokens.expiresAt == now.date.addingTimeInterval(3600))
    let request = try #require(await transport.requests.first)
    #expect(request.method == "POST")
    #expect(request.headers["Content-Type"] == "application/x-www-form-urlencoded")
    let f = form(request)
    #expect(f["grant_type"] == "authorization_code")
    #expect(f["code"] == "the+code/1")
    #expect(f["code_verifier"] == "ver")
    #expect(f["client_id"] == "client-1")
    #expect(f["client_secret"] == "shh")
    #expect(f["redirect_uri"] == "http://127.0.0.1:5000")
    // Verify raw body contains properly percent-encoded values
    let rawBody = String(decoding: request.body ?? Data(), as: UTF8.self)
    #expect(rawBody.contains("code=the%2Bcode%2F1"))
    #expect(rawBody.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A5000"))
}

@Test func refreshMapsInvalidGrantToAuthExpired() async throws {
    let transport = FakeTransport()
    await transport.route("token", [.json(["error": "invalid_grant"], status: 400)])
    await #expect(throws: SourceError.authExpired) {
        try await client(transport).refresh(refreshToken: "old")
    }
}

@Test func refreshSendsGrantAndKeepsNilRefreshToken() async throws {
    let transport = FakeTransport()
    await transport.route("token", [.json(["access_token": "at2", "expires_in": 100])])
    let tokens = try await client(transport).refresh(refreshToken: "rt")
    #expect(tokens.refreshToken == nil)
    let f = form(try #require(await transport.requests.first))
    #expect(f["grant_type"] == "refresh_token")
    #expect(f["refresh_token"] == "rt")
}

@Test func exchangeOtherOAuthErrorsAreInvalidResponse() async throws {
    let transport = FakeTransport()
    await transport.route("token", [.json(["error": "invalid_grant"], status: 400)])
    await #expect(throws: SourceError.invalidResponse("oauth error: invalid_grant")) {
        try await client(transport).exchange(code: "c", verifier: "v", redirectURI: URL(string: "http://127.0.0.1:1")!)
    }
}
