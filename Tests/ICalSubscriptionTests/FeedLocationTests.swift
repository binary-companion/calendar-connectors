import CalendarCore
import Foundation
import Testing
@testable import ICalSubscription

@Test func webcalLinksBecomeHTTPS() throws {
    #expect(try FeedLocation.url(from: "webcal://www.example.test/events/ical/42/abc/going").absoluteString
            == "https://www.example.test/events/ical/42/abc/going")
    #expect(try FeedLocation.url(from: "WEBCALS://example.test/a.ics").absoluteString == "https://example.test/a.ics")
}

@Test func httpsLinksAreAcceptedWithOrWithoutAnIcsEnding() throws {
    #expect(try FeedLocation.url(from: "https://example.test/feed.ics").absoluteString == "https://example.test/feed.ics")
    #expect(try FeedLocation.url(from: "https://example.test/feed?id=7").absoluteString == "https://example.test/feed?id=7")
}

@Test func surroundingWhitespaceIsTrimmed() throws {
    #expect(try FeedLocation.url(from: "  https://example.test/feed.ics \n").absoluteString == "https://example.test/feed.ics")
}

@Test func plainHTTPIsRefusedExceptForLoopback() throws {
    #expect(throws: SourceError.self) { try FeedLocation.url(from: "http://example.test/feed.ics") }
    #expect(try FeedLocation.url(from: "http://localhost:8008/feed.ics").absoluteString == "http://localhost:8008/feed.ics")
    #expect(try FeedLocation.url(from: "http://127.0.0.1:8008/feed.ics").host == "127.0.0.1")
    #expect(try FeedLocation.url(from: "http://[::1]:8008/feed.ics").absoluteString == "http://[::1]:8008/feed.ics")
    #expect(try FeedLocation.url(from: "http://[::1]/feed.ics").absoluteString == "http://[::1]/feed.ics")
}

@Test func linksWithEmbeddedCredentialsAreRefused() {
    #expect(throws: SourceError.self) { try FeedLocation.url(from: "https://me:pw@example.test/feed.ics") }
    #expect(throws: SourceError.self) { try FeedLocation.url(from: "https://me@example.test/feed.ics") }
}

@Test func filesPathsAndJunkAreRefused() {
    for text in ["file:///tmp/a.ics", "/tmp/a.ics", "a.ics", "meetup", "", "   ", "ftp://example.test/a.ics", "https://"] {
        #expect(throws: SourceError.self, "\(text)") { try FeedLocation.url(from: text) }
    }
}

@Test func refusalMessagesPointAtSubscriptionLinks() {
    do { _ = try FeedLocation.url(from: "file:///tmp/a.ics"); Issue.record("expected a throw") }
    catch { #expect(error as? SourceError == .invalidResponse(FeedLocation.invalidMessage)) }
}
