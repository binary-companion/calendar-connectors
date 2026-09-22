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
        CalendarDescriptor(id: "c", title: "C", colorHex: raw).colorHex
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
    #expect(!c.canWrite && !c.canEditAttendees && !c.canRespondToInvite && !c.providesConference && !c.supportsPush)
    #expect(c.syncKind == .none)
}

@Test func descriptorsAreStandardCalendarsUnlessToldOtherwise() {
    #expect(CalendarDescriptor(id: "c", title: "C").kind == .standard)
    #expect(CalendarDescriptor(id: "c", title: "C", kind: .birthdays).kind == .birthdays)
    #expect(CalendarDescriptor(id: "c", title: "C", kind: .subscribed).kind == .subscribed)
}
