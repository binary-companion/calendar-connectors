import CalendarOAuth
import Foundation

// oauth/pkce.json
//
// Entry point: PKCE.challenge(for:hasher:) with the library's own SHA-256 (PureSwiftSHA256, the default hasher).
//
//   input:    { "verifier": "<text>" }
//   expected: { "challenge": "<base64url of SHA-256 of the UTF-8 bytes, no padding>" }
//
// 50 verifiers: the RFC 7636 appendix B example, the empty string, strings whose UTF-8 length is at, just below and just above
// the SHA-256 block boundaries (55, 56, 63, 64, 65 bytes), verifiers of 43 and 128 characters, non-ASCII text, and
// seeded random verifiers of the PKCE alphabet (A-Z a-z 0-9 - . _ ~) of 43 to 128 characters. The random ones are drawn
// with the exporter's LCG, not with PKCE.randomString, whose element choice is the standard library's.

func exportOAuthPkce() -> [VectorCase] {
    var rng = LCG(state: 0x9BCE_5256_0009)
    let alphabet = Array("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")
    var verifiers = [
        "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk", "", "a", "abc", String(repeating: "a", count: 55), String(repeating: "a", count: 56),
        String(repeating: "a", count: 63), String(repeating: "a", count: 64), String(repeating: "a", count: 65), String(repeating: "A", count: 43),
        String(repeating: "~", count: 128), "caf\u{00E9}", "\u{4E2D}\u{6587}", "\u{1F642}", "with space", "line\nbreak",
    ]
    while verifiers.count < 50 {
        verifiers.append(String((0..<rng.int(43...128)).map { _ in alphabet[rng.below(alphabet.count)] }))
    }
    return verifiers.enumerated().map { index, verifier in
        VectorCase(name: "pkce-\(fourDigits(index + 1))", input: ["verifier": verifier], expected: ["challenge": PKCE.challenge(for: verifier)])
    }
}
