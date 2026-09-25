import CalendarTestSupport
import Foundation
import Testing
@testable import CalendarCore

private let utc = TimeZone(identifier: "UTC")!
private let window = DateInterval(start: Date(timeIntervalSince1970: 1_790_000_000), end: Date(timeIntervalSince1970: 1_790_000_000 + 86_400 * 3))

private func draft(_ title: String = "Sync") -> EventDraft {
    let start = window.start.addingTimeInterval(3600)
    return EventDraft(title: title, timing: EventTiming(start: start, end: start.addingTimeInterval(1800), timeZone: utc, isAllDay: false))
}

@Test func theFakePassesTheConformanceChecks() async {
    let source = FakeWritableSource()
    #expect(await WritableSourceConformance.violations(of: source, calendarID: "cal", window: window).isEmpty)
}

@Test func aFakeWithoutAttendeeSupportStillConforms() async {
    let source = FakeWritableSource(writableFields: [.title, .notes, .location, .timing], canRespond: false)
    #expect(!source.capabilities.canEditAttendees)
    #expect(await WritableSourceConformance.violations(of: source, calendarID: "cal", window: window).isEmpty)
}

@Test func aSourceThatCannotWriteLocationStillConforms() async {
    let source = FakeWritableSource(writableFields: [.title, .timing])
    #expect(await WritableSourceConformance.violations(of: source, calendarID: "cal", window: window).isEmpty)
}

@Test func createReadsBackAndStampsTheSource() async throws {
    let source = FakeWritableSource()
    let created = try await source.create(draft(), in: "cal", notify: .none)
    #expect(created.title == "Sync" && created.calendarID == "cal" && created.sourceID == source.id && created.version != nil)
    #expect(try await source.events(in: window).map(\.eventID) == [created.eventID])
}

@Test func anUnknownCalendarIsNotFoundAndUnwritableFieldsAreRefused() async {
    let source = FakeWritableSource(writableFields: [.title, .timing])
    await expectWriteError(.notFound) { _ = try await source.create(draft(), in: "nope", notify: .none) }
    var withNotes = draft(); withNotes.notes = "n"
    await expectWriteError(.unsupported(fields: [.notes])) { _ = try await source.create(withNotes, in: "cal", notify: .none) }
    #expect(source.writeCount == 0)
}

@Test func anEmptyPatchWritesNothing() async throws {
    let source = FakeWritableSource()
    let created = try await source.create(draft(), in: "cal", notify: .none)
    let before = source.writeCount
    let same = try await source.update(EventRef(created), EventPatch(), scope: .thisInstance, notify: .none)
    #expect(same == created && source.writeCount == before)
}

@Test func anEmptyPatchWithABaseReturnsTheBaseAndWritesNothing() async throws {
    let source = FakeWritableSource()
    let created = try await source.create(draft(), in: "cal", notify: .none)
    #expect(source.simulateExternalEdit(calendarID: "cal", eventID: created.eventID) { $0.location = "Elsewhere" })
    let before = source.writeCount
    let empty = EventEdit(created).patch   // nothing edited, but it carries `created` as its base
    #expect(empty.isEmpty && empty.base == created)
    let result = try await source.update(EventRef(created), empty, scope: .thisInstance, notify: .none)
    #expect(result == created && source.writeCount == before)
}

@Test func simulateExternalEditReportsWhetherTheEventExists() async throws {
    let source = FakeWritableSource()
    let created = try await source.create(draft(), in: "cal", notify: .none)
    #expect(source.simulateExternalEdit(calendarID: "cal", eventID: created.eventID) { $0.title = "Theirs" })
    #expect(!source.simulateExternalEdit(calendarID: "cal", eventID: "ghost") { $0.title = "Theirs" })
}

@Test func theFakeDoesNotModelRecurrenceSoItRefusesIt() async throws {
    let source = FakeWritableSource()
    #expect(!source.capabilities.writableFields.contains(.recurrence))
    var recurring = draft()
    recurring.recurrence = RecurrenceRule(frequency: .daily)
    await expectWriteError(.unsupported(fields: [.recurrence])) { _ = try await source.create(recurring, in: "cal", notify: .none) }
    let created = try await source.create(draft(), in: "cal", notify: .none)
    await expectWriteError(.unsupported(fields: [.recurrence])) {
        _ = try await source.update(EventRef(created), EventPatch(recurrence: .set(RecurrenceRule(frequency: .daily))), scope: .thisInstance, notify: .none)
    }
    #expect(source.writeCount == 1)   // only the plain create
}

@Test func aStaleVersionMergesWhenFieldsDoNotOverlap() async throws {
    let source = FakeWritableSource()
    let created = try await source.create(draft(), in: "cal", notify: .none)
    source.simulateExternalEdit(calendarID: "cal", eventID: created.eventID) { $0.location = "Elsewhere" }
    var edit = EventEdit(created)
    edit.event.title = "Renamed"
    let updated = try await source.update(EventRef(created), edit.patch, scope: .thisInstance, notify: .none)
    #expect(updated.title == "Renamed" && updated.location == "Elsewhere")
}

@Test func aStaleVersionConflictsWhenTheSameFieldChanged() async throws {
    let source = FakeWritableSource()
    let created = try await source.create(draft(), in: "cal", notify: .none)
    source.simulateExternalEdit(calendarID: "cal", eventID: created.eventID) { $0.title = "Theirs" }
    var edit = EventEdit(created)
    edit.event.title = "Mine"
    await expectWriteError(.conflict(fields: [.title])) {
        _ = try await source.update(EventRef(created), edit.patch, scope: .thisInstance, notify: .none)
    }
}

@Test func respondIsUnsupportedWhenTheSourceCannotRSVP() async throws {
    let source = FakeWritableSource(canRespond: false)
    let created = try await source.create(draft(), in: "cal", notify: .none)
    await expectWriteError(.unsupported(fields: [.attendees])) {
        _ = try await source.respond(to: EventRef(created), .accepted, scope: .thisInstance, notify: .none)
    }
}

@Test func capabilitiesFollowTheInvariants() {
    let full = FakeWritableSource().capabilities
    #expect(full.canWrite && full.canEditAttendees == full.writableFields.contains(.attendees))
    let limited = FakeWritableSource(writableFields: [.title, .timing]).capabilities
    #expect(!limited.canEditAttendees && limited.writableFields == [.title, .timing])
}

// MARK: - Fake behaviour beyond the brief

@Test func respondUpdatesTheSelfAttendeeAndRefusesNeedsAction() async throws {
    let source = FakeWritableSource()
    let created = try await source.create(draft(), in: "cal", notify: .none)
    await expectWriteError(.invalid("you are not an attendee of this event")) {
        _ = try await source.respond(to: EventRef(created), .accepted, scope: .thisInstance, notify: .none)
    }
    source.simulateExternalEdit(calendarID: "cal", eventID: created.eventID) {
        $0.attendees = [Attendee(email: "me@example.com", isSelf: true)]
    }
    await expectWriteError(.invalid("cannot respond with needsAction")) {
        _ = try await source.respond(to: EventRef(created), .needsAction, scope: .thisInstance, notify: .none)
    }
    let answered = try await source.respond(to: EventRef(created), .declined, scope: .thisInstance, notify: .none)
    #expect(answered.myResponse == .declined && answered.attendees.first?.response == .declined)
}

@Test func updateAndDeleteOfAMissingEventAreNotFound() async {
    let source = FakeWritableSource()
    let ref = EventRef(calendarID: "cal", eventID: "ghost")
    await expectWriteError(.notFound) { _ = try await source.update(ref, EventPatch(title: "x"), scope: .thisInstance, notify: .none) }
    await expectWriteError(.notFound) { try await source.delete(ref, scope: .thisInstance, notify: .none) }
    #expect(source.writeCount == 0)
}

@Test func updateRefusesUnwritableFieldsAndBadInputBeforeWriting() async throws {
    let source = FakeWritableSource(writableFields: [.title, .timing, .attendees, .reminders])
    let created = try await source.create(draft(), in: "cal", notify: .none)
    let before = source.writeCount
    await expectWriteError(.unsupported(fields: [.notes])) {
        _ = try await source.update(EventRef(created), EventPatch(notes: .set("n")), scope: .thisInstance, notify: .none)
    }
    await expectWriteError(.invalid("attendee email is not valid: nobody")) {
        _ = try await source.update(EventRef(created), EventPatch(attendees: AttendeeChanges(add: [AttendeeDraft(email: "nobody")])),
                                    scope: .thisInstance, notify: .none)
    }
    await expectWriteError(.invalid("reminder minutes must not be negative")) {
        _ = try await source.update(EventRef(created), EventPatch(reminders: .set([Reminder(minutesBefore: -5)])),
                                    scope: .thisInstance, notify: .none)
    }
    let backwards = EventTiming(start: created.end, end: created.start, timeZone: utc, isAllDay: false)
    await expectWriteError(.invalid("end must be after start")) {
        _ = try await source.update(EventRef(created), EventPatch(timing: backwards), scope: .thisInstance, notify: .none)
    }
    #expect(source.writeCount == before)
}

@Test func createRefusesAnInvalidDraft() async {
    let source = FakeWritableSource()
    var bad = draft()
    bad.timing.end = bad.timing.start
    await expectWriteError(.invalid("end must be after start")) { _ = try await source.create(bad, in: "cal", notify: .none) }
    #expect(source.writeCount == 0)
}

// MARK: - The conformance checks catch broken sources

/// Forwards to a fake and lets a test break one behaviour.
private final class BrokenSource: WritableCalendarSource, @unchecked Sendable {
    enum Flaw {
        case ignoresTitleChange, throwsOnUpdate, deleteKeepsEvent, silentlyAcceptsAttendees, canWriteFalse
        case emptyPatchBumpsVersion, titleUpdateShiftsEnd, titleUpdateFlipsAllDay, deleteAlwaysFails
    }
    let inner: FakeWritableSource
    let flaw: Flaw
    init(_ flaw: Flaw, writableFields: Set<EventField> = [.title, .location, .timing]) {
        self.flaw = flaw
        inner = FakeWritableSource(writableFields: writableFields)
    }
    var id: String { inner.id }
    var displayName: String { inner.displayName }
    var capabilities: SourceCapabilities {
        var caps = inner.capabilities
        if flaw == .canWriteFalse { caps.canWrite = false }
        return caps
    }
    func calendars() async throws -> [CalendarDescriptor] { try await inner.calendars() }
    func events(in interval: DateInterval) async throws -> [CalendarEvent] { try await inner.events(in: interval) }
    func changes() -> AsyncStream<CalendarChange> { inner.changes() }
    func create(_ draft: EventDraft, in calendarID: String, notify: NotifyPolicy) async throws -> CalendarEvent {
        try await inner.create(draft, in: calendarID, notify: notify)
    }
    func update(_ ref: EventRef, _ patch: EventPatch, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent {
        switch flaw {
        case .ignoresTitleChange: return try await inner.update(ref, EventPatch(), scope: scope, notify: notify)
        case .throwsOnUpdate: throw WriteError.forbidden("no")
        case .silentlyAcceptsAttendees where patch.attendees != nil:
            return try await inner.update(ref, EventPatch(), scope: scope, notify: notify)
        case .emptyPatchBumpsVersion where patch.isEmpty:
            var event = try await inner.update(ref, patch, scope: scope, notify: notify)
            event.version = "bumped-by-an-empty-patch"
            return event
        case .titleUpdateShiftsEnd where patch.title != nil:
            var event = try await inner.update(ref, patch, scope: scope, notify: notify)
            event.end = event.end.addingTimeInterval(60)
            return event
        case .titleUpdateFlipsAllDay where patch.title != nil:
            var event = try await inner.update(ref, patch, scope: scope, notify: notify)
            event.isAllDay = true
            return event
        default: return try await inner.update(ref, patch, scope: scope, notify: notify)
        }
    }
    func delete(_ ref: EventRef, scope: RecurrenceScope, notify: NotifyPolicy) async throws {
        if flaw == .deleteKeepsEvent { return }
        if flaw == .deleteAlwaysFails { throw WriteError.forbidden("no delete") }
        try await inner.delete(ref, scope: scope, notify: notify)
    }
    func respond(to ref: EventRef, _ response: ResponseStatus, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent {
        try await inner.respond(to: ref, response, scope: scope, notify: notify)
    }
}

@Test(arguments: [BrokenSource.Flaw.ignoresTitleChange, .throwsOnUpdate, .deleteKeepsEvent, .silentlyAcceptsAttendees, .canWriteFalse,
            .emptyPatchBumpsVersion, .titleUpdateShiftsEnd, .titleUpdateFlipsAllDay, .deleteAlwaysFails])
private func theConformanceChecksReportBrokenSources(flaw: BrokenSource.Flaw) async {
    let source = BrokenSource(flaw)
    let found = await WritableSourceConformance.violations(of: source, calendarID: "cal", window: window)
    #expect(!found.isEmpty, "expected a violation for \(flaw)")
}

@Test func theConformanceChecksNameTheFlawTheyFound() async {
    func found(_ flaw: BrokenSource.Flaw) async -> [String] {
        await WritableSourceConformance.violations(of: BrokenSource(flaw), calendarID: "cal", window: window)
    }
    #expect(await found(.emptyPatchBumpsVersion).contains("an empty patch changed the version"))
    #expect(await found(.titleUpdateShiftsEnd).contains("update changed a field the patch did not touch (timing)"))
    #expect(await found(.titleUpdateFlipsAllDay).contains("update changed a field the patch did not touch (timing)"))
}

@Test func theConformanceChecksReportACleanupThatFails() async {
    let found = await WritableSourceConformance.violations(of: BrokenSource(.deleteAlwaysFails), calendarID: "cal", window: window)
    #expect(found.contains { $0.hasPrefix("cleanup delete failed") }, "\(found)")
}

@Test func theConformanceChecksLeaveNothingBehindWhenAStepFails() async throws {
    let source = BrokenSource(.throwsOnUpdate)
    _ = await WritableSourceConformance.violations(of: source, calendarID: "cal", window: window)
    #expect(try await source.events(in: window).isEmpty)
}

@Test func theConformanceChecksRefuseAnUnwritableCalendarWithoutCrashing() async {
    let found = await WritableSourceConformance.violations(of: FakeWritableSource(), calendarID: "nope", window: window)
    #expect(found.count == 1 && found[0].hasPrefix("create threw"))
}

@Test func theFakeSourceRefusesAduplicateUIDOnTheSameCalendar() async throws {
    let source = FakeWritableSource(calendarIDs: ["a", "b"])
    let timing = EventTiming(start: Date(timeIntervalSince1970: 1000), end: Date(timeIntervalSince1970: 2000), timeZone: nil, isAllDay: false)
    let first = try await source.create(EventDraft(title: "T", timing: timing, uid: "u-1"), in: "a", notify: .none)
    do {
        _ = try await source.create(EventDraft(title: "T", timing: timing, uid: "u-1"), in: "a", notify: .none)
        Issue.record("expected alreadyExists")
    } catch WriteError.alreadyExists(let existing) { #expect(existing.eventID == first.eventID) }
    _ = try await source.create(EventDraft(title: "T", timing: timing, uid: "u-1"), in: "b", notify: .none)   // another calendar
}
