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

    func source(autoSchedule: Bool = true, userAddresses: [String] = defaultAddresses, transport: (any HTTPTransport)? = nil) async throws -> CalDAVCalendarSource {
        try await credentials.setSecrets(["username": "me@icloud.test", "password": "app-pass-1234"], for: "c1")
        let account = account(autoSchedule: autoSchedule, userAddresses: userAddresses)
        let connection = Connection(kindID: "icloud", connectionID: "c1", displayName: "me@icloud.test", config: account.config)
        let store = credentials
        let client = WebDAVClient(transport: transport ?? server, hostBase: "icloud.com", credentials: {
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
