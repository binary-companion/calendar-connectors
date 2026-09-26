import CalendarCore
import Foundation
import Testing
@testable import MicrosoftCalendar

@Test func microsoftIsAService() {
    #expect(CalendarService.microsoft.rawValue == "microsoft")
}

@Test func windowsNamesResolveToIANAZones() {
    #expect(WindowsTimeZones.timeZone(for: "Pacific Standard Time")?.identifier == "America/Los_Angeles")
    #expect(WindowsTimeZones.timeZone(for: "UTC")?.secondsFromGMT() == 0)
    #expect(WindowsTimeZones.timeZone(for: "W. Europe Standard Time")?.identifier == "Europe/Berlin")
}

@Test func ianaNamesPassThroughAndUnknownNamesAreNil() {
    #expect(WindowsTimeZones.timeZone(for: "Europe/Paris")?.identifier == "Europe/Paris")
    #expect(WindowsTimeZones.timeZone(for: "Customized Time Zone") == nil)
    #expect(WindowsTimeZones.timeZone(for: "") == nil)
}

@Test func everyTableEntryResolvesOnThisPlatform() {
    // A failure names the entry; fix it by using a spelling this platform knows (see the aliases list).
    let missing = WindowsTimeZones.table.filter { TimeZone(identifier: $0.value) == nil }.map(\.key).sorted()
    #expect(missing.isEmpty, "unresolvable: \(missing)")
}

@Test func windowsNameForAZone() {
    #expect(WindowsTimeZones.windowsName(for: TimeZone(identifier: "America/New_York")!) == "Eastern Standard Time")
    #expect(WindowsTimeZones.windowsName(for: TimeZone(identifier: "UTC")!) == "UTC")
    #expect(WindowsTimeZones.windowsName(for: TimeZone(identifier: "Asia/Kolkata")!) == "India Standard Time")
    #expect(WindowsTimeZones.windowsName(for: TimeZone(identifier: "Pacific/Fakaofo")!) == nil)
}

@Test func windowsNamesAreOneToOneWithIANAIds() {
    #expect(Set(WindowsTimeZones.table.values).count == WindowsTimeZones.table.count)
}

@Test func parsesGraphLocalDateTimesInAZone() {
    let la = TimeZone(identifier: "America/Los_Angeles")!
    let date = GraphTime.parse("2026-09-25T10:00:00.0000000", in: la)
    #expect(date == Date(timeIntervalSince1970: 1_790_355_600))   // 17:00 UTC
    #expect(GraphTime.parse("2026-09-25T10:00:00", in: la) == date)
    #expect(GraphTime.parse("2026-13-25T10:00:00", in: la) == nil)
    #expect(GraphTime.parse("2026-02-30T10:00:00", in: la) == nil)
    #expect(GraphTime.parse("nonsense", in: la) == nil)
}

@Test func formatsAndReadsDatePartsAndInstants() {
    let la = TimeZone(identifier: "America/Los_Angeles")!
    #expect(GraphTime.format(Date(timeIntervalSince1970: 1_790_355_600), in: la) == "2026-09-25T10:00:00")
    #expect(GraphTime.date("2026-09-25T00:00:00.0000000") == CalendarDate(year: 2026, month: 9, day: 25))
    #expect(GraphTime.midnight(CalendarDate(year: 2026, month: 9, day: 5)) == "2026-09-05T00:00:00")
    #expect(GraphTime.dateText(CalendarDate(year: 2026, month: 9, day: 5)) == "2026-09-05")
    #expect(GraphTime.instantText(Date(timeIntervalSince1970: 1_790_355_600)) == "2026-09-25T17:00:00Z")
}

@Test func parsesInstantsWithZOffsetAndFractions() {
    let expected = Date(timeIntervalSince1970: 1_790_355_600)
    #expect(GraphTime.parseInstant("2026-09-25T17:00:00Z") == expected)
    #expect(GraphTime.parseInstant("2026-09-25T17:00:00.1234567Z") == expected)
    #expect(GraphTime.parseInstant("2026-09-25T10:00:00-07:00") == expected)
    #expect(GraphTime.parseInstant("2026-09-25T22:30:00+05:30") == expected)
    #expect(GraphTime.parseInstant("2026-09-25T17:00:00.0000000") == expected)   // no offset: UTC
    #expect(GraphTime.parseInstant("garbage") == nil)
}
