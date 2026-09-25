import Foundation

/// What a calendar is, for grouping and defaults: `birthdays` and `subscribed` (holidays, read-only feeds) are
/// system-style calendars that rarely deserve a takeover.
public enum CalendarKind: String, Sendable { case standard, subscribed, birthdays }
public enum AccessRole: String, Sendable { case owner, writer, reader, freeBusyReader }
public enum EventStatus: String, Sendable { case confirmed, tentative, cancelled }
public enum Availability: String, Sendable { case busy, free, tentative, unavailable }
public enum Visibility: String, Sendable { case `default`, publicEvent, privateEvent, confidential }
public enum EventKind: String, Sendable { case standard, focusTime, outOfOffice, workingLocation, birthday, other }
public enum ResponseStatus: String, Sendable { case accepted, tentative, declined, needsAction }
public enum AttendeeRole: String, Sendable { case required, optional, resource }

/// Whether an event repeats. A real "no" has its own case, so nil stays "the source does not say".
public enum SeriesInfo: Hashable, Sendable {
    case notRecurring
    /// An instance of a series. `originalStart` is the instance's slot in the series (it differs from `start` for a
    /// moved instance); nil when the provider knows the series but not the slot.
    case occurrence(seriesID: String, originalStart: Date?)
}

/// The account owner's relation to an event. `.notInvited` is a real answer (an event you are not on).
public enum Participation: Hashable, Sendable {
    case notInvited
    case invited(ResponseStatus)
}
public enum ConferenceProvider: String, Sendable { case meet, teams, zoom, webex, goToMeeting, whereby, jitsi, slack, other }
/// Where a conference link came from, so consumers can decide how far to trust it.
public enum ConferenceOrigin: String, Sendable { case structured, location, url, notes, eventURL }

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
    public var origin: ConferenceOrigin
    public init(url: URL, provider: ConferenceProvider, origin: ConferenceOrigin = .structured) {
        self.url = url
        self.provider = provider
        self.origin = origin
    }

    /// Lowercased host plus path without trailing slashes, plus `?mtid=` for Webex (which keeps the meeting id
    /// in that query item). Two links with the same identity are the same meeting.
    public var identity: String {
        let host = url.host?.lowercased() ?? ""
        var path = url.path.lowercased()
        while path.hasSuffix("/") { path.removeLast() }
        var identity = host + path
        if host == "webex.com" || host.hasSuffix(".webex.com"),
           let meetingID = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems?
               .first(where: { $0.name.lowercased() == "mtid" })?.value {
            identity += "?mtid=" + meetingID.lowercased()
        }
        return identity
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
    public var kind: CalendarKind
    /// The reminders a new event on this calendar gets when it does not set its own (Google's calendar defaults);
    /// nil when the source does not say. Events with `useDefault` reminders carry these.
    public var defaultReminders: [Reminder]?

    public init(
        id: String, title: String, colorHex: String? = nil, accessRole: AccessRole = .reader,
        isPrimary: Bool = false, timeZone: TimeZone? = nil, accountName: String? = nil, kind: CalendarKind = .standard,
        defaultReminders: [Reminder]? = nil
    ) {
        self.id = id
        self.title = title
        self.colorHex = Self.normalizedHex(colorHex)
        self.accessRole = accessRole
        self.isPrimary = isPrimary
        self.timeZone = timeZone
        self.accountName = accountName
        self.kind = kind
        self.defaultReminders = defaultReminders
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
    /// The zone the event is shown in; always set. `start` and `end` (instants) are authoritative and this is a
    /// display hint: when the provider gives the event no zone the connector fills its fallback (Google: the
    /// calendar's zone, then UTC; EventKit: the device zone). An all-day event's days are read in this zone, not the
    /// device's.
    public var timeZone: TimeZone
    public var isAllDay: Bool
    public var status: EventStatus
    /// The fields below marked "provided" follow one rule: a field listed in `SourceCapabilities.providedFields` is
    /// never nil on that source's events. A field that is not listed may be nil, and nil always means "this source
    /// does not say", never "none". A real "none" has its own value (`[]`, `.notRecurring`, `.notInvited`).
    /// Content fields every source supports (`notes`, `location`, `url`, `uid`, `organizer`) use nil for "empty".
    public var availability: Availability?
    public var visibility: Visibility?
    public var kind: EventKind?
    /// Whether the event repeats and where it sits in its series.
    public var series: SeriesInfo?
    /// The series id when this is an instance of a series; nil for a single event and when unknown.
    public var seriesID: String? { if case .occurrence(let id, _)? = series { id } else { nil } }
    public var originalStart: Date? { if case .occurrence(_, let start)? = series { start } else { nil } }
    public var attendees: [Attendee]
    public var organizer: Attendee?
    /// Join links, most likely first: the provider's structured links, then links found in the location, url and
    /// notes (see `ConferenceDetector`).
    public var conferences: [ConferenceInfo]
    /// The first (most likely) link.
    public var conference: ConferenceInfo? { conferences.first }
    /// `[]` means none; nil means the source does not say.
    public var reminders: [Reminder]?
    public var url: URL?
    /// Opaque provider version (Google etag, EventKit modification date); the base of optimistic writes.
    public var version: String?
    /// The account owner's relation to the event, when the provider says.
    public var participation: Participation?
    /// The account owner's own response; nil both when unknown and when not invited (use `participation` to tell them apart).
    public var myResponse: ResponseStatus? { if case .invited(let response)? = participation { response } else { nil } }
    /// The source that produced this event (`Connection.sourceID`; "eventkit" for EventKit). Stamped by the source.
    public var sourceID: String?

    public init(
        eventID: String, uid: String? = nil, calendarID: String, title: String,
        notes: String? = nil, location: String? = nil, start: Date, end: Date,
        timeZone: TimeZone = TimeZone(identifier: "UTC")!, isAllDay: Bool = false, status: EventStatus = .confirmed,
        availability: Availability? = nil, visibility: Visibility? = nil, kind: EventKind? = nil,
        series: SeriesInfo? = nil, attendees: [Attendee] = [],
        organizer: Attendee? = nil, conferences: [ConferenceInfo] = [], reminders: [Reminder]? = nil,
        url: URL? = nil, version: String? = nil, participation: Participation? = nil,
        sourceID: String? = nil
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
        self.series = series
        self.attendees = attendees
        self.organizer = organizer
        self.conferences = conferences
        self.reminders = reminders
        self.url = url
        self.version = version
        self.participation = participation
        self.sourceID = sourceID
    }
}
