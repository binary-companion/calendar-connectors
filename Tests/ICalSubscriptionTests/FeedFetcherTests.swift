import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import ICalSubscription

private func body(of response: FeedResponse) -> Data? {
    if case .body(let data, _) = response { return data }
    return nil
}

private func ok(_ text: String = sampleFeed, headers: [String: String] = [:]) -> HTTPResponse {
    HTTPResponse(status: 200, headers: headers, body: Data(text.utf8))
}

@Test func aSuccessfulFetchReturnsTheBodyAndItsValidators() async throws {
    let transport = FakeTransport()
    await transport.route(privatePath, [ok(headers: ["ETag": "\"v1\"", "Last-Modified": "Mon, 05 Oct 2026 10:00:00 GMT"])])
    let response = try await FeedFetcher(transport: transport).fetch(feedURL)
    guard case .body(let data, let validators) = response else { Issue.record("expected a body"); return }
    #expect(String(decoding: data, as: UTF8.self) == sampleFeed)
    #expect(validators == FeedValidators(etag: "\"v1\"", lastModified: "Mon, 05 Oct 2026 10:00:00 GMT"))
    let request = try #require(await transport.requests.first)
    #expect(request.method == "GET" && request.headers["Accept"]?.hasPrefix("text/calendar") == true)
    #expect(request.headers["If-None-Match"] == nil)
}

@Test func validatorsMakeTheRequestConditionalAndA304MeansUnchanged() async throws {
    let transport = FakeTransport()
    await transport.route(privatePath, [HTTPResponse(status: 304)])
    let response = try await FeedFetcher(transport: transport)
        .fetch(feedURL, validators: FeedValidators(etag: "\"v1\"", lastModified: "Mon, 05 Oct 2026 10:00:00 GMT"))
    guard case .notModified = response else { Issue.record("expected notModified"); return }
    let request = try #require(await transport.requests.first)
    #expect(request.headers["If-None-Match"] == "\"v1\"" && request.headers["If-Modified-Since"] == "Mon, 05 Oct 2026 10:00:00 GMT")
}

@Test func aStray304WithoutValidatorsIsAnInvalidResponse() async {
    let transport = FakeTransport()
    await transport.route(privatePath, [HTTPResponse(status: 304)])
    await #expect(throws: SourceError.invalidResponse("the feed server answered 304")) {
        _ = try await FeedFetcher(transport: transport).fetch(feedURL)
    }
}

@Test func gonePrivateLinksMeanTheLinkNeedsReplacing() async {
    for status in [401, 403, 404, 410] {
        let transport = FakeTransport()
        await transport.route(privatePath, [HTTPResponse(status: status)])
        await #expect(throws: SourceError.authExpired, "\(status)") { _ = try await FeedFetcher(transport: transport).fetch(feedURL) }
    }
}

@Test func throttlingAndServerErrorsAreServerErrors() async {
    for status in [429, 500, 503] {
        let transport = FakeTransport()
        await transport.route(privatePath, [HTTPResponse(status: status)])
        await #expect(throws: SourceError.server(status: status), "\(status)") { _ = try await FeedFetcher(transport: transport).fetch(feedURL) }
    }
}

@Test func otherStatusesAreInvalidResponses() async {
    let transport = FakeTransport()
    await transport.route(privatePath, [HTTPResponse(status: 400)])
    await #expect(throws: SourceError.invalidResponse("the feed server answered 400")) { _ = try await FeedFetcher(transport: transport).fetch(feedURL) }
}

@Test func httpsRedirectsAreFollowedIncludingRelativeOnes() async throws {
    let transport = FakeTransport()
    await transport.route("other.example.test", [ok()])
    await transport.route("/relative", [ok("BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n")])
    await transport.route(privatePath, [HTTPResponse(status: 302, headers: ["Location": "https://other.example.test/feed.ics"])])
    #expect(body(of: try await FeedFetcher(transport: transport).fetch(feedURL)) != nil)
    #expect(await transport.requests(matching: "other.example.test").count == 1)

    let relative = FakeTransport()
    await relative.route("/relative", [ok("BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n")])
    await relative.route(privatePath, [HTTPResponse(status: 301, headers: ["Location": "/relative"])])
    #expect(body(of: try await FeedFetcher(transport: relative).fetch(feedURL)) != nil)
}

@Test func redirectsToPlainHTTPOrAnotherSchemeAreRefused() async {
    for location in ["http://other.example.test/feed.ics", "ftp://other.example.test/feed.ics", "file:///tmp/a.ics"] {
        let transport = FakeTransport()
        await transport.route(privatePath, [HTTPResponse(status: 302, headers: ["Location": location])])
        await #expect(throws: SourceError.invalidResponse("the feed moved somewhere TimeTug will not follow"), "\(location)") {
            _ = try await FeedFetcher(transport: transport).fetch(feedURL)
        }
        #expect(await transport.requests.count == 1)
    }
}

@Test func aRedirectToAnAddressWithAUserNameOrPasswordIsRefused() async {
    for location in ["https://me:pw@other.example.test/feed.ics", "https://me@other.example.test/feed.ics"] {
        let transport = FakeTransport()
        await transport.route(privatePath, [HTTPResponse(status: 302, headers: ["Location": location])])
        do {
            _ = try await FeedFetcher(transport: transport).fetch(feedURL)
            Issue.record("expected a refusal for \(location)")
        } catch {
            #expect(error as? SourceError == .invalidResponse("the feed moved somewhere TimeTug will not follow"))
            #expect(!"\(error)".contains("other.example.test") && !"\(error)".contains(privatePath))
        }
        #expect(await transport.requests.count == 1)
    }
}

@Test func aRedirectWithNoLocationIsRefused() async {
    let transport = FakeTransport()
    await transport.route(privatePath, [HTTPResponse(status: 302)])
    await #expect(throws: SourceError.invalidResponse("the feed moved somewhere TimeTug will not follow")) {
        _ = try await FeedFetcher(transport: transport).fetch(feedURL)
    }
}

@Test func aRedirectLoopStopsAfterFiveRedirects() async {
    let transport = FakeTransport()
    await transport.route(privatePath, [HTTPResponse(status: 302, headers: ["Location": feedURL.absoluteString])])
    await #expect(throws: SourceError.invalidResponse("the feed redirected too many times")) {
        _ = try await FeedFetcher(transport: transport).fetch(feedURL)
    }
    #expect(await transport.requests.count == FeedFetcher.maxRedirects + 1)
}

@Test func anOversizedBodyIsRefused() async throws {
    let transport = FakeTransport()
    await transport.route(privatePath, [HTTPResponse(status: 200, body: Data(count: FeedFetcher.maxBytes + 1))])
    await #expect(throws: SourceError.invalidResponse("the feed is too large")) { _ = try await FeedFetcher(transport: transport).fetch(feedURL) }
    let exactly = FakeTransport()
    await exactly.route(privatePath, [HTTPResponse(status: 200, body: Data(count: FeedFetcher.maxBytes))])
    #expect(body(of: try await FeedFetcher(transport: exactly).fetch(feedURL))?.count == FeedFetcher.maxBytes)
}

@Test func noErrorEverContainsTheLink() async {
    let responses = [301, 302, 400, 401, 403, 404, 410, 429, 500, 503].map { HTTPResponse(status: $0) }
    for response in responses {
        let transport = FakeTransport()
        await transport.route(privatePath, [response])
        do { _ = try await FeedFetcher(transport: transport).fetch(feedURL) }
        catch {
            let text = "\(error) \(String(describing: error)) \(error.localizedDescription)"
            #expect(!text.contains(privatePath) && !text.contains("example.test/events"), "status \(response.status): \(text)")
        }
    }
}

private struct EchoingTransport: HTTPTransport {
    let message: String
    func send(_ request: HTTPRequest) async throws -> HTTPResponse { throw SourceError.network(message) }
}

@Test func aTransportMessageThatEchoesOnlyTheQueryOrTheEncodedPathIsReplaced() async {
    let url = URL(string: "https://www.example.test/my%20events/PRIVATE%20X/going?token=QUERY-SECRET-77")!
    let echoes = ["bad request ?token=QUERY-SECRET-77", "token=QUERY-SECRET-77", "failed at /my%20events/PRIVATE%20X/going", "failed at /my events/PRIVATE X/going"]
    for echo in echoes {
        do {
            _ = try await FeedFetcher(transport: EchoingTransport(message: echo)).fetch(url)
            Issue.record("expected a failure")
        } catch {
            #expect(error as? SourceError == .network("the feed could not be reached"), "\(echo)")
        }
    }
}

@Test func aCancelledURLErrorFromAnyTransportIsACancellation() async {
    struct Cancelling: HTTPTransport {
        func send(_ request: HTTPRequest) async throws -> HTTPResponse { throw URLError(.cancelled) }
    }
    await #expect(throws: CancellationError.self) { _ = try await FeedFetcher(transport: Cancelling()).fetch(feedURL) }
}

@Test func a304WhenNoConditionalHeaderWasSentIsAnInvalidResponse() async {
    let transport = FakeTransport()
    await transport.route(privatePath, [HTTPResponse(status: 304)])
    await #expect(throws: SourceError.invalidResponse("the feed server answered 304")) {
        _ = try await FeedFetcher(transport: transport).fetch(feedURL, validators: FeedValidators(etag: nil, lastModified: nil))
    }
}

@Test func validatorsAreNotSentToARedirectTarget() async throws {
    let transport = FakeTransport()
    await transport.route("other.example.test", [HTTPResponse(status: 304)])
    await transport.route(privatePath, [HTTPResponse(status: 302, headers: ["Location": "https://other.example.test/feed.ics"])])
    await #expect(throws: SourceError.invalidResponse("the feed server answered 304")) {
        _ = try await FeedFetcher(transport: transport)
            .fetch(feedURL, validators: FeedValidators(etag: "\"v1\"", lastModified: "Mon, 05 Oct 2026 10:00:00 GMT"))
    }
    let requests = await transport.requests
    #expect(requests.count == 2)
    #expect(requests[0].headers["If-None-Match"] == "\"v1\"")
    #expect(requests[1].headers["If-None-Match"] == nil && requests[1].headers["If-Modified-Since"] == nil)
}

@Test func aChainOfExactlyFiveRedirectsIsFollowed() async throws {
    let transport = FakeTransport()
    let redirect = HTTPResponse(status: 302, headers: ["Location": feedURL.absoluteString])
    await transport.route(privatePath, Array(repeating: redirect, count: FeedFetcher.maxRedirects) + [ok()])
    #expect(body(of: try await FeedFetcher(transport: transport).fetch(feedURL)) != nil)
    #expect(await transport.requests.count == FeedFetcher.maxRedirects + 1)
}
