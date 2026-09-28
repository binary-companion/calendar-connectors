import CalendarCore
import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
import Testing

/// Answers every request to `/start` with a 302 to `/end`, and `/end` with 200 "arrived".
final class RedirectingProtocol: URLProtocol, @unchecked Sendable {
    override class func canInit(with request: URLRequest) -> Bool { request.url?.host == "redirect.test" }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let url = request.url!
        if url.path == "/start" {
            let target = URL(string: "https://redirect.test/end")!
            let response = HTTPURLResponse(url: url, statusCode: 302, httpVersion: "HTTP/1.1", headerFields: ["Location": target.absoluteString])!
            client?.urlProtocol(self, wasRedirectedTo: URLRequest(url: target), redirectResponse: response)
            // A client that declines the redirect gets this response as the task's result.
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocolDidFinishLoading(self)
        } else {
            let response = HTTPURLResponse(url: url, statusCode: 200, httpVersion: "HTTP/1.1", headerFields: [:])!
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: Data("arrived".utf8))
            client?.urlProtocolDidFinishLoading(self)
        }
    }
    override func stopLoading() {}
}

/// swift-corelibs-foundation traps when a custom `URLProtocol` reports a redirect, so these two run on Apple platforms only.
private let protocolsCanRedirect: Bool = {
    #if canImport(FoundationNetworking)
    false
    #else
    true
    #endif
}()

private func configuration() -> URLSessionConfiguration {
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [RedirectingProtocol.self]
    return configuration
}

@Test(.enabled(if: protocolsCanRedirect)) func transportCanDeclineRedirects() async throws {
    let transport = URLSessionTransport(configuration: configuration(), followsRedirects: false)
    let response = try await transport.send(HTTPRequest(url: URL(string: "https://redirect.test/start")!))
    #expect(response.status == 302)
    #expect(response.header("location") == "https://redirect.test/end")
}

@Test(.enabled(if: protocolsCanRedirect)) func transportFollowsRedirectsByDefault() async throws {
    let transport = URLSessionTransport(configuration: configuration(), followsRedirects: true)
    let response = try await transport.send(HTTPRequest(url: URL(string: "https://redirect.test/start")!))
    #expect(response.status == 200)
    #expect(String(decoding: response.body, as: UTF8.self) == "arrived")
}
