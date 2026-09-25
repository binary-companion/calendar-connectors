import CalendarTestSupport
import Foundation
import Testing
@testable import CalendarCore

@Test func eventIDIsUniqueAcrossCalendars() {
    let start = Date(timeIntervalSince1970: 0)
    let a = CalendarEvent(eventID: "e1", calendarID: "work", title: "T", start: start, end: start.addingTimeInterval(60))
    let b = CalendarEvent(eventID: "e1", calendarID: "home", title: "T", start: start, end: start.addingTimeInterval(60))
    #expect(a.id == "work/e1")
    #expect(a.id != b.id)
}

@Test func descriptorNormalizesColor() {
    func hex(_ raw: String?) -> String? {
        CalendarDescriptor(id: "c", title: "C", service: .google, colorHex: raw).colorHex
    }
    #expect(hex("#9fe1e7") == "#9FE1E7")
    #expect(hex("9fe1e7") == "#9FE1E7")
    #expect(hex("#abc") == "#AABBCC")
    #expect(hex("nope") == nil)
    #expect(hex(nil) == nil)
}

@Test func attendeeNormalizesEmail() {
    #expect(Attendee(email: "  Ann@Example.COM ").email == "ann@example.com")
    #expect(Attendee(email: "   ").email == nil)
}

@Test func capabilitiesDefaultToReadOnlyAndNoSync() {
    let c = SourceCapabilities()
    #expect(!c.canWrite && !c.canEditAttendees && !c.canRespondToInvite && c.providedFields.isEmpty && !c.supportsPush)
    #expect(c.syncKind == .none)
}

@Test func descriptorsAreStandardCalendarsUnlessToldOtherwise() {
    #expect(CalendarDescriptor(id: "c", title: "C", service: .google).kind == .standard)
    #expect(CalendarDescriptor(id: "c", title: "C", service: .google, kind: .birthdays).kind == .birthdays)
    #expect(CalendarDescriptor(id: "c", title: "C", service: .google, kind: .subscribed).kind == .subscribed)
}

// Provided fields.

@Test func aDeclaredFieldThatIsNilIsReportedAndAnUndeclaredNilIsFine() {
    var event = CalendarEvent(eventID: "e", calendarID: "c", title: "T", start: Date(timeIntervalSince1970: 0), end: Date(timeIntervalSince1970: 60))
    let capabilities = SourceCapabilities(providedFields: [.kind, .reminders])
    #expect(ProvidedFieldsConformance.violations(event: event, capabilities: capabilities) == ["declared field kind is nil", "declared field reminders is nil"])
    #expect(ProvidedFieldsConformance.violations(event: event, capabilities: SourceCapabilities()).isEmpty)
    event.kind = .standard
    event.reminders = []
    #expect(ProvidedFieldsConformance.violations(event: event, capabilities: capabilities).isEmpty)
}

@Test func seriesAndParticipationAccessorsSeparateUnknownFromNone() {
    var event = CalendarEvent(eventID: "e", calendarID: "c", title: "T", start: Date(timeIntervalSince1970: 0), end: Date(timeIntervalSince1970: 60))
    #expect(event.series == nil && event.seriesID == nil && event.participation == nil && event.myResponse == nil)
    event.series = .notRecurring
    event.participation = .notInvited
    #expect(event.seriesID == nil && event.myResponse == nil && event.series == .notRecurring)
    event.series = .occurrence(seriesID: "s", originalStart: Date(timeIntervalSince1970: 30))
    event.participation = .invited(.tentative)
    #expect(event.seriesID == "s" && event.originalStart == Date(timeIntervalSince1970: 30) && event.myResponse == .tentative)
}

// Availability, permissions and calendar identity (Issues 6 and 7).

@Test func closestAvailabilityFollowsTheFallbackLists() {
    let all: Set<Availability> = [.busy, .free, .tentative, .unavailable]
    for value in all { #expect(value.closest(in: all) == value) }
    #expect(Availability.tentative.closest(in: [.busy, .free]) == .busy)
    #expect(Availability.unavailable.closest(in: [.busy, .free]) == .busy)
    #expect(Availability.tentative.closest(in: [.free]) == .free)
    #expect(Availability.busy.closest(in: [.free]) == .free)
    #expect(Availability.free.closest(in: [.busy]) == .busy)
    #expect(Availability.busy.closest(in: [.tentative]) == nil)   // no fallback in the list
    #expect(Availability.busy.closest(in: []) == nil)
}

@Test func permissionsSummarizeIntoAnAccessRole() {
    #expect(CalendarPermissions(canViewDetails: false).accessRole == .freeBusyReader)
    #expect(CalendarPermissions(canViewDetails: true, canEdit: false, canShare: true).accessRole == .reader)
    #expect(CalendarPermissions(canViewDetails: true, canEdit: true, canShare: true).accessRole == .owner)
    #expect(CalendarPermissions(canViewDetails: true, canEdit: true, canShare: false).accessRole == .writer)
    #expect(CalendarPermissions(canViewDetails: true, canEdit: true, canShare: nil).accessRole == nil)   // EventKit, writable
}

@Test func serviceAndProviderAreStringBackedAndOpen() {
    #expect(CalendarService.eventKit.rawValue == "eventkit" && CalendarService(rawValue: "eventkit") == .eventKit)
    #expect(CalendarProvider(rawValue: "example.org") != .calDAV)
    #expect(CalendarProvider.iCloud.rawValue == "icloud")
}

@Test func draftAndPatchReportOnlyAvailabilityAndVisibilityAdjustments() {
    var stored = CalendarEvent(eventID: "e", calendarID: "c", title: "T", start: Date(timeIntervalSince1970: 0), end: Date(timeIntervalSince1970: 60))
    stored.availability = .busy
    stored.visibility = .default
    let draft = EventDraft(title: "T", timing: EventTiming(start: stored.start, end: stored.end, timeZone: nil, isAllDay: false), availability: .tentative, visibility: .default)
    #expect(draft.adjustments(comparedTo: stored) == [.availability])
    #expect(EventPatch(availability: .tentative, visibility: .confidential).adjustments(comparedTo: stored) == [.availability, .visibility])
    #expect(EventPatch(availability: .busy).adjustments(comparedTo: stored).isEmpty)
    stored.availability = nil   // unknown is never an adjustment
    #expect(draft.adjustments(comparedTo: stored).isEmpty)
}

// Reminders (Issue 4).

@Test func aReminderFiresRelativeToTheStartOrEndOrAtADate() {
    let start = Date(timeIntervalSince1970: 10_000), end = Date(timeIntervalSince1970: 13_600)
    #expect(Reminder(minutesBefore: 10).fireDate(eventStart: start, eventEnd: end) == start.addingTimeInterval(-600))
    #expect(Reminder(trigger: .relative(offset: -300, to: .end)).fireDate(eventStart: start, eventEnd: end) == end.addingTimeInterval(-300))
    let fixed = Date(timeIntervalSince1970: 5_000)
    #expect(Reminder(trigger: .absolute(fixed)).fireDate(eventStart: start, eventEnd: end) == fixed)
    #expect(Reminder(trigger: .location(StructuredLocation(title: "Home"), .enter)).fireDate(eventStart: start, eventEnd: end) == nil)
}

@Test func anAllDayReminderFiresExactlyItsOffsetBeforeTheCanonicalMidnightAcrossADaylightSavingChange() {
    // 2026-03-08 is 23 hours long in New York; the offset is a duration, not a wall-clock time.
    let zone = TimeZone(identifier: "America/New_York")!
    let range = AllDay.canonical(first: CalendarDate(year: 2026, month: 3, day: 8), endExclusive: CalendarDate(year: 2026, month: 3, day: 9), in: zone)!
    let reminder = Reminder.before(minutes: 9 * 60)
    #expect(reminder.fireDate(eventStart: range.start, eventEnd: range.end) == range.start.addingTimeInterval(-9 * 3600))
}

@Test func minutesBeforeIsNilUnlessTheTriggerIsRelativeToTheStart() {
    #expect(Reminder(minutesBefore: 15).minutesBefore == 15)
    #expect(Reminder(trigger: .relative(offset: -60, to: .end)).minutesBefore == nil)
    #expect(Reminder(trigger: .absolute(Date())).minutesBefore == nil)
    #expect(Reminder(trigger: .location(StructuredLocation(), .leave)).minutesBefore == nil)
}

@Test func reminderListsAreComparedIgnoringOrderAndTheCalendarDefaultFlag() {
    let a = [Reminder.before(minutes: 10, isCalendarDefault: true), .before(minutes: 60)]
    let b = [Reminder.before(minutes: 60, isCalendarDefault: false), .before(minutes: 10)]
    #expect(Reminder.sameSet(a, b))
    #expect(!Reminder.sameSet(a, [.before(minutes: 10), .before(minutes: 60, type: .email(address: nil))]))   // a changed alert type differs
}
