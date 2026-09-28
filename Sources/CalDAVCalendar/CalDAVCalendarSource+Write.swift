import CalendarCore
import Foundation
import ICalendar

struct FetchedResource: Sendable {
    var resource: EventResource
    var etag: String
    var url: URL
    var name: String
}

enum PutOutcome: Sendable {
    case stored(etag: String, resource: EventResource)
    case stale
}

extension CalDAVCalendarSource: WritableCalendarSource {
    static let seriesMessage = "this is a recurring series; use .allInSeries or read the occurrence first"
    static let slotMessage = "needs the occurrence's original start"

    // MARK: Plumbing

    /// `name.ics` or `name.ics#20260915T170000Z`: the resource name and whether the id names one occurrence.
    static func resourceName(of eventID: String) -> (name: String, isOccurrence: Bool) {
        guard let hash = eventID.firstIndex(of: "#") else { return (eventID, false) }
        return (String(eventID[..<hash]), true)
    }

    /// How far ahead a written `VTIMEZONE` must reach: the rule's `UNTIL`, 20 years for an open or counted rule.
    static func seriesEnd(start: Date, rule: RecurrenceRule?) -> Date {
        guard let rule else { return start }
        if case .until(let until) = rule.end { return until }
        return start.addingTimeInterval(20 * 366 * 86_400)
    }

    func fetch(calendarID: String, name: String) async throws -> FetchedResource {
        let url = resourceURL(calendarID: calendarID, name: name)
        let reply = try await client.send("GET", url)
        switch reply.response.status {
        case 200: break
        case 404, 410: throw WriteError.notFound
        case 403: throw WriteError.forbidden(nil)
        default: throw SourceError.invalidResponse("GET answered \(reply.response.status)")
        }
        guard let etag = reply.response.header("ETag") else { throw SourceError.invalidResponse("the server sent no ETag") }
        guard let resource = try? EventResource(data: reply.response.body) else { throw SourceError.invalidResponse("the event could not be read") }
        return FetchedResource(resource: resource, etag: etag, url: url, name: name)
    }

    /// PUTs the whole resource; 412 is `.stale`. The ETag comes from the response, or from one GET when the server
    /// sends none (the returned resource is then the server's copy).
    func put(_ resource: EventResource, to url: URL, ifMatch: String? = nil, ifNoneMatch: Bool = false) async throws -> PutOutcome {
        var headers = ["Content-Type": "text/calendar; charset=utf-8"]
        if let ifMatch { headers["If-Match"] = ifMatch }
        if ifNoneMatch { headers["If-None-Match"] = "*" }
        let reply = try await client.send("PUT", url, headers: headers, body: resource.serialized())
        switch reply.response.status {
        case 200, 201, 204: break
        case 412: return .stale
        case 403: throw WriteError.forbidden(nil)
        case 404, 410: throw WriteError.notFound
        default: throw SourceError.invalidResponse("PUT answered \(reply.response.status)")
        }
        await state.forget(url)
        if let etag = reply.response.header("ETag") { return .stored(etag: etag, resource: resource) }
        let reread = try await client.send("GET", url)
        guard reread.response.status == 200, let etag = reread.response.header("ETag"),
              let copy = try? EventResource(data: reread.response.body) else {
            throw SourceError.invalidResponse("the stored event could not be read back")
        }
        return .stored(etag: etag, resource: copy)
    }

    /// The occurrence at `originalStart`, or the master in its series form (a single event when it does not recur).
    func readBack(_ resource: EventResource, etag: String, calendarID: String, name: String, originalStart: Date?) async throws -> CalendarEvent {
        let context = context(calendarID: calendarID, zone: try await calendarZone(calendarID), resourceName: name, etag: etag)
        if let originalStart, let occurrence = EventReader.occurrence(in: resource, originalStart: originalStart, context: context) {
            return occurrence
        }
        // A resource with overrides but no master (an invite to single occurrences) reads as its first override.
        let everything = DateInterval(start: .distantPast, end: .distantFuture)
        guard let master = EventReader.masterEvent(of: resource, context: context)
            ?? EventReader.events(in: resource, overlapping: everything, context: context).first else { throw WriteError.notFound }
        return master
    }

    func isRecurring(_ resource: EventResource) -> Bool {
        !resource.overrides.isEmpty || resource.master.map { $0.property("RRULE") != nil || $0.property("RDATE") != nil } == true
    }

    func masterStart(_ resource: EventResource, calendarZone: TimeZone) -> Date? {
        resource.master.flatMap { EventReader.timing(of: $0, resolver: resource.resolver, calendarZone: calendarZone)?.start }
    }

    /// Whether the server would tell anyone but the account about a change: any attendee that is not the account, or an
    /// organizer that is not the account (an invite often lists only the account as an attendee, and an attendee's delete
    /// or exception is answered to the organizer). An organizer whose address cannot be read counts as someone else.
    func tellsOthers(_ resource: EventResource) -> Bool {
        resource.events.contains { vevent in
            let people = AttendeeMapper.read(vevent, selfAddresses: selfAddresses)
            return people.attendees.contains { !$0.isSelf } || people.organizer.map { !$0.isSelf } == true
        }
    }

    /// Implicit scheduling tells attendees of every change and cannot be stopped (`controlsNotifications == false`).
    func requireNotify(_ notify: NotifyPolicy, tellsOthers: Bool) throws {
        if notify != .all && tellsOthers { throw WriteError.unsupported(fields: [.attendees]) }
    }

    /// The event a patch is judged against after a stale write: the occurrence the caller edited when it still
    /// exists, else the master.
    func comparedEvent(_ fresh: FetchedResource, ref: EventRef, scope: RecurrenceScope, calendarZone: TimeZone) throws -> CalendarEvent {
        let context = context(calendarID: ref.calendarID, zone: calendarZone, resourceName: fresh.name, etag: fresh.etag)
        if isRecurring(fresh.resource), let slot = ref.originalStart {
            let occurrence = EventReader.occurrence(in: fresh.resource, originalStart: slot, context: context)
            if scope != .allInSeries {
                guard let occurrence else { throw WriteError.notFound }
                return occurrence
            }
            // A whole-series edit is judged against the series master, so another client's change to the series shows up
            // even when the occurrence the caller read has its own values. The caller's base is an occurrence, so the
            // timing stays the occurrence's.
            if var master = EventReader.masterEvent(of: fresh.resource, context: context) {
                if let occurrence {
                    master.start = occurrence.start
                    master.end = occurrence.end
                    master.timeZone = occurrence.timeZone
                    master.isAllDay = occurrence.isAllDay
                }
                return master
            }
            if let occurrence { return occurrence }   // a resource of overrides only
            throw WriteError.notFound
        }
        // The id names an occurrence of a series that has since become a single event: that occurrence no longer exists.
        if Self.resourceName(of: ref.eventID).isOccurrence && scope != .allInSeries { throw WriteError.notFound }
        guard let master = EventReader.masterEvent(of: fresh.resource, context: context) else { throw WriteError.notFound }
        return master
    }

    // MARK: Create

    public func create(_ draft: EventDraft, in calendarID: String, notify: NotifyPolicy) async throws -> CalendarEvent {
        try draft.validate()
        try WriteValidation.requireWritable(draft.usedFields, capabilities)
        try requireNotify(notify, tellsOthers: !draft.attendees.isEmpty)
        let stamp = now()
        let vevent = try EventWriter.vevent(from: draft, uid: draft.uid ?? makeUUID(), now: stamp, organizerAddress: organizerAddress)
        let zone = draft.timing.timeZone ?? TimeZone(identifier: "UTC")!
        let resource = try EventWriter.resource(for: vevent, zones: [zone], from: draft.timing.start,
                                                through: Self.seriesEnd(start: draft.timing.start, rule: draft.recurrence))
        if let uid = draft.uid { try await refuseDuplicate(uid: uid, calendarID: calendarID) }
        // A 412 means the name is taken: one more try with a new one.
        for _ in 0..<2 {
            let name = makeUUID() + ".ics"
            if case .stored(let etag, let copy) = try await put(resource, to: resourceURL(calendarID: calendarID, name: name), ifNoneMatch: true) {
                return try await readBack(copy, etag: etag, calendarID: calendarID, name: name, originalStart: nil)
            }
        }
        throw SourceError.invalidResponse("the server refused every new event name")
    }

    private func refuseDuplicate(uid: String, calendarID: String) async throws {
        let reply = try await client.report(calendarURL(calendarID), depth: 1, body: DAVXML.calendarQuery(uid: uid))
        switch reply.response.status {
        case 207: break
        case 403: throw WriteError.forbidden(nil)
        case 404, 410: throw WriteError.notFound
        default: throw SourceError.invalidResponse("calendar-query answered \(reply.response.status)")
        }
        let zone = try await calendarZone(calendarID)
        for response in try DAVXML.multistatus(reply.response.body).responses {
            guard let url = try? client.resolve(response.href, against: reply.url), let object = await calendarObject(response, at: url),
                  object.resource.uid == uid else { continue }
            let context = context(calendarID: calendarID, zone: zone, resourceName: url.lastPathComponent, etag: object.etag)
            let everything = DateInterval(start: .distantPast, end: .distantFuture)
            if let existing = EventReader.masterEvent(of: object.resource, context: context)
                ?? EventReader.events(in: object.resource, overlapping: everything, context: context).first {
                throw WriteError.alreadyExists(existing)
            }
        }
    }

    // MARK: Update

    public func update(_ ref: EventRef, _ patch: EventPatch, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent {
        let (name, isOccurrence) = Self.resourceName(of: ref.eventID)
        if patch.isEmpty {
            if let base = patch.base { return base }
            let current = try await fetch(calendarID: ref.calendarID, name: name)
            return try await readBack(current.resource, etag: current.etag, calendarID: ref.calendarID, name: name,
                                      originalStart: isOccurrence ? ref.originalStart : nil)
        }
        try WriteValidation.requireWritable(patch.touchedFields, capabilities)
        try patch.timing?.validate()
        if case .set(let rule) = patch.recurrence { try rule.validate() }
        if case .clear = patch.reminders { throw WriteError.unsupported(fields: [.reminders]) }
        if case .set(let reminders) = patch.reminders { _ = try AlarmMapper.alarms(from: reminders) }
        try Self.checkSeriesRef(ref, scope: scope, isOccurrence: isOccurrence, movesTime: patch.timing != nil)
        let zone = try await calendarZone(ref.calendarID)
        let url = resourceURL(calendarID: ref.calendarID, name: name)
        // The first attempt may use the copy the caller just read, when its version is that copy's ETag.
        var cached: FetchedResource? = nil
        if let version = ref.version, let copy = await state.resource(at: url, etag: version) {
            cached = FetchedResource(resource: copy, etag: version, url: url, name: name)
        }
        return try await PatchMerge.apply(
            patch: patch, version: ref.version,
            fetchCurrent: { try self.comparedEvent(try await self.fetch(calendarID: ref.calendarID, name: name), ref: ref, scope: scope, calendarZone: zone) },
            write: { version in
                let current: FetchedResource
                if let first = cached { current = first; cached = nil } else { current = try await self.fetch(calendarID: ref.calendarID, name: name) }
                if let version, version != current.etag { return .stale }
                try self.requireNotify(notify, tellsOthers: self.tellsOthers(current.resource) || !(patch.attendees?.add.isEmpty ?? true))
                if scope == .thisAndFollowing, isOccurrence, self.isRecurring(current.resource), let slot = ref.originalStart,
                   let first = self.masterStart(current.resource, calendarZone: zone), slot > first {
                    return try await self.splitSeries(current, ref: ref, patch: patch, slot: slot, calendarZone: zone)
                }
                var resource = current.resource
                let slot = try self.edit(&resource, ref: ref, patch: patch, scope: scope, isOccurrence: isOccurrence, calendarZone: zone)
                switch try await self.put(resource, to: current.url, ifMatch: current.etag) {
                case .stale: return .stale
                case .stored(let etag, let copy):
                    return .done(try await self.readBack(copy, etag: etag, calendarID: ref.calendarID, name: name, originalStart: slot))
                }
            })
    }

    /// What can be refused from the ref alone, before any request. A ref with an occurrence id or a series id belongs
    /// to a series; one with neither is a single event, where the scope is ignored.
    static func checkSeriesRef(_ ref: EventRef, scope: RecurrenceScope, isOccurrence: Bool, movesTime: Bool) throws {
        guard isOccurrence || ref.seriesID != nil else { return }
        if scope != .allInSeries && !isOccurrence { throw WriteError.invalid(seriesMessage) }
        if scope != .allInSeries && ref.originalStart == nil { throw WriteError.invalid(slotMessage) }
        // Moving the whole series from one occurrence needs that occurrence's slot to know how far it moved.
        if scope == .allInSeries && movesTime && ref.originalStart == nil { throw WriteError.unsupported(fields: [.timing]) }
    }

    /// Applies the patch in place and returns the occurrence to read back (nil: the master or single event).
    func edit(_ resource: inout EventResource, ref: EventRef, patch: EventPatch, scope: RecurrenceScope, isOccurrence: Bool,
              calendarZone: TimeZone) throws -> Date? {
        let stamp = now()
        defer {
            if let timing = patch.timing, let zone = timing.timeZone {
                resource.ensureTimeZones([zone], from: timing.start, through: timing.start.addingTimeInterval(20 * 366 * 86_400))
            }
        }
        guard isRecurring(resource) else {
            // The id names an occurrence of a series that has since become a single event (as in `delete`).
            if isOccurrence && scope != .allInSeries { throw WriteError.notFound }
            guard var master = resource.master else { throw WriteError.notFound }
            try EventWriter.apply(patch, to: &master, now: stamp, organizerAddress: organizerAddress, calendarZone: calendarZone)
            resource.setEvents([master])
            return nil
        }
        if scope != .allInSeries && !isOccurrence { throw WriteError.invalid(Self.seriesMessage) }
        if scope != .allInSeries && ref.originalStart == nil { throw WriteError.invalid(Self.slotMessage) }
        if scope == .thisInstance {
            let slot = ref.originalStart!
            if patch.recurrence != .keep { throw WriteError.unsupported(fields: [.recurrence]) }
            guard var override = SeriesEditor.override(in: resource, at: slot, calendarZone: calendarZone) else { throw WriteError.notFound }
            try EventWriter.apply(patch, to: &override, now: stamp, organizerAddress: organizerAddress, calendarZone: calendarZone)
            SeriesEditor.setOverride(override, at: slot, in: &resource, calendarZone: calendarZone)
            return slot
        }
        guard resource.master != nil else {
            // A resource of overrides only (an invite to single occurrences): there is no series to edit, so a
            // whole-series edit applies to each occurrence. One timing cannot describe several occurrences.
            guard scope == .allInSeries, patch.recurrence == .keep else { throw WriteError.unsupported(fields: [.recurrence]) }
            if patch.timing != nil && resource.overrides.count > 1 { throw WriteError.unsupported(fields: [.timing]) }
            var events = resource.overrides
            for index in events.indices { try EventWriter.apply(patch, to: &events[index], now: stamp, organizerAddress: organizerAddress, calendarZone: calendarZone) }
            resource.setEvents(events)
            return nil
        }
        // .allInSeries, or .thisAndFollowing at the first occurrence.
        guard var master = resource.master,
              let timing = EventReader.timing(of: master, resolver: resource.resolver, calendarZone: calendarZone) else { throw WriteError.notFound }
        var masterPatch = patch
        var delta: TimeInterval = 0
        if let wanted = patch.timing {
            // The caller moved one occurrence; the series moves by the same amount, which needs that occurrence's slot.
            guard let slot = ref.originalStart else { throw WriteError.unsupported(fields: [.timing]) }
            let hasExceptions = !resource.overrides.isEmpty || master.property("EXDATE") != nil
            if wanted.isAllDay != timing.isAllDay && hasExceptions { throw WriteError.unsupported(fields: [.timing]) }
            let moved = Self.seriesTiming(master: timing, slot: slot, wanted: wanted, calendarZone: calendarZone)
            masterPatch.timing = moved.timing
            delta = moved.delta
        }
        try EventWriter.apply(masterPatch, to: &master, now: stamp, organizerAddress: organizerAddress, calendarZone: calendarZone)
        resource.setEvents([master] + resource.overrides)
        if delta != 0 { SeriesEditor.shift(&resource, by: delta, calendarZone: calendarZone) }
        if patch.recurrence != .keep { SeriesEditor.pruneUnmatched(&resource, calendarZone: calendarZone) }
        return nil
    }

    /// The master's new timing when the occurrence at `slot` moves to `wanted`: the same shift (whole days for all-day)
    /// and `wanted`'s length and zone.
    static func seriesTiming(master: EventTimingInfo, slot: Date, wanted: EventTiming, calendarZone: TimeZone) -> (timing: EventTiming, delta: TimeInterval) {
        guard wanted.isAllDay else {
            let delta = wanted.start.timeIntervalSince(slot)
            let start = master.start.addingTimeInterval(delta)
            return (EventTiming(start: start, end: start.addingTimeInterval(wanted.end.timeIntervalSince(wanted.start)),
                                timeZone: wanted.timeZone, isAllDay: false), delta)
        }
        let zone = wanted.timeZone ?? calendarZone
        let days = AllDay.date(of: slot, in: zone).days(to: AllDay.date(of: wanted.start, in: zone))
        let length = AllDay.date(of: wanted.start, in: zone).days(to: AllDay.date(of: wanted.end, in: zone))
        let first = AllDay.date(of: master.start, in: zone).adding(days: days)
        let start = AllDay.startOfDay(first, in: zone) ?? master.start
        let end = AllDay.startOfDay(first.adding(days: length), in: zone) ?? start
        return (EventTiming(start: start, end: end, timeZone: zone, isAllDay: true), Double(days) * 86_400)
    }

    // MARK: Delete

    public func delete(_ ref: EventRef, scope: RecurrenceScope, notify: NotifyPolicy) async throws {
        let (name, isOccurrence) = Self.resourceName(of: ref.eventID)
        try Self.checkSeriesRef(ref, scope: scope, isOccurrence: isOccurrence, movesTime: false)
        let zone = try await calendarZone(ref.calendarID)
        // An occurrence delete is idempotent and local to that occurrence, so a 412 (another occurrence changed) is
        // re-applied to the fresh copy. Every attempt judges the fresh copy again: who would be told, and whether the
        // occurrence still exists. From the second attempt on, a resource or occurrence that is gone is the goal reached.
        for attempt in 0..<3 {
            do {
                let current = try await fetch(calendarID: ref.calendarID, name: name)
                if try await deleteAttempt(current, ref: ref, scope: scope, isOccurrence: isOccurrence, notify: notify, zone: zone) { return }
            } catch WriteError.notFound where attempt > 0 {
                return
            }
        }
        throw WriteError.conflict(fields: [.recurrence])
    }

    /// One delete against `current`: true when done, false when the server said the copy is stale.
    private func deleteAttempt(_ current: FetchedResource, ref: EventRef, scope: RecurrenceScope, isOccurrence: Bool,
                               notify: NotifyPolicy, zone: TimeZone) async throws -> Bool {
        try requireNotify(notify, tellsOthers: tellsOthers(current.resource))
        let recurring = isRecurring(current.resource)
        // The id names an occurrence of a series that has since become a single event: that occurrence no longer exists.
        if isOccurrence && !recurring && scope != .allInSeries { throw WriteError.notFound }
        var effective = recurring ? scope : .allInSeries
        if effective != .allInSeries {
            guard isOccurrence else { throw WriteError.invalid(Self.seriesMessage) }
            guard let slot = ref.originalStart else { throw WriteError.invalid(Self.slotMessage) }
            if effective == .thisAndFollowing, let first = masterStart(current.resource, calendarZone: zone), slot <= first { effective = .allInSeries }
        }
        if effective == .allInSeries {
            // Last writer wins, like the other connectors' deletes.
            return try await remove(current.url, ifMatch: nil)
        }
        let slot = ref.originalStart!
        var resource = current.resource
        if resource.master == nil {
            // A resource of overrides only: drop the occurrence (and, for this and following, the later ones).
            let resolver = resource.resolver
            let remaining = resource.overrides.filter { vevent in
                guard let id = EventReader.recurrenceID(of: vevent, resolver: resolver, calendarZone: zone) else { return true }
                return effective == .thisInstance ? id != slot : id < slot
            }
            if remaining.count == resource.overrides.count { throw WriteError.notFound }
            if remaining.isEmpty { return try await remove(current.url, ifMatch: current.etag) }
            resource.setEvents(remaining)
        } else if effective == .thisInstance {
            try SeriesEditor.exclude(slot, in: &resource, calendarZone: zone)
            if var master = resource.master {
                EventWriter.touch(&master, now: now(), bumpSequence: true)
                resource.setEvents([master] + resource.overrides)
            }
        } else {
            resource = try SeriesEditor.split(resource, at: slot, newUID: makeUUID(), calendarZone: zone, now: now()).head
        }
        if case .stored = try await put(resource, to: current.url, ifMatch: current.etag) { return true }
        return false
    }

    /// DELETE; false on a 412 (only when `ifMatch` was sent).
    private func remove(_ url: URL, ifMatch: String?) async throws -> Bool {
        let reply = try await client.send("DELETE", url, headers: ifMatch.map { ["If-Match": $0] } ?? [:])
        switch reply.response.status {
        case 200, 204: await state.forget(url); return true
        case 412: return false
        case 404, 410: throw WriteError.notFound
        case 403: throw WriteError.forbidden(nil)
        default: throw SourceError.invalidResponse("DELETE answered \(reply.response.status)")
        }
    }

    // MARK: Respond

    public func respond(to ref: EventRef, _ response: ResponseStatus, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent {
        guard response != .needsAction else { throw WriteError.invalid("a response must be accepted, tentative or declined") }
        guard capabilities.canRespondToInvite else { throw WriteError.unsupported(fields: [.attendees]) }
        // The organizer is always told of an answer.
        guard notify == .all else { throw WriteError.unsupported(fields: [.attendees]) }
        let (name, isOccurrence) = Self.resourceName(of: ref.eventID)
        // An attendee cannot split the organizer's series.
        if scope == .thisAndFollowing && (isOccurrence || ref.seriesID != nil) { throw WriteError.unsupported(fields: [.attendees]) }
        let zone = try await calendarZone(ref.calendarID)
        return try await PatchMerge.apply(
            patch: EventPatch(), version: ref.version,
            fetchCurrent: { try self.comparedEvent(try await self.fetch(calendarID: ref.calendarID, name: name), ref: ref, scope: scope, calendarZone: zone) },
            write: { version in
                let current = try await self.fetch(calendarID: ref.calendarID, name: name)
                if let version, version != current.etag { return .stale }
                var resource = current.resource
                let slot = try self.setResponse(response, in: &resource, ref: ref, scope: scope, isOccurrence: isOccurrence, calendarZone: zone)
                switch try await self.put(resource, to: current.url, ifMatch: current.etag) {
                case .stale: return .stale
                case .stored(let etag, let copy):
                    return .done(try await self.readBack(copy, etag: etag, calendarID: ref.calendarID, name: name, originalStart: slot))
                }
            })
    }

    private func setResponse(_ response: ResponseStatus, in resource: inout EventResource, ref: EventRef, scope: RecurrenceScope,
                             isOccurrence: Bool, calendarZone: TimeZone) throws -> Date? {
        let stamp = now()
        let me = selfAddresses
        guard isRecurring(resource) else {
            // The id names an occurrence of a series that has since become a single event (as in `delete`).
            if isOccurrence && scope != .allInSeries { throw WriteError.notFound }
            guard var master = resource.master, AttendeeMapper.setResponse(response, in: &master, selfAddresses: me) else {
                throw WriteError.unsupported(fields: [.attendees])
            }
            EventWriter.touch(&master, now: stamp, bumpSequence: false)
            resource.setEvents([master])
            return nil
        }
        switch scope {
        case .thisAndFollowing:
            throw WriteError.unsupported(fields: [.attendees])
        case .thisInstance:
            guard isOccurrence else { throw WriteError.invalid(Self.seriesMessage) }
            guard let slot = ref.originalStart else { throw WriteError.invalid(Self.slotMessage) }
            guard var override = SeriesEditor.override(in: resource, at: slot, calendarZone: calendarZone) else { throw WriteError.notFound }
            guard AttendeeMapper.setResponse(response, in: &override, selfAddresses: me) else { throw WriteError.unsupported(fields: [.attendees]) }
            EventWriter.touch(&override, now: stamp, bumpSequence: false)
            SeriesEditor.setOverride(override, at: slot, in: &resource, calendarZone: calendarZone)
            return slot
        case .allInSeries:
            // The answer is for the whole series: the master and every override.
            var events = resource.events
            var found = false
            for index in events.indices where AttendeeMapper.setResponse(response, in: &events[index], selfAddresses: me) {
                EventWriter.touch(&events[index], now: stamp, bumpSequence: false)
                found = true
            }
            guard found else { throw WriteError.unsupported(fields: [.attendees]) }
            resource.setEvents(events)
            return nil
        }
    }
}
