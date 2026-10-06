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
        transport: ICalSubscriptionKind.makeDefaultTransport(), monitor: ChangeMonitor(), maxAge: 900,
        now: { Date() }, defaultZone: .current)
    let events = try await source.events(in: DateInterval(start: Date(), duration: 90 * 86_400))
    print("LIVE calendars:", try await source.calendars().count, "events in 90 days:", events.count,
          "recurring:", events.filter { $0.series != .notRecurring }.count, "all-day:", events.filter(\.isAllDay).count,
          "with a link:", events.filter { !$0.conferences.isEmpty }.count)
}
