import CalendarCore
import Foundation

/// Edits a recurring resource in place. Every date it writes (`RECURRENCE-ID`, `EXDATE`, `RDATE`, `UNTIL`) uses the
/// master's own `DTSTART` form (the same `TZID`, `Z` or `VALUE=DATE`), so the server and other clients match them.
public enum SeriesEditor {
    struct Form {
        let zone: TimeZone
        let isAllDay: Bool
        let tzid: String?     // nil: UTC with Z (or floating when `floating`)
        let floating: Bool

        func property(_ name: String, _ date: Date) -> ICalProperty {
            if isAllDay {
                return ICalProperty(name: name, parameters: [ICalParameter("VALUE", "DATE")], value: ICalValues.dateText(AllDay.date(of: date, in: zone)))
            }
            if let tzid { return ICalProperty(name: name, parameters: [ICalParameter("TZID", tzid)], value: ICalValues.localText(date, in: zone)) }
            if floating { return ICalProperty(name: name, value: ICalValues.localText(date, in: zone)) }
            return ICalProperty(name: name, value: ICalValues.utcText(date))
        }

        /// One property listing several dates (EXDATE, RDATE).
        func listProperty(_ name: String, _ dates: [Date]) -> ICalProperty {
            let one = property(name, dates[0])
            let values = dates.map { property(name, $0).value }
            return ICalProperty(name: name, parameters: one.parameters, value: values.joined(separator: ","))
        }
    }

    static func form(of master: ICalComponent, resolver: TimeZoneResolver, calendarZone: TimeZone) -> Form? {
        guard let start = master.property("DTSTART"), let value = ICalValues.dateValue(start) else { return nil }
        switch value {
        case .date: return Form(zone: calendarZone, isAllDay: true, tzid: nil, floating: false)
        case .utc: return Form(zone: TimeZone(identifier: "UTC")!, isAllDay: false, tzid: nil, floating: false)
        case .local(_, let tzid?): return Form(zone: resolver.zone(for: tzid) ?? calendarZone, isAllDay: false, tzid: tzid, floating: false)
        case .local(_, nil): return Form(zone: calendarZone, isAllDay: false, tzid: nil, floating: true)
        }
    }

    // MARK: Overrides and exclusions

    public static func override(in resource: EventResource, at slot: Date, calendarZone: TimeZone) -> ICalComponent? {
        let resolver = resource.resolver
        if let existing = resource.overrides.first(where: { EventReader.recurrenceID(of: $0, resolver: resolver, calendarZone: calendarZone) == slot }) {
            return existing
        }
        guard let master = resource.master, let timing = EventReader.timing(of: master, resolver: resolver, calendarZone: calendarZone),
              let form = form(of: master, resolver: resolver, calendarZone: calendarZone),
              isSlot(slot, in: resource, calendarZone: calendarZone) else { return nil }
        var copy = master
        for name in ["RRULE", "RDATE", "EXDATE", "EXRULE"] { copy.removeProperties(named: name) }
        let end: Date
        if timing.isAllDay {
            end = AllDay.startOfDay(AllDay.date(of: slot, in: form.zone).adding(days: timing.allDayLength), in: form.zone) ?? slot
        } else {
            end = slot.addingTimeInterval(timing.end.timeIntervalSince(timing.start))
        }
        copy.set(form.property("DTSTART", slot))
        copy.set(form.property("DTEND", end))
        copy.removeProperties(named: "DURATION")
        copy.set(form.property("RECURRENCE-ID", slot))
        return copy
    }

    public static func setOverride(_ vevent: ICalComponent, at slot: Date, in resource: inout EventResource, calendarZone: TimeZone) {
        let resolver = resource.resolver
        var events = resource.events.filter { $0.property("RECURRENCE-ID") == nil || EventReader.recurrenceID(of: $0, resolver: resolver, calendarZone: calendarZone) != slot }
        events.append(vevent)
        resource.setEvents(events)
    }

    public static func exclude(_ slot: Date, in resource: inout EventResource, calendarZone: TimeZone) throws {
        let resolver = resource.resolver
        guard isSlot(slot, in: resource, calendarZone: calendarZone), var master = resource.master,
              let form = form(of: master, resolver: resolver, calendarZone: calendarZone) else { throw WriteError.notFound }
        var excluded = dates(master.properties(named: "EXDATE"), resolver: resolver, form: form)
        excluded.append(slot)
        master.set(form.listProperty("EXDATE", excluded.sorted()))
        let overrides = resource.overrides.filter { EventReader.recurrenceID(of: $0, resolver: resolver, calendarZone: calendarZone) != slot }
        resource.setEvents([master] + overrides)
    }

    // MARK: Series-wide changes

    public static func shift(_ resource: inout EventResource, by delta: TimeInterval, calendarZone: TimeZone) {
        guard delta != 0, var master = resource.master else { return }
        let resolver = resource.resolver
        guard let form = form(of: master, resolver: resolver, calendarZone: calendarZone) else { return }
        for name in ["EXDATE", "RDATE"] {
            let moved = dates(master.properties(named: name), resolver: resolver, form: form).map { self.moved($0, by: delta, form: form) }
            master.removeProperties(named: name)
            if !moved.isEmpty { master.append(form.listProperty(name, moved.sorted())) }
        }
        let overrides = resource.overrides.map { vevent -> ICalComponent in
            guard let slot = EventReader.recurrenceID(of: vevent, resolver: resolver, calendarZone: calendarZone) else { return vevent }
            var copy = vevent
            let timing = EventReader.timing(of: vevent, resolver: resolver, calendarZone: calendarZone)
            copy.set(form.property("RECURRENCE-ID", moved(slot, by: delta, form: form)))
            // An override still at its slot's time moves with the series; one the user moved keeps its own time.
            if let timing, timing.start == slot {
                copy.set(form.property("DTSTART", moved(timing.start, by: delta, form: form)))
                copy.set(form.property("DTEND", moved(timing.end, by: delta, form: form)))
            }
            return copy
        }
        resource.setEvents([master] + overrides)
    }

    public static func pruneUnmatched(_ resource: inout EventResource, calendarZone: TimeZone) {
        guard var master = resource.master else { return }
        let resolver = resource.resolver
        guard let form = form(of: master, resolver: resolver, calendarZone: calendarZone) else { return }
        // Judge against the rule without exclusions, so an EXDATE still matching an occurrence stays.
        var bare = resource
        var bareMaster = master
        bareMaster.removeProperties(named: "EXDATE")
        bare.setEvents([bareMaster])
        let kept = dates(master.properties(named: "EXDATE"), resolver: resolver, form: form).filter { isSlot($0, in: bare, calendarZone: calendarZone) }
        master.removeProperties(named: "EXDATE")
        if !kept.isEmpty { master.append(form.listProperty("EXDATE", kept.sorted())) }
        let overrides = resource.overrides.filter { vevent in
            EventReader.recurrenceID(of: vevent, resolver: resolver, calendarZone: calendarZone).map { isSlot($0, in: bare, calendarZone: calendarZone) } ?? false
        }
        resource.setEvents([master] + overrides)
    }

    // MARK: Split

    public static func split(
        _ resource: EventResource, at slot: Date, newUID: String, calendarZone: TimeZone, now: Date
    ) throws -> (head: EventResource, tail: EventResource) {
        let resolver = resource.resolver
        guard let master = resource.master, let timing = EventReader.timing(of: master, resolver: resolver, calendarZone: calendarZone),
              let form = form(of: master, resolver: resolver, calendarZone: calendarZone),
              let set = EventReader.recurrenceSet(of: master, zone: form.zone, isAllDay: timing.isAllDay, resolver: resolver)
        else { throw WriteError.notFound }
        guard set.rules.count <= 1, set.unparsed.isEmpty else { throw WriteError.unsupported(fields: [.recurrence]) }
        guard slot > timing.start, isSlotOrOverride(slot, in: resource, calendarZone: calendarZone) else { throw WriteError.notFound }

        var headMaster = master
        var tailMaster = master
        if let rule = set.rules.first {
            var headRule = rule
            var tailRule = rule
            switch rule.end {
            case .count(let total):
                let before = set.ruleInstanceCount(anchor: timing.start, timeZone: form.zone, isAllDay: timing.isAllDay, before: slot)
                guard total - before >= 1 else { throw WriteError.notFound }
                headRule.end = .count(before)
                tailRule.end = .count(total - before)
            case .until, .never:
                if timing.isAllDay {
                    // The previous calendar day, by date arithmetic: a 23-hour (spring-forward) day must not skip a date.
                    let previous = AllDay.date(of: slot, in: form.zone).adding(days: -1)
                    headRule.end = .until(AllDay.startOfDay(previous, in: form.zone) ?? slot.addingTimeInterval(-86_400))
                } else {
                    headRule.end = .until(slot.addingTimeInterval(-1))
                }
            }
            headMaster.set(ICalProperty(name: "RRULE", value: headRule.rruleString(allDay: timing.isAllDay, in: form.zone)))
            tailMaster.set(ICalProperty(name: "RRULE", value: tailRule.rruleString(allDay: timing.isAllDay, in: form.zone)))
        }
        for name in ["EXDATE", "RDATE"] {
            let all = dates(master.properties(named: name), resolver: resolver, form: form)
            headMaster.removeProperties(named: name)
            tailMaster.removeProperties(named: name)
            let early = all.filter { $0 < slot }, late = all.filter { $0 >= slot }
            if !early.isEmpty { headMaster.append(form.listProperty(name, early.sorted())) }
            if !late.isEmpty { tailMaster.append(form.listProperty(name, late.sorted())) }
        }
        tailMaster.set(ICalProperty(name: "UID", text: newUID))
        let end: Date = timing.isAllDay
            ? (AllDay.startOfDay(AllDay.date(of: slot, in: form.zone).adding(days: timing.allDayLength), in: form.zone) ?? slot)
            : slot.addingTimeInterval(timing.end.timeIntervalSince(timing.start))
        tailMaster.set(form.property("DTSTART", slot))
        tailMaster.set(form.property("DTEND", end))
        tailMaster.removeProperties(named: "DURATION")
        tailMaster.set(ICalProperty(name: "SEQUENCE", value: "0"))
        tailMaster.set(ICalProperty(name: "CREATED", value: ICalValues.utcText(now)))
        EventWriter.touch(&tailMaster, now: now, bumpSequence: false)
        EventWriter.touch(&headMaster, now: now, bumpSequence: true)

        var headOverrides: [ICalComponent] = [], tailOverrides: [ICalComponent] = []
        for vevent in resource.overrides {
            guard let id = EventReader.recurrenceID(of: vevent, resolver: resolver, calendarZone: calendarZone) else { continue }
            if id < slot { headOverrides.append(vevent) } else {
                var moved = vevent
                moved.set(ICalProperty(name: "UID", text: newUID))
                tailOverrides.append(moved)
            }
        }
        var head = resource
        head.setEvents([headMaster] + headOverrides)
        var tail = resource
        tail.setEvents([tailMaster] + tailOverrides)
        return (head, tail)
    }

    // MARK: Helpers

    /// `date` moved by `delta`; for an all-day form by whole days, so a daylight-saving change between the two dates
    /// cannot land the result on the evening before.
    static func moved(_ date: Date, by delta: TimeInterval, form: Form) -> Date {
        guard form.isAllDay else { return date.addingTimeInterval(delta) }
        let days = Int((delta / 86_400).rounded())
        return AllDay.startOfDay(AllDay.date(of: date, in: form.zone).adding(days: days), in: form.zone) ?? date.addingTimeInterval(delta)
    }

    static func dates(_ properties: [ICalProperty], resolver: TimeZoneResolver, form: Form) -> [Date] {
        properties.flatMap { ICalValues.dateValues($0).compactMap { resolver.date($0, floating: form.zone) } }
    }

    /// Whether `slot` is an occurrence of the master's set (EXDATEs applied).
    static func isSlot(_ slot: Date, in resource: EventResource, calendarZone: TimeZone) -> Bool {
        let resolver = resource.resolver
        guard let master = resource.master, let timing = EventReader.timing(of: master, resolver: resolver, calendarZone: calendarZone),
              let set = EventReader.recurrenceSet(of: master, zone: timing.zone, isAllDay: timing.isAllDay, resolver: resolver) else { return false }
        return set.occurrences(anchor: timing.start, duration: 0, timeZone: timing.zone, isAllDay: timing.isAllDay,
                               overlapping: DateInterval(start: slot, duration: 1), limit: 2).starts.contains(slot)
    }

    /// A slot of the set, or the slot of an existing override (a moved occurrence is still a split point).
    static func isSlotOrOverride(_ slot: Date, in resource: EventResource, calendarZone: TimeZone) -> Bool {
        let resolver = resource.resolver
        return isSlot(slot, in: resource, calendarZone: calendarZone)
            || resource.overrides.contains { EventReader.recurrenceID(of: $0, resolver: resolver, calendarZone: calendarZone) == slot }
    }
}
