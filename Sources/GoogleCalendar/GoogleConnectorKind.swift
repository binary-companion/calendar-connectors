import CalendarCore
import CalendarOAuth
import Foundation

/// The host app's Google Cloud OAuth client (type Desktop). Supplied from git-ignored configuration; never embedded here.
public struct GoogleOAuthConfig: Sendable {
    public var clientID: String
    public var clientSecret: String
    public init(clientID: String, clientSecret: String) {
        self.clientID = clientID
        self.clientSecret = clientSecret
    }
}

public struct GoogleConnectorKind: ConnectorKind {
    public static let kindID = "google"
    public static let scopes = [
        "https://www.googleapis.com/auth/calendar.events.readonly",
        "https://www.googleapis.com/auth/calendar.calendarlist.readonly",
    ]

    public var id: String { Self.kindID }
    public var displayName: String { "Google" }
    public var supportedPlatforms: Platform { [.macOS, .iOS, .linux, .windows] }
    public var authorization: AuthorizationMethod { .oauth }

    private let oauth: OAuthClient
    private let transport: any HTTPTransport
    private let now: @Sendable () -> Date
    private let sleep: Sleeper
    private let pollInterval: Duration
    private let hasher: any SHA256Hashing

    public init(
        config: GoogleOAuthConfig, transport: any HTTPTransport = URLSessionTransport(),
        now: @escaping @Sendable () -> Date = { Date() }, sleep: @escaping Sleeper = defaultSleeper,
        pollInterval: Duration = .seconds(60), hasher: any SHA256Hashing = PureSwiftSHA256()
    ) {
        self.oauth = OAuthClient(
            config: OAuthConfig(
                authorizationEndpoint: URL(string: "https://accounts.google.com/o/oauth2/v2/auth")!,
                tokenEndpoint: URL(string: "https://oauth2.googleapis.com/token")!,
                clientID: config.clientID, clientSecret: config.clientSecret, scopes: Self.scopes,
                extraAuthParams: ["access_type": "offline", "prompt": "consent"]),
            transport: transport, now: now)
        self.transport = transport
        self.now = now
        self.sleep = sleep
        self.pollInterval = pollInterval
        self.hasher = hasher
    }

    public func authorize(using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        let (email, refreshToken) = try await signIn(using: interaction)
        let connection = Connection(
            kindID: id, connectionID: UUID().uuidString, displayName: email, config: ["email": email])
        try await credentials.setSecrets([AccessTokenProvider.refreshTokenKey: refreshToken], for: connection.connectionID)
        return connection
    }

    public func reauthorize(
        _ connection: Connection, using interaction: any AuthorizationInteraction, credentials: any CredentialStore
    ) async throws -> Connection {
        let (email, refreshToken) = try await signIn(using: interaction)
        guard email == connection.config["email"] else {
            throw SourceError.invalidResponse("signed in as a different account")
        }
        try await credentials.setSecrets([AccessTokenProvider.refreshTokenKey: refreshToken], for: connection.connectionID)
        return connection
    }

    public func makeSource(
        for connection: Connection, credentials: any CredentialStore, syncState: any SyncStateStore
    ) throws -> any CalendarSource {
        let oauth = self.oauth
        let provider = AccessTokenProvider(
            connectionID: connection.connectionID, credentials: credentials,
            refresh: { try await oauth.refresh(refreshToken: $0) }, now: now)
        let api = GoogleAPIClient(transport: transport, tokens: provider, sleep: sleep)
        return GoogleCalendarSource(
            connection: connection, api: api, syncState: syncState, monitor: ChangeMonitor(interval: pollInterval, sleep: sleep))
    }

    /// Runs the browser flow and returns the account email (the primary calendar id) and the refresh token.
    /// Nothing is persisted here: callers store secrets only after this succeeds.
    private func signIn(using interaction: any AuthorizationInteraction) async throws -> (email: String, refreshToken: String) {
        let session = try await interaction.beginOAuthRedirect()
        let tokens: OAuthTokens
        do {
            let redirectURI = session.redirectURI
            let verifier = PKCE.randomString(length: 64)
            let state = PKCE.randomString(length: 32)
            let url = oauth.authorizationURL(redirectURI: redirectURI, state: state, codeChallenge: PKCE.challenge(for: verifier, hasher: hasher))
            let redirect = try await session.authorize(at: url)
            let code = try oauth.authorizationCode(from: redirect, expectedState: state)
            tokens = try await oauth.exchange(code: code, verifier: verifier, redirectURI: redirectURI)
        } catch {
            await session.close()
            throw error
        }
        await session.close()

        guard let refreshToken = tokens.refreshToken else {
            throw SourceError.invalidResponse("no refresh token returned")
        }
        let provider = AccessTokenProvider(
            connectionID: "signin", credentials: InMemoryCredentialStore(),
            refresh: { _ in throw SourceError.authExpired }, now: now, initial: tokens)
        let api = GoogleAPIClient(transport: transport, tokens: provider, sleep: sleep)
        guard let primary = try await api.calendarList().first(where: { $0.primary == true }) else {
            throw SourceError.invalidResponse("no primary calendar")
        }
        return (primary.id.lowercased(), refreshToken)
    }
}
