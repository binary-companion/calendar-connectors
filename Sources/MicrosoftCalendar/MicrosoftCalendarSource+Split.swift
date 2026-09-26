import CalendarCore
import Foundation

extension MicrosoftCalendarSource {
    /// Occurrences of the series that start before `split`, counted from the master's own start. A deleted occurrence
    /// still counts toward a `numbered` range, but Graph's `instances` may not list it, so this can undercount.
    private func priorInstances(calendarID: String, masterID: String, from start: Date, before split: Date) async throws -> Int {
        let url = api.url(
            path: GraphAPIClient.eventPath(calendarID, masterID) + "/instances",
            query: [
                URLQueryItem(name: "startDateTime", value: GraphTime.instantText(start.addingTimeInterval(-86_400))),
                URLQueryItem(name: "endDateTime", value: GraphTime.instantText(split)),
                URLQueryItem(name: "$select", value: "id"), URLQueryItem(name: "$top", value: "100"),
            ])
        var count = 0
        try await api.pages(GraphListPage<GraphIDDTO>.self, from: url) { count += $0.items.count }
        return count
    }

    private func patchRecurrence(_ calendarID: String, _ eventID: String, _ recurrence: [String: Any]) async throws {
        try await patchRecurrence(calendarID, eventID, body: try GraphWriteMapper.data(["recurrence": recurrence]))
    }

    private func patchRecurrence(_ calendarID: String, _ eventID: String, body: Data) async throws {
        _ = try await api.send(method: "PATCH", url: api.url(path: GraphAPIClient.eventPath(calendarID, eventID)), body: body)
    }

    /// Runs `patchRecurrence` in a task of its own, so it completes even if the caller was cancelled.
    private func patchRecurrenceUncancelled(_ calendarID: String, _ eventID: String, _ recurrence: [String: Any]) async throws {
        let body = try GraphWriteMapper.data(["recurrence": recurrence])
        try await Task { try await self.patchRecurrence(calendarID, eventID, body: body) }.value
    }

    /// Puts the master's original recurrence back after a failed insert. Runs even when the caller was cancelled (a
    /// cancelled request would otherwise fail at once and leave the series cut off). If it fails too the result is
    /// `WriteError.partial`, which names the insert's error.
    private func restoreRecurrence(_ calendarID: String, _ masterID: String, _ original: [String: Any], after cause: Error) async throws {
        do {
            try await patchRecurrenceUncancelled(calendarID, masterID, original)
        } catch {
            throw WriteError.partial(
                "the series was cut off before this occurrence but the new series could not be created (\(cause)), and restoring the original recurrence failed (\(error))")
        }
    }

    /// Restores the master after a truncation whose reply was lost; a successful restore leaves the caller to rethrow the
    /// truncation's own error.
    private func restoreTruncation(_ calendarID: String, _ masterID: String, _ original: [String: Any], after cause: Error) async throws {
        do {
            try await patchRecurrenceUncancelled(calendarID, masterID, original)
        } catch {
            throw WriteError.partial(
                "cutting the series off before this occurrence failed with an unknown outcome (\(cause)), and restoring the original recurrence failed (\(error)); check the calendar")
        }
    }

    /// Whether a failed request may nevertheless have been applied: the reply was lost (a 5xx, a broken connection, a
    /// cancellation) rather than the request refused. A 409 counts too: the insert carries a `transactionId` of ours
    /// alone, so a conflict can only mean an earlier attempt of the same POST went through.
    static func mightHaveApplied(_ error: Error) -> Bool {
        if let api = error as? GraphAPIError { return api == .conflict }
        if error is WriteError { return false }
        if let source = error as? SourceError {
            switch source {
            case .server, .network: return true
            default: return false
            }
        }
        return true
    }

    /// `.thisAndFollowing`: cut the master's recurrence just before the occurrence. Delete (`patch == nil`) stops there and
    /// returns nil. Update also inserts a new series from the occurrence with the patch applied. Splitting at the first
    /// occurrence is the same as `.allInSeries`. `NotifyPolicy` does not apply (Graph decides who is told).
    ///
    /// Failure handling for update: everything that can fail without changing anything happens before the truncation.
    /// The insert carries a `transactionId`, so if its reply is lost it is sent once more and the server returns the first
    /// result instead of a second event. If the insert fails for good, the master's original recurrence is restored, and
    /// if restoring fails too the result is `WriteError.partial`. Once the insert has succeeded it is never undone.
    func splitSeries(_ ref: EventRef, calendar: CalendarDescriptor, patch: EventPatch?, notify: NotifyPolicy) async throws -> CalendarEvent? {
        guard let masterID = ref.seriesID, let split = ref.originalStart else {
            throw WriteError.invalid("this and following needs the occurrence's original start")
        }
        let master = try await fetchRaw(ref.calendarID, masterID)
        let zone = try await accountZone(refresh: false)
        let masterDTO = try api.decode(GraphEventDTO.self, from: master.data)
        guard let first = GraphEventMapper.resolve(masterDTO, fallbackZone: zone.zone) else {
            throw SourceError.invalidResponse("microsoft: unreadable series")
        }
        if abs(first.start.timeIntervalSince(split)) < 1 {
            if let patch { return try await update(ref, patch, scope: .allInSeries, notify: notify) }
            try await delete(ref, scope: .allInSeries, notify: notify)
            return nil
        }
        guard let original = master.json["recurrence"] as? [String: Any], original["pattern"] != nil else {
            throw WriteError.invalid("the series has no recurrence rule")
        }
        let range = original["range"] as? [String: Any]
        let rangeZone = (range?["recurrenceTimeZone"] as? String).flatMap(WindowsTimeZones.timeZone(for:)) ?? zone.zone
        let splitDate = AllDay.date(of: split, in: rangeZone)
        let truncated = GraphRecurrenceMapper.truncated(original, endingBefore: splitDate)

        var newSeries: [String: Any]?
        if let patch {
            newSeries = try await newSeriesBody(
                ref, master: master, first: first, original: original, split: split, splitDate: splitDate, rangeZone: rangeZone,
                patch: patch, calendar: calendar)
        }
        do {
            try await patchRecurrence(ref.calendarID, masterID, truncated)
        } catch {
            // A delete stops here and repeating the truncation is harmless. An update goes on to insert, so a truncation whose
            // reply was lost (it may have been applied) must not stay half done: put the original back, so the caller may
            // retry, and report the original error. Definite failures (a 400, a 403, ...) changed nothing.
            if newSeries != nil, Self.mightHaveApplied(error) { try await restoreTruncation(ref.calendarID, masterID, original, after: error) }
            throw error
        }
        guard let newSeries else { return nil }

        let insertData = try GraphWriteMapper.data(newSeries)
        let zonePreferences = zone.readPreferences
        let insertURL = api.url(path: GraphAPIClient.calendarPath(ref.calendarID, "/events"))
        let created: Data
        do {
            created = try await api.send(method: "POST", url: insertURL, body: insertData, prefer: zonePreferences)
        } catch {
            guard Self.mightHaveApplied(error) else {
                try await restoreRecurrence(ref.calendarID, masterID, original, after: error)
                throw error
            }
            // The reply may have been lost after the insert applied: send it once more. The same `transactionId` makes the
            // server answer with the event it already made; a second event is not created.
            do {
                created = try await api.send(method: "POST", url: insertURL, body: insertData, prefer: zonePreferences)
            } catch let retryError {
                // Neither restoring (would duplicate the series if the insert applied) nor leaving it is safe to do blindly.
                throw WriteError.partial(
                    "the series was cut off before this occurrence and the outcome of creating the new series is unknown (\(error); retry: \(retryError)); check the calendar")
            }
        }
        return try mapped(created, calendar: calendar)
    }

    /// The insert body for the series that continues after the split. Everything that can fail without changing anything
    /// (the occurrence, the count of earlier occurrences, validating the patch) is done here, before the master is touched.
    private func newSeriesBody(
        _ ref: EventRef, master: RawEvent, first: GraphEventMapper.Resolved, original: [String: Any], split: Date,
        splitDate: CalendarDate, rangeZone: TimeZone, patch: EventPatch, calendar: CalendarDescriptor
    ) async throws -> [String: Any] {
        let instance = try await fetchRaw(ref.calendarID, ref.eventID)
        // The new series is built from the master and starts at the occurrence's original slot, so the occurrence's own
        // copy is read only to judge staleness like any other update.
        if let version = ref.version, let key = instance.changeKey, key != version {
            let overlapping = PatchMerge.conflicts(patch: patch, current: try mapped(instance.data, calendar: calendar))
            if !overlapping.isEmpty { throw WriteError.conflict(fields: overlapping) }
        }
        var remaining: Int?
        if let total = GraphRecurrenceMapper.occurrenceCount(in: original) {
            let masterID = ref.seriesID ?? ref.eventID
            let prior = try await priorInstances(calendarID: ref.calendarID, masterID: masterID, from: first.start, before: split)
            guard total - prior >= 1 else { throw WriteError.invalid("the series has no occurrences left to split off") }
            remaining = total - prior
        }
        var body = GraphWriteMapper.newSeriesBase(from: master.json)
        let slot = EventTiming(start: split, end: split.addingTimeInterval(first.end.timeIntervalSince(first.start)), timeZone: first.zone, isAllDay: first.isAllDay)
        for (key, value) in try GraphWriteMapper.timeFields(slot) { body[key] = value }
        let startDate = patch.timing.map { AllDay.date(of: $0.start, in: $0.timeZone ?? rangeZone) } ?? splitDate
        body["recurrence"] = GraphRecurrenceMapper.restarted(original, at: startDate, remaining: remaining)
        // The patch overrides what the master carried (including times, recurrence and the attendee list, which starts
        // from the master's guests).
        let anchor = RecurrenceAnchor(start: startDate, zone: patch.timing?.timeZone ?? rangeZone)
        for (key, value) in try GraphWriteMapper.patchBody(patch, currentAttendees: master.attendees, anchor: anchor) { body[key] = value }
        body["transactionId"] = UUID().uuidString.lowercased()
        return body
    }
}
