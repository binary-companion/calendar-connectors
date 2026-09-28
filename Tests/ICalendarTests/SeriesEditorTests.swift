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

private func allDayDailySeries(rrule: String) -> String {
    """
    BEGIN:VCALENDAR
    VERSION:2.0
    PRODID:-//Test//EN
    BEGIN:VEVENT
    UID:ALLDAY-UID
    DTSTAMP:20260301T000000Z
    SUMMARY:Daily all-day
    DTSTART;VALUE=DATE:20260305
    DTEND;VALUE=DATE:20260306
    \(rrule)
    END:VEVENT
    END:VCALENDAR
    """
}

@Test func allDaySplitAfterASpringForwardDayKeepsTheDayBeforeTheSlot() throws {
    let ny = TimeZone(identifier: "America/New_York")!
    var cal = Calendar(identifier: .gregorian)
    cal.timeZone = ny
    let mar9 = cal.date(from: DateComponents(year: 2026, month: 3, day: 9))!
    let mar8 = cal.date(from: DateComponents(year: 2026, month: 3, day: 8))!
    let r = try resource(allDayDailySeries(rrule: "RRULE:FREQ=DAILY"))
    let (head, tail) = try SeriesEditor.split(r, at: mar9, newUID: "new-uid", calendarZone: ny, now: now)
    // Mar 8 is 23 hours long in New York; the previous day is Mar 8, not Mar 7.
    #expect(head.master?.property("RRULE")?.value == "FREQ=DAILY;UNTIL=20260308")
    let ctx = EventReadContext(calendarID: "home", resourceName: "a.ics", etag: "\"e\"", sourceID: "s", calendarZone: ny, selfAddresses: [])
    let headStarts = EventReader.events(in: head, overlapping: DateInterval(start: mar8.addingTimeInterval(-86_400), end: mar9.addingTimeInterval(86_400 * 3)), context: ctx).map(\.start)
    #expect(headStarts.contains(mar8))
    #expect(!headStarts.contains(mar9))
    #expect(tail.master?.property("DTSTART")?.value == "20260309")
}

@Test func timedSplitAcrossTheNovemberChangeKeepsTheHeadStrictlyBeforeTheSlot() throws {
    let text = weeklySeries
        .replacingOccurrences(of: "RRULE:FREQ=WEEKLY;BYDAY=TU", with: "RRULE:FREQ=WEEKLY;BYDAY=SU")
        .replacingOccurrences(of: "EXDATE;TZID=America/Los_Angeles:20260908T100000\n", with: "")
        .replacingOccurrences(of: "20260901T100000", with: "20261018T013000")
        .replacingOccurrences(of: "20260901T103000", with: "20261018T020000")
    let r = try resource(text)
    let oct25 = laTime(2026, 10, 25, 1, 30), nov1 = laTime(2026, 11, 1, 1, 30), nov8 = laTime(2026, 11, 8, 1, 30)
    let wide = DateInterval(start: laTime(2026, 10, 1, 0), end: laTime(2026, 12, 1, 0))
    for slot in [nov1, nov8] {
        let (head, tail) = try SeriesEditor.split(r, at: slot, newUID: "new-uid", calendarZone: la, now: now)
        let h = EventReader.events(in: head, overlapping: wide, context: context()).map(\.start)
        let t = EventReader.events(in: tail, overlapping: wide, context: context()).map(\.start)
        #expect(h.contains(oct25) && h.allSatisfy { $0 < slot }, "head for \(slot)")
        #expect(t.first == slot, "tail for \(slot)")
    }
}
