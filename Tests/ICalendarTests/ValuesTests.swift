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
