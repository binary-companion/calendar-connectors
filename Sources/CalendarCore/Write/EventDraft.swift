import Foundation

/// Start, end, zone and all-day as one unit, so a write can never carry half a time range.
/// All-day timings use the library's canonical form (midnight of the first day in `timeZone`, `end` exclusive).
public struct EventTiming: Sendable, Equatable {
    public var start: Date
    public var end: Date
    public var timeZone: TimeZone?
    public var isAllDay: Bool

    public init(start: Date, end: Date, timeZone: TimeZone?, isAllDay: Bool) {
        self.start = start
        self.end = end
        self.timeZone = timeZone
        self.isAllDay = isAllDay
    }

    /// Every write calls this before any request or store mutation.
    public func validate() throws {
        // Checked first: NaN and infinite dates must not reach the ordering or calendar math below.
        guard start.timeIntervalSinceReferenceDate.isFinite, end.timeIntervalSinceReferenceDate.isFinite else {
            throw WriteError.invalid("times must be finite")
        }
        guard end > start else { throw WriteError.invalid("end must be after start") }
        guard isAllDay else { return }
        guard let zone = timeZone else { throw WriteError.invalid("an all-day event needs a time zone") }
        for instant in [start, end] where AllDay.startOfDay(AllDay.date(of: instant, in: zone), in: zone) != instant {
            throw WriteError.invalid("all-day times must be midnight in \(zone.identifier)")
        }
    }
}

public struct AttendeeDraft: Sendable, Hashable {
    public var name: String?
    public private(set) var email: String
    public var role: AttendeeRole

    public init(email: String, name: String? = nil, role: AttendeeRole = .required) {
        self.email = email.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        self.name = name
        self.role = role
    }

    /// One `@` with something on both sides; no whitespace, control characters or `, ; < >` (which would smuggle a
    /// second address or a display-name form into the provider request). Deliberately not a full RFC check; the
    /// provider has the final say.
    static func isValidEmail(_ email: String) -> Bool {
        let parts = email.split(separator: "@", omittingEmptySubsequences: false)
        guard parts.count == 2, !parts[0].isEmpty, !parts[1].isEmpty else { return false }
        let forbidden = CharacterSet.whitespacesAndNewlines.union(.controlCharacters).union(CharacterSet(charactersIn: ",;<>"))
        return email.unicodeScalars.allSatisfy { !forbidden.contains($0) }
    }
}

/// Everything needed to create an event. `EventDraft(copying:for:)` is the one lenient path.
public struct EventDraft: Sendable, Equatable {
    public var title: String
    public var notes: String?
    public var location: String?
    public var timing: EventTiming
    public var availability: Availability
    public var visibility: Visibility
    /// nil = the calendar's default reminders; empty = none.
    public var reminders: [Reminder]?
    public var attendees: [AttendeeDraft]
    public var conference: ConferenceRequest
    public var recurrence: RecurrenceRule?

    public init(
        title: String, timing: EventTiming, notes: String? = nil, location: String? = nil,
        availability: Availability = .busy, visibility: Visibility = .default, reminders: [Reminder]? = nil,
        attendees: [AttendeeDraft] = [], conference: ConferenceRequest = .none, recurrence: RecurrenceRule? = nil
    ) {
        self.title = title
        self.timing = timing
        self.notes = notes
        self.location = location
        self.availability = availability
        self.visibility = visibility
        self.reminders = reminders
        self.attendees = attendees
        self.conference = conference
        self.recurrence = recurrence
    }

    public func validate() throws {
        try timing.validate()
        try recurrence?.validate()
        for attendee in attendees where !AttendeeDraft.isValidEmail(attendee.email) {
            throw WriteError.invalid("attendee email is not valid: \(attendee.email)")
        }
        if let reminders, reminders.contains(where: { $0.minutesBefore < 0 }) {
            throw WriteError.invalid("reminder minutes must not be negative")
        }
    }

    /// The fields this draft sets beyond their defaults; connectors check them against `writableFields`.
    public var usedFields: Set<EventField> {
        var fields: Set<EventField> = [.title, .timing]
        if notes != nil { fields.insert(.notes) }
        if location != nil { fields.insert(.location) }
        if availability != .busy { fields.insert(.availability) }
        if visibility != .default { fields.insert(.visibility) }
        if reminders != nil { fields.insert(.reminders) }
        if !attendees.isEmpty { fields.insert(.attendees) }
        if conference != .none { fields.insert(.conference) }
        if recurrence != nil { fields.insert(.recurrence) }
        return fields
    }

    /// The enum-valued fields a connector maps to the closest value the calendar supports, when the stored copy differs
    /// from what was requested. Other provider normalization (reminder defaults, attendee order, all-day zones, text)
    /// is deliberately not reported: it is visible in the returned event but is not an adjustment the library made.
    public func adjustments(comparedTo stored: CalendarEvent) -> Set<EventField> {
        var changed: Set<EventField> = []
        if let value = stored.availability, value != availability { changed.insert(.availability) }
        if let value = stored.visibility, value != visibility { changed.insert(.visibility) }
        return changed
    }

    /// A best-effort copy of `event` for a target with `capabilities`: only fields in `writableFields` are carried
    /// over. Self and email-less attendees are dropped, an empty reminder list becomes "calendar defaults" (reads
    /// cannot tell the two apart), only a Meet link is re-requested, and recurrence is never copied (reads carry none).
    /// Attendees with an invalid email and negative reminders are dropped, so those two never make `validate()` fail.
    /// Timing is copied as is: a source event with a zero-length or inverted interval still fails `validate()`.
    public init(copying event: CalendarEvent, for capabilities: SourceCapabilities) {
        let writable = capabilities.writableFields
        let reminders = (event.reminders ?? []).filter { $0.minutesBefore >= 0 }
        self.init(
            title: event.title,
            timing: EventTiming(start: event.start, end: event.end, timeZone: event.timeZone, isAllDay: event.isAllDay),
            notes: writable.contains(.notes) ? event.notes : nil,
            location: writable.contains(.location) ? event.location : nil,
            availability: writable.contains(.availability) ? event.availability ?? .busy : .busy,
            visibility: writable.contains(.visibility) ? event.visibility ?? .default : .default,
            reminders: writable.contains(.reminders) && !reminders.isEmpty ? reminders : nil,
            attendees: writable.contains(.attendees)
                ? event.attendees.filter { !$0.isSelf }
                    .compactMap { a in a.email.map { AttendeeDraft(email: $0, name: a.name, role: a.role) } }
                    .filter { AttendeeDraft.isValidEmail($0.email) }
                : [],
            conference: writable.contains(.conference) && event.conferences.contains { $0.origin == .structured && $0.provider == .meet } ? .generate : .none,
            recurrence: nil)
    }
}
