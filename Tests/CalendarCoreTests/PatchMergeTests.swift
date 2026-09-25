import Foundation
import Testing
@testable import CalendarCore

private let utc = TimeZone(identifier: "UTC")!
private func instant(_ s: String) -> Date { ISO8601DateFormatter().date(from: s)! }

private func base() -> CalendarEvent {
    CalendarEvent(eventID: "e", calendarID: "c", title: "Planning", notes: "n", location: "Room",
                  start: instant("2026-09-21T10:00:00Z"), end: instant("2026-09-21T11:00:00Z"), timeZone: utc,
                  attendees: [Attendee(email: "bob@x.com", role: .required)], version: "v1")
}
private func patch(_ mutate: (inout EventEdit) -> Void) -> EventPatch {
    var edit = EventEdit(base())
    mutate(&edit)
    return edit.patch
}

@Test func noConflictWhenOthersChangedDifferentFields() {
    var current = base()
    current.location = "Elsewhere"
    current.version = "v2"
    #expect(PatchMerge.conflicts(patch: patch { $0.event.title = "New" }, current: current).isEmpty)
}

@Test func aTouchedFieldThatChangedElsewhereConflicts() {
    var current = base()
    current.title = "Changed"
    #expect(PatchMerge.conflicts(patch: patch { $0.event.title = "New" }, current: current) == [.title])
}

@Test func timingIsComparedAsAUnit() {
    var current = base()
    current.end = instant("2026-09-21T11:30:00Z")   // someone moved only the end
    #expect(PatchMerge.conflicts(patch: patch { $0.event.start = instant("2026-09-21T09:30:00Z") }, current: current) == [.timing])
}

@Test func attendeesConflictOnlyForTheEmailsThePatchTouches() {
    var current = base()
    current.attendees.append(Attendee(email: "new@x.com"))          // unrelated addition
    let addDee = patch { $0.event.attendees.append(Attendee(email: "dee@x.com")) }
    #expect(PatchMerge.conflicts(patch: addDee, current: current).isEmpty)
    current.attendees[0].role = .optional                            // someone changed bob's role
    let changeBob = patch { $0.event.attendees[0].role = .resource }
    #expect(PatchMerge.conflicts(patch: changeBob, current: current) == [.attendees])
    let removeBob = patch { $0.event.attendees.removeAll() }
    #expect(PatchMerge.conflicts(patch: removeBob, current: current) == [.attendees])
}

@Test func aPatchWithoutABaseConflictsOnEveryTouchedField() {
    let handBuilt = EventPatch(title: "X", location: .set("Y"))
    #expect(PatchMerge.conflicts(patch: handBuilt, current: base()) == [.title, .location])
}

@Test func recurrenceCannotBeJudgedSoItConflicts() {
    var edit = EventEdit(base())
    edit.event.title = "New"
    var p = edit.patch
    p.recurrence = .clear
    #expect(PatchMerge.conflicts(patch: p, current: base()) == [.recurrence])
}

@Test func applyReturnsTheFirstSuccessfulWrite() async throws {
    let result = try await PatchMerge.apply(patch: EventPatch(title: "X"), version: "v1",
                                            fetchCurrent: { base() }, write: { _ -> Attempt in .done("ok") })
    #expect(result == "ok")
}

@Test func applyRetriesOnTheFreshVersionWhenNothingOverlaps() async throws {
    var current = base()
    current.location = "Elsewhere"
    current.version = "v2"
    let seen = Box<[String?]>([])
    let result = try await PatchMerge.apply(
        patch: patch { $0.event.title = "New" }, version: "v1", fetchCurrent: { current },
        write: { version -> Attempt in
            seen.value.append(version)
            return version == "v2" ? .done("saved") : .stale
        })
    #expect(result == "saved" && seen.value == ["v1", "v2"])
}

@Test func applyJudgesEverySecondStaleResultAgain() async {
    let fetched = Box(0)
    await expectWriteError(.conflict(fields: [.title])) {
        _ = try await PatchMerge.apply(
            patch: patch { $0.event.title = "New" }, version: "v1",
            fetchCurrent: {
                fetched.value += 1
                var current = base()
                current.version = "v\(fetched.value + 1)"
                if fetched.value == 2 { current.title = "Changed" }   // the second concurrent edit overlaps
                return current
            },
            write: { _ -> Attempt in .stale })
    }
    #expect(fetched.value == 2)
}

@Test func applyGivesUpAfterMaxAttempts() async {
    let writes = Box(0)
    await expectWriteError(.conflict(fields: [.title])) {
        _ = try await PatchMerge.apply(patch: patch { $0.event.title = "New" }, version: "v1", maxAttempts: 3,
                                       fetchCurrent: { base() }, write: { _ -> Attempt in writes.value += 1; return .stale })
    }
    #expect(writes.value == 3)
}

@Test func aBaselessPatchFailsAtTheFirstStaleVersion() async {
    let fetched = Box(0)
    await expectWriteError(.conflict(fields: [.title])) {
        _ = try await PatchMerge.apply(patch: EventPatch(title: "X"), version: "v1",
                                       fetchCurrent: { fetched.value += 1; return base() }, write: { _ -> Attempt in .stale })
    }
    #expect(fetched.value == 1)
}

@Test func aRetryWithoutAFreshVersionFailsClosedWhenTheOriginalWasVersioned() async {
    var current = base()
    current.location = "Elsewhere"   // no overlap with a title patch, so a retry would be due
    current.version = nil
    let writes = Box(0)
    await expectWriteError(.conflict(fields: [.title])) {
        _ = try await PatchMerge.apply(patch: patch { $0.event.title = "New" }, version: "v1", fetchCurrent: { current },
                                       write: { _ -> Attempt in writes.value += 1; return .stale })
    }
    #expect(writes.value == 1)   // never retried unconditionally
}

@Test func aNilOriginalVersionStillRetriesWithoutOne() async throws {
    var current = base()
    current.location = "Elsewhere"
    current.version = nil
    let seen = Box<[String?]>([])
    let result = try await PatchMerge.apply(
        patch: patch { $0.event.title = "New" }, version: nil, fetchCurrent: { current },
        write: { version -> Attempt in
            seen.value.append(version)
            return seen.value.count == 2 ? .done("saved") : .stale
        })
    #expect(result == "saved" && seen.value == [nil, nil])
}

// MARK: edge cases beyond the plan

@Test func aNonPositiveMaxAttemptsStillWritesOnce() async throws {
    for attempts in [0, -1, Int.min] {
        let writes = Box(0)
        let result = try await PatchMerge.apply(
            patch: EventPatch(title: "X"), version: "v1", maxAttempts: attempts,
            fetchCurrent: { base() }, write: { _ -> Attempt in writes.value += 1; return .done("ok") })
        #expect(result == "ok" && writes.value == 1)
    }
}

@Test func cancellationStopsTheRetryLoop() async {
    let writes = Box(0)
    let task = Task {
        try await PatchMerge.apply(
            patch: EventPatch(), version: "v1", maxAttempts: 10, fetchCurrent: { base() },
            write: { _ -> Attempt in
                writes.value += 1
                withUnsafeCurrentTask { $0?.cancel() }
                return .stale
            })
    }
    do {
        _ = try await task.value
        Issue.record("expected cancellation")
    } catch is CancellationError {
        #expect(writes.value == 1)
    } catch {
        Issue.record("expected CancellationError, got \(error)")
    }
}

@Test func attendeeAlreadyInTheWantedStateIsNotAConflict() {
    // Someone else added dee with the same role, or already removed bob: the patch's outcome already holds.
    var current = base()
    current.attendees.append(Attendee(email: "dee@x.com", role: .required))
    let addDee = patch { $0.event.attendees.append(Attendee(email: "dee@x.com")) }
    #expect(PatchMerge.conflicts(patch: addDee, current: current).isEmpty)
    current.attendees[1].role = .optional                            // ...but with a different role: real conflict
    #expect(PatchMerge.conflicts(patch: addDee, current: current) == [.attendees])

    var gone = base()
    gone.attendees.removeAll()
    let removeBob = patch { $0.event.attendees.removeAll() }
    #expect(PatchMerge.conflicts(patch: removeBob, current: gone).isEmpty)
}

@Test func emptyAndMissingNotesAndLocationAreTheSame() {
    var current = base()
    current.notes = nil
    current.location = ""
    var editedBase = base()
    editedBase.notes = ""
    editedBase.location = nil
    var edit = EventEdit(editedBase)
    edit.event.notes = "x"
    edit.event.location = "y"
    #expect(PatchMerge.conflicts(patch: edit.patch, current: current).isEmpty)
}

@Test func reminderOrderIsNotAConflict() {
    var baseWithReminders = base()
    baseWithReminders.reminders = [Reminder(minutesBefore: 10), Reminder(minutesBefore: 30)]
    var edit = EventEdit(baseWithReminders)
    edit.event.reminders = [Reminder(minutesBefore: 5)]
    var current = baseWithReminders
    current.reminders = [Reminder(minutesBefore: 30), Reminder(minutesBefore: 10)]
    #expect(PatchMerge.conflicts(patch: edit.patch, current: current).isEmpty)
    current.reminders = [Reminder(minutesBefore: 30)]
    #expect(PatchMerge.conflicts(patch: edit.patch, current: current) == [.reminders])
}

private typealias Attempt = PatchMerge.Attempt<String>

private final class Box<T>: @unchecked Sendable {
    var value: T
    init(_ value: T) { self.value = value }
}

@Test func attendeeEmailsAreComparedCaseInsensitively() {
    // The provider stores a mixed-case address; the patch carries the normalised (lowercase) one.
    var mixed = base()
    mixed.attendees = [Attendee(email: "Bob@X.com", role: .required)]
    var edit = EventEdit(mixed)
    edit.event.attendees.removeAll()
    let removeBob = edit.patch
    #expect(removeBob.attendees?.remove == ["bob@x.com"])
    var current = mixed
    current.attendees[0].role = .optional                            // someone changed bob's role meanwhile
    #expect(PatchMerge.conflicts(patch: removeBob, current: current) == [.attendees])

    var upsert = EventEdit(mixed)
    upsert.event.attendees[0].role = .resource
    #expect(PatchMerge.conflicts(patch: upsert.patch, current: current) == [.attendees])
}
