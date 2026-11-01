import Foundation
import Testing
@testable import CalendarCore

private struct StubKind: ConnectorKind {
    var id: String { "stub" }
    var displayName: String { "Stub" }
    var supportedPlatforms: Platform { [.windows] }
    var authorization: AuthorizationMethod { .system }
    func authorize(using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        throw SourceError.notFound
    }
    func reauthorize(
        _ connection: Connection, using interaction: any AuthorizationInteraction, credentials: any CredentialStore
    ) async throws -> Connection { connection }
    func makeSource(for connection: Connection, credentials: any CredentialStore, syncState: any SyncStateStore) throws -> any CalendarSource {
        throw SourceError.notFound
    }
}

private struct FamilyKind: ConnectorKind, AccountIdentityFamily {
    var identityFamily: String { "family" }
    var id: String { "family-kind" }
    var displayName: String { "Family" }
    var supportedPlatforms: Platform { [.windows] }
    var authorization: AuthorizationMethod { .system }
    func authorize(using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        throw SourceError.notFound
    }
    func reauthorize(
        _ connection: Connection, using interaction: any AuthorizationInteraction, credentials: any CredentialStore
    ) async throws -> Connection { connection }
    func makeSource(for connection: Connection, credentials: any CredentialStore, syncState: any SyncStateStore) throws -> any CalendarSource {
        throw SourceError.notFound
    }
}

@Test func externalAccountInitDefaultsOptionalFieldsToNil() {
    let account = ExternalAccount(id: "acct-1")
    #expect(account.id == "acct-1")
    #expect(account.email == nil)
    #expect(account.connectionID == nil)
    let full = ExternalAccount(id: "acct-1", email: "me@example.com", connectionID: "c1")
    #expect(full.email == "me@example.com" && full.connectionID == "c1")
    #expect(full != account)
    #expect(Set([account, ExternalAccount(id: "acct-1")]).count == 1)
}

@Test func signInRequestWithoutAccountIsAFirstSignIn() {
    let request = ExternalSignInRequest(scopes: ["User.Read"])
    #expect(request.account == nil)
    #expect(request.email == nil)
    #expect(request.scopes == ["User.Read"])
    let again = ExternalSignInRequest(account: ExternalAccount(id: "a"), email: "me@example.com", scopes: ["User.Read"])
    #expect(again.account?.id == "a" && again.email == "me@example.com")
}

@Test func accessTokenAndSignInResultCarryTheirFields() {
    let expiry = Date(timeIntervalSince1970: 1_800_000_000)
    let token = ExternalAccessToken(accessToken: "t", expiresAt: expiry)
    #expect(token == ExternalAccessToken(accessToken: "t", expiresAt: expiry))
    #expect(token != ExternalAccessToken(accessToken: "t", expiresAt: expiry.addingTimeInterval(1)))
    let result = ExternalSignInResult(account: ExternalAccount(id: "a"), token: token)
    #expect(result.account.id == "a" && result.token == token)
}

@Test func kindWithoutIdentityFamilyYieldsNilOnCast() {
    let kind: any ConnectorKind = StubKind()
    #expect((kind as? AccountIdentityFamily) == nil)
    let family: any ConnectorKind = FamilyKind()
    #expect((family as? AccountIdentityFamily)?.identityFamily == "family")
}
