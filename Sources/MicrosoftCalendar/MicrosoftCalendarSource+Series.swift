import CalendarCore
import Foundation

extension MicrosoftCalendarSource: SeriesSource {
    /// The series master's rule and anchor time. Reads expand series, so an instance's `seriesID` is the master's
    /// event id. An unknown id, and an id that is not a recurring series master, are `SourceError.notFound`.
    /// Skipped and extra dates (`excludedDates`, `extraDates`) are nil: Graph v1.0 does not list them on the master.
    public func series(id: String, calendarID: String) async throws -> CalendarSeries {
        let zone = try await accountZone(refresh: false)
        do {
            let data = try await api.get(url: api.url(path: GraphAPIClient.eventPath(calendarID, id)), prefer: zone.readPreferences)
            let dto = try api.decode(GraphEventDTO.self, from: data)
            guard dto.type == "seriesMaster", dto.isCancelled != true, let recurrence = dto.recurrence,
                  let resolved = GraphEventMapper.resolve(dto, fallbackZone: zone.zone) else { throw SourceError.notFound }
            let rule = GraphRecurrenceMapper.rule(from: recurrence, in: resolved.zone)
            let unknown = rule == nil ? ["X-MS-RECURRENCE:\(recurrence.pattern?.type ?? "unknown")"] : []
            return CalendarSeries(
                seriesID: id, calendarID: calendarID, start: resolved.start, timeZone: resolved.zone, isAllDay: resolved.isAllDay,
                recurrence: RecurrenceSet(rules: rule.map { [$0] } ?? [], extraDates: nil, excludedDates: nil, unparsed: unknown))
        } catch let error as GraphAPIError {
            if error == .notFound || error == .gone { throw SourceError.notFound }
            throw error.sourceError
        }
    }
}
