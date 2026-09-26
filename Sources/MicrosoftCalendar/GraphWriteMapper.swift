import CalendarCore
import Foundation

/// Where a series starts, for turning a recurrence rule into Graph's `patternedRecurrence`: the first occurrence's date,
/// read in the zone the rule is anchored to.
struct RecurrenceAnchor {
    var start: CalendarDate
    var zone: TimeZone

    /// For validating a patch before the real anchor is known; the mapping errors do not depend on it.
    static let placeholder = RecurrenceAnchor(start: CalendarDate(year: 2000, month: 1, day: 3), zone: TimeZone(identifier: "UTC")!)
}

/// Pure translation of the library's write types into Graph JSON. Nothing here does I/O, so every rule is testable.
/// Anything Graph cannot store is `WriteError.unsupported`; nothing is silently dropped.
enum GraphWriteMapper {
    private static let utc = TimeZone(identifier: "UTC")!

    static func data(_ json: [String: Any]) throws -> Data {
        try JSONSerialization.data(withJSONObject: json, options: [.sortedKeys])
    }

    // MARK: Create

    static func createBody(_ draft: EventDraft) throws -> [String: Any] {
        try draft.validate()
        var body: [String: Any] = ["subject": draft.title]
        for (key, value) in try timeFields(draft.timing) { body[key] = value }
        if let notes = draft.notes { body["body"] = textBody(notes) }
        if let location = draft.location { body["location"] = ["displayName": location] }
        body["showAs"] = showAs(draft.availability)
        body["sensitivity"] = sensitivity(draft.visibility)
        if let reminders = draft.reminders { for (key, value) in try reminderFields(reminders) { body[key] = value } }
        if !draft.attendees.isEmpty { body["attendees"] = draft.attendees.map(attendee) }
        if draft.conference == .generate { for (key, value) in teamsFields { body[key] = value } }
        if let rule = draft.recurrence {
            let zone = draft.timing.timeZone ?? utc
            body["recurrence"] = try GraphRecurrenceMapper.recurrence(
                from: rule, start: AllDay.date(of: draft.timing.start, in: zone), in: zone)
        }
        return body
    }

    // MARK: Update

    /// The body of a PATCH: only the fields the patch touches. `currentAttendees` is the event's attendee list as
    /// Graph returned it (Graph replaces the whole list, so an edit starts from it); `anchor` is where the series
    /// starts, used only when the patch sets a recurrence.
    static func patchBody(_ patch: EventPatch, currentAttendees: [[String: Any]], anchor: RecurrenceAnchor) throws -> [String: Any] {
        var body: [String: Any] = [:]
        if let title = patch.title { body["subject"] = title }
        switch patch.notes {
        case .keep: break
        case .set(let notes): body["body"] = textBody(notes)
        case .clear: body["body"] = textBody("")
        }
        switch patch.location {
        case .keep: break
        case .set(let location): body["location"] = ["displayName": location]
        case .clear: body["location"] = ["displayName": ""]
        }
        if let timing = patch.timing {
            try timing.validate()
            for (key, value) in try timeFields(timing) { body[key] = value }
        }
        if let availability = patch.availability { body["showAs"] = showAs(availability) }
        if let visibility = patch.visibility { body["sensitivity"] = sensitivity(visibility) }
        switch patch.reminders {
        case .keep: break
        case .set(let reminders): for (key, value) in try reminderFields(reminders) { body[key] = value }
        case .clear: throw WriteError.unsupported(fields: [.reminders])   // Graph has no "calendar default" to fall back to
        }
        if let changes = patch.attendees, !changes.isEmpty {
            body["attendees"] = attendeeList(current: currentAttendees, changes: changes)
        }
        switch patch.conference {
        case nil: break
        case .generate?: for (key, value) in teamsFields { body[key] = value }
        case .remove?: body["isOnlineMeeting"] = false
        }
        switch patch.recurrence {
        case .keep: break
        case .set(let rule): body["recurrence"] = try GraphRecurrenceMapper.recurrence(from: rule, start: anchor.start, in: anchor.zone)
        case .clear: throw WriteError.unsupported(fields: [.recurrence])   // turning a series into a single event is not offered
        }
        return body
    }

    // MARK: Respond

    /// The action segment of `POST .../events/{id}/<action>`.
    static func respondAction(_ response: ResponseStatus) throws -> String {
        switch response {
        case .accepted: return "accept"
        case .tentative: return "tentativelyAccept"
        case .declined: return "decline"
        case .needsAction: throw WriteError.invalid("a response must be accepted, tentative or declined")
        }
    }

    // MARK: Pieces

    /// `start`, `end` and `isAllDay`. A timed event is written in its zone's Windows name; a zone with no Windows name
    /// is written in UTC with the same instants. An all-day event is written as midnight of its first day and of the day
    /// after its last, in the zone's Windows name (or UTC when there is none, which keeps the dates).
    static func timeFields(_ timing: EventTiming) throws -> [String: Any] {
        if timing.isAllDay {
            guard let zone = timing.timeZone else { throw WriteError.invalid("an all-day event needs a time zone") }
            let name = WindowsTimeZones.windowsName(for: zone) ?? "UTC"
            let first = AllDay.date(of: timing.start, in: zone)
            let after = AllDay.date(of: timing.end, in: zone)
            return [
                "isAllDay": true,
                "start": ["dateTime": GraphTime.midnight(first), "timeZone": name],
                "end": ["dateTime": GraphTime.midnight(after), "timeZone": name],
            ]
        }
        let zone = timing.timeZone ?? utc
        let (writtenZone, name) = WindowsTimeZones.windowsName(for: zone).map { (zone, $0) } ?? (utc, "UTC")
        return [
            "isAllDay": false,
            "start": ["dateTime": GraphTime.format(timing.start, in: writtenZone), "timeZone": name],
            "end": ["dateTime": GraphTime.format(timing.end, in: writtenZone), "timeZone": name],
        ]
    }

    static func textBody(_ text: String) -> [String: Any] { ["contentType": "text", "content": text] }

    static func showAs(_ availability: Availability) -> String {
        switch availability {
        case .busy: "busy"
        case .free: "free"
        case .tentative: "tentative"
        case .unavailable: "oof"
        }
    }

    static func sensitivity(_ visibility: Visibility) -> String {
        switch visibility {
        case .default, .publicEvent: "normal"
        case .privateEvent: "private"
        case .confidential: "confidential"
        }
    }

    /// Graph keeps one reminder per event. An empty list turns it off; one plain on-screen reminder before the start
    /// turns it on; anything else (several, a repeat, an email, one counted from the end) is refused.
    static func reminderFields(_ reminders: [Reminder]) throws -> [String: Any] {
        if reminders.isEmpty { return ["isReminderOn": false] }
        guard reminders.count == 1, let minutes = reminders[0].minutesBefore, minutes >= 0,
              reminders[0].type == .display, reminders[0].repeatCount == 0
        else { throw WriteError.unsupported(fields: [.reminders]) }
        return ["isReminderOn": true, "reminderMinutesBeforeStart": minutes]
    }

    /// Asks Graph for a Teams meeting. (Personal accounts may not support it; that comes back as an error.)
    static var teamsFields: [String: Any] { ["isOnlineMeeting": true, "onlineMeetingProvider": "teamsForBusiness"] }

    private static func roleName(_ role: AttendeeRole) -> String {
        switch role {
        case .required: "required"
        case .optional: "optional"
        case .resource: "resource"
        }
    }

    static func attendee(_ draft: AttendeeDraft) -> [String: Any] {
        var address: [String: Any] = ["address": draft.email]
        if let name = draft.name { address["name"] = name }
        return ["emailAddress": address, "type": roleName(draft.role)]
    }

    private static func address(of item: [String: Any]) -> String? {
        ((item["emailAddress"] as? [String: Any])?["address"] as? String)?.lowercased()
    }

    /// The attendee list after `changes`: the current people (their responses stay on the server), minus those removed,
    /// with each added person inserted or updated by email. Only the address, name and type are sent.
    static func attendeeList(current: [[String: Any]], changes: AttendeeChanges) -> [[String: Any]] {
        let removed = Set(changes.remove)
        var list: [[String: Any]] = current.compactMap { item in
            guard let email = address(of: item), !removed.contains(email) else { return nil }
            var kept: [String: Any] = ["emailAddress": item["emailAddress"] as Any]
            kept["type"] = item["type"] ?? "required"
            return kept
        }
        for draft in changes.add {
            if let index = list.firstIndex(where: { address(of: $0) == draft.email }) {
                list[index]["type"] = roleName(draft.role)
                if let name = draft.name, var emailAddress = list[index]["emailAddress"] as? [String: Any] {
                    emailAddress["name"] = name
                    list[index]["emailAddress"] = emailAddress
                }
            } else {
                list.append(attendee(draft))
            }
        }
        return list
    }

    // MARK: Splitting a series

    /// The fields of a series master that the series which continues after a split keeps: text, availability,
    /// sensitivity, reminder, guests and a Teams meeting. Times and recurrence are set by the caller.
    static func newSeriesBase(from master: [String: Any]) -> [String: Any] {
        var base: [String: Any] = [:]
        for key in ["subject", "showAs", "sensitivity", "isReminderOn", "reminderMinutesBeforeStart"] {
            if let value = master[key] { base[key] = value }
        }
        if let body = master["body"] as? [String: Any], let content = body["content"] as? String, !content.isEmpty {
            base["body"] = textBody(content)
        }
        if let name = (master["location"] as? [String: Any])?["displayName"] as? String, !name.isEmpty {
            base["location"] = ["displayName": name]
        }
        if let attendees = master["attendees"] as? [[String: Any]], !attendees.isEmpty {
            base["attendees"] = attendeeList(current: attendees, changes: AttendeeChanges())
        }
        // A new meeting is created for the new series (the old link belongs to the old one).
        if master["isOnlineMeeting"] as? Bool == true { for (key, value) in teamsFields { base[key] = value } }
        return base
    }
}
