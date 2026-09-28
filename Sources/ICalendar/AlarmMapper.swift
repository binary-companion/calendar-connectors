import CalendarCore
import Foundation

public enum AlarmMapper {
    /// Every readable `VALARM` as a `Reminder`; an alarm with an unreadable trigger is skipped. `isCalendarDefault` is
    /// true for Apple's default alarms (`X-APPLE-DEFAULT-ALARM` or `X-APPLE-LOCAL-DEFAULT-ALARM` set to TRUE) and false
    /// otherwise.
    public static func reminders(from alarms: [ICalComponent]) -> [Reminder] {
        alarms.compactMap(reminder)
    }

    static func reminder(_ alarm: ICalComponent) -> Reminder? {
        guard let triggerProperty = alarm.property("TRIGGER") else { return nil }
        var trigger: ReminderTrigger
        if triggerProperty.parameter("VALUE")?.uppercased() == "DATE-TIME" {
            guard case .utc(let date)? = ICalValues.dateValue(triggerProperty) else { return nil }
            trigger = .absolute(date)
        } else {
            guard let offset = ICalValues.duration(triggerProperty.value) else { return nil }
            trigger = .relative(offset: offset, to: triggerProperty.parameter("RELATED")?.uppercased() == "END" ? .end : .start)
        }
        if let proximity = alarm.property("X-APPLE-PROXIMITY")?.value.uppercased(), proximity == "ARRIVE" || proximity == "DEPART",
           let place = alarm.property("X-APPLE-STRUCTURED-LOCATION") {
            trigger = .location(structuredLocation(place), proximity == "ARRIVE" ? .enter : .leave)
        }
        let type: ReminderType
        switch alarm.property("ACTION")?.value.uppercased() ?? "DISPLAY" {
        case "DISPLAY": type = .display
        case "AUDIO": type = .audio(soundName: alarm.property("ATTACH")?.value)
        case "EMAIL": type = .email(address: alarm.property("ATTENDEE").flatMap { CalendarUserAddress.email(from: $0.value) })
        case "PROCEDURE": type = .procedure(url: alarm.property("ATTACH").flatMap { URL(string: $0.value) })
        case let other: type = .other(other)
        }
        let repeatCount = alarm.property("REPEAT").flatMap { Int($0.value) } ?? 0
        let interval = alarm.property("DURATION").flatMap { ICalValues.duration($0.value) }
        let isDefault = ["X-APPLE-DEFAULT-ALARM", "X-APPLE-LOCAL-DEFAULT-ALARM"].contains {
            alarm.property($0)?.value.uppercased() == "TRUE"
        }
        return Reminder(trigger: trigger, type: type, repeatCount: repeatCount, repeatInterval: repeatCount > 0 ? interval : nil,
                        isCalendarDefault: isDefault)
    }

    /// `geo:lat,long` with `X-TITLE` and `X-APPLE-RADIUS` parameters.
    static func structuredLocation(_ property: ICalProperty) -> StructuredLocation {
        var latitude: Double?, longitude: Double?
        if property.value.lowercased().hasPrefix("geo:") {
            let parts = property.value.dropFirst(4).split(separator: ",")
            if parts.count >= 2 { latitude = Double(parts[0]); longitude = Double(parts[1]) }
        }
        return StructuredLocation(title: property.parameter("X-TITLE"), latitude: latitude, longitude: longitude,
                                  radius: property.parameter("X-APPLE-RADIUS").flatMap(Double.init))
    }

    /// `VALARM`s for reminders a CalDAV server stores as written: relative to the start or end, or absolute, as a
    /// display or sound alert, optionally repeating. Anything else throws `.unsupported(fields: [.reminders])`.
    public static func alarms(from reminders: [Reminder]) throws -> [ICalComponent] {
        try reminders.map { reminder in
            var properties: [ICalProperty] = []
            switch reminder.type {
            case .display:
                properties += [ICalProperty(name: "ACTION", value: "DISPLAY"), ICalProperty(name: "DESCRIPTION", text: "Reminder")]
            case .audio(let sound):
                properties.append(ICalProperty(name: "ACTION", value: "AUDIO"))
                if let sound { properties.append(ICalProperty(name: "ATTACH", parameters: [ICalParameter("VALUE", "URI")], value: sound)) }
            default:
                throw WriteError.unsupported(fields: [.reminders])
            }
            switch reminder.trigger {
            case .relative(let offset, let anchor):
                properties.append(ICalProperty(name: "TRIGGER", parameters: anchor == .end ? [ICalParameter("RELATED", "END")] : [],
                                               value: ICalValues.durationText(offset)))
            case .absolute(let date):
                properties.append(ICalProperty(name: "TRIGGER", parameters: [ICalParameter("VALUE", "DATE-TIME")], value: ICalValues.utcText(date)))
            case .location:
                throw WriteError.unsupported(fields: [.reminders])
            }
            if reminder.repeatCount > 0, let interval = reminder.repeatInterval {
                properties += [ICalProperty(name: "REPEAT", value: String(reminder.repeatCount)),
                               ICalProperty(name: "DURATION", value: ICalValues.durationText(interval))]
            }
            return ICalComponent(name: "VALARM", properties: properties)
        }
    }
}
