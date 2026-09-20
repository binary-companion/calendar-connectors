import Foundation

public enum AccessRole: String, Sendable { case owner, writer, reader, freeBusyReader }
public enum EventStatus: String, Sendable { case confirmed, tentative, cancelled }
public enum Availability: String, Sendable { case busy, free }
public enum Visibility: String, Sendable { case `default`, publicEvent, privateEvent, confidential }
public enum EventKind: String, Sendable { case standard, focusTime, outOfOffice, workingLocation, birthday, other }
public enum ResponseStatus: String, Sendable { case accepted, tentative, declined, needsAction }
public enum AttendeeRole: String, Sendable { case required, optional, resource }
public enum ConferenceProvider: String, Sendable { case meet, teams, zoom, other }

public struct Attendee: Hashable, Sendable {
    public var name: String?
    public private(set) var email: String?
    public var role: AttendeeRole
    public var response: ResponseStatus
    public var isSelf: Bool
    public var isOrganizer: Bool

    public init(
        name: String? = nil, email: String? = nil, role: AttendeeRole = .required,
        response: ResponseStatus = .needsAction, isSelf: Bool = false, isOrganizer: Bool = false
    ) {
        self.name = name
        let trimmed = email?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        self.email = (trimmed?.isEmpty ?? true) ? nil : trimmed
        self.role = role
        self.response = response
        self.isSelf = isSelf
        self.isOrganizer = isOrganizer
    }
}

public struct ConferenceInfo: Hashable, Sendable {
    public var url: URL
    public var provider: ConferenceProvider
    public init(url: URL, provider: ConferenceProvider) {
        self.url = url
        self.provider = provider
    }
}

public struct Reminder: Hashable, Sendable {
    public var minutesBefore: Int
    public init(minutesBefore: Int) { self.minutesBefore = minutesBefore }
}

/// One calendar within an account.
public struct CalendarDescriptor: Hashable, Sendable, Identifiable {
    public var id: String
    public var title: String
    /// "#RRGGBB" uppercase, or nil when unknown or invalid.
    public private(set) var colorHex: String?
    public var accessRole: AccessRole
    public var isPrimary: Bool
    public var timeZone: TimeZone?
    /// The owning account, e.g. the signed-in email.
    public var accountName: String?

    public init(
        id: String, title: String, colorHex: String? = nil, accessRole: AccessRole = .reader,
        isPrimary: Bool = false, timeZone: TimeZone? = nil, accountName: String? = nil
    ) {
        self.id = id
        self.title = title
        self.colorHex = Self.normalizedHex(colorHex)
        self.accessRole = accessRole
        self.isPrimary = isPrimary
        self.timeZone = timeZone
        self.accountName = accountName
    }

    /// "#RGB", "#RRGGBB" or "RRGGBB" (any case) to "#RRGGBB" uppercase; nil for anything else.
    public static func normalizedHex(_ raw: String?) -> String? {
        guard var s = raw?.trimmingCharacters(in: .whitespacesAndNewlines) else { return nil }
        if s.hasPrefix("#") { s.removeFirst() }
        guard s.count == 3 || s.count == 6, s.allSatisfy(\.isASCII), s.allSatisfy(\.isHexDigit) else { return nil }
        s = s.uppercased()
        if s.count == 3 { s = s.map { "\($0)\($0)" }.joined() }
        return "#" + s
    }
}

public struct CalendarEvent: Hashable, Sendable, Identifiable {
    /// Unique within its calendar (a recurring instance has its own id).
    public var eventID: String
    /// Unique across the calendars of one source.
    public var id: String { "\(calendarID)/\(eventID)" }
    /// iCalendar UID (Google `iCalUID`); stable across copies of the same meeting.
    public var uid: String?
    public var calendarID: String
    public var title: String
    public var notes: String?
    public var location: String?
    /// All-day: midnight of the first day in `timeZone`, `end` exclusive (midnight after the last day).
    public var start: Date
    public var end: Date
    /// Always non-nil when `isAllDay`; interpret an all-day event's days in this zone, not the device's.
    public var timeZone: TimeZone?
    public var isAllDay: Bool
    public var status: EventStatus
    public var availability: Availability
    public var visibility: Visibility
    public var kind: EventKind
    public var seriesID: String?
    public var originalStart: Date?
    public var attendees: [Attendee]
    public var organizer: Attendee?
    public var conference: ConferenceInfo?
    public var reminders: [Reminder]
    public var url: URL?
    /// Opaque provider version (Google etag); Phase 3 uses it for optimistic writes.
    public var version: String?
    /// The account owner's own response, when the provider says.
    public var myResponse: ResponseStatus?

    public init(
        eventID: String, uid: String? = nil, calendarID: String, title: String,
        notes: String? = nil, location: String? = nil, start: Date, end: Date,
        timeZone: TimeZone? = nil, isAllDay: Bool = false, status: EventStatus = .confirmed,
        availability: Availability = .busy, visibility: Visibility = .default, kind: EventKind = .standard,
        seriesID: String? = nil, originalStart: Date? = nil, attendees: [Attendee] = [],
        organizer: Attendee? = nil, conference: ConferenceInfo? = nil, reminders: [Reminder] = [],
        url: URL? = nil, version: String? = nil, myResponse: ResponseStatus? = nil
    ) {
        self.eventID = eventID
        self.uid = uid
        self.calendarID = calendarID
        self.title = title
        self.notes = notes
        self.location = location
        self.start = start
        self.end = end
        self.timeZone = timeZone
        self.isAllDay = isAllDay
        self.status = status
        self.availability = availability
        self.visibility = visibility
        self.kind = kind
        self.seriesID = seriesID
        self.originalStart = originalStart
        self.attendees = attendees
        self.organizer = organizer
        self.conference = conference
        self.reminders = reminders
        self.url = url
        self.version = version
        self.myResponse = myResponse
    }
}
