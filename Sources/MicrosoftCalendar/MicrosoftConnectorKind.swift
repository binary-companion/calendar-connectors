import CalendarCore
import CalendarOAuth
import Foundation

/// The host app's Microsoft Entra app registration (a public client: no secret). Supplied from git-ignored
/// configuration; never embedded here.
public struct MicrosoftOAuthConfig: Sendable {
    public var clientID: String
    /// The host name the redirect URI is sent with. The registration lists `http://localhost` (Microsoft ignores the
    /// port for a loopback redirect), while the loopback listener reports `127.0.0.1`; the browser reaches the
    /// listener either way. nil keeps the session's own host.
    public var redirectHost: String?

    public init(clientID: String, redirectHost: String? = "localhost") {
        self.clientID = clientID
        self.redirectHost = redirectHost
    }
}

public struct MicrosoftConnectorKind: ConnectorKind {
    public static let kindID = "microsoft"
    /// `offline_access` returns the refresh token; `MailboxSettings.Read` is the account's time zone; `.Shared` reads
    /// and writes the shared and delegated calendars the account can open.
    public static let scopes = ["offline_access", "User.Read", "MailboxSettings.Read", "Calendars.ReadWrite", "Calendars.ReadWrite.Shared"]

    public var id: String { Self.kindID }
    public var displayName: String { "Microsoft" }
    public var supportedPlatforms: Platform { [.macOS, .iOS, .linux, .windows] }
    public var authorization: AuthorizationMethod { .oauth }

    private let config: MicrosoftOAuthConfig
    private let oauth: OAuthClient
    private let transport: any HTTPTransport
    private let now: @Sendable () -> Date
    private let sleep: Sleeper
    private let pollInterval: Duration
    private let hasher: any SHA256Hashing

    public init(
        config: MicrosoftOAuthConfig, transport: any HTTPTransport = URLSessionTransport(),
        now: @escaping @Sendable () -> Date = { Date() }, sleep: @escaping Sleeper = defaultSleeper,
        pollInterval: Duration = .seconds(60), hasher: any SHA256Hashing = PureSwiftSHA256()
    ) {
        self.config = config
        self.oauth = OAuthClient(
            config: OAuthConfig(
                authorizationEndpoint: URL(string: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize")!,
                tokenEndpoint: URL(string: "https://login.microsoftonline.com/common/oauth2/v2.0/token")!,
                clientID: config.clientID, scopes: Self.scopes,
                // Without this a browser already signed in to one Microsoft account signs in as it without asking.
                extraAuthParams: ["prompt": "select_account"]),
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
        let api = GraphAPIClient(transport: transport, tokens: provider, sleep: sleep)
        return MicrosoftCalendarSource(
            connection: connection, api: api, syncState: syncState,
            monitor: ChangeMonitor(interval: pollInterval, sleep: sleep), now: now)
    }

    /// The redirect URI with the configured host in place of the session's.
    private func redirectURI(for session: any OAuthRedirectSession) -> URL {
        guard let host = config.redirectHost, var components = URLComponents(url: session.redirectURI, resolvingAgainstBaseURL: false)
        else { return session.redirectURI }
        components.host = host
        return components.url ?? session.redirectURI
    }

    /// Runs the browser flow and returns the account's email address and the refresh token. Nothing is persisted here:
    /// callers store secrets only after this succeeds.
    private func signIn(using interaction: any AuthorizationInteraction) async throws -> (email: String, refreshToken: String) {
        let session = try await interaction.beginOAuthRedirect()
        let tokens: OAuthTokens
        do {
            let redirectURI = redirectURI(for: session)
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
        let api = GraphAPIClient(transport: transport, tokens: provider, sleep: sleep)
        return (try await accountEmail(api), refreshToken)
    }

    private struct MeDTO: Decodable {
        var mail: String?
        var userPrincipalName: String?
    }

    /// `mail`, else `userPrincipalName`, lowercased: the address the account is known by.
    private func accountEmail(_ api: GraphAPIClient) async throws -> String {
        let data: Data
        do {
            data = try await api.get(url: api.url(path: "/me", query: [URLQueryItem(name: "$select", value: "mail,userPrincipalName")]))
        } catch let error as GraphAPIError {
            throw error.sourceError
        }
        let me = try api.decode(MeDTO.self, from: data)
        guard let address = [me.mail, me.userPrincipalName].compactMap({ $0 }).first(where: { !$0.isEmpty }) else {
            throw SourceError.invalidResponse("the account has no email address")
        }
        return address.lowercased()
    }
}
