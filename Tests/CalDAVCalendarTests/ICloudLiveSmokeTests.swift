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

/// Line 1 the Apple ID, line 2 an app-specific password, from a file outside the repository. Never printed.
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
///   marked `SCHEDULE-AGENT=CLIENT` (it should not). Use only an address you read: if iCloud ignores the parameter, the PUT may send a real invitation
///   and the cleanup DELETE a cancellation to it.
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
        let change = try await source.checkForChanges()
        print("LIVE checkForChanges after writes: \(change.map { c -> String in if case .eventsChanged(let ids) = c { return "eventsChanged(\(ids?.count ?? 0))" }; return "calendarsChanged" } ?? "nil")")
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
