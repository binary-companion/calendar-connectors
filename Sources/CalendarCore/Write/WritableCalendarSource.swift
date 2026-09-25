import Foundation

/// A source that can also write. Read-only connectors never adopt it; callers check `source as? WritableCalendarSource`.
/// What a conforming source can write is in `capabilities` (`writableFields`, `controlsNotifications`,
/// `recurrenceScopes`, `canEditAttendees`, `canRespondToInvite`); which calendars are writable is each
/// `CalendarDescriptor.accessRole`. An operation the source cannot perform throws `WriteError.unsupported` before
/// changing anything. Writes return the event in the provider's resulting form, including its new `version`.
public protocol WritableCalendarSource: CalendarSource {
    func create(_ draft: EventDraft, in calendarID: String, notify: NotifyPolicy) async throws -> CalendarEvent
    /// An empty patch writes nothing and returns the patch's base event, or the current event when it has none.
    func update(_ ref: EventRef, _ patch: EventPatch, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent
    func delete(_ ref: EventRef, scope: RecurrenceScope, notify: NotifyPolicy) async throws
    /// `response` must be `.accepted`, `.tentative` or `.declined`.
    func respond(to ref: EventRef, _ response: ResponseStatus, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent
}
