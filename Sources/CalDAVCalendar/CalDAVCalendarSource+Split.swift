import CalendarCore
import Foundation
import ICalendar

extension CalDAVCalendarSource {
    /// What became of the write that cuts the original series short.
    private enum HeadOutcome {
        /// The truncation is on the server under `etag`.
        case stored(etag: String, resource: EventResource)
        /// 412: the copy read is out of date; nothing was written.
        case stale
        /// The write failed and a look at the resource shows it is untouched.
        case notApplied(Error)
        /// The write failed but the truncation is on the server (`etag`): the request's outcome was unclear.
        case appliedDespiteError(etag: String, resource: EventResource, cause: Error)
        /// The write failed and the resource now holds someone else's edit.
        case changedByOthers(Error)
        /// The write failed and the resource could not be looked at.
        case unknown(cause: Error, lookup: Error)
    }

    /// What became of the write that creates the new series.
    private enum TailOutcome {
        case stored(etag: String, resource: EventResource)
        /// Definitely not on the server.
        case absent(Error)
        /// The write failed and the server could not be asked whether it went through.
        case unknown(cause: Error, lookup: Error)
    }

    /// Whether a failed request may nevertheless have been applied: the reply was lost (a 5xx, a broken connection, a
    /// cancellation) or the read-back after a 2xx failed, rather than the request refused. Refusals (`WriteError`, an
    /// authentication or rate-limit answer) changed nothing.
    static func mightHaveApplied(_ error: Error) -> Bool {
        if error is WriteError { return false }
        if let source = error as? SourceError {
            switch source {
            case .authExpired, .rateLimited, .needsPermission, .notFound: return false
            case .network, .server, .invalidResponse: return true
            }
        }
        return true
    }

    /// `.thisAndFollowing` after the first occurrence: (1) the original resource keeps the occurrences before `slot`
    /// (`If-Match` its ETag), (2) a new resource with a new UID holds the rest with the patch applied, carrying the
    /// later overrides and exclusions. If (2) fails, the original body goes back with `If-Match` on the ETag (1)
    /// returned, even when the caller was cancelled; a 412 there (someone edited meanwhile) is not overwritten and the
    /// result is `WriteError.partial`. A write whose outcome is unclear (lost reply, cancellation, failed read-back) is
    /// looked up before anything is restored, so a step that did go through is never duplicated or undone blindly.
    func splitSeries(_ current: FetchedResource, ref: EventRef, patch: EventPatch, slot: Date, calendarZone: TimeZone) async throws -> PatchMerge.Attempt<CalendarEvent> {
        // An attendee cannot split the organizer's series: the new UID would be unknown to the organizer, whose next
        // update would bring the whole series back next to it (as for `respond`).
        if let master = current.resource.master,
           let organizer = AttendeeMapper.read(master, selfAddresses: selfAddresses).organizer, !organizer.isSelf {
            throw WriteError.unsupported(fields: [.attendees])
        }
        let stamp = now()
        let newUID = makeUUID()
        let newName = makeUUID() + ".ics"
        let parts = try SeriesEditor.split(current.resource, at: slot, newUID: newUID, calendarZone: calendarZone, now: stamp)
        let head = parts.head
        var tail = parts.tail
        guard var tailMaster = tail.master,
              let before = EventReader.timing(of: tailMaster, resolver: tail.resolver, calendarZone: calendarZone) else { throw WriteError.notFound }
        // Carried exceptions and exclusions cannot follow a switch between all-day and timed (as in `edit`).
        if let wanted = patch.timing, wanted.isAllDay != before.isAllDay,
           !tail.overrides.isEmpty || tailMaster.property("EXDATE") != nil {
            throw WriteError.unsupported(fields: [.timing])
        }
        var tailPatch = patch
        var delta: TimeInterval = 0
        if let wanted = patch.timing {
            let moved = Self.seriesTiming(master: before, slot: slot, wanted: wanted, calendarZone: calendarZone)
            tailPatch.timing = moved.timing
            delta = moved.delta
        }
        try EventWriter.apply(tailPatch, to: &tailMaster, now: stamp, organizerAddress: organizerAddress, calendarZone: calendarZone)
        tailMaster.set(ICalProperty(name: "SEQUENCE", value: "0"))
        // The carried exceptions are new components under a new UID: stamp them like the master (SeriesEditor.split
        // moves them across without touching DTSTAMP/LAST-MODIFIED).
        let carried = tail.overrides.map { vevent -> ICalComponent in
            var copy = vevent
            EventWriter.touch(&copy, now: stamp, bumpSequence: false)
            return copy
        }
        tail.setEvents([tailMaster] + carried)
        if delta != 0 { SeriesEditor.shift(&tail, by: delta, calendarZone: calendarZone) }
        if patch.recurrence != .keep { SeriesEditor.pruneUnmatched(&tail, calendarZone: calendarZone) }
        if let timing = patch.timing, let zone = timing.timeZone {
            tail.ensureTimeZones([zone], from: timing.start, through: timing.start.addingTimeInterval(20 * 366 * 86_400))
        }

        let truncated: (etag: String, resource: EventResource)
        switch await truncate(head, over: current, calendarID: ref.calendarID) {
        case .stale: return .stale
        case .stored(let etag, let resource): truncated = (etag, resource)
        case .notApplied(let error): throw error
        case .appliedDespiteError(let etag, let resource, let cause):
            // The truncation is on the server although the request reported an error: put the series back and report the
            // error, so the caller may retry from a whole series.
            try await restoreSeries(current, over: (etag, resource), cause: cause,
                                    why: "the series was cut short and the request reported an error")
        case .changedByOthers(let cause):
            throw WriteError.partial("cutting the series short failed with an unclear outcome (\(cause)) and the series was changed meanwhile; check the calendar")
        case .unknown(let cause, let lookup):
            throw WriteError.partial("cutting the series short failed with an unknown outcome (\(cause)) and looking at the series failed (\(lookup)); check the calendar")
        }

        switch await createTail(tail, uid: newUID, calendarID: ref.calendarID, name: newName) {
        case .stored(let etag, let copy):
            return .done(try await readBack(copy, etag: etag, calendarID: ref.calendarID, name: newName, originalStart: nil))
        case .absent(let cause):
            try await restoreSeries(current, over: truncated, cause: cause, why: "the series was cut short but the new series was not created")
        case .unknown(let cause, let lookup):
            // Restoring would duplicate the series if the new one exists, and leaving it loses occurrences if it does not.
            throw WriteError.partial("the series was cut short and the outcome of creating the new series is unknown (\(cause); lookup: \(lookup)); check the calendar")
        }
    }

    /// PUTs the truncated original under `If-Match`. After a failure that may have been applied, looks at the resource:
    /// the ETag read earlier means nothing was written; our own stamp means the truncation is there.
    private func truncate(_ head: EventResource, over current: FetchedResource, calendarID: String) async -> HeadOutcome {
        do {
            switch try await put(head, to: current.url, ifMatch: current.etag) {
            case .stale: return .stale
            case .stored(let etag, let resource): return .stored(etag: etag, resource: resource)
            }
        } catch {
            guard Self.mightHaveApplied(error) else { return .notApplied(error) }
            // An unstructured task does not inherit the caller's cancellation.
            let lookup = await Task { try await self.fetch(calendarID: calendarID, name: current.name) }.result
            switch lookup {
            case .failure(let failure): return .unknown(cause: error, lookup: failure)
            case .success(let fresh):
                if fresh.etag == current.etag { return .notApplied(error) }
                // Ours only if every component is as we wrote it: another client's edit to a kept override (or a server
                // applying an attendee's reply) must not be mistaken for our truncation and restored over.
                let ours = Self.fingerprint(fresh.resource) == Self.fingerprint(head)
                return ours ? .appliedDespiteError(etag: fresh.etag, resource: fresh.resource, cause: error) : .changedByOthers(error)
            }
        }
    }

    /// One line per component: its `RECURRENCE-ID` (empty for the master), `DTSTAMP`, `SEQUENCE` and, for the master, `RRULE`.
    private static func fingerprint(_ resource: EventResource) -> [String] {
        resource.events.map { vevent in
            ["RECURRENCE-ID", "DTSTAMP", "SEQUENCE", "RRULE"].map { vevent.property($0)?.value ?? "" }.joined(separator: "|")
        }.sorted()
    }

    /// PUTs the new series under `If-None-Match: *`. After a failure that may have been applied, asks the server for the
    /// name (it is ours alone): found means the write went through.
    private func createTail(_ tail: EventResource, uid: String, calendarID: String, name: String) async -> TailOutcome {
        do {
            switch try await put(tail, to: resourceURL(calendarID: calendarID, name: name), ifNoneMatch: true) {
            case .stored(let etag, let copy): return .stored(etag: etag, resource: copy)
            case .stale: return .absent(SourceError.invalidResponse("a resource with the new series' name already exists"))
            }
        } catch {
            guard Self.mightHaveApplied(error) else { return .absent(error) }
            let lookup = await Task { try await self.fetch(calendarID: calendarID, name: name) }.result
            switch lookup {
            case .success(let found): return found.resource.uid == uid ? .stored(etag: found.etag, resource: found.resource) : .absent(error)
            case .failure(WriteError.notFound): return .absent(error)
            case .failure(let failure): return .unknown(cause: error, lookup: failure)
            }
        }
    }

    /// Puts the original series back over the truncation (`If-Match` the truncation's ETag) and throws `cause`; a 412
    /// (someone edited the truncated series meanwhile) is not overwritten and any other failure of the restore is
    /// `WriteError.partial`. The restored master carries a higher `SEQUENCE` than the truncation and a new `DTSTAMP`:
    /// attendees' apps ignore an update that is not newer than the cut-short copy they already hold.
    private func restoreSeries(_ current: FetchedResource, over truncated: (etag: String, resource: EventResource), cause: Error, why: String) async throws -> Never {
        var restored = current.resource
        let sequence = (truncated.resource.master?.property("SEQUENCE").flatMap { Int($0.value) } ?? 0) + 1
        let stamp = now()
        let events = restored.events.map { vevent -> ICalComponent in
            var copy = vevent
            EventWriter.touch(&copy, now: stamp, bumpSequence: false)
            if copy.property("RECURRENCE-ID") == nil { copy.set(ICalProperty(name: "SEQUENCE", value: String(sequence))) }
            return copy
        }
        restored.setEvents(events)
        let url = current.url
        let result = await Task { [restored] in try await self.put(restored, to: url, ifMatch: truncated.etag) }.result
        switch result {
        case .success(.stored):
            throw cause
        case .success(.stale):
            throw WriteError.partial("\(why) (\(cause)); it was changed meanwhile, so it was not restored")
        case .failure(let restoreError):
            throw WriteError.partial("\(why) (\(cause)); restoring it failed (\(restoreError))")
        }
    }
}
