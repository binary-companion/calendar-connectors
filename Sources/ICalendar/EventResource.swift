import CalendarCore
import Foundation

/// One CalDAV calendar object resource: a `VCALENDAR` whose `VEVENT`s share one `UID` (the series master, if any, and
/// its `RECURRENCE-ID` overrides), plus its `VTIMEZONE`s and anything else the file holds.
public struct EventResource: Sendable, Equatable {
    public var calendar: ICalComponent

    public init(calendar: ICalComponent) throws {
        guard calendar.name == "VCALENDAR" else { throw ICalError.malformed("not a VCALENDAR") }
        guard !calendar.components(named: "VEVENT").isEmpty else { throw ICalError.malformed("no VEVENT") }
        self.calendar = calendar
    }

    public init(data: Data) throws { try self.init(calendar: ICalParser.parse(data)) }

    public var events: [ICalComponent] { calendar.components(named: "VEVENT") }
    public var master: ICalComponent? { events.first { $0.property("RECURRENCE-ID") == nil } }
    public var overrides: [ICalComponent] { events.filter { $0.property("RECURRENCE-ID") != nil } }
    public var uid: String? { events.first?.property("UID")?.text }
    public var resolver: TimeZoneResolver { TimeZoneResolver(calendar: calendar) }

    public func serialized() -> Data { Data(ICalSerializer.serialize(calendar).utf8) }

    /// Replaces the `VEVENT`s, keeping every other component where it was.
    public mutating func setEvents(_ events: [ICalComponent]) {
        let others = calendar.components.filter { $0.name != "VEVENT" }
        calendar.components = others + events
    }

    /// Adds a `VTIMEZONE` for each zone that has none yet (UTC needs none).
    public mutating func ensureTimeZones(_ zones: [TimeZone], from start: Date, through end: Date) {
        let present = Set(calendar.components(named: "VTIMEZONE").compactMap { $0.property("TZID")?.value })
        var added: [ICalComponent] = []
        for zone in zones where !present.contains(zone.identifier) && !added.contains(where: { $0.property("TZID")?.value == zone.identifier }) {
            if let definition = VTimeZoneWriter.component(for: zone, from: start, through: end) { added.append(definition) }
        }
        calendar.components = added + calendar.components
    }
}
