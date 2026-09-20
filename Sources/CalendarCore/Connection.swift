import Foundation

public typealias ConnectionID = String

/// Non-secret description of a connected account. The host app persists it (JSON, UserDefaults, ...).
public struct Connection: Codable, Hashable, Sendable, Identifiable {
    public var kindID: String
    public var connectionID: ConnectionID
    public var displayName: String
    public var config: [String: String]

    public init(kindID: String, connectionID: ConnectionID, displayName: String, config: [String: String] = [:]) {
        self.kindID = kindID
        self.connectionID = connectionID
        self.displayName = displayName
        self.config = config
    }

    public var id: ConnectionID { connectionID }
    /// The `CalendarSource.id` for this connection. Every consumer must use this helper.
    public var sourceID: String { "\(kindID)-\(connectionID)" }
}

public struct CredentialField: Hashable, Sendable {
    public var key: String
    public var label: String
    public var isSecret: Bool
    public init(key: String, label: String, isSecret: Bool = false) {
        self.key = key
        self.label = label
        self.isSecret = isSecret
    }
}

public enum AuthorizationMethod: Sendable {
    case oauth
    case password(fields: [CredentialField])
    case system
}

public struct Platform: OptionSet, Sendable {
    public let rawValue: Int
    public init(rawValue: Int) { self.rawValue = rawValue }
    public static let macOS = Platform(rawValue: 1 << 0)
    public static let iOS = Platform(rawValue: 1 << 1)
    public static let linux = Platform(rawValue: 1 << 2)
    public static let windows = Platform(rawValue: 1 << 3)

    public static var current: Platform {
        #if os(macOS)
        return .macOS
        #elseif os(iOS)
        return .iOS
        #elseif os(Linux)
        return .linux
        #elseif os(Windows)
        return .windows
        #else
        return []
        #endif
    }
}

/// A keyed map of secrets per connection (Google stores `["refresh_token": ...]`).
/// The host supplies the real store (TimeTug: Keychain). The library ships only an in-memory one.
public protocol CredentialStore: Sendable {
    func secrets(for connectionID: ConnectionID) async throws -> [String: String]?
    func setSecrets(_ secrets: [String: String], for connectionID: ConnectionID) async throws
    func removeSecrets(for connectionID: ConnectionID) async throws
}

public protocol SyncStateStore: Sendable {
    func token(for connectionID: ConnectionID, scope: String) async -> String?
    func setToken(_ token: String?, for connectionID: ConnectionID, scope: String) async
    func removeAll(for connectionID: ConnectionID) async
}

public actor InMemoryCredentialStore: CredentialStore {
    private var storage: [ConnectionID: [String: String]] = [:]
    public init() {}
    public var isEmpty: Bool { storage.isEmpty }
    public func secrets(for connectionID: ConnectionID) async throws -> [String: String]? { storage[connectionID] }
    public func setSecrets(_ secrets: [String: String], for connectionID: ConnectionID) async throws {
        storage[connectionID] = secrets
    }
    public func removeSecrets(for connectionID: ConnectionID) async throws { storage[connectionID] = nil }
}

public actor InMemorySyncStateStore: SyncStateStore {
    private var storage: [ConnectionID: [String: String]] = [:]
    public init() {}
    public func token(for connectionID: ConnectionID, scope: String) async -> String? { storage[connectionID]?[scope] }
    public func setToken(_ token: String?, for connectionID: ConnectionID, scope: String) async {
        storage[connectionID, default: [:]][scope] = token
    }
    public func removeAll(for connectionID: ConnectionID) async { storage[connectionID] = nil }
}

/// One in-progress OAuth redirect (e.g. a loopback listener) owned by the host app.
public protocol OAuthRedirectSession: Sendable {
    /// The exact redirect URI to put in the authorization URL and the token exchange, e.g. `http://127.0.0.1:53211`.
    var redirectURI: URL { get }
    /// Opens `authorizationURL` for the user, waits for the redirect and returns the redirect URL received.
    func authorize(at authorizationURL: URL) async throws -> URL
    func close() async
}

/// Implemented by the host app: the only OS/UI-specific part of authorization.
public protocol AuthorizationInteraction: Sendable {
    func beginOAuthRedirect() async throws -> any OAuthRedirectSession
    func promptCredentials(_ fields: [CredentialField]) async throws -> [String: String]
}

public protocol ConnectorKind: Sendable {
    var id: String { get }
    var displayName: String { get }
    var supportedPlatforms: Platform { get }
    var authorization: AuthorizationMethod { get }
    func authorize(using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection
    /// Re-runs sign-in for an existing connection (after `authExpired`). Keeps its `connectionID` (so `sourceID`,
    /// calendar keys and sync state stay valid), replaces the secrets, and throws `SourceError.invalidResponse`
    /// if the user signs in as a different account.
    func reauthorize(
        _ connection: Connection, using interaction: any AuthorizationInteraction, credentials: any CredentialStore
    ) async throws -> Connection
    func makeSource(
        for connection: Connection, credentials: any CredentialStore, syncState: any SyncStateStore
    ) throws -> any CalendarSource
}

public struct ConnectorRegistry: Sendable {
    private var kinds: [String: any ConnectorKind] = [:]
    public init() {}
    public mutating func register(_ kind: any ConnectorKind) { kinds[kind.id] = kind }
    public func kind(id: String) -> (any ConnectorKind)? { kinds[id] }
    /// Kinds usable on `platform`, sorted by id.
    public func kinds(for platform: Platform) -> [any ConnectorKind] {
        kinds.values.filter { !$0.supportedPlatforms.isDisjoint(with: platform) }.sorted { $0.id < $1.id }
    }
}
