import Foundation
import Testing
@testable import CalendarCore

private let utc = TimeZone(identifier: "UTC")!
private func instant(_ s: String) -> Date { ISO8601DateFormatter().date(from: s)! }

private func original() -> CalendarEvent {
    CalendarEvent(
        eventID: "e", uid: "u", calendarID: "c", title: "Planning", notes: "n", location: "Room",
        start: instant("2026-09-21T10:00:00Z"), end: instant("2026-09-21T11:00:00Z"), timeZone: utc,
        status: .confirmed, attendees: [Attendee(email: "me@x.com", isSelf: true), Attendee(email: "bob@x.com"),
                                        Attendee(email: "cy@x.com", role: .optional)],
        organizer: Attendee(email: "me@x.com", isSelf: true, isOrganizer: true),
        conferences: [ConferenceInfo(url: URL(string: "https://meet.google.com/abc")!, provider: .meet)],
        reminders: [Reminder(minutesBefore: 10)], url: URL(string: "https://x.test/e")!, version: "v1", myResponse: .accepted)
}

@Test func anUntouchedEditProducesAnEmptyPatch() {
    let edit = EventEdit(original())
    #expect(!edit.hasChanges && edit.patch.isEmpty && edit.patch.touchedFields.isEmpty)
}

@Test func diffCapturesScalarChangesAndClears() {
    var edit = EventEdit(original())
    edit.event.title = "Planning v2"
    edit.event.notes = nil
    edit.event.location = "Lab"
    edit.event.availability = .free
    edit.event.visibility = .privateEvent
    let patch = edit.patch
    #expect(patch.title == "Planning v2")
    #expect(patch.notes == .clear && patch.location == .set("Lab"))
    #expect(patch.availability == .free && patch.visibility == .privateEvent)
    #expect(patch.touchedFields == [.title, .notes, .location, .availability, .visibility])
    #expect(patch.base == original())
}

@Test func diffTreatsTimeAsOneUnit() {
    var edit = EventEdit(original())
    edit.event.end = instant("2026-09-21T11:30:00Z")
    let patch = edit.patch
    #expect(patch.timing == EventTiming(start: instant("2026-09-21T10:00:00Z"), end: instant("2026-09-21T11:30:00Z"), timeZone: utc, isAllDay: false))
    #expect(patch.touchedFields == [.timing])
}

@Test func diffBuildsAnAttendeeDeltaByEmail() throws {
    var edit = EventEdit(original())
    edit.event.attendees.removeAll { $0.email == "bob@x.com" }                    // removed
    edit.event.attendees.append(Attendee(name: "Dee", email: "Dee@x.com"))        // added
    if let i = edit.event.attendees.firstIndex(where: { $0.email == "cy@x.com" }) { edit.event.attendees[i].role = .required }   // role changed: upsert
    let changes = try #require(edit.patch.attendees)
    #expect(changes.remove == ["bob@x.com"])
    // Upserts follow the edited list's order (cy was already there, dee was appended).
    #expect(changes.add == [AttendeeDraft(email: "cy@x.com", role: .required), AttendeeDraft(email: "dee@x.com", name: "Dee")])
    #expect(edit.patch.touchedFields == [.attendees])
}

@Test func diffIgnoresProviderOwnedFieldsAndUnwritableConferenceChanges() {
    var edit = EventEdit(original())
    edit.event.status = .tentative
    edit.event.myResponse = .declined
    edit.event.version = "v9"
    edit.event.url = nil
    edit.event.organizer = nil
    edit.event.sourceID = "other"
    edit.event.conferences = [ConferenceInfo(url: URL(string: "https://zoom.us/j/1")!, provider: .zoom)]   // changed, not removable
    #expect(edit.patch.isEmpty)
    edit.event.conferences = []
    #expect(edit.patch.conference == .remove && edit.patch.touchedFields == [.conference])
}

@Test func diffSetsRemindersExplicitly() {
    var edit = EventEdit(original())
    edit.event.reminders = []
    #expect(edit.patch.reminders == .set([]))
    edit.event.reminders = [Reminder(minutesBefore: 5), Reminder(minutesBefore: 30)]
    #expect(edit.patch.reminders == .set([Reminder(minutesBefore: 5), Reminder(minutesBefore: 30)]))
}

@Test func attendeesWithoutAnEmailAreIgnoredByTheDiff() {
    var edit = EventEdit(original())
    edit.event.attendees.append(Attendee(name: "Ghost"))
    #expect(edit.patch.isEmpty)
}

@Test func aHandBuiltPatchHasNoBaseAndReportsTouchedFields() {
    let patch = EventPatch(title: "X", recurrence: .clear, conference: .generate)
    #expect(patch.base == nil && patch.touchedFields == [.title, .recurrence, .conference])
    #expect(EventPatch(attendees: AttendeeChanges(add: [], remove: [])).isEmpty)   // an empty delta touches nothing
    #expect(EventEdit(original()).patch.withoutBase() == EventPatch())
}

@Test func appliedToRewritesEveryRepresentableField() {
    let patch = EventPatch(
        title: "New", notes: .clear, location: .set("Lab"),
        timing: EventTiming(start: instant("2026-09-22T10:00:00Z"), end: instant("2026-09-22T11:00:00Z"), timeZone: utc, isAllDay: false),
        availability: .free, visibility: .confidential, reminders: .set([]),
        attendees: AttendeeChanges(add: [AttendeeDraft(email: "dee@x.com", name: "Dee"), AttendeeDraft(email: "cy@x.com", role: .required)], remove: ["bob@x.com"]),
        conference: .remove)
    let e = patch.applied(to: original())
    #expect(e.title == "New" && e.notes == nil && e.location == "Lab")
    #expect(e.start == instant("2026-09-22T10:00:00Z") && e.availability == .free && e.visibility == .confidential)
    #expect(e.reminders.isEmpty && e.conference == nil)
    #expect(e.attendees.map(\.email) == ["me@x.com", "cy@x.com", "dee@x.com"])
    #expect(e.attendees.first { $0.email == "cy@x.com" }?.role == .required)
}

@Test func duplicateEmailsInProviderDataDoNotTrapAndTheFirstWins() {
    var event = original()
    event.attendees.append(Attendee(name: "Bob again", email: "BOB@x.com", role: .optional))
    var edit = EventEdit(event)
    #expect(edit.patch.isEmpty)
    edit.event.attendees.removeAll { $0.email == "bob@x.com" }
    #expect(edit.patch.attendees?.remove == ["bob@x.com"])
}

@Test func clearingAnAttendeeNameAloneIsNotAChange() {
    // A draft cannot clear a name, so emitting an upsert would touch attendees (and may notify) and change nothing.
    var event = original()
    event.attendees[1].name = "Bob"
    var edit = EventEdit(event)
    edit.event.attendees[1].name = nil
    #expect(edit.patch.isEmpty)
    edit.event.attendees[1].role = .optional
    #expect(edit.patch.attendees?.add == [AttendeeDraft(email: "bob@x.com", role: .optional)])
}

@Test func aNameOnlyChangeToAValueIsAnUpsert() {
    var edit = EventEdit(original())
    edit.event.attendees[1].name = "Bob"          // nil -> "Bob"
    #expect(edit.patch.attendees?.add == [AttendeeDraft(email: "bob@x.com", name: "Bob")])
    #expect(edit.patch.attendees?.remove.isEmpty == true && edit.patch.touchedFields == [.attendees])
    var renamed = original()
    renamed.attendees[1].name = "Bob"
    var second = EventEdit(renamed)
    second.event.attendees[1].name = "Robert"     // "Bob" -> "Robert"
    #expect(second.patch.attendees?.add == [AttendeeDraft(email: "bob@x.com", name: "Robert")])
    #expect(second.patch.touchedFields == [.attendees])
}

@Test func rebuildingTheSelfAttendeeWithoutTheSelfFlagIsNotAnAddition() {
    var edit = EventEdit(original())
    edit.event.attendees[0] = Attendee(name: "Me", email: "ME@x.com")
    #expect(edit.patch.isEmpty)
}

@Test func applyingADiffReproducesTheEditedWritableFields() {
    var edit = EventEdit(original())
    edit.event.title = "T"
    edit.event.attendees.removeAll { $0.email == "bob@x.com" }
    edit.event.attendees.append(Attendee(name: "Dee", email: "dee@x.com"))
    edit.event.attendees[1].role = .required
    edit.event.reminders = []
    edit.event.conferences = []
    let result = edit.patch.applied(to: original())
    #expect(result.title == "T" && result.reminders.isEmpty && result.conference == nil)
    #expect(Set(result.attendees.map(\.email)) == Set(edit.event.attendees.map(\.email)))
    #expect(result.attendees.first { $0.email == "cy@x.com" }?.role == .required)
}

@Test func applyingAPatchMatchesAttendeeEmailsCaseInsensitively() {
    var event = original()
    event.attendees = [Attendee(email: "Bob@X.com", role: .required), Attendee(email: "Cy@X.com")]
    // `Attendee` lowercases its address on creation, so a provider's mixed-case address is already normalised:
    // an upsert updates that attendee instead of appending a duplicate.
    let upserted = EventPatch(attendees: AttendeeChanges(add: [AttendeeDraft(email: "bob@x.com", role: .optional)])).applied(to: event)
    #expect(upserted.attendees.count == 2 && upserted.attendees[0].role == .optional)
    // A removal by normalised address removes the mixed-case attendee.
    let removed = EventPatch(attendees: AttendeeChanges(remove: ["CY@x.com"])).applied(to: event)
    #expect(removed.attendees.map(\.email) == ["bob@x.com"])
}
