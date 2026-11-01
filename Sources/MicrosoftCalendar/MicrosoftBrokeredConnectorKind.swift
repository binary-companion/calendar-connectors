import CalendarCore
import CalendarOAuth
import Foundation

/// Microsoft accounts whose access tokens come from the host (Windows' Web Account Manager, a mobile account
/// broker) instead of the library's browser flow. The library stores no credential for such an account: the host's
/// account service owns the password, MFA, policy and the refresh token, and the library asks for short-lived access
/// tokens. The host decides whether to offer the kind by registering it with an `ExternalTokenSource`.
///
/// A separate kind next to `MicrosoftConnectorKind` (not a mode of it), so nothing about the browser kind changes.
public struct MicrosoftBrokeredConnectorKind: ConnectorKind, AccountIdentityFamily {
    public static let kindID = "microsoft-brokered"
    /// `MicrosoftConnectorKind.scopes` without `offline_access`: the broker keeps the refresh token itself. Read-only
    /// on purpose (ADR 0017), like the browser kind.
    public static let scopes = MicrosoftConnectorKind.scopes.filter { $0 != "offline_access" }

    /// `Connection.config` keys this kind writes, besides `email`.
    static let authModeKey = "authMode"
    static let authModeExternal = "external"
    static let externalAccountIDKey = "externalAccountID"

    public var id: String { Self.kindID }
    public var displayName: String { "Microsoft" }
    public var supportedPlatforms: Platform { [.macOS, .iOS, .linux, .windows] }
    public var authorization: AuthorizationMethod { .oauth }
    /// The same mailbox through the browser kind and through this one is one account.
    public var identityFamily: String { "microsoft" }

    private let tokenSource: any ExternalTokenSource
    private let transport: any HTTPTransport
    private let now: @Sendable () -> Date
    private let sleep: Sleeper
    private let pollInterval: Duration

    public init(
        tokenSource: any ExternalTokenSource, transport: any HTTPTransport = URLSessionTransport(),
        now: @escaping @Sendable () -> Date = { Date() }, sleep: @escaping Sleeper = defaultSleeper,
        pollInterval: Duration = .seconds(60)
    ) {
        self.tokenSource = tokenSource
        self.transport = transport
        self.now = now
        self.sleep = sleep
        self.pollInterval = pollInterval
    }

    /// The host shows its own sign-in; `interaction` and `credentials` are not used. Nothing is persisted here, and
    /// nothing is stored on failure.
    public func authorize(using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        let (email, accountID) = try await signIn(ExternalSignInRequest(scopes: Self.scopes))
        return Connection(
            kindID: id, connectionID: UUID().uuidString, displayName: email,
            config: ["email": email, Self.authModeKey: Self.authModeExternal, Self.externalAccountIDKey: accountID])
    }

    public func reauthorize(
        _ connection: Connection, using interaction: any AuthorizationInteraction, credentials: any CredentialStore
    ) async throws -> Connection {
        let storedEmail = connection.config["email"]
        // A connection without an account id (see `makeSource`) is reconnected without one: the person picks.
        let account = connection.config[Self.externalAccountIDKey].flatMap { $0.isEmpty ? nil : $0 }.map {
            ExternalAccount(id: $0, email: storedEmail, connectionID: connection.connectionID)
        }
        let (email, accountID) = try await signIn(ExternalSignInRequest(account: account, email: storedEmail, scopes: Self.scopes))
        guard email == storedEmail?.lowercased() else {
            throw SourceError.invalidResponse("signed in as a different account")
        }
        var result = connection
        result.config[Self.authModeKey] = Self.authModeExternal
        result.config[Self.externalAccountIDKey] = accountID
        return result
    }

    public func makeSource(
        for connection: Connection, credentials: any CredentialStore, syncState: any SyncStateStore
    ) throws -> any CalendarSource {
        guard let accountID = connection.config[Self.externalAccountIDKey], !accountID.isEmpty else {
            throw SourceError.invalidResponse("the account settings are incomplete; sign in again")
        }
        let provider = ExternalAccessTokenProvider(
            account: ExternalAccount(id: accountID, email: connection.config["email"], connectionID: connection.connectionID),
            scopes: Self.scopes, source: tokenSource, now: now)
        let api = GraphAPIClient(transport: transport, tokens: provider, sleep: sleep)
        return MicrosoftCalendarSource(
            connection: connection, api: api, syncState: syncState,
            monitor: ChangeMonitor(interval: pollInterval, sleep: sleep), now: now)
    }

    /// Runs the host's sign-in, then asks Graph who signed in: the email comes from Graph, never from the host.
    private func signIn(_ request: ExternalSignInRequest) async throws -> (email: String, accountID: String) {
        let result: ExternalSignInResult
        do {
            result = try await tokenSource.signIn(request)
        } catch let error as SourceError {
            throw error
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            // Not the error's own description: it could carry account data.
            throw SourceError.invalidResponse("the Windows account service failed")
        }
        let api = GraphAPIClient(transport: transport, tokens: SignInToken(result.token.accessToken), sleep: sleep)
        return (try await api.accountEmail(), result.account.id)
    }

    /// The token a sign-in returned, good for the one `/me` request after it. It cannot refresh: a 401 is `authExpired`.
    private struct SignInToken: AccessTokenSupplying {
        let token: String
        init(_ token: String) { self.token = token }
        func accessToken() async throws -> String { token }
        func invalidate() async {}
    }
}
