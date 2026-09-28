import CalendarCore
import Foundation
import ICalendar

struct CalDAVCalendarInfo: Sendable {
    var descriptor: CalendarDescriptor
    var url: URL
    var ctag: String?
    var syncToken: String?
    /// Floating times and all-day dates on this calendar are read in this zone.
    var zone: TimeZone
}

/// What a source has learned and reuses: the calendar list and parsed resources (by URL, while the ETag matches).
actor CalDAVSourceState {
    private(set) var calendars: [CalDAVCalendarInfo]?
    private var parsed: [URL: (etag: String, resource: EventResource)] = [:]

    func store(calendars: [CalDAVCalendarInfo]) { self.calendars = calendars }
    func resource(at url: URL, etag: String) -> EventResource? {
        guard let entry = parsed[url], entry.etag == etag else { return nil }
        return entry.resource
    }
    func remember(_ resource: EventResource, at url: URL, etag: String) { parsed[url] = (etag, resource) }
    func forget(_ url: URL) { parsed[url] = nil }
}

/// One CalDAV account (iCloud or another server): reads through `calendar-query`, polls with `sync-collection` (or the
/// ctag), writes whole resources with ETags, and expands recurring events itself.
public final class CalDAVCalendarSource: PollingCalendarSource {
    let connection: Connection
    let account: CalDAVAccountConfig
    let client: WebDAVClient
    let provider: CalendarProvider
    let syncState: any SyncStateStore
    let monitor: ChangeMonitor
    let now: @Sendable () -> Date
    let defaultZone: TimeZone
    let makeUUID: @Sendable () -> String
    let state = CalDAVSourceState()

    init(
        connection: Connection, account: CalDAVAccountConfig, client: WebDAVClient, provider: CalendarProvider,
        syncState: any SyncStateStore, monitor: ChangeMonitor, now: @escaping @Sendable () -> Date,
        defaultZone: TimeZone = .current, makeUUID: @escaping @Sendable () -> String = { UUID().uuidString }
    ) {
        self.connection = connection
        self.account = account
        self.client = client
        self.provider = provider
        self.syncState = syncState
        self.monitor = monitor
        self.now = now
        self.defaultZone = defaultZone
        self.makeUUID = makeUUID
    }

    public var id: String { connection.sourceID }
    public var displayName: String { connection.displayName }

    /// The addresses that mean "me" on an event: the principal's address set and the principal URL (some servers name
    /// the organizer by it). Empty when the server named no addresses, so nothing is ever marked as self.
    var selfAddresses: Set<String> {
        guard !account.userAddresses.isEmpty else { return [] }
        return Set(account.userAddresses + [account.principalURL.absoluteString.lowercased()])
    }

    /// The `mailto:` address the account organizes meetings as: the one matching the user name, else the first.
    var organizerAddress: String? {
        let mail = account.userAddresses.filter { $0.hasPrefix("mailto:") }
        return mail.first { $0 == "mailto:" + account.username.lowercased() } ?? mail.first
    }

    public var capabilities: SourceCapabilities {
        var provided: Set<ProvidedField> = [.visibility, .availability, .reminders, .series, .version, .uidScope, .recurrenceRules,
                                            .permissionDetails, .provider, .calendarTimeZone, .supportedAvailabilities]
        if !account.userAddresses.isEmpty { provided.insert(.participation) }
        var writable: Set<EventField> = [.title, .notes, .location, .timing, .availability, .visibility, .reminders, .recurrence]
        // Without server scheduling, attendees written would never be invited.
        if account.autoSchedule { writable.insert(.attendees) }
        return SourceCapabilities(
            canWrite: true, canEditAttendees: account.autoSchedule,
            canRespondToInvite: account.autoSchedule && !account.userAddresses.isEmpty,
            providedFields: provided, syncKind: .token, writableFields: writable, controlsNotifications: false,
            recurrenceScopes: Set(RecurrenceScope.allCases))
    }

    // MARK: URLs and ids

    func calendarURL(_ calendarID: String) -> URL { account.homeURL.appendingPathComponent(calendarID, isDirectory: true) }

    func resourceURL(calendarID: String, name: String) -> URL { calendarURL(calendarID).appendingPathComponent(name) }

    /// The collection path below the home, without its trailing slash; nil for the home itself or anything outside it.
    func calendarID(for url: URL) -> String? {
        let home = account.homeURL.path.hasSuffix("/") ? account.homeURL.path : account.homeURL.path + "/"
        let path = url.path
        guard path.hasPrefix(home) else { return nil }
        let id = String(path.dropFirst(home.count)).trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        return id.isEmpty ? nil : id
    }

    // MARK: Calendars

    public func calendars() async throws -> [CalendarDescriptor] { try await loadCalendars().map(\.descriptor) }

    func loadCalendars() async throws -> [CalDAVCalendarInfo] {
        let (status, answeredBy) = try await client.propfind(account.homeURL, depth: 1, [
            .resourceType, .displayName, .supportedComponents, .privileges, .calendarTimeZone, .calendarColor, .getCTag, .syncToken,
        ])
        let defaultID = try await defaultCalendarID()
        var list: [CalDAVCalendarInfo] = []
        for response in status.responses {
            guard let url = try? client.resolve(response.href, against: answeredBy), let id = calendarID(for: url),
                  let info = calendar(from: response, id: id, defaultID: defaultID) else { continue }
            list.append(info)
        }
        list.sort { $0.descriptor.id < $1.descriptor.id }
        await state.store(calendars: list)
        return list
    }

    func calendar(from response: DAVResponse, id: String, defaultID: String??) -> CalDAVCalendarInfo? {
        let types = response.properties[.resourceType]?.children ?? []
        func has(_ namespace: String, _ name: String) -> Bool { types.contains { $0.namespace == namespace && $0.name == name } }
        guard has(DAV.caldav, "calendar"), !has(DAV.caldav, "schedule-inbox"), !has(DAV.caldav, "schedule-outbox"),
              !has(DAV.calendarServer, "notification") else { return nil }
        if let set = response.properties[.supportedComponents] {
            let names = set.children(DAV.caldav, "comp").compactMap { $0.attributes["name"]?.uppercased() }
            guard names.contains("VEVENT") else { return nil }
        }
        // A server that does not report privileges is treated as the owner's own calendar.
        let privileges = response.properties[.privileges].map { set in
            Set(set.children(DAV.dav, "privilege").flatMap { $0.children.map(\.name) })
        }
        let subscribed = has(DAV.calendarServer, "subscribed")
        let canEdit = !subscribed && (privileges.map { !$0.isDisjoint(with: ["write", "write-content", "all"]) } ?? true)
        let permissions = CalendarPermissions(
            canViewDetails: privileges.map { !$0.isDisjoint(with: ["read", "all"]) } ?? true, canEdit: canEdit,
            canShare: privileges.map { !$0.isDisjoint(with: ["write-acl", "all"]) } ?? false, canViewPrivate: canEdit)
        let zone = response.properties[.calendarTimeZone].flatMap { Self.zone(fromCalendarTimeZone: $0.text) } ?? defaultZone
        let displayName = response.properties[.displayName]?.trimmedText ?? ""
        let color = response.properties[.calendarColor].map { String($0.trimmedText.prefix(7)) }
        let descriptor = CalendarDescriptor(
            id: id, title: displayName.isEmpty ? (id.split(separator: "/").last.map(String.init) ?? id) : displayName,
            service: .calDAV, colorHex: color, permissions: permissions, isDefault: defaultID.map { $0 == id }, timeZone: zone,
            accountName: connection.displayName, kind: subscribed ? .subscribed : .standard, provider: provider,
            supportedAvailabilities: [.busy, .free])
        return CalDAVCalendarInfo(descriptor: descriptor, url: calendarURL(id), ctag: response.properties[.getCTag]?.trimmedText,
                                  syncToken: response.properties[.syncToken]?.trimmedText, zone: zone)
    }

    /// The `calendar-timezone` value is a `VCALENDAR` holding one `VTIMEZONE`.
    static func zone(fromCalendarTimeZone text: String) -> TimeZone? {
        guard let calendar = try? ICalParser.parse(text),
              let tzid = calendar.components(named: "VTIMEZONE").first?.property("TZID")?.value else { return nil }
        return TimeZoneResolver(calendar: calendar).zone(for: tzid)
    }

    /// `.some(id)` for the calendar that receives invitations, `.some(nil)` when the server says none of these, nil when
    /// it does not say (RFC 6638 puts `schedule-default-calendar-URL` on the scheduling inbox; some servers on the
    /// principal). Only an expired sign-in is an error here.
    func defaultCalendarID() async throws -> String?? {
        do {
            let (principal, principalURL) = try await client.propfind(account.principalURL, depth: 0, [.scheduleDefaultCalendarURL, .scheduleInboxURL])
            let properties = principal.responses.first?.properties ?? [:]
            if let href = properties[.scheduleDefaultCalendarURL]?.child(DAV.dav, "href")?.trimmedText {
                return .some(calendarID(for: try client.resolve(href, against: principalURL)))
            }
            guard let inboxHref = properties[.scheduleInboxURL]?.child(DAV.dav, "href")?.trimmedText else { return nil }
            let inboxURL = try client.resolve(inboxHref, against: principalURL)
            let (inbox, answeredBy) = try await client.propfind(inboxURL, depth: 0, [.scheduleDefaultCalendarURL])
            guard let href = inbox.responses.first?.properties[.scheduleDefaultCalendarURL]?.child(DAV.dav, "href")?.trimmedText else { return nil }
            return .some(calendarID(for: try client.resolve(href, against: answeredBy)))
        } catch SourceError.authExpired {
            throw SourceError.authExpired
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            return nil
        }
    }

    func calendarZone(_ calendarID: String) async throws -> TimeZone {
        let calendars: [CalDAVCalendarInfo]
        if let stored = await state.calendars { calendars = stored } else { calendars = try await loadCalendars() }
        return calendars.first { $0.descriptor.id == calendarID }?.zone ?? defaultZone
    }

    func context(calendarID: String, zone: TimeZone, resourceName: String, etag: String?) -> EventReadContext {
        EventReadContext(calendarID: calendarID, resourceName: resourceName, etag: etag, sourceID: id, calendarZone: zone,
                         selfAddresses: selfAddresses)
    }

    // MARK: Events

    /// Uses the calendar list the last `calendars()` call or poll stored. A calendar removed since then (403, 404, 410)
    /// is skipped; one the account can see only as free/busy is not queried.
    public func events(in interval: DateInterval) async throws -> [CalendarEvent] {
        let calendars: [CalDAVCalendarInfo]
        if let stored = await state.calendars { calendars = stored } else { calendars = try await loadCalendars() }
        return try await withThrowingTaskGroup(of: [CalendarEvent].self) { group in
            for calendar in calendars where calendar.descriptor.permissions.canViewDetails {
                group.addTask { try await self.events(for: calendar, in: interval) }
            }
            var all: [CalendarEvent] = []
            for try await part in group { all += part }
            return all.sorted { ($0.start, $0.eventID) < ($1.start, $1.eventID) }
        }
    }

    private func events(for calendar: CalDAVCalendarInfo, in interval: DateInterval) async throws -> [CalendarEvent] {
        let reply = try await client.report(calendar.url, depth: 1, body: DAVXML.calendarQuery(from: interval.start, to: interval.end))
        switch reply.response.status {
        case 207: break
        case 403, 404, 410: return []
        default: throw SourceError.invalidResponse("calendar-query answered \(reply.response.status)")
        }
        var events: [CalendarEvent] = []
        for response in try DAVXML.multistatus(reply.response.body).responses {
            guard let url = try? client.resolve(response.href, against: reply.url),
                  let object = await calendarObject(response, at: url) else { continue }
            let context = context(calendarID: calendar.descriptor.id, zone: calendar.zone, resourceName: url.lastPathComponent, etag: object.etag)
            events += EventReader.events(in: object.resource, overlapping: interval, context: context)
        }
        return events
    }

    /// The parsed resource in a multistatus response (reused while its ETag is unchanged); nil when it has no ETag or
    /// cannot be read, so one bad file does not hide the rest.
    func calendarObject(_ response: DAVResponse, at url: URL) async -> (resource: EventResource, etag: String)? {
        guard let etag = response.properties[.getETag]?.trimmedText, !etag.isEmpty else { return nil }
        if let cached = await state.resource(at: url, etag: etag) { return (cached, etag) }
        guard let text = response.properties[.calendarData]?.text, let resource = try? EventResource(data: Data(text.utf8)) else { return nil }
        await state.remember(resource, at: url, etag: etag)
        return (resource, etag)
    }

    public func changes() -> AsyncStream<CalendarChange> { monitor.changes(polling: self) }
}
