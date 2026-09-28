import CalendarCore
import Foundation

extension CalDAVCalendarSource {
    static let calendarsScope = "calendars"
    static func calendarScope(_ id: String) -> String { "calendar:" + id }

    /// What a consumer must reload the calendar list for: calendars added or removed, renamed, recoloured, access,
    /// zone or default status changed. Event changes (the ctag) are not part of it.
    static func signature(_ calendars: [CalDAVCalendarInfo]) -> String {
        calendars.map { calendar in
            let d = calendar.descriptor
            return [d.id, d.title, d.colorHex ?? "", String(d.permissions.canEdit), String(d.permissions.canViewDetails), calendar.zone.identifier,
                    d.isDefault.map(String.init) ?? ""]
                .joined(separator: "\u{1F}")
        }.sorted().joined(separator: "\u{1E}")
    }

    /// `sync:<token>` when the calendar has a sync token (RFC 6578), else `ctag:<ctag>`.
    static func marker(_ calendar: CalDAVCalendarInfo) -> String {
        if let token = calendar.syncToken, !token.isEmpty { return "sync:" + token }
        return "ctag:" + (calendar.ctag ?? "")
    }

    /// One `PROPFIND` on the home; a `sync-collection` only for calendars whose token moved, so a token bump without
    /// a change is not reported. The first call stores the baseline and returns nil.
    public func checkForChanges() async throws -> CalendarChange? {
        let calendars = try await loadCalendars()
        let owner = connection.connectionID
        let signature = Self.signature(calendars)
        let stored = await syncState.token(for: owner, scope: Self.calendarsScope)
        guard stored == signature else {
            await syncState.setToken(signature, for: owner, scope: Self.calendarsScope)
            for calendar in calendars {
                await syncState.setToken(Self.marker(calendar), for: owner, scope: Self.calendarScope(calendar.descriptor.id))
            }
            return stored == nil ? nil : .calendarsChanged
        }
        // New markers are written only after every calendar was checked, so an error on one calendar cannot swallow the
        // change already found on another: the retry finds both.
        var changed = Set<String>()
        var pending: [(scope: String, marker: String)] = []
        for calendar in calendars {
            let outcome = try await check(calendar)
            if outcome.changed { changed.insert(calendar.descriptor.id) }
            if let marker = outcome.marker { pending.append((Self.calendarScope(calendar.descriptor.id), marker)) }
        }
        for entry in pending { await syncState.setToken(entry.marker, for: owner, scope: entry.scope) }
        return changed.isEmpty ? nil : .eventsChanged(calendarIDs: changed)
    }

    /// Whether the calendar changed, and the marker to store once the whole check has succeeded (nil: keep the old one).
    private func check(_ calendar: CalDAVCalendarInfo) async throws -> (changed: Bool, marker: String?) {
        let owner = connection.connectionID
        let current = Self.marker(calendar)
        let stored = await syncState.token(for: owner, scope: Self.calendarScope(calendar.descriptor.id))
        guard stored != current else { return (false, nil) }
        // A ctag server (or a calendar that changed how it reports) has nothing finer to ask.
        guard let stored, stored.hasPrefix("sync:"), current.hasPrefix("sync:") else { return (true, current) }
        let reply = try await client.report(calendar.url, depth: 1, body: DAVXML.syncCollection(token: String(stored.dropFirst(5))))
        if DAVXML.isInvalidSyncToken(reply.response) { return (true, current) }
        switch reply.response.status {
        case 207: break
        // Removed or lost access since the list was read: the next list signature reports it.
        case 403, 404, 410: return (false, nil)
        default: throw SourceError.invalidResponse("sync-collection answered \(reply.response.status)")
        }
        let status = try DAVXML.multistatus(reply.response.body)
        for response in status.responses {
            if let url = try? client.resolve(response.href, against: reply.url) { await state.forget(url) }
        }
        return (!status.responses.isEmpty, status.syncToken.map { "sync:" + $0 } ?? current)
    }
}
