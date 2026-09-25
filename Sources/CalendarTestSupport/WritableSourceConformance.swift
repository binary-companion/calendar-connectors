import CalendarCore
import Foundation

/// Behaviour every `WritableCalendarSource` must have. Run it against a source and a scratch calendar (the fake
/// here, the real EventKit source in the live tests). An empty result means the source conforms. It cleans up the
/// event it creates, even when a check fails part way.
public enum WritableSourceConformance {
    public static func violations(of source: any WritableCalendarSource, calendarID: String, window: DateInterval) async -> [String] {
        var found: [String] = []
        let caps = source.capabilities
        if !caps.canWrite { found.append("canWrite is false on a WritableCalendarSource") }
        if caps.canEditAttendees != caps.writableFields.contains(.attendees) {
            found.append("canEditAttendees does not match writableFields.contains(.attendees)")
        }
        // Only ask for what the source says it can write, so a source with fewer fields is not failed for create.
        let usesLocation = caps.writableFields.contains(.location)
        let utc = TimeZone(identifier: "UTC")!
        let start = window.start.addingTimeInterval(3600)
        let draft = EventDraft(
            title: "Conformance A", timing: EventTiming(start: start, end: start.addingTimeInterval(1800), timeZone: utc, isAllDay: false),
            location: usesLocation ? "Room 1" : nil)
        let created: CalendarEvent
        do {
            created = try await source.create(draft, in: calendarID, notify: .none)
        } catch {
            return found + ["create threw \(error)"]
        }
        // Where the event is now, so a failure part way still deletes it.
        var latest = created
        var deleted = false
        if created.title != "Conformance A" || created.calendarID != calendarID { found.append("create returned a different title or calendar") }

        do {
            let listed = try await source.events(in: window)
            if !listed.contains(where: { $0.eventID == created.eventID }) {
                found.append("the created event is not returned by events(in:)")
            }
            let renamed = try await source.update(EventRef(latest), EventPatch(title: "Conformance B"), scope: .thisInstance, notify: .none)
            latest = renamed
            if renamed.title != "Conformance B" { found.append("update did not change the title") }
            if usesLocation && renamed.location != "Room 1" { found.append("update changed a field the patch did not touch (location)") }
            if renamed.start != created.start || renamed.end != created.end || renamed.isAllDay != created.isAllDay {
                found.append("update changed a field the patch did not touch (timing)")
            }
            if renamed.version != nil, renamed.version == created.version { found.append("update did not change the version") }

            let unchanged = try await source.update(EventRef(renamed), EventPatch(), scope: .thisInstance, notify: .none)
            // The ref for the later delete follows whatever the source last returned, so a version-checking source
            // does not see a stale ref.
            latest = unchanged
            if unchanged.title != "Conformance B" { found.append("an empty patch changed the event") }
            if renamed.version != nil, unchanged.version != renamed.version { found.append("an empty patch changed the version") }

            if !caps.writableFields.contains(.attendees) {
                do {
                    _ = try await source.update(EventRef(renamed), EventPatch(attendees: AttendeeChanges(add: [AttendeeDraft(email: "a@b.c")])),
                                                scope: .thisInstance, notify: .none)
                    found.append("an unwritable field (attendees) was accepted")
                } catch WriteError.unsupported {
                } catch {
                    found.append("an unwritable field threw \(error) instead of .unsupported")
                }
            }

            try await source.delete(EventRef(latest), scope: .thisInstance, notify: .none)
            deleted = true
            let remaining = try await source.events(in: window)
            if remaining.contains(where: { $0.eventID == created.eventID }) {
                found.append("the event is still present after delete")
            }
            do {
                try await source.delete(EventRef(latest), scope: .thisInstance, notify: .none)
                found.append("deleting a missing event did not throw")
            } catch WriteError.notFound {
            } catch {
                found.append("deleting a missing event threw \(error) instead of .notFound")
            }
        } catch {
            found.append("threw \(error)")
        }
        if !deleted {
            do { try await source.delete(EventRef(latest), scope: .thisInstance, notify: .none) }
            catch { found.append("cleanup delete failed, the test event may be left behind: \(error)") }
        }
        return found
    }
}
