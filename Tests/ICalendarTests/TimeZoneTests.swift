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
