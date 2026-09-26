import Foundation

/// Decodes one array element without ever throwing, so a single malformed item is dropped (`value == nil`)
/// instead of failing the whole page.
struct LenientItem<T: Decodable>: Decodable {
    let value: T?
    init(from decoder: Decoder) throws { value = try? T(from: decoder) }
}

/// A Graph collection page. Items decode leniently; `deltaLink` is present on the last page of a delta query.
struct GraphListPage<Item: Decodable>: GraphPage {
    var value: [LenientItem<Item>]?
    var nextLink: String?
    var deltaLink: String?

    enum CodingKeys: String, CodingKey {
        case value
        case nextLink = "@odata.nextLink"
        case deltaLink = "@odata.deltaLink"
    }

    var items: [Item] { (value ?? []).compactMap(\.value) }
}

/// Only the id: a delta page is counted, never mapped.
struct GraphIDDTO: Decodable { var id: String? }

/// `{ "value": "..." }`, the shape of a single-property response such as `/me/mailboxSettings/timeZone`.
struct GraphValueDTO: Decodable { var value: String? }

struct GraphEmailDTO: Decodable {
    var name: String?
    var address: String?
}

struct GraphCalendarDTO: Decodable {
    var id: String
    var name: String?
    var hexColor: String?
    var isDefaultCalendar: Bool?
    var canEdit: Bool?
    var canShare: Bool?
    var canViewPrivateItems: Bool?
    var owner: GraphEmailDTO?
}

struct GraphTimeDTO: Decodable {
    var dateTime: String?
    var timeZone: String?
}

struct GraphAttendeeDTO: Decodable {
    struct Status: Decodable { var response: String? }
    var type: String?
    var status: Status?
    var emailAddress: GraphEmailDTO?
}

struct GraphRecipientDTO: Decodable { var emailAddress: GraphEmailDTO? }

struct GraphBodyDTO: Decodable {
    var contentType: String?
    var content: String?
}

struct GraphLocationDTO: Decodable { var displayName: String? }

struct GraphOnlineMeetingDTO: Decodable { var joinUrl: String? }

struct GraphRecurrenceDTO: Decodable {
    struct Pattern: Decodable {
        var type: String?
        var interval: Int?
        var month: Int?
        var dayOfMonth: Int?
        var daysOfWeek: [String]?
        var firstDayOfWeek: String?
        var index: String?
    }
    struct Range: Decodable {
        var type: String?
        var startDate: String?
        var endDate: String?
        var recurrenceTimeZone: String?
        var numberOfOccurrences: Int?
    }
    var pattern: Pattern?
    var range: Range?
}

struct GraphEventDTO: Decodable {
    struct ResponseStatus: Decodable { var response: String? }
    var id: String
    var iCalUId: String?
    var subject: String?
    var body: GraphBodyDTO?
    var location: GraphLocationDTO?
    var start: GraphTimeDTO?
    var end: GraphTimeDTO?
    var isAllDay: Bool?
    var isCancelled: Bool?
    var showAs: String?
    var sensitivity: String?
    /// `singleInstance`, `occurrence`, `exception` or `seriesMaster`.
    var type: String?
    var seriesMasterId: String?
    var originalStart: String?
    var originalStartTimeZone: String?
    var attendees: [GraphAttendeeDTO]?
    var organizer: GraphRecipientDTO?
    var isOrganizer: Bool?
    var responseStatus: ResponseStatus?
    var isReminderOn: Bool?
    var reminderMinutesBeforeStart: Int?
    var isOnlineMeeting: Bool?
    var onlineMeeting: GraphOnlineMeetingDTO?
    var changeKey: String?
    var lastModifiedDateTime: String?
    var createdDateTime: String?
    var webLink: String?
    var recurrence: GraphRecurrenceDTO?
}
