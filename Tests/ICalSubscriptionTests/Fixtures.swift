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
    var lines: [String] = ["BEGIN:VCALENDAR", "VERSION:2.0", "PRODID:-//Example//Feed 1.0//EN", "METHOD:PUBLISH"]
    lines += header
    lines += laTimeZone
    for event in events { lines += event }
    lines.append("END:VCALENDAR")
    return lines.joined(separator: "\r\n") + "\r\n"
}

func vevent(uid: String?, title: String, start: String, end: String, extra: [String] = []) -> [String] {
    var lines: [String] = ["BEGIN:VEVENT"]
    if let uid { lines.append("UID:\(uid)") }
    lines.append("DTSTAMP:20260901T000000Z")
    lines.append("SUMMARY:\(title)")
    lines.append("DTSTART;TZID=America/Los_Angeles:\(start)")
    lines.append("DTEND;TZID=America/Los_Angeles:\(end)")
    lines += extra
    lines.append("END:VEVENT")
    return lines
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
