import Foundation
import Testing
@testable import CalendarOAuth

private func hex(_ data: Data) -> String { data.map { String(format: "%02x", $0) }.joined() }

@Test func sha256EmptyInput() {
    #expect(hex(PureSwiftSHA256().sha256(Data())) == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
}

@Test func sha256Abc() {
    #expect(hex(PureSwiftSHA256().sha256(Data("abc".utf8))) == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
}

@Test func sha256TwoBlockMessage() {
    let message = "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
    #expect(hex(PureSwiftSHA256().sha256(Data(message.utf8))) == "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1")
}

@Test func sha256MillionAs() {
    let data = Data(repeating: UInt8(ascii: "a"), count: 1_000_000)
    #expect(hex(PureSwiftSHA256().sha256(data)) == "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0")
}

@Test func sha256PaddingBoundaries() {
    // 55, 56 and 64 bytes straddle the single-block padding limits.
    #expect(hex(PureSwiftSHA256().sha256(Data(repeating: 0x61, count: 55))) == "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318")
    #expect(hex(PureSwiftSHA256().sha256(Data(repeating: 0x61, count: 56))) == "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a")
    #expect(hex(PureSwiftSHA256().sha256(Data(repeating: 0x61, count: 64))) == "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb")
}

private struct FixedHasher: SHA256Hashing {
    func sha256(_ data: Data) -> Data { Data(repeating: 0xff, count: 32) }
}

@Test func pkceUsesTheInjectedHasher() {
    // 32 bytes of 0xff in base64url without padding.
    #expect(PKCE.challenge(for: "anything", hasher: FixedHasher()) == "__________________________________________8")
}

@Test func pkceDefaultMatchesRFC7636Vector() {
    #expect(PKCE.challenge(for: "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk") == "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
}
