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
