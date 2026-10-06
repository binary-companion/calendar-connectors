# Calendar connectors Phase 6: iCal subscription link, Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an "iCal link" connector that reads a private calendar subscription link (for example Meetup's Add to calendar links), polls it every 15 minutes, and shows its events in TimeTug like any other account.

**Architecture:** A new pure-Swift library target `ICalSubscription` (depends on `CalendarCore` and `ICalendar`) holds the connector kind and a read-only polling source. A feed is one `VCALENDAR` with many events, so a parser groups `VEVENT`s by UID into `EventResource`s and the existing `EventReader` expands recurrences. The macOS app registers the kind, gives it an icon, drops the redundant Meetup "Soon" tile, and fixes the sign-in error text for a one-field kind.

**Tech Stack:** Swift 6 (package tools 6.0), Swift Testing (`import Testing`) for the library, XCTest for the app, XcodeGen (`Apps/macOS/project.yml`), SwiftUI.

**Spec:** `docs/superpowers/specs/2026-10-06-ical-subscription-link-design.md`

## Global Constraints

- The kind's id is `icalsub`, its display name is "iCal link", and its one credential field is `CredentialField(key: "link", label: "iCal link", isSecret: true)`.
- `ICalSubscription` imports only `Foundation`, `CalendarCore` and `ICalendar` and builds on Linux (no Apple-only imports). Platforms: `[.macOS, .iOS, .linux, .windows]`.
- The link is accepted as `webcal://`, `webcals://` or `https://` (with or without an `.ics` ending); `http://` only for `localhost`, `127.0.0.1` and `::1`; a link with an embedded user name or password, a `file://` URL and a path are refused.
- The link lives only in the `CredentialStore` (Keychain) under the key `link`. `Connection.config` holds only `["host": <host>]`. The link is never logged and never appears in an error message, a `description` or a `displayName`.
- Redirects are followed by the library itself (at most 5) and only to `https`. The body is capped at 10 MB (`10 * 1024 * 1024` bytes).
- Status mapping: 200 reads; 304 is "unchanged" only when a conditional request was sent; 401, 403, 404 and 410 are `SourceError.authExpired`; 429 and 500 to 599 are `SourceError.server(status:)`; anything else is `SourceError.invalidResponse`.
- Poll interval 15 minutes (`.seconds(900)`); the parsed feed is reused while younger than that. The source is read-only (`canWrite = false`), `syncKind = .token`.
- Provided fields: `series`, `uidScope`, `provider`, `calendarTimeZone`. One calendar with id `"feed"`, provider `.subscription`, kind `.subscribed`.
- Library tests use Swift Testing (`@Test`, `#expect`), fakes from `CalendarTestSupport` (`FakeTransport`, `TestNow`), and run in the Linux CI job through `swift test --package-path Packages/CalendarConnectors`.
- Commits end with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. Finish by pushing and opening a pull request to `master` (squash-merged); never merge locally.
- Do not write a real feed link anywhere (code, tests, docs, commit messages). Tests use `https://www.example.test/events/ical/42/PRIVATE-PATH-0123/going`.

## Review Focus

- A 200 response that is not a calendar (an HTML login page): a clear "that link did not return a calendar" error, and nothing is stored at sign-in (Tasks 2, 5).
- A revoked or regenerated link answers 404: it surfaces as `authExpired`, the sheet says "iCal link was not accepted.", and no error text contains the link (Tasks 3, 5, 6).
- A feed with zero events is a valid sign-in, not an error (Tasks 2, 5).
- A hostile or broken server: an oversized body, a redirect to `http`, a redirect loop (Task 3).
- Events with no `UID`: they must neither crash nor vanish nor change identity between reads (Task 2).

---

### Task 1: Package target, service constant and `FeedLocation`

**Files:**
- Modify: `Packages/CalendarConnectors/Package.swift`
- Modify: `Packages/CalendarConnectors/Sources/CalendarCore/Model.swift` (the `CalendarService` constants, near line 205)
- Create: `Packages/CalendarConnectors/Sources/ICalSubscription/FeedLocation.swift`
- Create: `Packages/CalendarConnectors/Tests/ICalSubscriptionTests/FeedLocationTests.swift`

**Interfaces:**
- Consumes: `SourceError.invalidResponse(String)` from `CalendarCore`.
- Produces: `CalendarService.iCalSubscription` (raw value `"icalsub"`); `enum FeedLocation { static let invalidMessage: String; static func url(from text: String) throws -> URL }` (internal).

- [ ] **Step 1: Declare the target, product and test target**

In `Packages/CalendarConnectors/Package.swift` add the product after the `CalDAVCalendar` library line:

```swift
        .library(name: "ICalSubscription", targets: ["ICalSubscription"]),
```

the target after the `CalDAVCalendar` target line:

```swift
        .target(name: "ICalSubscription", dependencies: ["CalendarCore", "ICalendar"]),
```

and the test target after the `CalDAVCalendarTests` line:

```swift
        .testTarget(name: "ICalSubscriptionTests", dependencies: ["ICalSubscription", "ICalendar", "CalendarCore", "CalendarTestSupport"]),
```

- [ ] **Step 2: Add the service constant**

In `Packages/CalendarConnectors/Sources/CalendarCore/Model.swift`, below `public static let calDAV = CalendarService(rawValue: "caldav")` add:

```swift
    /// An iCalendar subscription link (Meetup's Add to calendar links, a Google secret address, ...).
    public static let iCalSubscription = CalendarService(rawValue: "icalsub")
```

- [ ] **Step 3: Write the failing tests**

Create `Packages/CalendarConnectors/Tests/ICalSubscriptionTests/FeedLocationTests.swift`:

```swift
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
```

- [ ] **Step 4: Run the tests to see them fail**

Run: `swift test --package-path Packages/CalendarConnectors --filter ICalSubscriptionTests`
Expected: build error, "cannot find 'FeedLocation' in scope" (the target has no source file yet).

- [ ] **Step 5: Implement `FeedLocation`**

Create `Packages/CalendarConnectors/Sources/ICalSubscription/FeedLocation.swift`:

```swift
import CalendarCore
import Foundation

/// The link a user pasted, checked and normalised. A feed link is a credential, so nothing here puts it in a message.
enum FeedLocation {
    static let invalidMessage = "enter an iCal link starting with https:// or webcal://"
    private static let loopbackHosts: Set<String> = ["localhost", "127.0.0.1", "::1", "[::1]"]

    /// `webcal://` and `webcals://` become `https://`; `https://` is kept; `http://` is allowed only for the loopback
    /// host (a local test server). Anything else, a link with a user name or password included, is refused.
    static func url(from text: String) throws -> URL {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard var components = URLComponents(string: trimmed), let scheme = components.scheme?.lowercased(),
              let host = components.host?.lowercased(), !host.isEmpty else {
            throw SourceError.invalidResponse(invalidMessage)
        }
        guard components.user == nil, components.password == nil else {
            throw SourceError.invalidResponse("enter the link without a user name or password")
        }
        switch scheme {
        case "webcal", "webcals", "https": components.scheme = "https"
        case "http" where loopbackHosts.contains(host): break
        default: throw SourceError.invalidResponse(invalidMessage)
        }
        guard let url = components.url else { throw SourceError.invalidResponse(invalidMessage) }
        return url
    }
}
```

- [ ] **Step 6: Run the tests to see them pass**

Run: `swift test --package-path Packages/CalendarConnectors --filter ICalSubscriptionTests`
Expected: all `FeedLocationTests` pass. If `"https://"` (empty host) or `"file:///tmp/a.ics"` does not throw, the `guard` on `host` is wrong; fix it, do not weaken the test.

- [ ] **Step 7: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "ICalSubscription: package target and feed link parsing" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: `FeedParser` (group a feed's events by UID)

**Files:**
- Create: `Packages/CalendarConnectors/Sources/ICalSubscription/FeedParser.swift`
- Create: `Packages/CalendarConnectors/Tests/ICalSubscriptionTests/Fixtures.swift`
- Create: `Packages/CalendarConnectors/Tests/ICalSubscriptionTests/FeedParserTests.swift`

**Interfaces:**
- Consumes: `ICalParser.parse(_ data: Data) throws -> ICalComponent`, `ICalComponent(name:properties:components:)`, `.property(_:)`, `.components(named:)`, `EventResource(calendar:) throws`, `EventReader.events(in:overlapping:context:)`, `EventReadContext(calendarID:resourceName:etag:sourceID:calendarZone:selfAddresses:)`.
- Produces: `struct FeedResource: Sendable { let name: String; let resource: EventResource }`; `struct ParsedFeed: Sendable { var name: String?; var colorHex: String?; var timeZone: TimeZone?; var resources: [FeedResource] }`; `enum FeedParser { static func parse(_ data: Data) throws -> ParsedFeed }`; test fixtures `pacific`, `privatePath`, `feedURL`, `pt(...)`, `september`, `feedICS(_:header:)`, `vevent(uid:title:start:end:extra:)`, `boardGames`, `weeklyWalk`, `sampleFeed`, `StubInteraction` (used by later tasks).

- [ ] **Step 1: Write the shared test fixtures**

Create `Packages/CalendarConnectors/Tests/ICalSubscriptionTests/Fixtures.swift`:

```swift
import CalendarCore
import Foundation

let pacific = TimeZone(identifier: "America/Los_Angeles")!
let privatePath = "PRIVATE-PATH-0123"
let feedURL = URL(string: "https://www.example.test/events/ical/42/\(privatePath)/going")!

func pt(_ y: Int, _ mo: Int, _ d: Int, _ h: Int = 10, _ mi: Int = 0) -> Date {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = pacific
    return calendar.date(from: DateComponents(year: y, month: mo, day: d, hour: h, minute: mi))!
}

let september = DateInterval(start: pt(2026, 9, 1, 0), end: pt(2026, 10, 1, 0))

private let laTimeZone = [
    "BEGIN:VTIMEZONE", "TZID:America/Los_Angeles", "X-LIC-LOCATION:America/Los_Angeles",
    "BEGIN:DAYLIGHT", "TZOFFSETFROM:-0800", "TZOFFSETTO:-0700", "TZNAME:PDT", "DTSTART:19700308T020000",
    "RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU", "END:DAYLIGHT",
    "BEGIN:STANDARD", "TZOFFSETFROM:-0700", "TZOFFSETTO:-0800", "TZNAME:PST", "DTSTART:19701101T020000",
    "RRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU", "END:STANDARD", "END:VTIMEZONE",
]

/// A feed shaped like a Meetup "going" feed: one calendar name, one `VTIMEZONE`, and the given events.
func feedICS(_ events: [[String]], header: [String] = ["X-WR-CALNAME:My Meetups"]) -> String {
    (["BEGIN:VCALENDAR", "VERSION:2.0", "PRODID:-//Example//Feed 1.0//EN", "METHOD:PUBLISH"] + header + laTimeZone
        + events.flatMap { $0 } + ["END:VCALENDAR"]).joined(separator: "\r\n") + "\r\n"
}

func vevent(uid: String?, title: String, start: String, end: String, extra: [String] = []) -> [String] {
    ["BEGIN:VEVENT"] + (uid.map { ["UID:\($0)"] } ?? []) + ["DTSTAMP:20260901T000000Z", "SUMMARY:\(title)",
        "DTSTART;TZID=America/Los_Angeles:\(start)", "DTEND;TZID=America/Los_Angeles:\(end)"] + extra + ["END:VEVENT"]
}

let boardGames = vevent(uid: "event_board@example.test", title: "Board games night", start: "20260910T190000",
                        end: "20260910T210000", extra: ["URL:https://www.example.test/events/1/", "LOCATION:Cafe"])
let weeklyWalk = vevent(uid: "event_walk@example.test", title: "Weekly walk", start: "20260901T100000",
                        end: "20260901T110000", extra: ["RRULE:FREQ=WEEKLY;BYDAY=TU"])
/// Two events in September: the single one on the 10th and a weekly Tuesday walk (five instances), six in all.
let sampleFeed = feedICS([boardGames, weeklyWalk])

/// Answers `promptCredentials` with fixed values and records the fields it was shown.
final class StubInteraction: AuthorizationInteraction, @unchecked Sendable {
    private let lock = NSLock()
    private var answers: [[String: String]]
    private var fields: [[CredentialField]] = []

    init(_ answers: [String: String]...) { self.answers = answers }

    var shown: [[CredentialField]] { lock.withLock { fields } }

    func beginOAuthRedirect() async throws -> any OAuthRedirectSession { throw SourceError.invalidResponse("not an OAuth kind") }

    func promptCredentials(_ requested: [CredentialField]) async throws -> [String: String] {
        try lock.withLock {
            fields.append(requested)
            guard !answers.isEmpty else { throw CancellationError() }
            return answers.removeFirst()
        }
    }
}
```

- [ ] **Step 2: Write the failing parser tests**

Create `Packages/CalendarConnectors/Tests/ICalSubscriptionTests/FeedParserTests.swift`:

```swift
import CalendarCore
import CalendarTestSupport
import Foundation
import ICalendar
import Testing
@testable import ICalSubscription

private func events(_ feed: ParsedFeed, in window: DateInterval = september) -> [CalendarEvent] {
    feed.resources.flatMap { item in
        EventReader.events(in: item.resource, overlapping: window, context: EventReadContext(
            calendarID: "feed", resourceName: item.name, etag: nil, sourceID: "icalsub-c1", calendarZone: pacific, selfAddresses: []))
    }.sorted { ($0.start, $0.id) < ($1.start, $1.id) }
}

@Test func groupsEventsByUIDAndNamesTheCalendar() throws {
    let feed = try FeedParser.parse(Data(sampleFeed.utf8))
    #expect(feed.name == "My Meetups")
    #expect(feed.resources.map(\.name) == ["event_board@example.test", "event_walk@example.test"])
    let all = events(feed)
    #expect(all.count == 6)
    #expect(Array(all.map(\.title).prefix(3)) == ["Weekly walk", "Weekly walk", "Board games night"])
    #expect(all[2].location == "Cafe" && all[2].url == URL(string: "https://www.example.test/events/1/"))
}

@Test func overridesStayWithTheirSeries() throws {
    let moved = vevent(uid: "event_walk@example.test", title: "Weekly walk (moved)", start: "20260916T140000", end: "20260916T150000",
                       extra: ["RECURRENCE-ID;TZID=America/Los_Angeles:20260915T100000"])
    let feed = try FeedParser.parse(Data(feedICS([weeklyWalk, moved]).utf8))
    #expect(feed.resources.count == 1)
    let all = events(feed)
    #expect(all.count == 5)
    #expect(all.contains { $0.title == "Weekly walk (moved)" && $0.start == pt(2026, 9, 16, 14) })
    #expect(!all.contains { $0.start == pt(2026, 9, 15, 10) })
}

@Test func theFeedCarriesItsOwnTimeZoneAndColour() throws {
    let header = ["X-WR-CALNAME:Walks", "X-WR-TIMEZONE:Europe/Berlin", "X-APPLE-CALENDAR-COLOR:#12ab34ff"]
    let feed = try FeedParser.parse(Data(feedICS([boardGames], header: header).utf8))
    #expect(feed.name == "Walks" && feed.timeZone == TimeZone(identifier: "Europe/Berlin") && feed.colorHex == "#12ab34")
}

@Test func aFeedWithNoEventsIsValid() throws {
    let feed = try FeedParser.parse(Data(feedICS([]).utf8))
    #expect(feed.resources.isEmpty && feed.name == "My Meetups")
}

@Test func aPageThatIsNotACalendarIsRefused() {
    for body in ["<html><body>Please sign in</body></html>", "", "BEGIN:VEVENT\r\nEND:VEVENT\r\n", "not a calendar at all"] {
        #expect(throws: SourceError.invalidResponse("that link did not return a calendar"), "\(body.prefix(20))") {
            try FeedParser.parse(Data(body.utf8))
        }
    }
}

@Test func eventsWithoutAUIDGetAStableSyntheticOne() throws {
    let bare = vevent(uid: nil, title: "No uid", start: "20260912T100000", end: "20260912T110000")
    let first = try FeedParser.parse(Data(feedICS([bare, boardGames]).utf8))
    let second = try FeedParser.parse(Data(feedICS([bare, boardGames]).utf8))
    #expect(first.resources.map(\.name) == second.resources.map(\.name))
    #expect(first.resources[0].name.hasPrefix("feed-") && first.resources.count == 2)
    #expect(events(first).contains { $0.title == "No uid" })
}

@Test func resourceNamesNeverContainTheSeparatorUsedInEventIDs() throws {
    let odd = vevent(uid: "a b#c/d@example.test", title: "Odd", start: "20260912T100000", end: "20260912T110000",
                     extra: ["RRULE:FREQ=WEEKLY"])
    let feed = try FeedParser.parse(Data(feedICS([odd]).utf8))
    let name = try #require(feed.resources.first?.name)
    #expect(!name.contains("#") && !name.contains("/") && !name.contains(" "))
    #expect(events(feed).allSatisfy { $0.eventID.hasPrefix(name + "#") })
}

@Test func allDayEventsAndFieldsConform() throws {
    let allDay = ["BEGIN:VEVENT", "UID:allday@example.test", "DTSTAMP:20260901T000000Z", "SUMMARY:Festival",
                  "DTSTART;VALUE=DATE:20260920", "DTEND;VALUE=DATE:20260922", "END:VEVENT"]
    let feed = try FeedParser.parse(Data(feedICS([allDay, boardGames, weeklyWalk]).utf8))
    let all = events(feed)
    let festival = try #require(all.first { $0.title == "Festival" })
    #expect(festival.isAllDay)
    for event in all { #expect(AllDayConformance.violations(event).isEmpty, "\(event.title)") }
}
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `swift test --package-path Packages/CalendarConnectors --filter FeedParserTests`
Expected: build error, "cannot find 'FeedParser' in scope".

- [ ] **Step 4: Implement `FeedParser`**

Create `Packages/CalendarConnectors/Sources/ICalSubscription/FeedParser.swift`:

```swift
import CalendarCore
import Foundation
import ICalendar

/// One UID's events from a feed (a master plus its overrides) as the resource the shared `EventReader` expects.
struct FeedResource: Sendable {
    let name: String
    let resource: EventResource
}

struct ParsedFeed: Sendable {
    var name: String?
    var colorHex: String?
    var timeZone: TimeZone?
    var resources: [FeedResource]
}

enum FeedParser {
    private static let notACalendar = "that link did not return a calendar"

    /// A feed is one `VCALENDAR` holding many events, while `EventResource` holds one UID's events. Events are grouped by
    /// `UID` (first-seen order); each group shares the calendar's header and `VTIMEZONE`s. A feed with no events is valid.
    static func parse(_ data: Data) throws -> ParsedFeed {
        let calendar: ICalComponent
        do { calendar = try ICalParser.parse(data) } catch { throw SourceError.invalidResponse(notACalendar) }
        guard calendar.name == "VCALENDAR" else { throw SourceError.invalidResponse(notACalendar) }

        let zones = calendar.components(named: "VTIMEZONE")
        var order: [String] = []
        var groups: [String: [ICalComponent]] = [:]
        for event in calendar.components(named: "VEVENT") {
            let uid = event.property("UID")?.text.nonEmpty ?? syntheticUID(for: event)
            if groups[uid] == nil { order.append(uid) }
            groups[uid, default: []].append(event)
        }
        var resources: [FeedResource] = []
        for uid in order {
            let wrapper = ICalComponent(name: "VCALENDAR", properties: calendar.properties, components: zones + (groups[uid] ?? []))
            guard let resource = try? EventResource(calendar: wrapper) else { continue }
            resources.append(FeedResource(name: resourceName(for: uid), resource: resource))
        }
        return ParsedFeed(
            name: calendar.property("X-WR-CALNAME")?.text.nonEmpty,
            colorHex: colorText(calendar.property("X-APPLE-CALENDAR-COLOR")?.value ?? calendar.property("COLOR")?.value),
            timeZone: calendar.property("X-WR-TIMEZONE").flatMap { TimeZone(identifier: $0.value) },
            resources: resources)
    }

    /// The UID as one URL-safe path segment: it is the base of every event id, which joins an occurrence's start with `#`.
    static func resourceName(for uid: String) -> String {
        var allowed = CharacterSet.alphanumerics
        allowed.insert(charactersIn: "-._~@")
        return uid.addingPercentEncoding(withAllowedCharacters: allowed) ?? uid
    }

    /// A UID for an event that has none: stable across reads because it comes only from the event's start and title.
    static func syntheticUID(for event: ICalComponent) -> String {
        let basis = (event.property("DTSTART")?.value ?? "") + "|" + (event.property("SUMMARY")?.value ?? "")
        return "feed-" + fnv1aHex(basis)
    }

    /// FNV-1a, 64 bit. Not for security: it only keeps a made-up UID the same from one read to the next.
    private static func fnv1aHex(_ text: String) -> String {
        var hash: UInt64 = 0xcbf2_9ce4_8422_2325
        for byte in text.utf8 {
            hash ^= UInt64(byte)
            hash = hash &* 0x0000_0100_0000_01b3
        }
        return String(hash, radix: 16)
    }

    /// `#RRGGBBAA` (Apple) is trimmed to `#RRGGBB`; the descriptor validates the rest.
    private static func colorText(_ raw: String?) -> String? {
        guard let raw else { return nil }
        return raw.hasPrefix("#") && raw.count == 9 ? String(raw.prefix(7)) : raw
    }
}

private extension String {
    var nonEmpty: String? { isEmpty ? nil : self }
}
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `swift test --package-path Packages/CalendarConnectors --filter FeedParserTests`
Expected: all pass. Two likely snags: (a) if `aPageThatIsNotACalendarIsRefused` fails for the `"BEGIN:VEVENT"` body it means the guard on `calendar.name` is missing; (b) if the empty body `""` does not throw, `ICalParser.parse("")` returned without a root and the `do/catch` must map that error too (it throws `malformed("unterminated component")`, so it should already be caught).

- [ ] **Step 6: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "ICalSubscription: parse a feed into per-UID event resources" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: `FeedFetcher` (download, redirects, limits, status mapping)

**Files:**
- Create: `Packages/CalendarConnectors/Sources/ICalSubscription/FeedFetcher.swift`
- Create: `Packages/CalendarConnectors/Tests/ICalSubscriptionTests/FeedFetcherTests.swift`

**Interfaces:**
- Consumes: `HTTPTransport.send(_:) async throws -> HTTPResponse`, `HTTPRequest(url:method:headers:body:)`, `HTTPResponse(status:headers:body:)`, `.header(_:)`.
- Produces: `struct FeedValidators: Sendable, Equatable { var etag: String?; var lastModified: String? }`; `enum FeedResponse: Sendable { case notModified; case body(Data, FeedValidators) }`; `struct FeedFetcher: Sendable { static let maxBytes: Int; static let maxRedirects: Int; init(transport: any HTTPTransport); func fetch(_ url: URL, validators: FeedValidators? = nil) async throws -> FeedResponse }`.

- [ ] **Step 1: Write the failing tests**

Create `Packages/CalendarConnectors/Tests/ICalSubscriptionTests/FeedFetcherTests.swift`:

```swift
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

@Test func anOversizedBodyIsRefused() async {
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
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `swift test --package-path Packages/CalendarConnectors --filter FeedFetcherTests`
Expected: build error, "cannot find 'FeedFetcher' in scope".

- [ ] **Step 3: Implement `FeedFetcher`**

Create `Packages/CalendarConnectors/Sources/ICalSubscription/FeedFetcher.swift`:

```swift
import CalendarCore
import Foundation

/// What a server said about a version of the feed, sent back on the next request so an unchanged feed costs no body.
struct FeedValidators: Sendable, Equatable {
    var etag: String?
    var lastModified: String?
}

enum FeedResponse: Sendable {
    case notModified
    case body(Data, FeedValidators)
}

/// Downloads a feed. The transport must not follow redirects itself (`URLSessionTransport(followsRedirects: false)`), so
/// every hop is checked here: only `https`, at most `maxRedirects`. No message built here contains the link: it is a credential.
struct FeedFetcher: Sendable {
    static let maxBytes = 10 * 1024 * 1024
    static let maxRedirects = 5
    private static let refusedRedirect = "the feed moved somewhere TimeTug will not follow"

    let transport: any HTTPTransport

    func fetch(_ url: URL, validators: FeedValidators? = nil) async throws -> FeedResponse {
        var current = url
        for _ in 0...Self.maxRedirects {
            var headers = ["Accept": "text/calendar, text/plain;q=0.5, */*;q=0.1"]
            if let etag = validators?.etag { headers["If-None-Match"] = etag }
            if let modified = validators?.lastModified { headers["If-Modified-Since"] = modified }
            let response = try await transport.send(HTTPRequest(url: current, headers: headers))
            switch response.status {
            case 200:
                guard response.body.count <= Self.maxBytes else { throw SourceError.invalidResponse("the feed is too large") }
                return .body(response.body, FeedValidators(etag: response.header("etag"), lastModified: response.header("last-modified")))
            case 304 where validators != nil:
                return .notModified
            case 301, 302, 303, 307, 308:
                guard let location = response.header("location"), let next = URL(string: location, relativeTo: current)?.absoluteURL,
                      next.scheme?.lowercased() == "https" else { throw SourceError.invalidResponse(Self.refusedRedirect) }
                current = next
            case 401, 403, 404, 410:
                throw SourceError.authExpired
            case 429, 500...599:
                throw SourceError.server(status: response.status)
            default:
                throw SourceError.invalidResponse("the feed server answered \(response.status)")
            }
        }
        throw SourceError.invalidResponse("the feed redirected too many times")
    }
}
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `swift test --package-path Packages/CalendarConnectors --filter FeedFetcherTests`
Expected: all pass. In `aRedirectLoopStopsAfterFiveRedirects` the loop sends one request for the original plus five redirects (`maxRedirects + 1` = 6) and then throws; if the count is off by one the loop bounds are wrong.

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "ICalSubscription: fetch a feed with checked redirects and size limit" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: `ICalSubscriptionSource` (read, cache, poll)

**Files:**
- Create: `Packages/CalendarConnectors/Sources/ICalSubscription/ICalSubscriptionSource.swift`
- Create: `Packages/CalendarConnectors/Tests/ICalSubscriptionTests/SourceTests.swift`

**Interfaces:**
- Consumes: `FeedFetcher`, `FeedParser`, `ParsedFeed`, `FeedValidators` (Tasks 2, 3); `EventReader`, `EventReadContext`; `ChangeMonitor(interval:maxBackoff:sleep:)`, `PollingCalendarSource`, `CalendarDescriptor(id:title:service:colorHex:permissions:isDefault:timeZone:accountName:kind:...provider:)`, `CalendarService.iCalSubscription`, `CalendarProvider.subscription`.
- Produces: `public final class ICalSubscriptionSource: PollingCalendarSource` with internal `init(connection: Connection, link: @escaping @Sendable () async throws -> URL, transport: any HTTPTransport, monitor: ChangeMonitor, maxAge: TimeInterval, now: @escaping @Sendable () -> Date, defaultZone: TimeZone)`; static `calendarID == "feed"`.

- [ ] **Step 1: Write the failing tests**

Create `Packages/CalendarConnectors/Tests/ICalSubscriptionTests/SourceTests.swift`:

```swift
import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import ICalSubscription

private let berlin = TimeZone(identifier: "Europe/Berlin")!

private func makeSource(_ transport: FakeTransport, now: TestNow = TestNow(), maxAge: TimeInterval = 900) -> ICalSubscriptionSource {
    ICalSubscriptionSource(
        connection: Connection(kindID: "icalsub", connectionID: "c1", displayName: "My Meetups (www.example.test)", config: ["host": "www.example.test"]),
        link: { feedURL }, transport: transport, monitor: ChangeMonitor(interval: .seconds(900), sleep: { _ in }),
        maxAge: maxAge, now: now.provider, defaultZone: berlin)
}

private func ics(_ text: String, etag: String? = nil) -> HTTPResponse {
    HTTPResponse(status: 200, headers: etag.map { ["ETag": $0] } ?? [:], body: Data(text.utf8))
}

@Test func theFeedIsOneReadOnlySubscribedCalendar() async throws {
    let transport = FakeTransport()
    await transport.route(privatePath, [ics(sampleFeed)])
    let source = makeSource(transport)
    let calendars = try await source.calendars()
    let calendar = try #require(calendars.first)
    #expect(calendars.count == 1 && calendar.id == "feed" && calendar.title == "My Meetups")
    #expect(calendar.service == .iCalSubscription && calendar.provider == .subscription && calendar.kind == .subscribed)
    #expect(calendar.permissions.canEdit == false && calendar.permissions.canViewDetails)
    #expect(calendar.timeZone == berlin && calendar.accountName == "My Meetups (www.example.test)")
    #expect(source.id == "icalsub-c1" && !source.capabilities.canWrite && source.capabilities.syncKind == .token)
    #expect(ProvidedFieldsConformance.violations(calendar: calendar, capabilities: source.capabilities).isEmpty)
    #expect(ProvidedFieldsConformance.violations(source: source).isEmpty)
}

@Test func theCalendarNameFallsBackToTheAccountName() async throws {
    let transport = FakeTransport()
    await transport.route(privatePath, [ics(feedICS([boardGames], header: []))])
    let calendar = try #require(try await makeSource(transport).calendars().first)
    #expect(calendar.title == "My Meetups (www.example.test)")
}

@Test func eventsAreExpandedSortedAndConform() async throws {
    let transport = FakeTransport()
    await transport.route(privatePath, [ics(sampleFeed)])
    let source = makeSource(transport)
    let events = try await source.events(in: september)
    #expect(events.count == 6 && events.map(\.start) == events.map(\.start).sorted())
    #expect(Array(events.map(\.title).prefix(3)) == ["Weekly walk", "Weekly walk", "Board games night"])
    #expect(events.allSatisfy { $0.calendarID == "feed" && $0.sourceID == "icalsub-c1" && $0.uidScope == .global })
    for event in events {
        #expect(ProvidedFieldsConformance.violations(event: event, capabilities: source.capabilities).isEmpty, "\(event.title)")
        #expect(AllDayConformance.violations(event).isEmpty, "\(event.title)")
    }
}

@Test func floatingTimesUseTheFeedsZoneElseTheDefault() async throws {
    let floating = ["BEGIN:VEVENT", "UID:float@example.test", "DTSTAMP:20260901T000000Z", "SUMMARY:Floating",
                    "DTSTART:20260910T120000", "DTEND:20260910T130000", "END:VEVENT"]
    let transport = FakeTransport()
    await transport.route(privatePath, [ics(feedICS([floating], header: ["X-WR-TIMEZONE:Asia/Tokyo"]))])
    let tokyo = TimeZone(identifier: "Asia/Tokyo")!
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = tokyo
    let event = try #require(try await makeSource(transport).events(in: september).first)
    #expect(event.start == calendar.date(from: DateComponents(year: 2026, month: 9, day: 10, hour: 12)))

    let plain = FakeTransport()
    await plain.route(privatePath, [ics(feedICS([floating], header: []))])
    let fallback = try #require(try await makeSource(plain).events(in: september).first)
    var berlinCalendar = Calendar(identifier: .gregorian)
    berlinCalendar.timeZone = berlin
    #expect(fallback.start == berlinCalendar.date(from: DateComponents(year: 2026, month: 9, day: 10, hour: 12)))
}

@Test func theParsedFeedIsReusedUntilItIsOlderThanTheMaximumAge() async throws {
    let transport = FakeTransport()
    await transport.route(privatePath, [ics(sampleFeed)])
    let now = TestNow()
    let source = makeSource(transport, now: now)
    _ = try await source.events(in: september)
    _ = try await source.calendars()
    _ = try await source.events(in: september)
    #expect(await transport.requests(matching: privatePath).count == 1)
    now.advance(901)
    _ = try await source.events(in: september)
    #expect(await transport.requests(matching: privatePath).count == 2)
}

@Test func theFirstCheckIsTheBaselineAndLaterChecksReportOnlyRealChanges() async throws {
    let changed = feedICS([boardGames])
    let transport = FakeTransport()
    await transport.route(privatePath, [ics(sampleFeed), ics(sampleFeed), ics(changed)])
    let source = makeSource(transport)
    #expect(try await source.checkForChanges() == nil)
    #expect(try await source.checkForChanges() == nil)
    #expect(try await source.checkForChanges() == .eventsChanged(calendarIDs: ["feed"]))
    #expect(try await source.events(in: september).map(\.title) == ["Board games night"])
}

@Test func aChangeSinceTheEventsWereLoadedIsReportedByTheFirstCheck() async throws {
    let transport = FakeTransport()
    await transport.route(privatePath, [ics(sampleFeed), ics(feedICS([boardGames]))])
    let source = makeSource(transport)
    #expect(try await source.events(in: september).count == 6)
    #expect(try await source.checkForChanges() == .eventsChanged(calendarIDs: ["feed"]))
}

@Test func validatorsLetAnUnchangedFeedAnswer304() async throws {
    let transport = FakeTransport()
    await transport.route(privatePath, [ics(sampleFeed, etag: "\"v1\""), HTTPResponse(status: 304)])
    let source = makeSource(transport)
    #expect(try await source.checkForChanges() == nil)
    #expect(try await source.checkForChanges() == nil)
    let second = try #require(await transport.requests(matching: privatePath).last)
    #expect(second.headers["If-None-Match"] == "\"v1\"")
    #expect(try await source.events(in: september).count == 6)
}

@Test func aRevokedLinkIsAuthExpiredEverywhere() async throws {
    let transport = FakeTransport()
    await transport.route(privatePath, [HTTPResponse(status: 404)])
    let source = makeSource(transport)
    await #expect(throws: SourceError.authExpired) { _ = try await source.events(in: september) }
    await #expect(throws: SourceError.authExpired) { _ = try await source.calendars() }
    await #expect(throws: SourceError.authExpired) { _ = try await source.checkForChanges() }
}

@Test func aPageThatStopsBeingACalendarKeepsTheLastGoodFeed() async throws {
    let transport = FakeTransport()
    await transport.route(privatePath, [ics(sampleFeed), ics("<html>Please sign in</html>")])
    let source = makeSource(transport)
    #expect(try await source.checkForChanges() == nil)
    await #expect(throws: SourceError.invalidResponse("that link did not return a calendar")) { _ = try await source.checkForChanges() }
    #expect(try await source.events(in: september).count == 6)
}

@Test func theChangesStreamReportsAChangeAndStopsOnAuthExpired() async throws {
    let transport = FakeTransport()
    await transport.route(privatePath, [ics(sampleFeed), ics(feedICS([boardGames])), HTTPResponse(status: 404)])
    var iterator = makeSource(transport).changes().makeAsyncIterator()
    #expect(await iterator.next() == .eventsChanged(calendarIDs: ["feed"]))
    #expect(await iterator.next() == .sourceFailed(.authExpired))
    #expect(await iterator.next() == nil)
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `swift test --package-path Packages/CalendarConnectors --filter SourceTests`
Expected: build error, "cannot find 'ICalSubscriptionSource' in scope".

- [ ] **Step 3: Implement the source**

Create `Packages/CalendarConnectors/Sources/ICalSubscription/ICalSubscriptionSource.swift`:

```swift
import CalendarCore
import Foundation
import ICalendar

/// What the source has loaded: the parsed feed, the bytes it came from (to spot a change) and what the server said
/// about that version.
actor FeedState {
    private(set) var feed: ParsedFeed?
    private(set) var body: Data?
    private(set) var validators: FeedValidators?
    private(set) var fetchedAt: Date?

    func store(feed: ParsedFeed, body: Data, validators: FeedValidators, at date: Date) {
        self.feed = feed
        self.body = body
        self.validators = validators
        fetchedAt = date
    }

    /// The feed is unchanged: keep what is held and restart its age.
    func unchanged(validators: FeedValidators?, at date: Date) {
        if let validators, validators.etag != nil || validators.lastModified != nil { self.validators = validators }
        fetchedAt = date
    }
}

/// One iCal subscription link: a read-only calendar of whatever the feed holds, re-read when it is older than `maxAge`.
public final class ICalSubscriptionSource: PollingCalendarSource {
    static let calendarID = "feed"

    private let connection: Connection
    private let link: @Sendable () async throws -> URL
    private let fetcher: FeedFetcher
    private let monitor: ChangeMonitor
    private let maxAge: TimeInterval
    private let now: @Sendable () -> Date
    private let defaultZone: TimeZone
    private let state = FeedState()

    /// `link` is read on every fetch, so a replaced link (after `reauthorize`) is picked up without rebuilding the source.
    init(
        connection: Connection, link: @escaping @Sendable () async throws -> URL, transport: any HTTPTransport,
        monitor: ChangeMonitor, maxAge: TimeInterval, now: @escaping @Sendable () -> Date, defaultZone: TimeZone
    ) {
        self.connection = connection
        self.link = link
        self.fetcher = FeedFetcher(transport: transport)
        self.monitor = monitor
        self.maxAge = maxAge
        self.now = now
        self.defaultZone = defaultZone
    }

    public var id: String { connection.sourceID }
    public var displayName: String { connection.displayName }

    public var capabilities: SourceCapabilities {
        SourceCapabilities(providedFields: [.series, .uidScope, .provider, .calendarTimeZone], syncKind: .token)
    }

    public func calendars() async throws -> [CalendarDescriptor] {
        let feed = try await currentFeed()
        return [CalendarDescriptor(
            id: Self.calendarID, title: feed.name ?? connection.displayName, service: .iCalSubscription, colorHex: feed.colorHex,
            permissions: CalendarPermissions(canViewDetails: true, canEdit: false), isDefault: false,
            timeZone: feed.timeZone ?? defaultZone, accountName: connection.displayName, kind: .subscribed, provider: .subscription)]
    }

    public func events(in interval: DateInterval) async throws -> [CalendarEvent] {
        let feed = try await currentFeed()
        let zone = feed.timeZone ?? defaultZone
        var events: [CalendarEvent] = []
        for item in feed.resources {
            let context = EventReadContext(
                calendarID: Self.calendarID, resourceName: item.name, etag: nil, sourceID: id, calendarZone: zone, selfAddresses: [])
            events += EventReader.events(in: item.resource, overlapping: interval, context: context)
        }
        return events.sorted { ($0.start, $0.id) < ($1.start, $1.id) }
    }

    public func changes() -> AsyncStream<CalendarChange> { monitor.changes(polling: self) }

    /// Re-reads the feed. The first check with nothing loaded yet is the baseline and reports nothing; a feed loaded
    /// earlier by `events(in:)` counts as the baseline, so a change since then is reported.
    public func checkForChanges() async throws -> CalendarChange? {
        let hadBaseline = await state.body != nil
        let changed = try await refresh()
        return hadBaseline && changed ? .eventsChanged(calendarIDs: [Self.calendarID]) : nil
    }

    private func currentFeed() async throws -> ParsedFeed {
        if let feed = await state.feed, let fetchedAt = await state.fetchedAt, now().timeIntervalSince(fetchedAt) < maxAge { return feed }
        try await refresh()
        guard let feed = await state.feed else { throw SourceError.invalidResponse("the feed has not loaded") }
        return feed
    }

    /// Fetches the feed (conditionally when the server gave validators). True when the content changed or was loaded for the
    /// first time. A body that is not a calendar throws and leaves the last good feed in place.
    @discardableResult
    private func refresh() async throws -> Bool {
        let url = try await link()
        let previous = await state.body
        let validators = await state.validators.flatMap { $0.etag != nil || $0.lastModified != nil ? $0 : nil }
        switch try await fetcher.fetch(url, validators: previous == nil ? nil : validators) {
        case .notModified:
            await state.unchanged(validators: nil, at: now())
            return false
        case .body(let data, let newValidators):
            if let previous, previous == data {
                await state.unchanged(validators: newValidators, at: now())
                return false
            }
            let feed = try FeedParser.parse(data)
            await state.store(feed: feed, body: data, validators: newValidators, at: now())
            return true
        }
    }
}
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `swift test --package-path Packages/CalendarConnectors --filter SourceTests`
Expected: all pass. If `theChangesStreamReportsAChangeAndStopsOnAuthExpired` hangs, the test sleeper is not injected (`sleep: { _ in }` in `makeSource`). If `floatingTimesUseTheFeedsZoneElseTheDefault` fails, check `EventReadContext.calendarZone` is `feed.timeZone ?? defaultZone` as above.

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "ICalSubscription: read-only polling source for a feed" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: `ICalSubscriptionKind` (sign-in, reauthorize, live check)

**Files:**
- Create: `Packages/CalendarConnectors/Sources/ICalSubscription/ICalSubscriptionKind.swift`
- Create: `Packages/CalendarConnectors/Tests/ICalSubscriptionTests/KindTests.swift`
- Create: `Packages/CalendarConnectors/Tests/ICalSubscriptionTests/LiveTests.swift`

**Interfaces:**
- Consumes: `ConnectorKind`, `CredentialPromptHelp`, `CredentialHelp(text:linkTitle:url:)`, `CredentialField`, `AuthorizationMethod.password(fields:)`, `AuthorizationInteraction.promptCredentials`, `CredentialStore.setSecrets/secrets`, `Connection(kindID:connectionID:displayName:config:)`, `ChangeMonitor`, `Sleeper`/`defaultSleeper`, `URLSessionTransport(followsRedirects:)`; `FeedLocation`, `FeedFetcher`, `FeedParser`, `ICalSubscriptionSource` (Tasks 1 to 4).
- Produces: `public struct ICalSubscriptionKind: ConnectorKind, CredentialPromptHelp` with `public static let kindID = "icalsub"` and `public init(transport:now:sleep:pollInterval:defaultZone:)`.

- [ ] **Step 1: Write the failing tests**

Create `Packages/CalendarConnectors/Tests/ICalSubscriptionTests/KindTests.swift`:

```swift
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
        #expect(await store.isEmpty && (await transport.requests).isEmpty)
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
```

Create `Packages/CalendarConnectors/Tests/ICalSubscriptionTests/LiveTests.swift`:

```swift
import CalendarCore
import Foundation
import Testing
@testable import ICalSubscription

/// Opt-in, never in CI: `TIMETUG_LIVE_ICALSUB=1 swift test --package-path Packages/CalendarConnectors --filter liveFeedLoads`.
/// Reads one feed link from the git-ignored `~/.config/timetug/icalsub-live`. Prints counts only, never the link or any event text.
@Test(.enabled(if: ProcessInfo.processInfo.environment["TIMETUG_LIVE_ICALSUB"] == "1"))
func liveFeedLoads() async throws {
    let path = NSString(string: "~/.config/timetug/icalsub-live").expandingTildeInPath
    let url = try FeedLocation.url(from: try String(contentsOfFile: path, encoding: .utf8))
    let source = ICalSubscriptionSource(
        connection: Connection(kindID: "icalsub", connectionID: "live", displayName: "Live"), link: { url },
        transport: URLSessionTransport(followsRedirects: false), monitor: ChangeMonitor(), maxAge: 900,
        now: { Date() }, defaultZone: .current)
    let events = try await source.events(in: DateInterval(start: Date(), duration: 90 * 86_400))
    print("LIVE calendars:", try await source.calendars().count, "events in 90 days:", events.count,
          "recurring:", events.filter { $0.series != .notRecurring }.count, "all-day:", events.filter(\.isAllDay).count,
          "with a link:", events.filter { !$0.conferences.isEmpty }.count)
    #expect(try await source.checkForChanges() == nil)
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `swift test --package-path Packages/CalendarConnectors --filter KindTests`
Expected: build error, "cannot find 'ICalSubscriptionKind' in scope".

- [ ] **Step 3: Implement the kind**

Create `Packages/CalendarConnectors/Sources/ICalSubscription/ICalSubscriptionKind.swift`:

```swift
import CalendarCore
import Foundation

/// A calendar subscription link (Meetup's Add to calendar links, a Google secret address, ...). One secret field: the
/// link. It is the credential, so it is stored only in the `CredentialStore`; the connection keeps just the host.
public struct ICalSubscriptionKind: ConnectorKind, CredentialPromptHelp {
    public static let kindID = "icalsub"
    static let linkKey = "link"
    private static let fields = [CredentialField(key: ICalSubscriptionKind.linkKey, label: "iCal link", isSecret: true)]

    private let transport: any HTTPTransport
    private let now: @Sendable () -> Date
    private let sleep: Sleeper
    private let pollInterval: Duration
    private let defaultZone: TimeZone

    public init(
        transport: any HTTPTransport = URLSessionTransport(followsRedirects: false), now: @escaping @Sendable () -> Date = { Date() },
        sleep: @escaping Sleeper = defaultSleeper, pollInterval: Duration = .seconds(900), defaultZone: TimeZone = .current
    ) {
        self.transport = transport
        self.now = now
        self.sleep = sleep
        self.pollInterval = pollInterval
        self.defaultZone = defaultZone
    }

    public var id: String { Self.kindID }
    public var displayName: String { "iCal link" }
    public var supportedPlatforms: Platform { [.macOS, .iOS, .linux, .windows] }
    public var authorization: AuthorizationMethod { .password(fields: Self.fields) }
    public var credentialHelp: CredentialHelp? {
        CredentialHelp(
            text: "Paste a calendar subscription link (webcal:// or https://). Do not use a downloaded .ics file: it will not update. "
                + "In Meetup, open Your events, choose Add to calendar, and copy any link in the menu.",
            linkTitle: "Open Meetup Your events", url: URL(string: "https://www.meetup.com/your-events/"))
    }

    public func authorize(using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        let (url, feed) = try await signIn(using: interaction)
        let host = url.host ?? ""
        let name = feed.name.map { "\($0) (\(host))" } ?? host
        let connection = Connection(kindID: id, connectionID: UUID().uuidString, displayName: name, config: ["host": host])
        try await credentials.setSecrets([Self.linkKey: url.absoluteString], for: connection.connectionID)
        return connection
    }

    /// Replaces the stored link (after the provider revoked or regenerated it). The old link stays if the new one fails.
    public func reauthorize(_ connection: Connection, using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        let (url, _) = try await signIn(using: interaction)
        try await credentials.setSecrets([Self.linkKey: url.absoluteString], for: connection.connectionID)
        var updated = connection
        updated.config["host"] = url.host ?? ""
        return updated
    }

    public func makeSource(for connection: Connection, credentials: any CredentialStore, syncState: any SyncStateStore) throws -> any CalendarSource {
        let connectionID = connection.connectionID
        return ICalSubscriptionSource(
            connection: connection,
            link: {
                guard let text = try await credentials.secrets(for: connectionID)?[Self.linkKey], let url = URL(string: text) else {
                    throw SourceError.authExpired
                }
                return url
            },
            transport: transport, monitor: ChangeMonitor(interval: pollInterval, sleep: sleep),
            maxAge: Double(pollInterval.components.seconds), now: now, defaultZone: defaultZone)
    }

    /// Nothing is stored here: callers store the link only after the feed has loaded and parsed.
    private func signIn(using interaction: any AuthorizationInteraction) async throws -> (URL, ParsedFeed) {
        let values = try await interaction.promptCredentials(Self.fields)
        let url = try FeedLocation.url(from: values[Self.linkKey] ?? "")
        guard case .body(let data, _) = try await FeedFetcher(transport: transport).fetch(url) else {
            throw SourceError.invalidResponse("the feed did not answer")
        }
        return (url, try FeedParser.parse(data))
    }
}
```

- [ ] **Step 4: Run the kind tests, then the whole target**

Run: `swift test --package-path Packages/CalendarConnectors --filter ICalSubscriptionTests`
Expected: all pass (the live test is skipped without `TIMETUG_LIVE_ICALSUB`).

- [ ] **Step 5: Run the whole library suite**

Run: `swift test --package-path Packages/CalendarConnectors`
Expected: PASS, no new warnings from the `ICalSubscription` target (Swift 6 strict concurrency is on).

- [ ] **Step 6: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "ICalSubscription: connector kind with link sign-in and reauthorize" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 6: App wiring, icon, Meetup tile removal, sign-in error text

**Files:**
- Modify: `Apps/macOS/project.yml` (two dependency lists)
- Modify: `Apps/macOS/Sources/AppConnectors.swift`
- Modify: `Apps/macOS/Sources/ProviderIcon.swift`
- Modify: `Apps/macOS/Sources/AccountsPane.swift:14-21` (the placeholder list)
- Modify: `Apps/macOS/Sources/AccountsController.swift:161-168` (`describeSignIn`)
- Modify: `Apps/macOS/Sources/SettingsSearch.swift:40`
- Test: `Apps/macOS/Tests/AppConnectorsTests.swift`, `Apps/macOS/Tests/ProviderIconTests.swift`, `Apps/macOS/Tests/CalendarSectionsTests.swift`, `Apps/macOS/Tests/AccountsControllerTests.swift`

**Interfaces:**
- Consumes: `ICalSubscriptionKind` (Task 5); `CredentialField`.
- Produces: the kind registered under `"icalsub"`; `ProviderIcon.Style.feed`; `AccountsController.describeSignIn` that says "<secret label> was not accepted." for a kind with only a secret field.

- [ ] **Step 1: Write the failing app tests**

In `Apps/macOS/Tests/AppConnectorsTests.swift` add `import ICalSubscription`? No: tests only use the registry. Add inside `AppConnectorsTests`:

```swift
    func testICalLinkIsAlwaysRegistered() {
        let registry = AppConnectors.makeRegistry(google: nil, microsoft: nil, eventKit: EventKitSource())
        XCTAssertEqual(registry.kind(id: "icalsub")?.displayName, "iCal link")
        guard case .password(let fields)? = registry.kind(id: "icalsub")?.authorization else { return XCTFail("the iCal link kind uses the credential sheet") }
        XCTAssertEqual(fields.map(\.key), ["link"])
        XCTAssertNotNil((registry.kind(id: "icalsub") as? CredentialPromptHelp)?.credentialHelp?.url)
    }
```

In `Apps/macOS/Tests/ProviderIconTests.swift` add:

```swift
    func testICalLinkAccountsGetTheirOwnMark() {
        XCTAssertEqual(ProviderIcon.Style.forKind("icalsub"), .feed)
    }
```

In `Apps/macOS/Tests/CalendarSectionsTests.swift`, next to the other `displayName(forKindID:)` assertions (line ~60) add:

```swift
        XCTAssertEqual(ProviderIcon.displayName(forKindID: "icalsub"), "iCal link")
```

In `Apps/macOS/Tests/AccountsControllerTests.swift`, next to the `describeSignIn` assertion (line ~256) add a new test method in the same class:

```swift
    func testRejectedLinkIsDescribedForAKindWithOnlyASecretField() {
        let link = [CredentialField(key: "link", label: "iCal link", isSecret: true)]
        XCTAssertEqual(AccountsController.describeSignIn(CalendarCore.SourceError.authExpired, fields: link), "iCal link was not accepted.")
        XCTAssertEqual(AccountsController.describeSignIn(CalendarCore.SourceError.invalidResponse("that link did not return a calendar"), fields: link),
                       "Sign-in failed: that link did not return a calendar.")
    }
```

- [ ] **Step 2: Add the package dependency and regenerate**

In `Apps/macOS/project.yml` add, after each `product: CalDAVCalendar` line (once in the `TimeTug` target, once in `TimeTugTests`):

```yaml
      - package: CalendarConnectors
        product: ICalSubscription
```

Run: `xcodegen generate --spec Apps/macOS/project.yml && git checkout Apps/macOS/Sources/Info.plist Apps/macOS/Widgets/Info.plist`
Expected: the project is regenerated; the two Info.plist files are restored (AGENTS.md: generation rewrites them).

- [ ] **Step 3: Run the app tests to see them fail**

Run: `xcodebuild -project Apps/macOS/TimeTug.xcodeproj -scheme TimeTug -destination 'platform=macOS' test -only-testing:TimeTugTests/AppConnectorsTests -only-testing:TimeTugTests/ProviderIconTests -only-testing:TimeTugTests/AccountsControllerTests`
Expected: compile errors (`.feed` does not exist) or failures.

- [ ] **Step 4: Register the kind**

In `Apps/macOS/Sources/AppConnectors.swift` add `import ICalSubscription` after `import GoogleCalendar`, and after `registry.register(CalDAVConnectorKind())`:

```swift
        registry.register(ICalSubscriptionKind())
```

- [ ] **Step 5: Give the kind an icon**

In `Apps/macOS/Sources/ProviderIcon.swift`:

- `Style` enum: `case google, microsoft, icloud, caldav, feed, generic`
- `forKind`: add `case "icalsub": .feed` before `default`
- `displayName(forKindID:)`: add `case "icalsub": "iCal link"` before `default`
- `body`: add after the `.caldav` case:

```swift
            case .feed:
                Image(systemName: "link")
                    .resizable().scaledToFit().foregroundStyle(Color(white: 0.35)).padding(size * 0.24)
```

- [ ] **Step 6: Remove the Meetup placeholder tile**

In `Apps/macOS/Sources/AccountsPane.swift` delete this line from `placeholderProviders`:

```swift
        .init(name: "Meetup", systemImage: "person.3"),
```

Leave Fastmail (CalDAV, not a feed link), Todoist, Zoom and Webex. Run: `grep -n "Meetup" Apps/macOS/Sources/AccountsPane.swift`
Expected: no output.

- [ ] **Step 7: Say a rejected link in the kind's own words**

In `Apps/macOS/Sources/AccountsController.swift` replace `describeSignIn` (and keep its doc comment, extending it) with:

```swift
    /// "Apple ID or app-specific password was not accepted." for a rejected password, in the kind's own words;
    /// "iCal link was not accepted." for a kind whose only field is secret; the usual description otherwise.
    static func describeSignIn(_ error: Error, fields: [CredentialField]) -> String {
        guard case CalendarCore.SourceError.authExpired = error, let secret = fields.first(where: \.isSecret) else { return describe(error) }
        guard let name = fields.first(where: { !$0.isSecret && $0.key == "username" }) ?? fields.first(where: { !$0.isSecret }) else {
            return "\(secret.label) was not accepted."
        }
        return "\(name.label) or \(secret.label.prefix(1).lowercased() + secret.label.dropFirst()) was not accepted."
    }
```

- [ ] **Step 8: Add search keywords**

In `Apps/macOS/Sources/SettingsSearch.swift:40` add `"ical", "ics", "webcal", "feed", "subscription", "meetup"` to the accounts entry's `keywords` array (after `"nextcloud"`).

- [ ] **Step 9: Run the app tests to see them pass, then the full app suite and a build**

Run: `xcodebuild -project Apps/macOS/TimeTug.xcodeproj -scheme TimeTug -destination 'platform=macOS' test`
Expected: TEST SUCCEEDED, including the four new tests and all existing `AccountsControllerTests` (the iCloud and CalDAV messages are unchanged).

- [ ] **Step 10: Commit**

```bash
git status --short   # only the intended files; Info.plist files must not appear
git add Apps/macOS
git commit -m "App: add the iCal link account type, drop the Meetup placeholder tile" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Documentation, privacy policy and manual checklist

**Files:**
- Modify: `docs/calendar-connectors-api.md` (header status, part 1 table and dependency line, new part 14, part 11 if it lists the gap)
- Create: `docs/decisions/0018-feed-link-is-a-credential.md`
- Modify: `site/public/privacy.html`
- Modify: `docs/superpowers/specs/2026-10-06-ical-subscription-link-design.md`
- Modify: `docs/manual-tests/macos-checklist.md`
- Modify: `docs/PROGRESS.md`
- Modify: `AGENTS.md`, `README.md`, `docs/architecture.md` only where they list the connector targets (see Step 6)

**Interfaces:**
- Consumes: the behavior built in Tasks 1 to 6.
- Produces: documentation only.

- [ ] **Step 1: Update the API contract**

In `docs/calendar-connectors-api.md`:

1. Header status paragraph: append "; part 14 describes the iCal subscription connector (Phase 6)".
2. Part 1 table: add a row after the `CalDAVCalendar` row:

```markdown
| `CalendarConnectors` / `ICalSubscription` | iCal subscription link connector: one private feed, read-only | `CalendarCore`, `ICalendar` | Portable |
```

3. The "Dependency direction" line: append "; `ICalSubscription` → `ICalendar` → `CalendarCore`".
4. Run `grep -n "ICS subscription\|No ICS" docs/calendar-connectors-api.md`; if part 11 lists "no subscription connector" as a gap, delete that line.
5. Insert this new part immediately before the heading `## Calendar identity and permissions`:

```markdown
## 14. `ICalSubscription` (Phase 6)

One connector kind, `icalsub` ("iCal link"), reads a private calendar subscription link: a `webcal://` or `https://` address that serves a `VCALENDAR` (Meetup's Add to calendar links, a Google secret address, an Outlook published calendar). Design: `docs/superpowers/specs/2026-10-06-ical-subscription-link-design.md`; the credential decision is ADR 0018.

- **Sign-in.** One secret field, `link`. `FeedLocation.url(from:)` maps `webcal`/`webcals` to `https`, allows plain `http` only for the loopback host, and refuses a link with a user name or password, a `file://` URL and a path. The feed is fetched and parsed before anything is stored; a feed with no events is valid.
- **What is stored.** The link, in the `CredentialStore` under `link`. `Connection.config` is `["host": <host>]` and the display name is `"<calendar name> (<host>)"`, or the host alone. The link never appears in a message, `description` or log.
- **Source.** `ICalSubscriptionSource` is a read-only `PollingCalendarSource` with one calendar, id `feed` (service `icalsub`, provider `.subscription`, kind `.subscribed`). The parsed feed is reused while younger than the poll interval (15 minutes). A feed is one `VCALENDAR`, so events are grouped by `UID` into one `EventResource` each and read with `EventReader`; an event without a `UID` gets a stable made-up one from its start and title.
- **Fetching.** `FeedFetcher` follows redirects itself (at most 5, only to `https`), caps the body at 10 MB and sends `If-None-Match` / `If-Modified-Since` when the server gave validators. 401, 403, 404 and 410 are `SourceError.authExpired` (the link was revoked or regenerated); 429 and 5xx are `SourceError.server(status:)`; a 200 that is not a calendar is `SourceError.invalidResponse`, and the last good feed is kept.
- **Change detection.** Each poll refetches the feed and compares its bytes with the last body; a difference is `.eventsChanged(calendarIDs: ["feed"])`. The first check with nothing loaded is the baseline; a feed loaded earlier by `events(in:)` counts as the baseline.
- **Capabilities.** `canWrite = false`, `syncKind = .token`. Provided fields: `series`, `uidScope`, `provider`, `calendarTimeZone`. `participation`, `visibility`, `availability` and `reminders` are not declared: a feed may or may not carry them.
- **Not supported.** A downloaded `.ics` file (a one-time copy that would go stale), writes and RSVP, more than one feed per account.
```

- [ ] **Step 2: Write ADR 0018**

Create `docs/decisions/0018-feed-link-is-a-credential.md`:

```markdown
# ADR 0018: A calendar feed link is a credential, and only the link is supported

**Status:** Accepted

## Context

Meetup's API needs a paid Pro subscription, but its Add to calendar menu offers private subscription links for the events a member has RSVP'd to. Such a link answers without any sign-in (checked 2026-10-05: `200 text/calendar`, no cookies), so possession of the link is the access. Other services (a Google secret address, an Outlook published calendar) work the same way. We first designed importing a downloaded `.ics` file as well, then dropped it: a downloaded file is a one-time copy of what the link serves and would go stale.

## Decision

- **One connector kind, `icalsub`, takes only a subscription link.** No file import and no Meetup-specific kind; Meetup appears as an example in the help text.
- **The link is stored like a password**: in the Keychain through `CredentialStore`, entered in a masked field, kept out of `Connection.config`, display names, logs and error messages. The connection keeps only the host.
- **A link the provider no longer honors (401, 403, 404, 410) is `authExpired`**, so the app asks for a new link through the existing sign-in-again flow. A temporary 404 from a provider can cause one needless prompt; that is accepted.
- **Redirects are followed by the library, only to `https`**, so a feed cannot send the link on to a plain-HTTP address.

## Consequences

- The privacy policy states that the link is a private address, where it is kept, and that nothing is written back.
- Anyone who holds the link can read the feed; the user revokes it at the provider.
- Writes and RSVP changes are out of reach for a feed; the source is read-only.
```

- [ ] **Step 3: Update the privacy policy**

In `site/public/privacy.html`:

1. After the `<li>` that begins `<strong>iCloud and other CalDAV calendars.</strong>` (ends `only sends them to that provider.</li>`), add:

```html
      <li><strong>Calendar subscription links, if you add one.</strong> You can paste an iCal link (a
        <code>webcal://</code> or <code>https://</code> address) that a service such as Meetup gives you for
        your events. TimeTug downloads that address every 15 minutes and reads the events in it: their titles,
        times, places, descriptions and links. It only reads; it cannot change anything at the service. Anyone
        who has the link can read the same feed, so treat it like a password, and revoke or replace it at the
        service if it leaks.</li>
```

2. In the Keychain list (`<li>For iCloud and other CalDAV providers: ...`), add after it:

```html
      <li>For calendar subscription links: the link you pasted.</li>
```

3. In the "Your list of accounts" bullet, change `(Google, Microsoft, iCloud, CalDAV)` to `(Google, Microsoft, iCloud, CalDAV, iCal link)` and after `and the email addresses your account uses on invitations.` add ` For an iCal link only the web site name is kept in this file; the link itself is in the Keychain.`
4. In the "Encrypted in transit" bullet, add after the CalDAV sentence: `For calendar subscription links TimeTug refuses any address that is not https:// and will not follow a redirect to one.` Keep the sentence structure of that paragraph intact: read it first and place the sentence where it reads naturally.
5. Run: `grep -n "Soon\|Meetup" site/public/privacy.html` and check nothing now contradicts the policy.

- [ ] **Step 4: Align the spec with what was built**

In `docs/superpowers/specs/2026-10-06-ical-subscription-link-design.md`:

1. Replace the "Change detection" paragraph's digest sentence with: "Each poll refetches the feed (or sends the conditional request) and compares the body with the last one it read; a difference returns `.eventsChanged(calendarIDs: [\"feed\"])`. The first check with nothing loaded is the baseline and returns nil; a feed loaded earlier by `events(in:)` counts as the baseline. Nothing is persisted: the app reloads events at launch anyway."
2. In "Security and privacy", change "Only the change-detection digest lives in the app's support folder, like other sync state." to "Nothing else about the feed is stored."
3. In the privacy bullet, change "(the link in the Keychain, and a digest to spot changes)" to "(the link in the Keychain)".
4. In the "Placement" table, App row: append ", `ProviderIcon` style, the Meetup \"Soon\" tile removed, sign-in error text for a one-field kind".
5. Change the status line to "implemented on the branch that adds `docs/superpowers/plans/2026-10-06-calendar-connectors-phase6-ical-link.md`; see that plan".

- [ ] **Step 5: Manual checklist and progress log**

Append to `docs/manual-tests/macos-checklist.md`:

```markdown
## iCal link accounts
- [ ] Settings > Accounts > Add an Account shows an "iCal link" tile with a link icon, and no "Meetup / Soon" tile.
- [ ] Clicking it opens the credential sheet with one masked "iCal link" field and the Meetup help text and link.
- [ ] A link copied from Meetup (Your events > Add to calendar) signs in; the account is named "My Meetups (www.meetup.com)" and its RSVP'd events appear in the popup, with recurring ones expanded.
- [ ] A bad address (`file:///...`, plain `http://`, a pasted file path) shows a plain message in the sheet and adds no account.
- [ ] A Meetup link you have regenerated or revoked: the account shows it needs signing in; "Sign in again" with the new link restores it; the sheet says "iCal link was not accepted." for a dead link.
- [ ] RSVP to a new Meetup: after at most 15 minutes the event appears without relaunching.
- [ ] Removing the account removes its calendars and the Keychain item (Keychain Access: no TimeTug entry for it).
```

Append a dated note to `docs/PROGRESS.md` "Notes": `- 2026-10-06: Phase 6 (iCal subscription link connector): spec docs/superpowers/specs/2026-10-06-ical-subscription-link-design.md, plan docs/superpowers/plans/2026-10-06-calendar-connectors-phase6-ical-link.md; opt-in live check TIMETUG_LIVE_ICALSUB=1 with ~/.config/timetug/icalsub-live; user to run the "iCal link accounts" manual checklist.`

- [ ] **Step 6: Find any other target lists**

Run: `grep -rn "CalDAVCalendar" AGENTS.md README.md docs/architecture.md .github Packages/CalendarConnectors/README.md 2>/dev/null`
For each hit that lists the connector targets or products, add `ICalSubscription` beside `CalDAVCalendar` in the same style. Do not touch CI workflows: `core-linux` already runs `swift test --package-path Packages/CalendarConnectors`, which now includes the new tests.

- [ ] **Step 7: Commit**

```bash
git add docs site AGENTS.md README.md
git commit -m "Docs: iCal subscription connector contract, ADR 0018, privacy policy, checklist" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 8: Verify and open the pull request

**Files:** none.

- [ ] **Step 1: Run every suite that this change can affect**

Run each and confirm PASS:

```bash
swift test --package-path Packages/CalendarConnectors
swift test --package-path Packages/CalendarBridge
xcodebuild -project Apps/macOS/TimeTug.xcodeproj -scheme TimeTug -destination 'platform=macOS' test
```

- [ ] **Step 2: Confirm nothing sensitive or stray is committed**

Run: `git diff --stat origin/master...HEAD && git diff origin/master...HEAD | grep -n "meetup.com/events/ical\|075109c9" || echo "no real feed link in the diff"`
Expected: the last line prints "no real feed link in the diff". Also confirm `git status --short` shows no `Info.plist` or `.xcodeproj` changes.

- [ ] **Step 3: Run the opt-in live check once, if the user has put a feed link in `~/.config/timetug/icalsub-live`**

Run: `TIMETUG_LIVE_ICALSUB=1 swift test --package-path Packages/CalendarConnectors --filter liveFeedLoads`
Expected: a `LIVE calendars: 1 events in 90 days: N ...` line and PASS. Never print or paste the link. If the file does not exist, skip and say so.

- [ ] **Step 4: Push and open the pull request**

```bash
git push -u origin HEAD
gh pr create --base master --title "Calendar connectors Phase 6: iCal subscription link" --body "$(cat <<'EOF'
## Summary
- New `ICalSubscription` connector (`icalsub`, "iCal link"): paste a webcal:// or https:// subscription link (for example Meetup's Add to calendar links) and TimeTug reads it every 15 minutes, read-only.
- The link is stored in the Keychain like a password and never appears in errors or logs; only the host is kept in the account list.
- App: new account type and icon, the redundant Meetup "Soon" tile removed, and a sign-in error that reads "iCal link was not accepted." for a dead link.
- Docs: API contract part 14, ADR 0018, privacy policy, manual checklist. Spec and plan under docs/superpowers.

## Test plan
- [ ] `swift test --package-path Packages/CalendarConnectors`
- [ ] App tests via xcodebuild
- [ ] Manual: "iCal link accounts" section of docs/manual-tests/macos-checklist.md

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Then call `mcp__ccd_pr__get_status` and bind the PR with `mcp__ccd_pr__bind_pr` if it is not already reported; do not poll CI yourself.
