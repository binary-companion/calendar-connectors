import CalendarTestSupport
import Foundation
import Testing
@testable import CalendarCore

private let ny = TimeZone(identifier: "America/New_York")!
private func iso(_ s: String) -> Date { ISO8601DateFormatter().date(from: s)! }

private func event(start: String, end: String, zone: TimeZone, allDay: Bool = true) -> CalendarEvent {
    CalendarEvent(eventID: "e", calendarID: "c", title: "T", start: iso(start), end: iso(end), timeZone: zone, isAllDay: allDay)
}

@Test func canonicalAllDayEventHasNoViolations() {
    let e = event(start: "2026-09-18T04:00:00Z", end: "2026-09-19T04:00:00Z", zone: ny)
    #expect(AllDayConformance.violations(e).isEmpty)
}

@Test func endAtEndOfDayIsAViolation() {
    let e = event(start: "2026-09-18T04:00:00Z", end: "2026-09-19T03:59:59Z", zone: ny)
    #expect(!AllDayConformance.violations(e).isEmpty)
}

@Test func endEqualToStartIsAViolation() {
    let e = event(start: "2026-09-18T04:00:00Z", end: "2026-09-18T04:00:00Z", zone: ny)
    #expect(!AllDayConformance.violations(e).isEmpty)
}

@Test func timedEventsAreNotChecked() {
    let e = event(start: "2026-09-18T10:15:00Z", end: "2026-09-18T10:15:00Z", zone: ny, allDay: false)
    #expect(AllDayConformance.violations(e).isEmpty)
}
