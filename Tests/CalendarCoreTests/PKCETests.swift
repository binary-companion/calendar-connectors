import Foundation
import Testing
@testable import CalendarCore

@Test func pkceChallengeMatchesRFC7636Vector() {
    #expect(PKCE.challenge(for: "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk")
        == "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
}

@Test func pkceRandomStringUsesUnreservedAlphabetAndLength() {
    let s = PKCE.randomString(length: 64)
    #expect(s.count == 64)
    let allowed = Set("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")
    #expect(s.allSatisfy { allowed.contains($0) })
    #expect(PKCE.randomString(length: 64) != s)
}
