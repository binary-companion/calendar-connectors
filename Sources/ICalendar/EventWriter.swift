import CalendarCore
import Foundation

public enum EventWriter {
    public static let productID = "-//TimeTug//CalendarConnectors//EN"
    private static let utc = TimeZone(identifier: "UTC")!

    /// A new `VEVENT`. With attendees, `organizerAddress` (the account's `mailto:` address) becomes the organizer and
    /// its own accepted attendee entry, as iCloud writes it. Fields are validated by the caller (`EventDraft.validate`
    /// and the source's capabilities).
    public static func vevent(from draft: EventDraft, uid: String, now: Date, organizerAddress: String?) throws -> ICalComponent {
        var vevent = ICalComponent(name: "VEVENT", properties: [
            ICalProperty(name: "UID", text: uid),
            ICalProperty(name: "DTSTAMP", value: ICalValues.utcText(now)),
            ICalProperty(name: "CREATED", value: ICalValues.utcText(now)),
            ICalProperty(name: "LAST-MODIFIED", value: ICalValues.utcText(now)),
            ICalProperty(name: "SEQUENCE", value: "0"),
            ICalProperty(name: "SUMMARY", text: draft.title),
        ])
        vevent.setText("DESCRIPTION", draft.notes)
        vevent.setText("LOCATION", draft.location)
        setTiming(draft.timing, on: &vevent)
        vevent.set(ICalProperty(name: "TRANSP", value: draft.availability.closest(in: [.busy, .free]) == .free ? "TRANSPARENT" : "OPAQUE"))
        if let value = classValue(draft.visibility) { vevent.set(ICalProperty(name: "CLASS", value: value)) }
        if let rule = draft.recurrence {
            let zone = draft.timing.timeZone ?? utc
            vevent.set(ICalProperty(name: "RRULE", value: rule.rruleString(allDay: draft.timing.isAllDay, in: zone)))
        }
        if !draft.attendees.isEmpty {
            if let organizerAddress {
                vevent.set(AttendeeMapper.organizerProperty(address: organizerAddress))
                vevent.append(ICalProperty(name: "ATTENDEE", parameters: [ICalParameter("PARTSTAT", "ACCEPTED"), ICalParameter("ROLE", "CHAIR")],
                                           value: organizerAddress))
            }
            for attendee in draft.attendees { vevent.append(AttendeeMapper.property(for: attendee)) }
        }
        vevent.components = try AlarmMapper.alarms(from: draft.reminders ?? [])
        return vevent
    }

    /// A resource holding `vevent` (and any overrides the caller adds later) with a `VTIMEZONE` for each zone.
    public static func resource(for vevent: ICalComponent, zones: [TimeZone], from start: Date, through end: Date) throws -> EventResource {
        var resource = try EventResource(calendar: ICalComponent(name: "VCALENDAR", properties: [
            ICalProperty(name: "VERSION", value: "2.0"),
            ICalProperty(name: "PRODID", text: productID),
            ICalProperty(name: "CALSCALE", value: "GREGORIAN"),
        ], components: [vevent]))
        resource.ensureTimeZones(zones, from: start, through: end)
        return resource
    }

    /// A date property in the form the connector writes: `VALUE=DATE` for all-day, `Z` for UTC, else `TZID`.
    public static func dateProperty(_ name: String, _ date: Date, zone: TimeZone, isAllDay: Bool) -> ICalProperty {
        if isAllDay {
            return ICalProperty(name: name, parameters: [ICalParameter("VALUE", "DATE")], value: ICalValues.dateText(AllDay.date(of: date, in: zone)))
        }
        if zone.identifier == "UTC" || zone.identifier == "GMT" { return ICalProperty(name: name, value: ICalValues.utcText(date)) }
        return ICalProperty(name: name, parameters: [ICalParameter("TZID", zone.identifier)], value: ICalValues.localText(date, in: zone))
    }

    public static func setTiming(_ timing: EventTiming, on vevent: inout ICalComponent) {
        let zone = timing.timeZone ?? utc
        vevent.set(dateProperty("DTSTART", timing.start, zone: zone, isAllDay: timing.isAllDay))
        vevent.set(dateProperty("DTEND", timing.end, zone: zone, isAllDay: timing.isAllDay))
        vevent.removeProperties(named: "DURATION")
    }

    /// Applies a patch whose fields the caller has already checked against the capabilities. Clearing reminders (the
    /// calendar's defaults) cannot be expressed in CalDAV and throws `.unsupported(fields: [.reminders])`. A generated
    /// or removed conference is refused by the caller before this is reached. `calendarZone` is the zone a date `UNTIL`
    /// of an all-day series is written in.
    public static func apply(_ patch: EventPatch, to vevent: inout ICalComponent, now: Date, organizerAddress: String?,
                             calendarZone: TimeZone? = nil) throws {
        if case .clear = patch.reminders { throw WriteError.unsupported(fields: [.reminders]) }
        if let title = patch.title { vevent.set(ICalProperty(name: "SUMMARY", text: title)) }
        switch patch.notes { case .keep: break; case .set(let text): vevent.setText("DESCRIPTION", text); case .clear: vevent.removeProperties(named: "DESCRIPTION") }
        switch patch.location { case .keep: break; case .set(let text): vevent.setText("LOCATION", text); case .clear: vevent.removeProperties(named: "LOCATION") }
        if let timing = patch.timing { setTiming(timing, on: &vevent) }
        if let availability = patch.availability {
            vevent.set(ICalProperty(name: "TRANSP", value: availability.closest(in: [.busy, .free]) == .free ? "TRANSPARENT" : "OPAQUE"))
        }
        if let visibility = patch.visibility {
            if let value = classValue(visibility) { vevent.set(ICalProperty(name: "CLASS", value: value)) } else { vevent.removeProperties(named: "CLASS") }
        }
        if case .set(let reminders) = patch.reminders {
            vevent.components = vevent.components.filter { $0.name != "VALARM" } + (try AlarmMapper.alarms(from: reminders))
        }
        if let changes = patch.attendees, !changes.isEmpty {
            let removed = Set(changes.remove.map { $0.lowercased() })
            vevent.properties.removeAll { $0.name == "ATTENDEE" && (AttendeeMapper.email(of: $0).map(removed.contains) ?? false) }
            if vevent.property("ORGANIZER") == nil, !changes.add.isEmpty, let organizerAddress {
                vevent.set(AttendeeMapper.organizerProperty(address: organizerAddress))
                vevent.append(ICalProperty(name: "ATTENDEE", parameters: [ICalParameter("PARTSTAT", "ACCEPTED"), ICalParameter("ROLE", "CHAIR")],
                                           value: organizerAddress))
            }
            for draft in changes.add {
                if let index = vevent.properties.firstIndex(where: { $0.name == "ATTENDEE" && AttendeeMapper.email(of: $0) == draft.email }) {
                    var existing = AttendeeMapper.property(for: draft)
                    existing.setParameter("PARTSTAT", vevent.properties[index].parameter("PARTSTAT") ?? "NEEDS-ACTION")
                    vevent.properties[index] = existing
                } else {
                    vevent.append(AttendeeMapper.property(for: draft))
                }
            }
        }
        switch patch.recurrence {
        case .keep: break
        case .set(let rule):
            let start = vevent.property("DTSTART")
            let isAllDay = start?.parameter("VALUE")?.uppercased() == "DATE"
            // An all-day series has no TZID, and its date UNTIL is read in the calendar's zone (as `create` and
            // `SeriesEditor.split` write it), so write it in that zone too.
            let zone = start?.parameter("TZID").flatMap { TimeZoneResolver().zone(for: $0) }
                ?? (isAllDay ? calendarZone ?? patch.timing?.timeZone : nil) ?? utc
            vevent.set(ICalProperty(name: "RRULE", value: rule.rruleString(allDay: isAllDay, in: zone)))
        case .clear:
            for name in ["RRULE", "RDATE", "EXDATE", "EXRULE"] { vevent.removeProperties(named: name) }
        }
        let significant = patch.timing != nil || patch.recurrence != .keep || !(patch.attendees?.isEmpty ?? true)
        touch(&vevent, now: now, bumpSequence: significant)
    }

    /// Updates `DTSTAMP` and `LAST-MODIFIED`, and raises `SEQUENCE` for a change attendees must hear about.
    public static func touch(_ vevent: inout ICalComponent, now: Date, bumpSequence: Bool) {
        vevent.set(ICalProperty(name: "DTSTAMP", value: ICalValues.utcText(now)))
        vevent.set(ICalProperty(name: "LAST-MODIFIED", value: ICalValues.utcText(now)))
        if bumpSequence {
            let current = vevent.property("SEQUENCE").flatMap { Int($0.value) } ?? 0
            vevent.set(ICalProperty(name: "SEQUENCE", value: String(current + 1)))
        }
    }

    static func classValue(_ visibility: Visibility) -> String? {
        switch visibility {
        case .default: return nil
        case .publicEvent: return "PUBLIC"
        case .privateEvent: return "PRIVATE"
        case .confidential: return "CONFIDENTIAL"
        }
    }
}
