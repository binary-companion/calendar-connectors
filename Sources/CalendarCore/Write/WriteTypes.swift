import Foundation

/// A part of an event that a write can touch. Used for validation, conflict reports and capabilities.
public enum EventField: String, Sendable, Hashable, CaseIterable {
    case title, notes, location, timing, availability, visibility, reminders, attendees, recurrence, conference
}

/// Whether the provider may email attendees about a write. Callers must choose; there is no default.
public enum NotifyPolicy: Sendable, Hashable { case all, externalOnly, none }

/// Which occurrences of a recurring event a write applies to. Ignored for an event that is not part of a series.
public enum RecurrenceScope: Sendable, Hashable, CaseIterable { case thisInstance, thisAndFollowing, allInSeries }

/// On a draft: whether to ask the provider for a generated conference link.
public enum ConferenceRequest: Sendable, Hashable { case none, generate }
/// On a patch: generate a link or remove the existing one.
public enum ConferenceChange: Sendable, Hashable { case generate, remove }

public enum WriteError: Error, Sendable, Equatable {
    /// The connector cannot write these fields (or the operation; RSVP is reported as `.attendees`).
    case unsupported(fields: Set<EventField>)
    /// Someone else changed a field this patch touches.
    case conflict(fields: Set<EventField>)
    /// The event or calendar is gone.
    case notFound
    /// A read-only calendar, or no permission on this event.
    case forbidden(String?)
    /// Malformed input: end before start, RSVP of `.needsAction`, a bad recurrence rule, ...
    case invalid(String)
    /// A create found an event with the draft's `uid` already on the target calendar; nothing was written. The payload
    /// is the calendar's stored copy, so the caller can decide whether to update it.
    case alreadyExists(CalendarEvent)
    /// A multi-step write stopped half way (Google `.thisAndFollowing`: series truncated, new series not created).
    case partial(String)
}

/// What a write needs to find an event. Build one from the event you read.
public struct EventRef: Hashable, Sendable {
    public var calendarID: String
    public var eventID: String
    /// The version the caller last saw; used for optimistic locking.
    public var version: String?
    /// An empty id means "no series" (normalised to nil here and in `init`).
    public var seriesID: String? { didSet { if seriesID?.isEmpty == true { seriesID = nil } } }
    /// The occurrence's slot in its series (Google `originalStartTime`, EventKit `occurrenceDate`); differs from
    /// `start` for a moved occurrence. It identifies the occurrence when `eventID` is shared and is the split
    /// point for `.thisAndFollowing`.
    public var originalStart: Date?

    public init(calendarID: String, eventID: String, version: String? = nil, seriesID: String? = nil, originalStart: Date? = nil) {
        self.calendarID = calendarID
        self.eventID = eventID
        self.version = version
        // An empty id means "no series" for every connector (Google and EventKit would otherwise read `""` differently).
        self.seriesID = seriesID?.isEmpty == true ? nil : seriesID
        self.originalStart = originalStart
    }

    public init(_ event: CalendarEvent) {
        self.init(calendarID: event.calendarID, eventID: event.eventID, version: event.version,
                  seriesID: event.seriesID, originalStart: event.originalStart)
    }
}

public enum WriteValidation {
    /// Throws `.unsupported` naming every field in `fields` that `capabilities.writableFields` lacks.
    public static func requireWritable(_ fields: Set<EventField>, _ capabilities: SourceCapabilities) throws {
        let missing = fields.subtracting(capabilities.writableFields)
        if !missing.isEmpty { throw WriteError.unsupported(fields: missing) }
    }
}
