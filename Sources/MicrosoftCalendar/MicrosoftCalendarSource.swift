import CalendarCore
import Foundation

/// The account's default time zone, and how a request asks Graph to express times in it.
struct AccountZone: Sendable {
    /// The name as Graph gave it (a Windows name or an IANA id), sent back in `Prefer: outlook.timezone`.
    var name: String
    var zone: TimeZone

    static let utc = AccountZone(name: "UTC", zone: TimeZone(identifier: "UTC")!)

    /// Times come back in the account zone (so an all-day event falls on the date the user sees) and bodies as text.
    var readPreferences: [String] { ["outlook.timezone=\"\(name)\"", "outlook.body-content-type=\"text\""] }
}

/// What a source has learned from Graph and reuses: the calendar list and the account zone.
actor MicrosoftSourceState {
    private(set) var calendars: [CalendarDescriptor]?
    private(set) var zone: AccountZone?
    func store(calendars: [CalendarDescriptor]) { self.calendars = calendars }
    func store(zone: AccountZone) { self.zone = zone }
}

public final class MicrosoftCalendarSource: PollingCalendarSource {
    let connection: Connection
    let api: GraphAPIClient
    let syncState: any SyncStateStore
    let monitor: ChangeMonitor
    let now: @Sendable () -> Date
    let state = MicrosoftSourceState()

    init(connection: Connection, api: GraphAPIClient, syncState: any SyncStateStore, monitor: ChangeMonitor, now: @escaping @Sendable () -> Date) {
        self.connection = connection
        self.api = api
        self.syncState = syncState
        self.monitor = monitor
        self.now = now
    }

    public var id: String { connection.sourceID }
    public var displayName: String { connection.displayName }
    var accountEmail: String? { connection.config["email"]?.lowercased() }

    public var capabilities: SourceCapabilities {
        SourceCapabilities(
            canWrite: true, canEditAttendees: true, canRespondToInvite: true,
            providedFields: [.kind, .visibility, .availability, .reminders, .series, .participation, .structuredConference, .version,
                             .lastModified, .created, .uidScope, .recurrenceRules, .isDefault, .calendarTimeZone, .provider,
                             .supportedAvailabilities, .permissionDetails],
            syncKind: .token, writableFields: Set(EventField.allCases), controlsNotifications: false,
            recurrenceScopes: Set(RecurrenceScope.allCases))
    }

    // MARK: Account zone and calendars

    /// The mailbox time zone (`MailboxSettings.Read`). A definite "no" (the scope was refused, the zone is unknown to
    /// the table) is UTC and remembered; a failure that may pass (network, server, throttling) is rethrown so the
    /// events are not read in the wrong zone.
    func accountZone(refresh: Bool) async throws -> AccountZone {
        if !refresh, let known = await state.zone { return known }
        let zone: AccountZone
        do {
            let data = try await api.get(url: api.url(path: "/me/mailboxSettings/timeZone"))
            if let name = try api.decode(GraphValueDTO.self, from: data).value, let resolved = WindowsTimeZones.timeZone(for: name) {
                zone = AccountZone(name: name, zone: resolved)
            } else {
                zone = .utc
            }
        } catch is GraphAPIError {
            zone = .utc
        }
        await state.store(zone: zone)
        return zone
    }

    /// Always asks Graph (an explicit request) and remembers the answer for `events(in:)`.
    public func calendars() async throws -> [CalendarDescriptor] { try await loadCalendars(refreshZone: true) }

    func loadCalendars(refreshZone: Bool) async throws -> [CalendarDescriptor] {
        let zone = try await accountZone(refresh: refreshZone)
        var entries: [GraphCalendarDTO] = []
        do {
            try await api.pages(
                GraphListPage<GraphCalendarDTO>.self,
                from: api.url(path: "/me/calendars", query: [URLQueryItem(name: "$top", value: "100")]),
                handle: { entries += $0.items })
        } catch let error as GraphAPIError {
            throw error.sourceError
        }
        let list = entries.map { GraphEventMapper.descriptor(from: $0, accountName: accountEmail, zone: zone.zone) }
        await state.store(calendars: list)
        return list
    }

    // MARK: Events

    /// Uses the calendar list the last `calendars()` call or poll stored, so a refresh makes one list request. A
    /// calendar removed since then answers 404 or 403 and is skipped.
    public func events(in interval: DateInterval) async throws -> [CalendarEvent] {
        let calendars: [CalendarDescriptor]
        if let stored = await state.calendars { calendars = stored } else { calendars = try await loadCalendars(refreshZone: true) }
        let zone = try await accountZone(refresh: false)
        return try await withThrowingTaskGroup(of: [CalendarEvent].self) { group in
            for calendar in calendars {
                group.addTask { try await self.events(for: calendar, in: interval, zone: zone) }
            }
            var all: [CalendarEvent] = []
            for try await part in group { all += part }
            return all.sorted { ($0.start, $0.id) < ($1.start, $1.id) }
        }
    }

    private func events(for calendar: CalendarDescriptor, in interval: DateInterval, zone: AccountZone) async throws -> [CalendarEvent] {
        let url = api.url(
            path: GraphAPIClient.calendarPath(calendar.id, "/calendarView"),
            query: [
                URLQueryItem(name: "startDateTime", value: GraphTime.instantText(interval.start)),
                URLQueryItem(name: "endDateTime", value: GraphTime.instantText(interval.end)),
                URLQueryItem(name: "$top", value: "100"),
                URLQueryItem(name: "$select", value: GraphEventMapper.eventSelect),
            ])
        var events: [CalendarEvent] = []
        do {
            try await api.pages(GraphListPage<GraphEventDTO>.self, from: url, prefer: zone.readPreferences) { page in
                events += page.items.compactMap { GraphEventMapper.map($0, calendar: calendar, accountEmail: self.accountEmail, sourceID: self.id) }
            }
        } catch let error as GraphAPIError {
            // A calendar that was removed or lost access is skipped; anything else is not ours to interpret.
            if error == .notFound || error == .forbidden { return [] }
            throw error.sourceError
        }
        return events
    }

    public func changes() -> AsyncStream<CalendarChange> { monitor.changes(polling: self) }
}
