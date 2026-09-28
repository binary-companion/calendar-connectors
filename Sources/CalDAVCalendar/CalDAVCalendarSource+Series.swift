import CalendarCore
import Foundation
import ICalendar

extension CalDAVCalendarSource: SeriesSource {
    /// `id` is the resource name (`SeriesInfo.occurrence(seriesID:)`). A resource that is missing or does not recur is
    /// `SourceError.notFound`.
    public func series(id: String, calendarID: String) async throws -> CalendarSeries {
        let reply = try await client.send("GET", resourceURL(calendarID: calendarID, name: id))
        switch reply.response.status {
        case 200: break
        case 403, 404, 410: throw SourceError.notFound
        default: throw SourceError.invalidResponse("GET answered \(reply.response.status)")
        }
        guard let resource = try? EventResource(data: reply.response.body) else { throw SourceError.invalidResponse("the event could not be read") }
        let context = context(calendarID: calendarID, zone: try await calendarZone(calendarID), resourceName: id, etag: reply.response.header("ETag"))
        guard let series = EventReader.series(of: resource, context: context) else { throw SourceError.notFound }
        return series
    }
}
