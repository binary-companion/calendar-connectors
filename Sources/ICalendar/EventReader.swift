import CalendarCore
import Foundation

public struct EventReadContext: Sendable {
    public var calendarID: String
    /// The resource's last path segment; the series id and the base of every event id.
    public var resourceName: String
    public var etag: String?
    public var sourceID: String?
    /// Floating times and all-day dates are read in this zone.
    public var calendarZone: TimeZone
    /// The account's own calendar-user addresses, lowercased (`mailto:...`, `urn:uuid:...`, principal URLs).
    public var selfAddresses: Set<String>

    public init(calendarID: String, resourceName: String, etag: String?, sourceID: String?, calendarZone: TimeZone, selfAddresses: Set<String>) {
        self.calendarID = calendarID
        self.resourceName = resourceName
        self.etag = etag
        self.sourceID = sourceID
        self.calendarZone = calendarZone
        self.selfAddresses = selfAddresses
    }
}

/// When a `VEVENT` happens. All-day events keep their length in days so an occurrence across a daylight-saving change
/// still ends at midnight.
public struct EventTimingInfo: Sendable, Equatable {
    public var start: Date
    public var end: Date
    public var zone: TimeZone
    public var isAllDay: Bool
    /// Days covered by an all-day event (end exclusive); 0 for timed events.
    public var allDayLength: Int
}

public enum EventReader {
    public static let instanceLimit = 5000

    public static func eventID(resourceName: String, originalStart: Date?, isAllDay: Bool, zone: TimeZone) -> String {
        guard let originalStart else { return resourceName }
        let suffix = isAllDay ? ICalValues.dateText(AllDay.date(of: originalStart, in: zone)) : ICalValues.utcText(originalStart)
        return resourceName + "#" + suffix
    }

    /// Every event of the resource overlapping `window`: expanded occurrences of the master, overrides in place of their
    /// slots (an override moved into the window counts, its slot does not), or the single event.
    public static func events(in resource: EventResource, overlapping window: DateInterval, context: EventReadContext) -> [CalendarEvent] {
        let resolver = resource.resolver
        var result: [CalendarEvent] = []
        var overridden = Set<Date>()
        for vevent in resource.overrides {
            guard let slot = recurrenceID(of: vevent, resolver: resolver, calendarZone: context.calendarZone),
                  let timing = timing(of: vevent, resolver: resolver, calendarZone: context.calendarZone) else { continue }
            overridden.insert(slot)
            if overlaps(timing.start, timing.end, window) {
                result.append(map(vevent, timing: timing, originalStart: slot, recurring: true, context: context, resolver: resolver))
            }
        }
        if let master = resource.master, let timing = timing(of: master, resolver: resolver, calendarZone: context.calendarZone) {
            if let set = recurrenceSet(of: master, zone: timing.zone, isAllDay: timing.isAllDay, resolver: resolver) {
                let expansion = set.occurrences(anchor: timing.start, duration: timing.end.timeIntervalSince(timing.start),
                                                timeZone: timing.zone, isAllDay: timing.isAllDay, overlapping: window, limit: instanceLimit)
                for start in expansion.starts where !overridden.contains(start) {
                    result.append(map(master, timing: moved(timing, to: start), originalStart: start, recurring: true, context: context, resolver: resolver))
                }
            } else if overlaps(timing.start, timing.end, window) {
                result.append(map(master, timing: timing, originalStart: nil, recurring: false, context: context, resolver: resolver))
            }
        }
        return result.sorted { ($0.start, $0.eventID) < ($1.start, $1.eventID) }
    }

    /// The series master in its series form (`eventID` and `seriesID` = the resource name, `originalStart` = its start),
    /// or the single event for a resource that does not recur. nil when the resource has no master.
    public static func masterEvent(of resource: EventResource, context: EventReadContext) -> CalendarEvent? {
        let resolver = resource.resolver
        guard let master = resource.master, let timing = timing(of: master, resolver: resolver, calendarZone: context.calendarZone) else { return nil }
        let recurring = recurrenceSet(of: master, zone: timing.zone, isAllDay: timing.isAllDay, resolver: resolver) != nil
        var event = map(master, timing: timing, originalStart: recurring ? timing.start : nil, recurring: recurring, context: context, resolver: resolver)
        event.eventID = context.resourceName
        return event
    }

    /// The occurrence whose slot is `originalStart`: its override, or the master's instance there. nil when that slot
    /// is excluded or is not an occurrence.
    public static func occurrence(in resource: EventResource, originalStart: Date, context: EventReadContext) -> CalendarEvent? {
        let resolver = resource.resolver
        for vevent in resource.overrides where recurrenceID(of: vevent, resolver: resolver, calendarZone: context.calendarZone) == originalStart {
            guard let timing = timing(of: vevent, resolver: resolver, calendarZone: context.calendarZone) else { return nil }
            return map(vevent, timing: timing, originalStart: originalStart, recurring: true, context: context, resolver: resolver)
        }
        guard let master = resource.master, let timing = timing(of: master, resolver: resolver, calendarZone: context.calendarZone),
              let set = recurrenceSet(of: master, zone: timing.zone, isAllDay: timing.isAllDay, resolver: resolver) else { return nil }
        let probe = DateInterval(start: originalStart, duration: 1)
        let found = set.occurrences(anchor: timing.start, duration: 0, timeZone: timing.zone, isAllDay: timing.isAllDay, overlapping: probe, limit: 2)
        guard found.starts.contains(originalStart) else { return nil }
        return map(master, timing: moved(timing, to: originalStart), originalStart: originalStart, recurring: true, context: context, resolver: resolver)
    }

    public static func series(of resource: EventResource, context: EventReadContext) -> CalendarSeries? {
        let resolver = resource.resolver
        guard let master = resource.master, let timing = timing(of: master, resolver: resolver, calendarZone: context.calendarZone),
              let set = recurrenceSet(of: master, zone: timing.zone, isAllDay: timing.isAllDay, resolver: resolver) else { return nil }
        return CalendarSeries(seriesID: context.resourceName, calendarID: context.calendarID, start: timing.start,
                              timeZone: timing.zone, isAllDay: timing.isAllDay, recurrence: set)
    }

    // MARK: Parts

    public static func timing(of vevent: ICalComponent, resolver: TimeZoneResolver, calendarZone: TimeZone) -> EventTimingInfo? {
        guard let startProperty = vevent.property("DTSTART"), let startValue = ICalValues.dateValue(startProperty) else { return nil }
        if case .date(let first) = startValue {
            var length = 1
            if let endProperty = vevent.property("DTEND"), case .date(let last)? = ICalValues.dateValue(endProperty) {
                length = max(1, first.days(to: last))
            } else if let duration = vevent.property("DURATION").flatMap({ ICalValues.duration($0.value) }), duration >= 86_400 {
                length = Int(duration / 86_400)
            }
            guard let range = AllDay.canonical(first: first, endExclusive: first.adding(days: length), in: calendarZone) else { return nil }
            return EventTimingInfo(start: range.start, end: range.end, zone: calendarZone, isAllDay: true, allDayLength: length)
        }
        guard let start = resolver.date(startValue, floating: calendarZone) else { return nil }
        var end = start
        if let endProperty = vevent.property("DTEND"), let value = ICalValues.dateValue(endProperty),
           let date = resolver.date(value, floating: calendarZone), date >= start {
            end = date
        } else if let duration = vevent.property("DURATION").flatMap({ ICalValues.duration($0.value) }), duration >= 0 {
            end = start.addingTimeInterval(duration)
        }
        return EventTimingInfo(start: start, end: end, zone: resolver.displayZone(startValue, fallback: calendarZone), isAllDay: false, allDayLength: 0)
    }

    public static func recurrenceID(of vevent: ICalComponent, resolver: TimeZoneResolver, calendarZone: TimeZone) -> Date? {
        guard let property = vevent.property("RECURRENCE-ID"), let value = ICalValues.dateValue(property) else { return nil }
        return resolver.date(value, floating: calendarZone)
    }

    /// The master's `RRULE`, `RDATE` and `EXDATE`, with dates resolved by this file's zones; nil when it does not recur.
    /// An `RRULE` that cannot be read is kept in `unparsed` (so the set reports `hasUnreadableRule`).
    public static func recurrenceSet(of master: ICalComponent, zone: TimeZone, isAllDay: Bool, resolver: TimeZoneResolver) -> RecurrenceSet? {
        let ruleProperties = master.properties(named: "RRULE")
        let extra = master.properties(named: "RDATE")
        guard !ruleProperties.isEmpty || !extra.isEmpty else { return nil }
        var rules: [RecurrenceRule] = []
        var unparsed: [String] = []
        for property in ruleProperties {
            if let rule = try? RecurrenceRule(rrule: property.value, in: zone) { rules.append(rule) } else { unparsed.append(ICalSerializer.contentLine(property)) }
        }
        func dates(_ properties: [ICalProperty]) -> [Date] {
            properties.flatMap { ICalValues.dateValues($0).compactMap { resolver.date($0, floating: zone) } }
        }
        return RecurrenceSet(rules: rules, extraDates: dates(extra), excludedDates: dates(master.properties(named: "EXDATE")), unparsed: unparsed)
    }

    private static func moved(_ timing: EventTimingInfo, to start: Date) -> EventTimingInfo {
        var copy = timing
        copy.start = start
        if timing.isAllDay {
            let first = AllDay.date(of: start, in: timing.zone)
            copy.end = AllDay.startOfDay(first.adding(days: timing.allDayLength), in: timing.zone) ?? start.addingTimeInterval(Double(timing.allDayLength) * 86_400)
        } else {
            copy.end = start.addingTimeInterval(timing.end.timeIntervalSince(timing.start))
        }
        return copy
    }

    private static func overlaps(_ start: Date, _ end: Date, _ window: DateInterval) -> Bool {
        end > start ? (start < window.end && end > window.start) : (start >= window.start && start < window.end)
    }

    private static func map(
        _ vevent: ICalComponent, timing: EventTimingInfo, originalStart: Date?, recurring: Bool, context: EventReadContext,
        resolver: TimeZoneResolver
    ) -> CalendarEvent {
        let (attendees, organizer) = AttendeeMapper.read(vevent, selfAddresses: context.selfAddresses)
        let notes = vevent.property("DESCRIPTION")?.text
        let location = vevent.property("LOCATION")?.text
        let url = vevent.property("URL").flatMap { URL(string: $0.value) }
        let status: EventStatus
        switch vevent.property("STATUS")?.value.uppercased() {
        case "CANCELLED": status = .cancelled
        case "TENTATIVE": status = .tentative
        default: status = .confirmed
        }
        let visibility: Visibility
        switch vevent.property("CLASS")?.value.uppercased() {
        case "PUBLIC": visibility = .publicEvent
        case "PRIVATE": visibility = .privateEvent
        case "CONFIDENTIAL": visibility = .confidential
        default: visibility = .default
        }
        func utc(_ name: String) -> Date? {
            guard let property = vevent.property(name), case .utc(let date)? = ICalValues.dateValue(property) else { return nil }
            return date
        }
        return CalendarEvent(
            eventID: eventID(resourceName: context.resourceName, originalStart: recurring ? originalStart : nil, isAllDay: timing.isAllDay, zone: timing.zone),
            uid: vevent.property("UID")?.text, uidScope: .global, calendarID: context.calendarID,
            title: vevent.property("SUMMARY")?.text ?? "", notes: notes?.isEmpty == true ? nil : notes,
            location: location?.isEmpty == true ? nil : location, start: timing.start, end: timing.end, timeZone: timing.zone,
            isAllDay: timing.isAllDay, status: status,
            availability: vevent.property("TRANSP")?.value.uppercased() == "TRANSPARENT" ? .free : .busy,
            visibility: visibility,
            series: recurring ? .occurrence(seriesID: context.resourceName, originalStart: originalStart) : .notRecurring,
            attendees: attendees, organizer: organizer,
            conferences: ConferenceDetector.conferences(location: location, url: url, notes: notes),
            reminders: AlarmMapper.reminders(from: vevent.components(named: "VALARM")),
            url: url, version: context.etag, lastModified: utc("LAST-MODIFIED"), created: utc("CREATED"),
            participation: AttendeeMapper.participation(attendees: attendees, organizer: organizer, knowsSelf: !context.selfAddresses.isEmpty),
            sourceID: context.sourceID)
    }
}
