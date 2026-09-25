import Foundation

/// Field-level conflict handling shared by every connector that has a version to compare (Google etag, EventKit
/// modification date). A stale version is not an error by itself: it is a conflict only when someone changed a
/// field this patch touches.
public enum PatchMerge {
    public enum Attempt<Result> {
        case done(Result)
        /// The provider rejected the write because the version is out of date.
        case stale
    }

    /// The touched fields whose value in `current` differs from `patch.base`. A patch without a base cannot be
    /// compared, so every touched field is a conflict. Attendees are compared only for the emails the patch adds or
    /// removes (a concurrent change to someone else is not a conflict). Recurrence cannot be read back, so a patch
    /// that touches it always conflicts on a stale version.
    public static func conflicts(patch: EventPatch, current: CalendarEvent) -> Set<EventField> {
        guard let base = patch.base else { return patch.touchedFields }
        var found = Set<EventField>()
        for field in patch.touchedFields {
            switch field {
            case .title: if current.title != base.title { found.insert(field) }
            case .notes: if (current.notes ?? "") != (base.notes ?? "") { found.insert(field) }
            case .location: if (current.location ?? "") != (base.location ?? "") { found.insert(field) }
            case .timing:
                let same = current.start == base.start && current.end == base.end
                    && current.timeZone.identifier == base.timeZone.identifier && current.isAllDay == base.isAllDay
                if !same { found.insert(field) }
            case .availability: if current.availability != base.availability { found.insert(field) }
            case .visibility: if current.visibility != base.visibility { found.insert(field) }
            case .reminders: if minutes(current.reminders) != minutes(base.reminders) { found.insert(field) }
            case .attendees: if attendeesDiffer(patch.attendees, base: base, current: current) { found.insert(field) }
            case .recurrence: found.insert(field)
            case .conference: if structuredURLs(current) != structuredURLs(base) { found.insert(field) }
            }
        }
        return found
    }

    /// The provider's own conference links; links found in the notes change whenever the notes do, which is not a conference change.
    private static func structuredURLs(_ event: CalendarEvent) -> [URL] { event.conferences.filter { $0.origin == .structured }.map(\.url) }

    /// Providers do not promise to keep reminder order, so compare them as a sorted list.
    private static func minutes(_ reminders: [Reminder]) -> [Int] { reminders.map(\.minutesBefore).sorted() }

    /// An email conflicts when its role changed between `base` and `current` and `current` does not already hold
    /// what the patch wants (an added attendee with the requested role, or one that is already gone).
    private static func attendeesDiffer(_ changes: AttendeeChanges?, base: CalendarEvent, current: CalendarEvent) -> Bool {
        guard let changes else { return false }
        func role(_ email: String, in event: CalendarEvent) -> AttendeeRole? {
            event.attendees.first { $0.email == email }?.role
        }
        func changed(_ email: String, wanted: AttendeeRole?) -> Bool {
            let now = role(email, in: current)
            return now != role(email, in: base) && now != wanted
        }
        return changes.remove.contains { changed($0, wanted: nil) } || changes.add.contains { changed($0.email, wanted: $0.role) }
    }

    /// Runs `write` with `version`. When it reports `.stale`, fetches the current event and judges the patch against
    /// it: a real overlap throws `.conflict(fields:)`; otherwise the write is retried on the fresh version, and a
    /// further stale result is judged the same way. After `maxAttempts` writes it throws
    /// `.conflict(fields: patch.touchedFields)` (the event is being edited concurrently). It fails closed the same way
    /// when a retry is due but the fresh event carries no version although the write was versioned: retrying without
    /// one would be an unconditional write that could overwrite an edit made after the fetch. A write that started
    /// without a version has nothing to lose and retries as before.
    public static func apply<Result>(
        patch: EventPatch, version: String?, maxAttempts: Int = 3,
        fetchCurrent: () async throws -> CalendarEvent,
        write: (String?) async throws -> Attempt<Result>
    ) async throws -> Result {
        var version = version
        // At least one write, whatever the caller passed (a negative count would trap the range).
        for _ in 0..<max(1, maxAttempts) {
            try Task.checkCancellation()
            switch try await write(version) {
            case .done(let result):
                return result
            case .stale:
                let current = try await fetchCurrent()
                let overlapping = conflicts(patch: patch, current: current)
                if !overlapping.isEmpty { throw WriteError.conflict(fields: overlapping) }
                if version != nil && current.version == nil { throw WriteError.conflict(fields: patch.touchedFields) }
                version = current.version
            }
        }
        throw WriteError.conflict(fields: patch.touchedFields)
    }
}
