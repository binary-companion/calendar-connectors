import Foundation

public enum AuthorizationError: Error, Sendable, Equatable {
    /// The user declined or closed the sign-in.
    case cancelled
    case stateMismatch
    case missingCode
}

public struct OAuthConfig: Sendable {
    public var authorizationEndpoint: URL
    public var tokenEndpoint: URL
    public var clientID: String
    /// For desktop clients this is not confidential, but the token endpoint still requires it. Supplied by the host app.
    public var clientSecret: String?
    public var scopes: [String]
    public var extraAuthParams: [String: String]

    public init(
        authorizationEndpoint: URL, tokenEndpoint: URL, clientID: String, clientSecret: String? = nil,
        scopes: [String], extraAuthParams: [String: String] = [:]
    ) {
        self.authorizationEndpoint = authorizationEndpoint
        self.tokenEndpoint = tokenEndpoint
        self.clientID = clientID
        self.clientSecret = clientSecret
        self.scopes = scopes
        self.extraAuthParams = extraAuthParams
    }
}

public struct OAuthTokens: Sendable, Equatable {
    public var accessToken: String
    public var expiresAt: Date
    public var refreshToken: String?
    public init(accessToken: String, expiresAt: Date, refreshToken: String? = nil) {
        self.accessToken = accessToken
        self.expiresAt = expiresAt
        self.refreshToken = refreshToken
    }
}

public struct OAuthClient: Sendable {
    private let config: OAuthConfig
    private let transport: any HTTPTransport
    private let now: @Sendable () -> Date

    public init(config: OAuthConfig, transport: any HTTPTransport, now: @escaping @Sendable () -> Date) {
        self.config = config
        self.transport = transport
        self.now = now
    }

    public func authorizationURL(redirectURI: URL, state: String, codeChallenge: String) -> URL {
        var items = [
            URLQueryItem(name: "client_id", value: config.clientID),
            URLQueryItem(name: "redirect_uri", value: redirectURI.absoluteString),
            URLQueryItem(name: "response_type", value: "code"),
            URLQueryItem(name: "scope", value: config.scopes.joined(separator: " ")),
            URLQueryItem(name: "state", value: state),
            URLQueryItem(name: "code_challenge", value: codeChallenge),
            URLQueryItem(name: "code_challenge_method", value: "S256"),
        ]
        for (name, value) in config.extraAuthParams.sorted(by: { $0.key < $1.key }) {
            items.append(URLQueryItem(name: name, value: value))
        }
        var components = URLComponents(url: config.authorizationEndpoint, resolvingAgainstBaseURL: false)!
        components.queryItems = items
        return components.url!
    }

    /// Extracts the authorization code from the redirect URL, validating `state`.
    public func authorizationCode(from redirect: URL, expectedState: String) throws -> String {
        let items = URLComponents(url: redirect, resolvingAgainstBaseURL: false)?.queryItems ?? []
        func value(_ name: String) -> String? { items.first { $0.name == name }?.value }
        guard value("state") == expectedState else { throw AuthorizationError.stateMismatch }
        if let error = value("error") {
            if error == "access_denied" { throw AuthorizationError.cancelled }
            throw SourceError.invalidResponse("oauth error: \(error)")
        }
        guard let code = value("code"), !code.isEmpty else { throw AuthorizationError.missingCode }
        return code
    }

    public func exchange(code: String, verifier: String, redirectURI: URL) async throws -> OAuthTokens {
        try await token(
            [
                "grant_type": "authorization_code", "code": code, "code_verifier": verifier,
                "redirect_uri": redirectURI.absoluteString,
            ], isRefresh: false)
    }

    /// Throws `SourceError.authExpired` when the refresh token is no longer valid (`invalid_grant`).
    public func refresh(refreshToken: String) async throws -> OAuthTokens {
        try await token(["grant_type": "refresh_token", "refresh_token": refreshToken], isRefresh: true)
    }

    private struct TokenResponse: Decodable {
        var accessToken: String?
        var expiresIn: Double?
        var refreshToken: String?
        var error: String?
    }

    private func token(_ params: [String: String], isRefresh: Bool) async throws -> OAuthTokens {
        var fields = params
        fields["client_id"] = config.clientID
        if let secret = config.clientSecret { fields["client_secret"] = secret }
        let body = fields.sorted { $0.key < $1.key }
            .map { "\(Self.encode($0.key))=\(Self.encode($0.value))" }.joined(separator: "&")
        let response = try await transport.send(HTTPRequest(
            url: config.tokenEndpoint, method: "POST",
            headers: ["Content-Type": "application/x-www-form-urlencoded", "Accept": "application/json"],
            body: Data(body.utf8)))
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let parsed = try? decoder.decode(TokenResponse.self, from: response.body)
        guard (200..<300).contains(response.status), let accessToken = parsed?.accessToken else {
            let error = parsed?.error ?? "HTTP \(response.status)"
            if isRefresh, error == "invalid_grant" { throw SourceError.authExpired }
            throw SourceError.invalidResponse("oauth error: \(error)")
        }
        return OAuthTokens(
            accessToken: accessToken,
            expiresAt: now().addingTimeInterval(parsed?.expiresIn ?? 3600),
            refreshToken: parsed?.refreshToken)
    }

    private static let unreserved = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-._~"))
    private static func encode(_ s: String) -> String { s.addingPercentEncoding(withAllowedCharacters: unreserved) ?? s }
}
