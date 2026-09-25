import CalendarCore
import Foundation

extension GoogleCalendarSource: SeriesSource {
    /// The series master's rule lines (`RRULE`, `EXDATE`, `RDATE`, ...) and anchor time. Reads expand series, so an
    /// instance's `seriesID` is the master's event id. An id that is not a recurring series (no `recurrence`, or a
    /// cancelled master) and an unknown id are `SourceError.notFound`.
    public func series(id: String, calendarID: String) async throws -> CalendarSeries {
        do {
            let data = try await api.get(
                path: GoogleAPIClient.eventPath(calendarID, id),
                query: [URLQueryItem(name: "fields", value: "id,status,recurrence,start,end")])
            let dto = try api.decode(GoogleEventDTO.self, from: data)
            guard dto.status != "cancelled", let lines = dto.recurrence, !lines.isEmpty else { throw SourceError.notFound }
            let calendarZone = try await zone(ofCalendar: calendarID)
            guard let start = GoogleEventMapper.resolve(dto.start, calendarZone: calendarZone) else {
                throw SourceError.invalidResponse("google: unreadable series")
            }
            let zone = start.zone ?? calendarZone
            return CalendarSeries(
                seriesID: id, calendarID: calendarID, start: start.date, timeZone: zone, isAllDay: start.isAllDay,
                recurrence: RecurrenceSet(iCalendarLines: lines, timeZone: zone, isAllDay: start.isAllDay))
        } catch let error as GoogleAPIError {
            if error == .notFound || error == .gone { throw SourceError.notFound }
            throw error.sourceError
        }
    }

    private func zone(ofCalendar calendarID: String) async throws -> TimeZone {
        let calendars: [CalendarDescriptor]
        if let stored = await calendarList.list { calendars = stored } else { calendars = try await self.calendars() }
        return calendars.first { $0.id == calendarID }?.timeZone ?? TimeZone(identifier: "UTC")!
    }
}
