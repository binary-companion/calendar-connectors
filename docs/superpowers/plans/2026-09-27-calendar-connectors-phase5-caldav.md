# Calendar Connectors Phase 5 (CalDAV and iCloud) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let TimeTug users add an iCloud account (Apple ID and app-specific password) or any other CalDAV account in Settings > Accounts, with the same read, change-detection, write and series support as Google and Microsoft.

**Architecture:** Three layers in the portable `Packages/CalendarConnectors` library. `CalendarCore` gains a recurrence expander (`RecurrenceRule.instances`, `RecurrenceSet.occurrences`). A new `ICalendar` product parses and writes iCalendar and maps `VEVENT` to and from `CalendarEvent`. A new `CalDAVCalendar` product holds a small WebDAV client, discovery, two connector kinds (`icloud`, `caldav`) and one `CalDAVCalendarSource`. The macOS app gains a generic credential sheet and registers both kinds.

**Tech Stack:** Swift 6 (library, tools 6.0, Swift Testing), `FoundationXML` on Linux, Swift 5 mode + SwiftUI + XCTest (app), XcodeGen.

**Spec:** `docs/superpowers/specs/2026-09-27-calendar-connectors-phase5-caldav-design.md` (read it first; this plan argues from it).

## Global Constraints

- The library stays dependency-free and portable: no Apple-only frameworks in `CalendarCore`, `ICalendar` or `CalDAVCalendar`; everything builds and tests on Linux swift 6.0 (`core-linux` CI job). XML parsing uses `XMLParser` with `#if canImport(FoundationXML) import FoundationXML #endif`; networking types need `#if canImport(FoundationNetworking) import FoundationNetworking #endif`.
- `ICalendar` depends only on `CalendarCore`. `CalDAVCalendar` depends only on `CalendarCore` and `ICalendar`.
- Kind ids and names: `icloud` / "iCloud" (server fixed at `https://caldav.icloud.com`, host base `icloud.com`, provider `.iCloud`); `caldav` / "Other CalDAV" (server entered, provider `.calDAV`). Service `.calDAV` (`"caldav"`) for both.
- Credential field keys: `serverURL` (CalDAV only), `username`, `password` (secret). The Keychain stores `username` and `password`; `Connection.config` holds `serverURL`, `username`, `principalURL`, `homeURL`, `userAddresses` (newline-separated) and `autoSchedule` (`"true"`/`"false"`). Never a password in config, logs or test output.
- Basic auth only over HTTPS (plain `http` only to `localhost`/`127.0.0.1`); credentials only to `host == base || host.hasSuffix("." + base)`; redirects followed by `WebDAVClient` itself (max 5) over a transport built with `followsRedirects: false`.
- `controlsNotifications = false`. A write that would tell someone else (other attendees on create/update/delete; every `respond`) with a policy other than `.all` throws `WriteError.unsupported(fields: [.attendees])` before any request.
- Expansion limit: 5000 instances per resource per query; iteration cap 200,000 periods.
- `xcodegen generate` rewrites `Apps/macOS/Sources/Info.plist` and `Apps/macOS/Widgets/Info.plist`: restore them with `git checkout` before committing.
- Library tests: `swift test --package-path Packages/CalendarConnectors`. App tests: from `Apps/macOS`, `xcodegen generate` then `xcodebuild -project TimeTug.xcodeproj -scheme TimeTug -destination 'platform=macOS' test`.
- Do not merge the PR; the user squash-merges when ready. Live tests are opt-in (`TIMETUG_LIVE_ICLOUD=1`) and never run in CI.
- Match the surrounding code: Swift Testing `@Test`/`#expect`/`#require` in the library, doc comments on public API in the existing voice (plain sentences, no marketing), no `print` outside live tests.

## Review Focus

1. **An old recurring event** (daily or weekly since 2015, no `COUNT`) must still show today's instances: the expander fast-forwards to the window and the 5000-instance limit counts only emitted instances. Test: Task 3, `oldDailySeriesStillShowsThisWeek`.
2. **An invitation to one occurrence only** (a resource with a `RECURRENCE-ID` override and no master) must show that occurrence. Test: Task 6, `overrideWithoutMasterIsShown`.
3. **A non-IANA `TZID`** (`Eastern Standard Time`, `/mozilla.org/20050126_1/America/New_York`, or a custom name with only a `VTIMEZONE`) must resolve to the right offsets. Test: Task 5, `resolvesWindowsMozillaAndCustomZones`.
4. **A password with non-ASCII characters or a colon** must be sent as UTF-8 Basic auth and never appear in errors. Test: Task 9, `basicAuthEncodesUTF8AndColons`.
5. **A server that answers with other XML prefixes or LF-only iCalendar** (`d:` instead of `D:`, no `CRLF`, a final line without newline) must parse the same. Tests: Task 4, `parsesLFOnlyAndMissingFinalNewline`; Task 9, `multistatusIgnoresPrefixes`.

---

## File map

| File | Responsibility |
|---|---|
| Move `Sources/MicrosoftCalendar/WindowsTimeZones.swift` → `Sources/CalendarCore/WindowsTimeZones.swift` | Windows ↔ IANA names, now public, shared with `ICalendar` |
| Modify `Sources/CalendarCore/Model.swift` | `CalendarService.calDAV` |
| Modify `Sources/CalendarCore/Connection.swift` | `CredentialHelp`, `CredentialPromptHelp` |
| Modify `Sources/CalendarCore/HTTP.swift` | `URLSessionTransport(configuration:followsRedirects:)` |
| Create `Sources/CalendarCore/Recurrence/RuleExpander.swift` | `RecurrenceRule.instances(...)`, `RuleExpansion` |
| Create `Sources/CalendarCore/Recurrence/RecurrenceSet+Occurrences.swift` | `RecurrenceSet.occurrences(...)`, `ruleInstanceCount(...)`, `RecurrenceExpansion` |
| Create `Sources/ICalendar/ContentLine.swift`, `Component.swift`, `ICalText.swift` | Parser/serializer, component tree, escaping |
| Create `Sources/ICalendar/ICalValues.swift`, `TimeZoneResolver.swift`, `VTimeZoneWriter.swift` | Date-time, date, duration values; `VTIMEZONE` in and out |
| Create `Sources/ICalendar/EventResource.swift`, `EventReader.swift`, `AlarmMapper.swift`, `AttendeeMapper.swift` | A calendar object resource; `VEVENT` → `CalendarEvent`; `VALARM` ↔ `Reminder`; `ATTENDEE`/`ORGANIZER` |
| Create `Sources/ICalendar/EventWriter.swift`, `SeriesEditor.swift` | Draft → `VEVENT`, patch applier; overrides, `EXDATE`, split arithmetic |
| Create `Sources/CalDAVCalendar/XMLTree.swift`, `DAVXML.swift`, `WebDAVClient.swift` | XML DOM, multistatus and request bodies, HTTP with auth, redirects, host rule |
| Create `Sources/CalDAVCalendar/CalDAVAccountConfig.swift`, `CalDAVDiscovery.swift`, `CalDAVConnectorKinds.swift` | Config keys, discovery, `ICloudConnectorKind`, `CalDAVConnectorKind` |
| Create `Sources/CalDAVCalendar/CalDAVCalendarSource.swift` (+ `+Sync`, `+Write`, `+Split`, `+Series`) | The source |
| Create `Tests/ICalendarTests/*`, `Tests/CalDAVCalendarTests/*` (incl. `FakeCalDAVServer.swift`, `ICloudLiveSmokeTests.swift`) | Tests |
| Modify `Packages/CalendarConnectors/Package.swift` | Two products, two targets, two test targets |
| Create `Apps/macOS/Sources/CredentialPrompter.swift`, `CredentialSheet.swift` | The generic sheet and its model |
| Modify `Apps/macOS/Sources/AppConnectors.swift`, `AppCoordinator.swift`, `AccountsController.swift`, `AccountsPane.swift`, `ProviderIcon.swift`, `SettingsSearch.swift`, `Apps/macOS/project.yml` | App wiring |
| Create `Apps/macOS/Tests/CredentialPrompterTests.swift`; modify `AppConnectorsTests.swift`, `AccountsControllerTests.swift`, `ProviderIconTests.swift`, `CalendarSectionsTests.swift`, `SettingsSearchTests.swift` | App tests |
| Modify `scripts/ci/check-architecture.sh`, `scripts/ci/tests/test-check-architecture.sh` | Allow `FoundationXML` in the connector library |
| Create `docs/decisions/0016-icalendar-and-client-side-expansion.md`; modify `docs/calendar-connectors-api.md`, `AGENTS.md`, `.agents/skills/running-live-calendar-tests/SKILL.md`, `.agents/skills/configuring-local-app-builds/SKILL.md`, `docs/decisions/0014-reminder-model.md`, `docs/decisions/0015-recurrence-model.md`, the spec | Docs |

All library paths are under `Packages/CalendarConnectors/`.

---

## Part A: CalendarCore

### Task 1: Groundwork (shared zones, service, credential help, redirect control)

**Files:**
- Move: `Packages/CalendarConnectors/Sources/MicrosoftCalendar/WindowsTimeZones.swift` → `Packages/CalendarConnectors/Sources/CalendarCore/WindowsTimeZones.swift`
- Modify: `Packages/CalendarConnectors/Sources/CalendarCore/Model.swift` (`CalendarService`)
- Modify: `Packages/CalendarConnectors/Sources/CalendarCore/Connection.swift` (after `AuthorizationMethod`)
- Modify: `Packages/CalendarConnectors/Sources/CalendarCore/HTTP.swift`
- Test: `Packages/CalendarConnectors/Tests/CalendarCoreTests/TransportRedirectTests.swift`, `Packages/CalendarConnectors/Tests/CalendarCoreTests/ModelTests.swift`

**Interfaces:**
- Produces: `public enum WindowsTimeZones { public static func timeZone(for name: String) -> TimeZone?; public static func windowsName(for zone: TimeZone) -> String? }`; `CalendarService.calDAV`; `public struct CredentialHelp: Sendable, Hashable { public var text: String; public var linkTitle: String?; public var url: URL? }`; `public protocol CredentialPromptHelp { var credentialHelp: CredentialHelp? { get } }`; `URLSessionTransport.init(configuration: URLSessionConfiguration, followsRedirects: Bool)`.

- [ ] **Step 1: Move `WindowsTimeZones` and make it public**

```bash
git mv Packages/CalendarConnectors/Sources/MicrosoftCalendar/WindowsTimeZones.swift Packages/CalendarConnectors/Sources/CalendarCore/WindowsTimeZones.swift
```

In the moved file change `enum WindowsTimeZones` to `public enum WindowsTimeZones`, and the two `static func` declarations (`timeZone(for:)` at the end, `windowsName(for:)`) to `public static func`. Add `import Foundation` at the top if the file relies on `MicrosoftCalendar` imports (it only needs Foundation). Replace its doc comment's first line with: `/// Windows time zone names (Outlook, Exchange, some iCalendar TZIDs) to IANA identifiers and back.` The Microsoft sources and tests already `import CalendarCore`, so they compile unchanged.

- [ ] **Step 2: Add the service constant and the credential help types**

In `Model.swift`, inside `CalendarService` after `microsoft`:

```swift
    public static let calDAV = CalendarService(rawValue: "caldav")
```

and update the doc comment above the struct from `later "microsoft", "caldav"` to `"microsoft", "caldav"`.

In `Connection.swift`, after `AuthorizationMethod`:

```swift
/// A short explanation a host shows next to a credential prompt (for example that iCloud needs an app-specific
/// password), with an optional link.
public struct CredentialHelp: Sendable, Hashable {
    public var text: String
    public var linkTitle: String?
    public var url: URL?
    public init(text: String, linkTitle: String? = nil, url: URL? = nil) {
        self.text = text
        self.linkTitle = linkTitle
        self.url = url
    }
}

/// Adopted by a `.password` connector kind that has help to show with its fields. Optional: hosts check with `as?`.
public protocol CredentialPromptHelp {
    var credentialHelp: CredentialHelp? { get }
}
```

- [ ] **Step 3: Write the failing redirect test**

`URLProtocol` stubs work on Apple platforms and on swift-corelibs-foundation, so this test also runs in `core-linux`.

```swift
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

private func configuration() -> URLSessionConfiguration {
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [RedirectingProtocol.self]
    return configuration
}

@Test func transportCanDeclineRedirects() async throws {
    let transport = URLSessionTransport(configuration: configuration(), followsRedirects: false)
    let response = try await transport.send(HTTPRequest(url: URL(string: "https://redirect.test/start")!))
    #expect(response.status == 302)
    #expect(response.header("location") == "https://redirect.test/end")
}

@Test func transportFollowsRedirectsByDefault() async throws {
    let transport = URLSessionTransport(configuration: configuration(), followsRedirects: true)
    let response = try await transport.send(HTTPRequest(url: URL(string: "https://redirect.test/start")!))
    #expect(response.status == 200)
    #expect(String(decoding: response.body, as: UTF8.self) == "arrived")
}
```

In `ModelTests.swift` add:

```swift
@Test func calDAVServiceRawValue() {
    #expect(CalendarService.calDAV.rawValue == "caldav")
}
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `swift test --package-path Packages/CalendarConnectors --filter "transport|calDAVService"`
Expected: compile error, `URLSessionTransport` has no `init(configuration:followsRedirects:)`.

- [ ] **Step 5: Implement `followsRedirects`**

In `HTTP.swift`, replace `URLSessionTransport`'s stored property and initializer with:

```swift
public struct URLSessionTransport: HTTPTransport {
    private let session: URLSession
    public init(session: URLSession = .shared) { self.session = session }

    /// A transport with its own session. With `followsRedirects: false` a 3xx comes back as the response instead of
    /// being followed, so the caller can check where it points before sending credentials there (CalDAV).
    public init(configuration: URLSessionConfiguration = .default, followsRedirects: Bool) {
        if followsRedirects {
            self.session = URLSession(configuration: configuration)
        } else {
            self.session = URLSession(configuration: configuration, delegate: RedirectRefusingDelegate(), delegateQueue: nil)
        }
    }
```

(keep `send` unchanged) and add below the struct:

```swift
/// Declines every redirect, so the task finishes with the 3xx response.
private final class RedirectRefusingDelegate: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
    func urlSession(
        _ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void
    ) {
        completionHandler(nil)
    }
}
```

- [ ] **Step 6: Run the whole package's tests**

Run: `swift test --package-path Packages/CalendarConnectors`
Expected: PASS (the Microsoft tests still pass after the move).

- [ ] **Step 7: Commit**

```bash
git add -A Packages/CalendarConnectors
git commit -m "CalendarCore: shared Windows zones, caldav service, credential help, redirect control"
```

---

### Task 2: The rule expander (`RecurrenceRule.instances`)

**Files:**
- Create: `Packages/CalendarConnectors/Sources/CalendarCore/Recurrence/RuleExpander.swift`
- Test: `Packages/CalendarConnectors/Tests/CalendarCoreTests/RuleExpanderTests.swift`

**Interfaces:**
- Consumes: `RecurrenceRule` (all parts), `CalendarDate`, `AllDay`.
- Produces:
  ```swift
  public struct RuleExpansion: Sendable, Equatable { public var starts: [Date]; public var truncated: Bool }
  extension RecurrenceRule {
      public func instances(anchor: Date, timeZone: TimeZone, isAllDay: Bool, before bound: Date, limit: Int,
                            skipTo: Date? = nil) -> RuleExpansion
  }
  ```
  `starts` is in order and starts with `anchor` (when `anchor < bound`); `COUNT` counts `anchor` as the first instance; `UNTIL` is inclusive; `limit` caps emitted starts; `skipTo` (ignored with `COUNT`) skips whole periods that end before it.
  Also `public enum WallClock { public static func date(_ day: CalendarDate, hour: Int, minute: Int, second: Int, in zone: TimeZone) -> Date? }` (a skipped local time moves forward by the gap; a repeated one uses the first), used again by `ICalendar` in Task 5.

- [ ] **Step 1: Write the failing tests (RFC 5545 section 3.8.5.3 examples, DST, all-day, limits)**

```swift
import CalendarCore
import Foundation
import Testing

private let ny = TimeZone(identifier: "America/New_York")!

private func local(_ y: Int, _ mo: Int, _ d: Int, _ h: Int = 9, _ mi: Int = 0, _ s: Int = 0, zone: TimeZone = ny) -> Date {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = zone
    return calendar.date(from: DateComponents(year: y, month: mo, day: d, hour: h, minute: mi, second: s))!
}

/// Local "yyyy-MM-dd HH:mm" texts, easy to compare with the RFC's listings.
private func texts(_ dates: [Date], zone: TimeZone = ny) -> [String] {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = zone
    return dates.map {
        let c = calendar.dateComponents([.year, .month, .day, .hour, .minute], from: $0)
        return String(format: "%04d-%02d-%02d %02d:%02d", c.year!, c.month!, c.day!, c.hour!, c.minute!)
    }
}

private func expand(_ rule: String, _ anchor: Date, before: Date = local(2010, 1, 1), limit: Int = 5000, skipTo: Date? = nil) throws -> [String] {
    let r = try RecurrenceRule(rrule: rule, in: ny)
    return texts(r.instances(anchor: anchor, timeZone: ny, isAllDay: false, before: before, limit: limit, skipTo: skipTo).starts)
}

private func days(_ list: [String]) -> [String] { list.map { String($0.prefix(10)) } }

@Test func dailyForTenOccurrences() throws {
    #expect(days(try expand("FREQ=DAILY;COUNT=10", local(1997, 9, 2))) ==
        (2...11).map { String(format: "1997-09-%02d", $0) })
}

@Test func dailyUntilIsInclusiveAndCountsCorrectly() throws {
    let result = try expand("FREQ=DAILY;UNTIL=19971224T000000Z", local(1997, 9, 2))
    #expect(result.count == 113)
    #expect(result.last == "1997-12-23 09:00")
}

@Test func everyTenDaysFiveTimes() throws {
    #expect(days(try expand("FREQ=DAILY;INTERVAL=10;COUNT=5", local(1997, 9, 2))) ==
        ["1997-09-02", "1997-09-12", "1997-09-22", "1997-10-02", "1997-10-12"])
}

@Test func everyDayInJanuaryForThreeYears() throws {
    let result = try expand("FREQ=YEARLY;UNTIL=20000131T140000Z;BYMONTH=1;BYDAY=SU,MO,TU,WE,TH,FR,SA", local(1998, 1, 1))
    #expect(result.count == 93)
    #expect(result.allSatisfy { $0.dropFirst(5).hasPrefix("01-") })
}

@Test func weeklyForTenOccurrences() throws {
    #expect(days(try expand("FREQ=WEEKLY;COUNT=10", local(1997, 9, 2))) ==
        ["1997-09-02", "1997-09-09", "1997-09-16", "1997-09-23", "1997-09-30",
         "1997-10-07", "1997-10-14", "1997-10-21", "1997-10-28", "1997-11-04"])
}

@Test func weeklyTuesdayThursdayForFiveWeeks() throws {
    #expect(days(try expand("FREQ=WEEKLY;UNTIL=19971007T000000Z;WKST=SU;BYDAY=TU,TH", local(1997, 9, 2))) ==
        ["1997-09-02", "1997-09-04", "1997-09-09", "1997-09-11", "1997-09-16",
         "1997-09-18", "1997-09-23", "1997-09-25", "1997-09-30", "1997-10-02"])
}

@Test func everyOtherWeekMondayWednesdayFriday() throws {
    #expect(days(try expand("FREQ=WEEKLY;INTERVAL=2;UNTIL=19971224T000000Z;WKST=SU;BYDAY=MO,WE,FR", local(1997, 9, 1))) ==
        ["1997-09-01", "1997-09-03", "1997-09-05", "1997-09-15", "1997-09-17", "1997-09-19", "1997-09-29",
         "1997-10-01", "1997-10-03", "1997-10-13", "1997-10-15", "1997-10-17", "1997-10-27", "1997-10-29", "1997-10-31",
         "1997-11-10", "1997-11-12", "1997-11-14", "1997-11-24", "1997-11-26", "1997-11-28",
         "1997-12-08", "1997-12-10", "1997-12-12", "1997-12-22"])
}

@Test func weekStartChangesTheResult() throws {
    #expect(days(try expand("FREQ=WEEKLY;INTERVAL=2;COUNT=4;BYDAY=TU,SU;WKST=MO", local(1997, 8, 5))) ==
        ["1997-08-05", "1997-08-10", "1997-08-19", "1997-08-24"])
    #expect(days(try expand("FREQ=WEEKLY;INTERVAL=2;COUNT=4;BYDAY=TU,SU;WKST=SU", local(1997, 8, 5))) ==
        ["1997-08-05", "1997-08-17", "1997-08-19", "1997-08-31"])
}

@Test func monthlyFirstFriday() throws {
    #expect(days(try expand("FREQ=MONTHLY;COUNT=10;BYDAY=1FR", local(1997, 9, 5))) ==
        ["1997-09-05", "1997-10-03", "1997-11-07", "1997-12-05", "1998-01-02",
         "1998-02-06", "1998-03-06", "1998-04-03", "1998-05-01", "1998-06-05"])
}

@Test func monthlySecondToLastMonday() throws {
    #expect(days(try expand("FREQ=MONTHLY;COUNT=6;BYDAY=-2MO", local(1997, 9, 22))) ==
        ["1997-09-22", "1997-10-20", "1997-11-17", "1997-12-22", "1998-01-19", "1998-02-16"])
}

@Test func monthlyThirdToLastDay() throws {
    #expect(days(try expand("FREQ=MONTHLY;COUNT=6;BYMONTHDAY=-3", local(1997, 9, 28))) ==
        ["1997-09-28", "1997-10-29", "1997-11-28", "1997-12-29", "1998-01-29", "1998-02-26"])
}

@Test func lastWorkdayOfTheMonth() throws {
    #expect(days(try expand("FREQ=MONTHLY;COUNT=7;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1", local(1997, 9, 30))) ==
        ["1997-09-30", "1997-10-31", "1997-11-28", "1997-12-31", "1998-01-30", "1998-02-27", "1998-03-31"])
}

@Test func thirdTuesdayWednesdayOrThursday() throws {
    #expect(days(try expand("FREQ=MONTHLY;COUNT=3;BYDAY=TU,WE,TH;BYSETPOS=3", local(1997, 9, 4))) ==
        ["1997-09-04", "1997-10-07", "1997-11-06"])
}

@Test func invalidDatesAreSkipped() throws {
    #expect(days(try expand("FREQ=MONTHLY;BYMONTHDAY=15,30;COUNT=5", local(2007, 1, 15))) ==
        ["2007-01-15", "2007-01-30", "2007-02-15", "2007-03-15", "2007-03-30"])
}

@Test func fridayThe13th() throws {
    // The RFC lists the rule with an EXDATE for the anchor; the rule alone starts with the anchor.
    let result = try expand("FREQ=MONTHLY;BYDAY=FR;BYMONTHDAY=13", local(1997, 9, 2), before: local(2000, 12, 31))
    #expect(days(Array(result.dropFirst())) == ["1998-02-13", "1998-03-13", "1998-11-13", "1999-08-13", "2000-10-13"])
}

@Test func yearlyInJuneAndJuly() throws {
    #expect(days(try expand("FREQ=YEARLY;COUNT=10;BYMONTH=6,7", local(1997, 6, 10))) ==
        ["1997-06-10", "1997-07-10", "1998-06-10", "1998-07-10", "1999-06-10",
         "1999-07-10", "2000-06-10", "2000-07-10", "2001-06-10", "2001-07-10"])
}

@Test func mondayOfWeekTwenty() throws {
    #expect(days(try expand("FREQ=YEARLY;BYWEEKNO=20;BYDAY=MO", local(1997, 5, 12), before: local(2000, 1, 1))) ==
        ["1997-05-12", "1998-05-11", "1999-05-17"])
}

@Test func everyThirdYearOnYearDays() throws {
    #expect(days(try expand("FREQ=YEARLY;INTERVAL=3;COUNT=10;BYYEARDAY=1,100,200", local(1997, 1, 1))) ==
        ["1997-01-01", "1997-04-10", "1997-07-19", "2000-01-01", "2000-04-09",
         "2000-07-18", "2003-01-01", "2003-04-10", "2003-07-19", "2006-01-01"])
}

@Test func everyThursdayInMarch() throws {
    #expect(days(try expand("FREQ=YEARLY;BYMONTH=3;BYDAY=TH", local(1997, 3, 13), before: local(1999, 12, 31))) ==
        ["1997-03-13", "1997-03-20", "1997-03-27", "1998-03-05", "1998-03-12", "1998-03-19", "1998-03-26",
         "1999-03-04", "1999-03-11", "1999-03-18", "1999-03-25"])
}

@Test func twentiethMondayOfTheYear() throws {
    #expect(days(try expand("FREQ=YEARLY;BYDAY=20MO", local(1997, 5, 19), before: local(2000, 1, 1))) ==
        ["1997-05-19", "1998-05-18", "1999-05-17"])
}

@Test func presidentialElectionDay() throws {
    #expect(days(try expand("FREQ=YEARLY;INTERVAL=4;BYMONTH=11;BYDAY=TU;BYMONTHDAY=2,3,4,5,6,7,8", local(1996, 11, 5), before: local(2005, 1, 1))) ==
        ["1996-11-05", "2000-11-07", "2004-11-02"])
}

@Test func everyFifteenMinutesSixTimes() throws {
    #expect(try expand("FREQ=MINUTELY;INTERVAL=15;COUNT=6", local(1997, 9, 2)) ==
        ["1997-09-02 09:00", "1997-09-02 09:15", "1997-09-02 09:30", "1997-09-02 09:45", "1997-09-02 10:00", "1997-09-02 10:15"])
}

@Test func everyTwentyMinutesDuringTheDay() throws {
    let result = try expand("FREQ=DAILY;BYHOUR=9,10,11,12,13,14,15,16;BYMINUTE=0,20,40", local(1997, 9, 2), before: local(1997, 9, 3, 0))
    #expect(result.count == 24)
    #expect(result.first == "1997-09-02 09:00")
    #expect(result.last == "1997-09-02 16:40")
}

@Test func keepsWallClockAcrossDST() throws {
    let result = try expand("FREQ=DAILY;COUNT=3", local(2026, 3, 7, 9), before: local(2027, 1, 1))
    #expect(result == ["2026-03-07 09:00", "2026-03-08 09:00", "2026-03-09 09:00"])
}

@Test func missingLocalTimeMovesForward() throws {
    let result = try expand("FREQ=DAILY;COUNT=3", local(2026, 3, 7, 2, 30), before: local(2027, 1, 1))
    #expect(result == ["2026-03-07 02:30", "2026-03-08 03:30", "2026-03-09 02:30"])
}

@Test func repeatedLocalTimeUsesTheFirst() throws {
    let rule = try RecurrenceRule(rrule: "FREQ=DAILY;COUNT=2")
    let starts = rule.instances(anchor: local(2026, 10, 31, 1, 30), timeZone: ny, isAllDay: false, before: local(2027, 1, 1), limit: 10).starts
    #expect(starts[1] == Date(timeIntervalSince1970: 1_793_511_000))   // 2026-11-01 05:30Z = 01:30 EDT, not EST
}

@Test func allDayRulesProduceCanonicalMidnights() throws {
    let rule = try RecurrenceRule(rrule: "FREQ=WEEKLY;COUNT=3")
    let anchor = AllDay.startOfDay(CalendarDate(year: 2026, month: 3, day: 2), in: ny)!
    let starts = rule.instances(anchor: anchor, timeZone: ny, isAllDay: true, before: local(2027, 1, 1), limit: 10).starts
    #expect(starts == [2, 9, 16].map { AllDay.startOfDay(CalendarDate(year: 2026, month: 3, day: $0), in: ny)! })
}

@Test func leapDayYearlyOnlyInLeapYears() throws {
    #expect(days(try expand("FREQ=YEARLY;COUNT=3", local(2024, 2, 29))) == ["2024-02-29", "2028-02-29", "2032-02-29"])
}

@Test func limitTruncates() throws {
    let rule = try RecurrenceRule(rrule: "FREQ=DAILY")
    let result = rule.instances(anchor: local(2026, 1, 1), timeZone: ny, isAllDay: false, before: local(2027, 1, 1), limit: 5)
    #expect(result.starts.count == 5)
    #expect(result.truncated)
}

@Test func anchorAtOrAfterBoundGivesNothing() throws {
    let rule = try RecurrenceRule(rrule: "FREQ=DAILY")
    #expect(rule.instances(anchor: local(2026, 1, 1), timeZone: ny, isAllDay: false, before: local(2026, 1, 1), limit: 5).starts.isEmpty)
}

@Test func skipToJumpsWholePeriodsWithoutChangingResults() throws {
    let rule = try RecurrenceRule(rrule: "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,TH")
    let anchor = local(2015, 1, 5)
    let cut = local(2026, 9, 1)
    let full = rule.instances(anchor: anchor, timeZone: ny, isAllDay: false, before: local(2026, 10, 1), limit: 100_000).starts
    let skipped = rule.instances(anchor: anchor, timeZone: ny, isAllDay: false, before: local(2026, 10, 1), limit: 100_000, skipTo: cut).starts
    #expect(skipped.first == anchor)
    #expect(skipped.filter { $0 >= cut } == full.filter { $0 >= cut })
    #expect(skipped.count < 20)   // the years before the cut were not generated
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `swift test --package-path Packages/CalendarConnectors --filter RuleExpander`
Expected: compile error, `instances(anchor:...)` not defined.

- [ ] **Step 3: Implement the expander**

Create `RuleExpander.swift`:

```swift
import Foundation

/// The starts a rule generated, and whether a limit stopped it early.
public struct RuleExpansion: Sendable, Equatable {
    public var starts: [Date]
    /// True when `limit` or the iteration cap stopped the expansion before the rule's end or the bound.
    public var truncated: Bool
    public init(starts: [Date], truncated: Bool) {
        self.starts = starts
        self.truncated = truncated
    }
}

extension RecurrenceRule {
    /// The starts this rule generates from `anchor` (iCalendar `DTSTART`), in order. `anchor` is always the first start
    /// (RFC 5545 counts it as the first instance even when the rule would not generate it); then every generated start
    /// after it, until the rule's `COUNT` (which includes the anchor) or `UNTIL` (inclusive), the first start at or
    /// after `bound`, or `limit` starts. Expansion is on wall-clock time in `timeZone`, so a 09:00 rule stays at 09:00
    /// across daylight saving; a local time that does not exist moves forward by the gap and a repeated one uses its
    /// first occurrence. An all-day rule gives canonical all-day starts (`AllDay.startOfDay`).
    ///
    /// `skipTo` lets a caller that only needs starts from some instant on skip whole periods before it without
    /// generating them (a daily series from years ago stays cheap). It is ignored when the rule has a `COUNT`, which
    /// needs every earlier instance. The anchor is still returned first.
    ///
    /// Limitation: `BYWEEKNO` only matches days inside the period's own year (week 1 days in the previous December
    /// are not generated).
    public func instances(
        anchor: Date, timeZone: TimeZone, isAllDay: Bool, before bound: Date, limit: Int, skipTo: Date? = nil
    ) -> RuleExpansion {
        RuleExpander(rule: self, anchor: anchor, zone: timeZone, isAllDay: isAllDay).run(bound: bound, limit: max(1, limit), skipTo: skipTo)
    }
}

struct RuleExpander {
    static let iterationCap = 200_000

    let rule: RecurrenceRule
    let anchor: Date
    let zone: TimeZone
    let isAllDay: Bool
    let calendar: Calendar
    let anchorDate: CalendarDate
    let anchorHour: Int, anchorMinute: Int, anchorSecond: Int

    // BY-parts after RFC 5545 defaults (see `init`).
    let byMonth: Set<Int>
    let byMonthDay: [Int]
    let byWeekday: [RecurrenceRule.WeekdayOccurrence]
    let byYearDay: [Int]
    let byWeekNo: [Int]
    let byHour: [Int], byMinute: [Int], bySecond: [Int]

    init(rule: RecurrenceRule, anchor: Date, zone: TimeZone, isAllDay: Bool) {
        self.rule = rule
        self.anchor = anchor
        self.zone = zone
        self.isAllDay = isAllDay
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = zone
        self.calendar = calendar
        let c = calendar.dateComponents([.year, .month, .day, .hour, .minute, .second], from: anchor)
        anchorDate = CalendarDate(year: c.year!, month: c.month!, day: c.day!)
        anchorHour = isAllDay ? 0 : c.hour!
        anchorMinute = isAllDay ? 0 : c.minute!
        anchorSecond = isAllDay ? 0 : c.second!

        var months = Set(rule.months)
        var monthDays = rule.monthDays
        var weekdays = rule.weekdays
        // RFC 5545: parts the rule leaves out come from DTSTART (the same defaults as python-dateutil).
        if rule.weekNumbers.isEmpty && rule.yearDays.isEmpty && rule.monthDays.isEmpty && rule.weekdays.isEmpty {
            switch rule.frequency {
            case .yearly:
                if months.isEmpty { months = [anchorDate.month] }
                monthDays = [anchorDate.day]
            case .monthly:
                monthDays = [anchorDate.day]
            case .weekly:
                weekdays = [RecurrenceRule.WeekdayOccurrence(Self.weekday(of: anchorDate))]
            default:
                break
            }
        }
        byMonth = months
        byMonthDay = monthDays
        byWeekday = weekdays
        byYearDay = rule.yearDays
        byWeekNo = rule.weekNumbers
        byHour = rule.hours
        byMinute = rule.minutes
        bySecond = rule.seconds
    }

    // MARK: Driver

    func run(bound: Date, limit: Int, skipTo: Date?) -> RuleExpansion {
        guard anchor < bound else { return RuleExpansion(starts: [], truncated: false) }
        var starts = [anchor]
        var produced = 1
        let maxCount: Int? = { if case .count(let n) = rule.end { return n } else { return nil } }()
        let until: Date? = { if case .until(let d) = rule.end { return d } else { return nil } }()
        if let maxCount, produced >= maxCount { return RuleExpansion(starts: starts, truncated: false) }
        if starts.count >= limit { return RuleExpansion(starts: starts, truncated: true) }

        var index = maxCount == nil ? firstIndex(skipTo: skipTo) : 0
        var iterations = 0
        while true {
            iterations += 1
            if iterations > Self.iterationCap { return RuleExpansion(starts: starts, truncated: true) }
            guard let period = period(at: index) else { break }
            if period.start >= bound { break }
            if let until, period.start > until { break }
            for candidate in candidates(in: period) {
                guard candidate > anchor else { continue }
                if let until, candidate > until { return RuleExpansion(starts: starts, truncated: false) }
                if candidate >= bound { return RuleExpansion(starts: starts, truncated: false) }
                starts.append(candidate)
                produced += 1
                if let maxCount, produced >= maxCount { return RuleExpansion(starts: starts, truncated: false) }
                if starts.count >= limit { return RuleExpansion(starts: starts, truncated: true) }
            }
            index += rule.interval
        }
        return RuleExpansion(starts: starts, truncated: false)
    }

    // MARK: Periods

    /// One period of the rule: the days it covers (daily and longer) or its first instant (sub-daily).
    struct Period {
        var start: Date
        var days: [CalendarDate]
    }

    /// The period `index` periods after the anchor's (a multiple of `interval`).
    func period(at index: Int) -> Period? {
        switch rule.frequency {
        case .yearly:
            let year = anchorDate.year + index
            let first = CalendarDate(year: year, month: 1, day: 1)
            return Period(start: startInstant(first), days: (0..<Self.daysInYear(year)).map { first.adding(days: $0) })
        case .monthly:
            let total = anchorDate.month - 1 + index
            let year = anchorDate.year + Int(floor(Double(total) / 12)), month = ((total % 12) + 12) % 12 + 1
            let first = CalendarDate(year: year, month: month, day: 1)
            return Period(start: startInstant(first), days: (0..<Self.daysInMonth(year, month)).map { first.adding(days: $0) })
        case .weekly:
            let first = weekStart(of: anchorDate).adding(days: 7 * index)
            return Period(start: startInstant(first), days: (0..<7).map { first.adding(days: $0) })
        case .daily:
            let day = anchorDate.adding(days: index)
            return Period(start: startInstant(day), days: [day])
        case .hourly, .minutely, .secondly:
            let unit: TimeInterval = rule.frequency == .hourly ? 3600 : rule.frequency == .minutely ? 60 : 1
            let floored = floorToUnit(anchor, unit: unit)
            return Period(start: floored.addingTimeInterval(unit * Double(index)), days: [])
        }
    }

    /// With no COUNT, the first period worth generating for `skipTo`: a multiple of `interval`, one interval early for safety.
    func firstIndex(skipTo: Date?) -> Int {
        guard let skipTo, skipTo > anchor else { return 0 }
        let target = CalendarDate(skipTo, calendar: calendar)
        let raw: Int
        switch rule.frequency {
        case .yearly: raw = target.year - anchorDate.year
        case .monthly: raw = (target.year - anchorDate.year) * 12 + (target.month - anchorDate.month)
        case .weekly: raw = Self.daysBetween(weekStart(of: anchorDate), weekStart(of: target)) / 7
        case .daily: raw = Self.daysBetween(anchorDate, target)
        case .hourly: raw = Int(skipTo.timeIntervalSince(anchor) / 3600)
        case .minutely: raw = Int(skipTo.timeIntervalSince(anchor) / 60)
        case .secondly: raw = Int(skipTo.timeIntervalSince(anchor))
        }
        return max(0, (raw / rule.interval - 1) * rule.interval)
    }

    // MARK: Candidates

    func candidates(in period: Period) -> [Date] {
        var list: [Date]
        switch rule.frequency {
        case .hourly, .minutely, .secondly:
            list = subDailyCandidates(periodStart: period.start)
        default:
            let days = period.days.filter(dayMatches)
            list = []
            for day in days {
                if isAllDay {
                    if let start = AllDay.startOfDay(day, in: zone) { list.append(start) }
                    continue
                }
                for h in (byHour.isEmpty ? [anchorHour] : byHour.sorted()) {
                    for m in (byMinute.isEmpty ? [anchorMinute] : byMinute.sorted()) {
                        for s in (bySecond.isEmpty ? [anchorSecond] : bySecond.sorted()) {
                            if let date = localDate(day, h, m, s) { list.append(date) }
                        }
                    }
                }
            }
        }
        list = Array(Set(list)).sorted()
        guard !rule.setPositions.isEmpty, !list.isEmpty else { return list }
        var picked: [Date] = []
        for position in rule.setPositions {
            let i = position > 0 ? position - 1 : list.count + position
            if list.indices.contains(i) { picked.append(list[i]) }
        }
        return Array(Set(picked)).sorted()
    }

    private func subDailyCandidates(periodStart: Date) -> [Date] {
        let unit: TimeInterval = rule.frequency == .hourly ? 3600 : rule.frequency == .minutely ? 60 : 1
        var offsets: [TimeInterval] = []
        switch rule.frequency {
        case .hourly:
            for m in (byMinute.isEmpty ? [anchorMinute] : byMinute.sorted()) {
                for s in (bySecond.isEmpty ? [anchorSecond] : bySecond.sorted()) { offsets.append(Double(m * 60 + s)) }
            }
        case .minutely:
            offsets = (bySecond.isEmpty ? [anchorSecond] : bySecond.sorted()).map(Double.init)
        default:
            offsets = [0]
        }
        return offsets.compactMap { offset -> Date? in
            guard offset < unit else { return nil }
            let date = periodStart.addingTimeInterval(offset)
            let c = calendar.dateComponents([.year, .month, .day, .hour, .minute, .second], from: date)
            let day = CalendarDate(year: c.year!, month: c.month!, day: c.day!)
            guard dayMatches(day) else { return nil }
            if !byHour.isEmpty && !byHour.contains(c.hour!) { return nil }
            if rule.frequency != .hourly && !byMinute.isEmpty && !byMinute.contains(c.minute!) { return nil }
            if rule.frequency == .secondly && !bySecond.isEmpty && !bySecond.contains(c.second!) { return nil }
            return date
        }
    }

    func dayMatches(_ day: CalendarDate) -> Bool {
        if !byMonth.isEmpty && !byMonth.contains(day.month) { return false }
        if !byWeekNo.isEmpty {
            guard let (week, weeksInYear) = weekNumber(of: day) else { return false }
            if !byWeekNo.contains(where: { $0 == week || $0 == week - weeksInYear - 1 }) { return false }
        }
        if !byYearDay.isEmpty {
            let n = Self.daysBetween(CalendarDate(year: day.year, month: 1, day: 1), day) + 1
            let total = Self.daysInYear(day.year)
            if !byYearDay.contains(where: { $0 == n || $0 == n - total - 1 }) { return false }
        }
        if !byMonthDay.isEmpty {
            let total = Self.daysInMonth(day.year, day.month)
            if !byMonthDay.contains(where: { $0 == day.day || $0 == day.day - total - 1 }) { return false }
        }
        if !byWeekday.isEmpty && !byWeekday.contains(where: { weekdayMatches($0, day) }) { return false }
        return true
    }

    /// A plain weekday matches any such day; an ordinal counts within the month (monthly rules, and yearly rules with
    /// `BYMONTH`) or within the year (other yearly rules). Other frequencies ignore the ordinal.
    private func weekdayMatches(_ occurrence: RecurrenceRule.WeekdayOccurrence, _ day: CalendarDate) -> Bool {
        guard Self.weekday(of: day) == occurrence.weekday else { return false }
        guard let ordinal = occurrence.ordinal else { return true }
        let inMonth = rule.frequency == .monthly || (rule.frequency == .yearly && !byMonth.isEmpty)
        guard inMonth || rule.frequency == .yearly else { return true }
        let first = inMonth ? CalendarDate(year: day.year, month: day.month, day: 1) : CalendarDate(year: day.year, month: 1, day: 1)
        let length = inMonth ? Self.daysInMonth(day.year, day.month) : Self.daysInYear(day.year)
        let offset = Self.daysBetween(first, day)
        let fromStart = offset / 7 + 1
        let fromEnd = -((length - 1 - offset) / 7 + 1)
        return ordinal == fromStart || ordinal == fromEnd
    }

    // MARK: Weeks

    /// The day on or before `day` that is the rule's week start.
    func weekStart(of day: CalendarDate) -> CalendarDate {
        let order = RecurrenceRule.Weekday.mondayFirst
        let back = (order.firstIndex(of: Self.weekday(of: day))! - order.firstIndex(of: rule.weekStart)! + 7) % 7
        return day.adding(days: -back)
    }

    /// Week 1 is the first week (starting on `WKST`) with at least four days in the year. Returns the week number and
    /// the number of weeks in that year, or nil for a day that belongs to the neighbouring year's weeks.
    func weekNumber(of day: CalendarDate) -> (Int, Int)? {
        let firstWeek = weekStart(of: CalendarDate(year: day.year, month: 1, day: 4))
        let nextFirstWeek = weekStart(of: CalendarDate(year: day.year + 1, month: 1, day: 4))
        guard day >= firstWeek, day < nextFirstWeek else { return nil }
        return (Self.daysBetween(firstWeek, day) / 7 + 1, Self.daysBetween(firstWeek, nextFirstWeek) / 7)
    }

    // MARK: Date helpers

    private func localDate(_ day: CalendarDate, _ h: Int, _ m: Int, _ s: Int) -> Date? {
        WallClock.date(day, hour: h, minute: m, second: s, in: zone)
    }

    private func startInstant(_ day: CalendarDate) -> Date {
        AllDay.startOfDay(day, in: zone) ?? anchor
    }

    private func floorToUnit(_ date: Date, unit: TimeInterval) -> Date {
        let c = calendar.dateComponents([.year, .month, .day, .hour, .minute, .second], from: date)
        let parts: DateComponents
        switch unit {
        case 3600: parts = DateComponents(year: c.year, month: c.month, day: c.day, hour: c.hour)
        case 60: parts = DateComponents(year: c.year, month: c.month, day: c.day, hour: c.hour, minute: c.minute)
        default: parts = c
        }
        return calendar.date(from: parts) ?? date
    }

    static func weekday(of day: CalendarDate) -> RecurrenceRule.Weekday {
        var utc = Calendar(identifier: .gregorian)
        utc.timeZone = TimeZone(identifier: "UTC")!
        let noon = utc.date(from: DateComponents(year: day.year, month: day.month, day: day.day, hour: 12))!
        // Calendar weekday: 1 = Sunday ... 7 = Saturday.
        return [.sunday, .monday, .tuesday, .wednesday, .thursday, .friday, .saturday][utc.component(.weekday, from: noon) - 1]
    }

    static func daysBetween(_ a: CalendarDate, _ b: CalendarDate) -> Int {
        var utc = Calendar(identifier: .gregorian)
        utc.timeZone = TimeZone(identifier: "UTC")!
        let da = utc.date(from: DateComponents(year: a.year, month: a.month, day: a.day, hour: 12))!
        let db = utc.date(from: DateComponents(year: b.year, month: b.month, day: b.day, hour: 12))!
        return Int((db.timeIntervalSince(da) / 86_400).rounded())
    }

    static func daysInMonth(_ year: Int, _ month: Int) -> Int {
        switch month {
        case 2: return isLeap(year) ? 29 : 28
        case 4, 6, 9, 11: return 30
        default: return 31
        }
    }

    static func daysInYear(_ year: Int) -> Int { isLeap(year) ? 366 : 365 }
    static func isLeap(_ year: Int) -> Bool { (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 }
}

/// Wall-clock times in a zone, resolved the way iCalendar needs them: a time skipped by a daylight-saving jump moves
/// forward by the gap, and a time that happens twice uses the first. Computed from offsets rather than `Calendar`
/// matching policies so it behaves the same on swift-corelibs-foundation.
public enum WallClock {
    public static func date(_ day: CalendarDate, hour: Int, minute: Int, second: Int, in zone: TimeZone) -> Date? {
        var utc = Calendar(identifier: .gregorian)
        utc.timeZone = TimeZone(identifier: "UTC")!
        guard let wall = utc.date(from: DateComponents(year: day.year, month: day.month, day: day.day, hour: hour, minute: minute, second: second)),
              let noon = utc.date(from: DateComponents(year: day.year, month: day.month, day: day.day, hour: 12)) else { return nil }
        let before = TimeInterval(zone.secondsFromGMT(for: noon.addingTimeInterval(-86_400)))
        let after = TimeInterval(zone.secondsFromGMT(for: noon.addingTimeInterval(86_400)))
        let early = wall.addingTimeInterval(-before)   // read with the offset in force before a transition that day
        let late = wall.addingTimeInterval(-after)     // and with the offset after it
        let earlyOK = TimeInterval(zone.secondsFromGMT(for: early)) == before
        let lateOK = TimeInterval(zone.secondsFromGMT(for: late)) == after
        switch (earlyOK, lateOK) {
        case (true, true): return min(early, late)     // equal on an ordinary day; the first of a repeated time
        case (true, false): return early
        case (false, true): return late
        case (false, false): return early              // a skipped time, moved forward by the gap
        }
    }
}

extension RecurrenceRule.Weekday {
    static let mondayFirst: [RecurrenceRule.Weekday] = [.monday, .tuesday, .wednesday, .thursday, .friday, .saturday, .sunday]
}

extension CalendarDate {
    init(_ date: Date, calendar: Calendar) {
        let c = calendar.dateComponents([.year, .month, .day], from: date)
        self.init(year: c.year!, month: c.month!, day: c.day!)
    }
}
```

The three daylight-saving tests also run in `core-linux` (Task 17 Step 4 runs them in the Linux image), which is why `WallClock` avoids `Calendar.nextDate` matching policies.

- [ ] **Step 4: Run the tests**

Run: `swift test --package-path Packages/CalendarConnectors --filter RuleExpander`
Expected: PASS. If an RFC example fails, compare against RFC 5545 section 3.8.5.3 before changing the test: the lists above are copied from it.

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors/Sources/CalendarCore/Recurrence/RuleExpander.swift Packages/CalendarConnectors/Tests/CalendarCoreTests/RuleExpanderTests.swift
git commit -m "CalendarCore: expand RFC 5545 recurrence rules on wall-clock time"
```

---

### Task 3: Set expansion (`RecurrenceSet.occurrences`) and split counting

**Files:**
- Create: `Packages/CalendarConnectors/Sources/CalendarCore/Recurrence/RecurrenceSet+Occurrences.swift`
- Modify: `Packages/CalendarConnectors/Sources/CalendarCore/Recurrence/RecurrenceSet.swift` (doc comment)
- Test: `Packages/CalendarConnectors/Tests/CalendarCoreTests/RecurrenceSetOccurrencesTests.swift`

**Interfaces:**
- Consumes: `RecurrenceRule.instances(...)` (Task 2).
- Produces:
  ```swift
  public struct RecurrenceExpansion: Sendable, Equatable {
      public var starts: [Date]          // original starts overlapping the window, sorted, deduplicated
      public var truncated: Bool
      public var hasUnreadableRule: Bool // an unparsed RRULE: callers show only the anchor
  }
  extension RecurrenceSet {
      public func occurrences(anchor: Date, duration: TimeInterval, timeZone: TimeZone, isAllDay: Bool,
                              overlapping window: DateInterval, limit: Int = 5000) -> RecurrenceExpansion
      public func ruleInstanceCount(anchor: Date, timeZone: TimeZone, isAllDay: Bool, before date: Date) -> Int
  }
  ```

- [ ] **Step 1: Write the failing tests**

```swift
import CalendarCore
import Foundation
import Testing

private let ny = TimeZone(identifier: "America/New_York")!
private func local(_ y: Int, _ mo: Int, _ d: Int, _ h: Int = 9, _ mi: Int = 0) -> Date {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = ny
    return calendar.date(from: DateComponents(year: y, month: mo, day: d, hour: h, minute: mi))!
}

@Test func windowKeepsOnlyOverlappingInstances() throws {
    let set = RecurrenceSet(rules: [try RecurrenceRule(rrule: "FREQ=DAILY")])
    let result = set.occurrences(anchor: local(2026, 9, 1), duration: 3600, timeZone: ny, isAllDay: false,
                                 overlapping: DateInterval(start: local(2026, 9, 10, 9, 30), end: local(2026, 9, 12, 9)))
    // 10th 09:00-10:00 overlaps (started before the window), 11th inside, 12th starts at the end (excluded).
    #expect(result.starts == [local(2026, 9, 10), local(2026, 9, 11)])
    #expect(!result.truncated)
}

@Test func exdatesRemoveAndRdatesAdd() throws {
    let set = RecurrenceSet(rules: [try RecurrenceRule(rrule: "FREQ=DAILY;COUNT=5")],
                            extraDates: [local(2026, 9, 20)], excludedDates: [local(2026, 9, 3)])
    let result = set.occurrences(anchor: local(2026, 9, 1), duration: 1800, timeZone: ny, isAllDay: false,
                                 overlapping: DateInterval(start: local(2026, 9, 1, 0), end: local(2026, 10, 1)))
    #expect(result.starts == [local(2026, 9, 1), local(2026, 9, 2), local(2026, 9, 4), local(2026, 9, 5), local(2026, 9, 20)])
}

@Test func allDayExdateMatchesByDay() throws {
    let day = { (d: Int) in AllDay.startOfDay(CalendarDate(year: 2026, month: 9, day: d), in: ny)! }
    let set = RecurrenceSet(rules: [try RecurrenceRule(rrule: "FREQ=DAILY;COUNT=3")], excludedDates: [day(2)])
    let result = set.occurrences(anchor: day(1), duration: 86_400, timeZone: ny, isAllDay: true,
                                 overlapping: DateInterval(start: day(1), end: day(10)))
    #expect(result.starts == [day(1), day(3)])
}

@Test func oldDailySeriesStillShowsThisWeek() throws {
    let set = RecurrenceSet(rules: [try RecurrenceRule(rrule: "FREQ=DAILY")])
    let result = set.occurrences(anchor: local(2015, 1, 1), duration: 1800, timeZone: ny, isAllDay: false,
                                 overlapping: DateInterval(start: local(2026, 9, 27, 0), end: local(2026, 10, 4, 0)))
    #expect(result.starts.count == 7)
    #expect(result.starts.first == local(2026, 9, 27))
    #expect(!result.truncated)
}

@Test func oldWeeklySeriesWithIntervalStillShowsThisMonth() throws {
    let set = RecurrenceSet(rules: [try RecurrenceRule(rrule: "FREQ=WEEKLY;INTERVAL=2;BYDAY=TU")])
    let anchor = local(2015, 1, 6)   // a Tuesday
    let result = set.occurrences(anchor: anchor, duration: 1800, timeZone: ny, isAllDay: false,
                                 overlapping: DateInterval(start: local(2026, 9, 1, 0), end: local(2026, 10, 1, 0)))
    // Every second Tuesday from 2015-01-06: 2026-09-01 is 608 weeks later (even), so 09-01, 09-15 and 09-29.
    #expect(result.starts == [local(2026, 9, 1), local(2026, 9, 15), local(2026, 9, 29)])
}

@Test func largeCountSeriesStillShowsTheWindow() throws {
    // COUNT cannot skip ahead, so the years of instances before the window must not use up the limit.
    let set = RecurrenceSet(rules: [try RecurrenceRule(rrule: "FREQ=DAILY;COUNT=10000")])
    let result = set.occurrences(anchor: local(2015, 1, 1), duration: 1800, timeZone: ny, isAllDay: false,
                                 overlapping: DateInterval(start: local(2026, 9, 27, 0), end: local(2026, 10, 4, 0)))
    #expect(result.starts.count == 7)
    #expect(!result.truncated)
}

@Test func unreadableRuleIsReported() {
    let set = RecurrenceSet(iCalendarLines: ["RRULE:FREQ=FORTNIGHTLY"], timeZone: ny, isAllDay: false)
    let result = set.occurrences(anchor: local(2026, 9, 1), duration: 1800, timeZone: ny, isAllDay: false,
                                 overlapping: DateInterval(start: local(2026, 9, 1, 0), end: local(2026, 10, 1)))
    #expect(result.hasUnreadableRule)
    #expect(result.starts == [local(2026, 9, 1)])
}

@Test func limitIsReported() throws {
    let set = RecurrenceSet(rules: [try RecurrenceRule(rrule: "FREQ=MINUTELY")])
    let result = set.occurrences(anchor: local(2026, 9, 1), duration: 60, timeZone: ny, isAllDay: false,
                                 overlapping: DateInterval(start: local(2026, 9, 1), end: local(2026, 9, 30)), limit: 100)
    #expect(result.starts.count == 100)
    #expect(result.truncated)
}

@Test func ruleInstanceCountIncludesAnchorAndExcludedDates() throws {
    let set = RecurrenceSet(rules: [try RecurrenceRule(rrule: "FREQ=WEEKLY;COUNT=10")], excludedDates: [local(2026, 9, 8)])
    // Instances before 09-22: 09-01, 09-08 (excluded but counted, as RFC 5545 COUNT does), 09-15.
    #expect(set.ruleInstanceCount(anchor: local(2026, 9, 1), timeZone: ny, isAllDay: false, before: local(2026, 9, 22)) == 3)
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `swift test --package-path Packages/CalendarConnectors --filter RecurrenceSetOccurrences`
Expected: compile error, `occurrences(anchor:...)` not defined.

- [ ] **Step 3: Implement**

```swift
import Foundation

/// The occurrences of a series that overlap a window.
public struct RecurrenceExpansion: Sendable, Equatable {
    /// Original starts (the instants the rules put the occurrences at), sorted and without duplicates.
    public var starts: [Date]
    /// True when the limit stopped the expansion early.
    public var truncated: Bool
    /// True when an `RRULE` line could not be read (it sits in `unparsed`): the starts then hold only the anchor, and a
    /// caller should show just the first occurrence and the series' exceptions.
    public var hasUnreadableRule: Bool
    public init(starts: [Date], truncated: Bool, hasUnreadableRule: Bool) {
        self.starts = starts
        self.truncated = truncated
        self.hasUnreadableRule = hasUnreadableRule
    }
}

extension RecurrenceSet {
    /// Every occurrence whose span (`start ..< start + duration`) overlaps `window`: the rules' instances from `anchor`
    /// plus `extraDates`, minus `excludedDates` (all-day series compare calendar days in `timeZone`). At most `limit`
    /// starts. Expansion skips whole periods before the window, so an old series stays cheap.
    public func occurrences(
        anchor: Date, duration: TimeInterval, timeZone: TimeZone, isAllDay: Bool, overlapping window: DateInterval,
        limit: Int = 5000
    ) -> RecurrenceExpansion {
        let unreadable = unparsed.contains { $0.trimmingCharacters(in: .whitespaces).uppercased().hasPrefix("RRULE") }
        let span = max(duration, 0)
        func overlaps(_ start: Date) -> Bool {
            span == 0 ? (start >= window.start && start < window.end) : (start < window.end && start.addingTimeInterval(span) > window.start)
        }
        if unreadable {
            return RecurrenceExpansion(starts: overlaps(anchor) ? [anchor] : [], truncated: false, hasUnreadableRule: true)
        }
        var candidates: [Date] = [anchor]
        var truncated = false
        let skipTo = window.start.addingTimeInterval(-span)
        for rule in rules {
            // A COUNT rule cannot skip, so its instances before the window must not use up the limit; COUNT itself
            // bounds it (and the iteration cap guards the rest). Otherwise skipping leaves only a few early instances.
            let ruleLimit: Int = { if case .count = rule.end { return Int.max / 2 } else { return limit + 64 } }()
            let expansion = rule.instances(anchor: anchor, timeZone: timeZone, isAllDay: isAllDay, before: window.end,
                                           limit: ruleLimit, skipTo: skipTo)
            candidates += expansion.starts
            truncated = truncated || expansion.truncated
        }
        candidates += extraDates ?? []
        let excluded = excludedDates ?? []
        func key(_ date: Date) -> String {
            isAllDay ? "\(AllDay.date(of: date, in: timeZone))" : "\(date.timeIntervalSince1970)"
        }
        let excludedKeys = Set(excluded.map(key))
        var seen = Set<String>()
        var result: [Date] = []
        for start in candidates.sorted() where overlaps(start) && !excludedKeys.contains(key(start)) {
            if seen.insert(key(start)).inserted { result.append(start) }
        }
        if result.count > limit {
            result = Array(result.prefix(limit))
            truncated = true
        }
        return RecurrenceExpansion(starts: result, truncated: truncated, hasUnreadableRule: false)
    }

    /// How many instances the first rule generates strictly before `date`, the anchor included and excluded dates
    /// counted (RFC 5545 `COUNT` counts generated instances before `EXDATE` removes any). Used to split a `COUNT` rule.
    public func ruleInstanceCount(anchor: Date, timeZone: TimeZone, isAllDay: Bool, before date: Date) -> Int {
        guard let rule = rules.first else { return anchor < date ? 1 : 0 }
        return rule.instances(anchor: anchor, timeZone: timeZone, isAllDay: isAllDay, before: date, limit: Int.max / 2).starts.count
    }
}
```

In `RecurrenceSet.swift` replace the type's doc comment with:

```swift
/// Everything that makes up a series' repetition: its rules and the extra and skipped dates. Read from a provider's
/// iCalendar lines (`RRULE`, `EXDATE`, `RDATE`) and written back unchanged. Connectors whose server expands occurrences
/// (Google, Microsoft, EventKit) never expand; a connector that reads raw iCalendar (CalDAV) expands with
/// `occurrences(anchor:duration:timeZone:isAllDay:overlapping:limit:)`.
```

- [ ] **Step 4: Run the tests**

Run: `swift test --package-path Packages/CalendarConnectors --filter "RecurrenceSet|RuleExpander"`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors/Sources/CalendarCore/Recurrence Packages/CalendarConnectors/Tests/CalendarCoreTests/RecurrenceSetOccurrencesTests.swift
git commit -m "CalendarCore: expand recurrence sets over a window, count instances for splits"
```

---
## Part B: ICalendar

### Task 4: Content lines and the component tree

**Files:**
- Modify: `Packages/CalendarConnectors/Package.swift`
- Create: `Packages/CalendarConnectors/Sources/ICalendar/ICalText.swift`, `Component.swift`, `ContentLine.swift`
- Test: `Packages/CalendarConnectors/Tests/ICalendarTests/ContentLineTests.swift`

**Interfaces:**
- Produces:
  ```swift
  public enum ICalError: Error, Equatable, Sendable { case malformed(String) }
  public enum ICalText { public static func escape(_: String) -> String; public static func unescape(_: String) -> String }
  public struct ICalParameter: Hashable, Sendable { public var name: String; public var values: [String]
      public init(name: String, values: [String]); public init(_ name: String, _ value: String) }
  public struct ICalProperty: Hashable, Sendable { public var name: String; public var parameters: [ICalParameter]; public var value: String
      public init(name: String, parameters: [ICalParameter] = [], value: String)
      public init(name: String, text: String, parameters: [ICalParameter] = [])
      public var text: String; public func parameter(_ name: String) -> String?; public mutating func setParameter(_ name: String, _ value: String?) }
  public struct ICalComponent: Hashable, Sendable { public var name: String; public var properties: [ICalProperty]; public var components: [ICalComponent]
      public init(name: String, properties: [ICalProperty] = [], components: [ICalComponent] = [])
      public func property(_ name: String) -> ICalProperty?; public func properties(named: String) -> [ICalProperty]
      public func components(named: String) -> [ICalComponent]; public mutating func set(_ property: ICalProperty)
      public mutating func setText(_ name: String, _ text: String?); public mutating func removeProperties(named: String)
      public mutating func append(_ property: ICalProperty) }
  public enum ICalParser { public static func parse(_ data: Data) throws -> ICalComponent; public static func parse(_ text: String) throws -> ICalComponent }
  public enum ICalSerializer { public static func serialize(_ component: ICalComponent) -> String; public static func contentLine(_ property: ICalProperty) -> String }
  ```

- [ ] **Step 1: Add the target to the package**

In `Package.swift` add the product `.library(name: "ICalendar", targets: ["ICalendar"]),` after `MicrosoftCalendar`, the target `.target(name: "ICalendar", dependencies: ["CalendarCore"]),` and the test target `.testTarget(name: "ICalendarTests", dependencies: ["ICalendar", "CalendarCore", "CalendarTestSupport"]),`.

- [ ] **Step 2: Write the failing tests**

```swift
import CalendarCore
import Foundation
import Testing
@testable import ICalendar

private let sample = """
BEGIN:VCALENDAR\r
VERSION:2.0\r
PRODID:-//Example//EN\r
BEGIN:VEVENT\r
UID:abc-123\r
SUMMARY:Planning\\, part 2\\nwith notes\r
ATTENDEE;CN="Doe, Jane";ROLE=REQ-PARTICIPANT;DELEGATED-FROM="mailto:a@x.test","mailto:b@x.test":mailto:jane@x.test\r
URL:https://x.test:8443/a;b\r
X-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC\r
X-CUSTOM;X-FLAG=1:kept\r
BEGIN:VALARM\r
ACTION:DISPLAY\r
TRIGGER:-PT15M\r
END:VALARM\r
END:VEVENT\r
END:VCALENDAR\r

"""

@Test func parsesNestedComponents() throws {
    let root = try ICalParser.parse(sample)
    #expect(root.name == "VCALENDAR")
    let event = try #require(root.components(named: "VEVENT").first)
    #expect(event.property("UID")?.value == "abc-123")
    #expect(event.components(named: "VALARM").first?.property("TRIGGER")?.value == "-PT15M")
}

@Test func parsesQuotedParametersAndMultipleValues() throws {
    let event = try #require(try ICalParser.parse(sample).components(named: "VEVENT").first)
    let attendee = try #require(event.property("ATTENDEE"))
    #expect(attendee.parameter("CN") == "Doe, Jane")
    #expect(attendee.parameters.first { $0.name == "DELEGATED-FROM" }?.values == ["mailto:a@x.test", "mailto:b@x.test"])
    #expect(attendee.value == "mailto:jane@x.test")
}

@Test func valueMayContainColonsAndSemicolons() throws {
    let event = try #require(try ICalParser.parse(sample).components(named: "VEVENT").first)
    #expect(event.property("URL")?.value == "https://x.test:8443/a;b")
}

@Test func unescapesText() throws {
    let event = try #require(try ICalParser.parse(sample).components(named: "VEVENT").first)
    #expect(event.property("SUMMARY")?.text == "Planning, part 2\nwith notes")
    #expect(ICalText.escape("a;b,c\\d\ne") == "a\\;b\\,c\\\\d\\ne")
    #expect(ICalText.unescape(ICalText.escape("x;y,z\\\n")) == "x;y,z\\\n")
}

@Test func foldsAt75OctetsWithoutSplittingUTF8() throws {
    let long = String(repeating: "é🙂a", count: 40)
    let component = ICalComponent(name: "VEVENT", properties: [ICalProperty(name: "SUMMARY", text: long)])
    let text = ICalSerializer.serialize(component)
    for line in text.components(separatedBy: "\r\n") { #expect(line.utf8.count <= 75) }
    #expect(text.hasSuffix("END:VEVENT\r\n"))
    #expect(try ICalParser.parse(text).property("SUMMARY")?.text == long)
}

@Test func unfoldsAFoldThatSplitsAUTF8Sequence() throws {
    // "é" is C3 A9; a careless server folds between the two bytes.
    var data = Data("BEGIN:VEVENT\r\nSUMMARY:caf".utf8)
    data.append(0xC3)
    data.append(contentsOf: Array("\r\n ".utf8))
    data.append(0xA9)
    data.append(contentsOf: Array("\r\nEND:VEVENT\r\n".utf8))
    #expect(try ICalParser.parse(data).property("SUMMARY")?.text == "café")
}

@Test func parsesLFOnlyAndMissingFinalNewline() throws {
    let text = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\nSUMMARY:long\n  folded\nEND:VEVENT\nEND:VCALENDAR"
    let event = try #require(try ICalParser.parse(text).components(named: "VEVENT").first)
    #expect(event.property("SUMMARY")?.text == "long folded")
}

@Test func roundTripKeepsUnknownPropertiesAndParameters() throws {
    let first = try ICalParser.parse(sample)
    let again = try ICalParser.parse(ICalSerializer.serialize(first))
    #expect(again == first)
    let event = try #require(again.components(named: "VEVENT").first)
    #expect(event.property("X-CUSTOM")?.parameter("X-FLAG") == "1")
    #expect(event.property("X-APPLE-TRAVEL-ADVISORY-BEHAVIOR")?.value == "AUTOMATIC")
}

@Test func rejectsUnbalancedAndUnterminatedInput() {
    #expect(throws: ICalError.self) { try ICalParser.parse("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nEND:VCALENDAR\r\n") }
    #expect(throws: ICalError.self) { try ICalParser.parse("BEGIN:VCALENDAR\r\n") }
    #expect(throws: ICalError.self) { try ICalParser.parse("SUMMARY:outside\r\n") }
}

@Test func setReplacesAtTheFirstPosition() {
    var component = ICalComponent(name: "VEVENT", properties: [
        ICalProperty(name: "UID", value: "1"), ICalProperty(name: "SUMMARY", value: "a"),
        ICalProperty(name: "LOCATION", value: "x"), ICalProperty(name: "SUMMARY", value: "b"),
    ])
    component.set(ICalProperty(name: "summary", value: "c"))
    #expect(component.properties.map(\.name) == ["UID", "SUMMARY", "LOCATION"])
    #expect(component.property("SUMMARY")?.value == "c")
    component.setText("LOCATION", nil)
    #expect(component.property("LOCATION") == nil)
}
```

- [ ] **Step 3: Run to verify they fail**

Run: `swift test --package-path Packages/CalendarConnectors --filter ContentLine`
Expected: compile errors (no `ICalendar` sources yet).

- [ ] **Step 4: Implement**

`ICalText.swift`:

```swift
import Foundation

/// RFC 5545 TEXT escaping (section 3.3.11).
public enum ICalText {
    public static func escape(_ text: String) -> String {
        var out = ""
        for character in text {
            switch character {
            case "\\": out += "\\\\"
            case ";": out += "\\;"
            case ",": out += "\\,"
            case "\n", "\r\n": out += "\\n"
            case "\r": continue
            default: out.append(character)
            }
        }
        return out
    }

    public static func unescape(_ text: String) -> String {
        var out = ""
        var escaping = false
        for character in text {
            if escaping {
                out += (character == "n" || character == "N") ? "\n" : String(character)
                escaping = false
            } else if character == "\\" {
                escaping = true
            } else {
                out.append(character)
            }
        }
        if escaping { out += "\\" }
        return out
    }
}
```

`Component.swift`:

```swift
import Foundation

public struct ICalParameter: Hashable, Sendable {
    public var name: String
    /// Unquoted values; the serializer quotes a value that needs it.
    public var values: [String]
    public init(name: String, values: [String]) {
        self.name = name.uppercased()
        self.values = values
    }
    public init(_ name: String, _ value: String) { self.init(name: name, values: [value]) }
}

/// One content line. `value` is the text after the colon exactly as it appears on the wire (TEXT escapes kept), so a
/// property the library does not model is written back unchanged.
public struct ICalProperty: Hashable, Sendable {
    public var name: String
    public var parameters: [ICalParameter]
    public var value: String

    public init(name: String, parameters: [ICalParameter] = [], value: String) {
        self.name = name.uppercased()
        self.parameters = parameters
        self.value = value
    }

    /// A TEXT property: `text` is escaped for the wire.
    public init(name: String, text: String, parameters: [ICalParameter] = []) {
        self.init(name: name, parameters: parameters, value: ICalText.escape(text))
    }

    /// The value read as TEXT.
    public var text: String { ICalText.unescape(value) }

    /// The first value of the parameter, or nil.
    public func parameter(_ name: String) -> String? {
        let key = name.uppercased()
        return parameters.first { $0.name == key }?.values.first
    }

    /// Sets the parameter to one value, or removes it for nil.
    public mutating func setParameter(_ name: String, _ value: String?) {
        let key = name.uppercased()
        guard let value else { parameters.removeAll { $0.name == key }; return }
        if let index = parameters.firstIndex(where: { $0.name == key }) {
            parameters[index].values = [value]
        } else {
            parameters.append(ICalParameter(key, value))
        }
    }
}

/// A component (`VCALENDAR`, `VEVENT`, `VALARM`, `VTIMEZONE`, ...) with its properties and children, in file order.
public struct ICalComponent: Hashable, Sendable {
    public var name: String
    public var properties: [ICalProperty]
    public var components: [ICalComponent]

    public init(name: String, properties: [ICalProperty] = [], components: [ICalComponent] = []) {
        self.name = name.uppercased()
        self.properties = properties
        self.components = components
    }

    public func property(_ name: String) -> ICalProperty? {
        let key = name.uppercased()
        return properties.first { $0.name == key }
    }

    public func properties(named name: String) -> [ICalProperty] {
        let key = name.uppercased()
        return properties.filter { $0.name == key }
    }

    public func components(named name: String) -> [ICalComponent] {
        let key = name.uppercased()
        return components.filter { $0.name == key }
    }

    /// Replaces every property with this name by `property`, at the position of the first one (or appends it).
    public mutating func set(_ property: ICalProperty) {
        if let first = properties.firstIndex(where: { $0.name == property.name }) {
            properties[first] = property
            var index = properties.index(after: first)
            while index < properties.endIndex {
                if properties[index].name == property.name { properties.remove(at: index) } else { index = properties.index(after: index) }
            }
        } else {
            properties.append(property)
        }
    }

    /// Sets a TEXT property, or removes it for nil or empty text.
    public mutating func setText(_ name: String, _ text: String?) {
        guard let text, !text.isEmpty else { removeProperties(named: name); return }
        set(ICalProperty(name: name, text: text))
    }

    public mutating func removeProperties(named name: String) {
        let key = name.uppercased()
        properties.removeAll { $0.name == key }
    }

    public mutating func append(_ property: ICalProperty) { properties.append(property) }
}
```

`ContentLine.swift`:

```swift
import Foundation

public enum ICalError: Error, Equatable, Sendable {
    case malformed(String)
}

public enum ICalParser {
    /// Unfolds at the byte level first (a server may fold inside a UTF-8 sequence), then parses.
    public static func parse(_ data: Data) throws -> ICalComponent {
        var bytes: [UInt8] = []
        bytes.reserveCapacity(data.count)
        let input = [UInt8](data)
        var i = 0
        while i < input.count {
            // CRLF or LF followed by a space or tab is a fold: drop all three (or two).
            if input[i] == 0x0D, i + 2 < input.count, input[i + 1] == 0x0A, input[i + 2] == 0x20 || input[i + 2] == 0x09 { i += 3; continue }
            if input[i] == 0x0A, i + 1 < input.count, input[i + 1] == 0x20 || input[i + 1] == 0x09 { i += 2; continue }
            bytes.append(input[i])
            i += 1
        }
        return try parse(String(decoding: bytes, as: UTF8.self))
    }

    /// Accepts CRLF, LF or CR line ends, folded lines and a missing final line end.
    public static func parse(_ text: String) throws -> ICalComponent {
        var stack: [ICalComponent] = []
        var root: ICalComponent?
        for line in unfold(text) where !line.trimmingCharacters(in: .whitespaces).isEmpty {
            let property = try parseLine(line)
            switch property.name {
            case "BEGIN":
                guard root == nil else { throw ICalError.malformed("content after the top-level component") }
                stack.append(ICalComponent(name: property.value))
            case "END":
                guard let done = stack.popLast(), done.name == property.value.uppercased() else {
                    throw ICalError.malformed("unbalanced END:\(property.value)")
                }
                if stack.isEmpty { root = done } else { stack[stack.count - 1].components.append(done) }
            default:
                guard !stack.isEmpty else { throw ICalError.malformed("property outside a component: \(property.name)") }
                stack[stack.count - 1].properties.append(property)
            }
        }
        guard stack.isEmpty, let root else { throw ICalError.malformed("unterminated component") }
        return root
    }

    static func unfold(_ text: String) -> [String] {
        var lines: [String] = []
        for raw in text.split(omittingEmptySubsequences: false, whereSeparator: { $0 == "\n" || $0 == "\r\n" || $0 == "\r" }) {
            if let first = raw.first, first == " " || first == "\t", !lines.isEmpty {
                lines[lines.count - 1] += raw.dropFirst()
            } else {
                lines.append(String(raw))
            }
        }
        return lines
    }

    static func parseLine(_ line: String) throws -> ICalProperty {
        let chars = Array(line)
        var i = 0
        func fail() -> ICalError { .malformed("bad content line: \(line.prefix(60))") }
        var name = ""
        while i < chars.count, chars[i] != ";", chars[i] != ":" { name.append(chars[i]); i += 1 }
        guard !name.isEmpty, i < chars.count else { throw fail() }
        var parameters: [ICalParameter] = []
        while chars[i] == ";" {
            i += 1
            var parameterName = ""
            while i < chars.count, chars[i] != "=" { parameterName.append(chars[i]); i += 1 }
            guard i < chars.count, !parameterName.isEmpty else { throw fail() }
            i += 1
            var values: [String] = []
            while true {
                var value = ""
                if i < chars.count, chars[i] == "\"" {
                    i += 1
                    while i < chars.count, chars[i] != "\"" { value.append(chars[i]); i += 1 }
                    guard i < chars.count else { throw fail() }
                    i += 1
                } else {
                    while i < chars.count, chars[i] != ",", chars[i] != ";", chars[i] != ":" { value.append(chars[i]); i += 1 }
                }
                values.append(value)
                guard i < chars.count else { throw fail() }
                if chars[i] == "," { i += 1; continue }
                break
            }
            parameters.append(ICalParameter(name: parameterName, values: values))
        }
        guard chars[i] == ":" else { throw fail() }
        return ICalProperty(name: name, parameters: parameters, value: String(chars[(i + 1)...]))
    }
}

public enum ICalSerializer {
    /// CRLF line ends, lines folded at 75 octets without splitting a UTF-8 sequence.
    public static func serialize(_ component: ICalComponent) -> String {
        var out = ""
        write(component, into: &out)
        return out
    }

    /// One unfolded content line without its line end.
    public static func contentLine(_ property: ICalProperty) -> String {
        var line = property.name
        for parameter in property.parameters {
            line += ";" + parameter.name + "=" + parameter.values.map(quoted).joined(separator: ",")
        }
        return line + ":" + property.value
    }

    private static func write(_ component: ICalComponent, into out: inout String) {
        out += fold("BEGIN:" + component.name)
        for property in component.properties { out += fold(contentLine(property)) }
        for child in component.components { write(child, into: &out) }
        out += fold("END:" + component.name)
    }

    private static func quoted(_ value: String) -> String {
        value.contains(where: { $0 == ";" || $0 == ":" || $0 == "," }) ? "\"" + value + "\"" : value
    }

    private static func fold(_ line: String) -> String {
        var out = ""
        var octets = 0
        for scalar in line.unicodeScalars {
            let size = String(scalar).utf8.count
            if octets + size > 75 {
                out += "\r\n "
                octets = 1
            }
            out.unicodeScalars.append(scalar)
            octets += size
        }
        return out + "\r\n"
    }
}
```

- [ ] **Step 5: Run the tests**

Run: `swift test --package-path Packages/CalendarConnectors --filter ContentLine`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add Packages/CalendarConnectors/Package.swift Packages/CalendarConnectors/Sources/ICalendar Packages/CalendarConnectors/Tests/ICalendarTests
git commit -m "ICalendar: content lines, component tree, escaping and folding"
```

---

### Task 5: Values, time zones in and out

**Files:**
- Create: `Packages/CalendarConnectors/Sources/ICalendar/ICalValues.swift`, `TimeZoneResolver.swift`, `VTimeZoneWriter.swift`
- Test: `Packages/CalendarConnectors/Tests/ICalendarTests/ValuesTests.swift`, `TimeZoneTests.swift`

**Interfaces:**
- Consumes: `ICalProperty`, `ICalComponent` (Task 4); `WallClock`, `WindowsTimeZones`, `AllDay`, `CalendarDate` (CalendarCore).
- Produces:
  ```swift
  public struct LocalDateTime: Hashable, Sendable { public var date: CalendarDate; public var hour: Int; public var minute: Int; public var second: Int }
  public enum ICalDateValue: Hashable, Sendable { case date(CalendarDate); case utc(Date); case local(LocalDateTime, tzid: String?) }
  public enum ICalValues {
      public static func dateValue(_ property: ICalProperty) -> ICalDateValue?
      public static func dateValues(_ property: ICalProperty) -> [ICalDateValue]   // empty when any token is bad or VALUE=PERIOD
      public static func duration(_ text: String) -> TimeInterval?
      public static func durationText(_ seconds: TimeInterval) -> String
      public static func utcText(_ date: Date) -> String          // 20260927T150000Z
      public static func dateText(_ date: CalendarDate) -> String // 20260927
      public static func localText(_ date: Date, in zone: TimeZone) -> String // 20260927T110000
  }
  public struct TimeZoneResolver: Sendable {
      public init(); public init(calendar: ICalComponent)
      public func zone(for tzid: String) -> TimeZone?
      public func date(_ value: ICalDateValue, floating zone: TimeZone) -> Date?
      public func displayZone(_ value: ICalDateValue, fallback: TimeZone) -> TimeZone
  }
  public enum VTimeZoneWriter { public static func component(for zone: TimeZone, from start: Date, through end: Date) -> ICalComponent? }
  ```

- [ ] **Step 1: Write the failing tests**

`ValuesTests.swift`:

```swift
import CalendarCore
import Foundation
import Testing
@testable import ICalendar

@Test func parsesDateUTCZonedAndFloatingValues() {
    #expect(ICalValues.dateValue(ICalProperty(name: "DTSTART", parameters: [ICalParameter("VALUE", "DATE")], value: "20260927"))
        == .date(CalendarDate(year: 2026, month: 9, day: 27)))
    #expect(ICalValues.dateValue(ICalProperty(name: "DTSTART", value: "20260927T150000Z"))
        == .utc(Date(timeIntervalSince1970: 1_790_521_200)))
    #expect(ICalValues.dateValue(ICalProperty(name: "DTSTART", parameters: [ICalParameter("TZID", "Europe/Berlin")], value: "20260927T170000"))
        == .local(LocalDateTime(date: CalendarDate(year: 2026, month: 9, day: 27), hour: 17, minute: 0, second: 0), tzid: "Europe/Berlin"))
    #expect(ICalValues.dateValue(ICalProperty(name: "DTSTART", value: "20260927T170000"))
        == .local(LocalDateTime(date: CalendarDate(year: 2026, month: 9, day: 27), hour: 17, minute: 0, second: 0), tzid: nil))
    // An 8-digit value without VALUE=DATE is still a date (servers omit the parameter).
    #expect(ICalValues.dateValue(ICalProperty(name: "DTSTART", value: "20260927")) == .date(CalendarDate(year: 2026, month: 9, day: 27)))
    #expect(ICalValues.dateValue(ICalProperty(name: "DTSTART", value: "2026-09-27")) == nil)
}

@Test func parsesDateListsAndRejectsPeriods() {
    let list = ICalProperty(name: "EXDATE", parameters: [ICalParameter("TZID", "America/New_York")], value: "20260901T090000,20260908T090000")
    #expect(ICalValues.dateValues(list).count == 2)
    #expect(ICalValues.dateValues(ICalProperty(name: "RDATE", parameters: [ICalParameter("VALUE", "PERIOD")], value: "20260901T090000Z/PT1H")).isEmpty)
}

@Test func parsesAndWritesDurations() {
    #expect(ICalValues.duration("PT15M") == 900)
    #expect(ICalValues.duration("-PT15M") == -900)
    #expect(ICalValues.duration("P1DT2H3M4S") == 93_784)
    #expect(ICalValues.duration("P2W") == 1_209_600)
    #expect(ICalValues.duration("+P1D") == 86_400)
    #expect(ICalValues.duration("PT") == nil)
    #expect(ICalValues.duration("15M") == nil)
    #expect(ICalValues.durationText(-900) == "-PT15M")
    #expect(ICalValues.durationText(93_784) == "P1DT2H3M4S")
    #expect(ICalValues.durationText(0) == "PT0S")
    #expect(ICalValues.durationText(86_400) == "P1D")
}

@Test func formatsTexts() {
    let date = Date(timeIntervalSince1970: 1_790_521_200)   // 2026-09-27 15:00Z
    #expect(ICalValues.utcText(date) == "20260927T150000Z")
    #expect(ICalValues.localText(date, in: TimeZone(identifier: "America/New_York")!) == "20260927T110000")
    #expect(ICalValues.dateText(CalendarDate(year: 2026, month: 1, day: 5)) == "20260105")
}
```

`TimeZoneTests.swift`:

```swift
import CalendarCore
import Foundation
import Testing
@testable import ICalendar

private func instant(_ y: Int, _ m: Int, _ d: Int) -> Date {
    var utc = Calendar(identifier: .gregorian)
    utc.timeZone = TimeZone(identifier: "UTC")!
    return utc.date(from: DateComponents(year: y, month: m, day: d, hour: 12))!
}

private func sameOffsets(_ a: TimeZone?, _ b: TimeZone, year: Int = 2026) -> Bool {
    guard let a else { return false }
    return [instant(year, 1, 15), instant(year, 3, 20), instant(year, 7, 15), instant(year, 11, 15)].allSatisfy {
        a.secondsFromGMT(for: $0) == b.secondsFromGMT(for: $0)
    }
}

@Test func resolvesWindowsMozillaAndCustomZones() throws {
    let resolver = TimeZoneResolver()
    let newYork = TimeZone(identifier: "America/New_York")!
    #expect(sameOffsets(resolver.zone(for: "America/New_York"), newYork))
    #expect(sameOffsets(resolver.zone(for: "Eastern Standard Time"), newYork))
    #expect(sameOffsets(resolver.zone(for: "/mozilla.org/20050126_1/America/New_York"), newYork))
    #expect(resolver.zone(for: "Nowhere/Special") == nil)

    // A custom TZID defined only by its VTIMEZONE (generated here from Berlin's rules, then renamed).
    let berlin = TimeZone(identifier: "Europe/Berlin")!
    var definition = try #require(VTimeZoneWriter.component(for: berlin, from: instant(2026, 1, 1), through: instant(2027, 12, 31)))
    definition.set(ICalProperty(name: "TZID", value: "My Office Zone"))
    let calendar = ICalComponent(name: "VCALENDAR", components: [definition])
    #expect(sameOffsets(TimeZoneResolver(calendar: calendar).zone(for: "My Office Zone"), berlin))
}

@Test func fixedOffsetFallbackForAZoneWithoutDaylightTime() {
    let definition = ICalComponent(name: "VTIMEZONE", properties: [ICalProperty(name: "TZID", value: "Custom +0530")], components: [
        ICalComponent(name: "STANDARD", properties: [
            ICalProperty(name: "DTSTART", value: "19700101T000000"),
            ICalProperty(name: "TZOFFSETFROM", value: "+0530"),
            ICalProperty(name: "TZOFFSETTO", value: "+0530"),
        ]),
    ])
    let zone = TimeZoneResolver(calendar: ICalComponent(name: "VCALENDAR", components: [definition])).zone(for: "Custom +0530")
    #expect(zone?.secondsFromGMT(for: instant(2026, 7, 1)) == 19_800)
}

@Test func xLicLocationWins() {
    let definition = ICalComponent(name: "VTIMEZONE", properties: [
        ICalProperty(name: "TZID", value: "Weird"), ICalProperty(name: "X-LIC-LOCATION", value: "Asia/Tokyo"),
    ])
    #expect(TimeZoneResolver(calendar: ICalComponent(name: "VCALENDAR", components: [definition])).zone(for: "Weird")?.identifier == "Asia/Tokyo")
}

@Test func resolvesValuesWithTheRightZone() {
    let resolver = TimeZoneResolver()
    let floating = TimeZone(identifier: "America/Los_Angeles")!
    let local = LocalDateTime(date: CalendarDate(year: 2026, month: 9, day: 27), hour: 9, minute: 0, second: 0)
    #expect(resolver.date(.local(local, tzid: "Europe/Berlin"), floating: floating) == Date(timeIntervalSince1970: 1_790_492_400))
    #expect(resolver.date(.local(local, tzid: nil), floating: floating) == Date(timeIntervalSince1970: 1_790_524_800))
    #expect(resolver.date(.date(CalendarDate(year: 2026, month: 9, day: 27)), floating: floating)
        == AllDay.startOfDay(CalendarDate(year: 2026, month: 9, day: 27), in: floating))
    #expect(resolver.displayZone(.local(local, tzid: "Europe/Berlin"), fallback: floating).identifier == "Europe/Berlin")
    #expect(resolver.displayZone(.utc(Date()), fallback: floating).identifier == "UTC" || resolver.displayZone(.utc(Date()), fallback: floating).identifier == "GMT")
    #expect(resolver.displayZone(.local(local, tzid: nil), fallback: floating) == floating)
}

@Test func vtimezoneCoversEveryTransitionInTheRange() throws {
    let newYork = TimeZone(identifier: "America/New_York")!
    let component = try #require(VTimeZoneWriter.component(for: newYork, from: instant(2026, 6, 1), through: instant(2046, 6, 1)))
    #expect(component.property("TZID")?.value == "America/New_York")
    let daylight = component.components(named: "DAYLIGHT")
    let standard = component.components(named: "STANDARD")
    // One DST start and one end per year from a year before the start through the end: 2025 ... 2046.
    #expect(daylight.count >= 21 && standard.count >= 21)
    #expect(daylight.allSatisfy { $0.property("TZOFFSETTO")?.value == "-0400" && $0.property("TZOFFSETFROM")?.value == "-0500" })
    #expect(daylight.first?.property("DTSTART")?.value == "20250309T020000")
    #expect(VTimeZoneWriter.component(for: TimeZone(identifier: "UTC")!, from: instant(2026, 1, 1), through: instant(2027, 1, 1)) == nil)
}

@Test func zoneWithoutTransitionsGetsOneStandardObservance() throws {
    let tokyo = TimeZone(identifier: "Asia/Tokyo")!
    let component = try #require(VTimeZoneWriter.component(for: tokyo, from: instant(2026, 1, 1), through: instant(2027, 1, 1)))
    let standard = component.components(named: "STANDARD")
    #expect(standard.count == 1)
    #expect(standard.first?.property("TZOFFSETTO")?.value == "+0900")
    #expect(component.components(named: "DAYLIGHT").isEmpty)
}
```

(`1_790_521_200` is 2026-09-27T15:00:00Z; `1_790_492_400` is 07:00Z = 09:00 CEST; `1_790_524_800` is 16:00Z = 09:00 PDT. Check them with `date -u -r <n>` if a test disagrees before changing code.)

- [ ] **Step 2: Run to verify they fail**

Run: `swift test --package-path Packages/CalendarConnectors --filter "Values|TimeZone"`
Expected: compile errors.

- [ ] **Step 3: Implement `ICalValues.swift`**

```swift
import CalendarCore
import Foundation

public struct LocalDateTime: Hashable, Sendable {
    public var date: CalendarDate
    public var hour: Int
    public var minute: Int
    public var second: Int
    public init(date: CalendarDate, hour: Int, minute: Int, second: Int) {
        self.date = date
        self.hour = hour
        self.minute = minute
        self.second = second
    }
}

/// A DATE or DATE-TIME value as written: a date, a UTC instant, or a wall-clock time in a named zone (`tzid`) or
/// floating (`tzid == nil`, read in the calendar's zone).
public enum ICalDateValue: Hashable, Sendable {
    case date(CalendarDate)
    case utc(Date)
    case local(LocalDateTime, tzid: String?)
}

public enum ICalValues {
    private static let utcZone = TimeZone(identifier: "UTC")!

    /// The property's first value, read with its `VALUE` and `TZID` parameters; nil when it is malformed.
    public static func dateValue(_ property: ICalProperty) -> ICalDateValue? {
        dateValues(property).first
    }

    /// Every value of a list property (`EXDATE`, `RDATE`); empty when any token is malformed or the values are periods.
    public static func dateValues(_ property: ICalProperty) -> [ICalDateValue] {
        let kind = property.parameter("VALUE")?.uppercased()
        if kind == "PERIOD" { return [] }
        let tzid = property.parameter("TZID")
        var values: [ICalDateValue] = []
        for token in property.value.split(separator: ",") {
            guard let value = parse(String(token), isDate: kind == "DATE", tzid: tzid) else { return [] }
            values.append(value)
        }
        return values
    }

    static func parse(_ token: String, isDate: Bool, tzid: String?) -> ICalDateValue? {
        let chars = Array(token.trimmingCharacters(in: .whitespaces).uppercased())
        func number(_ range: Range<Int>) -> Int? {
            guard chars.count >= range.upperBound, chars[range].allSatisfy({ $0.isASCII && $0.isNumber }) else { return nil }
            return Int(String(chars[range]))
        }
        guard let year = number(0..<4), let month = number(4..<6), let day = number(6..<8),
              (1...12).contains(month), (1...31).contains(day) else { return nil }
        let date = CalendarDate(year: year, month: month, day: day)
        if chars.count == 8 { return .date(date) }
        guard !isDate, chars.count >= 15, chars[8] == "T", let hour = number(9..<11), let minute = number(11..<13),
              let second = number(13..<15), hour < 24, minute < 60, second <= 60 else { return nil }
        let local = LocalDateTime(date: date, hour: hour, minute: minute, second: min(second, 59))
        if chars.count == 16, chars[15] == "Z" {
            var calendar = Calendar(identifier: .gregorian)
            calendar.timeZone = utcZone
            guard let instant = calendar.date(from: DateComponents(year: year, month: month, day: day, hour: hour, minute: minute, second: local.second))
            else { return nil }
            return .utc(instant)
        }
        guard chars.count == 15 else { return nil }
        return .local(local, tzid: tzid)
    }

    /// RFC 5545 DURATION (`[+-]P[nW][nD][T[nH][nM][nS]]`); nil when malformed.
    public static func duration(_ text: String) -> TimeInterval? {
        var chars = Array(text.trimmingCharacters(in: .whitespaces).uppercased())
        var sign: Double = 1
        if chars.first == "-" { sign = -1; chars.removeFirst() } else if chars.first == "+" { chars.removeFirst() }
        guard chars.first == "P" else { return nil }
        chars.removeFirst()
        var total: Double = 0
        var number = ""
        var inTime = false
        var sawPart = false
        for c in chars {
            switch c {
            case "T": guard number.isEmpty, !inTime else { return nil }; inTime = true
            case "0"..."9": number.append(c)
            case "W", "D", "H", "M", "S":
                guard let n = Double(number) else { return nil }
                let unit: Double
                switch (c, inTime) {
                case ("W", false): unit = 604_800
                case ("D", false): unit = 86_400
                case ("H", true): unit = 3600
                case ("M", true): unit = 60
                case ("S", true): unit = 1
                default: return nil
                }
                total += n * unit
                number = ""
                sawPart = true
            default: return nil
            }
        }
        guard number.isEmpty, sawPart else { return nil }
        return sign * total
    }

    public static func durationText(_ seconds: TimeInterval) -> String {
        var rest = Int(abs(seconds).rounded())
        let sign = seconds < 0 ? "-" : ""
        if rest == 0 { return "PT0S" }
        let days = rest / 86_400; rest %= 86_400
        let hours = rest / 3600; rest %= 3600
        let minutes = rest / 60
        let secs = rest % 60
        var text = sign + "P" + (days > 0 ? "\(days)D" : "")
        if hours + minutes + secs > 0 {
            text += "T" + (hours > 0 ? "\(hours)H" : "") + (minutes > 0 ? "\(minutes)M" : "") + (secs > 0 ? "\(secs)S" : "")
        }
        return text
    }

    public static func utcText(_ date: Date) -> String { localText(date, in: utcZone) + "Z" }

    public static func dateText(_ date: CalendarDate) -> String {
        String(format: "%04d%02d%02d", date.year, date.month, date.day)
    }

    public static func localText(_ date: Date, in zone: TimeZone) -> String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = zone
        let c = calendar.dateComponents([.year, .month, .day, .hour, .minute, .second], from: date)
        return String(format: "%04d%02d%02dT%02d%02d%02d", c.year!, c.month!, c.day!, c.hour!, c.minute!, c.second!)
    }
}
```

- [ ] **Step 4: Implement `TimeZoneResolver.swift`**

```swift
import CalendarCore
import Foundation

/// Turns a `TZID` into a `TimeZone`: an IANA id; the IANA id inside a path-style id (`/mozilla.org/.../America/New_York`);
/// a Windows name; the file's own `VTIMEZONE` (its `X-LIC-LOCATION`, else the system zone with the same offsets and
/// transition months, else a fixed offset). nil when nothing fits.
public struct TimeZoneResolver: Sendable {
    private let definitions: [String: ICalComponent]

    public init() { definitions = [:] }

    public init(calendar: ICalComponent) {
        var found: [String: ICalComponent] = [:]
        for definition in calendar.components(named: "VTIMEZONE") {
            if let id = definition.property("TZID")?.value { found[id] = definition }
        }
        definitions = found
    }

    public func zone(for tzid: String) -> TimeZone? {
        let id = tzid.trimmingCharacters(in: .whitespaces)
        if let zone = TimeZone(identifier: id) { return zone }
        let parts = id.split(separator: "/").map(String.init)
        if parts.count > 1 {
            for start in 1..<parts.count {
                if let zone = TimeZone(identifier: parts[start...].joined(separator: "/")) { return zone }
            }
        }
        if let zone = WindowsTimeZones.timeZone(for: id) { return zone }
        if let definition = definitions[id] { return Self.zone(matching: definition) }
        return nil
    }

    /// The instant a value names. Floating times and dates are read in `zone`; an unknown TZID too.
    public func date(_ value: ICalDateValue, floating zone: TimeZone) -> Date? {
        switch value {
        case .date(let day): return AllDay.startOfDay(day, in: zone)
        case .utc(let instant): return instant
        case .local(let local, let tzid):
            let resolved = tzid.flatMap { self.zone(for: $0) } ?? zone
            return WallClock.date(local.date, hour: local.hour, minute: local.minute, second: local.second, in: resolved)
        }
    }

    /// The zone to show a value in: its TZID's zone, UTC for a UTC value, `fallback` for floating times and dates.
    public func displayZone(_ value: ICalDateValue, fallback: TimeZone) -> TimeZone {
        switch value {
        case .utc: return TimeZone(identifier: "UTC")!
        case .local(_, let tzid?): return zone(for: tzid) ?? fallback
        default: return fallback
        }
    }

    // MARK: VTIMEZONE matching

    private static func zone(matching definition: ICalComponent) -> TimeZone? {
        if let location = definition.property("X-LIC-LOCATION")?.value, let zone = TimeZone(identifier: location) { return zone }
        let observances = definition.components.filter { $0.name == "STANDARD" || $0.name == "DAYLIGHT" }
        func latest(_ kind: String) -> ICalComponent? {
            observances.filter { $0.name == kind }.max { ($0.property("DTSTART")?.value ?? "") < ($1.property("DTSTART")?.value ?? "") }
        }
        guard let standard = latest("STANDARD"), let standardOffset = offset(standard.property("TZOFFSETTO")?.value) else { return nil }
        let daylight = latest("DAYLIGHT")
        let daylightOffset = daylight.flatMap { offset($0.property("TZOFFSETTO")?.value) }
        let year = max(2000, Int(String((standard.property("DTSTART")?.value ?? "2000").prefix(4))) ?? 2000)
        var utc = Calendar(identifier: .gregorian)
        utc.timeZone = TimeZone(identifier: "UTC")!
        let january = utc.date(from: DateComponents(year: year, month: 1, day: 15, hour: 12))!
        let july = utc.date(from: DateComponents(year: year, month: 7, day: 15, hour: 12))!
        let wanted: Set<Int> = daylightOffset.map { [standardOffset, $0] } ?? [standardOffset]
        let candidates = TimeZone.knownTimeZoneIdentifiers.sorted().compactMap(TimeZone.init(identifier:)).filter {
            Set([$0.secondsFromGMT(for: january), $0.secondsFromGMT(for: july)]) == wanted
        }
        if let daylight, let months = transitionMonths(daylight: daylight, standard: standard) {
            let start = utc.date(from: DateComponents(year: year, month: 1, day: 1))!
            if let match = candidates.first(where: { transitionMonths(of: $0, after: start, calendar: utc) == months }) { return match }
        }
        return candidates.first ?? TimeZone(secondsFromGMT: standardOffset)
    }

    /// `+HHMM`, `-HHMM` or `+HHMMSS` in seconds.
    static func offset(_ text: String?) -> Int? {
        guard let text, text.count == 5 || text.count == 7, let sign = text.first, sign == "+" || sign == "-" else { return nil }
        let digits = Array(text.dropFirst())
        guard digits.allSatisfy({ $0.isASCII && $0.isNumber }) else { return nil }
        let hours = Int(String(digits[0..<2]))!, minutes = Int(String(digits[2..<4]))!
        let seconds = digits.count == 6 ? Int(String(digits[4..<6]))! : 0
        return (sign == "-" ? -1 : 1) * (hours * 3600 + minutes * 60 + seconds)
    }

    /// The months daylight time starts and ends in, from the observances' `RRULE:...;BYMONTH=` (or their `DTSTART`).
    private static func transitionMonths(daylight: ICalComponent, standard: ICalComponent) -> [Int]? {
        func month(_ observance: ICalComponent) -> Int? {
            if let rule = observance.property("RRULE")?.value,
               let part = rule.split(separator: ";").first(where: { $0.uppercased().hasPrefix("BYMONTH=") }) {
                return Int(part.dropFirst(8))
            }
            return observance.property("DTSTART").flatMap { Int(String($0.value.dropFirst(4).prefix(2))) }
        }
        guard let start = month(daylight), let end = month(standard) else { return nil }
        return [start, end]
    }

    private static func transitionMonths(of zone: TimeZone, after start: Date, calendar: Calendar) -> [Int]? {
        guard let first = zone.nextDaylightSavingTimeTransition(after: start),
              let second = zone.nextDaylightSavingTimeTransition(after: first) else { return nil }
        let (a, b) = zone.isDaylightSavingTime(for: first) ? (first, second) : (second, first)
        return [calendar.component(.month, from: a), calendar.component(.month, from: b)]
    }
}
```

- [ ] **Step 5: Implement `VTimeZoneWriter.swift`**

```swift
import Foundation

/// Builds the `VTIMEZONE` a written event needs for its `TZID` (RFC 4791 requires one for each TZID used).
public enum VTimeZoneWriter {
    /// One observance per transition from a year before `start` through `end` (at most 400), or a single `STANDARD`
    /// observance for a zone without transitions. nil for UTC, which is written with `Z` instead.
    public static func component(for zone: TimeZone, from start: Date, through end: Date) -> ICalComponent? {
        if zone.identifier == "UTC" || zone.identifier == "GMT" { return nil }
        let utc = TimeZone(identifier: "UTC")!
        var observances: [ICalComponent] = []
        // January 1 of the year before `start`'s year: a fixed 366-day lookback from `start` itself can land after
        // that year's DST-start transition (e.g. `start` in June), skipping straight to the following year's.
        var utcCalendar = Calendar(identifier: .gregorian)
        utcCalendar.timeZone = utc
        let startYear = utcCalendar.component(.year, from: start)
        var cursor = utcCalendar.date(from: DateComponents(year: startYear - 1, month: 1, day: 1)) ?? start.addingTimeInterval(-366 * 86_400)
        while observances.count < 400, let transition = zone.nextDaylightSavingTimeTransition(after: cursor), transition <= end {
            let from = zone.secondsFromGMT(for: transition.addingTimeInterval(-1))
            let to = zone.secondsFromGMT(for: transition)
            var properties = [
                // DTSTART is the transition's local time read with the offset before it.
                ICalProperty(name: "DTSTART", value: ICalValues.localText(transition.addingTimeInterval(TimeInterval(from)), in: utc)),
                ICalProperty(name: "TZOFFSETFROM", value: offsetText(from)),
                ICalProperty(name: "TZOFFSETTO", value: offsetText(to)),
            ]
            if let name = zone.abbreviation(for: transition) { properties.append(ICalProperty(name: "TZNAME", text: name)) }
            observances.append(ICalComponent(name: zone.isDaylightSavingTime(for: transition) ? "DAYLIGHT" : "STANDARD", properties: properties))
            cursor = transition
        }
        if observances.isEmpty {
            let offset = zone.secondsFromGMT(for: start)
            observances = [ICalComponent(name: "STANDARD", properties: [
                ICalProperty(name: "DTSTART", value: "19700101T000000"),
                ICalProperty(name: "TZOFFSETFROM", value: offsetText(offset)),
                ICalProperty(name: "TZOFFSETTO", value: offsetText(offset)),
            ])]
        }
        return ICalComponent(name: "VTIMEZONE", properties: [ICalProperty(name: "TZID", value: zone.identifier)], components: observances)
    }

    static func offsetText(_ seconds: Int) -> String {
        let sign = seconds < 0 ? "-" : "+"
        let value = abs(seconds)
        let text = String(format: "%@%02d%02d", sign, value / 3600, (value % 3600) / 60)
        return value % 60 == 0 ? text : text + String(format: "%02d", value % 60)
    }
}
```

- [ ] **Step 6: Run the tests**

Run: `swift test --package-path Packages/CalendarConnectors --filter "Values|TimeZone|ContentLine"`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add Packages/CalendarConnectors/Sources/ICalendar Packages/CalendarConnectors/Tests/ICalendarTests
git commit -m "ICalendar: date, duration and time zone values; VTIMEZONE reading and writing"
```

---

### Task 6: Reading events (resource, attendees, alarms, expansion)

**Files:**
- Create: `Packages/CalendarConnectors/Sources/ICalendar/EventResource.swift`, `AttendeeMapper.swift`, `AlarmMapper.swift`, `EventReader.swift`
- Test: `Packages/CalendarConnectors/Tests/ICalendarTests/EventReaderTests.swift`, `AlarmMapperTests.swift`, `Fixtures.swift`

**Interfaces:**
- Consumes: Tasks 3-5; `CalendarUserAddress`, `ConferenceDetector`, `AllDay`, `RecurrenceSet`.
- Produces:
  ```swift
  public struct EventResource: Sendable, Equatable {
      public var calendar: ICalComponent
      public init(calendar: ICalComponent) throws; public init(data: Data) throws
      public var events: [ICalComponent] { get }; public var master: ICalComponent? { get }; public var overrides: [ICalComponent] { get }
      public var uid: String? { get }; public var resolver: TimeZoneResolver { get }
      public func serialized() -> Data
      public mutating func setEvents(_ events: [ICalComponent])
      public mutating func ensureTimeZones(_ zones: [TimeZone], from start: Date, through end: Date)
  }
  public struct EventReadContext: Sendable {
      public var calendarID: String; public var resourceName: String; public var etag: String?; public var sourceID: String?
      public var calendarZone: TimeZone; public var selfAddresses: Set<String>
      public init(calendarID:resourceName:etag:sourceID:calendarZone:selfAddresses:)
  }
  public enum EventReader {
      public static let instanceLimit: Int   // 5000
      public static func events(in resource: EventResource, overlapping window: DateInterval, context: EventReadContext) -> [CalendarEvent]
      public static func masterEvent(of resource: EventResource, context: EventReadContext) -> CalendarEvent?
      public static func occurrence(in resource: EventResource, originalStart: Date, context: EventReadContext) -> CalendarEvent?
      public static func series(of resource: EventResource, context: EventReadContext) -> CalendarSeries?
      public static func eventID(resourceName: String, originalStart: Date?, isAllDay: Bool, zone: TimeZone) -> String
      public static func recurrenceSet(of master: ICalComponent, zone: TimeZone, isAllDay: Bool, resolver: TimeZoneResolver) -> RecurrenceSet?
      public static func timing(of vevent: ICalComponent, resolver: TimeZoneResolver, calendarZone: TimeZone) -> EventTimingInfo?
      public static func recurrenceID(of vevent: ICalComponent, resolver: TimeZoneResolver, calendarZone: TimeZone) -> Date?
  }
  public struct EventTimingInfo: Sendable, Equatable { public var start: Date; public var end: Date; public var zone: TimeZone; public var isAllDay: Bool; public var allDayLength: Int }
  public enum AttendeeMapper {
      public static func read(_ vevent: ICalComponent, selfAddresses: Set<String>) -> (attendees: [Attendee], organizer: Attendee?)
      public static func isSelf(_ address: String, email: String?, selfAddresses: Set<String>) -> Bool
      public static func participation(attendees: [Attendee], organizer: Attendee?, knowsSelf: Bool) -> Participation?
  }
  public enum AlarmMapper { public static func reminders(from alarms: [ICalComponent]) -> [Reminder] }
  ```
  `selfAddresses` are lowercased raw calendar-user addresses (`mailto:me@x.test`, `urn:uuid:...`, principal URLs).

- [ ] **Step 1: Write the fixtures and failing tests**

`Fixtures.swift`:

```swift
import CalendarCore
import Foundation
@testable import ICalendar

let la = TimeZone(identifier: "America/Los_Angeles")!

func laTime(_ y: Int, _ mo: Int, _ d: Int, _ h: Int = 10, _ mi: Int = 0) -> Date {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = la
    return calendar.date(from: DateComponents(year: y, month: mo, day: d, hour: h, minute: mi))!
}

func context(_ name: String = "4F2A.ics", selfAddresses: Set<String> = ["mailto:me@icloud.test"]) -> EventReadContext {
    EventReadContext(calendarID: "home", resourceName: name, etag: "\"e1\"", sourceID: "icloud-conn",
                     calendarZone: la, selfAddresses: selfAddresses)
}

func resource(_ text: String) throws -> EventResource {
    try EventResource(data: Data(text.replacingOccurrences(of: "\n", with: "\r\n").utf8))
}

/// Shaped like an iCloud export (scrubbed): a weekly Tuesday 10:00 series in Los Angeles from 2026-09-01, one excluded
/// date, one occurrence moved to Wednesday 14:00, attendees (the organizer's own address is a principal URL), a default alarm.
let weeklySeries = """
BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Apple Inc.//iCloud//EN
BEGIN:VTIMEZONE
TZID:America/Los_Angeles
BEGIN:DAYLIGHT
DTSTART:20070311T020000
RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU
TZOFFSETFROM:-0800
TZOFFSETTO:-0700
END:DAYLIGHT
BEGIN:STANDARD
DTSTART:20071104T020000
RRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU
TZOFFSETFROM:-0700
TZOFFSETTO:-0800
END:STANDARD
END:VTIMEZONE
BEGIN:VEVENT
UID:4F2A-UID
DTSTAMP:20260901T000000Z
CREATED:20260801T120000Z
LAST-MODIFIED:20260902T120000Z
SEQUENCE:2
SUMMARY:Team sync
DESCRIPTION:Agenda\\nhttps://zoom.us/j/123456789
LOCATION:Room 4
DTSTART;TZID=America/Los_Angeles:20260901T100000
DTEND;TZID=America/Los_Angeles:20260901T103000
RRULE:FREQ=WEEKLY;BYDAY=TU
EXDATE;TZID=America/Los_Angeles:20260908T100000
TRANSP:OPAQUE
CLASS:PRIVATE
STATUS:CONFIRMED
ORGANIZER;CN=Me:mailto:me@icloud.test
ATTENDEE;CN=Me;PARTSTAT=ACCEPTED;ROLE=CHAIR:mailto:me@icloud.test
ATTENDEE;CN=Ann;PARTSTAT=TENTATIVE;ROLE=REQ-PARTICIPANT:mailto:Ann@Example.test
ATTENDEE;CN=Room;CUTYPE=ROOM;PARTSTAT=ACCEPTED:mailto:room@example.test
ATTENDEE;CN=Bo;ROLE=OPT-PARTICIPANT;PARTSTAT=NEEDS-ACTION:urn:uuid:1234
X-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC
BEGIN:VALARM
ACTION:DISPLAY
DESCRIPTION:Reminder
TRIGGER:-PT15M
X-APPLE-DEFAULT-ALARM:TRUE
END:VALARM
END:VEVENT
BEGIN:VEVENT
UID:4F2A-UID
DTSTAMP:20260901T000000Z
RECURRENCE-ID;TZID=America/Los_Angeles:20260915T100000
SUMMARY:Team sync (moved)
DTSTART;TZID=America/Los_Angeles:20260916T140000
DTEND;TZID=America/Los_Angeles:20260916T143000
ORGANIZER;CN=Me:mailto:me@icloud.test
ATTENDEE;CN=Me;PARTSTAT=ACCEPTED:mailto:me@icloud.test
END:VEVENT
END:VCALENDAR
"""
```

`EventReaderTests.swift`:

```swift
import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import ICalendar

private let september = DateInterval(start: laTime(2026, 9, 1, 0), end: laTime(2026, 10, 1, 0))

@Test func weeklySeriesExpandsWithExdateAndMovedOverride() throws {
    let events = EventReader.events(in: try resource(weeklySeries), overlapping: september, context: context())
    #expect(events.map(\.start) == [laTime(2026, 9, 1), laTime(2026, 9, 16, 14), laTime(2026, 9, 22), laTime(2026, 9, 29)])
    let moved = events[1]
    #expect(moved.title == "Team sync (moved)")
    #expect(moved.series == .occurrence(seriesID: "4F2A.ics", originalStart: laTime(2026, 9, 15)))
    #expect(moved.eventID == "4F2A.ics#20260915T170000Z")
    #expect(events[0].eventID == "4F2A.ics#20260901T170000Z")
    #expect(events[0].end == laTime(2026, 9, 1, 10, 30))
}

@Test func mapsEveryField() throws {
    let first = try #require(EventReader.events(in: try resource(weeklySeries), overlapping: september, context: context()).first)
    #expect(first.uid == "4F2A-UID")
    #expect(first.uidScope == .global)
    #expect(first.calendarID == "home")
    #expect(first.sourceID == "icloud-conn")
    #expect(first.title == "Team sync")
    #expect(first.notes == "Agenda\nhttps://zoom.us/j/123456789")
    #expect(first.location == "Room 4")
    #expect(first.timeZone.identifier == "America/Los_Angeles")
    #expect(!first.isAllDay)
    #expect(first.status == .confirmed)
    #expect(first.availability == .busy)
    #expect(first.visibility == .privateEvent)
    #expect(first.version == "\"e1\"")
    #expect(first.lastModified == Date(timeIntervalSince1970: 1_788_350_400))   // 2026-09-02T12:00:00Z
    #expect(first.created == Date(timeIntervalSince1970: 1_785_585_600))        // 2026-08-01T12:00:00Z
    #expect(first.conference?.provider == .zoom)
    #expect(first.reminders == [Reminder(trigger: .relative(offset: -900, to: .start), isCalendarDefault: true)])
}

@Test func attendeesSelfAndParticipation() throws {
    let first = try #require(EventReader.events(in: try resource(weeklySeries), overlapping: september, context: context()).first)
    #expect(first.organizer?.email == "me@icloud.test")
    #expect(first.organizer?.isSelf == true)
    #expect(first.attendees.map(\.email) == ["me@icloud.test", "ann@example.test", "room@example.test", nil])
    #expect(first.attendees.map(\.response) == [.accepted, .tentative, .accepted, .needsAction])
    #expect(first.attendees.map(\.role) == [.required, .required, .resource, .optional])
    #expect(first.attendees[0].isSelf && first.attendees[0].isOrganizer)
    #expect(first.attendees[3].name == "Bo")
    #expect(first.participation == .invited(.accepted))
}

@Test func noSelfAddressesGivesNilParticipation() throws {
    let first = try #require(EventReader.events(in: try resource(weeklySeries), overlapping: september,
                                                context: context(selfAddresses: [])).first)
    #expect(first.participation == nil)
    #expect(first.attendees.allSatisfy { !$0.isSelf })
}

@Test func overrideMovedIntoWindowIsShownAndItsSlotIsNot() throws {
    // A window holding only Wednesday the 16th: the override is in it; its slot (Tuesday the 15th) is not.
    let window = DateInterval(start: laTime(2026, 9, 16, 0), end: laTime(2026, 9, 17, 0))
    let events = EventReader.events(in: try resource(weeklySeries), overlapping: window, context: context())
    #expect(events.map(\.title) == ["Team sync (moved)"])
    // A window holding only Tuesday the 15th shows nothing: the occurrence moved away.
    let slot = DateInterval(start: laTime(2026, 9, 15, 0), end: laTime(2026, 9, 16, 0))
    #expect(EventReader.events(in: try resource(weeklySeries), overlapping: slot, context: context()).isEmpty)
}

@Test func overrideWithoutMasterIsShown() throws {
    let text = """
    BEGIN:VCALENDAR
    VERSION:2.0
    BEGIN:VEVENT
    UID:only-one
    RECURRENCE-ID:20260915T170000Z
    DTSTART:20260915T170000Z
    DTEND:20260915T180000Z
    SUMMARY:Just this one
    END:VEVENT
    END:VCALENDAR
    """
    let events = EventReader.events(in: try resource(text), overlapping: september, context: context("only.ics"))
    #expect(events.count == 1)
    #expect(events.first?.series == .occurrence(seriesID: "only.ics", originalStart: Date(timeIntervalSince1970: 1_789_491_600)))
    #expect(events.first?.timeZone.identifier == "UTC" || events.first?.timeZone.identifier == "GMT")
}

@Test func cancelledOverrideIsReturnedCancelled() throws {
    let text = weeklySeries.replacingOccurrences(of: "SUMMARY:Team sync (moved)", with: "SUMMARY:Team sync (moved)\nSTATUS:CANCELLED")
    let moved = try #require(EventReader.events(in: try resource(text), overlapping: september, context: context())
        .first { $0.title == "Team sync (moved)" })
    #expect(moved.status == .cancelled)
}

@Test func singleEventIsNotRecurring() throws {
    let text = """
    BEGIN:VCALENDAR
    VERSION:2.0
    BEGIN:VEVENT
    UID:single
    DTSTART;TZID=America/Los_Angeles:20260910T090000
    DURATION:PT45M
    SUMMARY:One off
    TRANSP:TRANSPARENT
    END:VEVENT
    END:VCALENDAR
    """
    let event = try #require(EventReader.events(in: try resource(text), overlapping: september, context: context("single.ics")).first)
    #expect(event.eventID == "single.ics")
    #expect(event.series == .notRecurring)
    #expect(event.end == laTime(2026, 9, 10, 9, 45))
    #expect(event.availability == .free)
    #expect(event.visibility == .default)
    #expect(event.reminders == [])
    #expect(event.participation == .notInvited)
}

@Test func allDayUsesTheCalendarZoneAndPassesConformance() throws {
    let text = """
    BEGIN:VCALENDAR
    VERSION:2.0
    BEGIN:VEVENT
    UID:day
    DTSTART;VALUE=DATE:20260910
    DTEND;VALUE=DATE:20260912
    RRULE:FREQ=WEEKLY;COUNT=2
    SUMMARY:Offsite
    END:VEVENT
    END:VCALENDAR
    """
    let events = EventReader.events(in: try resource(text), overlapping: september, context: context("day.ics"))
    #expect(events.count == 2)
    for event in events {
        #expect(event.isAllDay)
        #expect(AllDayConformance.violations(event) == [])
    }
    #expect(events[0].start == AllDay.startOfDay(CalendarDate(year: 2026, month: 9, day: 10), in: la))
    #expect(events[0].end == AllDay.startOfDay(CalendarDate(year: 2026, month: 9, day: 12), in: la))
    #expect(events[0].eventID == "day.ics#20260910")
}

@Test func unreadableRuleShowsFirstOccurrenceAndOverrides() throws {
    let text = weeklySeries.replacingOccurrences(of: "RRULE:FREQ=WEEKLY;BYDAY=TU", with: "RRULE:FREQ=FORTNIGHTLY")
    let events = EventReader.events(in: try resource(text), overlapping: september, context: context())
    #expect(events.map(\.start) == [laTime(2026, 9, 1), laTime(2026, 9, 16, 14)])
}

@Test func windowsTZIDIsResolved() throws {
    let text = weeklySeries.replacingOccurrences(of: "TZID=America/Los_Angeles", with: "TZID=Pacific Standard Time")
    let first = try #require(EventReader.events(in: try resource(text), overlapping: september, context: context()).first)
    #expect(first.start == laTime(2026, 9, 1))
}

@Test func masterOccurrenceAndSeriesLookups() throws {
    let r = try resource(weeklySeries)
    let master = try #require(EventReader.masterEvent(of: r, context: context()))
    #expect(master.eventID == "4F2A.ics")
    #expect(master.series == .occurrence(seriesID: "4F2A.ics", originalStart: laTime(2026, 9, 1)))
    #expect(EventReader.occurrence(in: r, originalStart: laTime(2026, 9, 22), context: context())?.eventID == "4F2A.ics#20260922T170000Z")
    #expect(EventReader.occurrence(in: r, originalStart: laTime(2026, 9, 15), context: context())?.title == "Team sync (moved)")
    #expect(EventReader.occurrence(in: r, originalStart: laTime(2026, 9, 8), context: context()) == nil)        // excluded
    #expect(EventReader.occurrence(in: r, originalStart: laTime(2026, 9, 23), context: context()) == nil)       // not a slot
    let series = try #require(EventReader.series(of: r, context: context()))
    #expect(series.seriesID == "4F2A.ics")
    #expect(series.recurrence.rules.first?.frequency == .weekly)
    #expect(series.recurrence.excludedDates == [laTime(2026, 9, 8)])
}

@Test func readEventsMeetTheProvidedFieldRule() throws {
    let capabilities = SourceCapabilities(providedFields: [.visibility, .availability, .reminders, .series, .participation, .version, .uidScope])
    for event in EventReader.events(in: try resource(weeklySeries), overlapping: september, context: context()) {
        #expect(ProvidedFieldsConformance.violations(event: event, capabilities: capabilities) == [])
    }
}
```

`AlarmMapperTests.swift`:

```swift
import CalendarCore
import Foundation
import Testing
@testable import ICalendar

private func alarm(_ lines: [String]) throws -> ICalComponent {
    try ICalParser.parse((["BEGIN:VALARM"] + lines + ["END:VALARM"]).joined(separator: "\r\n"))
}

@Test func readsRelativeAbsoluteRepeatingAndEndAlarms() throws {
    let reminders = AlarmMapper.reminders(from: [
        try alarm(["ACTION:DISPLAY", "TRIGGER:-PT10M"]),
        try alarm(["ACTION:AUDIO", "TRIGGER;RELATED=END:PT0S", "ATTACH;VALUE=URI:Chord"]),
        try alarm(["ACTION:DISPLAY", "TRIGGER;VALUE=DATE-TIME:20260927T150000Z", "REPEAT:2", "DURATION:PT5M"]),
        try alarm(["ACTION:EMAIL", "TRIGGER:-P1D", "ATTENDEE:mailto:me@x.test"]),
        try alarm(["ACTION:DISPLAY", "TRIGGER:not-a-duration"]),
    ])
    #expect(reminders == [
        Reminder(trigger: .relative(offset: -600, to: .start), isCalendarDefault: false),
        Reminder(trigger: .relative(offset: 0, to: .end), type: .audio(soundName: "Chord"), isCalendarDefault: false),
        Reminder(trigger: .absolute(Date(timeIntervalSince1970: 1_790_521_200)), repeatCount: 2, repeatInterval: 300, isCalendarDefault: false),
        Reminder(trigger: .relative(offset: -86_400, to: .start), type: .email(address: "me@x.test"), isCalendarDefault: false),
    ])
}

@Test func readsAppleProximityAlarms() throws {
    let reminders = AlarmMapper.reminders(from: [try alarm([
        "ACTION:DISPLAY", "TRIGGER;VALUE=DATE-TIME:19760401T005545Z", "X-APPLE-PROXIMITY:ARRIVE",
        "X-APPLE-STRUCTURED-LOCATION;VALUE=URI;X-APPLE-RADIUS=100;X-TITLE=Office:geo:37.33,-122.03",
    ])])
    #expect(reminders == [Reminder(trigger: .location(StructuredLocation(title: "Office", latitude: 37.33, longitude: -122.03, radius: 100), .enter),
                                   isCalendarDefault: false)])
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `swift test --package-path Packages/CalendarConnectors --filter "EventReader|AlarmMapper"`
Expected: compile errors.

- [ ] **Step 3: Implement `EventResource.swift`**

```swift
import CalendarCore
import Foundation

/// One CalDAV calendar object resource: a `VCALENDAR` whose `VEVENT`s share one `UID` (the series master, if any, and
/// its `RECURRENCE-ID` overrides), plus its `VTIMEZONE`s and anything else the file holds.
public struct EventResource: Sendable, Equatable {
    public var calendar: ICalComponent

    public init(calendar: ICalComponent) throws {
        guard calendar.name == "VCALENDAR" else { throw ICalError.malformed("not a VCALENDAR") }
        guard !calendar.components(named: "VEVENT").isEmpty else { throw ICalError.malformed("no VEVENT") }
        self.calendar = calendar
    }

    public init(data: Data) throws { try self.init(calendar: ICalParser.parse(data)) }

    public var events: [ICalComponent] { calendar.components(named: "VEVENT") }
    public var master: ICalComponent? { events.first { $0.property("RECURRENCE-ID") == nil } }
    public var overrides: [ICalComponent] { events.filter { $0.property("RECURRENCE-ID") != nil } }
    public var uid: String? { events.first?.property("UID")?.text }
    public var resolver: TimeZoneResolver { TimeZoneResolver(calendar: calendar) }

    public func serialized() -> Data { Data(ICalSerializer.serialize(calendar).utf8) }

    /// Replaces the `VEVENT`s, keeping every other component where it was.
    public mutating func setEvents(_ events: [ICalComponent]) {
        let others = calendar.components.filter { $0.name != "VEVENT" }
        calendar.components = others + events
    }

    /// Adds a `VTIMEZONE` for each zone that has none yet (UTC needs none).
    public mutating func ensureTimeZones(_ zones: [TimeZone], from start: Date, through end: Date) {
        let present = Set(calendar.components(named: "VTIMEZONE").compactMap { $0.property("TZID")?.value })
        var added: [ICalComponent] = []
        for zone in zones where !present.contains(zone.identifier) && !added.contains(where: { $0.property("TZID")?.value == zone.identifier }) {
            if let definition = VTimeZoneWriter.component(for: zone, from: start, through: end) { added.append(definition) }
        }
        calendar.components = added + calendar.components
    }
}
```

- [ ] **Step 4: Implement `AttendeeMapper.swift`**

```swift
import CalendarCore
import Foundation

public enum AttendeeMapper {
    public static func read(_ vevent: ICalComponent, selfAddresses: Set<String>) -> (attendees: [Attendee], organizer: Attendee?) {
        let organizer = vevent.property("ORGANIZER").map { property -> Attendee in
            let email = email(of: property)
            return Attendee(name: property.parameter("CN"), email: email, role: .required, response: .accepted,
                            isSelf: isSelf(property.value, email: email, selfAddresses: selfAddresses), isOrganizer: true)
        }
        let attendees = vevent.properties(named: "ATTENDEE").map { property -> Attendee in
            let email = email(of: property)
            return Attendee(
                name: property.parameter("CN"), email: email, role: role(of: property), response: response(of: property),
                isSelf: isSelf(property.value, email: email, selfAddresses: selfAddresses),
                isOrganizer: email != nil && email == organizer?.email)
        }
        return (attendees, organizer)
    }

    /// The address as mail: from the value (`mailto:`), else from the `EMAIL` parameter (RFC 7986).
    static func email(of property: ICalProperty) -> String? {
        CalendarUserAddress.email(from: property.value) ?? CalendarUserAddress.email(from: property.parameter("EMAIL"))
    }

    public static func isSelf(_ address: String, email: String?, selfAddresses: Set<String>) -> Bool {
        if selfAddresses.contains(address.lowercased()) { return true }
        guard let email else { return false }
        return selfAddresses.contains("mailto:" + email)
    }

    static func role(of property: ICalProperty) -> AttendeeRole {
        switch property.parameter("CUTYPE")?.uppercased() {
        case "RESOURCE", "ROOM": return .resource
        default: break
        }
        switch property.parameter("ROLE")?.uppercased() {
        case "OPT-PARTICIPANT", "NON-PARTICIPANT": return .optional
        default: return .required
        }
    }

    static func response(of property: ICalProperty) -> ResponseStatus {
        switch property.parameter("PARTSTAT")?.uppercased() {
        case "ACCEPTED": return .accepted
        case "TENTATIVE": return .tentative
        case "DECLINED": return .declined
        default: return .needsAction
        }
    }

    static func partstat(_ response: ResponseStatus) -> String {
        switch response {
        case .accepted: return "ACCEPTED"
        case .tentative: return "TENTATIVE"
        case .declined: return "DECLINED"
        case .needsAction: return "NEEDS-ACTION"
        }
    }

    /// The same rule as the Google and Microsoft mappers: your attendee entry's response; an organizer who is you with
    /// no attendee entry is accepted; otherwise not invited. nil when the account's own addresses are unknown.
    public static func participation(attendees: [Attendee], organizer: Attendee?, knowsSelf: Bool) -> Participation? {
        guard knowsSelf else { return nil }
        if let me = attendees.first(where: \.isSelf) { return .invited(me.response) }
        if organizer?.isSelf == true { return .invited(.accepted) }
        return .notInvited
    }
}
```

- [ ] **Step 5: Implement `AlarmMapper.swift` (reading; Task 7 adds writing)**

```swift
import CalendarCore
import Foundation

public enum AlarmMapper {
    /// Every readable `VALARM` as a `Reminder`; an alarm with an unreadable trigger is skipped. `isCalendarDefault` is
    /// true for Apple's default alarms (`X-APPLE-DEFAULT-ALARM` or `X-APPLE-LOCAL-DEFAULT-ALARM` set to TRUE) and false
    /// otherwise.
    public static func reminders(from alarms: [ICalComponent]) -> [Reminder] {
        alarms.compactMap(reminder)
    }

    static func reminder(_ alarm: ICalComponent) -> Reminder? {
        guard let triggerProperty = alarm.property("TRIGGER") else { return nil }
        var trigger: ReminderTrigger
        if triggerProperty.parameter("VALUE")?.uppercased() == "DATE-TIME" {
            guard case .utc(let date)? = ICalValues.dateValue(triggerProperty) else { return nil }
            trigger = .absolute(date)
        } else {
            guard let offset = ICalValues.duration(triggerProperty.value) else { return nil }
            trigger = .relative(offset: offset, to: triggerProperty.parameter("RELATED")?.uppercased() == "END" ? .end : .start)
        }
        if let proximity = alarm.property("X-APPLE-PROXIMITY")?.value.uppercased(), proximity == "ARRIVE" || proximity == "DEPART",
           let place = alarm.property("X-APPLE-STRUCTURED-LOCATION") {
            trigger = .location(structuredLocation(place), proximity == "ARRIVE" ? .enter : .leave)
        }
        let type: ReminderType
        switch alarm.property("ACTION")?.value.uppercased() ?? "DISPLAY" {
        case "DISPLAY": type = .display
        case "AUDIO": type = .audio(soundName: alarm.property("ATTACH")?.value)
        case "EMAIL": type = .email(address: alarm.property("ATTENDEE").flatMap { CalendarUserAddress.email(from: $0.value) })
        case "PROCEDURE": type = .procedure(url: alarm.property("ATTACH").flatMap { URL(string: $0.value) })
        case let other: type = .other(other)
        }
        let repeatCount = alarm.property("REPEAT").flatMap { Int($0.value) } ?? 0
        let interval = alarm.property("DURATION").flatMap { ICalValues.duration($0.value) }
        let isDefault = ["X-APPLE-DEFAULT-ALARM", "X-APPLE-LOCAL-DEFAULT-ALARM"].contains {
            alarm.property($0)?.value.uppercased() == "TRUE"
        }
        return Reminder(trigger: trigger, type: type, repeatCount: repeatCount, repeatInterval: repeatCount > 0 ? interval : nil,
                        isCalendarDefault: isDefault)
    }

    /// `geo:lat,long` with `X-TITLE` and `X-APPLE-RADIUS` parameters.
    static func structuredLocation(_ property: ICalProperty) -> StructuredLocation {
        var latitude: Double?, longitude: Double?
        if property.value.lowercased().hasPrefix("geo:") {
            let parts = property.value.dropFirst(4).split(separator: ",")
            if parts.count >= 2 { latitude = Double(parts[0]); longitude = Double(parts[1]) }
        }
        return StructuredLocation(title: property.parameter("X-TITLE"), latitude: latitude, longitude: longitude,
                                  radius: property.parameter("X-APPLE-RADIUS").flatMap(Double.init))
    }
}
```

- [ ] **Step 6: Implement `EventReader.swift`**

```swift
import CalendarCore
import Foundation

public struct EventReadContext: Sendable {
    public var calendarID: String
    /// The resource's last path segment; the series id and the base of every event id.
    public var resourceName: String
    public var etag: String?
    public var sourceID: String?
    /// Floating times and all-day dates are read in this zone.
    public var calendarZone: TimeZone
    /// The account's own calendar-user addresses, lowercased (`mailto:...`, `urn:uuid:...`, principal URLs).
    public var selfAddresses: Set<String>

    public init(calendarID: String, resourceName: String, etag: String?, sourceID: String?, calendarZone: TimeZone, selfAddresses: Set<String>) {
        self.calendarID = calendarID
        self.resourceName = resourceName
        self.etag = etag
        self.sourceID = sourceID
        self.calendarZone = calendarZone
        self.selfAddresses = selfAddresses
    }
}

/// When a `VEVENT` happens. All-day events keep their length in days so an occurrence across a daylight-saving change
/// still ends at midnight.
public struct EventTimingInfo: Sendable, Equatable {
    public var start: Date
    public var end: Date
    public var zone: TimeZone
    public var isAllDay: Bool
    /// Days covered by an all-day event (end exclusive); 0 for timed events.
    public var allDayLength: Int
}

public enum EventReader {
    public static let instanceLimit = 5000

    public static func eventID(resourceName: String, originalStart: Date?, isAllDay: Bool, zone: TimeZone) -> String {
        guard let originalStart else { return resourceName }
        let suffix = isAllDay ? ICalValues.dateText(AllDay.date(of: originalStart, in: zone)) : ICalValues.utcText(originalStart)
        return resourceName + "#" + suffix
    }

    /// Every event of the resource overlapping `window`: expanded occurrences of the master, overrides in place of their
    /// slots (an override moved into the window counts, its slot does not), or the single event.
    public static func events(in resource: EventResource, overlapping window: DateInterval, context: EventReadContext) -> [CalendarEvent] {
        let resolver = resource.resolver
        var result: [CalendarEvent] = []
        var overridden = Set<Date>()
        for vevent in resource.overrides {
            guard let slot = recurrenceID(of: vevent, resolver: resolver, calendarZone: context.calendarZone),
                  let timing = timing(of: vevent, resolver: resolver, calendarZone: context.calendarZone) else { continue }
            overridden.insert(slot)
            if overlaps(timing.start, timing.end, window) {
                result.append(map(vevent, timing: timing, originalStart: slot, recurring: true, context: context, resolver: resolver))
            }
        }
        if let master = resource.master, let timing = timing(of: master, resolver: resolver, calendarZone: context.calendarZone) {
            if let set = recurrenceSet(of: master, zone: timing.zone, isAllDay: timing.isAllDay, resolver: resolver) {
                let expansion = set.occurrences(anchor: timing.start, duration: timing.end.timeIntervalSince(timing.start),
                                                timeZone: timing.zone, isAllDay: timing.isAllDay, overlapping: window, limit: instanceLimit)
                for start in expansion.starts where !overridden.contains(start) {
                    result.append(map(master, timing: moved(timing, to: start), originalStart: start, recurring: true, context: context, resolver: resolver))
                }
            } else if overlaps(timing.start, timing.end, window) {
                result.append(map(master, timing: timing, originalStart: nil, recurring: false, context: context, resolver: resolver))
            }
        }
        return result.sorted { ($0.start, $0.eventID) < ($1.start, $1.eventID) }
    }

    /// The series master in its series form (`eventID` and `seriesID` = the resource name, `originalStart` = its start),
    /// or the single event for a resource that does not recur. nil when the resource has no master.
    public static func masterEvent(of resource: EventResource, context: EventReadContext) -> CalendarEvent? {
        let resolver = resource.resolver
        guard let master = resource.master, let timing = timing(of: master, resolver: resolver, calendarZone: context.calendarZone) else { return nil }
        let recurring = recurrenceSet(of: master, zone: timing.zone, isAllDay: timing.isAllDay, resolver: resolver) != nil
        var event = map(master, timing: timing, originalStart: recurring ? timing.start : nil, recurring: recurring, context: context, resolver: resolver)
        event.eventID = context.resourceName
        return event
    }

    /// The occurrence whose slot is `originalStart`: its override, or the master's instance there. nil when that slot
    /// is excluded or is not an occurrence.
    public static func occurrence(in resource: EventResource, originalStart: Date, context: EventReadContext) -> CalendarEvent? {
        let resolver = resource.resolver
        for vevent in resource.overrides where recurrenceID(of: vevent, resolver: resolver, calendarZone: context.calendarZone) == originalStart {
            guard let timing = timing(of: vevent, resolver: resolver, calendarZone: context.calendarZone) else { return nil }
            return map(vevent, timing: timing, originalStart: originalStart, recurring: true, context: context, resolver: resolver)
        }
        guard let master = resource.master, let timing = timing(of: master, resolver: resolver, calendarZone: context.calendarZone),
              let set = recurrenceSet(of: master, zone: timing.zone, isAllDay: timing.isAllDay, resolver: resolver) else { return nil }
        let probe = DateInterval(start: originalStart, duration: 1)
        let found = set.occurrences(anchor: timing.start, duration: 0, timeZone: timing.zone, isAllDay: timing.isAllDay, overlapping: probe, limit: 2)
        guard found.starts.contains(originalStart) else { return nil }
        return map(master, timing: moved(timing, to: originalStart), originalStart: originalStart, recurring: true, context: context, resolver: resolver)
    }

    public static func series(of resource: EventResource, context: EventReadContext) -> CalendarSeries? {
        let resolver = resource.resolver
        guard let master = resource.master, let timing = timing(of: master, resolver: resolver, calendarZone: context.calendarZone),
              let set = recurrenceSet(of: master, zone: timing.zone, isAllDay: timing.isAllDay, resolver: resolver) else { return nil }
        return CalendarSeries(seriesID: context.resourceName, calendarID: context.calendarID, start: timing.start,
                              timeZone: timing.zone, isAllDay: timing.isAllDay, recurrence: set)
    }

    // MARK: Parts

    public static func timing(of vevent: ICalComponent, resolver: TimeZoneResolver, calendarZone: TimeZone) -> EventTimingInfo? {
        guard let startProperty = vevent.property("DTSTART"), let startValue = ICalValues.dateValue(startProperty) else { return nil }
        if case .date(let first) = startValue {
            var length = 1
            if let endProperty = vevent.property("DTEND"), case .date(let last)? = ICalValues.dateValue(endProperty) {
                length = max(1, first.days(to: last))
            } else if let duration = vevent.property("DURATION").flatMap({ ICalValues.duration($0.value) }), duration >= 86_400 {
                length = Int(duration / 86_400)
            }
            guard let range = AllDay.canonical(first: first, endExclusive: first.adding(days: length), in: calendarZone) else { return nil }
            return EventTimingInfo(start: range.start, end: range.end, zone: calendarZone, isAllDay: true, allDayLength: length)
        }
        guard let start = resolver.date(startValue, floating: calendarZone) else { return nil }
        var end = start
        if let endProperty = vevent.property("DTEND"), let value = ICalValues.dateValue(endProperty),
           let date = resolver.date(value, floating: calendarZone), date >= start {
            end = date
        } else if let duration = vevent.property("DURATION").flatMap({ ICalValues.duration($0.value) }), duration >= 0 {
            end = start.addingTimeInterval(duration)
        }
        return EventTimingInfo(start: start, end: end, zone: resolver.displayZone(startValue, fallback: calendarZone), isAllDay: false, allDayLength: 0)
    }

    public static func recurrenceID(of vevent: ICalComponent, resolver: TimeZoneResolver, calendarZone: TimeZone) -> Date? {
        guard let property = vevent.property("RECURRENCE-ID"), let value = ICalValues.dateValue(property) else { return nil }
        return resolver.date(value, floating: calendarZone)
    }

    /// The master's `RRULE`, `RDATE` and `EXDATE`, with dates resolved by this file's zones; nil when it does not recur.
    /// An `RRULE` that cannot be read is kept in `unparsed` (so the set reports `hasUnreadableRule`).
    public static func recurrenceSet(of master: ICalComponent, zone: TimeZone, isAllDay: Bool, resolver: TimeZoneResolver) -> RecurrenceSet? {
        let ruleProperties = master.properties(named: "RRULE")
        let extra = master.properties(named: "RDATE")
        guard !ruleProperties.isEmpty || !extra.isEmpty else { return nil }
        var rules: [RecurrenceRule] = []
        var unparsed: [String] = []
        for property in ruleProperties {
            if let rule = try? RecurrenceRule(rrule: property.value, in: zone) { rules.append(rule) } else { unparsed.append(ICalSerializer.contentLine(property)) }
        }
        func dates(_ properties: [ICalProperty]) -> [Date] {
            properties.flatMap { ICalValues.dateValues($0).compactMap { resolver.date($0, floating: zone) } }
        }
        return RecurrenceSet(rules: rules, extraDates: dates(extra), excludedDates: dates(master.properties(named: "EXDATE")), unparsed: unparsed)
    }

    private static func moved(_ timing: EventTimingInfo, to start: Date) -> EventTimingInfo {
        var copy = timing
        copy.start = start
        if timing.isAllDay {
            let first = AllDay.date(of: start, in: timing.zone)
            copy.end = AllDay.startOfDay(first.adding(days: timing.allDayLength), in: timing.zone) ?? start.addingTimeInterval(Double(timing.allDayLength) * 86_400)
        } else {
            copy.end = start.addingTimeInterval(timing.end.timeIntervalSince(timing.start))
        }
        return copy
    }

    private static func overlaps(_ start: Date, _ end: Date, _ window: DateInterval) -> Bool {
        end > start ? (start < window.end && end > window.start) : (start >= window.start && start < window.end)
    }

    private static func map(
        _ vevent: ICalComponent, timing: EventTimingInfo, originalStart: Date?, recurring: Bool, context: EventReadContext,
        resolver: TimeZoneResolver
    ) -> CalendarEvent {
        let (attendees, organizer) = AttendeeMapper.read(vevent, selfAddresses: context.selfAddresses)
        let notes = vevent.property("DESCRIPTION")?.text
        let location = vevent.property("LOCATION")?.text
        let url = vevent.property("URL").flatMap { URL(string: $0.value) }
        let status: EventStatus
        switch vevent.property("STATUS")?.value.uppercased() {
        case "CANCELLED": status = .cancelled
        case "TENTATIVE": status = .tentative
        default: status = .confirmed
        }
        let visibility: Visibility
        switch vevent.property("CLASS")?.value.uppercased() {
        case "PUBLIC": visibility = .publicEvent
        case "PRIVATE": visibility = .privateEvent
        case "CONFIDENTIAL": visibility = .confidential
        default: visibility = .default
        }
        func utc(_ name: String) -> Date? {
            guard let property = vevent.property(name), case .utc(let date)? = ICalValues.dateValue(property) else { return nil }
            return date
        }
        return CalendarEvent(
            eventID: eventID(resourceName: context.resourceName, originalStart: recurring ? originalStart : nil, isAllDay: timing.isAllDay, zone: timing.zone),
            uid: vevent.property("UID")?.text, uidScope: .global, calendarID: context.calendarID,
            title: vevent.property("SUMMARY")?.text ?? "", notes: notes?.isEmpty == true ? nil : notes,
            location: location?.isEmpty == true ? nil : location, start: timing.start, end: timing.end, timeZone: timing.zone,
            isAllDay: timing.isAllDay, status: status,
            availability: vevent.property("TRANSP")?.value.uppercased() == "TRANSPARENT" ? .free : .busy,
            visibility: visibility,
            series: recurring ? .occurrence(seriesID: context.resourceName, originalStart: originalStart) : .notRecurring,
            attendees: attendees, organizer: organizer,
            conferences: ConferenceDetector.conferences(location: location, url: url, notes: notes),
            reminders: AlarmMapper.reminders(from: vevent.components(named: "VALARM")),
            url: url, version: context.etag, lastModified: utc("LAST-MODIFIED"), created: utc("CREATED"),
            participation: AttendeeMapper.participation(attendees: attendees, organizer: organizer, knowsSelf: !context.selfAddresses.isEmpty),
            sourceID: context.sourceID)
    }
}
```

`timing(of:)` uses `CalendarDate.days(to:)`. `RuleExpander.daysBetween` is internal to `CalendarCore`, so add this public helper at the end of `Packages/CalendarConnectors/Sources/CalendarCore/Recurrence/RuleExpander.swift`:

```swift
extension CalendarDate {
    /// Whole days from `self` to `other` (negative when `other` is earlier).
    public func days(to other: CalendarDate) -> Int { RuleExpander.daysBetween(self, other) }
}
```


- [ ] **Step 7: Run the tests**

Run: `swift test --package-path Packages/CalendarConnectors --filter "EventReader|AlarmMapper|ICalendar"`
Expected: PASS. (`1_789_491_600` is 2026-09-15T17:00:00Z.)

- [ ] **Step 8: Commit**

```bash
git add Packages/CalendarConnectors/Sources Packages/CalendarConnectors/Tests/ICalendarTests
git commit -m "ICalendar: read VEVENT resources into events with expansion, attendees and alarms"
```

---

### Task 7: Writing events (draft, patch, alarms, attendees)

**Files:**
- Create: `Packages/CalendarConnectors/Sources/ICalendar/EventWriter.swift`
- Modify: `Packages/CalendarConnectors/Sources/ICalendar/AlarmMapper.swift` (writer), `AttendeeMapper.swift` (writer)
- Test: `Packages/CalendarConnectors/Tests/ICalendarTests/EventWriterTests.swift`

**Interfaces:**
- Consumes: Tasks 4-6; `EventDraft`, `EventPatch`, `EventTiming`, `AttendeeDraft`, `WriteError`.
- Produces:
  ```swift
  public enum AlarmMapper { public static func alarms(from reminders: [Reminder]) throws -> [ICalComponent] }  // throws .unsupported([.reminders])
  public enum AttendeeMapper {
      public static func property(for draft: AttendeeDraft) -> ICalProperty
      public static func organizerProperty(address: String) -> ICalProperty
      /// Sets the account's own PARTSTAT (clearing RSVP); false when the account is not an attendee.
      public static func setResponse(_ response: ResponseStatus, in vevent: inout ICalComponent, selfAddresses: Set<String>) -> Bool
  }
  public enum EventWriter {
      public static let productID: String   // "-//TimeTug//CalendarConnectors//EN"
      public static func vevent(from draft: EventDraft, uid: String, now: Date, organizerAddress: String?) throws -> ICalComponent
      public static func resource(for vevent: ICalComponent, zones: [TimeZone], from start: Date, through end: Date) throws -> EventResource
      public static func setTiming(_ timing: EventTiming, on vevent: inout ICalComponent)
      public static func apply(_ patch: EventPatch, to vevent: inout ICalComponent, now: Date, organizerAddress: String?) throws
      public static func touch(_ vevent: inout ICalComponent, now: Date, bumpSequence: Bool)
      public static func dateProperty(_ name: String, _ date: Date, zone: TimeZone, isAllDay: Bool) -> ICalProperty
  }
  ```
  Timed values are written with `TZID=<IANA id>` (UTC with `Z`); all-day values with `VALUE=DATE` in the event's zone; a draft with no zone is written in UTC.

- [ ] **Step 1: Write the failing tests**

```swift
import CalendarCore
import Foundation
import Testing
@testable import ICalendar

private let now = Date(timeIntervalSince1970: 1_790_000_000)

private func draft(_ timing: EventTiming? = nil) -> EventDraft {
    EventDraft(title: "Design review", timing: timing ?? EventTiming(start: laTime(2026, 9, 10, 9), end: laTime(2026, 9, 10, 10), timeZone: la, isAllDay: false),
               notes: "Bring, notes; please", location: "Room 2", availability: .free, visibility: .privateEvent,
               reminders: [.before(minutes: 10)], recurrence: try! RecurrenceRule(rrule: "FREQ=WEEKLY;COUNT=3"))
}

private func readBack(_ vevent: ICalComponent, zones: [TimeZone] = [la]) throws -> [CalendarEvent] {
    let resource = try EventWriter.resource(for: vevent, zones: zones, from: laTime(2026, 9, 1), through: laTime(2027, 9, 1))
    let parsed = try EventResource(data: resource.serialized())
    return EventReader.events(in: parsed, overlapping: DateInterval(start: laTime(2026, 9, 1, 0), end: laTime(2026, 10, 1, 0)), context: context("new.ics"))
}

@Test func draftRoundTripsThroughTheReader() throws {
    let events = try readBack(try EventWriter.vevent(from: draft(), uid: "new-uid", now: now, organizerAddress: nil))
    #expect(events.count == 3)
    let first = events[0]
    #expect(first.uid == "new-uid")
    #expect(first.title == "Design review")
    #expect(first.notes == "Bring, notes; please")
    #expect(first.location == "Room 2")
    #expect(first.start == laTime(2026, 9, 10, 9) && first.end == laTime(2026, 9, 10, 10))
    #expect(first.timeZone.identifier == "America/Los_Angeles")
    #expect(first.availability == .free)
    #expect(first.visibility == .privateEvent)
    #expect(first.reminders == [Reminder(trigger: .relative(offset: -600, to: .start), isCalendarDefault: false)])
}

@Test func writesTZIDAndVTIMEZONE() throws {
    let vevent = try EventWriter.vevent(from: draft(), uid: "u", now: now, organizerAddress: nil)
    #expect(vevent.property("DTSTART")?.parameter("TZID") == "America/Los_Angeles")
    #expect(vevent.property("DTSTART")?.value == "20260910T090000")
    #expect(vevent.property("DTSTAMP")?.value == ICalValues.utcText(now))
    #expect(vevent.property("SEQUENCE")?.value == "0")
    let resource = try EventWriter.resource(for: vevent, zones: [la], from: laTime(2026, 9, 1), through: laTime(2027, 9, 1))
    #expect(resource.calendar.components(named: "VTIMEZONE").first?.property("TZID")?.value == "America/Los_Angeles")
    #expect(resource.calendar.property("PRODID")?.value == EventWriter.productID)
    #expect(resource.calendar.property("VERSION")?.value == "2.0")
}

@Test func utcAndZonelessDraftsUseZ() throws {
    let timing = EventTiming(start: laTime(2026, 9, 10, 9), end: laTime(2026, 9, 10, 10), timeZone: nil, isAllDay: false)
    let vevent = try EventWriter.vevent(from: draft(timing), uid: "u", now: now, organizerAddress: nil)
    #expect(vevent.property("DTSTART")?.value == "20260910T160000Z")
    #expect(vevent.property("DTSTART")?.parameter("TZID") == nil)
}

@Test func allDayDraftUsesValueDate() throws {
    let start = AllDay.startOfDay(CalendarDate(year: 2026, month: 9, day: 10), in: la)!
    let end = AllDay.startOfDay(CalendarDate(year: 2026, month: 9, day: 12), in: la)!
    let vevent = try EventWriter.vevent(from: draft(EventTiming(start: start, end: end, timeZone: la, isAllDay: true)), uid: "u", now: now, organizerAddress: nil)
    #expect(vevent.property("DTSTART")?.parameter("VALUE") == "DATE")
    #expect(vevent.property("DTSTART")?.value == "20260910")
    #expect(vevent.property("DTEND")?.value == "20260912")
    #expect(vevent.property("RRULE")?.value == "FREQ=WEEKLY;COUNT=3")
}

@Test func remindersNilAndEmptyWriteNoAlarmAndUnsupportedOnesThrow() throws {
    var noReminders = draft()
    noReminders.reminders = nil
    #expect(try EventWriter.vevent(from: noReminders, uid: "u", now: now, organizerAddress: nil).components(named: "VALARM").isEmpty)
    noReminders.reminders = []
    #expect(try EventWriter.vevent(from: noReminders, uid: "u", now: now, organizerAddress: nil).components(named: "VALARM").isEmpty)
    #expect(throws: WriteError.unsupported(fields: [.reminders])) { try AlarmMapper.alarms(from: [Reminder(minutesBefore: 5), .before(minutes: 5, type: .email(address: nil))]) }
    #expect(throws: WriteError.unsupported(fields: [.reminders])) {
        try AlarmMapper.alarms(from: [Reminder(trigger: .location(StructuredLocation(title: "x"), .enter))])
    }
}

@Test func alarmsRoundTrip() throws {
    let reminders = [
        Reminder(trigger: .relative(offset: -600, to: .start)),
        Reminder(trigger: .relative(offset: 0, to: .end), type: .audio(soundName: "Chord")),
        Reminder(trigger: .absolute(Date(timeIntervalSince1970: 1_790_521_200)), repeatCount: 2, repeatInterval: 300),
    ]
    let back = AlarmMapper.reminders(from: try AlarmMapper.alarms(from: reminders))
    #expect(Reminder.sameSet(back, reminders))
}

@Test func attendeesWriteWithOrganizer() throws {
    var withGuests = draft()
    withGuests.attendees = [AttendeeDraft(email: "ann@example.test", name: "Ann"), AttendeeDraft(email: "room@example.test", role: .resource)]
    let vevent = try EventWriter.vevent(from: withGuests, uid: "u", now: now, organizerAddress: "mailto:me@icloud.test")
    #expect(vevent.property("ORGANIZER")?.value == "mailto:me@icloud.test")
    let attendees = vevent.properties(named: "ATTENDEE")
    #expect(attendees.map(\.value) == ["mailto:me@icloud.test", "mailto:ann@example.test", "mailto:room@example.test"])
    #expect(attendees[0].parameter("PARTSTAT") == "ACCEPTED")
    #expect(attendees[1].parameter("PARTSTAT") == "NEEDS-ACTION" && attendees[1].parameter("RSVP") == "TRUE" && attendees[1].parameter("CN") == "Ann")
    #expect(attendees[2].parameter("CUTYPE") == "RESOURCE")
}

@Test func setResponseChangesOnlyTheAccountsEntry() throws {
    var vevent = try #require(try resource(weeklySeries).master)
    #expect(AttendeeMapper.setResponse(.declined, in: &vevent, selfAddresses: ["mailto:me@icloud.test"]))
    let entries = vevent.properties(named: "ATTENDEE")
    #expect(entries[0].parameter("PARTSTAT") == "DECLINED" && entries[0].parameter("RSVP") == nil)
    #expect(entries[1].parameter("PARTSTAT") == "TENTATIVE")
    #expect(!AttendeeMapper.setResponse(.accepted, in: &vevent, selfAddresses: ["mailto:someone@else.test"]))
}

@Test func patchChangesOnlyTouchedPropertiesAndKeepsUnknownOnes() throws {
    var vevent = try #require(try resource(weeklySeries).master)
    let before = vevent
    try EventWriter.apply(EventPatch(title: "Renamed", location: .clear), to: &vevent, now: now, organizerAddress: nil)
    #expect(vevent.property("SUMMARY")?.text == "Renamed")
    #expect(vevent.property("LOCATION") == nil)
    #expect(vevent.property("DESCRIPTION") == before.property("DESCRIPTION"))
    #expect(vevent.property("X-APPLE-TRAVEL-ADVISORY-BEHAVIOR") != nil)
    #expect(vevent.property("SEQUENCE")?.value == "2")              // content only: no bump
    #expect(vevent.property("LAST-MODIFIED")?.value == ICalValues.utcText(now))
}

@Test func timingAndAttendeeChangesBumpTheSequence() throws {
    var vevent = try #require(try resource(weeklySeries).master)
    let timing = EventTiming(start: laTime(2026, 9, 1, 11), end: laTime(2026, 9, 1, 12), timeZone: la, isAllDay: false)
    try EventWriter.apply(EventPatch(timing: timing, attendees: AttendeeChanges(add: [AttendeeDraft(email: "new@example.test")], remove: ["ann@example.test"])),
                          to: &vevent, now: now, organizerAddress: "mailto:me@icloud.test")
    #expect(vevent.property("SEQUENCE")?.value == "3")
    #expect(vevent.property("DTSTART")?.value == "20260901T110000")
    #expect(vevent.property("DURATION") == nil)
    let addresses = vevent.properties(named: "ATTENDEE").map(\.value)
    #expect(addresses.contains("mailto:new@example.test"))
    #expect(!addresses.contains { $0.lowercased() == "mailto:ann@example.test" })
}

@Test func clearingRemindersIsUnsupported() throws {
    var vevent = try #require(try resource(weeklySeries).master)
    #expect(throws: WriteError.unsupported(fields: [.reminders])) {
        try EventWriter.apply(EventPatch(reminders: .clear), to: &vevent, now: now, organizerAddress: nil)
    }
}

@Test func recurrencePatchReplacesAndClears() throws {
    var vevent = try #require(try resource(weeklySeries).master)
    try EventWriter.apply(EventPatch(recurrence: .set(try RecurrenceRule(rrule: "FREQ=DAILY;COUNT=2"))), to: &vevent, now: now, organizerAddress: nil)
    #expect(vevent.properties(named: "RRULE").map(\.value) == ["FREQ=DAILY;COUNT=2"])
    try EventWriter.apply(EventPatch(recurrence: .clear), to: &vevent, now: now, organizerAddress: nil)
    #expect(vevent.property("RRULE") == nil && vevent.property("EXDATE") == nil && vevent.property("RDATE") == nil)
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `swift test --package-path Packages/CalendarConnectors --filter EventWriter`
Expected: compile errors.

- [ ] **Step 3: Add the writers to `AlarmMapper` and `AttendeeMapper`**

Append to `AlarmMapper`:

```swift
    /// `VALARM`s for reminders a CalDAV server stores as written: relative to the start or end, or absolute, as a
    /// display or sound alert, optionally repeating. Anything else throws `.unsupported(fields: [.reminders])`.
    public static func alarms(from reminders: [Reminder]) throws -> [ICalComponent] {
        try reminders.map { reminder in
            var properties: [ICalProperty] = []
            switch reminder.type {
            case .display:
                properties += [ICalProperty(name: "ACTION", value: "DISPLAY"), ICalProperty(name: "DESCRIPTION", text: "Reminder")]
            case .audio(let sound):
                properties.append(ICalProperty(name: "ACTION", value: "AUDIO"))
                if let sound { properties.append(ICalProperty(name: "ATTACH", parameters: [ICalParameter("VALUE", "URI")], value: sound)) }
            default:
                throw WriteError.unsupported(fields: [.reminders])
            }
            switch reminder.trigger {
            case .relative(let offset, let anchor):
                properties.append(ICalProperty(name: "TRIGGER", parameters: anchor == .end ? [ICalParameter("RELATED", "END")] : [],
                                               value: ICalValues.durationText(offset)))
            case .absolute(let date):
                properties.append(ICalProperty(name: "TRIGGER", parameters: [ICalParameter("VALUE", "DATE-TIME")], value: ICalValues.utcText(date)))
            case .location:
                throw WriteError.unsupported(fields: [.reminders])
            }
            if reminder.repeatCount > 0, let interval = reminder.repeatInterval {
                properties += [ICalProperty(name: "REPEAT", value: String(reminder.repeatCount)),
                               ICalProperty(name: "DURATION", value: ICalValues.durationText(interval))]
            }
            return ICalComponent(name: "VALARM", properties: properties)
        }
    }
```

Append to `AttendeeMapper`:

```swift
    public static func property(for draft: AttendeeDraft) -> ICalProperty {
        var parameters: [ICalParameter] = []
        if let name = draft.name, !name.isEmpty { parameters.append(ICalParameter("CN", name)) }
        switch draft.role {
        case .required: parameters.append(ICalParameter("ROLE", "REQ-PARTICIPANT"))
        case .optional: parameters.append(ICalParameter("ROLE", "OPT-PARTICIPANT"))
        case .resource: parameters += [ICalParameter("ROLE", "REQ-PARTICIPANT"), ICalParameter("CUTYPE", "RESOURCE")]
        }
        parameters += [ICalParameter("PARTSTAT", "NEEDS-ACTION"), ICalParameter("RSVP", "TRUE")]
        return ICalProperty(name: "ATTENDEE", parameters: parameters, value: "mailto:" + draft.email)
    }

    public static func organizerProperty(address: String) -> ICalProperty {
        ICalProperty(name: "ORGANIZER", value: address)
    }

    /// Sets the account's own `PARTSTAT` and clears `RSVP` on each of its `ATTENDEE` entries. Returns false when the
    /// account is not an attendee.
    public static func setResponse(_ response: ResponseStatus, in vevent: inout ICalComponent, selfAddresses: Set<String>) -> Bool {
        var found = false
        for index in vevent.properties.indices where vevent.properties[index].name == "ATTENDEE" {
            let property = vevent.properties[index]
            guard isSelf(property.value, email: email(of: property), selfAddresses: selfAddresses) else { continue }
            vevent.properties[index].setParameter("PARTSTAT", partstat(response))
            vevent.properties[index].setParameter("RSVP", nil)
            found = true
        }
        return found
    }
```

- [ ] **Step 4: Implement `EventWriter.swift`**

```swift
import CalendarCore
import Foundation

public enum EventWriter {
    public static let productID = "-//TimeTug//CalendarConnectors//EN"
    private static let utc = TimeZone(identifier: "UTC")!

    /// A new `VEVENT`. With attendees, `organizerAddress` (the account's `mailto:` address) becomes the organizer and
    /// its own accepted attendee entry, as iCloud writes it. Fields are validated by the caller (`EventDraft.validate`
    /// and the source's capabilities).
    public static func vevent(from draft: EventDraft, uid: String, now: Date, organizerAddress: String?) throws -> ICalComponent {
        var vevent = ICalComponent(name: "VEVENT", properties: [
            ICalProperty(name: "UID", text: uid),
            ICalProperty(name: "DTSTAMP", value: ICalValues.utcText(now)),
            ICalProperty(name: "CREATED", value: ICalValues.utcText(now)),
            ICalProperty(name: "LAST-MODIFIED", value: ICalValues.utcText(now)),
            ICalProperty(name: "SEQUENCE", value: "0"),
            ICalProperty(name: "SUMMARY", text: draft.title),
        ])
        vevent.setText("DESCRIPTION", draft.notes)
        vevent.setText("LOCATION", draft.location)
        setTiming(draft.timing, on: &vevent)
        vevent.set(ICalProperty(name: "TRANSP", value: draft.availability.closest(in: [.busy, .free]) == .free ? "TRANSPARENT" : "OPAQUE"))
        if let value = classValue(draft.visibility) { vevent.set(ICalProperty(name: "CLASS", value: value)) }
        if let rule = draft.recurrence {
            let zone = draft.timing.timeZone ?? utc
            vevent.set(ICalProperty(name: "RRULE", value: rule.rruleString(allDay: draft.timing.isAllDay, in: zone)))
        }
        if !draft.attendees.isEmpty {
            if let organizerAddress {
                vevent.set(AttendeeMapper.organizerProperty(address: organizerAddress))
                vevent.append(ICalProperty(name: "ATTENDEE", parameters: [ICalParameter("PARTSTAT", "ACCEPTED"), ICalParameter("ROLE", "CHAIR")],
                                           value: organizerAddress))
            }
            for attendee in draft.attendees { vevent.append(AttendeeMapper.property(for: attendee)) }
        }
        vevent.components = try AlarmMapper.alarms(from: draft.reminders ?? [])
        return vevent
    }

    /// A resource holding `vevent` (and any overrides the caller adds later) with a `VTIMEZONE` for each zone.
    public static func resource(for vevent: ICalComponent, zones: [TimeZone], from start: Date, through end: Date) throws -> EventResource {
        var resource = try EventResource(calendar: ICalComponent(name: "VCALENDAR", properties: [
            ICalProperty(name: "VERSION", value: "2.0"),
            ICalProperty(name: "PRODID", text: productID),
            ICalProperty(name: "CALSCALE", value: "GREGORIAN"),
        ], components: [vevent]))
        resource.ensureTimeZones(zones, from: start, through: end)
        return resource
    }

    /// A date property in the form the connector writes: `VALUE=DATE` for all-day, `Z` for UTC, else `TZID`.
    public static func dateProperty(_ name: String, _ date: Date, zone: TimeZone, isAllDay: Bool) -> ICalProperty {
        if isAllDay {
            return ICalProperty(name: name, parameters: [ICalParameter("VALUE", "DATE")], value: ICalValues.dateText(AllDay.date(of: date, in: zone)))
        }
        if zone.identifier == "UTC" || zone.identifier == "GMT" { return ICalProperty(name: name, value: ICalValues.utcText(date)) }
        return ICalProperty(name: name, parameters: [ICalParameter("TZID", zone.identifier)], value: ICalValues.localText(date, in: zone))
    }

    public static func setTiming(_ timing: EventTiming, on vevent: inout ICalComponent) {
        let zone = timing.timeZone ?? utc
        vevent.set(dateProperty("DTSTART", timing.start, zone: zone, isAllDay: timing.isAllDay))
        vevent.set(dateProperty("DTEND", timing.end, zone: zone, isAllDay: timing.isAllDay))
        vevent.removeProperties(named: "DURATION")
    }

    /// Applies a patch whose fields the caller has already checked against the capabilities. Clearing reminders (the
    /// calendar's defaults) cannot be expressed in CalDAV and throws `.unsupported(fields: [.reminders])`. A generated
    /// or removed conference is refused by the caller before this is reached.
    public static func apply(_ patch: EventPatch, to vevent: inout ICalComponent, now: Date, organizerAddress: String?) throws {
        if case .clear = patch.reminders { throw WriteError.unsupported(fields: [.reminders]) }
        if let title = patch.title { vevent.set(ICalProperty(name: "SUMMARY", text: title)) }
        switch patch.notes { case .keep: break; case .set(let text): vevent.setText("DESCRIPTION", text); case .clear: vevent.removeProperties(named: "DESCRIPTION") }
        switch patch.location { case .keep: break; case .set(let text): vevent.setText("LOCATION", text); case .clear: vevent.removeProperties(named: "LOCATION") }
        if let timing = patch.timing { setTiming(timing, on: &vevent) }
        if let availability = patch.availability {
            vevent.set(ICalProperty(name: "TRANSP", value: availability.closest(in: [.busy, .free]) == .free ? "TRANSPARENT" : "OPAQUE"))
        }
        if let visibility = patch.visibility {
            if let value = classValue(visibility) { vevent.set(ICalProperty(name: "CLASS", value: value)) } else { vevent.removeProperties(named: "CLASS") }
        }
        if case .set(let reminders) = patch.reminders {
            vevent.components = vevent.components.filter { $0.name != "VALARM" } + (try AlarmMapper.alarms(from: reminders))
        }
        if let changes = patch.attendees, !changes.isEmpty {
            let removed = Set(changes.remove.map { $0.lowercased() })
            vevent.properties.removeAll { $0.name == "ATTENDEE" && (AttendeeMapper.email(of: $0).map(removed.contains) ?? false) }
            if vevent.property("ORGANIZER") == nil, !changes.add.isEmpty, let organizerAddress {
                vevent.set(AttendeeMapper.organizerProperty(address: organizerAddress))
                vevent.append(ICalProperty(name: "ATTENDEE", parameters: [ICalParameter("PARTSTAT", "ACCEPTED"), ICalParameter("ROLE", "CHAIR")],
                                           value: organizerAddress))
            }
            for draft in changes.add {
                if let index = vevent.properties.firstIndex(where: { $0.name == "ATTENDEE" && AttendeeMapper.email(of: $0) == draft.email }) {
                    var existing = AttendeeMapper.property(for: draft)
                    existing.setParameter("PARTSTAT", vevent.properties[index].parameter("PARTSTAT") ?? "NEEDS-ACTION")
                    vevent.properties[index] = existing
                } else {
                    vevent.append(AttendeeMapper.property(for: draft))
                }
            }
        }
        switch patch.recurrence {
        case .keep: break
        case .set(let rule):
            let start = vevent.property("DTSTART")
            let isAllDay = start?.parameter("VALUE")?.uppercased() == "DATE"
            let zone = start?.parameter("TZID").flatMap { TimeZoneResolver().zone(for: $0) } ?? utc
            vevent.set(ICalProperty(name: "RRULE", value: rule.rruleString(allDay: isAllDay, in: zone)))
        case .clear:
            for name in ["RRULE", "RDATE", "EXDATE", "EXRULE"] { vevent.removeProperties(named: name) }
        }
        let significant = patch.timing != nil || patch.recurrence != .keep || !(patch.attendees?.isEmpty ?? true)
        touch(&vevent, now: now, bumpSequence: significant)
    }

    /// Updates `DTSTAMP` and `LAST-MODIFIED`, and raises `SEQUENCE` for a change attendees must hear about.
    public static func touch(_ vevent: inout ICalComponent, now: Date, bumpSequence: Bool) {
        vevent.set(ICalProperty(name: "DTSTAMP", value: ICalValues.utcText(now)))
        vevent.set(ICalProperty(name: "LAST-MODIFIED", value: ICalValues.utcText(now)))
        if bumpSequence {
            let current = vevent.property("SEQUENCE").flatMap { Int($0.value) } ?? 0
            vevent.set(ICalProperty(name: "SEQUENCE", value: String(current + 1)))
        }
    }

    static func classValue(_ visibility: Visibility) -> String? {
        switch visibility {
        case .default: return nil
        case .publicEvent: return "PUBLIC"
        case .privateEvent: return "PRIVATE"
        case .confidential: return "CONFIDENTIAL"
        }
    }
}
```

`FieldUpdate` must be `Equatable` for `patch.recurrence != .keep`; it already is (`FieldUpdate<Value: Sendable & Equatable>: Sendable, Equatable`).

- [ ] **Step 5: Run the tests**

Run: `swift test --package-path Packages/CalendarConnectors --filter "EventWriter|EventReader|AlarmMapper"`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add Packages/CalendarConnectors/Sources/ICalendar Packages/CalendarConnectors/Tests/ICalendarTests
git commit -m "ICalendar: write drafts and patches, alarms and attendees"
```

---

### Task 8: Series editing (overrides, exclusions, shifts, splits)

**Files:**
- Create: `Packages/CalendarConnectors/Sources/ICalendar/SeriesEditor.swift`
- Test: `Packages/CalendarConnectors/Tests/ICalendarTests/SeriesEditorTests.swift`

**Interfaces:**
- Consumes: Tasks 3-7.
- Produces:
  ```swift
  public enum SeriesEditor {
      /// The override VEVENT for a slot: the existing one, or a new one copied from the master.
      public static func override(in resource: EventResource, at slot: Date, calendarZone: TimeZone) -> ICalComponent?
      public static func setOverride(_ vevent: ICalComponent, at slot: Date, in resource: inout EventResource, calendarZone: TimeZone)
      public static func exclude(_ slot: Date, in resource: inout EventResource, calendarZone: TimeZone) throws
      /// After the master's start moved by `delta`: shift RECURRENCE-IDs, EXDATEs, RDATEs (and unmoved overrides).
      public static func shift(_ resource: inout EventResource, by delta: TimeInterval, calendarZone: TimeZone)
      /// After a rule change: drop overrides and EXDATEs that no longer match an occurrence.
      public static func pruneUnmatched(_ resource: inout EventResource, calendarZone: TimeZone)
      /// Everything before `slot` stays (count or until adjusted); everything from `slot` on moves to a new resource with `newUID`.
      public static func split(_ resource: EventResource, at slot: Date, newUID: String, calendarZone: TimeZone, now: Date) throws -> (head: EventResource, tail: EventResource)
  }
  ```
  Errors: a slot that is not an occurrence throws `WriteError.notFound`; a master with more than one `RRULE` throws `WriteError.unsupported(fields: [.recurrence])`.

- [ ] **Step 1: Write the failing tests**

```swift
import CalendarCore
import Foundation
import Testing
@testable import ICalendar

private let now = Date(timeIntervalSince1970: 1_790_000_000)
private let window = DateInterval(start: laTime(2026, 8, 1, 0), end: laTime(2027, 1, 1, 0))

private func starts(_ r: EventResource) -> [Date] {
    EventReader.events(in: r, overlapping: window, context: context()).map(\.start)
}

@Test func overrideForASlotCopiesTheMaster() throws {
    let r = try resource(weeklySeries)
    let copy = try #require(SeriesEditor.override(in: r, at: laTime(2026, 9, 22), calendarZone: la))
    #expect(copy.property("RECURRENCE-ID")?.value == "20260922T100000")
    #expect(copy.property("RECURRENCE-ID")?.parameter("TZID") == "America/Los_Angeles")
    #expect(copy.property("DTSTART")?.value == "20260922T100000")
    #expect(copy.property("DTEND")?.value == "20260922T103000")
    #expect(copy.property("RRULE") == nil && copy.property("EXDATE") == nil)
    #expect(copy.property("SUMMARY")?.text == "Team sync")
    let existing = try #require(SeriesEditor.override(in: r, at: laTime(2026, 9, 15), calendarZone: la))
    #expect(existing.property("SUMMARY")?.text == "Team sync (moved)")
    #expect(SeriesEditor.override(in: r, at: laTime(2026, 9, 23), calendarZone: la) == nil)
}

@Test func excludeAddsAnExdateAndDropsTheOverride() throws {
    var r = try resource(weeklySeries)
    try SeriesEditor.exclude(laTime(2026, 9, 15), in: &r, calendarZone: la)
    try SeriesEditor.exclude(laTime(2026, 9, 22), in: &r, calendarZone: la)
    #expect(r.overrides.isEmpty)
    #expect(starts(r).filter { $0 < laTime(2026, 10, 1) } == [laTime(2026, 9, 1), laTime(2026, 9, 29)])
    #expect(throws: WriteError.notFound) { try SeriesEditor.exclude(laTime(2026, 9, 23), in: &r, calendarZone: la) }
}

@Test func shiftMovesSlotsExdatesAndUnmovedOverrides() throws {
    var r = try resource(weeklySeries)
    // Move the master one hour later, as an all-in-series timing change does, then shift the rest.
    var master = try #require(r.master)
    EventWriter.setTiming(EventTiming(start: laTime(2026, 9, 1, 11), end: laTime(2026, 9, 1, 11, 30), timeZone: la, isAllDay: false), on: &master)
    r.setEvents([master] + r.overrides)
    SeriesEditor.shift(&r, by: 3600, calendarZone: la)
    let override = try #require(r.overrides.first)
    #expect(override.property("RECURRENCE-ID")?.value == "20260915T110000")
    #expect(override.property("DTSTART")?.value == "20260916T140000")   // it had its own time: kept
    #expect(r.master?.property("EXDATE")?.value == "20260908T110000")
    #expect(starts(r).filter { $0 < laTime(2026, 10, 1) } == [laTime(2026, 9, 1, 11), laTime(2026, 9, 16, 14), laTime(2026, 9, 22, 11), laTime(2026, 9, 29, 11)])
}

@Test func allDayShiftMovesByWholeDaysAcrossDST() throws {
    // A weekly all-day series from Friday 2026-10-30; moving it one day later crosses the 2026-11-01 DST change.
    var r = try resource("""
    BEGIN:VCALENDAR
    VERSION:2.0
    BEGIN:VEVENT
    UID:days
    DTSTART;VALUE=DATE:20261030
    DTEND;VALUE=DATE:20261031
    RRULE:FREQ=WEEKLY
    EXDATE;VALUE=DATE:20261106
    END:VEVENT
    END:VCALENDAR
    """)
    var master = try #require(r.master)
    master.set(ICalProperty(name: "DTSTART", parameters: [ICalParameter("VALUE", "DATE")], value: "20261031"))
    master.set(ICalProperty(name: "DTEND", parameters: [ICalParameter("VALUE", "DATE")], value: "20261101"))
    r.setEvents([master])
    // 25 hours between the two local midnights in Los Angeles: still one day.
    SeriesEditor.shift(&r, by: 90_000, calendarZone: la)
    #expect(r.master?.property("EXDATE")?.value == "20261107")
}

@Test func pruneDropsWhatNoLongerMatches() throws {
    var r = try resource(weeklySeries)
    var master = try #require(r.master)
    master.set(ICalProperty(name: "RRULE", value: "FREQ=WEEKLY;BYDAY=TU;INTERVAL=3"))   // 1st, 22nd: the 8th and 15th are gone
    r.setEvents([master] + r.overrides)
    SeriesEditor.pruneUnmatched(&r, calendarZone: la)
    #expect(r.overrides.isEmpty)
    #expect(r.master?.property("EXDATE") == nil)
}

@Test func splitUntilRule() throws {
    let text = weeklySeries.replacingOccurrences(of: "RRULE:FREQ=WEEKLY;BYDAY=TU", with: "RRULE:FREQ=WEEKLY;BYDAY=TU;UNTIL=20261231T235959Z")
    let (head, tail) = try SeriesEditor.split(try resource(text), at: laTime(2026, 9, 22), newUID: "new-uid", calendarZone: la, now: now)
    #expect(starts(head) == [laTime(2026, 9, 1), laTime(2026, 9, 16, 14)])
    #expect(head.master?.property("RRULE")?.value.contains("UNTIL=20260922T165959Z") == true)
    #expect(tail.uid == "new-uid")
    #expect(tail.master?.property("RRULE")?.value.contains("UNTIL=20261231T235959Z") == true)
    #expect(tail.master?.property("DTSTART")?.value == "20260922T100000")
    #expect(starts(tail).first == laTime(2026, 9, 22))
    #expect(starts(tail).last == laTime(2026, 12, 29))
}

@Test func splitCountRuleKeepsExactCounts() throws {
    let text = weeklySeries.replacingOccurrences(of: "RRULE:FREQ=WEEKLY;BYDAY=TU", with: "RRULE:FREQ=WEEKLY;BYDAY=TU;COUNT=6")
    let (head, tail) = try SeriesEditor.split(try resource(text), at: laTime(2026, 9, 22), newUID: "new-uid", calendarZone: la, now: now)
    // Before the 22nd the rule generated the 1st, 8th (excluded, still counted) and 15th (moved): COUNT=3 stays, 3 move.
    #expect(head.master?.property("RRULE")?.value == "FREQ=WEEKLY;BYDAY=TU;COUNT=3")
    #expect(tail.master?.property("RRULE")?.value == "FREQ=WEEKLY;BYDAY=TU;COUNT=3")
    #expect(starts(tail) == [laTime(2026, 9, 22), laTime(2026, 9, 29), laTime(2026, 10, 6)])
    #expect(head.master?.property("RRULE")?.value.contains("UNTIL") == false)
}

@Test func splitOpenRuleStaysOpenAndMovesLaterExceptions() throws {
    let text = weeklySeries
        .replacingOccurrences(of: "EXDATE;TZID=America/Los_Angeles:20260908T100000", with: "EXDATE;TZID=America/Los_Angeles:20260908T100000,20261006T100000")
    let (head, tail) = try SeriesEditor.split(try resource(text), at: laTime(2026, 9, 15), newUID: "new-uid", calendarZone: la, now: now)
    #expect(head.overrides.isEmpty)                                          // the 15th's override moved
    #expect(head.master?.property("EXDATE")?.value == "20260908T100000")
    #expect(tail.overrides.count == 1)
    #expect(tail.overrides.first?.property("UID")?.text == "new-uid")        // re-stamped
    #expect(tail.master?.property("EXDATE")?.value == "20261006T100000")
    #expect(tail.master?.property("RRULE")?.value == "FREQ=WEEKLY;BYDAY=TU")
    #expect(starts(tail).prefix(3) == [laTime(2026, 9, 16, 14), laTime(2026, 9, 22), laTime(2026, 9, 29)])
}

@Test func splitAtANonSlotIsNotFoundAndMultipleRulesAreUnsupported() throws {
    #expect(throws: WriteError.notFound) {
        try SeriesEditor.split(try resource(weeklySeries), at: laTime(2026, 9, 23), newUID: "x", calendarZone: la, now: now)
    }
    let twoRules = weeklySeries.replacingOccurrences(of: "RRULE:FREQ=WEEKLY;BYDAY=TU", with: "RRULE:FREQ=WEEKLY;BYDAY=TU\nRRULE:FREQ=MONTHLY")
    #expect(throws: WriteError.unsupported(fields: [.recurrence])) {
        try SeriesEditor.split(try resource(twoRules), at: laTime(2026, 9, 22), newUID: "x", calendarZone: la, now: now)
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `swift test --package-path Packages/CalendarConnectors --filter SeriesEditor`
Expected: compile errors.

- [ ] **Step 3: Implement**

```swift
import CalendarCore
import Foundation

/// Edits a recurring resource in place. Every date it writes (`RECURRENCE-ID`, `EXDATE`, `RDATE`, `UNTIL`) uses the
/// master's own `DTSTART` form (the same `TZID`, `Z` or `VALUE=DATE`), so the server and other clients match them.
public enum SeriesEditor {
    struct Form {
        let zone: TimeZone
        let isAllDay: Bool
        let tzid: String?     // nil: UTC with Z (or floating when `floating`)
        let floating: Bool

        func property(_ name: String, _ date: Date) -> ICalProperty {
            if isAllDay {
                return ICalProperty(name: name, parameters: [ICalParameter("VALUE", "DATE")], value: ICalValues.dateText(AllDay.date(of: date, in: zone)))
            }
            if let tzid { return ICalProperty(name: name, parameters: [ICalParameter("TZID", tzid)], value: ICalValues.localText(date, in: zone)) }
            if floating { return ICalProperty(name: name, value: ICalValues.localText(date, in: zone)) }
            return ICalProperty(name: name, value: ICalValues.utcText(date))
        }

        /// One property listing several dates (EXDATE, RDATE).
        func listProperty(_ name: String, _ dates: [Date]) -> ICalProperty {
            let one = property(name, dates[0])
            let values = dates.map { property(name, $0).value }
            return ICalProperty(name: name, parameters: one.parameters, value: values.joined(separator: ","))
        }
    }

    static func form(of master: ICalComponent, resolver: TimeZoneResolver, calendarZone: TimeZone) -> Form? {
        guard let start = master.property("DTSTART"), let value = ICalValues.dateValue(start) else { return nil }
        switch value {
        case .date: return Form(zone: calendarZone, isAllDay: true, tzid: nil, floating: false)
        case .utc: return Form(zone: TimeZone(identifier: "UTC")!, isAllDay: false, tzid: nil, floating: false)
        case .local(_, let tzid?): return Form(zone: resolver.zone(for: tzid) ?? calendarZone, isAllDay: false, tzid: tzid, floating: false)
        case .local(_, nil): return Form(zone: calendarZone, isAllDay: false, tzid: nil, floating: true)
        }
    }

    // MARK: Overrides and exclusions

    public static func override(in resource: EventResource, at slot: Date, calendarZone: TimeZone) -> ICalComponent? {
        let resolver = resource.resolver
        if let existing = resource.overrides.first(where: { EventReader.recurrenceID(of: $0, resolver: resolver, calendarZone: calendarZone) == slot }) {
            return existing
        }
        guard let master = resource.master, let timing = EventReader.timing(of: master, resolver: resolver, calendarZone: calendarZone),
              let form = form(of: master, resolver: resolver, calendarZone: calendarZone),
              isSlot(slot, in: resource, calendarZone: calendarZone) else { return nil }
        var copy = master
        for name in ["RRULE", "RDATE", "EXDATE", "EXRULE"] { copy.removeProperties(named: name) }
        let end: Date
        if timing.isAllDay {
            end = AllDay.startOfDay(AllDay.date(of: slot, in: form.zone).adding(days: timing.allDayLength), in: form.zone) ?? slot
        } else {
            end = slot.addingTimeInterval(timing.end.timeIntervalSince(timing.start))
        }
        copy.set(form.property("DTSTART", slot))
        copy.set(form.property("DTEND", end))
        copy.removeProperties(named: "DURATION")
        copy.set(form.property("RECURRENCE-ID", slot))
        return copy
    }

    public static func setOverride(_ vevent: ICalComponent, at slot: Date, in resource: inout EventResource, calendarZone: TimeZone) {
        let resolver = resource.resolver
        var events = resource.events.filter { $0.property("RECURRENCE-ID") == nil || EventReader.recurrenceID(of: $0, resolver: resolver, calendarZone: calendarZone) != slot }
        events.append(vevent)
        resource.setEvents(events)
    }

    public static func exclude(_ slot: Date, in resource: inout EventResource, calendarZone: TimeZone) throws {
        let resolver = resource.resolver
        guard isSlot(slot, in: resource, calendarZone: calendarZone), var master = resource.master,
              let form = form(of: master, resolver: resolver, calendarZone: calendarZone) else { throw WriteError.notFound }
        var excluded = dates(master.properties(named: "EXDATE"), resolver: resolver, form: form)
        excluded.append(slot)
        master.set(form.listProperty("EXDATE", excluded.sorted()))
        let overrides = resource.overrides.filter { EventReader.recurrenceID(of: $0, resolver: resolver, calendarZone: calendarZone) != slot }
        resource.setEvents([master] + overrides)
    }

    // MARK: Series-wide changes

    public static func shift(_ resource: inout EventResource, by delta: TimeInterval, calendarZone: TimeZone) {
        guard delta != 0, var master = resource.master else { return }
        let resolver = resource.resolver
        guard let form = form(of: master, resolver: resolver, calendarZone: calendarZone) else { return }
        for name in ["EXDATE", "RDATE"] {
            let moved = dates(master.properties(named: name), resolver: resolver, form: form).map { self.moved($0, by: delta, form: form) }
            master.removeProperties(named: name)
            if !moved.isEmpty { master.append(form.listProperty(name, moved.sorted())) }
        }
        let overrides = resource.overrides.map { vevent -> ICalComponent in
            guard let slot = EventReader.recurrenceID(of: vevent, resolver: resolver, calendarZone: calendarZone) else { return vevent }
            var copy = vevent
            let timing = EventReader.timing(of: vevent, resolver: resolver, calendarZone: calendarZone)
            copy.set(form.property("RECURRENCE-ID", moved(slot, by: delta, form: form)))
            // An override still at its slot's time moves with the series; one the user moved keeps its own time.
            if let timing, timing.start == slot {
                copy.set(form.property("DTSTART", moved(timing.start, by: delta, form: form)))
                copy.set(form.property("DTEND", moved(timing.end, by: delta, form: form)))
            }
            return copy
        }
        resource.setEvents([master] + overrides)
    }

    public static func pruneUnmatched(_ resource: inout EventResource, calendarZone: TimeZone) {
        guard var master = resource.master else { return }
        let resolver = resource.resolver
        guard let form = form(of: master, resolver: resolver, calendarZone: calendarZone) else { return }
        // Judge against the rule without exclusions, so an EXDATE still matching an occurrence stays.
        var bare = resource
        var bareMaster = master
        bareMaster.removeProperties(named: "EXDATE")
        bare.setEvents([bareMaster])
        let kept = dates(master.properties(named: "EXDATE"), resolver: resolver, form: form).filter { isSlot($0, in: bare, calendarZone: calendarZone) }
        master.removeProperties(named: "EXDATE")
        if !kept.isEmpty { master.append(form.listProperty("EXDATE", kept.sorted())) }
        let overrides = resource.overrides.filter { vevent in
            EventReader.recurrenceID(of: vevent, resolver: resolver, calendarZone: calendarZone).map { isSlot($0, in: bare, calendarZone: calendarZone) } ?? false
        }
        resource.setEvents([master] + overrides)
    }

    // MARK: Split

    public static func split(
        _ resource: EventResource, at slot: Date, newUID: String, calendarZone: TimeZone, now: Date
    ) throws -> (head: EventResource, tail: EventResource) {
        let resolver = resource.resolver
        guard let master = resource.master, let timing = EventReader.timing(of: master, resolver: resolver, calendarZone: calendarZone),
              let form = form(of: master, resolver: resolver, calendarZone: calendarZone),
              let set = EventReader.recurrenceSet(of: master, zone: form.zone, isAllDay: timing.isAllDay, resolver: resolver)
        else { throw WriteError.notFound }
        guard set.rules.count <= 1, set.unparsed.isEmpty else { throw WriteError.unsupported(fields: [.recurrence]) }
        guard slot > timing.start, isSlotOrOverride(slot, in: resource, calendarZone: calendarZone) else { throw WriteError.notFound }

        var headMaster = master
        var tailMaster = master
        if let rule = set.rules.first {
            var headRule = rule
            var tailRule = rule
            switch rule.end {
            case .count(let total):
                let before = set.ruleInstanceCount(anchor: timing.start, timeZone: form.zone, isAllDay: timing.isAllDay, before: slot)
                guard total - before >= 1 else { throw WriteError.notFound }
                headRule.end = .count(before)
                tailRule.end = .count(total - before)
            case .until, .never:
                headRule.end = .until(timing.isAllDay ? slot.addingTimeInterval(-86_400) : slot.addingTimeInterval(-1))
            }
            headMaster.set(ICalProperty(name: "RRULE", value: headRule.rruleString(allDay: timing.isAllDay, in: form.zone)))
            tailMaster.set(ICalProperty(name: "RRULE", value: tailRule.rruleString(allDay: timing.isAllDay, in: form.zone)))
        }
        for name in ["EXDATE", "RDATE"] {
            let all = dates(master.properties(named: name), resolver: resolver, form: form)
            headMaster.removeProperties(named: name)
            tailMaster.removeProperties(named: name)
            let early = all.filter { $0 < slot }, late = all.filter { $0 >= slot }
            if !early.isEmpty { headMaster.append(form.listProperty(name, early.sorted())) }
            if !late.isEmpty { tailMaster.append(form.listProperty(name, late.sorted())) }
        }
        tailMaster.set(ICalProperty(name: "UID", text: newUID))
        let end: Date = timing.isAllDay
            ? (AllDay.startOfDay(AllDay.date(of: slot, in: form.zone).adding(days: timing.allDayLength), in: form.zone) ?? slot)
            : slot.addingTimeInterval(timing.end.timeIntervalSince(timing.start))
        tailMaster.set(form.property("DTSTART", slot))
        tailMaster.set(form.property("DTEND", end))
        tailMaster.removeProperties(named: "DURATION")
        tailMaster.set(ICalProperty(name: "SEQUENCE", value: "0"))
        tailMaster.set(ICalProperty(name: "CREATED", value: ICalValues.utcText(now)))
        EventWriter.touch(&tailMaster, now: now, bumpSequence: false)
        EventWriter.touch(&headMaster, now: now, bumpSequence: true)

        var headOverrides: [ICalComponent] = [], tailOverrides: [ICalComponent] = []
        for vevent in resource.overrides {
            guard let id = EventReader.recurrenceID(of: vevent, resolver: resolver, calendarZone: calendarZone) else { continue }
            if id < slot { headOverrides.append(vevent) } else {
                var moved = vevent
                moved.set(ICalProperty(name: "UID", text: newUID))
                tailOverrides.append(moved)
            }
        }
        var head = resource
        head.setEvents([headMaster] + headOverrides)
        var tail = resource
        tail.setEvents([tailMaster] + tailOverrides)
        return (head, tail)
    }

    // MARK: Helpers

    /// `date` moved by `delta`; for an all-day form by whole days, so a daylight-saving change between the two dates
    /// cannot land the result on the evening before.
    static func moved(_ date: Date, by delta: TimeInterval, form: Form) -> Date {
        guard form.isAllDay else { return date.addingTimeInterval(delta) }
        let days = Int((delta / 86_400).rounded())
        return AllDay.startOfDay(AllDay.date(of: date, in: form.zone).adding(days: days), in: form.zone) ?? date.addingTimeInterval(delta)
    }

    static func dates(_ properties: [ICalProperty], resolver: TimeZoneResolver, form: Form) -> [Date] {
        properties.flatMap { ICalValues.dateValues($0).compactMap { resolver.date($0, floating: form.zone) } }
    }

    /// Whether `slot` is an occurrence of the master's set (EXDATEs applied).
    static func isSlot(_ slot: Date, in resource: EventResource, calendarZone: TimeZone) -> Bool {
        let resolver = resource.resolver
        guard let master = resource.master, let timing = EventReader.timing(of: master, resolver: resolver, calendarZone: calendarZone),
              let set = EventReader.recurrenceSet(of: master, zone: timing.zone, isAllDay: timing.isAllDay, resolver: resolver) else { return false }
        return set.occurrences(anchor: timing.start, duration: 0, timeZone: timing.zone, isAllDay: timing.isAllDay,
                               overlapping: DateInterval(start: slot, duration: 1), limit: 2).starts.contains(slot)
    }

    /// A slot of the set, or the slot of an existing override (a moved occurrence is still a split point).
    static func isSlotOrOverride(_ slot: Date, in resource: EventResource, calendarZone: TimeZone) -> Bool {
        let resolver = resource.resolver
        return isSlot(slot, in: resource, calendarZone: calendarZone)
            || resource.overrides.contains { EventReader.recurrenceID(of: $0, resolver: resolver, calendarZone: calendarZone) == slot }
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `swift test --package-path Packages/CalendarConnectors --filter "SeriesEditor|EventWriter|EventReader"`
Expected: PASS. If `splitUntilRule` disagrees only on the text of `UNTIL`, check `RecurrenceRule.untilText` (it renders a UTC date-time for timed rules); the instant must be one second before the slot.

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors/Sources/ICalendar/SeriesEditor.swift Packages/CalendarConnectors/Tests/ICalendarTests/SeriesEditorTests.swift
git commit -m "ICalendar: edit series (overrides, exclusions, shifts, exact splits)"
```

---
## Part C: CalDAVCalendar

### Task 9: XML and the WebDAV client

**Files:**
- Modify: `Packages/CalendarConnectors/Package.swift`
- Create: `Packages/CalendarConnectors/Sources/CalDAVCalendar/XMLTree.swift`, `DAVXML.swift`, `WebDAVClient.swift`
- Modify: `scripts/ci/check-architecture.sh`, `scripts/ci/tests/test-check-architecture.sh` (allow `FoundationXML`)
- Test: `Packages/CalendarConnectors/Tests/CalDAVCalendarTests/DAVXMLTests.swift`, `WebDAVClientTests.swift`

**Interfaces:**
- Consumes: `HTTPTransport`, `HTTPRequest`, `HTTPResponse`, `SourceError` (CalendarCore); `ICalValues.utcText` (Task 5).
- Produces:
  ```swift
  struct XMLTree: Sendable, Equatable {
      var namespace: String; var name: String; var attributes: [String: String]; var children: [XMLTree]; var text: String
      static func parse(_ data: Data) throws -> XMLTree            // throws SourceError.invalidResponse
      func child(_ namespace: String, _ name: String) -> XMLTree?
      func children(_ namespace: String, _ name: String) -> [XMLTree]
      func first(_ namespace: String, _ name: String) -> XMLTree?  // depth-first, self included
      var trimmedText: String
  }
  enum DAV { static let dav = "DAV:"; static let caldav = "urn:ietf:params:xml:ns:caldav"; static let calendarServer = "http://calendarserver.org/ns/"; static let apple = "http://apple.com/ns/ical/" }
  struct DAVProperty: Hashable, Sendable { var namespace: String; var name: String }   // + static constants (see Step 4)
  struct DAVResponse: Sendable, Equatable { var href: String; var status: Int?; var properties: [DAVProperty: XMLTree] }
  struct Multistatus: Sendable, Equatable { var responses: [DAVResponse]; var syncToken: String? }
  enum DAVXML {
      static func multistatus(_ data: Data) throws -> Multistatus
      static func propfind(_ properties: [DAVProperty]) -> Data
      static func calendarQuery(from start: Date, to end: Date) -> Data
      static func calendarQuery(uid: String) -> Data
      static func syncCollection(token: String?) -> Data
      static func isInvalidSyncToken(_ response: HTTPResponse) -> Bool
  }
  struct WebDAVCredentials: Sendable, Equatable { var username: String; var password: String }
  struct WebDAVReply: Sendable { var response: HTTPResponse; var url: URL }   // url: after redirects
  struct WebDAVClient: Sendable {
      init(transport: any HTTPTransport, hostBase: String, credentials: @escaping @Sendable () async throws -> WebDAVCredentials)
      static func isAllowed(_ url: URL, hostBase: String) -> Bool
      static func basicAuthorization(_ credentials: WebDAVCredentials) -> String
      func resolve(_ href: String, against base: URL) throws -> URL      // refuses another host
      func send(_ method: String, _ url: URL, headers: [String: String] = [:], body: Data? = nil) async throws -> WebDAVReply
      func propfind(_ url: URL, depth: Int, _ properties: [DAVProperty]) async throws -> (Multistatus, URL)
      func report(_ url: URL, depth: Int, body: Data) async throws -> WebDAVReply
  }
  ```
  `send` returns every status except: 401 throws `SourceError.authExpired`; 429 and 503 throw `SourceError.rateLimited(retryAfter:)` (from `Retry-After` seconds); other 5xx throw `SourceError.server(status:)`. Redirects (301, 302, 303, 307, 308) are followed up to 5 times; 303 becomes a `GET` without a body. `propfind` throws `SourceError.invalidResponse` unless the status is 207.

- [ ] **Step 1: Add the target**

In `Package.swift` add the product `.library(name: "CalDAVCalendar", targets: ["CalDAVCalendar"]),` after `ICalendar`, the target `.target(name: "CalDAVCalendar", dependencies: ["CalendarCore", "ICalendar"]),` and the test target `.testTarget(name: "CalDAVCalendarTests", dependencies: ["CalDAVCalendar", "ICalendar", "CalendarCore", "CalendarTestSupport"]),`.

- [ ] **Step 2: Write the failing tests**

`DAVXMLTests.swift`:

```swift
import CalendarCore
import Foundation
import Testing
@testable import CalDAVCalendar

/// Shaped like iCloud's answer to a depth-1 PROPFIND on a calendar home (scrubbed; uppercase prefixes).
let homeMultistatus = """
<?xml version="1.0" encoding="UTF-8"?>
<multistatus xmlns="DAV:">
 <response>
  <href>/123/calendars/home/</href>
  <propstat>
   <prop>
    <displayname>Home</displayname>
    <resourcetype><collection/><calendar xmlns="urn:ietf:params:xml:ns:caldav"/></resourcetype>
    <calendar-color xmlns="http://apple.com/ns/ical/">#FF2968FF</calendar-color>
    <getctag xmlns="http://calendarserver.org/ns/">HwoQEgwAAA</getctag>
    <sync-token>https://example.test/sync/1</sync-token>
   </prop>
   <status>HTTP/1.1 200 OK</status>
  </propstat>
  <propstat>
   <prop><calendar-timezone xmlns="urn:ietf:params:xml:ns:caldav"/></prop>
   <status>HTTP/1.1 404 Not Found</status>
  </propstat>
 </response>
</multistatus>
"""

@Test func parsesMultistatusWithFoundAndMissingProperties() throws {
    let result = try DAVXML.multistatus(Data(homeMultistatus.utf8))
    let response = try #require(result.responses.first)
    #expect(response.href == "/123/calendars/home/")
    #expect(response.properties[.displayName]?.trimmedText == "Home")
    #expect(response.properties[.calendarColor]?.trimmedText == "#FF2968FF")
    #expect(response.properties[.getCTag]?.trimmedText == "HwoQEgwAAA")
    #expect(response.properties[.syncToken]?.trimmedText == "https://example.test/sync/1")
    #expect(response.properties[.resourceType]?.child(DAV.caldav, "calendar") != nil)
    #expect(response.properties[.calendarTimeZone] == nil)   // 404 propstat is not a value
}

@Test func multistatusIgnoresPrefixes() throws {
    let prefixed = """
    <d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/">
      <d:response><d:href>/cal/a.ics</d:href>
        <d:propstat><d:prop><d:getetag>"7"</d:getetag><c:calendar-data>BEGIN:VCALENDAR</c:calendar-data></d:prop>
        <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
      <d:response><d:href>/cal/gone.ics</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response>
      <d:sync-token>tok-2</d:sync-token>
    </d:multistatus>
    """
    let result = try DAVXML.multistatus(Data(prefixed.utf8))
    #expect(result.responses.map(\.href) == ["/cal/a.ics", "/cal/gone.ics"])
    #expect(result.responses[0].properties[.getETag]?.trimmedText == "\"7\"")
    #expect(result.responses[0].properties[.calendarData]?.text == "BEGIN:VCALENDAR")
    #expect(result.responses[1].status == 404)
    #expect(result.syncToken == "tok-2")
}

@Test func rejectsNonXML() {
    #expect(throws: SourceError.self) { try DAVXML.multistatus(Data("<html><body>oops".utf8)) }
    #expect(throws: SourceError.self) { try DAVXML.multistatus(Data("<other xmlns=\"DAV:\"/>".utf8)) }
}

@Test func buildsRequestBodies() throws {
    let propfind = try XMLTree.parse(DAVXML.propfind([.displayName, .getCTag]))
    #expect(propfind.namespace == DAV.dav && propfind.name == "propfind")
    #expect(propfind.child(DAV.dav, "prop")?.child(DAV.calendarServer, "getctag") != nil)

    let start = Date(timeIntervalSince1970: 1_790_521_200)
    let query = try XMLTree.parse(DAVXML.calendarQuery(from: start, to: start.addingTimeInterval(86_400)))
    #expect(query.name == "calendar-query")
    let range = try #require(query.first(DAV.caldav, "time-range"))
    #expect(range.attributes["start"] == "20260927T150000Z" && range.attributes["end"] == "20260928T150000Z")
    #expect(query.first(DAV.caldav, "calendar-data") != nil && query.first(DAV.dav, "getetag") != nil)

    let byUID = try XMLTree.parse(DAVXML.calendarQuery(uid: "a<b&c"))
    #expect(byUID.first(DAV.caldav, "text-match")?.trimmedText == "a<b&c")

    let sync = try XMLTree.parse(DAVXML.syncCollection(token: nil))
    #expect(sync.name == "sync-collection" && sync.child(DAV.dav, "sync-token")?.trimmedText == "")
    #expect(sync.child(DAV.dav, "sync-level")?.trimmedText == "1")
}

@Test func recognizesAnInvalidSyncToken() {
    let body = "<error xmlns=\"DAV:\"><valid-sync-token/></error>"
    #expect(DAVXML.isInvalidSyncToken(HTTPResponse(status: 403, body: Data(body.utf8))))
    #expect(DAVXML.isInvalidSyncToken(HTTPResponse(status: 409, body: Data(body.utf8))))
    #expect(!DAVXML.isInvalidSyncToken(HTTPResponse(status: 403, body: Data("<error xmlns=\"DAV:\"><need-privileges/></error>".utf8))))
}
```

`WebDAVClientTests.swift`:

```swift
import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import CalDAVCalendar

private let secret = WebDAVCredentials(username: "me@icloud.test", password: "pä:ss wörd")

private func client(_ transport: FakeTransport, base: String = "icloud.com") -> WebDAVClient {
    WebDAVClient(transport: transport, hostBase: base, credentials: { secret })
}

@Test func basicAuthEncodesUTF8AndColons() async throws {
    #expect(WebDAVClient.basicAuthorization(secret) == "Basic " + Data("me@icloud.test:pä:ss wörd".utf8).base64EncodedString())
    let transport = FakeTransport()
    await transport.route("caldav.icloud.com", [HTTPResponse(status: 401)])
    do {
        _ = try await client(transport).send("PROPFIND", URL(string: "https://caldav.icloud.com/")!)
        Issue.record("expected authExpired")
    } catch let error as SourceError {
        #expect(error == .authExpired)
        #expect(!String(describing: error).contains("wörd"))
    }
    let sent = await transport.requests
    #expect(sent.first?.headers["Authorization"] == WebDAVClient.basicAuthorization(secret))
}

@Test func hostRuleMatchesAtALabelBoundary() {
    #expect(WebDAVClient.isAllowed(URL(string: "https://caldav.icloud.com/")!, hostBase: "icloud.com"))
    #expect(WebDAVClient.isAllowed(URL(string: "https://p42-caldav.icloud.com/x")!, hostBase: "icloud.com"))
    #expect(WebDAVClient.isAllowed(URL(string: "https://ICLOUD.com/")!, hostBase: "icloud.com"))
    #expect(!WebDAVClient.isAllowed(URL(string: "https://evilicloud.com/")!, hostBase: "icloud.com"))
    #expect(!WebDAVClient.isAllowed(URL(string: "https://icloud.com.evil.test/")!, hostBase: "icloud.com"))
    #expect(!WebDAVClient.isAllowed(URL(string: "http://caldav.icloud.com/")!, hostBase: "icloud.com"))
    #expect(WebDAVClient.isAllowed(URL(string: "http://localhost:8008/")!, hostBase: "localhost"))
    #expect(WebDAVClient.isAllowed(URL(string: "http://127.0.0.1:8008/")!, hostBase: "127.0.0.1"))
    #expect(!WebDAVClient.isAllowed(URL(string: "ftp://caldav.icloud.com/")!, hostBase: "icloud.com"))
}

@Test func followsRedirectsInsideTheBase() async throws {
    let transport = FakeTransport()
    await transport.route("https://caldav.icloud.com/.well-known/caldav", [HTTPResponse(status: 301, headers: ["Location": "https://p42-caldav.icloud.com/"])])
    await transport.route("https://p42-caldav.icloud.com/", [HTTPResponse(status: 207, body: Data("<multistatus xmlns=\"DAV:\"/>".utf8))])
    let reply = try await client(transport).send("PROPFIND", URL(string: "https://caldav.icloud.com/.well-known/caldav")!,
                                                 headers: ["Depth": "0"], body: Data("<x/>".utf8))
    #expect(reply.response.status == 207)
    #expect(reply.url.absoluteString == "https://p42-caldav.icloud.com/")
    let second = try #require(await transport.requests.last)
    #expect(second.method == "PROPFIND" && second.body == Data("<x/>".utf8) && second.headers["Depth"] == "0")
}

@Test func refusesARedirectToAnotherHostWithoutSendingCredentials() async throws {
    let transport = FakeTransport()
    await transport.route("https://caldav.icloud.com/", [HTTPResponse(status: 302, headers: ["Location": "https://collector.evil.test/steal"])])
    await #expect(throws: SourceError.self) { try await client(transport).send("GET", URL(string: "https://caldav.icloud.com/")!) }
    #expect(await transport.requests(matching: "evil.test").isEmpty)
}

@Test func refusesADowngradeToHTTP() async throws {
    let transport = FakeTransport()
    await transport.route("https://caldav.icloud.com/", [HTTPResponse(status: 301, headers: ["Location": "http://caldav.icloud.com/"])])
    await #expect(throws: SourceError.self) { try await client(transport).send("GET", URL(string: "https://caldav.icloud.com/")!) }
    #expect(await transport.requests.count == 1)
}

@Test func stopsAfterFiveRedirects() async throws {
    let transport = FakeTransport()
    await transport.route("https://caldav.icloud.com/", [HTTPResponse(status: 307, headers: ["Location": "/again"])])
    await #expect(throws: SourceError.self) { try await client(transport).send("GET", URL(string: "https://caldav.icloud.com/")!) }
    #expect(await transport.requests.count == 6)
}

@Test func seeOtherBecomesAGet() async throws {
    let transport = FakeTransport()
    await transport.route("https://caldav.icloud.com/a", [HTTPResponse(status: 303, headers: ["Location": "/b"])])
    await transport.route("https://caldav.icloud.com/b", [HTTPResponse(status: 200)])
    _ = try await client(transport).send("PUT", URL(string: "https://caldav.icloud.com/a")!, body: Data("x".utf8))
    let last = try #require(await transport.requests.last)
    #expect(last.method == "GET" && last.body == nil)
}

@Test func mapsErrorStatuses() async throws {
    let transport = FakeTransport()
    await transport.route("/limited", [HTTPResponse(status: 429, headers: ["Retry-After": "7"])])
    await transport.route("/down", [HTTPResponse(status: 503)])
    await transport.route("/broken", [HTTPResponse(status: 500)])
    await transport.route("/full", [HTTPResponse(status: 507)])
    await transport.route("/missing", [HTTPResponse(status: 404)])
    let c = client(transport)
    func status(_ path: String) async -> SourceError? {
        do { _ = try await c.send("GET", URL(string: "https://caldav.icloud.com" + path)!); return nil } catch { return error as? SourceError }
    }
    #expect(await status("/limited") == .rateLimited(retryAfter: 7))
    #expect(await status("/down") == .rateLimited(retryAfter: nil))
    #expect(await status("/broken") == .server(status: 500))
    #expect(await status("/full") == .server(status: 507))
    #expect(await status("/missing") == nil)   // returned to the caller
}

@Test func refusesToSendCredentialsOverHTTPOrToAnotherHost() async throws {
    let transport = FakeTransport()
    await #expect(throws: SourceError.self) { try await client(transport).send("GET", URL(string: "http://caldav.icloud.com/")!) }
    await #expect(throws: SourceError.self) { try await client(transport).send("GET", URL(string: "https://example.test/")!) }
    #expect(await transport.requests.isEmpty)
}

@Test func resolvesHrefsAndRefusesForeignOnes() throws {
    let c = client(FakeTransport())
    let base = URL(string: "https://p42-caldav.icloud.com/123/calendars/")!
    #expect(try c.resolve("/123/calendars/home/", against: base).absoluteString == "https://p42-caldav.icloud.com/123/calendars/home/")
    #expect(try c.resolve("home/a%20b.ics", against: base).absoluteString == "https://p42-caldav.icloud.com/123/calendars/home/a%20b.ics")
    #expect(try c.resolve("https://p07-caldav.icloud.com/123/principal/", against: base).host == "p07-caldav.icloud.com")
    #expect(throws: SourceError.self) { try c.resolve("https://evil.test/123/", against: base) }
}
```

- [ ] **Step 3: Run to verify they fail**

Run: `swift test --package-path Packages/CalendarConnectors --filter "DAVXML|WebDAVClient"`
Expected: compile errors.

- [ ] **Step 4: Implement `XMLTree.swift` and `DAVXML.swift`**

`XMLTree.swift`:

```swift
import CalendarCore
import Foundation
#if canImport(FoundationXML)
import FoundationXML
#endif

/// A namespace-resolved XML element: enough DOM for WebDAV multistatus bodies.
struct XMLTree: Sendable, Equatable {
    var namespace: String
    var name: String
    var attributes: [String: String] = [:]
    var children: [XMLTree] = []
    /// The element's own character data (CDATA included), children's text excluded.
    var text: String = ""

    var trimmedText: String { text.trimmingCharacters(in: .whitespacesAndNewlines) }

    func child(_ namespace: String, _ name: String) -> XMLTree? {
        children.first { $0.namespace == namespace && $0.name == name }
    }

    func children(_ namespace: String, _ name: String) -> [XMLTree] {
        children.filter { $0.namespace == namespace && $0.name == name }
    }

    /// Depth-first, `self` included.
    func first(_ namespace: String, _ name: String) -> XMLTree? {
        if self.namespace == namespace && self.name == name { return self }
        for child in children { if let found = child.first(namespace, name) { return found } }
        return nil
    }

    static func parse(_ data: Data) throws -> XMLTree {
        let builder = Builder()
        let parser = XMLParser(data: data)
        parser.shouldProcessNamespaces = true
        parser.shouldResolveExternalEntities = false
        parser.delegate = builder
        guard parser.parse(), let root = builder.root else {
            throw SourceError.invalidResponse("the server sent XML that could not be read")
        }
        return root
    }

    private final class Builder: NSObject, XMLParserDelegate {
        var stack: [XMLTree] = []
        var root: XMLTree?

        func parser(_ parser: XMLParser, didStartElement elementName: String, namespaceURI: String?, qualifiedName: String?,
                    attributes: [String: String] = [:]) {
            stack.append(XMLTree(namespace: namespaceURI ?? "", name: elementName, attributes: attributes))
        }

        func parser(_ parser: XMLParser, foundCharacters string: String) {
            if !stack.isEmpty { stack[stack.count - 1].text += string }
        }

        func parser(_ parser: XMLParser, foundCDATA CDATABlock: Data) {
            if !stack.isEmpty { stack[stack.count - 1].text += String(decoding: CDATABlock, as: UTF8.self) }
        }

        func parser(_ parser: XMLParser, didEndElement elementName: String, namespaceURI: String?, qualifiedName: String?) {
            guard let done = stack.popLast() else { return }
            if stack.isEmpty { root = done } else { stack[stack.count - 1].children.append(done) }
        }
    }
}
```

`DAVXML.swift`:

```swift
import CalendarCore
import Foundation
import ICalendar

enum DAV {
    static let dav = "DAV:"
    static let caldav = "urn:ietf:params:xml:ns:caldav"
    static let calendarServer = "http://calendarserver.org/ns/"
    static let apple = "http://apple.com/ns/ical/"
}

struct DAVProperty: Hashable, Sendable {
    var namespace: String
    var name: String

    static let currentUserPrincipal = DAVProperty(namespace: DAV.dav, name: "current-user-principal")
    static let calendarHomeSet = DAVProperty(namespace: DAV.caldav, name: "calendar-home-set")
    static let calendarUserAddressSet = DAVProperty(namespace: DAV.caldav, name: "calendar-user-address-set")
    static let scheduleInboxURL = DAVProperty(namespace: DAV.caldav, name: "schedule-inbox-URL")
    static let scheduleDefaultCalendarURL = DAVProperty(namespace: DAV.caldav, name: "schedule-default-calendar-URL")
    static let resourceType = DAVProperty(namespace: DAV.dav, name: "resourcetype")
    static let displayName = DAVProperty(namespace: DAV.dav, name: "displayname")
    static let supportedComponents = DAVProperty(namespace: DAV.caldav, name: "supported-calendar-component-set")
    static let privileges = DAVProperty(namespace: DAV.dav, name: "current-user-privilege-set")
    static let calendarTimeZone = DAVProperty(namespace: DAV.caldav, name: "calendar-timezone")
    static let calendarColor = DAVProperty(namespace: DAV.apple, name: "calendar-color")
    static let getCTag = DAVProperty(namespace: DAV.calendarServer, name: "getctag")
    static let syncToken = DAVProperty(namespace: DAV.dav, name: "sync-token")
    static let getETag = DAVProperty(namespace: DAV.dav, name: "getetag")
    static let calendarData = DAVProperty(namespace: DAV.caldav, name: "calendar-data")
}

struct DAVResponse: Sendable, Equatable {
    var href: String
    /// The response-level status (a `sync-collection` removal is 404); nil when the response has propstats.
    var status: Int?
    /// Properties from `200` propstats only.
    var properties: [DAVProperty: XMLTree]
}

struct Multistatus: Sendable, Equatable {
    var responses: [DAVResponse]
    var syncToken: String?
}

enum DAVXML {
    static func multistatus(_ data: Data) throws -> Multistatus {
        let root = try XMLTree.parse(data)
        guard root.namespace == DAV.dav, root.name == "multistatus" else {
            throw SourceError.invalidResponse("expected a multistatus body")
        }
        var responses: [DAVResponse] = []
        for response in root.children(DAV.dav, "response") {
            guard let href = response.child(DAV.dav, "href")?.trimmedText, !href.isEmpty else { continue }
            var properties: [DAVProperty: XMLTree] = [:]
            for propstat in response.children(DAV.dav, "propstat") where status(propstat.child(DAV.dav, "status")) == 200 {
                for property in propstat.child(DAV.dav, "prop")?.children ?? [] {
                    properties[DAVProperty(namespace: property.namespace, name: property.name)] = property
                }
            }
            responses.append(DAVResponse(href: href, status: status(response.child(DAV.dav, "status")), properties: properties))
        }
        return Multistatus(responses: responses, syncToken: root.child(DAV.dav, "sync-token")?.trimmedText)
    }

    /// `HTTP/1.1 200 OK` → 200.
    static func status(_ element: XMLTree?) -> Int? {
        guard let parts = element?.trimmedText.split(separator: " "), parts.count >= 2 else { return nil }
        return Int(parts[1])
    }

    static func propfind(_ properties: [DAVProperty]) -> Data {
        document("<d:propfind \(namespaces)><d:prop>\(properties.map(element).joined())</d:prop></d:propfind>")
    }

    static func calendarQuery(from start: Date, to end: Date) -> Data {
        document("""
        <c:calendar-query \(namespaces)><d:prop><d:getetag/><c:calendar-data/></d:prop>\
        <c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT">\
        <c:time-range start="\(ICalValues.utcText(start))" end="\(ICalValues.utcText(end))"/>\
        </c:comp-filter></c:comp-filter></c:filter></c:calendar-query>
        """)
    }

    static func calendarQuery(uid: String) -> Data {
        document("""
        <c:calendar-query \(namespaces)><d:prop><d:getetag/><c:calendar-data/></d:prop>\
        <c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT"><c:prop-filter name="UID">\
        <c:text-match collation="i;octet">\(escape(uid))</c:text-match>\
        </c:prop-filter></c:comp-filter></c:comp-filter></c:filter></c:calendar-query>
        """)
    }

    static func syncCollection(token: String?) -> Data {
        document("""
        <d:sync-collection \(namespaces)><d:sync-token>\(escape(token ?? ""))</d:sync-token><d:sync-level>1</d:sync-level>\
        <d:prop><d:getetag/></d:prop></d:sync-collection>
        """)
    }

    /// RFC 6578: a token the server no longer accepts is a 403 (or 409 on some servers) with `DAV:valid-sync-token`.
    static func isInvalidSyncToken(_ response: HTTPResponse) -> Bool {
        guard response.status == 403 || response.status == 409 else { return false }
        return (try? XMLTree.parse(response.body))?.first(DAV.dav, "valid-sync-token") != nil
    }

    private static let namespaces = "xmlns:d=\"DAV:\" xmlns:c=\"\(DAV.caldav)\" xmlns:cs=\"\(DAV.calendarServer)\" xmlns:a=\"\(DAV.apple)\""

    private static func element(_ property: DAVProperty) -> String {
        let prefix: String
        switch property.namespace {
        case DAV.dav: prefix = "d"
        case DAV.caldav: prefix = "c"
        case DAV.calendarServer: prefix = "cs"
        case DAV.apple: prefix = "a"
        default: return "<x:\(property.name) xmlns:x=\"\(escape(property.namespace))\"/>"
        }
        return "<\(prefix):\(property.name)/>"
    }

    private static func document(_ body: String) -> Data { Data(("<?xml version=\"1.0\" encoding=\"UTF-8\"?>" + body).utf8) }

    static func escape(_ text: String) -> String {
        text.replacingOccurrences(of: "&", with: "&amp;").replacingOccurrences(of: "<", with: "&lt;")
            .replacingOccurrences(of: ">", with: "&gt;").replacingOccurrences(of: "\"", with: "&quot;")
    }
}
```

- [ ] **Step 5: Implement `WebDAVClient.swift`**

```swift
import CalendarCore
import Foundation

struct WebDAVCredentials: Sendable, Equatable {
    var username: String
    var password: String
}

struct WebDAVReply: Sendable {
    var response: HTTPResponse
    /// The URL that answered, after redirects: relative hrefs in the body resolve against it.
    var url: URL
}

/// HTTP for one CalDAV account. Adds Basic auth to every request, only over HTTPS (plain HTTP only to the loopback
/// host) and only to hosts inside `hostBase`, and follows redirects itself under the same rule (the transport must not
/// follow them: `URLSessionTransport(followsRedirects: false)`). The password never appears in an error.
struct WebDAVClient: Sendable {
    static let maxRedirects = 5
    private static let loopback: Set<String> = ["localhost", "127.0.0.1"]

    let transport: any HTTPTransport
    let hostBase: String
    /// Read per request, so a sign-in again (new secrets in the store) reaches a source that already exists.
    let credentials: @Sendable () async throws -> WebDAVCredentials

    init(transport: any HTTPTransport, hostBase: String, credentials: @escaping @Sendable () async throws -> WebDAVCredentials) {
        self.transport = transport
        self.hostBase = hostBase.lowercased()
        self.credentials = credentials
    }

    /// `https` to `hostBase` or a subdomain of it (matched at a label boundary); `http` only to the loopback host.
    static func isAllowed(_ url: URL, hostBase: String) -> Bool {
        guard let host = url.host?.lowercased(), let scheme = url.scheme?.lowercased() else { return false }
        let base = hostBase.lowercased()
        guard host == base || host.hasSuffix("." + base) else { return false }
        switch scheme {
        case "https": return true
        case "http": return loopback.contains(host)
        default: return false
        }
    }

    static func basicAuthorization(_ credentials: WebDAVCredentials) -> String {
        "Basic " + Data("\(credentials.username):\(credentials.password)".utf8).base64EncodedString()
    }

    /// An href from a response, resolved against the URL that sent it. One that points outside `hostBase` is refused.
    func resolve(_ href: String, against base: URL) throws -> URL {
        guard let url = URL(string: href, relativeTo: base)?.absoluteURL, Self.isAllowed(url, hostBase: hostBase) else {
            throw SourceError.invalidResponse("the server pointed to another host")
        }
        return url
    }

    func send(_ method: String, _ url: URL, headers: [String: String] = [:], body: Data? = nil) async throws -> WebDAVReply {
        var method = method, url = url, body = body, headers = headers
        for _ in 0...Self.maxRedirects {
            guard Self.isAllowed(url, hostBase: hostBase) else {
                throw SourceError.invalidResponse("refused to send credentials to \(url.scheme ?? "?")://\(url.host ?? "?")")
            }
            var request = HTTPRequest(url: url, method: method, headers: headers, body: body)
            request.headers["Authorization"] = Self.basicAuthorization(try await credentials())
            if body != nil, request.headers["Content-Type"] == nil { request.headers["Content-Type"] = "application/xml; charset=utf-8" }
            let response = try await transport.send(request)
            switch response.status {
            case 301, 302, 303, 307, 308:
                guard let location = response.header("Location"), let next = URL(string: location, relativeTo: url)?.absoluteURL else {
                    throw SourceError.invalidResponse("a redirect without a location")
                }
                guard Self.isAllowed(next, hostBase: hostBase) else {
                    throw SourceError.invalidResponse("the server redirected to another host")
                }
                if response.status == 303 {
                    method = "GET"
                    body = nil
                    headers["Content-Type"] = nil
                }
                url = next
            case 401:
                throw SourceError.authExpired
            case 429, 503:
                throw SourceError.rateLimited(retryAfter: response.header("Retry-After").flatMap(TimeInterval.init))
            case 500...599:
                throw SourceError.server(status: response.status)
            default:
                return WebDAVReply(response: response, url: url)
            }
        }
        throw SourceError.invalidResponse("too many redirects")
    }

    /// A `PROPFIND` that must answer 207; returns the multistatus and the URL that answered.
    func propfind(_ url: URL, depth: Int, _ properties: [DAVProperty]) async throws -> (Multistatus, URL) {
        let reply = try await send("PROPFIND", url, headers: ["Depth": String(depth)], body: DAVXML.propfind(properties))
        guard reply.response.status == 207 else {
            throw SourceError.invalidResponse("PROPFIND answered \(reply.response.status)")
        }
        return (try DAVXML.multistatus(reply.response.body), reply.url)
    }

    func report(_ url: URL, depth: Int, body: Data) async throws -> WebDAVReply {
        try await send("REPORT", url, headers: ["Depth": String(depth)], body: body)
    }
}
```

A rate limit is not retried here: the source's `ChangeMonitor` already backs off, and a user-driven read reports it.

- [ ] **Step 6: Allow `FoundationXML` in the architecture check**

`scripts/ci/check-architecture.sh` lists the modules the connector library may import, and `import FoundationXML` in
`XMLTree.swift` is not on it yet (the `#if` does not hide the line from the check). In the script, change the
`only CalendarConnectors` line to:

```bash
only CalendarConnectors "Foundation FoundationNetworking FoundationXML Testing $LIB" $P/CalendarConnectors/Sources $P/CalendarConnectors/Tests
```

In `scripts/ci/tests/test-check-architecture.sh`, make the clean fixture use it (after the `FoundationNetworking` fixture line):

```bash
printf '#if canImport(FoundationXML)\nimport FoundationXML\n#endif\nimport CalendarCore\n' \
  > "$T/Packages/CalendarConnectors/Sources/CalendarOAuth/X.swift"
```

and keep a refusal next to the existing `Security` one, so the list stays closed:

```bash
expect Packages/CalendarConnectors/Sources/CalendarCore/X.swift 'import XMLCoder\n' "CalendarConnectors may not import XMLCoder"
```

Run: `scripts/ci/check-architecture.sh && bash scripts/ci/tests/test-check-architecture.sh`
Expected: both exit 0 (the check prints nothing).

- [ ] **Step 7: Run the tests**

Run: `swift test --package-path Packages/CalendarConnectors --filter "DAVXML|WebDAVClient"`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add Packages/CalendarConnectors/Package.swift Packages/CalendarConnectors/Sources/CalDAVCalendar Packages/CalendarConnectors/Tests/CalDAVCalendarTests scripts/ci/check-architecture.sh scripts/ci/tests/test-check-architecture.sh
git commit -m "CalDAVCalendar: XML tree, multistatus and request bodies, WebDAV client with the host rule"
```

---
### Task 10: The fake server, account config and discovery

**Files:**
- Create: `Packages/CalendarConnectors/Sources/CalDAVCalendar/CalDAVAccountConfig.swift`, `CalDAVDiscovery.swift`
- Create: `Packages/CalendarConnectors/Tests/CalDAVCalendarTests/FakeCalDAVServer.swift` (test helper, used by Tasks 10-14)
- Test: `Packages/CalendarConnectors/Tests/CalDAVCalendarTests/DiscoveryTests.swift`

**Interfaces:**
- Consumes: `WebDAVClient`, `DAVXML`, `DAVProperty`, `XMLTree` (Task 9).
- Produces:
  ```swift
  struct CalDAVAccountConfig: Sendable, Equatable {
      var serverURL: URL; var username: String; var principalURL: URL; var homeURL: URL
      var userAddresses: [String]    // lowercased
      var autoSchedule: Bool
      init(serverURL:username:principalURL:homeURL:userAddresses:autoSchedule:)
      init(config: [String: String]) throws   // SourceError.invalidResponse when a key is missing
      var config: [String: String]
  }
  struct CalDAVDiscovery: Sendable {
      let client: WebDAVClient
      func discover(serverURL: URL, username: String) async throws -> CalDAVAccountConfig
  }
  actor FakeCalDAVServer: HTTPTransport   // test target; see Step 1
  ```

- [ ] **Step 1: Write the fake server**

`FakeCalDAVServer.swift`:

```swift
import CalendarCore
import CalendarTestSupport
import Foundation
@testable import CalDAVCalendar

/// An in-memory CalDAV server with iCloud-shaped paths, behind `HTTPTransport`. It ignores the host and answers by
/// method and path, keeps ETags, ctags and sync tokens, checks `If-Match`/`If-None-Match`, and can be told to fail a
/// request. A `calendar-query` returns every resource of the calendar (the source filters by the window itself).
actor FakeCalDAVServer: HTTPTransport {
    struct Resource: Sendable { var body: String; var etag: String; var revision: Int }

    struct Collection: Sendable {
        var displayName: String
        var color: String? = "#1BADF8FF"
        /// nil: the server does not say (all components).
        var components: [String]? = ["VEVENT"]
        var privileges: [String] = ["read", "write"]
        var timeZoneID: String? = "America/Los_Angeles"
        var subscribed = false
        var resources: [String: Resource] = [:]
        var removed: [String: Int] = [:]
        var revision = 1
    }

    private struct Failure { var method: String; var pathContains: String; var status: Int; var remaining: Int; var skip: Int }

    static let principalPath = "/123/principal/"
    static let homePath = "/123/calendars/"
    static let inboxPath = "/123/inbox/"

    var username = "me@icloud.test"
    var password = "app-pass-1234"
    var userAddresses = ["mailto:me@icloud.test", "urn:uuid:11111111-2222-3333-4444-555555555555"]
    var autoSchedule = true
    var supportsSync = true
    var sendsETagOnPut = true
    var defaultCalendar: String? = "home"
    /// Where `/.well-known/caldav` redirects; nil answers 404.
    var wellKnownLocation: String? = "/"
    var collections: [String: Collection] = ["home": Collection(displayName: "Home")]
    private var revision = 1
    private var oldestValidToken = 0
    private var failures: [Failure] = []
    private(set) var log: [HTTPRequest] = []

    func configure(_ change: @Sendable (isolated FakeCalDAVServer) -> Void) { change(self) }

    // MARK: Test helpers

    /// A change made by someone else (another client).
    func store(_ calendar: String, _ name: String, _ ics: String) {
        revision += 1
        collections[calendar, default: Collection(displayName: calendar)].resources[name] =
            Resource(body: Self.crlf(ics), etag: "\"e\(revision)\"", revision: revision)
        collections[calendar]!.removed[name] = nil
        collections[calendar]!.revision = revision
    }

    func remove(_ calendar: String, _ name: String) {
        revision += 1
        collections[calendar]?.resources[name] = nil
        collections[calendar]?.removed[name] = revision
        collections[calendar]?.revision = revision
    }

    func body(_ calendar: String, _ name: String) -> String? { collections[calendar]?.resources[name]?.body }
    func etag(_ calendar: String, _ name: String) -> String? { collections[calendar]?.resources[name]?.etag }
    func names(_ calendar: String) -> [String] { (collections[calendar]?.resources.keys).map { $0.sorted() } ?? [] }
    func requests(_ method: String) -> [HTTPRequest] { log.filter { $0.method == method } }
    func clearLog() { log = [] }

    /// After letting `after` matching requests through, the next `times` requests with this method whose path contains
    /// `pathContains` answer `status`.
    func fail(_ method: String, pathContains: String, status: Int, times: Int = 1, after: Int = 0) {
        failures.append(Failure(method: method, pathContains: pathContains, status: status, remaining: times, skip: after))
    }

    /// Every sync token issued so far stops being accepted.
    func expireSyncTokens() {
        revision += 1
        oldestValidToken = revision
        for key in collections.keys { collections[key]!.revision = revision }
    }

    static func crlf(_ text: String) -> String {
        text.replacingOccurrences(of: "\r\n", with: "\n").replacingOccurrences(of: "\n", with: "\r\n")
    }

    // MARK: HTTPTransport

    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        log.append(request)
        guard request.headers["Authorization"] == WebDAVClient.basicAuthorization(WebDAVCredentials(username: username, password: password))
        else { return HTTPResponse(status: 401) }
        let raw = (URLComponents(url: request.url, resolvingAgainstBaseURL: false)?.percentEncodedPath ?? "").removingPercentEncoding ?? ""
        let path = raw.isEmpty ? "/" : raw
        if let index = failures.firstIndex(where: { $0.method == request.method && path.contains($0.pathContains) && $0.remaining > 0 }) {
            if failures[index].skip > 0 {
                failures[index].skip -= 1
            } else {
                failures[index].remaining -= 1
                return HTTPResponse(status: failures[index].status)
            }
        }
        if path.hasSuffix("/.well-known/caldav") {
            guard let wellKnownLocation else { return HTTPResponse(status: 404) }
            return HTTPResponse(status: 301, headers: ["Location": wellKnownLocation])
        }
        if request.method == "OPTIONS" {
            return HTTPResponse(status: 200, headers: ["DAV": "1, 2, 3, access-control, calendar-access" + (autoSchedule ? ", calendar-auto-schedule" : "")])
        }
        switch (request.method, path) {
        case ("PROPFIND", "/"):
            return multistatus([response("/", ["<d:current-user-principal><d:href>\(Self.principalPath)</d:href></d:current-user-principal>"])])
        case ("PROPFIND", Self.principalPath):
            let addresses = userAddresses.map { "<d:href>\(DAVXML.escape($0))</d:href>" }.joined()
            return multistatus([response(Self.principalPath, [
                "<c:calendar-home-set><d:href>\(Self.homePath)</d:href></c:calendar-home-set>",
                "<c:calendar-user-address-set>\(addresses)</c:calendar-user-address-set>",
                "<c:schedule-inbox-URL><d:href>\(Self.inboxPath)</d:href></c:schedule-inbox-URL>",
            ])])
        case ("PROPFIND", Self.inboxPath):
            let props = defaultCalendar.map {
                ["<c:schedule-default-calendar-URL><d:href>\(Self.homePath)\($0)/</d:href></c:schedule-default-calendar-URL>"]
            } ?? []
            return multistatus([response(Self.inboxPath, props)])
        case ("PROPFIND", Self.homePath):
            return multistatus(homeResponses())
        default:
            break
        }
        guard path.hasPrefix(Self.homePath) else { return HTTPResponse(status: 404) }
        let parts = path.dropFirst(Self.homePath.count).split(separator: "/").map(String.init)
        guard let calendar = parts.first, collections[calendar] != nil else { return HTTPResponse(status: 404) }
        if parts.count == 1 {
            switch request.method {
            case "REPORT": return report(calendar, request)
            case "PROPFIND": return multistatus([collectionResponse(calendar)])
            default: return HTTPResponse(status: 405)
            }
        }
        let name = parts[1]
        let existing = collections[calendar]!.resources[name]
        switch request.method {
        case "GET":
            guard let existing else { return HTTPResponse(status: 404) }
            return HTTPResponse(status: 200, headers: ["ETag": existing.etag, "Content-Type": "text/calendar"], body: Data(existing.body.utf8))
        case "PUT":
            guard collections[calendar]!.privileges.contains("write") else { return HTTPResponse(status: 403) }
            if request.headers["If-None-Match"] == "*", existing != nil { return HTTPResponse(status: 412) }
            if let match = request.headers["If-Match"], match != existing?.etag { return HTTPResponse(status: 412) }
            revision += 1
            let etag = "\"e\(revision)\""
            collections[calendar]!.resources[name] = Resource(body: String(decoding: request.body ?? Data(), as: UTF8.self), etag: etag, revision: revision)
            collections[calendar]!.removed[name] = nil
            collections[calendar]!.revision = revision
            return HTTPResponse(status: existing == nil ? 201 : 204, headers: sendsETagOnPut ? ["ETag": etag] : [:])
        case "DELETE":
            guard collections[calendar]!.privileges.contains("write") else { return HTTPResponse(status: 403) }
            guard let existing else { return HTTPResponse(status: 404) }
            if let match = request.headers["If-Match"], match != existing.etag { return HTTPResponse(status: 412) }
            remove(calendar, name)
            return HTTPResponse(status: 204)
        default:
            return HTTPResponse(status: 405)
        }
    }

    // MARK: Bodies

    private func token(_ revision: Int) -> String { "https://fake.test/sync/\(revision)" }
    private func href(_ calendar: String, _ name: String? = nil) -> String { Self.homePath + calendar + "/" + (name ?? "") }

    private func response(_ href: String, _ props: [String]) -> String {
        "<d:response><d:href>\(href)</d:href><d:propstat><d:prop>\(props.joined())</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"
    }

    private func multistatus(_ responses: [String], syncToken: String? = nil) -> HTTPResponse {
        let token = syncToken.map { "<d:sync-token>\($0)</d:sync-token>" } ?? ""
        let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><d:multistatus xmlns:d=\"DAV:\" xmlns:c=\"\(DAV.caldav)\" "
            + "xmlns:cs=\"\(DAV.calendarServer)\" xmlns:a=\"\(DAV.apple)\">\(responses.joined())\(token)</d:multistatus>"
        return HTTPResponse(status: 207, headers: ["Content-Type": "application/xml"], body: Data(xml.utf8))
    }

    private func homeResponses() -> [String] {
        var responses = [
            response(Self.homePath, ["<d:resourcetype><d:collection/></d:resourcetype>"]),
            response(Self.inboxPath, ["<d:resourcetype><d:collection/><c:schedule-inbox/></d:resourcetype>"]),
            response(Self.homePath + "outbox/", ["<d:resourcetype><d:collection/><c:schedule-outbox/></d:resourcetype>"]),
            response(Self.homePath + "notification/", ["<d:resourcetype><d:collection/><cs:notification/></d:resourcetype>"]),
        ]
        responses += collections.keys.sorted().map(collectionResponse)
        return responses
    }

    private func collectionResponse(_ name: String) -> String {
        let c = collections[name]!
        var props = [
            "<d:resourcetype><d:collection/><c:calendar/>\(c.subscribed ? "<cs:subscribed/>" : "")</d:resourcetype>",
            "<d:displayname>\(DAVXML.escape(c.displayName))</d:displayname>",
            "<cs:getctag>ctag-\(c.revision)</cs:getctag>",
            "<d:current-user-privilege-set>\(c.privileges.map { "<d:privilege><d:\($0)/></d:privilege>" }.joined())</d:current-user-privilege-set>",
        ]
        if let color = c.color { props.append("<a:calendar-color>\(color)</a:calendar-color>") }
        if let components = c.components {
            props.append("<c:supported-calendar-component-set>\(components.map { "<c:comp name=\"\($0)\"/>" }.joined())</c:supported-calendar-component-set>")
        }
        if supportsSync { props.append("<d:sync-token>\(token(c.revision))</d:sync-token>") }
        if let zone = c.timeZoneID {
            let vcalendar = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTIMEZONE\r\nTZID:\(zone)\r\nEND:VTIMEZONE\r\nEND:VCALENDAR\r\n"
            props.append("<c:calendar-timezone>\(DAVXML.escape(vcalendar))</c:calendar-timezone>")
        }
        return response(href(name), props)
    }

    private func report(_ calendar: String, _ request: HTTPRequest) -> HTTPResponse {
        guard let body = request.body, let root = try? XMLTree.parse(body) else { return HTTPResponse(status: 400) }
        let c = collections[calendar]!
        if root.name == "sync-collection" {
            let invalid = HTTPResponse(status: 403, body: Data("<d:error xmlns:d=\"DAV:\"><d:valid-sync-token/></d:error>".utf8))
            guard supportsSync else { return HTTPResponse(status: 403) }
            var since = 0
            let text = root.child(DAV.dav, "sync-token")?.trimmedText ?? ""
            if !text.isEmpty {
                guard let value = text.split(separator: "/").last.flatMap({ Int($0) }), value >= oldestValidToken, value <= revision else { return invalid }
                since = value
            }
            var responses = c.resources.filter { $0.value.revision > since }.sorted { $0.key < $1.key }
                .map { response(href(calendar, $0.key), ["<d:getetag>\(DAVXML.escape($0.value.etag))</d:getetag>"]) }
            responses += c.removed.filter { $0.value > since }.keys.sorted()
                .map { "<d:response><d:href>\(href(calendar, $0))</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response>" }
            return multistatus(responses, syncToken: token(c.revision))
        }
        let uid = root.first(DAV.caldav, "text-match")?.trimmedText
        let matching = c.resources.filter { entry in
            guard let uid else { return true }
            return entry.value.body.components(separatedBy: "\r\n").contains("UID:" + uid)
        }.sorted { $0.key < $1.key }
        return multistatus(matching.map {
            response(href(calendar, $0.key), [
                "<d:getetag>\(DAVXML.escape($0.value.etag))</d:getetag>",
                "<c:calendar-data>\(DAVXML.escape($0.value.body))</c:calendar-data>",
            ])
        })
    }
}

/// Hands out `uuid-1`, `uuid-2`, ... so tests know the names and UIDs a write will use.
final class UUIDSequence: @unchecked Sendable {
    private let lock = NSLock()
    private var value = 0
    func next() -> String { lock.withLock { value += 1; return "uuid-\(value)" } }
}

let fakeServerURL = URL(string: "https://caldav.icloud.com")!
let fakeHomeURL = URL(string: "https://caldav.icloud.com/123/calendars/")!
```

- [ ] **Step 2: Write the failing tests**

`DiscoveryTests.swift`:

```swift
import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import CalDAVCalendar

private func discovery(_ transport: any HTTPTransport, password: String = "app-pass-1234", base: String = "icloud.com") -> CalDAVDiscovery {
    CalDAVDiscovery(client: WebDAVClient(transport: transport, hostBase: base,
                                         credentials: { WebDAVCredentials(username: "me@icloud.test", password: password) }))
}

@Test func discoversAnICloudShapedAccount() async throws {
    let server = FakeCalDAVServer()
    let account = try await discovery(server).discover(serverURL: fakeServerURL, username: "me@icloud.test")
    #expect(account.principalURL.absoluteString == "https://caldav.icloud.com/123/principal/")
    #expect(account.homeURL == fakeHomeURL)
    #expect(account.userAddresses == ["mailto:me@icloud.test", "urn:uuid:11111111-2222-3333-4444-555555555555"])
    #expect(account.autoSchedule)
    #expect(account.username == "me@icloud.test")
    #expect(account.serverURL == fakeServerURL)
    let methods = await server.log.map(\.method)
    #expect(methods == ["PROPFIND", "PROPFIND", "PROPFIND", "OPTIONS"])
}

@Test func followsAPartitionHostRedirect() async throws {
    let server = FakeCalDAVServer()
    await server.configure { $0.wellKnownLocation = "https://p42-caldav.icloud.com/" }
    let account = try await discovery(server).discover(serverURL: fakeServerURL, username: "me@icloud.test")
    #expect(account.principalURL.host == "p42-caldav.icloud.com")
    #expect(account.homeURL.absoluteString == "https://p42-caldav.icloud.com/123/calendars/")
}

@Test func refusesARedirectToAForeignHost() async throws {
    let server = FakeCalDAVServer()
    await server.configure { $0.wellKnownLocation = "https://collector.evil.test/" }
    await #expect(throws: SourceError.self) { try await discovery(server).discover(serverURL: fakeServerURL, username: "me@icloud.test") }
    #expect(await server.log.allSatisfy { $0.url.host != "collector.evil.test" })
}

@Test func fallsBackToTheServerURLWithoutWellKnown() async throws {
    let server = FakeCalDAVServer()
    await server.configure { $0.wellKnownLocation = nil }
    let account = try await discovery(server).discover(serverURL: fakeServerURL, username: "me@icloud.test")
    #expect(account.homeURL == fakeHomeURL)
}

@Test func wrongPasswordIsAuthExpired() async throws {
    await #expect(throws: SourceError.authExpired) {
        try await discovery(FakeCalDAVServer(), password: "wrong").discover(serverURL: fakeServerURL, username: "me@icloud.test")
    }
}

@Test func noAutoScheduleIsRecorded() async throws {
    let server = FakeCalDAVServer()
    await server.configure { $0.autoSchedule = false }
    #expect(try await discovery(server).discover(serverURL: fakeServerURL, username: "me@icloud.test").autoSchedule == false)
}

@Test func missingPrincipalIsInvalid() async throws {
    let transport = FakeTransport()
    await transport.route("caldav.icloud.com", [HTTPResponse(status: 207, body: Data("<multistatus xmlns=\"DAV:\"/>".utf8))])
    await #expect(throws: SourceError.self) { try await discovery(transport).discover(serverURL: fakeServerURL, username: "me@icloud.test") }
}

@Test func configRoundTripsAndRejectsMissingKeys() throws {
    let account = CalDAVAccountConfig(
        serverURL: fakeServerURL, username: "me@icloud.test", principalURL: URL(string: "https://caldav.icloud.com/123/principal/")!,
        homeURL: fakeHomeURL, userAddresses: ["mailto:me@icloud.test", "urn:uuid:1"], autoSchedule: true)
    #expect(try CalDAVAccountConfig(config: account.config) == account)
    #expect(account.config["userAddresses"] == "mailto:me@icloud.test\nurn:uuid:1")
    #expect(account.config["autoSchedule"] == "true")
    #expect(account.config.values.allSatisfy { !$0.contains("app-pass") })
    var broken = account.config
    broken["homeURL"] = nil
    #expect(throws: SourceError.self) { try CalDAVAccountConfig(config: broken) }
}
```

- [ ] **Step 3: Run to verify they fail**

Run: `swift test --package-path Packages/CalendarConnectors --filter Discovery`
Expected: compile errors (`CalDAVDiscovery`, `CalDAVAccountConfig` missing).

- [ ] **Step 4: Implement `CalDAVAccountConfig.swift`**

```swift
import CalendarCore
import Foundation

/// What sign-in learned about a CalDAV account, kept in `Connection.config` (never a password).
struct CalDAVAccountConfig: Sendable, Equatable {
    var serverURL: URL
    var username: String
    var principalURL: URL
    var homeURL: URL
    /// The principal's `calendar-user-address-set`, lowercased: how the account appears as organizer or attendee.
    var userAddresses: [String]
    /// The server schedules invitations itself (`calendar-auto-schedule`, RFC 6638).
    var autoSchedule: Bool

    init(serverURL: URL, username: String, principalURL: URL, homeURL: URL, userAddresses: [String], autoSchedule: Bool) {
        self.serverURL = serverURL
        self.username = username
        self.principalURL = principalURL
        self.homeURL = homeURL
        self.userAddresses = userAddresses
        self.autoSchedule = autoSchedule
    }

    init(config: [String: String]) throws {
        func url(_ key: String) throws -> URL {
            guard let text = config[key], let url = URL(string: text) else {
                throw SourceError.invalidResponse("the account settings are incomplete; sign in again")
            }
            return url
        }
        guard let username = config["username"] else { throw SourceError.invalidResponse("the account settings are incomplete; sign in again") }
        self.init(
            serverURL: try url("serverURL"), username: username, principalURL: try url("principalURL"), homeURL: try url("homeURL"),
            userAddresses: (config["userAddresses"] ?? "").split(separator: "\n").map(String.init),
            autoSchedule: config["autoSchedule"] == "true")
    }

    var config: [String: String] {
        [
            "serverURL": serverURL.absoluteString, "username": username, "principalURL": principalURL.absoluteString,
            "homeURL": homeURL.absoluteString, "userAddresses": userAddresses.joined(separator: "\n"),
            "autoSchedule": autoSchedule ? "true" : "false",
        ]
    }
}
```

- [ ] **Step 5: Implement `CalDAVDiscovery.swift`**

```swift
import CalendarCore
import Foundation

/// RFC 6764 and RFC 4791 discovery: `/.well-known/caldav` (or the server URL itself), the current user's principal,
/// its calendar home and addresses, and whether the server schedules invitations.
struct CalDAVDiscovery: Sendable {
    let client: WebDAVClient

    func discover(serverURL: URL, username: String) async throws -> CalDAVAccountConfig {
        let principal = try await principalURL(serverURL: serverURL)
        let (status, answeredBy) = try await client.propfind(principal, depth: 0, [.calendarHomeSet, .calendarUserAddressSet])
        let properties = status.responses.first?.properties ?? [:]
        guard let homeHref = properties[.calendarHomeSet]?.child(DAV.dav, "href")?.trimmedText, !homeHref.isEmpty else {
            throw SourceError.invalidResponse("the server did not name a calendar home")
        }
        var home = try client.resolve(homeHref, against: answeredBy)
        if !home.absoluteString.hasSuffix("/") { home = URL(string: home.absoluteString + "/") ?? home }
        let addresses = (properties[.calendarUserAddressSet]?.children(DAV.dav, "href") ?? [])
            .map { $0.trimmedText.lowercased() }.filter { !$0.isEmpty }
        let options = try await client.send("OPTIONS", home)
        let dav = options.response.header("DAV") ?? ""
        let autoSchedule = dav.split(separator: ",").contains { $0.trimmingCharacters(in: .whitespaces).lowercased() == "calendar-auto-schedule" }
        return CalDAVAccountConfig(serverURL: serverURL, username: username, principalURL: principal, homeURL: home,
                                   userAddresses: addresses, autoSchedule: autoSchedule)
    }

    /// The well-known URL first; a server without it (404, 405, 501) is asked at the URL the user gave.
    private func principalURL(serverURL: URL) async throws -> URL {
        let wellKnown = serverURL.appendingPathComponent(".well-known/caldav")
        for candidate in [wellKnown, serverURL] {
            let reply = try await client.send("PROPFIND", candidate, headers: ["Depth": "0"], body: DAVXML.propfind([.currentUserPrincipal]))
            switch reply.response.status {
            case 207:
                let status = try DAVXML.multistatus(reply.response.body)
                guard let href = status.responses.first?.properties[.currentUserPrincipal]?.child(DAV.dav, "href")?.trimmedText,
                      !href.isEmpty else {
                    throw SourceError.invalidResponse("the server did not name a principal")
                }
                return try client.resolve(href, against: reply.url)
            case let status where candidate == wellKnown && [404, 405, 501].contains(status):
                continue
            default:
                throw SourceError.invalidResponse("the server answered \(reply.response.status) to discovery")
            }
        }
        throw SourceError.invalidResponse("the server did not name a principal")
    }
}
```

- [ ] **Step 6: Run the tests**

Run: `swift test --package-path Packages/CalendarConnectors --filter Discovery`
Expected: PASS. `missingPrincipalIsInvalid` routes every request to an empty multistatus, so the well-known answer already fails.

- [ ] **Step 7: Commit**

```bash
git add Packages/CalendarConnectors/Sources/CalDAVCalendar Packages/CalendarConnectors/Tests/CalDAVCalendarTests
git commit -m "CalDAVCalendar: account config, discovery and an in-memory CalDAV server for tests"
```

---
### Task 11: The source (reads, series) and the two connector kinds

**Files:**
- Create: `Packages/CalendarConnectors/Sources/CalDAVCalendar/CalDAVCalendarSource.swift`, `CalDAVCalendarSource+Series.swift`, `CalDAVConnectorKinds.swift`
- Test: `Packages/CalendarConnectors/Tests/CalDAVCalendarTests/Harness.swift`, `SourceReadTests.swift`, `ConnectorKindTests.swift`

**Interfaces:**
- Consumes: Tasks 6, 9, 10; `CredentialHelp`, `CredentialPromptHelp`, `URLSessionTransport(followsRedirects:)` (Task 1).
- Produces:
  ```swift
  struct CalDAVCalendarInfo: Sendable { var descriptor: CalendarDescriptor; var url: URL; var ctag: String?; var syncToken: String?; var zone: TimeZone }
  actor CalDAVSourceState {
      var calendars: [CalDAVCalendarInfo]? { get }
      func store(calendars: [CalDAVCalendarInfo])
      func resource(at url: URL, etag: String) -> EventResource?
      func remember(_ resource: EventResource, at url: URL, etag: String)
      func forget(_ url: URL)
  }
  public final class CalDAVCalendarSource: PollingCalendarSource, SeriesSource {
      init(connection: Connection, account: CalDAVAccountConfig, client: WebDAVClient, provider: CalendarProvider,
           syncState: any SyncStateStore, monitor: ChangeMonitor, now: @escaping @Sendable () -> Date,
           defaultZone: TimeZone = .current, makeUUID: @escaping @Sendable () -> String = { UUID().uuidString })
      let connection, account, client, provider, syncState, monitor, now, defaultZone, makeUUID, state
      var selfAddresses: Set<String> { get }; var organizerAddress: String? { get }
      func calendarURL(_ calendarID: String) -> URL
      func resourceURL(calendarID: String, name: String) -> URL
      func calendarID(for url: URL) -> String?
      func loadCalendars() async throws -> [CalDAVCalendarInfo]
      func calendarZone(_ calendarID: String) async throws -> TimeZone
      func context(calendarID: String, zone: TimeZone, resourceName: String, etag: String?) -> EventReadContext
      func calendarObject(_ response: DAVResponse, at url: URL) async -> (resource: EventResource, etag: String)?
  }
  public struct ICloudConnectorKind: ConnectorKind, CredentialPromptHelp   // kindID "icloud"
  public struct CalDAVConnectorKind: ConnectorKind, CredentialPromptHelp   // kindID "caldav"
  struct CalDAVAccountSetup: Sendable { ...; static func serverURL(from text: String) throws -> URL }
  ```
  Calendar ids are the collection path below the home URL, without the trailing slash (`home`, `A1B2-...`), so they survive a move to another partition host.

- [ ] **Step 1: Write the test harness and fixtures**

`Harness.swift`:

```swift
import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import CalDAVCalendar

let pacific = TimeZone(identifier: "America/Los_Angeles")!
/// Calendars without `calendar-timezone` fall back to this zone in the harness (a different one, so tests can tell).
let harnessDefaultZone = TimeZone(identifier: "Europe/Berlin")!
let september = DateInterval(start: pt(2026, 9, 1, 0), end: pt(2026, 10, 1, 0))

func pt(_ y: Int, _ mo: Int, _ d: Int, _ h: Int = 10, _ mi: Int = 0) -> Date {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = pacific
    return calendar.date(from: DateComponents(year: y, month: mo, day: d, hour: h, minute: mi))!
}

private func calendarFile(_ events: [[String]]) -> String {
    (["BEGIN:VCALENDAR", "VERSION:2.0", "PRODID:-//Apple Inc.//iCloud//EN"] + events.flatMap { $0 } + ["END:VCALENDAR"])
        .joined(separator: "\r\n") + "\r\n"
}

/// A single timed event in Los Angeles on 2026-09-10 09:00-10:00, with an Apple property the model does not cover.
func singleICS(uid: String = "single-uid", title: String = "Dentist", attendees: [String] = []) -> String {
    calendarFile([["BEGIN:VEVENT", "UID:\(uid)", "DTSTAMP:20260901T000000Z", "SEQUENCE:0", "SUMMARY:\(title)",
                   "DTSTART;TZID=America/Los_Angeles:20260910T090000", "DTEND;TZID=America/Los_Angeles:20260910T100000"]
                  + attendees + ["X-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC", "END:VEVENT"]])
}

/// A weekly Tuesday 10:00-10:30 Los Angeles series from 2026-09-01: the 8th excluded, the 15th moved to Wednesday the
/// 16th at 14:00. With an organizer, the account is invited (NEEDS-ACTION); pass `organizer: nil` for no attendees.
func weeklyICS(uid: String = "weekly-uid", rule: String = "FREQ=WEEKLY;BYDAY=TU", organizer: String? = "mailto:boss@example.test") -> String {
    let people = organizer.map {
        ["ORGANIZER;CN=Boss:\($0)", "ATTENDEE;CN=Boss;PARTSTAT=ACCEPTED:\($0)",
         "ATTENDEE;CN=Me;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:me@icloud.test"]
    } ?? []
    let master = ["BEGIN:VEVENT", "UID:\(uid)", "DTSTAMP:20260901T000000Z", "SEQUENCE:0", "SUMMARY:Team sync",
                  "DTSTART;TZID=America/Los_Angeles:20260901T100000", "DTEND;TZID=America/Los_Angeles:20260901T103000",
                  "RRULE:\(rule)", "EXDATE;TZID=America/Los_Angeles:20260908T100000"]
        + people + ["X-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC", "END:VEVENT"]
    let moved = ["BEGIN:VEVENT", "UID:\(uid)", "DTSTAMP:20260901T000000Z", "RECURRENCE-ID;TZID=America/Los_Angeles:20260915T100000",
                 "SUMMARY:Team sync (moved)", "DTSTART;TZID=America/Los_Angeles:20260916T140000",
                 "DTEND;TZID=America/Los_Angeles:20260916T143000"] + people + ["END:VEVENT"]
    return calendarFile([master, moved])
}

struct CalDAVHarness {
    let server = FakeCalDAVServer()
    let credentials = InMemoryCredentialStore()
    let sync = InMemorySyncStateStore()
    let uuids = UUIDSequence()
    let now = TestNow(Date(timeIntervalSince1970: 1_790_000_000))
    static let defaultAddresses = ["mailto:me@icloud.test", "urn:uuid:11111111-2222-3333-4444-555555555555"]

    func account(autoSchedule: Bool = true, userAddresses: [String] = defaultAddresses) -> CalDAVAccountConfig {
        CalDAVAccountConfig(serverURL: fakeServerURL, username: "me@icloud.test",
                            principalURL: URL(string: "https://caldav.icloud.com/123/principal/")!, homeURL: fakeHomeURL,
                            userAddresses: userAddresses, autoSchedule: autoSchedule)
    }

    func source(autoSchedule: Bool = true, userAddresses: [String] = defaultAddresses) async throws -> CalDAVCalendarSource {
        try await credentials.setSecrets(["username": "me@icloud.test", "password": "app-pass-1234"], for: "c1")
        let account = account(autoSchedule: autoSchedule, userAddresses: userAddresses)
        let connection = Connection(kindID: "icloud", connectionID: "c1", displayName: "me@icloud.test", config: account.config)
        let store = credentials
        let client = WebDAVClient(transport: server, hostBase: "icloud.com", credentials: {
            let secrets = try await store.secrets(for: "c1") ?? [:]
            return WebDAVCredentials(username: secrets["username"] ?? "", password: secrets["password"] ?? "")
        })
        let uuids = uuids
        return CalDAVCalendarSource(
            connection: connection, account: account, client: client, provider: .iCloud, syncState: sync,
            monitor: ChangeMonitor(interval: .seconds(60), sleep: { _ in }), now: now.provider,
            defaultZone: harnessDefaultZone, makeUUID: { uuids.next() })
    }
}

/// Answers `promptCredentials` with fixed values and records the fields it was shown.
final class StubInteraction: AuthorizationInteraction, @unchecked Sendable {
    private let lock = NSLock()
    private var answers: [[String: String]]
    private(set) var shown: [[CredentialField]] = []

    init(_ answers: [String: String]...) { self.answers = answers }

    func beginOAuthRedirect() async throws -> any OAuthRedirectSession { throw SourceError.invalidResponse("not an OAuth kind") }

    func promptCredentials(_ fields: [CredentialField]) async throws -> [String: String] {
        try lock.withLock {
            shown.append(fields)
            guard !answers.isEmpty else { throw CancellationError() }
            return answers.removeFirst()
        }
    }
}
```

- [ ] **Step 2: Write the failing read tests**

`SourceReadTests.swift`:

```swift
import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import CalDAVCalendar

private func addCollections(_ server: FakeCalDAVServer) async {
    await server.configure {
        $0.collections["tasks"] = .init(displayName: "Reminders", components: ["VTODO"])
        $0.collections["shared"] = .init(displayName: "Shared", color: nil, privileges: ["read"], timeZoneID: nil)
        $0.collections["holidays"] = .init(displayName: "Holidays", privileges: ["read"], subscribed: true)
    }
}

@Test func listsEventCalendarsOnly() async throws {
    let h = CalDAVHarness()
    await addCollections(h.server)
    let source = try await h.source()
    let calendars = try await source.calendars()
    #expect(calendars.map(\.id) == ["holidays", "home", "shared"])
    let home = try #require(calendars.first { $0.id == "home" })
    #expect(home.title == "Home" && home.colorHex == "#1BADF8")
    #expect(home.permissions.canEdit && home.permissions.canViewDetails)
    #expect(home.isDefault == true)
    #expect(home.timeZone?.identifier == "America/Los_Angeles")
    #expect(home.service == .calDAV && home.provider == .iCloud && home.kind == .standard)
    #expect(home.accountName == "me@icloud.test")
    #expect(home.supportedAvailabilities == [.busy, .free])
    let shared = try #require(calendars.first { $0.id == "shared" })
    #expect(!shared.permissions.canEdit && shared.isDefault == false && shared.colorHex == nil)
    #expect(shared.timeZone == harnessDefaultZone)
    let holidays = try #require(calendars.first { $0.id == "holidays" })
    #expect(holidays.kind == .subscribed && !holidays.permissions.canEdit)
    for calendar in calendars {
        #expect(ProvidedFieldsConformance.violations(calendar: calendar, capabilities: source.capabilities) == [])
    }
}

@Test func defaultIsNilWhenTheServerDoesNotSay() async throws {
    let h = CalDAVHarness()
    await h.server.configure { $0.defaultCalendar = nil }
    #expect(try await h.source().calendars().allSatisfy { $0.isDefault == nil })
}

@Test func readsEventsFromEveryVisibleCalendar() async throws {
    let h = CalDAVHarness()
    await addCollections(h.server)
    await h.server.store("home", "single.ics", singleICS())
    await h.server.store("shared", "weekly.ics", weeklyICS())
    let source = try await h.source()
    let events = try await source.events(in: september)
    #expect(events.map(\.title) == ["Team sync", "Dentist", "Team sync (moved)", "Team sync", "Team sync"])
    let dentist = try #require(events.first { $0.title == "Dentist" })
    #expect(dentist.eventID == "single.ics" && dentist.calendarID == "home" && dentist.series == .notRecurring)
    let etag = await h.server.etag("home", "single.ics")
    #expect(dentist.version == etag)
    #expect(dentist.sourceID == "icloud-c1")
    let moved = try #require(events.first { $0.title == "Team sync (moved)" })
    #expect(moved.calendarID == "shared" && moved.series == .occurrence(seriesID: "weekly.ics", originalStart: pt(2026, 9, 15)))
    #expect(moved.participation == .invited(.needsAction))
    for event in events {
        #expect(ProvidedFieldsConformance.violations(event: event, capabilities: source.capabilities) == [])
        #expect(AllDayConformance.violations(event) == [])
    }
    let query = try #require(await h.server.requests("REPORT").first)
    #expect(String(decoding: query.body ?? Data(), as: UTF8.self).contains("start=\"20260901T070000Z\""))
}

@Test func withoutAddressesParticipationIsNotDeclared() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS())
    let source = try await h.source(userAddresses: [])
    #expect(!source.capabilities.providedFields.contains(.participation))
    #expect(!source.capabilities.canRespondToInvite)
    let events = try await source.events(in: september)
    #expect(events.allSatisfy { $0.participation == nil && $0.attendees.allSatisfy { !$0.isSelf } })
}

@Test func skipsACalendarThatDisappearedAndUnreadableResources() async throws {
    let h = CalDAVHarness()
    await addCollections(h.server)
    await h.server.store("home", "single.ics", singleICS())
    await h.server.store("home", "broken.ics", "this is not iCalendar")
    await h.server.store("shared", "weekly.ics", weeklyICS())
    let source = try await h.source()
    _ = try await source.calendars()
    await h.server.fail("REPORT", pathContains: "/shared/", status: 404)
    #expect(try await source.events(in: september).map(\.title) == ["Dentist"])
}

@Test func freeBusyOnlyCalendarsAreNotQueried() async throws {
    let h = CalDAVHarness()
    await h.server.configure { $0.collections["busy"] = .init(displayName: "Busy", privileges: ["read-free-busy"]) }
    let source = try await h.source()
    let busy = try #require(try await source.calendars().first { $0.id == "busy" })
    #expect(!busy.permissions.canViewDetails)
    _ = try await source.events(in: september)
    #expect(await h.server.requests("REPORT").allSatisfy { !$0.url.path.contains("/busy") })
}

@Test func wrongStoredPasswordIsAuthExpired() async throws {
    let h = CalDAVHarness()
    let source = try await h.source()
    try await h.credentials.setSecrets(["username": "me@icloud.test", "password": "revoked"], for: "c1")
    await #expect(throws: SourceError.authExpired) { try await source.calendars() }
}

@Test func capabilitiesFollowAutoSchedule() async throws {
    let h = CalDAVHarness()
    let scheduling = try await h.source(autoSchedule: true).capabilities
    #expect(scheduling.canEditAttendees && scheduling.canRespondToInvite && scheduling.writableFields.contains(.attendees))
    let plain = try await h.source(autoSchedule: false).capabilities
    #expect(!plain.canEditAttendees && !plain.canRespondToInvite && !plain.writableFields.contains(.attendees))
    for caps in [scheduling, plain] {
        #expect(caps.canEditAttendees == caps.writableFields.contains(.attendees))
        #expect(caps.canWrite && !caps.controlsNotifications && caps.syncKind == .token)
        #expect(caps.recurrenceScopes == Set(RecurrenceScope.allCases))
        #expect(!caps.writableFields.contains(.conference))
    }
}

@Test func seriesReturnsTheRulesOrNotFound() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS())
    await h.server.store("home", "single.ics", singleICS())
    let source = try await h.source()
    let series = try await source.series(id: "weekly.ics", calendarID: "home")
    #expect(series.seriesID == "weekly.ics" && series.start == pt(2026, 9, 1) && series.timeZone == pacific)
    #expect(series.recurrence.rules.first?.frequency == .weekly)
    #expect(series.recurrence.excludedDates == [pt(2026, 9, 8)])
    await #expect(throws: SourceError.notFound) { try await source.series(id: "single.ics", calendarID: "home") }
    await #expect(throws: SourceError.notFound) { try await source.series(id: "nope.ics", calendarID: "home") }
    #expect(ProvidedFieldsConformance.violations(source: source) == [])
}
```

The expected title order in `readsEventsFromEveryVisibleCalendar` is by start: 1 Sep 10:00 (Team sync), 10 Sep 09:00 (Dentist), 16 Sep 14:00 (moved), 22 and 29 Sep.

- [ ] **Step 3: Write the failing kind tests**

`ConnectorKindTests.swift`:

```swift
import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import CalDAVCalendar

private let good = ["username": "  me@icloud.test ", "password": "app-pass-1234"]

@Test func iCloudKindDescribesItself() throws {
    let kind = ICloudConnectorKind(transport: FakeCalDAVServer())
    #expect(kind.id == "icloud" && kind.displayName == "iCloud")
    guard case .password(let fields) = kind.authorization else { Issue.record("not a password kind"); return }
    #expect(fields.map(\.key) == ["username", "password"] && fields.map(\.isSecret) == [false, true])
    #expect(fields.map(\.label) == ["Apple ID", "App-specific password"])
    #expect(kind.credentialHelp?.url != nil)
    #expect(kind.supportedPlatforms.contains(.linux))
}

@Test func iCloudSignInStoresConfigAndSecrets() async throws {
    let server = FakeCalDAVServer()
    let store = InMemoryCredentialStore()
    let connection = try await ICloudConnectorKind(transport: server).authorize(using: StubInteraction(good), credentials: store)
    #expect(connection.kindID == "icloud" && connection.displayName == "me@icloud.test")
    let account = try CalDAVAccountConfig(config: connection.config)
    #expect(account.username == "me@icloud.test" && account.homeURL == fakeHomeURL && account.autoSchedule)
    #expect(!connection.config.values.contains { $0.contains("app-pass") })
    #expect(try await store.secrets(for: connection.connectionID) == ["username": "me@icloud.test", "password": "app-pass-1234"])
}

@Test func failedSignInStoresNothing() async throws {
    let store = InMemoryCredentialStore()
    await #expect(throws: SourceError.authExpired) {
        try await ICloudConnectorKind(transport: FakeCalDAVServer())
            .authorize(using: StubInteraction(["username": "me@icloud.test", "password": "wrong"]), credentials: store)
    }
    #expect(await store.isEmpty)
}

@Test func caldavKindValidatesTheServerAddress() async throws {
    #expect(try CalDAVAccountSetup.serverURL(from: " caldav.example.test/dav ").absoluteString == "https://caldav.example.test/dav")
    #expect(try CalDAVAccountSetup.serverURL(from: "http://localhost:8008").absoluteString == "http://localhost:8008")
    #expect(throws: SourceError.self) { try CalDAVAccountSetup.serverURL(from: "http://caldav.example.test") }
    #expect(throws: SourceError.self) { try CalDAVAccountSetup.serverURL(from: "https://") }
    #expect(throws: SourceError.self) { try CalDAVAccountSetup.serverURL(from: "https://me:secret@caldav.example.test/dav/") }
    let server = FakeCalDAVServer()
    let interaction = StubInteraction(["serverURL": "http://caldav.example.test", "username": "me", "password": "p"])
    await #expect(throws: SourceError.self) {
        try await CalDAVConnectorKind(transport: server).authorize(using: interaction, credentials: InMemoryCredentialStore())
    }
    #expect(await server.log.isEmpty)
}

@Test func caldavKindSignsInToTheEnteredServer() async throws {
    let server = FakeCalDAVServer()
    await server.configure { $0.username = "me"; $0.password = "p" }
    let kind = CalDAVConnectorKind(transport: server)
    guard case .password(let fields) = kind.authorization else { Issue.record("not a password kind"); return }
    #expect(fields.map(\.key) == ["serverURL", "username", "password"])
    let connection = try await kind.authorize(
        using: StubInteraction(["serverURL": "https://caldav.example.test", "username": "me", "password": "p"]), credentials: InMemoryCredentialStore())
    #expect(connection.kindID == "caldav" && connection.displayName == "me@caldav.example.test")
    #expect(try CalDAVAccountConfig(config: connection.config).homeURL.host == "caldav.example.test")
}

@Test func reauthorizeKeepsTheConnectionAndRefusesAnotherAccount() async throws {
    let server = FakeCalDAVServer()
    let store = InMemoryCredentialStore()
    let kind = ICloudConnectorKind(transport: server)
    let connection = try await kind.authorize(using: StubInteraction(good), credentials: store)
    await server.configure { $0.password = "new-pass" }
    let again = try await kind.reauthorize(connection, using: StubInteraction(["username": "me@icloud.test", "password": "new-pass"]), credentials: store)
    #expect(again.connectionID == connection.connectionID)
    #expect(try await store.secrets(for: connection.connectionID)?["password"] == "new-pass")

    var other = connection
    other.config["principalURL"] = "https://caldav.icloud.com/999/principal/"
    await #expect(throws: SourceError.invalidResponse("signed in as a different account")) {
        try await kind.reauthorize(other, using: StubInteraction(["username": "me@icloud.test", "password": "new-pass"]), credentials: store)
    }
}

@Test func makeSourceReadsSecretsFromTheStore() async throws {
    let server = FakeCalDAVServer()
    let store = InMemoryCredentialStore()
    let kind = ICloudConnectorKind(transport: server)
    let connection = try await kind.authorize(using: StubInteraction(good), credentials: store)
    let source = try kind.makeSource(for: connection, credentials: store, syncState: InMemorySyncStateStore())
    #expect(source.id == "icloud-\(connection.connectionID)")
    #expect(source is CalDAVCalendarSource)
    #expect(try await source.calendars().map(\.id) == ["home"])
    try await store.removeSecrets(for: connection.connectionID)
    await #expect(throws: SourceError.authExpired) { try await source.calendars() }
    var broken = connection
    broken.config = [:]
    #expect(throws: SourceError.self) { try kind.makeSource(for: broken, credentials: store, syncState: InMemorySyncStateStore()) }
}
```

- [ ] **Step 4: Run to verify they fail**

Run: `swift test --package-path Packages/CalendarConnectors --filter "SourceRead|ConnectorKind"`
Expected: compile errors.

- [ ] **Step 5: Implement `CalDAVCalendarSource.swift`**

```swift
import CalendarCore
import Foundation
import ICalendar

struct CalDAVCalendarInfo: Sendable {
    var descriptor: CalendarDescriptor
    var url: URL
    var ctag: String?
    var syncToken: String?
    /// Floating times and all-day dates on this calendar are read in this zone.
    var zone: TimeZone
}

/// What a source has learned and reuses: the calendar list and parsed resources (by URL, while the ETag matches).
actor CalDAVSourceState {
    private(set) var calendars: [CalDAVCalendarInfo]?
    private var parsed: [URL: (etag: String, resource: EventResource)] = [:]

    func store(calendars: [CalDAVCalendarInfo]) { self.calendars = calendars }
    func resource(at url: URL, etag: String) -> EventResource? {
        guard let entry = parsed[url], entry.etag == etag else { return nil }
        return entry.resource
    }
    func remember(_ resource: EventResource, at url: URL, etag: String) { parsed[url] = (etag, resource) }
    func forget(_ url: URL) { parsed[url] = nil }
}

/// One CalDAV account (iCloud or another server): reads through `calendar-query`, polls with `sync-collection` (or the
/// ctag), writes whole resources with ETags, and expands recurring events itself.
public final class CalDAVCalendarSource: PollingCalendarSource {
    let connection: Connection
    let account: CalDAVAccountConfig
    let client: WebDAVClient
    let provider: CalendarProvider
    let syncState: any SyncStateStore
    let monitor: ChangeMonitor
    let now: @Sendable () -> Date
    let defaultZone: TimeZone
    let makeUUID: @Sendable () -> String
    let state = CalDAVSourceState()

    init(
        connection: Connection, account: CalDAVAccountConfig, client: WebDAVClient, provider: CalendarProvider,
        syncState: any SyncStateStore, monitor: ChangeMonitor, now: @escaping @Sendable () -> Date,
        defaultZone: TimeZone = .current, makeUUID: @escaping @Sendable () -> String = { UUID().uuidString }
    ) {
        self.connection = connection
        self.account = account
        self.client = client
        self.provider = provider
        self.syncState = syncState
        self.monitor = monitor
        self.now = now
        self.defaultZone = defaultZone
        self.makeUUID = makeUUID
    }

    public var id: String { connection.sourceID }
    public var displayName: String { connection.displayName }

    /// The addresses that mean "me" on an event: the principal's address set and the principal URL (some servers name
    /// the organizer by it). Empty when the server named no addresses, so nothing is ever marked as self.
    var selfAddresses: Set<String> {
        guard !account.userAddresses.isEmpty else { return [] }
        return Set(account.userAddresses + [account.principalURL.absoluteString.lowercased()])
    }

    /// The `mailto:` address the account organizes meetings as: the one matching the user name, else the first.
    var organizerAddress: String? {
        let mail = account.userAddresses.filter { $0.hasPrefix("mailto:") }
        return mail.first { $0 == "mailto:" + account.username.lowercased() } ?? mail.first
    }

    public var capabilities: SourceCapabilities {
        var provided: Set<ProvidedField> = [.visibility, .availability, .reminders, .series, .version, .uidScope, .recurrenceRules,
                                            .permissionDetails, .provider, .calendarTimeZone, .supportedAvailabilities]
        if !account.userAddresses.isEmpty { provided.insert(.participation) }
        var writable: Set<EventField> = [.title, .notes, .location, .timing, .availability, .visibility, .reminders, .recurrence]
        // Without server scheduling, attendees written would never be invited.
        if account.autoSchedule { writable.insert(.attendees) }
        return SourceCapabilities(
            canWrite: true, canEditAttendees: account.autoSchedule,
            canRespondToInvite: account.autoSchedule && !account.userAddresses.isEmpty,
            providedFields: provided, syncKind: .token, writableFields: writable, controlsNotifications: false,
            recurrenceScopes: Set(RecurrenceScope.allCases))
    }

    // MARK: URLs and ids

    func calendarURL(_ calendarID: String) -> URL { account.homeURL.appendingPathComponent(calendarID, isDirectory: true) }

    func resourceURL(calendarID: String, name: String) -> URL { calendarURL(calendarID).appendingPathComponent(name) }

    /// The collection path below the home, without its trailing slash; nil for the home itself or anything outside it.
    func calendarID(for url: URL) -> String? {
        let home = account.homeURL.path.hasSuffix("/") ? account.homeURL.path : account.homeURL.path + "/"
        let path = url.path
        guard path.hasPrefix(home) else { return nil }
        let id = String(path.dropFirst(home.count)).trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        return id.isEmpty ? nil : id
    }

    // MARK: Calendars

    public func calendars() async throws -> [CalendarDescriptor] { try await loadCalendars().map(\.descriptor) }

    func loadCalendars() async throws -> [CalDAVCalendarInfo] {
        let (status, answeredBy) = try await client.propfind(account.homeURL, depth: 1, [
            .resourceType, .displayName, .supportedComponents, .privileges, .calendarTimeZone, .calendarColor, .getCTag, .syncToken,
        ])
        let defaultID = try await defaultCalendarID()
        var list: [CalDAVCalendarInfo] = []
        for response in status.responses {
            guard let url = try? client.resolve(response.href, against: answeredBy), let id = calendarID(for: url),
                  let info = calendar(from: response, id: id, defaultID: defaultID) else { continue }
            list.append(info)
        }
        list.sort { $0.descriptor.id < $1.descriptor.id }
        await state.store(calendars: list)
        return list
    }

    func calendar(from response: DAVResponse, id: String, defaultID: String??) -> CalDAVCalendarInfo? {
        let types = response.properties[.resourceType]?.children ?? []
        func has(_ namespace: String, _ name: String) -> Bool { types.contains { $0.namespace == namespace && $0.name == name } }
        guard has(DAV.caldav, "calendar"), !has(DAV.caldav, "schedule-inbox"), !has(DAV.caldav, "schedule-outbox"),
              !has(DAV.calendarServer, "notification") else { return nil }
        if let set = response.properties[.supportedComponents] {
            let names = set.children(DAV.caldav, "comp").compactMap { $0.attributes["name"]?.uppercased() }
            guard names.contains("VEVENT") else { return nil }
        }
        // A server that does not report privileges is treated as the owner's own calendar.
        let privileges = response.properties[.privileges].map { set in
            Set(set.children(DAV.dav, "privilege").flatMap { $0.children.map(\.name) })
        }
        let subscribed = has(DAV.calendarServer, "subscribed")
        let canEdit = !subscribed && (privileges.map { !$0.isDisjoint(with: ["write", "write-content", "all"]) } ?? true)
        let permissions = CalendarPermissions(
            canViewDetails: privileges.map { !$0.isDisjoint(with: ["read", "all"]) } ?? true, canEdit: canEdit,
            canShare: privileges.map { !$0.isDisjoint(with: ["write-acl", "all"]) } ?? false, canViewPrivate: canEdit)
        let zone = response.properties[.calendarTimeZone].flatMap { Self.zone(fromCalendarTimeZone: $0.text) } ?? defaultZone
        let displayName = response.properties[.displayName]?.trimmedText ?? ""
        let color = response.properties[.calendarColor].map { String($0.trimmedText.prefix(7)) }
        let descriptor = CalendarDescriptor(
            id: id, title: displayName.isEmpty ? (id.split(separator: "/").last.map(String.init) ?? id) : displayName,
            service: .calDAV, colorHex: color, permissions: permissions, isDefault: defaultID.map { $0 == id }, timeZone: zone,
            accountName: connection.displayName, kind: subscribed ? .subscribed : .standard, provider: provider,
            supportedAvailabilities: [.busy, .free])
        return CalDAVCalendarInfo(descriptor: descriptor, url: calendarURL(id), ctag: response.properties[.getCTag]?.trimmedText,
                                  syncToken: response.properties[.syncToken]?.trimmedText, zone: zone)
    }

    /// The `calendar-timezone` value is a `VCALENDAR` holding one `VTIMEZONE`.
    static func zone(fromCalendarTimeZone text: String) -> TimeZone? {
        guard let calendar = try? ICalParser.parse(text),
              let tzid = calendar.components(named: "VTIMEZONE").first?.property("TZID")?.value else { return nil }
        return TimeZoneResolver(calendar: calendar).zone(for: tzid)
    }

    /// `.some(id)` for the calendar that receives invitations, `.some(nil)` when the server says none of these, nil when
    /// it does not say (RFC 6638 puts `schedule-default-calendar-URL` on the scheduling inbox; some servers on the
    /// principal). Only an expired sign-in is an error here.
    func defaultCalendarID() async throws -> String?? {
        do {
            let (principal, principalURL) = try await client.propfind(account.principalURL, depth: 0, [.scheduleDefaultCalendarURL, .scheduleInboxURL])
            let properties = principal.responses.first?.properties ?? [:]
            if let href = properties[.scheduleDefaultCalendarURL]?.child(DAV.dav, "href")?.trimmedText {
                return .some(calendarID(for: try client.resolve(href, against: principalURL)))
            }
            guard let inboxHref = properties[.scheduleInboxURL]?.child(DAV.dav, "href")?.trimmedText else { return nil }
            let inboxURL = try client.resolve(inboxHref, against: principalURL)
            let (inbox, answeredBy) = try await client.propfind(inboxURL, depth: 0, [.scheduleDefaultCalendarURL])
            guard let href = inbox.responses.first?.properties[.scheduleDefaultCalendarURL]?.child(DAV.dav, "href")?.trimmedText else { return nil }
            return .some(calendarID(for: try client.resolve(href, against: answeredBy)))
        } catch SourceError.authExpired {
            throw SourceError.authExpired
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            return nil
        }
    }

    func calendarZone(_ calendarID: String) async throws -> TimeZone {
        let calendars: [CalDAVCalendarInfo]
        if let stored = await state.calendars { calendars = stored } else { calendars = try await loadCalendars() }
        return calendars.first { $0.descriptor.id == calendarID }?.zone ?? defaultZone
    }

    func context(calendarID: String, zone: TimeZone, resourceName: String, etag: String?) -> EventReadContext {
        EventReadContext(calendarID: calendarID, resourceName: resourceName, etag: etag, sourceID: id, calendarZone: zone,
                         selfAddresses: selfAddresses)
    }

    // MARK: Events

    /// Uses the calendar list the last `calendars()` call or poll stored. A calendar removed since then (403, 404, 410)
    /// is skipped; one the account can see only as free/busy is not queried.
    public func events(in interval: DateInterval) async throws -> [CalendarEvent] {
        let calendars: [CalDAVCalendarInfo]
        if let stored = await state.calendars { calendars = stored } else { calendars = try await loadCalendars() }
        return try await withThrowingTaskGroup(of: [CalendarEvent].self) { group in
            for calendar in calendars where calendar.descriptor.permissions.canViewDetails {
                group.addTask { try await self.events(for: calendar, in: interval) }
            }
            var all: [CalendarEvent] = []
            for try await part in group { all += part }
            return all.sorted { ($0.start, $0.eventID) < ($1.start, $1.eventID) }
        }
    }

    private func events(for calendar: CalDAVCalendarInfo, in interval: DateInterval) async throws -> [CalendarEvent] {
        let reply = try await client.report(calendar.url, depth: 1, body: DAVXML.calendarQuery(from: interval.start, to: interval.end))
        switch reply.response.status {
        case 207: break
        case 403, 404, 410: return []
        default: throw SourceError.invalidResponse("calendar-query answered \(reply.response.status)")
        }
        var events: [CalendarEvent] = []
        for response in try DAVXML.multistatus(reply.response.body).responses {
            guard let url = try? client.resolve(response.href, against: reply.url),
                  let object = await calendarObject(response, at: url) else { continue }
            let context = context(calendarID: calendar.descriptor.id, zone: calendar.zone, resourceName: url.lastPathComponent, etag: object.etag)
            events += EventReader.events(in: object.resource, overlapping: interval, context: context)
        }
        return events
    }

    /// The parsed resource in a multistatus response (reused while its ETag is unchanged); nil when it has no ETag or
    /// cannot be read, so one bad file does not hide the rest.
    func calendarObject(_ response: DAVResponse, at url: URL) async -> (resource: EventResource, etag: String)? {
        guard let etag = response.properties[.getETag]?.trimmedText, !etag.isEmpty else { return nil }
        if let cached = await state.resource(at: url, etag: etag) { return (cached, etag) }
        guard let text = response.properties[.calendarData]?.text, let resource = try? EventResource(data: Data(text.utf8)) else { return nil }
        await state.remember(resource, at: url, etag: etag)
        return (resource, etag)
    }

    public func changes() -> AsyncStream<CalendarChange> { monitor.changes(polling: self) }
}
```

`checkForChanges()` is required by `PollingCalendarSource`; until Task 12 add this stub at the end of the class (Task 12 replaces it):

```swift
    public func checkForChanges() async throws -> CalendarChange? { nil }
```

- [ ] **Step 6: Implement `CalDAVCalendarSource+Series.swift`**

```swift
import CalendarCore
import Foundation
import ICalendar

extension CalDAVCalendarSource: SeriesSource {
    /// `id` is the resource name (`SeriesInfo.occurrence(seriesID:)`). A resource that is missing or does not recur is
    /// `SourceError.notFound`.
    public func series(id: String, calendarID: String) async throws -> CalendarSeries {
        let reply = try await client.send("GET", resourceURL(calendarID: calendarID, name: id))
        switch reply.response.status {
        case 200: break
        case 403, 404, 410: throw SourceError.notFound
        default: throw SourceError.invalidResponse("GET answered \(reply.response.status)")
        }
        guard let resource = try? EventResource(data: reply.response.body) else { throw SourceError.invalidResponse("the event could not be read") }
        let context = context(calendarID: calendarID, zone: try await calendarZone(calendarID), resourceName: id, etag: reply.response.header("ETag"))
        guard let series = EventReader.series(of: resource, context: context) else { throw SourceError.notFound }
        return series
    }
}
```

This reads the rule through the ICalendar reader (and its `VTIMEZONE` resolver) rather than `RecurrenceSet(iCalendarLines:)`, so a custom `TZID` on `EXDATE` resolves; record it in the spec's "As built".

- [ ] **Step 7: Implement `CalDAVConnectorKinds.swift`**

```swift
import CalendarCore
import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

/// What the iCloud and "Other CalDAV" kinds share: sign-in (prompt, discover, store), sign-in again, and the source.
struct CalDAVAccountSetup: Sendable {
    let kindID: String
    let provider: CalendarProvider
    let fields: [CredentialField]
    /// iCloud's fixed server; nil when the user enters one.
    let fixedServer: URL?
    /// iCloud's host base (`icloud.com`, which covers the partition hosts); nil means the entered server's host.
    let fixedHostBase: String?
    let transport: any HTTPTransport
    let now: @Sendable () -> Date
    let sleep: Sleeper
    let pollInterval: Duration

    func hostBase(for server: URL) -> String { fixedHostBase ?? server.host?.lowercased() ?? "" }

    func authorize(using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        let (account, secret) = try await signIn(using: interaction)
        let connection = Connection(kindID: kindID, connectionID: UUID().uuidString, displayName: displayName(account), config: account.config)
        try await credentials.setSecrets(["username": secret.username, "password": secret.password], for: connection.connectionID)
        return connection
    }

    func reauthorize(_ connection: Connection, using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        let previous = try CalDAVAccountConfig(config: connection.config)
        let (account, secret) = try await signIn(using: interaction)
        guard account.principalURL.path == previous.principalURL.path,
              hostBase(for: account.serverURL) == hostBase(for: previous.serverURL) else {
            throw SourceError.invalidResponse("signed in as a different account")
        }
        try await credentials.setSecrets(["username": secret.username, "password": secret.password], for: connection.connectionID)
        var updated = connection
        updated.config = account.config
        return updated
    }

    func makeSource(for connection: Connection, credentials: any CredentialStore, syncState: any SyncStateStore) throws -> any CalendarSource {
        let account = try CalDAVAccountConfig(config: connection.config)
        let connectionID = connection.connectionID
        let client = WebDAVClient(transport: transport, hostBase: hostBase(for: account.serverURL), credentials: {
            guard let secrets = try await credentials.secrets(for: connectionID), let username = secrets["username"],
                  let password = secrets["password"] else { throw SourceError.authExpired }
            return WebDAVCredentials(username: username, password: password)
        })
        return CalDAVCalendarSource(
            connection: connection, account: account, client: client, provider: provider, syncState: syncState,
            monitor: ChangeMonitor(interval: pollInterval, sleep: sleep), now: now)
    }

    /// Nothing is stored here: callers store secrets only after discovery succeeds.
    private func signIn(using interaction: any AuthorizationInteraction) async throws -> (CalDAVAccountConfig, WebDAVCredentials) {
        let values = try await interaction.promptCredentials(fields)
        let username = (values["username"] ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        let password = values["password"] ?? ""
        guard !username.isEmpty, !password.isEmpty else { throw SourceError.invalidResponse("a user name and password are required") }
        let server = try fixedServer ?? Self.serverURL(from: values["serverURL"] ?? "")
        let secret = WebDAVCredentials(username: username, password: password)
        let client = WebDAVClient(transport: transport, hostBase: hostBase(for: server), credentials: { secret })
        return (try await CalDAVDiscovery(client: client).discover(serverURL: server, username: username), secret)
    }

    /// The address the user typed: `https://` is assumed when no scheme is given; plain `http` only to the loopback host.
    static func serverURL(from text: String) throws -> URL {
        var trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.contains("://") { trimmed = "https://" + trimmed }
        guard let url = URL(string: trimmed), let host = url.host?.lowercased(), !host.isEmpty, let scheme = url.scheme?.lowercased() else {
            throw SourceError.invalidResponse("the server address is not a valid URL")
        }
        guard scheme == "https" || (scheme == "http" && ["localhost", "127.0.0.1"].contains(host)) else {
            throw SourceError.invalidResponse("the server address must start with https://")
        }
        guard url.user == nil, url.password == nil else {
            throw SourceError.invalidResponse("enter the user name and password in their own fields, not in the server address")
        }
        return url
    }

    /// The Apple ID for iCloud; for another server the user name, with the host when the name is not an address.
    private func displayName(_ account: CalDAVAccountConfig) -> String {
        if fixedServer != nil || account.username.contains("@") { return account.username }
        return "\(account.username)@\(account.serverURL.host ?? "")"
    }
}

/// iCloud through CalDAV: an Apple ID and an app-specific password; the server is fixed.
public struct ICloudConnectorKind: ConnectorKind, CredentialPromptHelp {
    public static let kindID = "icloud"
    public static let serverURL = URL(string: "https://caldav.icloud.com")!
    private let setup: CalDAVAccountSetup

    public init(
        transport: any HTTPTransport = URLSessionTransport(followsRedirects: false), now: @escaping @Sendable () -> Date = { Date() },
        sleep: @escaping Sleeper = defaultSleeper, pollInterval: Duration = .seconds(60)
    ) {
        setup = CalDAVAccountSetup(
            kindID: Self.kindID, provider: .iCloud,
            fields: [CredentialField(key: "username", label: "Apple ID"), CredentialField(key: "password", label: "App-specific password", isSecret: true)],
            fixedServer: Self.serverURL, fixedHostBase: "icloud.com", transport: transport, now: now, sleep: sleep, pollInterval: pollInterval)
    }

    public var id: String { Self.kindID }
    public var displayName: String { "iCloud" }
    public var supportedPlatforms: Platform { [.macOS, .iOS, .linux, .windows] }
    public var authorization: AuthorizationMethod { .password(fields: setup.fields) }
    public var credentialHelp: CredentialHelp? {
        CredentialHelp(
            text: "iCloud needs an app-specific password, not your Apple Account password. Create one at account.apple.com under Sign-In and Security.",
            linkTitle: "Create an app-specific password", url: URL(string: "https://account.apple.com/account/manage"))
    }

    public func authorize(using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        try await setup.authorize(using: interaction, credentials: credentials)
    }

    public func reauthorize(_ connection: Connection, using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        try await setup.reauthorize(connection, using: interaction, credentials: credentials)
    }

    public func makeSource(for connection: Connection, credentials: any CredentialStore, syncState: any SyncStateStore) throws -> any CalendarSource {
        try setup.makeSource(for: connection, credentials: credentials, syncState: syncState)
    }
}

/// Any other CalDAV server (Fastmail, Nextcloud, ...): the user enters the server address, user name and password.
public struct CalDAVConnectorKind: ConnectorKind, CredentialPromptHelp {
    public static let kindID = "caldav"
    private let setup: CalDAVAccountSetup

    public init(
        transport: any HTTPTransport = URLSessionTransport(followsRedirects: false), now: @escaping @Sendable () -> Date = { Date() },
        sleep: @escaping Sleeper = defaultSleeper, pollInterval: Duration = .seconds(60)
    ) {
        setup = CalDAVAccountSetup(
            kindID: Self.kindID, provider: .calDAV,
            fields: [CredentialField(key: "serverURL", label: "Server address"), CredentialField(key: "username", label: "User name"),
                     CredentialField(key: "password", label: "Password", isSecret: true)],
            fixedServer: nil, fixedHostBase: nil, transport: transport, now: now, sleep: sleep, pollInterval: pollInterval)
    }

    public var id: String { Self.kindID }
    public var displayName: String { "Other CalDAV" }
    public var supportedPlatforms: Platform { [.macOS, .iOS, .linux, .windows] }
    public var authorization: AuthorizationMethod { .password(fields: setup.fields) }
    public var credentialHelp: CredentialHelp? {
        CredentialHelp(text: "Use the CalDAV address your provider gives you, for example https://caldav.fastmail.com. Many providers need an app password here.")
    }

    public func authorize(using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        try await setup.authorize(using: interaction, credentials: credentials)
    }

    public func reauthorize(_ connection: Connection, using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        try await setup.reauthorize(connection, using: interaction, credentials: credentials)
    }

    public func makeSource(for connection: Connection, credentials: any CredentialStore, syncState: any SyncStateStore) throws -> any CalendarSource {
        try setup.makeSource(for: connection, credentials: credentials, syncState: syncState)
    }
}
```

- [ ] **Step 8: Run the tests**

Run: `swift test --package-path Packages/CalendarConnectors --filter "CalDAVCalendarTests"`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add Packages/CalendarConnectors/Sources/CalDAVCalendar Packages/CalendarConnectors/Tests/CalDAVCalendarTests
git commit -m "CalDAVCalendar: read calendars, events and series; iCloud and Other CalDAV connector kinds"
```

---
### Task 12: Change detection

**Files:**
- Create: `Packages/CalendarConnectors/Sources/CalDAVCalendar/CalDAVCalendarSource+Sync.swift`
- Modify: `Packages/CalendarConnectors/Sources/CalDAVCalendar/CalDAVCalendarSource.swift` (remove the `checkForChanges` stub)
- Test: `Packages/CalendarConnectors/Tests/CalDAVCalendarTests/ChangeDetectionTests.swift`

**Interfaces:**
- Consumes: Task 11 (`loadCalendars`, `CalDAVCalendarInfo`, `state`), `DAVXML.syncCollection`, `DAVXML.isInvalidSyncToken`.
- Produces: `public func checkForChanges() async throws -> CalendarChange?`. Sync state scopes: `calendars` (a signature of the calendar list) and `calendar:<id>` (`sync:<token>` or `ctag:<ctag>`).

- [ ] **Step 1: Write the failing tests**

```swift
import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import CalDAVCalendar

@Test func firstCheckIsABaselineAndAQuietOneIsNil() async throws {
    let h = CalDAVHarness()
    let source = try await h.source()
    #expect(try await source.checkForChanges() == nil)
    #expect(try await source.checkForChanges() == nil)
    #expect(await h.server.requests("REPORT").isEmpty)   // unchanged tokens need no sync-collection
}

@Test func anotherClientsEditAndRemovalAreReported() async throws {
    let h = CalDAVHarness()
    let source = try await h.source()
    _ = try await source.checkForChanges()
    await h.server.store("home", "single.ics", singleICS())
    #expect(try await source.checkForChanges() == .eventsChanged(calendarIDs: ["home"]))
    #expect(try await source.checkForChanges() == nil)
    await h.server.remove("home", "single.ics")
    #expect(try await source.checkForChanges() == .eventsChanged(calendarIDs: ["home"]))
}

@Test func aNewCalendarIsCalendarsChanged() async throws {
    let h = CalDAVHarness()
    let source = try await h.source()
    _ = try await source.checkForChanges()
    await h.server.configure { $0.collections["work"] = .init(displayName: "Work") }
    #expect(try await source.checkForChanges() == .calendarsChanged)
    #expect(try await source.checkForChanges() == nil)
    await h.server.configure { $0.collections["work"]?.displayName = "Work (renamed)" }
    #expect(try await source.checkForChanges() == .calendarsChanged)
}

@Test func anExpiredTokenRebaselinesAndReportsTheCalendar() async throws {
    let h = CalDAVHarness()
    let source = try await h.source()
    _ = try await source.checkForChanges()
    await h.server.expireSyncTokens()
    await h.server.store("home", "single.ics", singleICS())
    #expect(try await source.checkForChanges() == .eventsChanged(calendarIDs: ["home"]))
    #expect(try await source.checkForChanges() == nil)
}

@Test func aServerWithoutSyncTokensFallsBackToTheCtag() async throws {
    let h = CalDAVHarness()
    await h.server.configure { $0.supportsSync = false }
    let source = try await h.source()
    _ = try await source.checkForChanges()
    await h.server.store("home", "single.ics", singleICS())
    #expect(try await source.checkForChanges() == .eventsChanged(calendarIDs: ["home"]))
    #expect(await h.server.requests("REPORT").isEmpty)
}

@Test func authExpiredEndsTheStream() async throws {
    let h = CalDAVHarness()
    let source = try await h.source()
    try await h.credentials.setSecrets(["username": "me@icloud.test", "password": "revoked"], for: "c1")
    var received: [CalendarChange] = []
    for await change in source.changes() { received.append(change) }
    #expect(received == [.sourceFailed(.authExpired)])
}

@Test func aFailureOnOneCalendarDoesNotSwallowAnothersChange() async throws {
    let h = CalDAVHarness()
    await h.server.configure { $0.collections["work"] = .init(displayName: "Work") }
    let source = try await h.source()
    _ = try await source.checkForChanges()
    await h.server.store("home", "a.ics", singleICS(uid: "a"))
    await h.server.store("work", "b.ics", singleICS(uid: "b"))
    await h.server.fail("REPORT", pathContains: "/work/", status: 500)
    await #expect(throws: SourceError.server(status: 500)) { try await source.checkForChanges() }
    // `home` was found changed before `work` failed; the retry must still report both.
    #expect(try await source.checkForChanges() == .eventsChanged(calendarIDs: ["home", "work"]))
    #expect(try await source.checkForChanges() == nil)
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `swift test --package-path Packages/CalendarConnectors --filter ChangeDetection`
Expected: FAIL (the stub always returns nil).

- [ ] **Step 3: Implement**

Delete the `checkForChanges` stub from `CalDAVCalendarSource.swift` and create `CalDAVCalendarSource+Sync.swift`:

```swift
import CalendarCore
import Foundation

extension CalDAVCalendarSource {
    static let calendarsScope = "calendars"
    static func calendarScope(_ id: String) -> String { "calendar:" + id }

    /// What a consumer must reload the calendar list for: calendars added or removed, renamed, recoloured, access or
    /// zone changed. Event changes (the ctag) are not part of it.
    static func signature(_ calendars: [CalDAVCalendarInfo]) -> String {
        calendars.map { calendar in
            let d = calendar.descriptor
            return [d.id, d.title, d.colorHex ?? "", String(d.permissions.canEdit), String(d.permissions.canViewDetails), calendar.zone.identifier]
                .joined(separator: "\u{1F}")
        }.sorted().joined(separator: "\u{1E}")
    }

    /// `sync:<token>` when the calendar has a sync token (RFC 6578), else `ctag:<ctag>`.
    static func marker(_ calendar: CalDAVCalendarInfo) -> String {
        if let token = calendar.syncToken, !token.isEmpty { return "sync:" + token }
        return "ctag:" + (calendar.ctag ?? "")
    }

    /// One `PROPFIND` on the home; a `sync-collection` only for calendars whose token moved, so a token bump without
    /// a change is not reported. The first call stores the baseline and returns nil.
    public func checkForChanges() async throws -> CalendarChange? {
        let calendars = try await loadCalendars()
        let owner = connection.connectionID
        let signature = Self.signature(calendars)
        let stored = await syncState.token(for: owner, scope: Self.calendarsScope)
        guard stored == signature else {
            await syncState.setToken(signature, for: owner, scope: Self.calendarsScope)
            for calendar in calendars {
                await syncState.setToken(Self.marker(calendar), for: owner, scope: Self.calendarScope(calendar.descriptor.id))
            }
            return stored == nil ? nil : .calendarsChanged
        }
        // New markers are written only after every calendar was checked, so an error on one calendar cannot swallow the
        // change already found on another: the retry finds both.
        var changed = Set<String>()
        var pending: [(scope: String, marker: String)] = []
        for calendar in calendars {
            let outcome = try await check(calendar)
            if outcome.changed { changed.insert(calendar.descriptor.id) }
            if let marker = outcome.marker { pending.append((Self.calendarScope(calendar.descriptor.id), marker)) }
        }
        for entry in pending { await syncState.setToken(entry.marker, for: owner, scope: entry.scope) }
        return changed.isEmpty ? nil : .eventsChanged(calendarIDs: changed)
    }

    /// Whether the calendar changed, and the marker to store once the whole check has succeeded (nil: keep the old one).
    private func check(_ calendar: CalDAVCalendarInfo) async throws -> (changed: Bool, marker: String?) {
        let owner = connection.connectionID
        let current = Self.marker(calendar)
        let stored = await syncState.token(for: owner, scope: Self.calendarScope(calendar.descriptor.id))
        guard stored != current else { return (false, nil) }
        // A ctag server (or a calendar that changed how it reports) has nothing finer to ask.
        guard let stored, stored.hasPrefix("sync:"), current.hasPrefix("sync:") else { return (true, current) }
        let reply = try await client.report(calendar.url, depth: 1, body: DAVXML.syncCollection(token: String(stored.dropFirst(5))))
        if DAVXML.isInvalidSyncToken(reply.response) { return (true, current) }
        switch reply.response.status {
        case 207: break
        // Removed or lost access since the list was read: the next list signature reports it.
        case 403, 404, 410: return (false, nil)
        default: throw SourceError.invalidResponse("sync-collection answered \(reply.response.status)")
        }
        let status = try DAVXML.multistatus(reply.response.body)
        for response in status.responses {
            if let url = try? client.resolve(response.href, against: reply.url) { await state.forget(url) }
        }
        return (!status.responses.isEmpty, status.syncToken.map { "sync:" + $0 } ?? current)
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `swift test --package-path Packages/CalendarConnectors --filter "ChangeDetection|SourceRead"`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors/Sources/CalDAVCalendar Packages/CalendarConnectors/Tests/CalDAVCalendarTests
git commit -m "CalDAVCalendar: change detection with sync-collection and a ctag fallback"
```

---

### Task 13: Writes (create, update, delete, respond)

**Files:**
- Create: `Packages/CalendarConnectors/Sources/CalDAVCalendar/CalDAVCalendarSource+Write.swift`, `CalDAVCalendarSource+Split.swift` (a stub Task 14 fills)
- Test: `Packages/CalendarConnectors/Tests/CalDAVCalendarTests/WriteTests.swift`

**Interfaces:**
- Consumes: Tasks 6-8 (`EventReader`, `EventWriter`, `AlarmMapper`, `AttendeeMapper.setResponse`, `SeriesEditor`), Task 11.
- Produces:
  ```swift
  struct FetchedResource: Sendable { var resource: EventResource; var etag: String; var url: URL; var name: String }
  enum PutOutcome: Sendable { case stored(etag: String, resource: EventResource); case stale }
  extension CalDAVCalendarSource: WritableCalendarSource {
      static func resourceName(of eventID: String) -> (name: String, isOccurrence: Bool)
      static func seriesEnd(start: Date, rule: RecurrenceRule?) -> Date
      func fetch(calendarID: String, name: String) async throws -> FetchedResource
      func put(_ resource: EventResource, to url: URL, ifMatch: String? = nil, ifNoneMatch: Bool = false) async throws -> PutOutcome
      func readBack(_ resource: EventResource, etag: String, calendarID: String, name: String, originalStart: Date?) async throws -> CalendarEvent
      func isRecurring(_ resource: EventResource) -> Bool
      func masterStart(_ resource: EventResource, calendarZone: TimeZone) -> Date?
      func tellsOthers(_ resource: EventResource) -> Bool
  }
  // +Split.swift (Task 14 replaces the body)
  func splitSeries(_ current: FetchedResource, ref: EventRef, patch: EventPatch, slot: Date, calendarZone: TimeZone) async throws -> PatchMerge.Attempt<CalendarEvent>
  ```

- [ ] **Step 1: Write the failing tests**

```swift
import CalendarCore
import CalendarTestSupport
import Foundation
import ICalendar
import Testing
@testable import CalDAVCalendar

private func expectWriteError(_ expected: WriteError, _ body: () async throws -> Void) async {
    do {
        try await body()
        Issue.record("expected \(expected), but nothing was thrown")
    } catch let error as WriteError {
        #expect(error == expected)
    } catch {
        Issue.record("expected \(expected), got \(error)")
    }
}

private func stored(_ h: CalDAVHarness, _ name: String, calendar: String = "home") async throws -> EventResource {
    let body = try #require(await h.server.body(calendar, name))
    return try EventResource(data: Data(body.utf8))
}

private func occurrence(_ source: CalDAVCalendarSource, on day: Int, hour: Int = 10) async throws -> CalendarEvent {
    try #require(try await source.events(in: september).first { $0.originalStart == pt(2026, 9, day, hour) })
}

private let timing = EventTiming(start: pt(2026, 9, 10, 9), end: pt(2026, 9, 10, 10), timeZone: pacific, isAllDay: false)

@Test func passesTheWritableConformanceSuite() async throws {
    for autoSchedule in [true, false] {
        let h = CalDAVHarness()
        let source = try await h.source(autoSchedule: autoSchedule)
        #expect(await WritableSourceConformance.violations(of: source, calendarID: "home", window: september) == [])
    }
}

@Test func createPutsANewResourceAndReturnsItsVersion() async throws {
    let h = CalDAVHarness()
    let source = try await h.source()
    let created = try await source.create(EventDraft(title: "Design review", timing: timing, reminders: [.before(minutes: 10)]), in: "home", notify: .none)
    #expect(created.eventID == "uuid-2.ics" && created.uid == "uuid-1" && created.series == .notRecurring)
    let etag = await h.server.etag("home", "uuid-2.ics")
    #expect(created.version == etag)
    let put = try #require(await h.server.requests("PUT").first)
    #expect(put.headers["If-None-Match"] == "*" && put.headers["Content-Type"]?.hasPrefix("text/calendar") == true)
    let resource = try await stored(h, "uuid-2.ics")
    #expect(resource.calendar.components(named: "VTIMEZONE").first?.property("TZID")?.value == "America/Los_Angeles")
    #expect(created.reminders == [Reminder(trigger: .relative(offset: -600, to: .start), isCalendarDefault: false)])
}

@Test func createWithAKnownUIDIsAlreadyExists() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "single.ics", singleICS(uid: "shared-uid"))
    let source = try await h.source()
    do {
        _ = try await source.create(EventDraft(title: "Copy", timing: timing, uid: "shared-uid"), in: "home", notify: .none)
        Issue.record("expected alreadyExists")
    } catch WriteError.alreadyExists(let existing) {
        #expect(existing.eventID == "single.ics" && existing.title == "Dentist")
    }
    #expect(await h.server.requests("PUT").isEmpty)
}

@Test func createRetriesANameClashOnce() async throws {
    let h = CalDAVHarness()
    let source = try await h.source()
    await h.server.fail("PUT", pathContains: "uuid-2.ics", status: 412)
    let created = try await source.create(EventDraft(title: "x", timing: timing), in: "home", notify: .none)
    #expect(created.eventID == "uuid-3.ics")
}

@Test func createRecurringReturnsTheSeriesMaster() async throws {
    let h = CalDAVHarness()
    let source = try await h.source()
    let series = try await source.create(
        EventDraft(title: "Weekly", timing: timing, recurrence: try RecurrenceRule(rrule: "FREQ=WEEKLY;COUNT=3")), in: "home", notify: .none)
    #expect(series.seriesID == series.eventID && series.originalStart == series.start)
    await expectWriteError(.invalid("this is a recurring series; use .allInSeries or read the occurrence first")) {
        _ = try await source.update(EventRef(series), EventPatch(title: "y"), scope: .thisInstance, notify: .none)
    }
}

@Test func refusedFieldsAndPoliciesWriteNothing() async throws {
    let h = CalDAVHarness()
    let source = try await h.source()
    let guests = [AttendeeDraft(email: "ann@example.test")]
    await expectWriteError(.unsupported(fields: [.attendees])) {
        _ = try await source.create(EventDraft(title: "x", timing: timing, attendees: guests), in: "home", notify: .none)
    }
    await expectWriteError(.unsupported(fields: [.conference])) {
        _ = try await source.create(EventDraft(title: "x", timing: timing, conference: .generate), in: "home", notify: .all)
    }
    await expectWriteError(.unsupported(fields: [.reminders])) {
        _ = try await source.create(EventDraft(title: "x", timing: timing, reminders: [.before(minutes: 5, type: .email(address: nil))]), in: "home", notify: .all)
    }
    let plain = try await h.source(autoSchedule: false)
    await expectWriteError(.unsupported(fields: [.attendees])) {
        _ = try await plain.create(EventDraft(title: "x", timing: timing, attendees: guests), in: "home", notify: .all)
    }
    #expect(await h.server.requests("PUT").isEmpty)
    let invited = try await source.create(EventDraft(title: "Meet", timing: timing, attendees: guests), in: "home", notify: .all)
    #expect(invited.organizer?.isSelf == true && invited.attendees.map(\.email) == ["me@icloud.test", "ann@example.test"])
}

@Test func updateKeepsPropertiesTheModelDoesNotCover() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "single.ics", singleICS())
    let source = try await h.source()
    let event = try #require(try await source.events(in: september).first)
    let renamed = try await source.update(EventRef(event), EventPatch(title: "Dentist (moved)"), scope: .thisInstance, notify: .none)
    #expect(renamed.title == "Dentist (moved)" && renamed.version != event.version)
    let put = try #require(await h.server.requests("PUT").last)
    #expect(put.headers["If-Match"] == event.version)
    #expect(try await stored(h, "single.ics").master?.property("X-APPLE-TRAVEL-ADVISORY-BEHAVIOR")?.value == "AUTOMATIC")
    await expectWriteError(.unsupported(fields: [.reminders])) {
        _ = try await source.update(EventRef(renamed), EventPatch(reminders: .clear), scope: .thisInstance, notify: .none)
    }
}

@Test func thisInstanceAddsThenEditsAnOverride() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    let tuesday = try await occurrence(source, on: 22)
    let edited = try await source.update(EventRef(tuesday), EventPatch(title: "Special"), scope: .thisInstance, notify: .none)
    #expect(edited.eventID == tuesday.eventID && edited.title == "Special")
    #expect(try await stored(h, "weekly.ics").overrides.count == 2)
    let again = try await source.update(EventRef(edited), EventPatch(location: .set("Room 9")), scope: .thisInstance, notify: .none)
    #expect(again.title == "Special" && again.location == "Room 9")
    #expect(try await stored(h, "weekly.ics").overrides.count == 2)
    let titles = try await source.events(in: september).map(\.title)
    #expect(titles == ["Team sync", "Team sync (moved)", "Special", "Team sync"])
}

@Test func allInSeriesTimingShiftsExceptionsWithTheSeries() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    let tuesday = try await occurrence(source, on: 22)
    let later = EventTiming(start: pt(2026, 9, 22, 11), end: pt(2026, 9, 22, 11, 30), timeZone: pacific, isAllDay: false)
    let series = try await source.update(EventRef(tuesday), EventPatch(timing: later), scope: .allInSeries, notify: .none)
    #expect(series.eventID == "weekly.ics" && series.start == pt(2026, 9, 1, 11))
    let starts = try await source.events(in: september).map(\.start)
    #expect(starts == [pt(2026, 9, 1, 11), pt(2026, 9, 16, 14), pt(2026, 9, 22, 11), pt(2026, 9, 29, 11)])
    var noSlot = EventRef(tuesday)
    noSlot.originalStart = nil
    await expectWriteError(.unsupported(fields: [.timing])) {
        _ = try await source.update(noSlot, EventPatch(timing: later), scope: .allInSeries, notify: .none)
    }
    await expectWriteError(.invalid("needs the occurrence's original start")) {
        _ = try await source.update(noSlot, EventPatch(title: "x"), scope: .thisInstance, notify: .none)
    }
}

@Test func aStaleVersionRetriesUnlessTheSameFieldChanged() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    let tuesday = try await occurrence(source, on: 22)
    // Someone else renames another occurrence: the resource's ETag moves, but not the field we touch.
    let other = try await occurrence(source, on: 29)
    _ = try await source.update(EventRef(other), EventPatch(title: "Other change"), scope: .thisInstance, notify: .none)
    var withRoom = tuesday
    withRoom.location = "Room 2"
    let moved = try await source.update(EventRef(tuesday), EventPatch(from: tuesday, to: withRoom), scope: .thisInstance, notify: .none)
    #expect(moved.location == "Room 2" && moved.title == "Team sync")
    // Now the same occurrence's title changes under a stale base.
    let fresh = try await occurrence(source, on: 22)
    _ = try await source.update(EventRef(fresh), EventPatch(title: "Theirs"), scope: .thisInstance, notify: .none)
    var mine = fresh
    mine.title = "Mine"
    await expectWriteError(.conflict(fields: [.title])) {
        _ = try await source.update(EventRef(fresh), EventPatch(from: fresh, to: mine), scope: .thisInstance, notify: .none)
    }
}

@Test func notifyPolicyIsRefusedWhenOthersWouldBeTold() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "meeting.ics", singleICS(uid: "m", title: "Meeting", attendees: [
        "ORGANIZER:mailto:me@icloud.test", "ATTENDEE;PARTSTAT=ACCEPTED:mailto:me@icloud.test", "ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:ann@example.test",
    ]))
    let source = try await h.source()
    let meeting = try #require(try await source.events(in: september).first)
    for policy in [NotifyPolicy.none, .externalOnly] {
        await expectWriteError(.unsupported(fields: [.attendees])) {
            _ = try await source.update(EventRef(meeting), EventPatch(title: "x"), scope: .thisInstance, notify: policy)
        }
        await expectWriteError(.unsupported(fields: [.attendees])) { try await source.delete(EventRef(meeting), scope: .thisInstance, notify: policy) }
    }
    #expect(await h.server.requests("PUT").isEmpty && (await h.server.requests("DELETE")).isEmpty)
    _ = try await source.update(EventRef(meeting), EventPatch(title: "Renamed"), scope: .thisInstance, notify: .all)
}

@Test func deletesByScope() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    try await source.delete(EventRef(try await occurrence(source, on: 22)), scope: .thisInstance, notify: .none)
    #expect(try await source.events(in: september).map(\.start) == [pt(2026, 9, 1), pt(2026, 9, 16, 14), pt(2026, 9, 29)])
    // The moved occurrence's slot is the 15th at 10:00, not its new time.
    try await source.delete(EventRef(try await occurrence(source, on: 15)), scope: .thisAndFollowing, notify: .none)
    #expect(try await source.events(in: september).map(\.start) == [pt(2026, 9, 1)])
    try await source.delete(EventRef(try await occurrence(source, on: 1)), scope: .allInSeries, notify: .none)
    #expect(await h.server.names("home").isEmpty)
    let deletion = try #require(await h.server.requests("DELETE").last)
    #expect(deletion.headers["If-Match"] == nil)
}

@Test func deletingAnOccurrenceRetriesThenReportsAConflict() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    let tuesday = try await occurrence(source, on: 22)
    await h.server.fail("PUT", pathContains: "weekly.ics", status: 412, times: 2)
    try await source.delete(EventRef(tuesday), scope: .thisInstance, notify: .none)
    #expect(await h.server.requests("PUT").count == 3)
    let thursday = try await occurrence(source, on: 29)
    await h.server.fail("PUT", pathContains: "weekly.ics", status: 412, times: 3)
    await expectWriteError(.conflict(fields: [.recurrence])) { try await source.delete(EventRef(thursday), scope: .thisInstance, notify: .none) }
    await expectWriteError(.notFound) { try await source.delete(EventRef(calendarID: "home", eventID: "gone.ics"), scope: .allInSeries, notify: .none) }
}

@Test func respondSetsTheAccountsAnswer() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS())
    let source = try await h.source()
    let tuesday = try await occurrence(source, on: 22)
    #expect(tuesday.participation == .invited(.needsAction))
    let answered = try await source.respond(to: EventRef(tuesday), .accepted, scope: .thisInstance, notify: .all)
    #expect(answered.participation == .invited(.accepted))
    #expect(try await occurrence(source, on: 29).participation == .invited(.needsAction))
    let series = try await source.respond(to: EventRef(tuesday), .declined, scope: .allInSeries, notify: .all)
    #expect(series.participation == .invited(.declined))
    #expect(try await source.events(in: september).allSatisfy { $0.participation == .invited(.declined) })
    await expectWriteError(.unsupported(fields: [.attendees])) { _ = try await source.respond(to: EventRef(tuesday), .accepted, scope: .thisInstance, notify: .none) }
    await expectWriteError(.unsupported(fields: [.attendees])) { _ = try await source.respond(to: EventRef(tuesday), .accepted, scope: .thisAndFollowing, notify: .all) }
    await expectWriteError(.invalid("a response must be accepted, tentative or declined")) {
        _ = try await source.respond(to: EventRef(tuesday), .needsAction, scope: .thisInstance, notify: .all)
    }
    await h.server.store("home", "single.ics", singleICS())
    let mine = try #require(try await source.events(in: september).first { $0.title == "Dentist" })
    await expectWriteError(.unsupported(fields: [.attendees])) { _ = try await source.respond(to: EventRef(mine), .accepted, scope: .thisInstance, notify: .all) }
}

@Test func forbiddenCalendarIsForbidden() async throws {
    let h = CalDAVHarness()
    await h.server.configure { $0.collections["shared"] = .init(displayName: "Shared", privileges: ["read"]) }
    await h.server.store("shared", "single.ics", singleICS())
    let source = try await h.source()
    await expectWriteError(.forbidden(nil)) { _ = try await source.create(EventDraft(title: "x", timing: timing), in: "shared", notify: .none) }
}
```

`EventPatch(from:to:)` diffs two events and sets `base`, so a stale version is judged field by field; a hand-built `EventPatch(...)` has no base and conflicts on any stale version.

- [ ] **Step 2: Run to verify they fail**

Run: `swift test --package-path Packages/CalendarConnectors --filter WriteTests`
Expected: compile errors (`CalDAVCalendarSource` is not a `WritableCalendarSource`).

- [ ] **Step 3: Implement `CalDAVCalendarSource+Split.swift` (stub)**

```swift
import CalendarCore
import Foundation

extension CalDAVCalendarSource {
    /// `.thisAndFollowing` after the first occurrence. Filled in by the split task.
    func splitSeries(_ current: FetchedResource, ref: EventRef, patch: EventPatch, slot: Date, calendarZone: TimeZone) async throws -> PatchMerge.Attempt<CalendarEvent> {
        throw WriteError.unsupported(fields: [.recurrence])
    }
}
```

- [ ] **Step 4: Implement `CalDAVCalendarSource+Write.swift`**

```swift
import CalendarCore
import Foundation
import ICalendar

struct FetchedResource: Sendable {
    var resource: EventResource
    var etag: String
    var url: URL
    var name: String
}

enum PutOutcome: Sendable {
    case stored(etag: String, resource: EventResource)
    case stale
}

extension CalDAVCalendarSource: WritableCalendarSource {
    static let seriesMessage = "this is a recurring series; use .allInSeries or read the occurrence first"
    static let slotMessage = "needs the occurrence's original start"

    // MARK: Plumbing

    /// `name.ics` or `name.ics#20260915T170000Z`: the resource name and whether the id names one occurrence.
    static func resourceName(of eventID: String) -> (name: String, isOccurrence: Bool) {
        guard let hash = eventID.firstIndex(of: "#") else { return (eventID, false) }
        return (String(eventID[..<hash]), true)
    }

    /// How far ahead a written `VTIMEZONE` must reach: the rule's `UNTIL`, 20 years for an open or counted rule.
    static func seriesEnd(start: Date, rule: RecurrenceRule?) -> Date {
        guard let rule else { return start }
        if case .until(let until) = rule.end { return until }
        return start.addingTimeInterval(20 * 366 * 86_400)
    }

    func fetch(calendarID: String, name: String) async throws -> FetchedResource {
        let url = resourceURL(calendarID: calendarID, name: name)
        let reply = try await client.send("GET", url)
        switch reply.response.status {
        case 200: break
        case 404, 410: throw WriteError.notFound
        case 403: throw WriteError.forbidden(nil)
        default: throw SourceError.invalidResponse("GET answered \(reply.response.status)")
        }
        guard let etag = reply.response.header("ETag") else { throw SourceError.invalidResponse("the server sent no ETag") }
        guard let resource = try? EventResource(data: reply.response.body) else { throw SourceError.invalidResponse("the event could not be read") }
        return FetchedResource(resource: resource, etag: etag, url: url, name: name)
    }

    /// PUTs the whole resource; 412 is `.stale`. The ETag comes from the response, or from one GET when the server
    /// sends none (the returned resource is then the server's copy).
    func put(_ resource: EventResource, to url: URL, ifMatch: String? = nil, ifNoneMatch: Bool = false) async throws -> PutOutcome {
        var headers = ["Content-Type": "text/calendar; charset=utf-8"]
        if let ifMatch { headers["If-Match"] = ifMatch }
        if ifNoneMatch { headers["If-None-Match"] = "*" }
        let reply = try await client.send("PUT", url, headers: headers, body: resource.serialized())
        switch reply.response.status {
        case 200, 201, 204: break
        case 412: return .stale
        case 403: throw WriteError.forbidden(nil)
        case 404, 410: throw WriteError.notFound
        default: throw SourceError.invalidResponse("PUT answered \(reply.response.status)")
        }
        await state.forget(url)
        if let etag = reply.response.header("ETag") { return .stored(etag: etag, resource: resource) }
        let reread = try await client.send("GET", url)
        guard reread.response.status == 200, let etag = reread.response.header("ETag"),
              let copy = try? EventResource(data: reread.response.body) else {
            throw SourceError.invalidResponse("the stored event could not be read back")
        }
        return .stored(etag: etag, resource: copy)
    }

    /// The occurrence at `originalStart`, or the master in its series form (a single event when it does not recur).
    func readBack(_ resource: EventResource, etag: String, calendarID: String, name: String, originalStart: Date?) async throws -> CalendarEvent {
        let context = context(calendarID: calendarID, zone: try await calendarZone(calendarID), resourceName: name, etag: etag)
        if let originalStart, let occurrence = EventReader.occurrence(in: resource, originalStart: originalStart, context: context) {
            return occurrence
        }
        guard let master = EventReader.masterEvent(of: resource, context: context) else { throw WriteError.notFound }
        return master
    }

    func isRecurring(_ resource: EventResource) -> Bool {
        !resource.overrides.isEmpty || resource.master.map { $0.property("RRULE") != nil || $0.property("RDATE") != nil } == true
    }

    func masterStart(_ resource: EventResource, calendarZone: TimeZone) -> Date? {
        resource.master.flatMap { EventReader.timing(of: $0, resolver: resource.resolver, calendarZone: calendarZone)?.start }
    }

    /// Whether the server would tell anyone but the account about a change: any attendee that is not the account.
    func tellsOthers(_ resource: EventResource) -> Bool {
        resource.events.contains { vevent in AttendeeMapper.read(vevent, selfAddresses: selfAddresses).attendees.contains { !$0.isSelf } }
    }

    /// Implicit scheduling tells attendees of every change and cannot be stopped (`controlsNotifications == false`).
    func requireNotify(_ notify: NotifyPolicy, tellsOthers: Bool) throws {
        if notify != .all && tellsOthers { throw WriteError.unsupported(fields: [.attendees]) }
    }

    /// The event a patch is judged against after a stale write: the occurrence the caller edited when it still
    /// exists, else the master.
    func comparedEvent(_ fresh: FetchedResource, ref: EventRef, scope: RecurrenceScope, calendarZone: TimeZone) throws -> CalendarEvent {
        let context = context(calendarID: ref.calendarID, zone: calendarZone, resourceName: fresh.name, etag: fresh.etag)
        if isRecurring(fresh.resource), let slot = ref.originalStart {
            if let occurrence = EventReader.occurrence(in: fresh.resource, originalStart: slot, context: context) { return occurrence }
            if scope != .allInSeries { throw WriteError.notFound }
        }
        guard let master = EventReader.masterEvent(of: fresh.resource, context: context) else { throw WriteError.notFound }
        return master
    }

    // MARK: Create

    public func create(_ draft: EventDraft, in calendarID: String, notify: NotifyPolicy) async throws -> CalendarEvent {
        try draft.validate()
        try WriteValidation.requireWritable(draft.usedFields, capabilities)
        try requireNotify(notify, tellsOthers: !draft.attendees.isEmpty)
        let stamp = now()
        let vevent = try EventWriter.vevent(from: draft, uid: draft.uid ?? makeUUID(), now: stamp, organizerAddress: organizerAddress)
        let zone = draft.timing.timeZone ?? TimeZone(identifier: "UTC")!
        let resource = try EventWriter.resource(for: vevent, zones: [zone], from: draft.timing.start,
                                                through: Self.seriesEnd(start: draft.timing.start, rule: draft.recurrence))
        if let uid = draft.uid { try await refuseDuplicate(uid: uid, calendarID: calendarID) }
        // A 412 means the name is taken: one more try with a new one.
        for _ in 0..<2 {
            let name = makeUUID() + ".ics"
            if case .stored(let etag, let copy) = try await put(resource, to: resourceURL(calendarID: calendarID, name: name), ifNoneMatch: true) {
                return try await readBack(copy, etag: etag, calendarID: calendarID, name: name, originalStart: nil)
            }
        }
        throw SourceError.invalidResponse("the server refused every new event name")
    }

    private func refuseDuplicate(uid: String, calendarID: String) async throws {
        let reply = try await client.report(calendarURL(calendarID), depth: 1, body: DAVXML.calendarQuery(uid: uid))
        switch reply.response.status {
        case 207: break
        case 403: throw WriteError.forbidden(nil)
        case 404, 410: throw WriteError.notFound
        default: throw SourceError.invalidResponse("calendar-query answered \(reply.response.status)")
        }
        let zone = try await calendarZone(calendarID)
        for response in try DAVXML.multistatus(reply.response.body).responses {
            guard let url = try? client.resolve(response.href, against: reply.url), let object = await calendarObject(response, at: url),
                  object.resource.uid == uid else { continue }
            let context = context(calendarID: calendarID, zone: zone, resourceName: url.lastPathComponent, etag: object.etag)
            let everything = DateInterval(start: .distantPast, end: .distantFuture)
            if let existing = EventReader.masterEvent(of: object.resource, context: context)
                ?? EventReader.events(in: object.resource, overlapping: everything, context: context).first {
                throw WriteError.alreadyExists(existing)
            }
        }
    }

    // MARK: Update

    public func update(_ ref: EventRef, _ patch: EventPatch, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent {
        let (name, isOccurrence) = Self.resourceName(of: ref.eventID)
        if patch.isEmpty {
            if let base = patch.base { return base }
            let current = try await fetch(calendarID: ref.calendarID, name: name)
            return try await readBack(current.resource, etag: current.etag, calendarID: ref.calendarID, name: name,
                                      originalStart: isOccurrence ? ref.originalStart : nil)
        }
        try WriteValidation.requireWritable(patch.touchedFields, capabilities)
        try patch.timing?.validate()
        if case .set(let rule) = patch.recurrence { try rule.validate() }
        if case .clear = patch.reminders { throw WriteError.unsupported(fields: [.reminders]) }
        if case .set(let reminders) = patch.reminders { _ = try AlarmMapper.alarms(from: reminders) }
        try Self.checkSeriesRef(ref, scope: scope, isOccurrence: isOccurrence, movesTime: patch.timing != nil)
        let zone = try await calendarZone(ref.calendarID)
        let url = resourceURL(calendarID: ref.calendarID, name: name)
        // The first attempt may use the copy the caller just read, when its version is that copy's ETag.
        var cached: FetchedResource? = nil
        if let version = ref.version, let copy = await state.resource(at: url, etag: version) {
            cached = FetchedResource(resource: copy, etag: version, url: url, name: name)
        }
        return try await PatchMerge.apply(
            patch: patch, version: ref.version,
            fetchCurrent: { try self.comparedEvent(try await self.fetch(calendarID: ref.calendarID, name: name), ref: ref, scope: scope, calendarZone: zone) },
            write: { version in
                let current: FetchedResource
                if let first = cached { current = first; cached = nil } else { current = try await self.fetch(calendarID: ref.calendarID, name: name) }
                if let version, version != current.etag { return .stale }
                try self.requireNotify(notify, tellsOthers: self.tellsOthers(current.resource) || !(patch.attendees?.add.isEmpty ?? true))
                if scope == .thisAndFollowing, isOccurrence, self.isRecurring(current.resource), let slot = ref.originalStart,
                   let first = self.masterStart(current.resource, calendarZone: zone), slot > first {
                    return try await self.splitSeries(current, ref: ref, patch: patch, slot: slot, calendarZone: zone)
                }
                var resource = current.resource
                let slot = try self.edit(&resource, ref: ref, patch: patch, scope: scope, isOccurrence: isOccurrence, calendarZone: zone)
                switch try await self.put(resource, to: current.url, ifMatch: current.etag) {
                case .stale: return .stale
                case .stored(let etag, let copy):
                    return .done(try await self.readBack(copy, etag: etag, calendarID: ref.calendarID, name: name, originalStart: slot))
                }
            })
    }

    /// What can be refused from the ref alone, before any request. A ref with an occurrence id or a series id belongs
    /// to a series; one with neither is a single event, where the scope is ignored.
    static func checkSeriesRef(_ ref: EventRef, scope: RecurrenceScope, isOccurrence: Bool, movesTime: Bool) throws {
        guard isOccurrence || ref.seriesID != nil else { return }
        if scope != .allInSeries && !isOccurrence { throw WriteError.invalid(seriesMessage) }
        if scope != .allInSeries && ref.originalStart == nil { throw WriteError.invalid(slotMessage) }
        // Moving the whole series from one occurrence needs that occurrence's slot to know how far it moved.
        if scope == .allInSeries && movesTime && ref.originalStart == nil { throw WriteError.unsupported(fields: [.timing]) }
    }

    /// Applies the patch in place and returns the occurrence to read back (nil: the master or single event).
    func edit(_ resource: inout EventResource, ref: EventRef, patch: EventPatch, scope: RecurrenceScope, isOccurrence: Bool,
              calendarZone: TimeZone) throws -> Date? {
        let stamp = now()
        defer {
            if let timing = patch.timing, let zone = timing.timeZone {
                resource.ensureTimeZones([zone], from: timing.start, through: timing.start.addingTimeInterval(20 * 366 * 86_400))
            }
        }
        guard isRecurring(resource) else {
            guard var master = resource.master else { throw WriteError.notFound }
            try EventWriter.apply(patch, to: &master, now: stamp, organizerAddress: organizerAddress)
            resource.setEvents([master])
            return nil
        }
        if scope != .allInSeries && !isOccurrence { throw WriteError.invalid(Self.seriesMessage) }
        if scope != .allInSeries && ref.originalStart == nil { throw WriteError.invalid(Self.slotMessage) }
        if scope == .thisInstance {
            let slot = ref.originalStart!
            if patch.recurrence != .keep { throw WriteError.invalid("a recurrence change applies to the whole series") }
            guard var override = SeriesEditor.override(in: resource, at: slot, calendarZone: calendarZone) else { throw WriteError.notFound }
            try EventWriter.apply(patch, to: &override, now: stamp, organizerAddress: organizerAddress)
            SeriesEditor.setOverride(override, at: slot, in: &resource, calendarZone: calendarZone)
            return slot
        }
        // .allInSeries, or .thisAndFollowing at the first occurrence.
        guard var master = resource.master,
              let timing = EventReader.timing(of: master, resolver: resource.resolver, calendarZone: calendarZone) else { throw WriteError.notFound }
        var masterPatch = patch
        var delta: TimeInterval = 0
        if let wanted = patch.timing {
            // The caller moved one occurrence; the series moves by the same amount, which needs that occurrence's slot.
            guard let slot = ref.originalStart else { throw WriteError.unsupported(fields: [.timing]) }
            let hasExceptions = !resource.overrides.isEmpty || master.property("EXDATE") != nil
            if wanted.isAllDay != timing.isAllDay && hasExceptions { throw WriteError.unsupported(fields: [.timing]) }
            let moved = Self.seriesTiming(master: timing, slot: slot, wanted: wanted, calendarZone: calendarZone)
            masterPatch.timing = moved.timing
            delta = moved.delta
        }
        try EventWriter.apply(masterPatch, to: &master, now: stamp, organizerAddress: organizerAddress)
        resource.setEvents([master] + resource.overrides)
        if delta != 0 { SeriesEditor.shift(&resource, by: delta, calendarZone: calendarZone) }
        if patch.recurrence != .keep { SeriesEditor.pruneUnmatched(&resource, calendarZone: calendarZone) }
        return nil
    }

    /// The master's new timing when the occurrence at `slot` moves to `wanted`: the same shift (whole days for all-day)
    /// and `wanted`'s length and zone.
    static func seriesTiming(master: EventTimingInfo, slot: Date, wanted: EventTiming, calendarZone: TimeZone) -> (timing: EventTiming, delta: TimeInterval) {
        guard wanted.isAllDay else {
            let delta = wanted.start.timeIntervalSince(slot)
            let start = master.start.addingTimeInterval(delta)
            return (EventTiming(start: start, end: start.addingTimeInterval(wanted.end.timeIntervalSince(wanted.start)),
                                timeZone: wanted.timeZone, isAllDay: false), delta)
        }
        let zone = wanted.timeZone ?? calendarZone
        let days = AllDay.date(of: slot, in: zone).days(to: AllDay.date(of: wanted.start, in: zone))
        let length = AllDay.date(of: wanted.start, in: zone).days(to: AllDay.date(of: wanted.end, in: zone))
        let first = AllDay.date(of: master.start, in: zone).adding(days: days)
        let start = AllDay.startOfDay(first, in: zone) ?? master.start
        let end = AllDay.startOfDay(first.adding(days: length), in: zone) ?? start
        return (EventTiming(start: start, end: end, timeZone: zone, isAllDay: true), Double(days) * 86_400)
    }

    // MARK: Delete

    public func delete(_ ref: EventRef, scope: RecurrenceScope, notify: NotifyPolicy) async throws {
        let (name, isOccurrence) = Self.resourceName(of: ref.eventID)
        try Self.checkSeriesRef(ref, scope: scope, isOccurrence: isOccurrence, movesTime: false)
        let zone = try await calendarZone(ref.calendarID)
        var current = try await fetch(calendarID: ref.calendarID, name: name)
        try requireNotify(notify, tellsOthers: tellsOthers(current.resource))
        var effective = isRecurring(current.resource) ? scope : .allInSeries
        if effective != .allInSeries {
            guard isOccurrence else { throw WriteError.invalid(Self.seriesMessage) }
            guard let slot = ref.originalStart else { throw WriteError.invalid(Self.slotMessage) }
            if effective == .thisAndFollowing, let first = masterStart(current.resource, calendarZone: zone), slot <= first { effective = .allInSeries }
        }
        if effective == .allInSeries {
            // Last writer wins, like the other connectors' deletes.
            let reply = try await client.send("DELETE", current.url)
            switch reply.response.status {
            case 200, 204: await state.forget(current.url); return
            case 404, 410: throw WriteError.notFound
            case 403: throw WriteError.forbidden(nil)
            default: throw SourceError.invalidResponse("DELETE answered \(reply.response.status)")
            }
        }
        let slot = ref.originalStart!
        // Idempotent and local to this occurrence, so a 412 (another occurrence changed) is re-applied to the fresh copy.
        for attempt in 0..<3 {
            if attempt > 0 { current = try await fetch(calendarID: ref.calendarID, name: name) }
            var resource = current.resource
            do {
                if effective == .thisInstance {
                    try SeriesEditor.exclude(slot, in: &resource, calendarZone: zone)
                    if var master = resource.master {
                        EventWriter.touch(&master, now: now(), bumpSequence: true)
                        resource.setEvents([master] + resource.overrides)
                    }
                } else {
                    resource = try SeriesEditor.split(resource, at: slot, newUID: makeUUID(), calendarZone: zone, now: now()).head
                }
            } catch WriteError.notFound where attempt > 0 {
                return   // someone else removed it meanwhile
            }
            if case .stored = try await put(resource, to: current.url, ifMatch: current.etag) { return }
        }
        throw WriteError.conflict(fields: [.recurrence])
    }

    // MARK: Respond

    public func respond(to ref: EventRef, _ response: ResponseStatus, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent {
        guard response != .needsAction else { throw WriteError.invalid("a response must be accepted, tentative or declined") }
        guard capabilities.canRespondToInvite else { throw WriteError.unsupported(fields: [.attendees]) }
        // The organizer is always told of an answer.
        guard notify == .all else { throw WriteError.unsupported(fields: [.attendees]) }
        let (name, isOccurrence) = Self.resourceName(of: ref.eventID)
        // An attendee cannot split the organizer's series.
        if scope == .thisAndFollowing && (isOccurrence || ref.seriesID != nil) { throw WriteError.unsupported(fields: [.attendees]) }
        let zone = try await calendarZone(ref.calendarID)
        return try await PatchMerge.apply(
            patch: EventPatch(), version: ref.version,
            fetchCurrent: { try self.comparedEvent(try await self.fetch(calendarID: ref.calendarID, name: name), ref: ref, scope: scope, calendarZone: zone) },
            write: { version in
                let current = try await self.fetch(calendarID: ref.calendarID, name: name)
                if let version, version != current.etag { return .stale }
                var resource = current.resource
                let slot = try self.setResponse(response, in: &resource, ref: ref, scope: scope, isOccurrence: isOccurrence, calendarZone: zone)
                switch try await self.put(resource, to: current.url, ifMatch: current.etag) {
                case .stale: return .stale
                case .stored(let etag, let copy):
                    return .done(try await self.readBack(copy, etag: etag, calendarID: ref.calendarID, name: name, originalStart: slot))
                }
            })
    }

    private func setResponse(_ response: ResponseStatus, in resource: inout EventResource, ref: EventRef, scope: RecurrenceScope,
                             isOccurrence: Bool, calendarZone: TimeZone) throws -> Date? {
        let stamp = now()
        let me = selfAddresses
        guard isRecurring(resource) else {
            guard var master = resource.master, AttendeeMapper.setResponse(response, in: &master, selfAddresses: me) else {
                throw WriteError.unsupported(fields: [.attendees])
            }
            EventWriter.touch(&master, now: stamp, bumpSequence: false)
            resource.setEvents([master])
            return nil
        }
        switch scope {
        case .thisAndFollowing:
            throw WriteError.unsupported(fields: [.attendees])
        case .thisInstance:
            guard isOccurrence else { throw WriteError.invalid(Self.seriesMessage) }
            guard let slot = ref.originalStart else { throw WriteError.invalid(Self.slotMessage) }
            guard var override = SeriesEditor.override(in: resource, at: slot, calendarZone: calendarZone) else { throw WriteError.notFound }
            guard AttendeeMapper.setResponse(response, in: &override, selfAddresses: me) else { throw WriteError.unsupported(fields: [.attendees]) }
            EventWriter.touch(&override, now: stamp, bumpSequence: false)
            SeriesEditor.setOverride(override, at: slot, in: &resource, calendarZone: calendarZone)
            return slot
        case .allInSeries:
            // The answer is for the whole series: the master and every override.
            guard resource.master != nil else { throw WriteError.notFound }
            var events = resource.events
            var found = false
            for index in events.indices where AttendeeMapper.setResponse(response, in: &events[index], selfAddresses: me) {
                EventWriter.touch(&events[index], now: stamp, bumpSequence: false)
                found = true
            }
            guard found else { throw WriteError.unsupported(fields: [.attendees]) }
            resource.setEvents(events)
            return nil
        }
    }
}
```

- [ ] **Step 5: Run the tests**

Run: `swift test --package-path Packages/CalendarConnectors --filter "WriteTests|ConnectorKind|SourceRead|ChangeDetection"`
Expected: PASS. If `passesTheWritableConformanceSuite` reports "the created event is not returned by events(in:)", check that `put` calls `state.forget` and that `events(in:)` uses `calendarObject` (a stale cache entry for the same URL and ETag cannot happen, since a new PUT gives a new ETag).

- [ ] **Step 6: Commit**

```bash
git add Packages/CalendarConnectors/Sources/CalDAVCalendar Packages/CalendarConnectors/Tests/CalDAVCalendarTests
git commit -m "CalDAVCalendar: create, update, delete and respond with ETags, PatchMerge and the notify rule"
```

---

### Task 14: Splitting a series (`.thisAndFollowing`)

**Files:**
- Modify: `Packages/CalendarConnectors/Sources/CalDAVCalendar/CalDAVCalendarSource+Split.swift`
- Test: `Packages/CalendarConnectors/Tests/CalDAVCalendarTests/SplitTests.swift`

**Interfaces:**
- Consumes: `SeriesEditor.split`, `SeriesEditor.shift`, `SeriesEditor.pruneUnmatched` (Task 8); `put`, `readBack`, `FetchedResource`, `seriesTiming` (Task 13).
- Produces: the real `splitSeries(...)`.

- [ ] **Step 1: Write the failing tests**

```swift
import CalendarCore
import CalendarTestSupport
import Foundation
import ICalendar
import Testing
@testable import CalDAVCalendar

private func occurrence(_ source: CalDAVCalendarSource, on day: Int, hour: Int = 10) async throws -> CalendarEvent {
    try #require(try await source.events(in: september).first { $0.originalStart == pt(2026, 9, day, hour) })
}

private let autumn = DateInterval(start: pt(2026, 9, 1, 0), end: pt(2026, 11, 1, 0))

@Test func splitAnUntilSeriesMovesLaterExceptions() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(rule: "FREQ=WEEKLY;BYDAY=TU;UNTIL=20261013T235959Z", organizer: nil))
    let source = try await h.source()
    let fifteenth = try await occurrence(source, on: 15)   // the moved occurrence
    let tail = try await source.update(EventRef(fifteenth), EventPatch(title: "New sync"), scope: .thisAndFollowing, notify: .none)
    #expect(tail.eventID == "uuid-2.ics" && tail.seriesID == "uuid-2.ics" && tail.uid == "uuid-1")
    let head = try EventResource(data: Data(try #require(await h.server.body("home", "weekly.ics")).utf8))
    #expect(head.master?.property("RRULE")?.value.contains("UNTIL=20260915T165959Z") == true)
    #expect(head.overrides.isEmpty)
    let newer = try EventResource(data: Data(try #require(await h.server.body("home", "uuid-2.ics")).utf8))
    #expect(newer.overrides.count == 1 && newer.overrides.first?.property("UID")?.text == "uuid-1")
    let events = try await source.events(in: autumn)
    #expect(events.map(\.title) == ["Team sync", "Team sync (moved)", "New sync", "New sync", "New sync", "New sync"])
    #expect(events.map(\.start) == [pt(2026, 9, 1), pt(2026, 9, 16, 14), pt(2026, 9, 22), pt(2026, 9, 29), pt(2026, 10, 6), pt(2026, 10, 13)])
}

@Test func splitACountSeriesKeepsTheTotal() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(rule: "FREQ=WEEKLY;BYDAY=TU;COUNT=6", organizer: nil))
    let source = try await h.source()
    _ = try await source.update(EventRef(try await occurrence(source, on: 22)), EventPatch(title: "Later"), scope: .thisAndFollowing, notify: .none)
    let head = try EventResource(data: Data(try #require(await h.server.body("home", "weekly.ics")).utf8))
    let tail = try EventResource(data: Data(try #require(await h.server.body("home", "uuid-2.ics")).utf8))
    #expect(head.master?.property("RRULE")?.value == "FREQ=WEEKLY;BYDAY=TU;COUNT=3")
    #expect(tail.master?.property("RRULE")?.value == "FREQ=WEEKLY;BYDAY=TU;COUNT=3")
    let events = try await source.events(in: autumn)
    #expect(events.map(\.start) == [pt(2026, 9, 1), pt(2026, 9, 16, 14), pt(2026, 9, 22), pt(2026, 9, 29), pt(2026, 10, 6)])
}

@Test func splitWithATimeChangeMovesTheNewSeries() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    let later = EventTiming(start: pt(2026, 9, 22, 15), end: pt(2026, 9, 22, 16), timeZone: pacific, isAllDay: false)
    _ = try await source.update(EventRef(try await occurrence(source, on: 22)), EventPatch(timing: later), scope: .thisAndFollowing, notify: .none)
    let starts = try await source.events(in: september).map(\.start)
    #expect(starts == [pt(2026, 9, 1), pt(2026, 9, 16, 14), pt(2026, 9, 22, 15), pt(2026, 9, 29, 15)])
}

@Test func splitAtTheFirstOccurrenceIsAllInSeries() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    let series = try await source.update(EventRef(try await occurrence(source, on: 1)), EventPatch(title: "Renamed"), scope: .thisAndFollowing, notify: .none)
    #expect(series.eventID == "weekly.ics")
    #expect(await h.server.names("home") == ["weekly.ics"])
    #expect(try await source.events(in: september).filter { $0.title == "Renamed" }.count == 3)
}

@Test func aFailedSecondStepRestoresTheOriginal() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    let before = try await source.events(in: september).map(\.start)
    await h.server.fail("PUT", pathContains: "uuid-2.ics", status: 507)
    await #expect(throws: SourceError.server(status: 507)) {
        _ = try await source.update(EventRef(try await occurrence(source, on: 22)), EventPatch(title: "x"), scope: .thisAndFollowing, notify: .none)
    }
    #expect(try await source.events(in: september).map(\.start) == before)
    #expect(await h.server.names("home") == ["weekly.ics"])
}

@Test func aStaleRestoreIsPartial() async throws {
    let h = CalDAVHarness()
    await h.server.store("home", "weekly.ics", weeklyICS(organizer: nil))
    let source = try await h.source()
    await h.server.fail("PUT", pathContains: "uuid-2.ics", status: 507)
    await h.server.fail("PUT", pathContains: "weekly.ics", status: 412, after: 1)   // the truncation passes, the restore is stale
    do {
        _ = try await source.update(EventRef(try await occurrence(source, on: 22)), EventPatch(title: "x"), scope: .thisAndFollowing, notify: .none)
        Issue.record("expected partial")
    } catch WriteError.partial(let message) {
        #expect(message.contains("not restored"))
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `swift test --package-path Packages/CalendarConnectors --filter SplitTests`
Expected: FAIL with `unsupported(fields: [recurrence])` from the stub (except `splitAtTheFirstOccurrenceIsAllInSeries`).

- [ ] **Step 3: Implement**

Replace `CalDAVCalendarSource+Split.swift` with:

```swift
import CalendarCore
import Foundation
import ICalendar

extension CalDAVCalendarSource {
    /// `.thisAndFollowing` after the first occurrence: (1) the original resource keeps the occurrences before `slot`
    /// (`If-Match` its ETag), (2) a new resource with a new UID holds the rest with the patch applied, carrying the
    /// later overrides and exclusions. If (2) fails, the original body goes back with `If-Match` on the ETag (1)
    /// returned, even when the caller was cancelled; a 412 there (someone edited meanwhile) is not overwritten and the
    /// result is `WriteError.partial`.
    func splitSeries(_ current: FetchedResource, ref: EventRef, patch: EventPatch, slot: Date, calendarZone: TimeZone) async throws -> PatchMerge.Attempt<CalendarEvent> {
        let stamp = now()
        let newUID = makeUUID()
        let newName = makeUUID() + ".ics"
        let parts = try SeriesEditor.split(current.resource, at: slot, newUID: newUID, calendarZone: calendarZone, now: stamp)
        let head = parts.head
        var tail = parts.tail
        guard var tailMaster = tail.master,
              let before = EventReader.timing(of: tailMaster, resolver: tail.resolver, calendarZone: calendarZone) else { throw WriteError.notFound }
        var tailPatch = patch
        var delta: TimeInterval = 0
        if let wanted = patch.timing {
            let moved = Self.seriesTiming(master: before, slot: slot, wanted: wanted, calendarZone: calendarZone)
            tailPatch.timing = moved.timing
            delta = moved.delta
        }
        try EventWriter.apply(tailPatch, to: &tailMaster, now: stamp, organizerAddress: organizerAddress)
        tailMaster.set(ICalProperty(name: "SEQUENCE", value: "0"))
        tail.setEvents([tailMaster] + tail.overrides)
        if delta != 0 { SeriesEditor.shift(&tail, by: delta, calendarZone: calendarZone) }
        if patch.recurrence != .keep { SeriesEditor.pruneUnmatched(&tail, calendarZone: calendarZone) }
        if let timing = patch.timing, let zone = timing.timeZone {
            tail.ensureTimeZones([zone], from: timing.start, through: timing.start.addingTimeInterval(20 * 366 * 86_400))
        }

        let truncatedETag: String
        switch try await put(head, to: current.url, ifMatch: current.etag) {
        case .stale: return .stale
        case .stored(let etag, _): truncatedETag = etag
        }

        let created: PutOutcome
        do {
            created = try await put(tail, to: resourceURL(calendarID: ref.calendarID, name: newName), ifNoneMatch: true)
            guard case .stored = created else { throw SourceError.invalidResponse("a resource with the new series' name already exists") }
        } catch {
            let original = current.resource
            let url = current.url
            // An unstructured task does not inherit the caller's cancellation.
            let restore = await Task { try await self.put(original, to: url, ifMatch: truncatedETag) }.result
            switch restore {
            case .success(.stored):
                throw error
            case .success(.stale):
                throw WriteError.partial("the series was cut short but the new series was not created (\(error)); it was changed meanwhile, so it was not restored")
            case .failure(let restoreError):
                throw WriteError.partial("the series was cut short but the new series was not created (\(error)); restoring it failed (\(restoreError))")
            }
        }
        guard case .stored(let etag, let copy) = created else { throw WriteError.notFound }
        return .done(try await readBack(copy, etag: etag, calendarID: ref.calendarID, name: newName, originalStart: nil))
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `swift test --package-path Packages/CalendarConnectors`
Expected: the whole package passes (Google, Microsoft, core, ICalendar, CalDAV).

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors/Sources/CalDAVCalendar Packages/CalendarConnectors/Tests/CalDAVCalendarTests
git commit -m "CalDAVCalendar: split a series with exact counts, moved exceptions and restore on failure"
```

---

### Task 15: The iCloud live smoke test

**Files:**
- Create: `Packages/CalendarConnectors/Tests/CalDAVCalendarTests/ICloudLiveSmokeTests.swift`

**Interfaces:**
- Consumes: `ICloudConnectorKind` (Task 11), `StubInteraction` (Task 11 harness), `CalDAVCalendarSource.account`, `.client`, `resourceURL(calendarID:name:)`, `calendarURL(_:)` (Task 11), `WebDAVClient.send`, `.report`, `DAVXML.syncCollection`, `DAVXML.isInvalidSyncToken` (Task 9), `checkForChanges()` (Task 12), writes (Tasks 13-14).
- Produces: nothing other tasks use. It answers the spec's "Risks (verify in the live test)" with `LIVE` lines.

The spec put this test in `CalendarApple`; it lives in `CalDAVCalendarTests` instead because it needs no Apple-only code (an
in-memory credential store is enough) and can then reach the source's internals with `@testable` for the raw probes. Task 17
records this in the spec's "As built".

- [ ] **Step 1: Write the test**

```swift
import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import CalDAVCalendar

private let liveICloud = ProcessInfo.processInfo.environment["TIMETUG_LIVE_ICLOUD"] == "1"

/// Every event this test creates starts with this prefix, and nothing without it is ever deleted.
private let smokePrefix = "TimeTug write smoke"

/// The only calendar the test writes to; the user creates it in the account first.
private let testCalendarTitle = "TimeTug Live Test"

/// Line 1 the Apple ID, line 2 an app-specific password, from a git-ignored file. Never printed.
private func liveCredentials() throws -> [String: String] {
    let url = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".config/timetug/icloud-live")
    let lines = try String(contentsOf: url, encoding: .utf8)
        .split(whereSeparator: \.isNewline)
        .map { $0.trimmingCharacters(in: .whitespaces) }
    guard lines.count >= 2, !lines[0].isEmpty, !lines[1].isEmpty else {
        throw SourceError.invalidResponse("~/.config/timetug/icloud-live needs two lines: the Apple ID, then an app-specific password")
    }
    return ["username": lines[0], "password": lines[1]]
}

private func utcStamp(_ date: Date) -> String {
    let formatter = DateFormatter()
    formatter.locale = Locale(identifier: "en_US_POSIX")
    formatter.timeZone = TimeZone(identifier: "UTC")
    formatter.dateFormat = "yyyyMMdd'T'HHmmss'Z'"
    return formatter.string(from: date)
}

/// A minimal event written byte for byte, to see what the server does with a body it did not produce.
private func probeICS(uid: String, start: Date, organizer: String? = nil, attendee: String? = nil) -> String {
    var lines = ["BEGIN:VCALENDAR", "VERSION:2.0", "PRODID:-//TimeTug//Live probe//EN", "BEGIN:VEVENT", "UID:\(uid)",
                 "DTSTAMP:\(utcStamp(Date()))", "DTSTART:\(utcStamp(start))", "DTEND:\(utcStamp(start.addingTimeInterval(1800)))",
                 "SUMMARY:\(smokePrefix) probe"]
    if let organizer { lines.append("ORGANIZER:\(organizer)") }
    if let attendee { lines.append("ATTENDEE;PARTSTAT=NEEDS-ACTION;RSVP=TRUE;SCHEDULE-AGENT=CLIENT:\(attendee)") }
    lines += ["END:VEVENT", "END:VCALENDAR"]
    return lines.joined(separator: "\r\n") + "\r\n"
}

/// Deletes the probe resources and every smoke event in `window` (a series once). Failures are printed, not thrown.
private func cleanUp(_ source: CalDAVCalendarSource, calendarID: String, window: DateInterval, probes: [URL]) async {
    await Task.detached {   // detached so it still runs when the test task was cancelled
        for url in probes { _ = try? await source.client.send("DELETE", url) }
        do {
            let mine = try await source.events(in: window).filter { $0.title.hasPrefix(smokePrefix) && $0.calendarID == calendarID }
            var seen = Set<String>()
            for event in mine {
                guard seen.insert(event.seriesID ?? event.eventID).inserted else { continue }
                do { try await source.delete(EventRef(event), scope: .allInSeries, notify: .none) }
                catch { print("LIVE cleanup could not delete a smoke event: \(error)") }
            }
        } catch { print("LIVE cleanup could not list events: \(error)") }
    }.value
}

/// Opt-in: `TIMETUG_LIVE_ICLOUD=1 swift test --package-path Packages/CalendarConnectors --filter iCloudLiveSmoke`.
/// Needs `~/.config/timetug/icloud-live` (the Apple ID, then an app-specific password) and a calendar named
/// "TimeTug Live Test" in that account; it writes only there. Prints `LIVE ...` lines, never the credentials, addresses or
/// calendar titles. Paste them into the Phase 5 spec's "Live findings":
/// - discovery: whether the principal is on a partition host, auto-schedule, and the count and schemes of the user addresses;
/// - each calendar's default flag, permissions, zone and whether it has a colour;
/// - whether events carry `lastModified` and `created` (decides two provided fields), and whether an edit changes the version;
/// - whether PUT answers with an ETag and whether the server rewrites the body;
/// - a weekly series of four: a `.thisInstance` edit, then a split at the third occurrence (expected `[2, 2]`);
/// - `checkForChanges` after the writes, and how the server answers an unknown sync token;
/// - with `TIMETUG_LIVE_ICLOUD_ATTENDEE=<an address you read>`: whether iCloud stamps `SCHEDULE-STATUS` on an attendee
///   marked `SCHEDULE-AGENT=CLIENT` (it should not; also check that mailbox for an invitation).
@Test(.enabled(if: liveICloud), .timeLimit(.minutes(10))) func iCloudLiveSmoke() async throws {
    let kind = ICloudConnectorKind()
    let credentials = InMemoryCredentialStore()
    let connection = try await kind.authorize(using: StubInteraction(try liveCredentials()), credentials: credentials)
    let made = try kind.makeSource(for: connection, credentials: credentials, syncState: InMemorySyncStateStore())
    let source = try #require(made as? CalDAVCalendarSource)
    let account = source.account
    let principalHost = account.principalURL.host?.lowercased() ?? ""
    let schemes = Set(account.userAddresses.compactMap { $0.split(separator: ":").first.map { $0.lowercased() } }).sorted()
    print("LIVE discovery partitionHost=\(principalHost != "caldav.icloud.com" && principalHost.hasSuffix(".icloud.com")) "
        + "homeOnPrincipalHost=\(account.homeURL.host == account.principalURL.host) autoSchedule=\(account.autoSchedule) "
        + "addresses=\(account.userAddresses.count) schemes=\(schemes)")

    let calendars = try await source.calendars()
    for calendar in calendars {
        print("LIVE calendar default=\(String(describing: calendar.isDefault)) canEdit=\(calendar.permissions.canEdit) "
            + "zone=\(calendar.timeZone?.identifier ?? "nil") color=\(calendar.colorHex != nil)")
    }
    let target = try #require(calendars.first { $0.title == testCalendarTitle }, "create a calendar named \(testCalendarTitle) in the account")
    _ = try await source.checkForChanges()   // the baseline

    let zone = target.timeZone ?? TimeZone(identifier: "America/Los_Angeles")!
    var local = Calendar(identifier: .gregorian)
    local.timeZone = zone
    let tomorrow = try #require(local.date(byAdding: .day, value: 1, to: Date()))
    let start = try #require(local.date(bySettingHour: 3, minute: 0, second: 0, of: tomorrow))
    func timing(_ offsetDays: Int) -> EventTiming {
        let s = start.addingTimeInterval(Double(offsetDays) * 86_400)
        return EventTiming(start: s, end: s.addingTimeInterval(1800), timeZone: zone, isAllDay: false)
    }
    // The series below ends about day 23 (weekly, four occurrences from day 2).
    let window = DateInterval(start: start.addingTimeInterval(-3600), duration: 86_400 * 30)
    var probes: [URL] = []

    await cleanUp(source, calendarID: target.id, window: window, probes: [])   // leftovers from an earlier run
    do {
        // 1. A single event: create, edit, read back.
        let single = try await source.create(EventDraft(title: smokePrefix, timing: timing(0), location: "Room 1"), in: target.id, notify: .none)
        print("LIVE single lastModified=\(single.lastModified != nil) created=\(single.created != nil) version=\(single.version != nil)")
        let renamed = try await source.update(EventRef(single), EventPatch(title: "\(smokePrefix) renamed"), scope: .thisInstance, notify: .none)
        #expect(renamed.title == "\(smokePrefix) renamed" && renamed.location == "Room 1")
        print("LIVE edit changed the version: \(renamed.version != single.version)")
        let reread = try await source.events(in: window).first { $0.eventID == renamed.eventID }
        print("LIVE reread lastModified=\(reread?.lastModified != nil) created=\(reread?.created != nil)")
        try await source.delete(EventRef(renamed), scope: .thisInstance, notify: .none)

        // 2. A raw PUT: does the server answer with an ETag, and does it keep the body it was given?
        let probeUID = "timetug-live-probe-\(UUID().uuidString)"
        let probeURL = source.resourceURL(calendarID: target.id, name: "\(probeUID).ics")
        probes.append(probeURL)
        let sent = Data(probeICS(uid: probeUID, start: start.addingTimeInterval(3600)).utf8)
        let put = try await source.client.send(
            "PUT", probeURL, headers: ["Content-Type": "text/calendar; charset=utf-8", "If-None-Match": "*"], body: sent)
        let got = try await source.client.send("GET", probeURL)
        print("LIVE put status=\(put.response.status) etag=\(put.response.header("ETag") != nil) "
            + "sameETagOnGET=\(put.response.header("ETag") == got.response.header("ETag")) bodyRewritten=\(got.response.body != sent)")
        _ = try await source.client.send("DELETE", probeURL)

        // 3. A weekly series of four: edit the second occurrence, then split at the third.
        let seriesTitle = "\(smokePrefix) series"
        _ = try await source.create(
            EventDraft(title: seriesTitle, timing: timing(2), recurrence: RecurrenceRule(frequency: .weekly, end: .count(4))),
            in: target.id, notify: .none)
        let occurrences = try await source.events(in: window).filter { $0.title == seriesTitle }.sorted { $0.start < $1.start }
        print("LIVE series occurrences=\(occurrences.count) originalStarts=\(occurrences.allSatisfy { $0.originalStart != nil })")
        try #require(occurrences.count == 4)
        _ = try await source.update(EventRef(occurrences[1]), EventPatch(title: "\(seriesTitle) moved"), scope: .thisInstance, notify: .none)
        let third = try #require(try await source.events(in: window).first { $0.originalStart == occurrences[2].originalStart })
        _ = try await source.update(EventRef(third), EventPatch(title: "\(seriesTitle) 2"), scope: .thisAndFollowing, notify: .none)
        let after = try await source.events(in: window).filter { $0.title.hasPrefix(seriesTitle) }
        let counts = Dictionary(grouping: after, by: { $0.seriesID ?? $0.eventID }).values.map(\.count).sorted()
        print("LIVE split instance counts per series = \(counts); edited occurrence kept: \(after.contains { $0.title == "\(seriesTitle) moved" })")
        #expect(counts == [2, 2])

        // 4. Change detection after our own writes, and an unknown sync token.
        print("LIVE checkForChanges after writes: \(String(describing: try await source.checkForChanges()))")
        do {
            let bogus = try await source.client.report(
                source.calendarURL(target.id), depth: 1, body: DAVXML.syncCollection(token: "https://caldav.icloud.com/sync/timetug-invalid"))
            print("LIVE unknown sync token status=\(bogus.response.status) recognised=\(DAVXML.isInvalidSyncToken(bogus.response))")
        } catch {
            print("LIVE unknown sync token threw: \(error)")
        }

        // 5. Optional: does iCloud honour SCHEDULE-AGENT=CLIENT?
        if let attendee = ProcessInfo.processInfo.environment["TIMETUG_LIVE_ICLOUD_ATTENDEE"], !attendee.isEmpty {
            let uid = "timetug-live-probe-\(UUID().uuidString)"
            let url = source.resourceURL(calendarID: target.id, name: "\(uid).ics")
            probes.append(url)
            let organizer = try #require(account.userAddresses.first { $0.lowercased().hasPrefix("mailto:") })
            let body = Data(probeICS(uid: uid, start: start.addingTimeInterval(7200), organizer: organizer, attendee: "mailto:\(attendee)").utf8)
            let stored = try await source.client.send(
                "PUT", url, headers: ["Content-Type": "text/calendar; charset=utf-8", "If-None-Match": "*"], body: body)
            let text = String(decoding: try await source.client.send("GET", url).response.body, as: UTF8.self)
            print("LIVE schedule-agent=client put=\(stored.response.status) scheduleStatus=\(text.contains("SCHEDULE-STATUS")) "
                + "agentKept=\(text.contains("SCHEDULE-AGENT=CLIENT"))")
            _ = try await source.client.send("DELETE", url)
        }
    } catch {
        print("LIVE smoke failed: \(error)")
        await cleanUp(source, calendarID: target.id, window: window, probes: probes)
        throw error
    }
    await cleanUp(source, calendarID: target.id, window: window, probes: probes)
}
```

- [ ] **Step 2: Check it is skipped without the switch**

Run: `swift test --package-path Packages/CalendarConnectors --filter iCloudLiveSmoke`
Expected: builds; the test is reported as skipped (no network traffic). Then run the whole package once more
(`swift test --package-path Packages/CalendarConnectors`): PASS.

- [ ] **Step 3: Commit**

```bash
git add Packages/CalendarConnectors/Tests/CalDAVCalendarTests/ICloudLiveSmokeTests.swift
git commit -m "CalDAVCalendar: opt-in iCloud live smoke test"
```

- [ ] **Step 4: Offer the live run (controller only, never a subagent)**

The live run touches the user's real iCloud account. Do not run it unprompted: ask the user whether to run it now. It needs
`~/.config/timetug/icloud-live` (two lines: Apple ID, then an app-specific password from account.apple.com > Sign-In and
Security > App-Specific Passwords) and a calendar named "TimeTug Live Test". Never read, print or paste that file's contents.
If they agree:

```bash
TIMETUG_LIVE_ICLOUD=1 swift test --package-path Packages/CalendarConnectors --filter iCloudLiveSmoke 2>&1 | grep -E "LIVE|passed|failed|error"
```

Paste the `LIVE` lines into the spec's "Live findings" (Task 17 commits the spec). If they decline, Task 17 leaves "Live
findings" saying the run is pending. A `LIVE` finding that contradicts the code (for example `bodyRewritten=true` with a
matching ETag, or counts other than `[2, 2]`) is a bug to fix before the PR, with a fake-server test that reproduces it.

---

## Part D: The app

### Task 16: The credential sheet and the two kinds in Settings > Accounts

**Files:**
- Create: `Apps/macOS/Sources/CredentialPrompter.swift`, `Apps/macOS/Sources/CredentialSheet.swift`
- Modify: `Apps/macOS/Sources/AccountsController.swift`, `AppCoordinator.swift`, `AppConnectors.swift`, `AccountsPane.swift`,
  `ProviderIcon.swift`, `SettingsSearch.swift`, `Apps/macOS/project.yml`
- Test: create `Apps/macOS/Tests/CredentialPrompterTests.swift`; modify `AccountsControllerTests.swift`, `AppConnectorsTests.swift`,
  `ProviderIconTests.swift`, `CalendarSectionsTests.swift`, `SettingsSearchTests.swift`

**Interfaces:**
- Consumes: `ICloudConnectorKind()`, `CalDAVConnectorKind()` (Task 11); `CredentialHelp`, `CredentialPromptHelp` (Task 1);
  `LoopbackAuthorizationInteraction(openURL:promptCredentials:presenter:...)` (existing, `CalendarApple`).
- Produces:
  ```swift
  @MainActor final class CredentialPrompter: ObservableObject {
      struct Request: Identifiable, Equatable { let id: UUID; var title: String; var fields: [CredentialField]; var help: CredentialHelp?; var values: [String: String]; var error: String? }
      @Published private(set) var request: Request?
      private(set) var lastNonSecretValues: [String: String]?
      func prepare(title: String, help: CredentialHelp?, values: [String: String] = [:], error: String? = nil)
      func prompt(_ fields: [CredentialField]) async throws -> [String: String]
      func submit(_ values: [String: String])
      func cancel()
  }
  struct CredentialSheet: View { static func isComplete(_ values: [String: String], fields: [CredentialField]) -> Bool }
  struct CredentialSheetPresenter: ViewModifier
  // AccountsController
  init(..., unconfiguredKindIDs: [String] = [], credentialPrompter: CredentialPrompter? = nil)
  let credentialPrompter: CredentialPrompter
  @Published private(set) var waitingText: String
  static func describeSignIn(_ error: Error, fields: [CredentialField]) -> String
  ```

The app target is Swift 5 mode with XCTest. Run app tests from `Apps/macOS`. `xcodegen generate` rewrites
`Apps/macOS/Sources/Info.plist` and `Apps/macOS/Widgets/Info.plist`; restore both with
`git checkout Apps/macOS/Sources/Info.plist Apps/macOS/Widgets/Info.plist` before every commit.

- [ ] **Step 1: Link the new product**

In `Apps/macOS/project.yml`, add CalDAVCalendar after `MicrosoftCalendar` in the `TimeTug` target's dependencies and again
in the `TimeTugTests` target's dependencies:

```yaml
      - package: CalendarConnectors
        product: CalDAVCalendar
```

- [ ] **Step 2: Write the failing prompter tests**

`Apps/macOS/Tests/CredentialPrompterTests.swift`:

```swift
import CalendarCore
import XCTest
@testable import TimeTug

/// Polls `condition` on the main actor for up to two seconds.
@MainActor
func waitUntil(_ condition: () -> Bool, file: StaticString = #filePath, line: UInt = #line) async throws {
    for _ in 0..<200 where !condition() { try await Task.sleep(for: .milliseconds(10)) }
    XCTAssertTrue(condition(), "timed out waiting", file: file, line: line)
}

@MainActor
final class CredentialPrompterTests: XCTestCase {
    private let fields = [
        CredentialField(key: "username", label: "Apple ID"),
        CredentialField(key: "password", label: "App-specific password", isSecret: true),
    ]

    func testPromptShowsThePreparedRequestAndReturnsWhatWasSubmitted() async throws {
        let prompter = CredentialPrompter()
        let help = CredentialHelp(text: "Use an app-specific password.", linkTitle: "Create one", url: URL(string: "https://account.apple.com"))
        prompter.prepare(title: "iCloud account", help: help, values: ["username": "me@icloud.test", "password": "never shown"], error: "Try again.")
        let answer = Task { try await prompter.prompt(fields) }
        try await waitUntil { prompter.request != nil }
        let request = try XCTUnwrap(prompter.request)
        XCTAssertEqual(request.title, "iCloud account")
        XCTAssertEqual(request.fields, fields)
        XCTAssertEqual(request.help, help)
        XCTAssertEqual(request.values, ["username": "me@icloud.test"])   // a secret is never prefilled
        XCTAssertEqual(request.error, "Try again.")
        prompter.submit(["username": "me@icloud.test", "password": "app-pass"])
        let values = try await answer.value
        XCTAssertEqual(values, ["username": "me@icloud.test", "password": "app-pass"])
        XCTAssertNil(prompter.request)
        XCTAssertEqual(prompter.lastNonSecretValues, ["username": "me@icloud.test"])
    }

    func testCancelEndsThePromptWithCancellation() async throws {
        let prompter = CredentialPrompter()
        let answer = Task { try await prompter.prompt(fields) }
        try await waitUntil { prompter.request != nil }
        prompter.cancel()
        do {
            _ = try await answer.value
            XCTFail("expected a cancellation")
        } catch {
            XCTAssertTrue(error is CancellationError)
        }
        XCTAssertNil(prompter.request)
        XCTAssertNil(prompter.lastNonSecretValues)
    }

    func testCancellingTheWaitingTaskClosesTheSheet() async throws {
        let prompter = CredentialPrompter()
        let answer = Task { try await prompter.prompt(fields) }
        try await waitUntil { prompter.request != nil }
        answer.cancel()
        do {
            _ = try await answer.value
            XCTFail("expected a cancellation")
        } catch {
            XCTAssertTrue(error is CancellationError)
        }
        try await waitUntil { prompter.request == nil }
    }

    func testPrepareForgetsTheLastSubmission() async throws {
        let prompter = CredentialPrompter()
        let answer = Task { try await prompter.prompt(fields) }
        try await waitUntil { prompter.request != nil }
        prompter.submit(["username": "a", "password": "b"])
        _ = try await answer.value
        XCTAssertNotNil(prompter.lastNonSecretValues)
        prompter.prepare(title: "Again", help: nil)
        XCTAssertNil(prompter.lastNonSecretValues)
    }

    func testSignInNeedsEveryField() {
        XCTAssertFalse(CredentialSheet.isComplete(["username": "me"], fields: fields))
        XCTAssertFalse(CredentialSheet.isComplete(["username": "  ", "password": "p"], fields: fields))
        XCTAssertTrue(CredentialSheet.isComplete(["username": "me", "password": "p"], fields: fields))
    }
}
```

- [ ] **Step 3: Write the failing controller, registry, icon, section and search tests**

In `AccountsControllerTests.swift`, replace `NoInteraction` (it becomes unused) with an interaction that answers through a
prompter, add a password kind, and let `makeController` register it:

```swift
private struct PrompterInteraction: AuthorizationInteraction {
    let prompter: CredentialPrompter
    func beginOAuthRedirect() async throws -> any OAuthRedirectSession { throw CalendarCore.SourceError.invalidResponse("unused") }
    func promptCredentials(_ fields: [CredentialField]) async throws -> [String: String] { try await prompter.prompt(fields) }
}

/// A `.password` kind like "Other CalDAV": it prompts, then accepts only `goodPassword` (or throws `failure`).
private final class PasswordKind: ConnectorKind, CredentialPromptHelp, @unchecked Sendable {
    let id = "caldav"
    let displayName = "Other CalDAV"
    let supportedPlatforms = Platform.macOS
    let fields = [
        CredentialField(key: "serverURL", label: "Server address"), CredentialField(key: "username", label: "User name"),
        CredentialField(key: "password", label: "Password", isSecret: true),
    ]
    var authorization: AuthorizationMethod { .password(fields: fields) }
    let credentialHelp: CredentialHelp? = CredentialHelp(text: "Use the address your provider gives you.")
    var goodPassword = "right"
    var failure: Error?
    private(set) var attempts = 0

    func authorize(using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        let values = try await signIn(interaction)
        let connection = Connection(kindID: id, connectionID: "p\(attempts)", displayName: values["username"] ?? "",
                                    config: ["serverURL": values["serverURL"] ?? "", "username": values["username"] ?? ""])
        try await credentials.setSecrets(["username": values["username"] ?? "", "password": values["password"] ?? ""], for: connection.connectionID)
        return connection
    }
    func reauthorize(_ connection: Connection, using interaction: any AuthorizationInteraction, credentials: any CredentialStore) async throws -> Connection {
        _ = try await signIn(interaction)
        return connection
    }
    private func signIn(_ interaction: any AuthorizationInteraction) async throws -> [String: String] {
        attempts += 1
        let values = try await interaction.promptCredentials(fields)
        if let failure { throw failure }
        guard values["password"] == goodPassword else { throw CalendarCore.SourceError.authExpired }
        return values
    }
    func makeSource(for connection: Connection, credentials: any CredentialStore, syncState: any SyncStateStore) throws -> any CalendarCore.CalendarSource {
        fatalError("controller tests build core sources through the reconciler closure")
    }
}
```

Replace `makeController` with:

```swift
    private func makeController(
        credentials override: (any CredentialStore)? = nil, password: PasswordKind? = nil, prompter: CredentialPrompter? = nil
    ) -> AccountsController {
        var registry = ConnectorRegistry()
        registry.register(kind)
        if let password { registry.register(password) }
        let prompter = prompter ?? CredentialPrompter()
        return AccountsController(
            registry: registry, connectionStore: connectionStore, credentials: override ?? credentials,
            syncState: syncState, interaction: PrompterInteraction(prompter: prompter), settings: settings, reconciler: reconciler,
            applySources: { [unowned self] sources in applied.append(sources.map(\.id)) },
            requestEventKitAccess: {}, credentialPrompter: prompter)
    }
```

Add these tests to `AccountsControllerTests`:

```swift
    private let typed = ["serverURL": "https://dav.example.test", "username": "me"]

    private func submit(_ prompter: CredentialPrompter, password: String) {
        prompter.submit(typed.merging(["password": password]) { $1 })
    }

    func testAPasswordSignInUsesTheSheetAndAddsTheAccount() async throws {
        let prompter = CredentialPrompter()
        let controller = makeController(password: PasswordKind(), prompter: prompter)
        controller.beginAddAccount(kindID: "caldav")
        try await waitUntil { prompter.request != nil }
        XCTAssertEqual(controller.waitingText, "Signing in…")
        let request = try XCTUnwrap(prompter.request)
        XCTAssertEqual(request.title, "Other CalDAV account")
        XCTAssertEqual(request.help?.text, "Use the address your provider gives you.")
        XCTAssertEqual(request.fields.map(\.key), ["serverURL", "username", "password"])
        XCTAssertNil(request.error)
        submit(prompter, password: "right")
        try await waitUntil { !controller.isWorking }
        XCTAssertEqual(controller.accounts.map(\.displayName), ["me"])
        XCTAssertNil(controller.errorMessage)
    }

    func testARejectedPasswordReopensTheSheetWithTheErrorAndWhatWasTyped() async throws {
        let password = PasswordKind()
        let prompter = CredentialPrompter()
        let controller = makeController(password: password, prompter: prompter)
        controller.beginAddAccount(kindID: "caldav")
        try await waitUntil { prompter.request != nil }
        submit(prompter, password: "wrong")
        try await waitUntil { prompter.request?.error != nil }
        let retry = try XCTUnwrap(prompter.request)
        XCTAssertEqual(retry.error, "User name or password was not accepted.")
        XCTAssertEqual(retry.values, typed)   // never the password
        XCTAssertTrue(controller.accounts.isEmpty)
        submit(prompter, password: "right")
        try await waitUntil { !controller.isWorking }
        XCTAssertEqual(controller.accounts.map(\.connectionID), ["p2"])
        XCTAssertEqual(password.attempts, 2)
        XCTAssertNil(controller.errorMessage)
    }

    func testOtherSignInFailuresReopenTheSheetWithTheirDescription() async throws {
        let password = PasswordKind()
        password.failure = CalendarCore.SourceError.invalidResponse("the server address must start with https://")
        let prompter = CredentialPrompter()
        let controller = makeController(password: password, prompter: prompter)
        controller.beginAddAccount(kindID: "caldav")
        try await waitUntil { prompter.request != nil }
        submit(prompter, password: "right")
        try await waitUntil { prompter.request?.error != nil }
        XCTAssertEqual(prompter.request?.error, "Sign-in failed: the server address must start with https://.")
        prompter.cancel()
        try await waitUntil { !controller.isWorking }
        XCTAssertTrue(controller.accounts.isEmpty)
        XCTAssertNil(controller.errorMessage)
    }

    func testCancellingTheSheetAddsNothingAndShowsNoError() async throws {
        let password = PasswordKind()
        let prompter = CredentialPrompter()
        let controller = makeController(password: password, prompter: prompter)
        controller.beginAddAccount(kindID: "caldav")
        try await waitUntil { prompter.request != nil }
        prompter.cancel()
        try await waitUntil { !controller.isWorking }
        XCTAssertTrue(controller.accounts.isEmpty)
        XCTAssertNil(controller.errorMessage)
        XCTAssertEqual(password.attempts, 1)
    }

    func testCancelInThePaneClosesTheSheet() async throws {
        let prompter = CredentialPrompter()
        let controller = makeController(password: PasswordKind(), prompter: prompter)
        controller.beginAddAccount(kindID: "caldav")
        try await waitUntil { prompter.request != nil }
        controller.cancelAuthorization()
        try await waitUntil { !controller.isWorking }
        XCTAssertNil(prompter.request)
        XCTAssertTrue(controller.accounts.isEmpty)
        XCTAssertNil(controller.errorMessage)
    }

    func testSignInAgainPrefillsTheSavedNonSecretFields() async throws {
        let prompter = CredentialPrompter()
        let controller = makeController(password: PasswordKind(), prompter: prompter)
        controller.beginAddAccount(kindID: "caldav")
        try await waitUntil { prompter.request != nil }
        submit(prompter, password: "right")
        try await waitUntil { !controller.isWorking }
        controller.beginReauthorize(connectionID: "p1")
        try await waitUntil { prompter.request != nil }
        XCTAssertEqual(prompter.request?.values, typed)
        submit(prompter, password: "right")
        try await waitUntil { !controller.isWorking }
        XCTAssertNil(controller.errorMessage)
    }

    func testBrowserSignInsSayTheyWaitForTheBrowser() async throws {
        let prompter = CredentialPrompter()
        let controller = makeController(password: PasswordKind(), prompter: prompter)
        controller.beginAddAccount(kindID: "caldav")
        try await waitUntil { prompter.request != nil }
        prompter.cancel()
        try await waitUntil { !controller.isWorking }
        await controller.addAccount(kindID: "google")
        XCTAssertEqual(controller.waitingText, "Waiting for your browser…")
    }

    func testOtherCalDAVIsOfferedLast() {
        let controller = makeController(password: PasswordKind())
        XCTAssertEqual(controller.availableKinds.map(\.id), ["google", "caldav"])
    }

    func testARejectedPasswordNamesTheKindsOwnFields() {
        let icloud = [CredentialField(key: "username", label: "Apple ID"), CredentialField(key: "password", label: "App-specific password", isSecret: true)]
        XCTAssertEqual(AccountsController.describeSignIn(CalendarCore.SourceError.authExpired, fields: icloud),
                       "Apple ID or app-specific password was not accepted.")
        XCTAssertEqual(AccountsController.describeSignIn(CalendarCore.SourceError.server(status: 500), fields: icloud),
                       "Sign-in failed (server(status: 500)).")
    }
```

In `AppConnectorsTests.swift`:

```swift
    func testICloudAndOtherCalDAVAreAlwaysRegistered() {
        let registry = AppConnectors.makeRegistry(google: nil, microsoft: nil, eventKit: EventKitSource())
        XCTAssertEqual(registry.kind(id: "icloud")?.displayName, "iCloud")
        XCTAssertEqual(registry.kind(id: "caldav")?.displayName, "Other CalDAV")
        guard case .password? = registry.kind(id: "icloud")?.authorization else { return XCTFail("iCloud signs in with a password") }
        XCTAssertNotNil((registry.kind(id: "icloud") as? CredentialPromptHelp)?.credentialHelp?.url)
    }
```

In `ProviderIconTests.swift`, replace `testUnknownProvidersFallBackToAGenericIcon` and add:

```swift
    func testICloudAndCalDAVAccountsGetTheirOwnMarks() {
        XCTAssertEqual(ProviderIcon.Style.forKind("icloud"), .icloud)
        XCTAssertEqual(ProviderIcon.Style.forKind("caldav"), .caldav)
    }

    func testUnknownProvidersFallBackToAGenericIcon() {
        XCTAssertEqual(ProviderIcon.Style.forKind("zoom"), .generic)
        XCTAssertEqual(ProviderIcon.Style.forKind(""), .generic)
    }
```

In `CalendarSectionsTests.swift`, change `testProviderNames` to:

```swift
    func testProviderNames() {
        XCTAssertEqual(ProviderIcon.displayName(forKindID: "google"), "Google")
        XCTAssertEqual(ProviderIcon.displayName(forKindID: "icloud"), "iCloud")
        XCTAssertEqual(ProviderIcon.displayName(forKindID: "caldav"), "CalDAV")
        XCTAssertEqual(ProviderIcon.displayName(forKindID: "zoom"), "Zoom")
    }
```

In `SettingsSearchTests.swift`:

```swift
    func testCalDAVProvidersFindTheAccountsPane() {
        for query in ["icloud", "caldav", "fastmail", "nextcloud"] {
            XCTAssertTrue(ids(query).contains("accounts"), query)
        }
    }
```

- [ ] **Step 4: Run to verify they fail**

Run (from `Apps/macOS`): `xcodegen generate && xcodebuild -project TimeTug.xcodeproj -scheme TimeTug -destination 'platform=macOS' test -only-testing:TimeTugTests/CredentialPrompterTests -only-testing:TimeTugTests/AccountsControllerTests -only-testing:TimeTugTests/AppConnectorsTests -only-testing:TimeTugTests/ProviderIconTests -only-testing:TimeTugTests/CalendarSectionsTests -only-testing:TimeTugTests/SettingsSearchTests`
Expected: the build fails (`CredentialPrompter`, `CredentialSheet`, `.icloud`, `waitingText`, `describeSignIn` and the
`credentialPrompter:` argument do not exist).

- [ ] **Step 5: Implement `CredentialPrompter.swift`**

```swift
import CalendarCore
import Foundation

/// The model behind the credential sheet. A `.password` connector kind's `promptCredentials` waits in `prompt` until the
/// user signs in or cancels. Before each attempt the accounts controller prepares the title, help, prefilled values and
/// the last error; the sheet shows `request` and answers with `submit` or `cancel`.
@MainActor
final class CredentialPrompter: ObservableObject {
    struct Request: Identifiable, Equatable {
        let id = UUID()
        var title: String
        var fields: [CredentialField]
        var help: CredentialHelp?
        /// Prefilled values, non-secret fields only.
        var values: [String: String]
        var error: String?
    }

    @Published private(set) var request: Request?
    /// The non-secret values of the form submitted since the last `prepare`; nil when none was submitted.
    private(set) var lastNonSecretValues: [String: String]?

    private var prepared: (title: String, help: CredentialHelp?, values: [String: String], error: String?) = ("", nil, [:], nil)
    private var continuation: CheckedContinuation<[String: String], Error>?

    func prepare(title: String, help: CredentialHelp?, values: [String: String] = [:], error: String? = nil) {
        prepared = (title, help, values, error)
        lastNonSecretValues = nil
    }

    /// Shows the sheet for `fields` and returns what the user entered. Throws `CancellationError` on Cancel, or when the
    /// calling task is cancelled (which also closes the sheet).
    func prompt(_ fields: [CredentialField]) async throws -> [String: String] {
        cancel()   // never leave an earlier prompt waiting
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                guard !Task.isCancelled else { return continuation.resume(throwing: CancellationError()) }
                self.continuation = continuation
                let nonSecret = Set(fields.filter { !$0.isSecret }.map(\.key))
                request = Request(title: prepared.title, fields: fields, help: prepared.help,
                                  values: prepared.values.filter { nonSecret.contains($0.key) }, error: prepared.error)
            }
        } onCancel: {
            Task { @MainActor in self.cancel() }
        }
    }

    func submit(_ values: [String: String]) {
        guard let shown = request, let waiting = continuation else { return }
        let nonSecret = Set(shown.fields.filter { !$0.isSecret }.map(\.key))
        lastNonSecretValues = values.filter { nonSecret.contains($0.key) }
        request = nil
        continuation = nil
        waiting.resume(returning: values)
    }

    func cancel() {
        request = nil
        guard let waiting = continuation else { return }
        continuation = nil
        waiting.resume(throwing: CancellationError())
    }
}
```

The `Task.isCancelled` check inside the continuation covers a cancellation that lands before the continuation is stored
(the handler's `cancel()` would then find nothing to resume).

- [ ] **Step 6: Implement `CredentialSheet.swift`**

```swift
import CalendarCore
import SwiftUI

/// The sign-in form for a connector kind that asks for a user name and password (iCloud, Other CalDAV): one field per
/// `CredentialField`, the kind's help below, and the last error at the top.
struct CredentialSheet: View {
    let request: CredentialPrompter.Request
    let onSubmit: ([String: String]) -> Void
    let onCancel: () -> Void
    @State private var values: [String: String]

    init(request: CredentialPrompter.Request, onSubmit: @escaping ([String: String]) -> Void, onCancel: @escaping () -> Void) {
        self.request = request
        self.onSubmit = onSubmit
        self.onCancel = onCancel
        _values = State(initialValue: request.values)
    }

    /// Sign In needs every field; spaces alone do not count.
    static func isComplete(_ values: [String: String], fields: [CredentialField]) -> Bool {
        fields.allSatisfy { !(values[$0.key] ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(request.title).font(.headline)
            if let error = request.error {
                Label(error, systemImage: "exclamationmark.triangle.fill")
                    .font(.callout)
                    .foregroundStyle(.orange)
            }
            Form {
                ForEach(request.fields, id: \.key) { field in
                    if field.isSecret {
                        SecureField(field.label, text: binding(for: field.key))
                    } else {
                        TextField(field.label, text: binding(for: field.key))
                            .autocorrectionDisabled()
                    }
                }
            }
            .formStyle(.columns)
            if let help = request.help {
                VStack(alignment: .leading, spacing: 4) {
                    Text(help.text)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    if let url = help.url {
                        Link(help.linkTitle ?? url.absoluteString, destination: url).font(.footnote)
                    }
                }
            }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel, action: onCancel)
                    .keyboardShortcut(.cancelAction)
                Button("Sign In") { onSubmit(values) }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!Self.isComplete(values, fields: request.fields))
            }
        }
        .padding(20)
        .frame(width: 400)
    }

    private func binding(for key: String) -> Binding<String> {
        Binding(get: { values[key] ?? "" }, set: { values[key] = $0 })
    }
}

/// Shows the credential sheet while `prompter` has a request. Closing it any way other than Sign In cancels the sign-in.
struct CredentialSheetPresenter: ViewModifier {
    @ObservedObject var prompter: CredentialPrompter

    func body(content: Content) -> some View {
        content.sheet(item: Binding(get: { prompter.request }, set: { if $0 == nil { prompter.cancel() } })) { request in
            CredentialSheet(request: request, onSubmit: { prompter.submit($0) }, onCancel: { prompter.cancel() })
        }
    }
}
```

- [ ] **Step 7: Change `AccountsController.swift`**

Add the published waiting text and the prompter next to the other properties:

```swift
    /// What the pane shows while a sign-in runs: a password sign-in happens in the app, an OAuth one in the browser.
    @Published private(set) var waitingText = "Waiting for your browser…"

    /// Shows the credential sheet for `.password` kinds; the same instance answers `promptCredentials` (see AppCoordinator).
    let credentialPrompter: CredentialPrompter
```

Extend `init` with a last parameter `credentialPrompter: CredentialPrompter? = nil` and set
`self.credentialPrompter = credentialPrompter ?? CredentialPrompter()`.

Replace `availableKinds` so the catch-all kind comes last in the "+" grid (the registry sorts by id, which would put
"caldav" first):

```swift
    /// Account kinds offered by `+` (system-permission kinds such as Apple Calendar are a switch, not an account).
    /// "Other CalDAV" goes last: it is the catch-all for providers without their own entry.
    var availableKinds: [any ConnectorKind] {
        let kinds = registry.kinds(for: .current).filter { if case .system = $0.authorization { false } else { true } }
        return kinds.filter { $0.id != "caldav" } + kinds.filter { $0.id == "caldav" }
    }
```

In `addAccount`, replace `let connection = try await kind.authorize(using: interaction, credentials: credentials)` with:

```swift
            let connection = try await signIn(with: kind, prefill: [:]) {
                try await kind.authorize(using: interaction, credentials: credentials)
            }
```

In `reauthorize`, replace `let updated = try await kind.reauthorize(connection, using: interaction, credentials: credentials)` with:

```swift
            let updated = try await signIn(with: kind, prefill: connection.config) {
                try await kind.reauthorize(connection, using: interaction, credentials: credentials)
            }
```

Add the loop and the text (below `cancelAuthorization`):

```swift
    /// Runs one sign-in. A `.password` kind signs in through the credential sheet: when a submitted form fails, the sheet
    /// opens again with the error and what the user typed (never the password), until it succeeds or the user cancels.
    /// `prefill` may hold any connection config; only the kind's non-secret fields are shown.
    private func signIn(
        with kind: any ConnectorKind, prefill: [String: String], _ attempt: () async throws -> Connection
    ) async throws -> Connection {
        guard case .password(let fields) = kind.authorization else {
            waitingText = "Waiting for your browser…"
            return try await attempt()
        }
        waitingText = "Signing in…"
        let help = (kind as? CredentialPromptHelp)?.credentialHelp
        var values = prefill
        var message: String?
        while true {
            credentialPrompter.prepare(title: "\(kind.displayName) account", help: help, values: values, error: message)
            do {
                return try await attempt()
            } catch let error as CancellationError {
                throw error
            } catch {
                if Task.isCancelled { throw CancellationError() }
                guard let typed = credentialPrompter.lastNonSecretValues else { throw error }   // failed before the form
                values = typed
                message = Self.describeSignIn(error, fields: fields)
            }
        }
    }

    /// "Apple ID or app-specific password was not accepted." for a rejected password, in the kind's own words;
    /// the usual description otherwise.
    static func describeSignIn(_ error: Error, fields: [CredentialField]) -> String {
        guard case CalendarCore.SourceError.authExpired = error,
              let name = fields.first(where: { !$0.isSecret && $0.key == "username" }) ?? fields.first(where: { !$0.isSecret }),
              let secret = fields.first(where: \.isSecret) else { return describe(error) }
        return "\(name.label) or \(secret.label.prefix(1).lowercased() + secret.label.dropFirst()) was not accepted."
    }
```

Give `describe` a case for a readable server message, before the general `SourceError` case:

```swift
    private static func describe(_ error: Error) -> String {
        switch error {
        case CalendarCore.SourceError.authExpired: "Sign-in was not completed."
        case CalendarCore.SourceError.invalidResponse(let message): "Sign-in failed: \(message)."
        case let e as CalendarCore.SourceError: "Sign-in failed (\(e))."
        default: error.localizedDescription
        }
    }
```

- [ ] **Step 8: Wire the app**

`AppConnectors.swift`: add `import CalDAVCalendar` and register both kinds unconditionally (they need no client id):

```swift
    static func makeRegistry(google: GoogleOAuthConfig?, microsoft: MicrosoftOAuthConfig?, eventKit: EventKitSource) -> ConnectorRegistry {
        var registry = ConnectorRegistry()
        registry.register(EventKitConnectorKind(source: eventKit))
        registry.register(ICloudConnectorKind())
        registry.register(CalDAVConnectorKind())
        if let google { registry.register(GoogleConnectorKind(config: google, hasher: CryptoKitSHA256())) }
        if let microsoft { registry.register(MicrosoftConnectorKind(config: microsoft, hasher: CryptoKitSHA256())) }
        return registry
    }
```

`AppCoordinator.swift`: add `private let credentialPrompter = CredentialPrompter()` above `lazy var accounts`, pass the
prompt closure to the interaction and the prompter to the controller:

```swift
        interaction: LoopbackAuthorizationInteraction(
            openURL: { url in await MainActor.run { NSWorkspace.shared.open(url) } },
            promptCredentials: { [credentialPrompter] fields in try await credentialPrompter.prompt(fields) },
            presenter: BrowserPreferringPresenter(
                sheet: WebAuthenticationSessionPresenter(anchor: { [weak self] in self?.settingsWindow.currentWindow ?? NSApp.keyWindow }))),
```

and end the `AccountsController(...)` call with `unconfiguredKindIDs: unconfiguredKindIDs, credentialPrompter: credentialPrompter)`.

`AccountsPane.swift`:
- remove `.init(name: "iCloud", systemImage: "icloud"),` and `.init(name: "Other CalDAV", systemImage: "link"),` from
  `placeholderProviders` (they are real kinds now; Fastmail stays as a "Soon" tile);
- in `waitingRow`, show `Text(accounts.waitingText).font(.callout)` instead of the fixed text;
- after `.settingsHighlight("accounts", navigation: navigation)` add
  `.modifier(CredentialSheetPresenter(prompter: accounts.credentialPrompter))`.

`ProviderIcon.swift`:

```swift
    enum Style: Equatable {
        case google, microsoft, icloud, caldav, generic

        static func forKind(_ kindID: String) -> Style {
            switch kindID {
            case "google": .google
            case "microsoft": .microsoft
            case "icloud": .icloud
            case "caldav": .caldav
            default: .generic
            }
        }
    }
```

```swift
    /// "google" gives "Google"; names with their own capitalisation are spelled out; the rest are just capitalised.
    static func displayName(forKindID kindID: String) -> String {
        switch kindID {
        case "icloud": "iCloud"
        case "caldav": "CalDAV"
        default: kindID.prefix(1).uppercased() + kindID.dropFirst()
        }
    }
```

and two cases in `body`'s switch (SF Symbols, no brand artwork; the white tile stays for every non-generic style):

```swift
            case .icloud:
                Image(systemName: "icloud.fill")
                    .resizable().scaledToFit().foregroundStyle(Color(red: 0.24, green: 0.56, blue: 0.98)).padding(size * 0.18)
            case .caldav:
                Image(systemName: "calendar")
                    .resizable().scaledToFit().foregroundStyle(Color(white: 0.35)).padding(size * 0.2)
```

`SettingsSearch.swift`: add `"caldav", "fastmail", "nextcloud"` to the `accounts` entry's keywords after `"icloud"`.

- [ ] **Step 9: Run the app tests**

Run (from `Apps/macOS`): `xcodegen generate && xcodebuild -project TimeTug.xcodeproj -scheme TimeTug -destination 'platform=macOS' test`
Expected: all tests pass. Then `git checkout Apps/macOS/Sources/Info.plist Apps/macOS/Widgets/Info.plist`.

- [ ] **Step 10: Look at it**

Launch the built app (not with `pkill -x TimeTug` to restart it; quit it from its menu). Open Settings > Accounts: "iCloud"
and "Other CalDAV" are real tiles (Other CalDAV last), the "Soon" tiles no longer list them, and "+ iCloud" opens the sheet
with the two fields, the help text and the link; Sign In is disabled until both fields have text; Cancel closes it with no
error. Do not sign in to a real account here; that is the user's manual checklist.

- [ ] **Step 11: Commit**

```bash
git add Apps/macOS/Sources Apps/macOS/Tests Apps/macOS/project.yml
git status --short Apps/macOS   # Info.plist files must not be listed
git commit -m "App: iCloud and Other CalDAV accounts with a generic credential sheet"
```

---

## Part E: Docs, verification, pull request

### Task 17: Documentation, full verification and the PR

**Files:**
- Create: `docs/decisions/0016-icalendar-and-client-side-expansion.md`
- Modify: `docs/calendar-connectors-api.md`, `docs/decisions/0014-reminder-model.md`, `docs/decisions/0015-recurrence-model.md`,
  `AGENTS.md`, `.agents/skills/running-live-calendar-tests/SKILL.md`, `.agents/skills/configuring-local-app-builds/SKILL.md`,
  `docs/superpowers/specs/2026-09-27-calendar-connectors-phase5-caldav-design.md`

**Interfaces:**
- Consumes: everything above. Every type and function name written into the docs must be checked against the code as
  built (`grep -rn "public " Packages/CalendarConnectors/Sources/ICalendar Packages/CalendarConnectors/Sources/CalDAVCalendar`);
  where a task renamed something, the docs follow the code.
- Produces: the PR (not merged).

Write in the docs' existing voice: plain sentences, facts, no marketing words, no "simply" or "just".

- [ ] **Step 1: ADR 0016**

`docs/decisions/0016-icalendar-and-client-side-expansion.md`:

```markdown
# ADR 0016: iCalendar and recurrence expansion in the library

**Status:** Accepted

## Context

CalDAV (RFC 4791) stores each event as a whole iCalendar resource: the series master with its `RRULE`, `RDATE` and
`EXDATE`, plus one `VEVENT` per changed occurrence. Google and Microsoft expand a series on the server; a CalDAV server
may (`<C:expand>`), but support is uneven and an expanded answer drops the master and the rule, which series reads and
splits need. ADR 0015 said the library does no expansion because every provider did it. iCloud through EventKit was
also considered and rejected: it is not portable and cannot invite or reply (ADR 0012).

## Decision

- **The library expands recurrence itself.** `RecurrenceRule.instances(anchor:timeZone:isAllDay:before:limit:skipTo:)`
  expands one rule in the series' zone (wall-clock times, DST gaps moved forward, repeated times take the first);
  `RecurrenceSet.occurrences(anchor:duration:timeZone:isAllDay:overlapping:limit:)` combines rules, `RDATE` and `EXDATE`
  for a window. Limits: 5000 emitted instances per resource per query, 200,000 periods per rule; a series that started
  years ago fast-forwards to the window instead of counting from its start (except with `COUNT`). A rule the library
  cannot read shows the first occurrence only.
- **`ICalendar` is its own product,** depending only on `CalendarCore`: a byte-level parser and a serializer (75-octet
  folding that never splits a UTF-8 character), a component tree that keeps unknown properties and parameters, time zone
  resolution (IANA, Windows and Mozilla-prefixed names, `VTIMEZONE` rule matching, then a fixed offset) and `VTIMEZONE`
  output, and the `VEVENT` ↔ `CalendarEvent`, `VALARM` ↔ `Reminder` and attendee mappings. A write edits the stored tree in
  place, so properties TimeTug does not model survive.
- **`CalDAVCalendar`** depends on `CalendarCore` and `ICalendar` and parses XML with `XMLParser` (`FoundationXML` on Linux).

## Consequences

- One connector serves iCloud and any other CalDAV server, and a future ICS subscription connector can reuse `ICalendar`.
- Expansion bugs are now ours: the expander is tested against every RFC 5545 section 3.8.5.3 example, DST changes and
  all-day series, and the conformance suites run against a fake CalDAV server.
- The library still has no external dependencies and builds on Linux.
```

- [ ] **Step 2: Amend ADRs 0014 and 0015**

In `docs/decisions/0014-reminder-model.md`, change the deferred line to:

```markdown
- Deferred: snooze and acknowledged state. (The `VALARM` reader and writer arrived with the CalDAV connector, in
  `ICalendar`'s `AlarmMapper`, with its round-trip test; see ADR 0016.)
```

In `docs/decisions/0015-recurrence-model.md`, append to the `RecurrenceSet` bullet: `Amended by ADR 0016: the library now
expands rules itself (RecurrenceRule.instances, RecurrenceSet.occurrences) for CalDAV; Google, Microsoft and EventKit
still expand on the provider.` and remove the sentence "There is no rule expansion: providers expand occurrences on read."

- [ ] **Step 3: The API contract**

In `docs/calendar-connectors-api.md`:

1. Status paragraph: "part 12 describes the Microsoft connector (Phase 4); part 13 describes `ICalendar` and the CalDAV
   and iCloud connector (Phase 5)."
2. Part 1 table, after the `MicrosoftCalendar` row:

   ```markdown
   | `CalendarConnectors` / `ICalendar` | iCalendar (RFC 5545) parser and writer, time zones, `VEVENT`/`VALARM` mapping, series editing | `CalendarCore` | Portable |
   | `CalendarConnectors` / `CalDAVCalendar` | CalDAV connector (iCloud and other servers) over a small WebDAV client | `CalendarCore`, `ICalendar` | Portable; `FoundationXML` on Linux |
   ```

   and the dependency sentence: "`GoogleCalendar` and `MicrosoftCalendar` → `CalendarOAuth` → `CalendarCore`;
   `CalDAVCalendar` → `ICalendar` → `CalendarCore`; ...".
3. Part 3 capability table, a row after EventKit:

   ```markdown
   | CalDAV (iCloud, other) | true | true when the server schedules (`calendar-auto-schedule`) and the account has addresses | same as `canEditAttendees` | true (`participation` only with addresses) | `.token` (`sync-collection`, else ctag) | all but `conference` (attendees only when scheduling) | false | all three |
   ```

4. Part 10, the `NotifyPolicy` paragraph: add "CalDAV: only when the write tells nobody else (no attendees other than the
   account on create, update and delete); `respond` always tells the organizer, so it needs `.all`." Wherever part 10 or
   the Recurrence section says the library does no expansion, say that `CalDAVCalendar` expands client-side with
   `RecurrenceSet.occurrences` (ADR 0016).
5. Part 11: replace "No CalDAV connector yet" with:

   ```markdown
   - **CalDAV behavior beyond the fake server (unverified until the live run).** Everything in part 13 is tested against
     an in-memory CalDAV server; the opt-in iCloud smoke test and the manual checklist in the Phase 5 spec confirm it
     against iCloud. Other servers (Fastmail, Nextcloud) are untested live. `SCHEDULE-AGENT=CLIENT` is not used, so a
     write that would email someone needs `NotifyPolicy.all`.
   ```

6. A new part 13 after part 12 (before "Calendar identity and permissions"):

   ```markdown
   ## 13. `ICalendar` and `CalDAVCalendar` (Phase 5)

   `docs/superpowers/specs/2026-09-27-calendar-connectors-phase5-caldav-design.md` is the design; ADR 0016 records why
   iCalendar and expansion live in the library. Public surface: `ICloudConnectorKind(transport:now:sleep:pollInterval:)`,
   `CalDAVConnectorKind(transport:now:sleep:pollInterval:)`, the `CalDAVCalendarSource` returned by `makeSource` (a
   `WritableCalendarSource`, a `SeriesSource` and a `PollingCalendarSource`), and the `ICalendar` product. The WebDAV
   client, XML and discovery are internal.

   - **Kinds:** `icloud` ("iCloud", server `https://caldav.icloud.com`, credentials allowed to `icloud.com` and its
     subdomains, provider `.iCloud`) and `caldav` ("Other CalDAV", server entered, credentials allowed to that host and its
     subdomains, provider `.calDAV`). Both use service `.calDAV`, all platforms, and `.password(fields:)`: `username` and
     `password` (secret), plus `serverURL` for `caldav`. Both adopt `CredentialPromptHelp` (help text; iCloud links to
     account.apple.com for app-specific passwords). A server address without a scheme gets `https://`; plain `http` is
     refused except to `localhost` and `127.0.0.1`.
   - **Sign-in:** the kind prompts once, then discovers: `PROPFIND` on `/.well-known/caldav` (then the entered URL on 404,
     405 or 501), `current-user-principal`, then `calendar-home-set` and `calendar-user-address-set`, then `OPTIONS` for the
     `DAV:` header. Nothing is stored until discovery succeeds. `Connection.config` holds `serverURL`, `username`,
     `principalURL`, `homeURL`, `userAddresses` (newline-separated) and `autoSchedule`; the credential store holds
     `username` and `password`. `displayName` is the user name when it contains "@", else `user@host`. `reauthorize`
     throws `invalidResponse("signed in as a different account")` when the principal or host differs. A 401 is
     `.authExpired`. Retrying a rejected password is the host's job (TimeTug's sheet reopens with the error).
   - **Transport and security:** Basic auth, UTF-8, only over HTTPS (or the loopback exception) and only to the allowed
     hosts. The transport does not follow redirects (`URLSessionTransport(followsRedirects: false)`); `WebDAVClient`
     follows up to 5 itself (303 becomes `GET`), checks every hop and every `href` against the host rule, and refuses
     anything else with `invalidResponse`. 429 and 503 are `.rateLimited`, other 5xx `.server`. The password is never
     in an error or a log.
   - **Calendars:** collections under the home with the `calendar` resource type that accept `VEVENT`. The id is the
     path below the home without the trailing slash (stable across partition hosts). Colour from
     `calendar-color`, zone from `calendar-timezone` (else the source's default zone), permissions from
     `current-user-privilege-set`. `isDefault` comes from the inbox's `schedule-default-calendar-URL` when the server
     has one, else nil.
   - **Events:** `calendar-query` for the window, then client-side expansion (ADR 0016). A resource with overrides and no
     master (an invitation to one occurrence) shows its overrides. `eventID` is the resource name, plus the original start
     for an occurrence; `version` is the ETag; `uidScope` is `.global`. Cancelled `VEVENT`s are dropped.
   - **Change detection:** `PROPFIND` depth 1 on the home for `getctag` and `sync-token`; a changed calendar set is
     `.calendarsChanged`; a changed token runs `sync-collection` and reports `.eventsChanged(calendarIDs:)`. An invalid
     token re-baselines and reports the calendar changed; a server without `sync-token` falls back to the ctag. The first
     call is the baseline.
   - **Writes:** every write is a whole-resource `PUT` with `If-Match` (create uses `If-None-Match: *` and a new
     `<uuid>.ics`), driven by `PatchMerge`; a stale write retries through `PatchMerge` and conflicts only on the fields both
     sides changed. `.thisInstance` adds or edits an override; deleting an occurrence adds an `EXDATE` (with `If-Match`,
     retried three times, then `conflict([.recurrence])`); `.allInSeries` delete is a plain `DELETE`. `.thisAndFollowing`
     after the first occurrence truncates the master (`UNTIL`, or `COUNT` minus the instances before the split) and
     creates a new resource with a new UID that carries the later overrides and `EXDATE`s (shifted when the start moves);
     if the new resource cannot be stored the original is restored, and a failed or stale restore is `WriteError.partial`.
     A draft's `uid` is used for the duplicate check (a `calendar-query` by UID; a hit is `alreadyExists`). `conference`
     is not writable. `controlsNotifications == false`: the server schedules on its own, so a write that would tell
     someone else with a policy other than `.all` throws `unsupported(fields: [.attendees])` before any request.
     `respond` sets the account's `PARTSTAT` in the organizer's copy and needs `canRespondToInvite`.
   - **`SeriesSource`:** `series(id:calendarID:)` GETs the resource and returns the master's rules, `RDATE`s and
     `EXDATE`s; an unknown id or a non-recurring resource is `.notFound`.
   - **Testing.** `CalDAVCalendarTests` run against `FakeCalDAVServer` (an in-memory `HTTPTransport`) with
     `WritableSourceConformance`, `AllDayConformance` and `ProvidedFieldsConformance`. The live smoke test is opt-in and
     never runs in CI: `TIMETUG_LIVE_ICLOUD=1 swift test --package-path Packages/CalendarConnectors --filter iCloudLiveSmoke`
     (credentials from `~/.config/timetug/icloud-live`; writes only to a calendar named "TimeTug Live Test").
   ```

   Check each statement in part 13 against the code; fix the doc, not the code, where they differ (and tell the
   controller when a difference looks like a bug).

- [ ] **Step 4: AGENTS.md and the skills**

`AGENTS.md`:
- in the `Packages/CalendarConnectors` line, list the new products after `GoogleCalendar`: "`MicrosoftCalendar` (Microsoft
  connector), `ICalendar` (iCalendar parser, writer and event mapping) and `CalDAVCalendar` (CalDAV connector with an
  iCloud preset)" (add `MicrosoftCalendar` only if the line does not name it yet);
- in the dependency rule, add `ICalendar -> CalendarCore; CalDAVCalendar -> CalendarCore + ICalendar` and add
  `CalDAVCalendar` to the connector products the app depends on;
- skill descriptions: `running-live-calendar-tests` "…against real Apple Calendar, Google, Microsoft and iCloud accounts";
  `configuring-local-app-builds` stays about OAuth, adding "(iCloud and Other CalDAV need no configuration)".

`.agents/skills/running-live-calendar-tests/SKILL.md`: add "iCloud" to the description's provider list, a table row

```markdown
| iCloud | `TIMETUG_LIVE_ICLOUD=1 swift test --package-path Packages/CalendarConnectors --filter iCloudLiveSmoke` |
```

and these notes:

```markdown
- iCloud needs no interactive sign-in: the test reads `~/.config/timetug/icloud-live` (line 1 the Apple ID, line 2 an
  app-specific password from account.apple.com > Sign-In and Security). Never read, print or paste that file. The user
  creates a calendar named "TimeTug Live Test"; the test writes only there and deletes what it made. Optional:
  `TIMETUG_LIVE_ICLOUD_ATTENDEE=<an address the user reads>` checks whether iCloud honors `SCHEDULE-AGENT=CLIENT`.
- Paste the `LIVE` lines into the Phase 5 spec's "Live findings".
```

`.agents/skills/configuring-local-app-builds/SKILL.md`: under "Google and Microsoft accounts" add one line: "iCloud and
Other CalDAV are always registered (no client id). If they are missing from Settings > Accounts, check
`AppConnectors.makeRegistry`."

- [ ] **Step 5: The spec**

In `docs/superpowers/specs/2026-09-27-calendar-connectors-phase5-caldav-design.md`:
- Status line: "implemented on branch `<current branch>`; see As built."
- Add an "## As built" section before "Live findings":

```markdown
## As built

Differences from the design above:

- The iCloud live smoke test is in `CalDAVCalendarTests` (`iCloudLiveSmoke`), not `CalendarApple`: it needs no Apple-only
  code and uses the source's internals for raw probes. Run it with
  `TIMETUG_LIVE_ICLOUD=1 swift test --package-path Packages/CalendarConnectors --filter iCloudLiveSmoke`.
- `URLSessionTransport(configuration:followsRedirects:)` instead of `(session:followsRedirects:)` (a session's delegate
  is fixed when it is made).
- `CredentialPromptHelp.credentialHelp` is a `CredentialHelp` struct (`text`, `linkTitle`, `url`) instead of a tuple.
- `FakeCalDAVServer` is its own `HTTPTransport`, not built on `FakeTransport` (it keeps resources, ETags and sync tokens).
- `series(id:)` reads the master through `EventReader` and its time zone resolver, not
  `RecurrenceSet(iCalendarLines:)`, so non-IANA `TZID`s resolve the same way as in reads.
- A patch is compared (`PatchMerge`) with the occurrence at `originalStart` whenever the ref has one, else the master.
- The "Other CalDAV" display name is the user name alone when it already contains "@".
- `canRespondToInvite` also needs `userAddresses` (without them the account's attendee entry cannot be found).
- Changing a series between all-day and timed while it has changed occurrences throws `unsupported([.timing])`.
- The architecture check (`scripts/ci/check-architecture.sh`) now allows `FoundationXML` in the connector library.
```

- In "Live findings", paste the `LIVE` lines from Task 15 Step 4, or write "Pending: the opt-in live run has not been
  done yet." if the user declined.

- [ ] **Step 6: Verify everything**

Run each and read the output (do not trust an earlier run):

```bash
swift test --package-path Packages/CalendarConnectors
swift test --package-path Packages/CalendarApple
swift test --package-path Packages/CalendarBridge
swift test --package-path Packages/TimeTugCore
scripts/ci/check-architecture.sh && bash scripts/ci/tests/test-check-architecture.sh
docker run --rm -v "$PWD":/w -w /w swift:6.0 swift test --package-path Packages/CalendarConnectors --scratch-path /w/.build/linux
```

Then from `Apps/macOS`: `xcodegen generate && xcodebuild -project TimeTug.xcodeproj -scheme TimeTug -destination 'platform=macOS' test`,
then `git checkout Apps/macOS/Sources/Info.plist Apps/macOS/Widgets/Info.plist`.
Expected: everything passes; the Linux run proves `FoundationXML`, the redirect delegate and the zone transitions work on
swift-corelibs-foundation. If Docker is not running, say so in the PR body instead of claiming Linux passed (the
`core-linux` CI job runs it too). Also check `git status --short` shows no stray files (`.build/` is ignored).

- [ ] **Step 7: Commit the docs**

```bash
git add docs AGENTS.md .agents/skills
git commit -m "Docs: Phase 5 CalDAV and iCloud (ADR 0016, API contract part 13, skills, as built)"
```

- [ ] **Step 8: DeepSeek review of the branch (controller)**

Use the `deepseek-review` skill. Files: every changed Swift source under `Packages/CalendarConnectors/Sources/ICalendar`,
`Sources/CalDAVCalendar`, `Sources/CalendarCore/Recurrence`, the changed app sources, and the matching tests; context: the
spec and part 13 of the API doc. Background: the facts in "Global Constraints". Questions, one risk each:
- "Trace a `.thisAndFollowing` update on the third occurrence of a `COUNT=6` series whose fifth occurrence is moved: what
  are the head's and tail's `RRULE`s, where does the moved override end up, and what does a failed tail PUT leave?"
- "Can any path send the `Authorization` header to a host other than the base or its subdomains, including after a
  redirect or through an `href` in a multistatus?"
- "Trace a weekly `America/Los_Angeles` series at 01:30 across the November DST change: which instants does the
  expander emit, and does an all-day series keep its dates?"
- "Can a write with `NotifyPolicy.none` reach the server when the event has attendees other than the account?"
- "In `CredentialPrompter`, can a prompt hang forever or resume its continuation twice (cancel racing submit, task
  cancellation before the continuation is stored)?"

Treat every finding as unverified: check each Critical or Important one against the code, fix the real ones with a test
first, commit, and tell the user how many findings were real and which were false.

- [ ] **Step 9: Push and open the PR (not merged)**

```bash
git push -u origin HEAD
gh pr create --base master --title "Calendar connectors Phase 5: CalDAV and iCloud" --body-file "$SCRATCH/pr-body.md"
```

The body (written to the scratchpad first) covers: what users get (iCloud and Other CalDAV in Settings > Accounts, the
sign-in sheet); the three layers (expander in `CalendarCore`, `ICalendar`, `CalDAVCalendar`); the notable decisions
(`controlsNotifications = false` and the refusal rule, client-side expansion, redirects followed by the client); what
was verified (each command in Step 6 with its result, the live run or "pending", the DeepSeek review outcome); the
spec's manual checklist for the user; and ends with the line `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
Then bind the PR with the desktop PR tools (`get_status`, `bind_pr` if needed) and report the link. Do not merge: the user
squash-merges when ready.
