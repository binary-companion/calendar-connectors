import Foundation

struct GoogleTimeDTO: Decodable {
    var date: String?
    var dateTime: String?
    var timeZone: String?
}

struct GoogleAttendeeDTO: Decodable {
    var email: String?
    var displayName: String?
    var responseStatus: String?
    var optional: Bool?
    var resource: Bool?
    var organizer: Bool?
    var isSelf: Bool?

    enum CodingKeys: String, CodingKey {
        case email, displayName, responseStatus, optional, resource, organizer
        case isSelf = "self"
    }
}

struct GoogleOrganizerDTO: Decodable {
    var email: String?
    var displayName: String?
    var isSelf: Bool?

    enum CodingKeys: String, CodingKey {
        case email, displayName
        case isSelf = "self"
    }
}

struct GoogleConferenceDTO: Decodable {
    struct EntryPoint: Decodable {
        var entryPointType: String?
        var uri: String?
    }
    struct Solution: Decodable {
        struct Key: Decodable { var type: String? }
        var key: Key?
        var name: String?
    }
    var entryPoints: [EntryPoint]?
    var conferenceSolution: Solution?
}

struct GoogleRemindersDTO: Decodable {
    struct Override: Decodable { var minutes: Int? }
    var useDefault: Bool?
    var overrides: [Override]?
}

struct GoogleEventDTO: Decodable {
    var id: String
    var iCalUID: String?
    var status: String?
    var summary: String?
    var description: String?
    var location: String?
    var htmlLink: String?
    var etag: String?
    var updated: String?
    var created: String?
    var hangoutLink: String?
    var transparency: String?
    var visibility: String?
    var eventType: String?
    var recurringEventId: String?
    /// Present on a series master only (reads expand series, so they never see it); the write path uses it.
    var recurrence: [String]?
    var start: GoogleTimeDTO?
    var end: GoogleTimeDTO?
    var originalStartTime: GoogleTimeDTO?
    var attendees: [GoogleAttendeeDTO]?
    var organizer: GoogleOrganizerDTO?
    var conferenceData: GoogleConferenceDTO?
    var reminders: GoogleRemindersDTO?
}

/// Decodes one array element without ever throwing, so a single malformed item is dropped (`value == nil`)
/// instead of failing the whole page.
struct LenientItem<T: Decodable>: Decodable {
    let value: T?
    init(from decoder: Decoder) throws { value = try? T(from: decoder) }
}

struct GoogleEventsPageDTO: Decodable {
    var items: [LenientItem<GoogleEventDTO>]?
    var nextPageToken: String?
}

struct GoogleCalendarListEntryDTO: Decodable {
    var id: String
    var summary: String?
    var summaryOverride: String?
    var backgroundColor: String?
    var accessRole: String?
    var primary: Bool?
    var timeZone: String?
    var hidden: Bool?
    var deleted: Bool?
    struct DefaultReminder: Decodable { var minutes: Int? }
    var defaultReminders: [DefaultReminder]?
}

struct GoogleCalendarListPageDTO: Decodable {
    var items: [GoogleCalendarListEntryDTO]?
    var nextPageToken: String?
}
