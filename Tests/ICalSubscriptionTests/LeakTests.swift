import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import ICalSubscription

private struct ThrowingTransport: HTTPTransport {
    let error: any Error
    func send(_ request: HTTPRequest) async throws -> HTTPResponse { throw error }
}

private func leaks(_ error: any Error) -> Bool {
    let text = "\(error) \(String(describing: error)) \(error.localizedDescription) \((error as NSError).userInfo)"
    return text.contains(privatePath) || text.contains("example.test/events")
}

private func transports() async -> [(String, any HTTPTransport)] {
    func answering(_ response: HTTPResponse) async -> FakeTransport {
        let transport = FakeTransport()
        await transport.route(privatePath, [response])
        return transport
    }
    let urlError = URLError(.notConnectedToInternet, userInfo: [
        NSURLErrorFailingURLErrorKey: feedURL, NSURLErrorFailingURLStringErrorKey: feedURL.absoluteString,
        NSLocalizedDescriptionKey: "could not reach \(feedURL.absoluteString)"])
    return [
        ("network message with the link", ThrowingTransport(error: SourceError.network("failed: \(feedURL.absoluteString)"))),
        ("network message with the path", ThrowingTransport(error: SourceError.network("no route to /events/ical/42/\(privatePath)/going"))),
        ("a URLError that carries the link", ThrowingTransport(error: urlError)),
        ("404", await answering(HTTPResponse(status: 404, body: Data("not found \(feedURL.absoluteString)".utf8)))),
        ("500", await answering(HTTPResponse(status: 500, body: Data("boom \(feedURL.absoluteString)".utf8)))),
        ("200 that is not a calendar", await answering(HTTPResponse(status: 200, body: Data("<html>\(feedURL.absoluteString)</html>".utf8)))),
    ]
}

private func source(_ transport: any HTTPTransport) -> ICalSubscriptionSource {
    ICalSubscriptionSource(
        connection: Connection(kindID: "icalsub", connectionID: "c1", displayName: "x (www.example.test)", config: ["host": "www.example.test"]),
        link: { feedURL }, transport: transport, monitor: ChangeMonitor(interval: .seconds(900), sleep: { _ in }),
        maxAge: 900, now: TestNow().provider, defaultZone: .current)
}

@Test func noErrorFromSignInEventsOrChecksEverContainsTheLink() async {
    for (name, transport) in await transports() {
        var errors: [any Error] = []
        do {
            _ = try await ICalSubscriptionKind(transport: transport)
                .authorize(using: StubInteraction(["link": feedURL.absoluteString]), credentials: InMemoryCredentialStore())
        } catch { errors.append(error) }
        do { _ = try await source(transport).events(in: september) } catch { errors.append(error) }
        do { _ = try await source(transport).checkForChanges() } catch { errors.append(error) }
        #expect(errors.count == 3, "\(name): every call should fail")
        for error in errors { #expect(!leaks(error), "\(name): \(error)") }
    }
}

@Test func aTransportFailureWithoutTheLinkKeepsItsMessage() async {
    do {
        _ = try await FeedFetcher(transport: ThrowingTransport(error: SourceError.network("The Internet connection appears to be offline.")))
            .fetch(feedURL)
        Issue.record("expected a failure")
    } catch {
        #expect(error as? SourceError == .network("The Internet connection appears to be offline."))
    }
}
