import Foundation

public enum SyncKind: Sendable { case none, token, notification }

/// A field a source reliably fills on the events (or calendars) it reads. A listed field is never nil; an unlisted one
/// may be nil or partly filled, and nil means "this source does not say", never "none".
public enum ProvidedField: String, Sendable, Hashable, CaseIterable {
    // Events
    case kind, visibility, availability, reminders, series, participation
    /// Supplies structured conference data (Google `conferenceData`); says nothing about whether an event has links.
    case structuredConference
    case version
}

public struct SourceCapabilities: Equatable, Sendable {
    public var canWrite: Bool
    public var canEditAttendees: Bool
    public var canRespondToInvite: Bool
    /// What this source reliably fills on the events it reads (see `ProvidedField`). Separate from `writableFields`,
    /// which is what a write can change; a few names overlap (reminders, availability) with different meanings.
    public var providedFields: Set<ProvidedField>
    public var syncKind: SyncKind
    public var supportsPush: Bool
    /// The fields create/update can write; drives validation and `EventDraft(copying:for:)`. Empty when read-only.
    public var writableFields: Set<EventField>
    /// Honors `NotifyPolicy` (Google: `sendUpdates`); false means the server decides.
    public var controlsNotifications: Bool
    /// Scopes accepted for update/delete on a recurring series. Empty when read-only.
    public var recurrenceScopes: Set<RecurrenceScope>

    public init(
        canWrite: Bool = false, canEditAttendees: Bool = false, canRespondToInvite: Bool = false,
        providedFields: Set<ProvidedField> = [], syncKind: SyncKind = .none, supportsPush: Bool = false,
        writableFields: Set<EventField> = [], controlsNotifications: Bool = false,
        recurrenceScopes: Set<RecurrenceScope> = []
    ) {
        self.canWrite = canWrite
        self.canEditAttendees = canEditAttendees
        self.canRespondToInvite = canRespondToInvite
        self.providedFields = providedFields
        self.syncKind = syncKind
        self.supportsPush = supportsPush
        self.writableFields = writableFields
        self.controlsNotifications = controlsNotifications
        self.recurrenceScopes = recurrenceScopes
    }
}

/// What the library reports through `changes()`. Sources throw `SourceError`; they never leak provider types.
public enum SourceError: Error, Sendable, Equatable {
    case authExpired
    case network(String)
    case rateLimited(retryAfter: TimeInterval?)
    case server(status: Int)
    case invalidResponse(String)
    /// The OS or user has not granted access to a local data store (EventKit). Never thrown by network connectors.
    case needsPermission
}

public enum CalendarChange: Equatable, Sendable {
    /// The calendar set changed. Consumers must reload the calendar list AND events; event changes detected in the
    /// same check are not reported separately.
    case calendarsChanged
    /// nil calendar ids mean the scope is unknown.
    case eventsChanged(calendarIDs: Set<String>?)
    /// Terminal: the stream finishes after this (e.g. `.authExpired`) so the app can surface it and re-authorize.
    case sourceFailed(SourceError)
}

public protocol CalendarSource: Sendable {
    var id: String { get }
    var displayName: String { get }
    var capabilities: SourceCapabilities { get }
    func calendars() async throws -> [CalendarDescriptor]
    /// Events of every visible calendar of the account that overlap `interval`.
    func events(in interval: DateInterval) async throws -> [CalendarEvent]
    func changes() -> AsyncStream<CalendarChange>
}

public protocol PollingCalendarSource: CalendarSource {
    /// One cheap incremental check (sync token / delta). Returns the change since the last call, or nil for none.
    /// The first call establishes the baseline and returns nil.
    func checkForChanges() async throws -> CalendarChange?
}
