import CalendarCore
import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

/// What the iCloud and "Other CalDAV" kinds share: sign-in (prompt, discover, store), sign-in again, and the source.
struct CalDAVAccountSetup: Sendable {
    let kindID: String
    let provider: CalendarProvider
    let fields: [CredentialField]
    /// iCloud's fixed server; nil when the user enters one.
    let fixedServer: URL?
    /// iCloud's host base (`icloud.com`, which covers the partition hosts); nil means the entered server's host.
    let fixedHostBase: String?
    let transport: any HTTPTransport
    let now: @Sendable () -> Date
    let sleep: Sleeper
    let pollInterval: Duration

    func hostBase(for server: URL) -> String { fixedHostBase ?? server.host?.lowercased() ?? "" }

    func authorize(using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        let (account, secret) = try await signIn(using: interaction)
        let connection = Connection(kindID: kindID, connectionID: UUID().uuidString, displayName: displayName(account), config: account.config)
        try await credentials.setSecrets(["username": secret.username, "password": secret.password], for: connection.connectionID)
        return connection
    }

    func reauthorize(_ connection: Connection, using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        let previous = try CalDAVAccountConfig(config: connection.config)
        let (account, secret) = try await signIn(using: interaction)
        guard account.principalURL.path == previous.principalURL.path,
              hostBase(for: account.serverURL) == hostBase(for: previous.serverURL) else {
            throw SourceError.invalidResponse("signed in as a different account")
        }
        try await credentials.setSecrets(["username": secret.username, "password": secret.password], for: connection.connectionID)
        var updated = connection
        updated.config = account.config
        return updated
    }

    func makeSource(for connection: Connection, credentials: any CredentialStore, syncState: any SyncStateStore) throws -> any CalendarSource {
        let account = try CalDAVAccountConfig(config: connection.config)
        let connectionID = connection.connectionID
        let client = WebDAVClient(transport: transport, hostBase: hostBase(for: account.serverURL), credentials: {
            guard let secrets = try await credentials.secrets(for: connectionID), let username = secrets["username"],
                  let password = secrets["password"] else { throw SourceError.authExpired }
            return WebDAVCredentials(username: username, password: password)
        })
        return CalDAVCalendarSource(
            connection: connection, account: account, client: client, provider: provider, syncState: syncState,
            monitor: ChangeMonitor(interval: pollInterval, sleep: sleep), now: now)
    }

    /// Nothing is stored here: callers store secrets only after discovery succeeds.
    private func signIn(using interaction: any AuthorizationInteraction) async throws -> (CalDAVAccountConfig, WebDAVCredentials) {
        let values = try await interaction.promptCredentials(fields)
        let username = (values["username"] ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        let password = values["password"] ?? ""
        guard !username.isEmpty, !password.isEmpty else { throw SourceError.invalidResponse("a user name and password are required") }
        let server = try fixedServer ?? Self.serverURL(from: values["serverURL"] ?? "")
        let secret = WebDAVCredentials(username: username, password: password)
        let client = WebDAVClient(transport: transport, hostBase: hostBase(for: server), credentials: { secret })
        return (try await CalDAVDiscovery(client: client).discover(serverURL: server, username: username), secret)
    }

    /// The address the user typed: `https://` is assumed when no scheme is given; plain `http` only to the loopback host.
    static func serverURL(from text: String) throws -> URL {
        var trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.contains("://") { trimmed = "https://" + trimmed }
        guard let url = URL(string: trimmed), let host = url.host?.lowercased(), !host.isEmpty, let scheme = url.scheme?.lowercased() else {
            throw SourceError.invalidResponse("the server address is not a valid URL")
        }
        guard scheme == "https" || (scheme == "http" && ["localhost", "127.0.0.1"].contains(host)) else {
            throw SourceError.invalidResponse("the server address must start with https://")
        }
        guard url.user == nil, url.password == nil else {
            throw SourceError.invalidResponse("enter the user name and password in their own fields, not in the server address")
        }
        return url
    }

    /// The Apple ID for iCloud; for another server the user name, with the host when the name is not an address.
    private func displayName(_ account: CalDAVAccountConfig) -> String {
        if fixedServer != nil || account.username.contains("@") { return account.username }
        return "\(account.username)@\(account.serverURL.host ?? "")"
    }
}

/// iCloud through CalDAV: an Apple ID and an app-specific password; the server is fixed.
public struct ICloudConnectorKind: ConnectorKind, CredentialPromptHelp {
    public static let kindID = "icloud"
    public static let serverURL = URL(string: "https://caldav.icloud.com")!
    private let setup: CalDAVAccountSetup

    public init(
        transport: any HTTPTransport = URLSessionTransport(followsRedirects: false), now: @escaping @Sendable () -> Date = { Date() },
        sleep: @escaping Sleeper = defaultSleeper, pollInterval: Duration = .seconds(60)
    ) {
        setup = CalDAVAccountSetup(
            kindID: Self.kindID, provider: .iCloud,
            fields: [CredentialField(key: "username", label: "Apple ID"), CredentialField(key: "password", label: "App-specific password", isSecret: true)],
            fixedServer: Self.serverURL, fixedHostBase: "icloud.com", transport: transport, now: now, sleep: sleep, pollInterval: pollInterval)
    }

    public var id: String { Self.kindID }
    public var displayName: String { "iCloud" }
    public var supportedPlatforms: Platform { [.macOS, .iOS, .linux, .windows] }
    public var authorization: AuthorizationMethod { .password(fields: setup.fields) }
    public var credentialHelp: CredentialHelp? {
        CredentialHelp(
            text: "iCloud needs an app-specific password, not your Apple Account password. Create one at account.apple.com under Sign-In and Security.",
            linkTitle: "Create an app-specific password", url: URL(string: "https://account.apple.com/account/manage"))
    }

    public func authorize(using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        try await setup.authorize(using: interaction, credentials: credentials)
    }

    public func reauthorize(_ connection: Connection, using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        try await setup.reauthorize(connection, using: interaction, credentials: credentials)
    }

    public func makeSource(for connection: Connection, credentials: any CredentialStore, syncState: any SyncStateStore) throws -> any CalendarSource {
        try setup.makeSource(for: connection, credentials: credentials, syncState: syncState)
    }
}

/// Any other CalDAV server (Fastmail, Nextcloud, ...): the user enters the server address, user name and password.
public struct CalDAVConnectorKind: ConnectorKind, CredentialPromptHelp {
    public static let kindID = "caldav"
    private let setup: CalDAVAccountSetup

    public init(
        transport: any HTTPTransport = URLSessionTransport(followsRedirects: false), now: @escaping @Sendable () -> Date = { Date() },
        sleep: @escaping Sleeper = defaultSleeper, pollInterval: Duration = .seconds(60)
    ) {
        setup = CalDAVAccountSetup(
            kindID: Self.kindID, provider: .calDAV,
            fields: [CredentialField(key: "serverURL", label: "Server address"), CredentialField(key: "username", label: "User name"),
                     CredentialField(key: "password", label: "Password", isSecret: true)],
            fixedServer: nil, fixedHostBase: nil, transport: transport, now: now, sleep: sleep, pollInterval: pollInterval)
    }

    public var id: String { Self.kindID }
    public var displayName: String { "Other CalDAV" }
    public var supportedPlatforms: Platform { [.macOS, .iOS, .linux, .windows] }
    public var authorization: AuthorizationMethod { .password(fields: setup.fields) }
    public var credentialHelp: CredentialHelp? {
        CredentialHelp(text: "Use the CalDAV address your provider gives you, for example https://caldav.fastmail.com. Many providers need an app password here.")
    }

    public func authorize(using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        try await setup.authorize(using: interaction, credentials: credentials)
    }

    public func reauthorize(_ connection: Connection, using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        try await setup.reauthorize(connection, using: interaction, credentials: credentials)
    }

    public func makeSource(for connection: Connection, credentials: any CredentialStore, syncState: any SyncStateStore) throws -> any CalendarSource {
        try setup.makeSource(for: connection, credentials: credentials, syncState: syncState)
    }
}
