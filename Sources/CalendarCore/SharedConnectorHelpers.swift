import Foundation

extension Participation {
    /// A `self` attendee gives their response; an organizer who is you with no attendee entry (an event with no
    /// guests) counts as accepted; anything else is an event you are not on.
    public static func resolving(attendees: [Attendee], organizer: Attendee?) -> Participation {
        if let me = attendees.first(where: \.isSelf) { return .invited(me.response) }
        if organizer?.isSelf == true { return .invited(.accepted) }
        return .notInvited
    }
}

extension EventRef {
    /// The id a write addresses: the instance, or the series master for `.allInSeries` (whose version differs from the
    /// instance's, so the caller's version cannot lock it; a master ref's own version can).
    public func writeTarget(scope: RecurrenceScope) -> (id: String, useVersion: Bool) {
        guard let series = seriesID, !series.isEmpty else { return (eventID, true) }
        return scope == .allInSeries ? (series, series == eventID) : (eventID, true)
    }
}

/// Decodes a provider response body; any failure becomes `SourceError.invalidResponse` naming the type.
public func decodeResponse<T: Decodable>(_ type: T.Type, from data: Data) throws -> T {
    do { return try JSONDecoder().decode(type, from: data) }
    catch { throw SourceError.invalidResponse("could not decode \(T.self)") }
}

/// One incremental check across a connection's calendars. `checkCalendar` returns whether that calendar changed; the
/// first call for a calendar takes its baseline and returns false. A change to the calendar set itself wins over
/// event changes, and calendars that left the set lose their stored tokens.
public func detectCalendarChanges(
    calendarIDs: [String], setScope: String, connectionID: ConnectionID, syncState: any SyncStateStore,
    checkCalendar: (String) async throws -> Bool
) async throws -> CalendarChange? {
    let ids = calendarIDs.sorted()
    let setKey = ids.joined(separator: "\n")

    let previousKey = await syncState.token(for: connectionID, scope: setScope)
    let setChanged = previousKey != nil && previousKey != setKey
    if let previousKey, setChanged {
        for removed in Set(previousKey.split(separator: "\n").map(String.init)).subtracting(ids) {
            await syncState.setToken(nil, for: connectionID, scope: removed)
        }
    }

    var changed = Set<String>()
    for id in ids where try await checkCalendar(id) { changed.insert(id) }
    await syncState.setToken(setKey, for: connectionID, scope: setScope)

    if setChanged { return .calendarsChanged }
    return changed.isEmpty ? nil : .eventsChanged(calendarIDs: changed)
}
