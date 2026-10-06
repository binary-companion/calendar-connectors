import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import ICalSubscription

private let webcal = "webcal://www.example.test/events/ical/42/\(privatePath)/going"

private func serving(_ responses: [HTTPResponse]) async -> FakeTransport {
    let transport = FakeTransport()
    await transport.route(privatePath, responses)
    return transport
}

private func feedResponse(_ text: String = sampleFeed) -> HTTPResponse { HTTPResponse(status: 200, body: Data(text.utf8)) }

@Test func theKindDescribesItself() throws {
    let kind = ICalSubscriptionKind(transport: FakeTransport())
    #expect(kind.id == "icalsub" && kind.displayName == "iCal link")
    guard case .password(let fields) = kind.authorization else { Issue.record("not a password kind"); return }
    #expect(fields == [CredentialField(key: "link", label: "iCal link", isSecret: true)])
    let help = try #require(kind.credentialHelp)
    #expect(help.text.contains("Meetup") && help.text.contains("will not update") && help.url == URL(string: "https://www.meetup.com/your-events/"))
    #expect(kind.supportedPlatforms.contains(.linux) && kind.supportedPlatforms.contains(.macOS))
}

@Test func signInStoresTheLinkAsASecretAndOnlyTheHostInTheConfig() async throws {
    let transport = await serving([feedResponse()])
    let store = InMemoryCredentialStore()
    let interaction = StubInteraction(["link": "  \(webcal) "])
    let connection = try await ICalSubscriptionKind(transport: transport).authorize(using: interaction, credentials: store)
    #expect(connection.kindID == "icalsub" && connection.displayName == "My Meetups (www.example.test)")
    #expect(connection.config == ["host": "www.example.test"])
    #expect(try await store.secrets(for: connection.connectionID) == ["link": feedURL.absoluteString])
    #expect(!connection.displayName.contains(privatePath) && !connection.config.values.contains { $0.contains(privatePath) })
    #expect(interaction.shown.count == 1)
    let request = try #require(await transport.requests.first)
    #expect(request.url == feedURL)
}

@Test func aFeedWithNoNameIsNamedByItsHostAndAFeedWithNoEventsIsValid() async throws {
    let transport = await serving([feedResponse(feedICS([], header: []))])
    let connection = try await ICalSubscriptionKind(transport: transport)
        .authorize(using: StubInteraction(["link": webcal]), credentials: InMemoryCredentialStore())
    #expect(connection.displayName == "www.example.test")
}

@Test func badInputStoresNothingAndSendsNothing() async throws {
    for text in ["http://www.example.test/a.ics", "file:///tmp/a.ics", "/tmp/a.ics", "", "https://me:pw@www.example.test/a.ics"] {
        let transport = await serving([feedResponse()])
        let store = InMemoryCredentialStore()
        await #expect(throws: SourceError.self, "\(text)") {
            _ = try await ICalSubscriptionKind(transport: transport).authorize(using: StubInteraction(["link": text]), credentials: store)
        }
        let stored = await store.isEmpty
        let sent = await transport.requests
        #expect(stored && sent.isEmpty)
    }
}

@Test func aLinkThatIsNotACalendarStoresNothing() async throws {
    let transport = await serving([feedResponse("<html><body>Please sign in</body></html>")])
    let store = InMemoryCredentialStore()
    await #expect(throws: SourceError.invalidResponse("that link did not return a calendar")) {
        _ = try await ICalSubscriptionKind(transport: transport).authorize(using: StubInteraction(["link": webcal]), credentials: store)
    }
    #expect(await store.isEmpty)
}

@Test func aRevokedLinkAtSignInIsAuthExpiredAndStoresNothing() async throws {
    let transport = await serving([HTTPResponse(status: 404)])
    let store = InMemoryCredentialStore()
    await #expect(throws: SourceError.authExpired) {
        _ = try await ICalSubscriptionKind(transport: transport).authorize(using: StubInteraction(["link": webcal]), credentials: store)
    }
    #expect(await store.isEmpty)
}

@Test func aCancelledPromptStoresNothing() async throws {
    let store = InMemoryCredentialStore()
    await #expect(throws: CancellationError.self) {
        _ = try await ICalSubscriptionKind(transport: FakeTransport()).authorize(using: StubInteraction(), credentials: store)
    }
    #expect(await store.isEmpty)
}

@Test func reauthorizeReplacesTheLinkAndKeepsTheConnectionID() async throws {
    let store = InMemoryCredentialStore()
    let first = try await ICalSubscriptionKind(transport: await serving([feedResponse()]))
        .authorize(using: StubInteraction(["link": webcal]), credentials: store)
    let newer = "https://www.example.test/events/ical/42/PRIVATE-PATH-4567/going"
    let transport = FakeTransport()
    await transport.route("PRIVATE-PATH-4567", [feedResponse()])
    let updated = try await ICalSubscriptionKind(transport: transport)
        .reauthorize(first, using: StubInteraction(["link": newer]), credentials: store)
    #expect(updated.connectionID == first.connectionID && updated.displayName == first.displayName)
    #expect(try await store.secrets(for: first.connectionID) == ["link": newer])
}

@Test func reauthorizeToAnotherHostRebuildsTheNameForTheNewHostKeepingTheFeedName() async throws {
    let store = InMemoryCredentialStore()
    let first = try await ICalSubscriptionKind(transport: await serving([feedResponse()]))
        .authorize(using: StubInteraction(["link": webcal]), credentials: store)
    let transport = FakeTransport()
    await transport.route("PRIVATE-PATH-4567", [feedResponse(feedICS([boardGames], header: ["X-WR-CALNAME:Renamed"]))])
    let updated = try await ICalSubscriptionKind(transport: transport).reauthorize(
        first, using: StubInteraction(["link": "https://calendar.example.org/x/PRIVATE-PATH-4567/going"]), credentials: store)
    #expect(updated.config == ["host": "calendar.example.org"])
    #expect(updated.displayName == "My Meetups (calendar.example.org)")
    #expect(!updated.displayName.contains("PRIVATE-PATH"))
}

@Test func reauthorizeToAnotherHostForANamelessFeedUsesTheNewFeedNameOrTheHost() async throws {
    let store = InMemoryCredentialStore()
    let first = try await ICalSubscriptionKind(transport: await serving([feedResponse(feedICS([boardGames], header: []))]))
        .authorize(using: StubInteraction(["link": webcal]), credentials: store)
    #expect(first.displayName == "www.example.test")
    let transport = FakeTransport()
    await transport.route("PRIVATE-PATH-4567", [feedResponse(feedICS([boardGames], header: []))])
    let updated = try await ICalSubscriptionKind(transport: transport).reauthorize(
        first, using: StubInteraction(["link": "https://calendar.example.org/x/PRIVATE-PATH-4567/going"]), credentials: store)
    #expect(updated.displayName == "calendar.example.org")
}

@Test func aFailedReauthorizeKeepsTheOldLink() async throws {
    let store = InMemoryCredentialStore()
    let first = try await ICalSubscriptionKind(transport: await serving([feedResponse()]))
        .authorize(using: StubInteraction(["link": webcal]), credentials: store)
    let transport = FakeTransport()
    await transport.route("PRIVATE-PATH-4567", [HTTPResponse(status: 404)])
    await #expect(throws: SourceError.authExpired) {
        _ = try await ICalSubscriptionKind(transport: transport)
            .reauthorize(first, using: StubInteraction(["link": "https://www.example.test/events/ical/42/PRIVATE-PATH-4567/going"]), credentials: store)
    }
    #expect(try await store.secrets(for: first.connectionID) == ["link": feedURL.absoluteString])
}

@Test func theSourceReadsThroughTheStoredLink() async throws {
    let store = InMemoryCredentialStore()
    let transport = await serving([feedResponse()])
    let kind = ICalSubscriptionKind(transport: transport, sleep: { _ in })
    let connection = try await kind.authorize(using: StubInteraction(["link": webcal]), credentials: store)
    let source = try kind.makeSource(for: connection, credentials: store, syncState: InMemorySyncStateStore())
    #expect(source.id == "icalsub-\(connection.connectionID)")
    #expect(try await source.events(in: september).count == 6)
}

@Test func aSourceWithNoStoredLinkNeedsSigningInAgain() async throws {
    let connection = Connection(kindID: "icalsub", connectionID: "gone", displayName: "x")
    let source = try ICalSubscriptionKind(transport: FakeTransport())
        .makeSource(for: connection, credentials: InMemoryCredentialStore(), syncState: InMemorySyncStateStore())
    await #expect(throws: SourceError.authExpired) { _ = try await source.events(in: september) }
}

@Test func theDefaultSessionKeepsNothingOnDisk() {
    let configuration = ICalSubscriptionKind.feedSessionConfiguration
    #expect(configuration.urlCache == nil)
    #expect(configuration.requestCachePolicy == .reloadIgnoringLocalCacheData)
    #expect(configuration.httpShouldSetCookies == false)
    #expect(configuration.httpCookieStorage == nil)
}
