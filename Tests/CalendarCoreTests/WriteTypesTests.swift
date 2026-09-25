import Foundation
import Testing
@testable import CalendarCore

private func event(series: String? = nil, original: Date? = nil) -> CalendarEvent {
    CalendarEvent(eventID: "e1", calendarID: "cal", title: "T", start: Date(timeIntervalSince1970: 1000),
                  end: Date(timeIntervalSince1970: 2000), series: series.map { .occurrence(seriesID: $0, originalStart: original) }, version: "v1")
}

@Test func eventRefCopiesTheFieldsAWriteNeeds() {
    let original = Date(timeIntervalSince1970: 900)
    let ref = EventRef(event(series: "s1", original: original))
    #expect(ref == EventRef(calendarID: "cal", eventID: "e1", version: "v1", seriesID: "s1", originalStart: original))
}

@Test func capabilitiesDefaultToNothingWritable() {
    let c = SourceCapabilities()
    #expect(c.writableFields.isEmpty && c.recurrenceScopes.isEmpty && !c.controlsNotifications)
}

@Test func eventsHaveNoSourceIDUntilASourceStampsOne() {
    #expect(event().sourceID == nil)
    var stamped = event()
    stamped.sourceID = "google-1"
    #expect(stamped.sourceID == "google-1" && stamped.id == "cal/e1")
}

@Test func requireWritableNamesEveryMissingField() async {
    let caps = SourceCapabilities(canWrite: true, writableFields: [.title, .timing])
    #expect(throws: Never.self) { try WriteValidation.requireWritable([.title], caps) }
    await expectWriteError(.unsupported(fields: [.attendees, .visibility])) {
        try WriteValidation.requireWritable([.title, .attendees, .visibility], caps)
    }
}

@Test func writeErrorsCompareByPayload() {
    #expect(WriteError.conflict(fields: [.title]) == .conflict(fields: [.title]))
    #expect(WriteError.conflict(fields: [.title]) != .conflict(fields: [.notes]))
    #expect(WriteError.forbidden(nil) != .forbidden("read-only calendar"))
}

@Test func anEmptySeriesIDIsNoSeries() {
    #expect(EventRef(calendarID: "cal", eventID: "e1", seriesID: "").seriesID == nil)
    #expect(EventRef(calendarID: "cal", eventID: "e1", seriesID: "s1").seriesID == "s1")
    #expect(EventRef(event(series: "")).seriesID == nil)
}

@Test func settingTheSeriesIDToEmptyLaterIsNoSeriesToo() {
    var ref = EventRef(calendarID: "cal", eventID: "e1", seriesID: "s1")
    ref.seriesID = ""
    #expect(ref.seriesID == nil)
    ref.seriesID = "s2"
    #expect(ref.seriesID == "s2")
}
