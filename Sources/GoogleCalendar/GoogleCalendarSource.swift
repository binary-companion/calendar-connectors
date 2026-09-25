import CalendarCore
import Foundation

public final class GoogleCalendarSource: PollingCalendarSource {
    static let eventFields =
        "nextPageToken,items(id,iCalUID,updated,created,status,summary,description,location,htmlLink,etag,hangoutLink,transparency,visibility,eventType,recurringEventId,start,end,originalStartTime,attendees(email,displayName,responseStatus,optional,resource,organizer,self),organizer(email,displayName,self),conferenceData(entryPoints(entryPointType,uri),conferenceSolution(key(type),name)),reminders(useDefault,overrides(method,minutes)))"

    let connection: Connection
    let api: GoogleAPIClient
    let syncState: any SyncStateStore
    let monitor: ChangeMonitor
    let calendarList = CalendarListCache()

    init(connection: Connection, api: GoogleAPIClient, syncState: any SyncStateStore, monitor: ChangeMonitor) {
        self.connection = connection
        self.api = api
        self.syncState = syncState
        self.monitor = monitor
    }

    public var id: String { connection.sourceID }
    public var displayName: String { connection.displayName }
    public var capabilities: SourceCapabilities {
        SourceCapabilities(
            canWrite: true, canEditAttendees: true, canRespondToInvite: true, providedFields: [.kind, .visibility, .availability, .reminders, .series, .participation, .structuredConference, .version, .lastModified, .created, .uidScope, .recurrenceRules,
                             .isDefault, .calendarTimeZone, .defaultReminders, .provider, .supportedAvailabilities, .permissionDetails],
            syncKind: .token,
            writableFields: Set(EventField.allCases), controlsNotifications: true, recurrenceScopes: Set(RecurrenceScope.allCases))
    }

    /// Always asks Google (an explicit request) and remembers the answer for `events(in:)`.
    public func calendars() async throws -> [CalendarDescriptor] {
        let account = connection.config["email"]
        let list = try await api.calendarList().compactMap { GoogleEventMapper.descriptor(from: $0, accountName: account) }
        await calendarList.store(list)
        return list
    }

    /// Uses the calendar list the last `calendars()` call or poll stored, so a refresh (which asks for the list and
    /// then the events) makes one `calendarList` request. A calendar removed since then answers 404 or 403 and is skipped.
    public func events(in interval: DateInterval) async throws -> [CalendarEvent] {
        let calendars: [CalendarDescriptor]
        if let stored = await calendarList.list { calendars = stored } else { calendars = try await self.calendars() }
        return try await withThrowingTaskGroup(of: [CalendarEvent].self) { group in
            for calendar in calendars {
                group.addTask { try await self.events(for: calendar, in: interval) }
            }
            var all: [CalendarEvent] = []
            for try await part in group { all += part }
            return all.sorted { ($0.start, $0.id) < ($1.start, $1.id) }
        }
    }

    private func events(for calendar: CalendarDescriptor, in interval: DateInterval) async throws -> [CalendarEvent] {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        let query = [
            URLQueryItem(name: "singleEvents", value: "true"),
            URLQueryItem(name: "timeMin", value: formatter.string(from: interval.start)),
            URLQueryItem(name: "timeMax", value: formatter.string(from: interval.end)),
            URLQueryItem(name: "maxResults", value: "250"),
            URLQueryItem(name: "showDeleted", value: "false"),
            URLQueryItem(name: "fields", value: Self.eventFields),
        ]
        var events: [CalendarEvent] = []
        do {
            try await api.pages(
                GoogleEventsPageDTO.self, path: GoogleAPIClient.calendarPath(calendar.id, "/events"), query: query,
                next: { $0.nextPageToken },
                handle: { events += ($0.items ?? []).compactMap(\.value).compactMap { GoogleEventMapper.map($0, calendar: calendar, sourceID: self.connection.sourceID) } })
        } catch let error as GoogleAPIError {
            // A calendar that was removed or lost access is skipped; anything else is not ours to interpret.
            if error == .notFound || error == .forbidden { return [] }
            throw error.sourceError
        }
        return events
    }

    public func changes() -> AsyncStream<CalendarChange> { monitor.changes(polling: self) }

    static let calendarSetScope = "_calendars"

    private struct SyncPageDTO: Decodable {
        // Items are only counted; an item that fails to parse is still a change.
        var items: [LenientItem<Item>]?
        struct Item: Decodable { var id: String? }
        var nextPageToken: String?
        var nextSyncToken: String?
    }

    public func checkForChanges() async throws -> CalendarChange? {
        let calendars = try await self.calendars()
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
                if let token = await syncState.token(for: connectionID, scope: id) {
                    if try await poll(calendarID: id, token: token) { changed.insert(id) }
                } else {
                    try await bootstrap(calendarID: id)
                }
            } catch let error as GoogleAPIError {
                if error == .notFound || error == .forbidden { continue } // removed or no longer readable
                throw error.sourceError
            }
        }
        await syncState.setToken(setKey, for: connectionID, scope: Self.calendarSetScope)

        if setChanged { return .calendarsChanged }
        return changed.isEmpty ? nil : .eventsChanged(calendarIDs: changed)
    }

    /// Lists the whole calendar (no time window; Google forbids combining a sync token with one) only to obtain a token.
    private func bootstrap(calendarID: String) async throws {
        var token: String?
        try await api.pages(
            SyncPageDTO.self, path: GoogleAPIClient.calendarPath(calendarID, "/events"),
            query: [
                URLQueryItem(name: "showDeleted", value: "true"),
                URLQueryItem(name: "maxResults", value: "2500"),
                URLQueryItem(name: "fields", value: "nextPageToken,nextSyncToken"),
            ],
            next: { $0.nextPageToken }, handle: { if let t = $0.nextSyncToken { token = t } })
        guard let token else { throw SourceError.invalidResponse("no nextSyncToken") }
        await syncState.setToken(token, for: connection.connectionID, scope: calendarID)
    }

    /// True when anything changed since `token`. Always walks to the final page, because only it carries the new token.
    private func poll(calendarID: String, token: String) async throws -> Bool {
        var anyItems = false
        var newToken: String?
        do {
            try await api.pages(
                SyncPageDTO.self, path: GoogleAPIClient.calendarPath(calendarID, "/events"),
                query: [
                    URLQueryItem(name: "syncToken", value: token),
                    URLQueryItem(name: "showDeleted", value: "true"),
                    URLQueryItem(name: "maxResults", value: "250"),
                    URLQueryItem(name: "fields", value: "nextPageToken,nextSyncToken,items(id)"),
                ],
                next: { $0.nextPageToken },
                handle: {
                    if !($0.items ?? []).isEmpty { anyItems = true }
                    if let t = $0.nextSyncToken { newToken = t }
                })
        } catch GoogleAPIError.gone {
            await syncState.setToken(nil, for: connection.connectionID, scope: calendarID)
            try await bootstrap(calendarID: calendarID)
            return true
        }
        guard let newToken else { throw SourceError.invalidResponse("no nextSyncToken") }
        await syncState.setToken(newToken, for: connection.connectionID, scope: calendarID)
        return anyItems
    }
}

/// The last calendar list this source fetched. `events(in:)` reads it; `calendars()` and each poll refresh it.
actor CalendarListCache {
    private(set) var list: [CalendarDescriptor]?
    func store(_ list: [CalendarDescriptor]) { self.list = list }
}
