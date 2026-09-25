import CalendarCore
import Foundation

/// An in-memory `WritableCalendarSource` for tests. Non-recurring events only: the scope is ignored and recurrence is
/// not modelled, so by default `.recurrence` is not among the writable fields and a draft or patch that sets it throws
/// `.unsupported(fields: [.recurrence])`. It never has a self attendee of its own; a test that needs one adds it with
/// `simulateExternalEdit`.
public final class FakeWritableSource: WritableCalendarSource, @unchecked Sendable {
    public let id = "fake-writable"
    public let displayName = "Fake writable"
    public let capabilities: SourceCapabilities

    private let lock = NSLock()
    private let calendarIDs: Set<String>
    private var events: [String: CalendarEvent] = [:]   // by `CalendarEvent.id`
    private var counter = 0
    private var writes = 0

    public init(calendarIDs: [String] = ["cal"], writableFields: Set<EventField> = Set(EventField.allCases).subtracting([.recurrence]), canRespond: Bool = true) {
        self.calendarIDs = Set(calendarIDs)
        self.capabilities = SourceCapabilities(
            canWrite: true, canEditAttendees: writableFields.contains(.attendees), canRespondToInvite: canRespond,
            writableFields: writableFields, controlsNotifications: true, recurrenceScopes: Set(RecurrenceScope.allCases))
    }

    /// Number of successful writes; lets tests assert that a call wrote nothing.
    public var writeCount: Int { lock.withLock { writes } }

    /// Simulates another writer: applies `mutate` and bumps the version. Returns false (and does nothing) when there is
    /// no such event, so a test that asserts the result notices a mistyped id.
    @discardableResult
    public func simulateExternalEdit(calendarID: String, eventID: String, _ mutate: (inout CalendarEvent) -> Void) -> Bool {
        lock.withLock {
            let key = "\(calendarID)/\(eventID)"
            guard var event = events[key] else { return false }
            mutate(&event)
            counter += 1
            event.version = "v\(counter)"
            events[key] = event
            return true
        }
    }

    public func calendars() async throws -> [CalendarDescriptor] {
        calendarIDs.sorted().map { CalendarDescriptor(id: $0, title: $0, accessRole: .owner) }
    }

    public func events(in interval: DateInterval) async throws -> [CalendarEvent] {
        lock.withLock { events.values.filter { $0.start < interval.end && $0.end > interval.start }.sorted { ($0.start, $0.id) < ($1.start, $1.id) } }
    }

    public func changes() -> AsyncStream<CalendarChange> { AsyncStream { $0.finish() } }

    public func create(_ draft: EventDraft, in calendarID: String, notify: NotifyPolicy) async throws -> CalendarEvent {
        try draft.validate()
        try WriteValidation.requireWritable(draft.usedFields, capabilities)
        guard calendarIDs.contains(calendarID) else { throw WriteError.notFound }
        return lock.withLock {
            counter += 1
            writes += 1
            var event = CalendarEvent(
                eventID: "ev\(counter)", calendarID: calendarID, title: draft.title, notes: draft.notes, location: draft.location,
                start: draft.timing.start, end: draft.timing.end, timeZone: draft.timing.timeZone ?? TimeZone(identifier: "UTC")!, isAllDay: draft.timing.isAllDay,
                availability: draft.availability, visibility: draft.visibility,
                attendees: draft.attendees.map { Attendee(name: $0.name, email: $0.email, role: $0.role) },
                reminders: draft.reminders ?? [], version: "v\(counter)", sourceID: id)
            if draft.conference == .generate {
                event.conferences = [ConferenceInfo(url: URL(string: "https://meet.example/\(counter)")!, provider: .meet)]
            }
            events[event.id] = event
            return event
        }
    }

    public func update(_ ref: EventRef, _ patch: EventPatch, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent {
        try WriteValidation.requireWritable(patch.touchedFields, capabilities)
        try Self.validate(patch)
        let key = "\(ref.calendarID)/\(ref.eventID)"
        guard let existing = lock.withLock({ events[key] }) else { throw WriteError.notFound }
        if patch.isEmpty { return patch.base ?? existing }
        return try await PatchMerge.apply(
            patch: patch, version: ref.version,
            fetchCurrent: { self.lock.withLock { self.events[key] } ?? existing },
            write: { version in try self.write(key, version, patch) })
    }

    /// The patch's attendee emails, reminders and timing get the same checks as a draft's. Only the patch's values
    /// matter, so they ride on a throwaway draft.
    private static func validate(_ patch: EventPatch) throws {
        let placeholder = Date(timeIntervalSince1970: 0)
        var reminders: [Reminder]?
        if case .set(let set) = patch.reminders { reminders = set }
        try EventDraft(
            title: "", timing: patch.timing ?? EventTiming(start: placeholder, end: placeholder.addingTimeInterval(1), timeZone: nil, isAllDay: false),
            reminders: reminders, attendees: patch.attendees?.add ?? [], recurrence: nil).validate()
    }

    private func write(_ key: String, _ version: String?, _ patch: EventPatch) throws -> PatchMerge.Attempt<CalendarEvent> {
        try lock.withLock {
            guard let current = events[key] else { throw WriteError.notFound }
            if let version, current.version != version { return .stale }
            var updated = patch.applied(to: current)
            if patch.conference == .generate {
                updated.conferences = [ConferenceInfo(url: URL(string: "https://meet.example/\(counter + 1)")!, provider: .meet)]
            }
            counter += 1
            writes += 1
            updated.version = "v\(counter)"
            events[key] = updated
            return .done(updated)
        }
    }

    public func delete(_ ref: EventRef, scope: RecurrenceScope, notify: NotifyPolicy) async throws {
        try lock.withLock {
            guard events.removeValue(forKey: "\(ref.calendarID)/\(ref.eventID)") != nil else { throw WriteError.notFound }
            writes += 1
        }
    }

    public func respond(to ref: EventRef, _ response: ResponseStatus, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent {
        guard capabilities.canRespondToInvite else { throw WriteError.unsupported(fields: [.attendees]) }
        guard response != .needsAction else { throw WriteError.invalid("cannot respond with needsAction") }
        let key = "\(ref.calendarID)/\(ref.eventID)"
        return try lock.withLock {
            guard var event = events[key] else { throw WriteError.notFound }
            guard let index = event.attendees.firstIndex(where: \.isSelf) else { throw WriteError.invalid("you are not an attendee of this event") }
            event.attendees[index].response = response
            event.myResponse = response
            counter += 1
            writes += 1
            event.version = "v\(counter)"
            events[key] = event
            return event
        }
    }
}
