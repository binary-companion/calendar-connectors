import CalendarCore
import Foundation

public enum PKCE {
    private static let alphabet = Array("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")

    public static func randomString(length: Int = 64, using generator: inout some RandomNumberGenerator) -> String {
        String((0..<length).map { _ in alphabet.randomElement(using: &generator)! })
    }

    public static func randomString(length: Int = 64) -> String {
        var generator = SystemRandomNumberGenerator()
        return randomString(length: length, using: &generator)
    }

    /// base64url(SHA-256(verifier)) without padding (RFC 7636, method S256).
    public static func challenge(for verifier: String, hasher: any SHA256Hashing = PureSwiftSHA256()) -> String {
        hasher.sha256(Data(verifier.utf8)).base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
}
