import CalendarCore
import CalendarOAuth
import CalendarTestSupport
import Foundation
import Testing
@testable import GoogleCalendar

private func makeClient(_ transport: FakeTransport) async throws -> GoogleAPIClient {
    let store = InMemoryCredentialStore()
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt"], for: "c1")
    let now = TestNow()
    let provider = AccessTokenProvider(
        connectionID: "c1", credentials: store,
        refresh: { _ in OAuthTokens(accessToken: "at", expiresAt: now.date.addingTimeInterval(3600)) }, now: now.provider)
    return GoogleAPIClient(transport: transport, tokens: provider, sleep: { _ in })
}

private func thrown(_ body: () async throws -> Void) async -> Error? {
    do { try await body(); return nil } catch { return error }
}

private func errorBody(_ reason: String, message: String = "m", status: Int) -> HTTPResponse {
    .json(["error": ["errors": [["reason": reason]], "message": message]], status: status)
}

@Test func sendPostsABodyWithJsonHeadersAndExtraHeaders() async throws {
    let transport = FakeTransport()
    await transport.route("events", [.json(["id": "x"])])
    let client = try await makeClient(transport)
    _ = try await client.send(method: "PATCH", path: "/calendars/c/events/e", query: [URLQueryItem(name: "sendUpdates", value: "none")],
                              body: Data(#"{"summary":"S"}"#.utf8), headers: ["If-Match": "e1"], mode: .write)
    let request = try #require(await transport.requests.last)
    #expect(request.method == "PATCH" && request.url.absoluteString.hasSuffix("/calendars/c/events/e?sendUpdates=none"))
    #expect(request.headers["Content-Type"] == "application/json" && request.headers["If-Match"] == "e1")
    #expect(request.headers["Authorization"] == "Bearer at" && String(decoding: request.body ?? Data(), as: UTF8.self) == #"{"summary":"S"}"#)
}

@Test func aStaleIfMatchSurfacesAsPreconditionFailed() async throws {
    let transport = FakeTransport()
    await transport.route("events", [errorBody("conditionNotMet", status: 412)])
    let client = try await makeClient(transport)
    let error = await thrown { _ = try await client.send(method: "PATCH", path: "/calendars/c/events/e", mode: .write) }
    #expect(error as? GoogleAPIError == .preconditionFailed)
}

@Test func writeModeMapsAnyOtherForbiddenReasonButReadModeKeepsItsBehaviour() async throws {
    let transport = FakeTransport()
    await transport.route("events", [errorBody("forbiddenForNonOrganizer", status: 403)])
    let client = try await makeClient(transport)
    let write = await thrown { _ = try await client.send(method: "PATCH", path: "/calendars/c/events/e", mode: .write) }
    #expect(write as? GoogleAPIError == .forbidden)
    let read = await thrown { _ = try await client.get(path: "/calendars/c/events/e", query: []) }
    #expect(read as? SourceError == .invalidResponse("HTTP 403: forbiddenForNonOrganizer"))
}

@Test func insufficientPermissionsIsStillAuthExpiredInWriteMode() async throws {
    let transport = FakeTransport()
    await transport.route("events", [errorBody("insufficientPermissions", status: 403)])
    let client = try await makeClient(transport)
    let error = await thrown { _ = try await client.send(method: "POST", path: "/calendars/c/events", mode: .write) }
    #expect(error as? SourceError == .authExpired)
}

@Test func writeModeMapsBadRequestWithItsMessage() async throws {
    let transport = FakeTransport()
    await transport.route("events", [errorBody("invalid", message: "Invalid start time", status: 400)])
    let client = try await makeClient(transport)
    let error = await thrown { _ = try await client.send(method: "POST", path: "/calendars/c/events", mode: .write) }
    #expect(error as? GoogleAPIError == .badRequest("Invalid start time"))
}

@Test func goneAndNotFoundKeepTheirMeaning() async throws {
    let transport = FakeTransport()
    await transport.route("gone", [errorBody("deleted", status: 410)])
    await transport.route("missing", [errorBody("notFound", status: 404)])
    let client = try await makeClient(transport)
    #expect(await thrown { _ = try await client.get(path: "/gone", query: []) } as? GoogleAPIError == .gone)
    #expect(await thrown { _ = try await client.send(method: "DELETE", path: "/missing", mode: .write) } as? GoogleAPIError == .notFound)
}

@Test func aNoContentResponseSucceeds() async throws {
    let transport = FakeTransport()
    await transport.route("events", [HTTPResponse(status: 204)])
    let client = try await makeClient(transport)
    let data = try await client.send(method: "DELETE", path: "/calendars/c/events/e", mode: .write)
    #expect(data.isEmpty)
}

@Test func eventPathEncodesBothIds() {
    #expect(GoogleAPIClient.eventPath("me@x.com", "a_b/c") == "/calendars/me%40x.com/events/a_b%2Fc")
}

@Test func aReadNeverMapsA412ToPreconditionFailed() async throws {
    let transport = FakeTransport()
    await transport.route("events", [errorBody("conditionNotMet", status: 412)])
    let client = try await makeClient(transport)
    let error = await thrown { _ = try await client.get(path: "/calendars/c/events", query: []) }
    #expect(error as? SourceError == .invalidResponse("HTTP 412"))
}

@Test func aReadKeepsMappingABadRequestToAGenericError() async throws {
    let transport = FakeTransport()
    await transport.route("events", [errorBody("invalid", status: 400)])
    let client = try await makeClient(transport)
    let error = await thrown { _ = try await client.get(path: "/calendars/c/events", query: []) }
    #expect(error as? SourceError == .invalidResponse("HTTP 400"))
}

@Test func percentEncodeEscapesNonAsciiInsteadOfTrappingInTheUrl() async throws {
    #expect(GoogleAPIClient.percentEncode("café") == "caf%C3%A9")
    let transport = FakeTransport()
    await transport.route("events", [.json([:])])
    let client = try await makeClient(transport)
    _ = try await client.send(method: "GET", path: GoogleAPIClient.eventPath("kalender-ü@x.de", "é"), mode: .write)
    let request = try #require(await transport.requests.last)
    #expect(request.url.absoluteString.hasSuffix("/calendars/kalender-%C3%BC%40x.de/events/%C3%A9"))
}

@Test func anEmptyQueryLeavesNoTrailingQuestionMark() async throws {
    let transport = FakeTransport()
    await transport.route("events", [.json([:])])
    let client = try await makeClient(transport)
    _ = try await client.send(method: "DELETE", path: "/calendars/c/events/e", mode: .write)
    #expect(try #require(await transport.requests.last).url.absoluteString.hasSuffix("/events/e"))
}

@Test func aRateLimitedWriteIsRetriedThenSucceeds() async throws {
    let transport = FakeTransport()
    await transport.route("events", [errorBody("rateLimitExceeded", status: 403), .json(["id": "x"])])
    let client = try await makeClient(transport)
    _ = try await client.send(method: "POST", path: "/calendars/c/events", body: Data("{}".utf8), mode: .write)
    #expect(await transport.requests.count == 2)
}

@Test func aWrite401RefreshesOnceThenGivesUp() async throws {
    let transport = FakeTransport()
    await transport.route("events", [HTTPResponse(status: 401)])
    let client = try await makeClient(transport)
    let error = await thrown { _ = try await client.send(method: "POST", path: "/calendars/c/events", mode: .write) }
    #expect(error as? SourceError == .authExpired)
    #expect(await transport.requests.count == 2)
}

@Test func aServerErrorOnAWriteIsNotRetried() async throws {
    let transport = FakeTransport()
    await transport.route("events", [HTTPResponse(status: 503)])
    let client = try await makeClient(transport)
    let error = await thrown { _ = try await client.send(method: "POST", path: "/calendars/c/events", mode: .write) }
    #expect(error as? SourceError == .server(status: 503))
    #expect(await transport.requests.count == 1)
}

@Test func aReadClassifiesA403ByItsFirstErrorOnly() async throws {
    // The first error has no reason, the second says `forbidden`: a read keeps judging the first error alone.
    let transport = FakeTransport()
    let body: [String: Any] = ["error": ["errors": [["message": "no reason"], ["reason": "forbidden"]], "message": "m"]]
    await transport.route("events", [.json(body, status: 403)])
    let client = try await makeClient(transport)
    let error = await thrown { _ = try await client.get(path: "/calendars/c/events", query: []) }
    #expect(error as? SourceError == .invalidResponse("HTTP 403: unknown"))
}

@Test func aWrite409IsATypedConflictButAReadKeepsItsGenericError() async throws {
    let transport = FakeTransport()
    await transport.route("events", [errorBody("duplicate", status: 409)])
    let client = try await makeClient(transport)
    let write = await thrown { _ = try await client.send(method: "POST", path: "/calendars/c/events", mode: .write) }
    #expect(write as? GoogleAPIError == .conflict)
    let read = await thrown { _ = try await client.get(path: "/calendars/c/events", query: []) }
    #expect(read as? SourceError == .invalidResponse("HTTP 409"))
}
