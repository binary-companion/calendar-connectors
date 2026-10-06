import Foundation
import Testing
@testable import CalendarCore

@Test func participationUsesTheSelfAttendeesResponse() {
    let me = Attendee(email: "me@x.com", response: .tentative, isSelf: true)
    let other = Attendee(email: "o@x.com", response: .accepted)
    #expect(Participation.resolving(attendees: [other, me], organizer: other) == .invited(.tentative))
}

@Test func participationCountsASelfOrganizerWithNoAttendeeEntryAsAccepted() {
    let me = Attendee(email: "me@x.com", isSelf: true)
    #expect(Participation.resolving(attendees: [], organizer: me) == .invited(.accepted))
}

@Test func participationIsNotInvitedWhenYouAreNeitherAttendeeNorOrganizer() {
    let other = Attendee(email: "o@x.com")
    #expect(Participation.resolving(attendees: [other], organizer: other) == .notInvited)
    #expect(Participation.resolving(attendees: [], organizer: nil) == .notInvited)
}

@Test func writeTargetIsTheInstanceWhenThereIsNoSeries() {
    let ref = EventRef(calendarID: "c", eventID: "e1")
    for scope in RecurrenceScope.allCases {
        let t = ref.writeTarget(scope: scope)
        #expect(t.id == "e1" && t.useVersion)
    }
}

@Test func writeTargetAddressesTheMasterForAllInSeriesWithoutTheInstanceVersion() {
    let ref = EventRef(calendarID: "c", eventID: "e1_20260101", seriesID: "e1")
    let all = ref.writeTarget(scope: .allInSeries)
    #expect(all.id == "e1" && !all.useVersion)
    let one = ref.writeTarget(scope: .thisInstance)
    #expect(one.id == "e1_20260101" && one.useVersion)
}

@Test func writeTargetKeepsTheLockWhenTheRefIsTheSeriesMaster() {
    let ref = EventRef(calendarID: "c", eventID: "e1", seriesID: "e1")
    let t = ref.writeTarget(scope: .allInSeries)
    #expect(t.id == "e1" && t.useVersion)
}

private struct Item: Decodable, Equatable { let n: Int }

@Test func decodeResponseDecodesValidJSON() throws {
    #expect(try decodeResponse(Item.self, from: Data(#"{"n":3}"#.utf8)) == Item(n: 3))
}

@Test func decodeResponseThrowsInvalidResponseNamingTheType() {
    #expect(throws: SourceError.invalidResponse("could not decode Item")) {
        try decodeResponse(Item.self, from: Data("nope".utf8))
    }
}

@Test func firstCheckTakesABaselineAndReportsNothing() async throws {
    let state = InMemorySyncStateStore()
    var seen: [String] = []
    let change = try await detectCalendarChanges(
        calendarIDs: ["b", "a"], setScope: "set", connectionID: "c1", syncState: state
    ) { id in seen.append(id); return false }
    #expect(change == nil)
    #expect(seen == ["a", "b"])
    #expect(await state.token(for: "c1", scope: "set") == "a\nb")
}

@Test func reportsWhichCalendarsChanged() async throws {
    let state = InMemorySyncStateStore()
    await state.setToken("a\nb", for: "c1", scope: "set")
    let change = try await detectCalendarChanges(
        calendarIDs: ["a", "b"], setScope: "set", connectionID: "c1", syncState: state
    ) { $0 == "b" }
    #expect(change == .eventsChanged(calendarIDs: ["b"]))
}

@Test func aChangedCalendarSetWinsAndForgetsRemovedCalendars() async throws {
    let state = InMemorySyncStateStore()
    await state.setToken("a\nb", for: "c1", scope: "set")
    await state.setToken("tok-b", for: "c1", scope: "b")
    let change = try await detectCalendarChanges(
        calendarIDs: ["a"], setScope: "set", connectionID: "c1", syncState: state
    ) { _ in true }
    #expect(change == .calendarsChanged)
    #expect(await state.token(for: "c1", scope: "b") == nil)
    #expect(await state.token(for: "c1", scope: "set") == "a")
}

@Test func nothingChangedReturnsNil() async throws {
    let state = InMemorySyncStateStore()
    await state.setToken("a", for: "c1", scope: "set")
    let change = try await detectCalendarChanges(
        calendarIDs: ["a"], setScope: "set", connectionID: "c1", syncState: state
    ) { _ in false }
    #expect(change == nil)
}

@Test func aThrowingCheckPropagates() async {
    let state = InMemorySyncStateStore()
    await #expect(throws: SourceError.authExpired) {
        _ = try await detectCalendarChanges(
            calendarIDs: ["a"], setScope: "set", connectionID: "c1", syncState: state
        ) { _ in throw SourceError.authExpired }
    }
}
