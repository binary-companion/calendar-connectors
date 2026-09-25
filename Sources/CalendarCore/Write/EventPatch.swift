import Foundation

/// A field change where "leave alone" and "remove" are different things.
public enum FieldUpdate<Value: Sendable & Equatable>: Sendable, Equatable {
    case keep, set(Value), clear
}

/// A delta on an event's attendees, never a replacement list, so an edit cannot wipe other people's responses.
/// `add` upserts by email: an existing attendee keeps their response and takes the new name and role.
public struct AttendeeChanges: Sendable, Equatable {
    public var add: [AttendeeDraft]
    /// Normalized (trimmed, lowercased) emails.
    public var remove: [String]

    public init(add: [AttendeeDraft] = [], remove: [String] = []) {
        self.add = add
        self.remove = remove.map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
    }

    public var isEmpty: Bool { add.isEmpty && remove.isEmpty }
}

/// The changes to make to an existing event. Only these fields are sent, so data the library does not model
/// (attachments, extended properties, colors) is never disturbed.
public struct EventPatch: Sendable, Equatable {
    public var title: String?
    public var notes: FieldUpdate<String>
    public var location: FieldUpdate<String>
    /// Time is one unit: start, end, zone and all-day together.
    public var timing: EventTiming?
    public var availability: Availability?
    public var visibility: Visibility?
    /// `.clear` = the calendar's default reminders; `.set([])` = none.
    public var reminders: FieldUpdate<[Reminder]>
    public var attendees: AttendeeChanges?
    public var recurrence: FieldUpdate<RecurrenceRule>
    public var conference: ConferenceChange?
    /// The original the patch was diffed from; what conflicts are judged against. nil for a hand-built patch,
    /// which conflicts on any stale version.
    public private(set) var base: CalendarEvent?

    public init(
        title: String? = nil, notes: FieldUpdate<String> = .keep, location: FieldUpdate<String> = .keep,
        timing: EventTiming? = nil, availability: Availability? = nil, visibility: Visibility? = nil,
        reminders: FieldUpdate<[Reminder]> = .keep, attendees: AttendeeChanges? = nil,
        recurrence: FieldUpdate<RecurrenceRule> = .keep, conference: ConferenceChange? = nil
    ) {
        self.title = title
        self.notes = notes
        self.location = location
        self.timing = timing
        self.availability = availability
        self.visibility = visibility
        self.reminders = reminders
        self.attendees = attendees
        self.recurrence = recurrence
        self.conference = conference
        self.base = nil
    }

    public var touchedFields: Set<EventField> {
        var fields = Set<EventField>()
        if title != nil { fields.insert(.title) }
        if notes != .keep { fields.insert(.notes) }
        if location != .keep { fields.insert(.location) }
        if timing != nil { fields.insert(.timing) }
        if availability != nil { fields.insert(.availability) }
        if visibility != nil { fields.insert(.visibility) }
        if reminders != .keep { fields.insert(.reminders) }
        if let attendees, !attendees.isEmpty { fields.insert(.attendees) }
        if recurrence != .keep { fields.insert(.recurrence) }
        if conference != nil { fields.insert(.conference) }
        return fields
    }

    public var isEmpty: Bool { touchedFields.isEmpty }

    public func withoutBase() -> EventPatch {
        var copy = self
        copy.base = nil
        return copy
    }

    /// The minimal patch that turns `original` into `edited`. Compares title, notes, location, timing, availability,
    /// visibility, reminders and attendees (by normalized email; a changed role or name is an upsert), plus the
    /// removal of a conference. Ignores provider-owned fields (ids, uid, calendar, source, organizer, status, kind,
    /// url, version, series fields, `myResponse`, attendee responses and flags), a changed or added conference
    /// (only `.generate` and `.remove` are writable) and recurrence (reads carry none).
    public init(from original: CalendarEvent, to edited: CalendarEvent) {
        self.init()
        if edited.title != original.title { title = edited.title }
        notes = Self.update(from: original.notes, to: edited.notes)
        location = Self.update(from: original.location, to: edited.location)
        let sameTiming = original.start == edited.start && original.end == edited.end
            && original.timeZone?.identifier == edited.timeZone?.identifier && original.isAllDay == edited.isAllDay
        if !sameTiming {
            timing = EventTiming(start: edited.start, end: edited.end, timeZone: edited.timeZone, isAllDay: edited.isAllDay)
        }
        if edited.availability != original.availability { availability = edited.availability }
        if edited.visibility != original.visibility { visibility = edited.visibility }
        if edited.reminders != original.reminders { reminders = .set(edited.reminders) }
        attendees = Self.attendeeChanges(from: original.attendees, to: edited.attendees)
        if original.conference != nil && edited.conference == nil { conference = .remove }
        base = original
    }

    private static func update(from old: String?, to new: String?) -> FieldUpdate<String> {
        guard old != new else { return .keep }
        return new.map { .set($0) } ?? .clear
    }

    private static func attendeeChanges(from original: [Attendee], to edited: [Attendee]) -> AttendeeChanges? {
        // Provider data may repeat an email; the first occurrence wins (and must not trap the dictionary).
        let before = Dictionary(original.filter { !$0.isSelf }.compactMap { a in a.email.map { ($0, a) } },
                                uniquingKeysWith: { first, _ in first })
        // The account owner is never added or removed by an edit, even if the copy lost the `isSelf` flag.
        let selfEmails = Set(original.filter(\.isSelf).compactMap(\.email))
        var add: [AttendeeDraft] = []
        var seen = Set<String>()
        for attendee in edited where !attendee.isSelf {
            guard let email = attendee.email, !selfEmails.contains(email), seen.insert(email).inserted else { continue }
            // A draft cannot clear a name, so dropping one is not a change (it would touch attendees for nothing).
            if let old = before[email], old.role == attendee.role, attendee.name == nil || old.name == attendee.name { continue }
            add.append(AttendeeDraft(email: email, name: attendee.name, role: attendee.role))
        }
        let remove = before.keys.filter { !seen.contains($0) }.sorted()
        let changes = AttendeeChanges(add: add, remove: remove)
        return changes.isEmpty ? nil : changes
    }

    /// The event with this patch applied. Recurrence and a generated conference are not representable on
    /// `CalendarEvent`; `.remove` clears the conference. Used by the in-memory test source.
    public func applied(to event: CalendarEvent) -> CalendarEvent {
        var e = event
        if let title { e.title = title }
        switch notes { case .keep: break; case .set(let v): e.notes = v; case .clear: e.notes = nil }
        switch location { case .keep: break; case .set(let v): e.location = v; case .clear: e.location = nil }
        if let timing {
            e.start = timing.start
            e.end = timing.end
            e.timeZone = timing.timeZone
            e.isAllDay = timing.isAllDay
        }
        if let availability { e.availability = availability }
        if let visibility { e.visibility = visibility }
        switch reminders { case .keep: break; case .set(let v): e.reminders = v; case .clear: e.reminders = [] }
        if let attendees {
            let removed = Set(attendees.remove)
            e.attendees.removeAll { $0.email.map(removed.contains) ?? false }
            for draft in attendees.add {
                if let i = e.attendees.firstIndex(where: { $0.email == draft.email }) {
                    if let name = draft.name { e.attendees[i].name = name }
                    e.attendees[i].role = draft.role
                } else {
                    e.attendees.append(Attendee(name: draft.name, email: draft.email, role: draft.role))
                }
            }
        }
        if conference == .remove { e.conference = nil }
        return e
    }
}

/// A tracked edit: the event as read plus a working copy. `patch` is what changed, `hasChanges` whether anything did.
public struct EventEdit: Sendable {
    public let original: CalendarEvent
    public var event: CalendarEvent

    public init(_ original: CalendarEvent) {
        self.original = original
        self.event = original
    }

    public var patch: EventPatch { EventPatch(from: original, to: event) }
    public var hasChanges: Bool { !patch.isEmpty }
}
