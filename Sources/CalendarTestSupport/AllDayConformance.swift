import CalendarCore
import Foundation

/// Every connector's mapper tests run their all-day fixtures through this so no connector can emit a
/// provider-native form. An empty result means the event is canonical.
public enum AllDayConformance {
    public static func violations(_ event: CalendarEvent) -> [String] {
        guard event.isAllDay else { return [] }
        guard let zone = event.timeZone else { return ["all-day event has no timeZone"] }
        var found: [String] = []
        func isMidnight(_ instant: Date) -> Bool {
            AllDay.startOfDay(AllDay.date(of: instant, in: zone), in: zone) == instant
        }
        if !isMidnight(event.start) { found.append("start is not the start of a day in \(zone.identifier)") }
        if !isMidnight(event.end) { found.append("end is not the start of a day in \(zone.identifier)") }
        if event.end <= event.start { found.append("end is not after start") }
        return found
    }
}
