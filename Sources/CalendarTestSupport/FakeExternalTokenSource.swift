import CalendarCore
import Foundation

/// A scripted `ExternalTokenSource` for tests of connectors and of hosts. Silent requests and sign-ins each consume
/// their scripted answers in order and repeat the last one; with nothing scripted they throw
/// `SourceError.invalidResponse`. Every call is recorded.
public actor FakeExternalTokenSource: ExternalTokenSource {
    public struct TokenCall: Sendable, Equatable {
        public let account: ExternalAccount
        public let scopes: [String]
        public let forceRefresh: Bool

        public init(account: ExternalAccount, scopes: [String], forceRefresh: Bool) {
            self.account = account
            self.scopes = scopes
            self.forceRefresh = forceRefresh
        }
    }

    public private(set) var tokenCalls: [TokenCall] = []
    public private(set) var signInCalls: [ExternalSignInRequest] = []
    private var tokenScript: [Result<ExternalAccessToken, any Error>]
    private var signInScript: [Result<ExternalSignInResult, any Error>]

    public init(
        tokens: [Result<ExternalAccessToken, any Error>] = [], signIns: [Result<ExternalSignInResult, any Error>] = []
    ) {
        tokenScript = tokens
        signInScript = signIns
    }

    /// Replaces the scripted silent answers.
    public func scriptTokens(_ results: [Result<ExternalAccessToken, any Error>]) { tokenScript = results }
    /// Replaces the scripted sign-in answers.
    public func scriptSignIns(_ results: [Result<ExternalSignInResult, any Error>]) { signInScript = results }

    public func accessToken(for account: ExternalAccount, scopes: [String], forceRefresh: Bool) async throws -> ExternalAccessToken {
        tokenCalls.append(TokenCall(account: account, scopes: scopes, forceRefresh: forceRefresh))
        return try Self.next(&tokenScript).get()
    }

    public func signIn(_ request: ExternalSignInRequest) async throws -> ExternalSignInResult {
        signInCalls.append(request)
        return try Self.next(&signInScript).get()
    }

    private static func next<T>(_ script: inout [Result<T, any Error>]) -> Result<T, any Error> {
        guard let first = script.first else {
            return .failure(SourceError.invalidResponse("FakeExternalTokenSource has no scripted answer"))
        }
        if script.count > 1 { script.removeFirst() }
        return first
    }
}
