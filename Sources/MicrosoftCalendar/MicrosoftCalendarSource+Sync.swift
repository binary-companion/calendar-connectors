import CalendarCore
import Foundation

/// A calendar's delta position: the link to poll next, and when its window was baselined. Kept in the sync state
/// store as `"<baseline seconds>|<link>"` because the store holds one string.
struct DeltaState: Equatable {
    var baselineDate: Date
    var link: URL

    init(baselineDate: Date, link: URL) {
        self.baselineDate = baselineDate
        self.link = link
    }

    init?(_ stored: String) {
        guard let bar = stored.firstIndex(of: "|"), let seconds = TimeInterval(stored[..<bar]),
              let link = URL(string: String(stored[stored.index(after: bar)...])) else { return nil }
        self.init(baselineDate: Date(timeIntervalSince1970: seconds), link: link)
    }

    var stored: String { "\(Int(baselineDate.timeIntervalSince1970))|\(link.absoluteString)" }
}

extension MicrosoftCalendarSource {
    static let calendarSetScope = "_calendars"
    /// The delta window: this far back and this far ahead of the day it was baselined.
    static let windowBefore: TimeInterval = 30 * 86_400
    static let windowAfter: TimeInterval = 365 * 86_400
    /// A window is renewed once it is this old, so it always keeps 16 days back and 350 days ahead.
    static let windowRenewal: TimeInterval = 14 * 86_400
    private static let pagePreference = "odata.maxpagesize=200"

    /// One incremental check. The first call for a calendar takes its baseline and reports nothing.
    public func checkForChanges() async throws -> CalendarChange? {
        let calendars = try await loadCalendars(refreshZone: false)
        let ids = calendars.map(\.id).sorted()
        let setKey = ids.joined(separator: "\n")
        let connectionID = connection.connectionID

        let previousKey = await syncState.token(for: connectionID, scope: Self.calendarSetScope)
        let setChanged = previousKey != nil && previousKey != setKey
        if let previousKey, setChanged {
            for removed in Set(previousKey.split(separator: "\n").map(String.init)).subtracting(ids) {
                await syncState.setToken(nil, for: connectionID, scope: removed)
            }
        }

        var changed = Set<String>()
        for id in ids {
            do {
                if let stored = await syncState.token(for: connectionID, scope: id), let delta = DeltaState(stored) {
                    if try await poll(calendarID: id, delta: delta) { changed.insert(id) }
                } else {
                    try await baseline(calendarID: id)
                }
            } catch let error as GraphAPIError {
                if error == .notFound || error == .forbidden { continue }   // removed or no longer readable
                throw error.sourceError
            }
        }
        await syncState.setToken(setKey, for: connectionID, scope: Self.calendarSetScope)

        if setChanged { return .calendarsChanged }
        return changed.isEmpty ? nil : .eventsChanged(calendarIDs: changed)
    }

    /// Lists the window only to obtain the delta link.
    private func baseline(calendarID: String) async throws {
        let taken = now()
        let url = api.url(
            path: GraphAPIClient.calendarPath(calendarID, "/calendarView/delta"),
            query: [
                URLQueryItem(name: "startDateTime", value: GraphTime.instantText(taken.addingTimeInterval(-Self.windowBefore))),
                URLQueryItem(name: "endDateTime", value: GraphTime.instantText(taken.addingTimeInterval(Self.windowAfter))),
            ])
        var link: String?
        try await api.pages(GraphListPage<GraphIDDTO>.self, from: url, prefer: [Self.pagePreference]) { if let d = $0.deltaLink { link = d } }
        guard let link, let deltaURL = URL(string: link) else { throw SourceError.invalidResponse("no deltaLink") }
        await syncState.setToken(DeltaState(baselineDate: taken, link: deltaURL).stored, for: connection.connectionID, scope: calendarID)
    }

    /// True when anything changed since `delta`. Always walks to the last page, because only it carries the new link.
    /// A link Graph no longer accepts, and a window that has aged out, take a new baseline; both report a change,
    /// because an edit between the last poll and the new baseline would otherwise never be reported.
    private func poll(calendarID: String, delta: DeltaState) async throws -> Bool {
        var anyItems = false
        var newLink: String?
        do {
            try await api.pages(GraphListPage<GraphIDDTO>.self, from: delta.link, prefer: [Self.pagePreference]) { page in
                // Items are only counted; an item that fails to parse is still a change (removals arrive as `@removed`).
                if !(page.value ?? []).isEmpty { anyItems = true }
                if let d = page.deltaLink { newLink = d }
            }
        } catch GraphAPIError.gone {
            try await baseline(calendarID: calendarID)
            return true
        }
        guard let newLink, let url = URL(string: newLink) else { throw SourceError.invalidResponse("no deltaLink") }
        await syncState.setToken(DeltaState(baselineDate: delta.baselineDate, link: url).stored, for: connection.connectionID, scope: calendarID)
        if now().timeIntervalSince(delta.baselineDate) > Self.windowRenewal {
            try await baseline(calendarID: calendarID)
            return true
        }
        return anyItems
    }
}
