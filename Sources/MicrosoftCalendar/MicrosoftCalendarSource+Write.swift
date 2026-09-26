import CalendarCore
import Foundation

extension MicrosoftCalendarSource: WritableCalendarSource {
    public func create(_ draft: EventDraft, in calendarID: String, notify: NotifyPolicy) async throws -> CalendarEvent {
        try await translated {
            try draft.validate()
            try WriteValidation.requireWritable(draft.usedFields, capabilities)
            // Build (and so validate) the body before the first request.
            let body = try GraphWriteMapper.createBody(draft)
            let calendar = try await writableCalendar(calendarID)
            if let uid = draft.uid, !uid.isEmpty, let existing = try await existingEvent(uid: uid, in: calendar) {
                throw WriteError.alreadyExists(existing)
            }
            let zone = try await accountZone(refresh: false)
            let data = try await api.send(
                method: "POST", url: api.url(path: GraphAPIClient.calendarPath(calendarID, "/events")),
                body: try GraphWriteMapper.data(body), prefer: zone.readPreferences)
            return try mapped(data, calendar: calendar)
        }
    }

    /// The calendar's own copy of the meeting with this iCalendar UID, if any (a series master, else the first
    /// instance or exception). Graph does not let a client set `iCalUId`, so an event this library creates gets its own
    /// UID; this finds copies that arrived with the uid (an invitation), which is the duplicate a caller wants to avoid.
    private func existingEvent(uid: String, in calendar: CalendarDescriptor) async throws -> CalendarEvent? {
        let zone = try await accountZone(refresh: false)
        let escaped = uid.replacingOccurrences(of: "'", with: "''")
        let url = api.url(
            path: GraphAPIClient.calendarPath(calendar.id, "/events"),
            query: [
                URLQueryItem(name: "$filter", value: "iCalUId eq '\(escaped)'"), URLQueryItem(name: "$top", value: "50"),
                URLQueryItem(name: "$select", value: GraphEventMapper.eventSelect),
            ])
        var found: [GraphEventDTO] = []
        try await api.pages(GraphListPage<GraphEventDTO>.self, from: url, prefer: zone.readPreferences) {
            found += $0.items.filter { $0.isCancelled != true }
        }
        guard let match = found.first(where: { $0.type == "seriesMaster" }) ?? found.first else { return nil }
        return try mapped(dto: match, calendar: calendar)
    }

    /// Callers read expanded instances, so a `timing` in `patch` is the instance's absolute date. Sent to the series
    /// master (`.allInSeries`) it would move the whole series, so a series-wide time change is accepted only when `ref`
    /// is the series' first occurrence (its `originalStart` equals the master's start, within a second); otherwise it
    /// throws `WriteError.unsupported(fields: [.timing])` before any PATCH. Other fields are unaffected.
    public func update(_ ref: EventRef, _ patch: EventPatch, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent {
        try await translated {
            try WriteValidation.requireWritable(patch.touchedFields, capabilities)
            try Self.requireEventID(ref)
            try Self.requireOccurrence(ref, scope: scope)
            if scope == .thisInstance, ref.seriesID != nil, patch.recurrence != .keep { throw WriteError.unsupported(fields: [.recurrence]) }
            // Validate the whole patch before the first request; attendees and the anchor are filled in from a fresh fetch.
            _ = try GraphWriteMapper.patchBody(patch, currentAttendees: [], anchor: .placeholder)
            let calendar = try await writableCalendar(ref.calendarID)
            if patch.isEmpty {
                if let base = patch.base { return base }
                return try mapped(try await fetchRaw(ref.calendarID, ref.eventID).data, calendar: calendar)
            }
            if scope == .thisAndFollowing, ref.seriesID != nil {
                guard let event = try await splitSeries(ref, calendar: calendar, patch: patch, notify: notify) else {
                    throw SourceError.invalidResponse("microsoft: the split returned no event")
                }
                return event
            }
            let (targetID, useVersion) = target(ref, scope: scope)
            let targetIsMaster = ref.seriesID == targetID
            if scope == .allInSeries, patch.timing != nil, targetID != ref.eventID {
                try await requireFirstOccurrence(ref, seriesID: targetID)
            }
            return try await PatchMerge.apply(
                patch: patch, version: useVersion ? ref.version : nil,
                fetchCurrent: { try self.mapped(try await self.fetchRaw(ref.calendarID, targetID).data, calendar: calendar) },
                write: { expected -> PatchMerge.Attempt<CalendarEvent> in
                    // Graph has no If-Match for events, so the version check is a read just before the write; a change
                    // made in between is not caught (the window is one round trip).
                    let raw = try await self.fetchRaw(ref.calendarID, targetID)
                    if let expected, let key = raw.changeKey, key != expected { return .stale }
                    let zone = try await self.accountZone(refresh: false)
                    var body = try GraphWriteMapper.patchBody(
                        patch, currentAttendees: raw.attendees, anchor: self.anchor(of: raw, patch: patch, fallback: zone.zone))
                    // Moving a series' first occurrence moves the day its range starts on, so the range moves with it.
                    if targetIsMaster, let timing = patch.timing, body["recurrence"] == nil,
                       let recurrence = raw.json["recurrence"] as? [String: Any] {
                        let zone = timing.timeZone ?? zone.zone
                        body["recurrence"] = GraphRecurrenceMapper.restarted(
                            recurrence, at: AllDay.date(of: timing.start, in: zone), remaining: nil)
                    }
                    let data = try await self.api.send(
                        method: "PATCH", url: self.api.url(path: GraphAPIClient.eventPath(ref.calendarID, targetID)),
                        body: try GraphWriteMapper.data(body), prefer: zone.readPreferences)
                    return .done(try self.mapped(data, calendar: calendar))
                })
        }
    }

    public func delete(_ ref: EventRef, scope: RecurrenceScope, notify: NotifyPolicy) async throws {
        try await translated {
            try Self.requireEventID(ref)
            try Self.requireOccurrence(ref, scope: scope)
            let calendar = try await writableCalendar(ref.calendarID)
            if scope == .thisAndFollowing, ref.seriesID != nil {
                _ = try await splitSeries(ref, calendar: calendar, patch: nil, notify: notify)
                return
            }
            let (targetID, _) = target(ref, scope: scope)
            _ = try await api.send(method: "DELETE", url: api.url(path: GraphAPIClient.eventPath(ref.calendarID, targetID)))
        }
    }

    /// `notify == .none` sends the response without telling the organizer (`sendResponse: false`); any other policy sends
    /// it. This is the one place Graph lets a caller choose.
    public func respond(to ref: EventRef, _ response: ResponseStatus, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent {
        try await translated {
            // Splitting a series only to change one guest's response is not offered.
            if scope == .thisAndFollowing, ref.seriesID != nil { throw WriteError.unsupported(fields: [.attendees]) }
            try Self.requireEventID(ref)
            try Self.requireOccurrence(ref, scope: scope)
            let action = try GraphWriteMapper.respondAction(response)
            let calendar = try await writableCalendar(ref.calendarID)
            let (targetID, _) = target(ref, scope: scope)
            _ = try await api.send(
                method: "POST", url: api.url(path: GraphAPIClient.eventPath(ref.calendarID, targetID) + "/\(action)"),
                body: try GraphWriteMapper.data(["sendResponse": notify != .none]))
            return try mapped(try await fetchRaw(ref.calendarID, targetID).data, calendar: calendar)
        }
    }

    // MARK: Helpers

    /// Translates provider outcomes into `WriteError`; `SourceError` and `WriteError` pass through unchanged.
    func translated<T>(_ body: () async throws -> T) async throws -> T {
        do { return try await body() }
        catch let error as GraphAPIError {
            switch error {
            case .gone, .notFound: throw WriteError.notFound
            case .forbidden: throw WriteError.forbidden(nil)
            case .badRequest(let message): throw WriteError.invalid(message)
            // No write sends If-Match, and a create carries no client-chosen id, so neither is expected.
            case .preconditionFailed, .conflict: throw SourceError.invalidResponse("microsoft: unexpected \(error)")
            }
        }
    }

    /// An empty id would address the events collection itself instead of one event.
    private static func requireEventID(_ ref: EventRef) throws {
        if ref.eventID.isEmpty { throw WriteError.invalid("the event id must not be empty") }
    }

    /// A write result for a series master carries its own id as `seriesID` (see `mapped`), so a caller can tell it from an
    /// occurrence. A single-instance write on it would hit the whole series, so it is refused; `.allInSeries`, and
    /// `.thisAndFollowing` at the master's start, work as usual.
    private static func requireOccurrence(_ ref: EventRef, scope: RecurrenceScope) throws {
        if scope == .thisInstance, let series = ref.seriesID, series == ref.eventID {
            throw WriteError.invalid("this is a recurring series; use .allInSeries or read the occurrence first")
        }
    }

    /// The calendar, provided it exists and the account can write to it.
    func writableCalendar(_ calendarID: String) async throws -> CalendarDescriptor {
        guard let calendar = try await loadCalendars(refreshZone: false).first(where: { $0.id == calendarID }) else { throw WriteError.notFound }
        guard calendar.permissions.canEdit else { throw WriteError.forbidden("read-only calendar") }
        return calendar
    }

    func mapped(_ data: Data, calendar: CalendarDescriptor) throws -> CalendarEvent {
        try mapped(dto: try api.decode(GraphEventDTO.self, from: data), calendar: calendar)
    }

    func mapped(dto: GraphEventDTO, calendar: CalendarDescriptor) throws -> CalendarEvent {
        if dto.isCancelled == true { throw WriteError.notFound }
        guard let event = GraphEventMapper.map(dto, calendar: calendar, accountEmail: accountEmail, sourceID: id) else {
            throw SourceError.invalidResponse("microsoft: unreadable event")
        }
        return event
    }

    struct RawEvent {
        var data: Data
        var json: [String: Any]
        var changeKey: String? { json["changeKey"] as? String }
        var attendees: [[String: Any]] { json["attendees"] as? [[String: Any]] ?? [] }
    }

    /// The full event resource, so unmodeled fields and the attendee list are present. A cancelled event is
    /// `.notFound`, so it is never patched back to life.
    func fetchRaw(_ calendarID: String, _ eventID: String) async throws -> RawEvent {
        let zone = try await accountZone(refresh: false)
        let data = try await api.get(url: api.url(path: GraphAPIClient.eventPath(calendarID, eventID)), prefer: zone.readPreferences)
        // `try?`: a body that is not JSON throws a Cocoa error, which must not escape the library.
        guard let json = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            throw SourceError.invalidResponse("microsoft: unreadable event")
        }
        if json["isCancelled"] as? Bool == true { throw WriteError.notFound }
        return RawEvent(data: data, json: json)
    }

    /// The id a write addresses: the instance, or the series master for `.allInSeries` (whose version differs from the
    /// instance's, so the caller's version cannot lock it; a master ref's own version can).
    private func target(_ ref: EventRef, scope: RecurrenceScope) -> (id: String, useVersion: Bool) {
        guard let series = ref.seriesID, !series.isEmpty else { return (ref.eventID, true) }
        return scope == .allInSeries ? (series, series == ref.eventID) : (ref.eventID, true)
    }

    /// Where a recurrence set by `patch` starts: the patch's own timing, else the event as fetched.
    private func anchor(of raw: RawEvent, patch: EventPatch, fallback: TimeZone) -> RecurrenceAnchor {
        if let timing = patch.timing {
            let zone = timing.timeZone ?? fallback
            return RecurrenceAnchor(start: AllDay.date(of: timing.start, in: zone), zone: zone)
        }
        guard let dto = try? api.decode(GraphEventDTO.self, from: raw.data), let times = GraphEventMapper.resolve(dto, fallbackZone: fallback)
        else { return RecurrenceAnchor(start: AllDay.date(of: Date(), in: fallback), zone: fallback) }
        return RecurrenceAnchor(start: AllDay.date(of: times.start, in: times.zone), zone: times.zone)
    }

    /// Throws `.unsupported(fields: [.timing])` unless `ref` is the first occurrence of the series (see `update`).
    private func requireFirstOccurrence(_ ref: EventRef, seriesID: String) async throws {
        let raw = try await fetchRaw(ref.calendarID, seriesID)
        let zone = try await accountZone(refresh: false)
        guard let dto = try? api.decode(GraphEventDTO.self, from: raw.data),
              let first = GraphEventMapper.resolve(dto, fallbackZone: zone.zone),
              let slot = ref.originalStart, abs(slot.timeIntervalSince(first.start)) < 1 else {
            throw WriteError.unsupported(fields: [.timing])
        }
    }
}
