import Foundation

// The seam for hosts that get access tokens from the operating system's own account service (Windows' Web Account
// Manager, a mobile account broker) instead of running the OAuth browser flow. The library never sees a password
// or a refresh token for such an account; it only asks the host for short-lived access tokens.

/// An account the host's account service knows. `id` is the host's opaque identifier and is authoritative; `email`
/// and `connectionID` are informational (a host can use them to find the account when its id is not enough).
public struct ExternalAccount: Hashable, Sendable {
    public var id: String
    public var email: String?
    public var connectionID: ConnectionID?

    public init(id: String, email: String? = nil, connectionID: ConnectionID? = nil) {
        self.id = id
        self.email = email
        self.connectionID = connectionID
    }
}

public struct ExternalAccessToken: Equatable, Sendable {
    public var accessToken: String
    public var expiresAt: Date

    public init(accessToken: String, expiresAt: Date) {
        self.accessToken = accessToken
        self.expiresAt = expiresAt
    }
}

/// An interactive sign-in. `account` nil is a first sign-in (the host lets the person pick or confirm an account);
/// a given account is a reconnect, with `email` as a hint for the host's account picker.
public struct ExternalSignInRequest: Sendable {
    public var account: ExternalAccount?
    public var email: String?
    public var scopes: [String]

    public init(account: ExternalAccount? = nil, email: String? = nil, scopes: [String]) {
        self.account = account
        self.email = email
        self.scopes = scopes
    }
}

public struct ExternalSignInResult: Sendable {
    public var account: ExternalAccount
    public var token: ExternalAccessToken

    public init(account: ExternalAccount, token: ExternalAccessToken) {
        self.account = account
        self.token = token
    }
}

/// Implemented by the host. Errors follow the library's usual mapping: `SourceError.authExpired` when the person must
/// sign in again, `SourceError.network` when offline, `CancellationError` when the person dismisses the dialog.
public protocol ExternalTokenSource: Sendable {
    /// A token without any user interface. Throws `SourceError.authExpired` when interaction is needed.
    func accessToken(for account: ExternalAccount, scopes: [String], forceRefresh: Bool) async throws -> ExternalAccessToken
    /// A token with the host's own sign-in user interface.
    func signIn(_ request: ExternalSignInRequest) async throws -> ExternalSignInResult
}

/// Adopted by a `ConnectorKind` whose accounts are the same people as another kind's (Microsoft in the browser and
/// Microsoft through the host's broker). Kinds with the same family and the same account name are one account.
/// Optional: hosts check with `as?`, and a kind without it is its own family (its kind id).
public protocol AccountIdentityFamily {
    var identityFamily: String { get }
}
