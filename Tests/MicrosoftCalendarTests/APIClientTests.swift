import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import MicrosoftCalendar

@Test func sendsBearerTokenAndAskForImmutableIds() async throws {
    let h = try await ClientHarness()
    await h.transport.route("/me/calendars", [.json(["value": []])])
    _ = try await h.client.get(url: h.client.url(path: "/me/calendars"), prefer: ["outlook.timezone=\"UTC\""])
    let request = try #require(await h.transport.requests.first)
    #expect(request.headers["Authorization"] == "Bearer at1")
    #expect(request.headers["Prefer"] == "IdType=\"ImmutableId\", outlook.timezone=\"UTC\"")
}

@Test func percentEncodesIdsInPaths() {
    #expect(GraphAPIClient.eventPath("AAMk=", "AQ/x+y") == "/me/calendars/AAMk%3D/events/AQ%2Fx%2By")
}

@Test func mapsStatusCodesToProviderErrors() async throws {
    let h = try await ClientHarness()
    for (status, expected) in [(404, GraphAPIError.notFound), (403, .forbidden), (410, .gone), (409, .conflict), (412, .preconditionFailed)] {
        let transport = FakeTransport()
        await transport.route("x", [.json([:], status: status)])
        let client = GraphAPIClient(transport: transport, tokens: h.client.tokens, sleep: h.client.sleep)
        do {
            _ = try await client.get(url: client.url(path: "/x"))
            Issue.record("status \(status) did not throw")
        } catch let error as GraphAPIError {
            #expect(error == expected)
        }
    }
}

@Test func aBadRequestCarriesGraphsMessageAndAResyncCodeMeansGone() async throws {
    let h = try await ClientHarness()
    await h.transport.route("bad", [graphError("ErrorInvalidRequest", message: "start after end", status: 400)])
    await h.transport.route("resync", [graphError("syncStateNotFound", status: 400)])
    do { _ = try await h.client.get(url: h.client.url(path: "/bad")); Issue.record("no throw") }
    catch let error as GraphAPIError { #expect(error == .badRequest("start after end")) }
    do { _ = try await h.client.get(url: h.client.url(path: "/resync")); Issue.record("no throw") }
    catch let error as GraphAPIError { #expect(error == .gone) }
}

@Test func a401RefreshesOnceThenFailsAsAuthExpired() async throws {
    let h = try await ClientHarness()
    await h.transport.route("me", [.json([:], status: 401), .json(["ok": true])])
    _ = try await h.client.get(url: h.client.url(path: "/me"))
    let tokens = await h.transport.requests.map { $0.headers["Authorization"] }
    #expect(tokens == ["Bearer at1", "Bearer at2"])

    let again = try await ClientHarness()
    await again.transport.route("me", [.json([:], status: 401)])
    await #expect(throws: SourceError.authExpired) { _ = try await again.client.get(url: again.client.url(path: "/me")) }
}

@Test func throttlingHonorsRetryAfterThenGivesUp() async throws {
    let h = try await ClientHarness()
    await h.transport.route("me", [.json([:], status: 429, headers: ["Retry-After": "7"]), .json(["ok": true])])
    _ = try await h.client.get(url: h.client.url(path: "/me"))
    #expect(await h.sleeps.durations == [.seconds(7)])

    let stuck = try await ClientHarness()
    await stuck.transport.route("me", [.json([:], status: 429, headers: ["Retry-After": "1"])])
    await #expect(throws: SourceError.rateLimited(retryAfter: 1)) { _ = try await stuck.client.get(url: stuck.client.url(path: "/me")) }
    #expect(await stuck.transport.requests.count == 4)   // the first try and three retries
}

@Test func aServiceUnavailableIsRetriedThenReportedAsAServerError() async throws {
    let h = try await ClientHarness()
    await h.transport.route("me", [.json([:], status: 503)])
    await #expect(throws: SourceError.server(status: 503)) { _ = try await h.client.get(url: h.client.url(path: "/me")) }
}

@Test func fiveHundredIsAServerErrorWithoutRetry() async throws {
    let h = try await ClientHarness()
    await h.transport.route("me", [.json([:], status: 500)])
    await #expect(throws: SourceError.server(status: 500)) { _ = try await h.client.get(url: h.client.url(path: "/me")) }
    #expect(await h.transport.requests.count == 1)
}

@Test func pagesFollowNextLinksAndRefuseAForeignHost() async throws {
    let h = try await ClientHarness()
    await h.transport.route("/me/calendars", [
        .json(["value": [["id": "a"]], "@odata.nextLink": "https://graph.microsoft.com/v1.0/me/calendars?$skip=1"]),
        .json(["value": [["id": "b"]]]),
    ])
    var ids: [String] = []
    try await h.client.pages(GraphListPage<GraphIDDTO>.self, from: h.client.url(path: "/me/calendars")) { ids += $0.items.compactMap(\.id) }
    #expect(ids == ["a", "b"])

    let evil = try await ClientHarness()
    await evil.transport.route("/me/calendars", [.json(["value": [], "@odata.nextLink": "https://evil.example/steal"])])
    await #expect(throws: SourceError.invalidResponse("unexpected link host")) {
        try await evil.client.pages(GraphListPage<GraphIDDTO>.self, from: evil.client.url(path: "/me/calendars")) { _ in }
    }
    #expect(await evil.transport.requests(matching: "evil.example").isEmpty)
}

@Test func aMalformedItemIsDroppedButStillCounted() throws {
    let data = Data(#"{"value":[{"id":"a"},{"id":5},{"id":"c"}],"@odata.deltaLink":"https://graph.microsoft.com/v1.0/d"}"#.utf8)
    let page = try JSONDecoder().decode(GraphListPage<GraphIDDTO>.self, from: data)
    #expect(page.value?.count == 3)
    #expect(page.items.compactMap(\.id) == ["a", "c"])
    #expect(page.deltaLink == "https://graph.microsoft.com/v1.0/d")
}
