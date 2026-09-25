import CalendarCore
import Foundation

enum GoogleEventMapper {
    static func descriptor(from dto: GoogleCalendarListEntryDTO, accountName: String?) -> CalendarDescriptor? {
        if dto.deleted == true || dto.hidden == true { return nil }
        let calendarKind = kind(ofCalendarID: dto.id)
        return CalendarDescriptor(
            id: dto.id, title: dto.summaryOverride ?? dto.summary ?? dto.id, service: .google, colorHex: dto.backgroundColor,
            permissions: permissions(forRole: dto.accessRole), isDefault: dto.primary ?? false,
            timeZone: dto.timeZone.flatMap { TimeZone(identifier: $0) }, accountName: accountName, kind: calendarKind,
            defaultReminders: (dto.defaultReminders ?? []).compactMap { $0.minutes.map(Reminder.init(minutesBefore:)) },
            provider: calendarKind == .standard ? .google : .subscription, supportedAvailabilities: supportedAvailabilities)
    }

    /// Google's transparency has two values.
    static let supportedAvailabilities: Set<Availability> = [.busy, .free]

    /// owner: everything; writer: edit but not share; reader: read only; freeBusyReader: no details.
    static func permissions(forRole role: String?) -> CalendarPermissions {
        switch role {
        case "owner": CalendarPermissions(canViewDetails: true, canEdit: true, canShare: true, canViewPrivate: true)
        case "writer": CalendarPermissions(canViewDetails: true, canEdit: true, canShare: false, canViewPrivate: true)
        case "freeBusyReader": CalendarPermissions(canViewDetails: false, canEdit: false, canShare: false, canViewPrivate: false)
        default: CalendarPermissions(canViewDetails: true, canEdit: false, canShare: false, canViewPrivate: false)
        }
    }

    /// Google's built-in feeds have well-known ids: contacts' birthdays and the regional holiday calendars.
    private static func kind(ofCalendarID id: String) -> CalendarKind {
        if id.hasSuffix("#contacts@group.v.calendar.google.com") { return .birthdays }
        if id.hasSuffix("#holiday@group.v.calendar.google.com") { return .subscribed }
        return .standard
    }

    /// Returns nil for cancelled events and for events whose times cannot be understood.
    static func map(_ dto: GoogleEventDTO, calendar: CalendarDescriptor, sourceID: String? = nil) -> CalendarEvent? {
        if dto.status == "cancelled" { return nil }
        let calendarZone = calendar.timeZone ?? TimeZone(identifier: "UTC")!
        guard let start = resolve(dto.start, calendarZone: calendarZone),
              let end = resolve(dto.end, calendarZone: calendarZone)
        else { return nil }

        let attendees = (dto.attendees ?? []).map(attendee)
        let organizer = dto.organizer.map {
            Attendee(name: $0.displayName, email: $0.email, role: .required, response: .accepted,
                     isSelf: $0.isSelf ?? false, isOrganizer: true)
        }
        return CalendarEvent(
            eventID: dto.id, uid: dto.iCalUID, calendarID: calendar.id, title: dto.summary ?? "(No title)",
            notes: dto.description, location: dto.location, start: start.date, end: end.date,
            timeZone: start.zone ?? calendarZone, isAllDay: start.isAllDay, status: dto.status == "tentative" ? .tentative : .confirmed,
            availability: dto.transparency == "transparent" ? .free : .busy,
            visibility: visibility(dto.visibility), kind: kind(dto.eventType),
            series: series(dto, calendarZone: calendarZone),
            attendees: attendees, organizer: organizer, conferences: conferences(dto),
            reminders: reminders(dto.reminders, calendar: calendar), url: dto.htmlLink.flatMap(URL.init(string:)),
            version: dto.etag, lastModified: dto.updated.flatMap(parseInstant), created: dto.created.flatMap(parseInstant), participation: participation(attendees: attendees, organizer: organizer), sourceID: sourceID)
    }

    struct Resolved {
        var date: Date
        var zone: TimeZone?
        var isAllDay: Bool
    }

    static func resolve(_ time: GoogleTimeDTO?, calendarZone: TimeZone) -> Resolved? {
        guard let time else { return nil }
        if let day = time.date {
            let parts = day.split(separator: "-").compactMap { Int($0) }
            guard parts.count == 3,
                  let date = AllDay.startOfDay(CalendarDate(year: parts[0], month: parts[1], day: parts[2]), in: calendarZone)
            else { return nil }
            return Resolved(date: date, zone: calendarZone, isAllDay: true)
        }
        guard let text = time.dateTime, let date = parseInstant(text) else { return nil }
        return Resolved(date: date, zone: time.timeZone.flatMap { TimeZone(identifier: $0) }, isAllDay: false)
    }

    static func parseInstant(_ text: String) -> Date? {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        if let date = formatter.date(from: text) { return date }
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter.date(from: text)
    }

    private static func attendee(_ dto: GoogleAttendeeDTO) -> Attendee {
        let role: AttendeeRole = dto.resource == true ? .resource : (dto.optional == true ? .optional : .required)
        let response: ResponseStatus
        switch dto.responseStatus {
        case "accepted": response = .accepted
        case "tentative": response = .tentative
        case "declined": response = .declined
        default: response = .needsAction
        }
        return Attendee(
            name: dto.displayName, email: dto.email, role: role, response: response,
            isSelf: dto.isSelf ?? false, isOrganizer: dto.organizer ?? false)
    }

    private static func visibility(_ raw: String?) -> Visibility {
        switch raw {
        case "public": .publicEvent
        case "private": .privateEvent
        case "confidential": .confidential
        default: .default
        }
    }

    private static func kind(_ raw: String?) -> EventKind {
        switch raw {
        case nil, "default": .standard
        case "focusTime": .focusTime
        case "outOfOffice": .outOfOffice
        case "workingLocation": .workingLocation
        case "birthday": .birthday
        default: .other
        }
    }

    /// Every video entry point, then `hangoutLink`, then links found in the description (an event imported from an
    /// invite carries its Teams or Zoom link only there). `url` is not scanned: Google's `htmlLink` is never a join link.
    private static func conferences(_ dto: GoogleEventDTO) -> [ConferenceInfo] {
        var structured: [ConferenceInfo] = []
        let solution = dto.conferenceData?.conferenceSolution
        for entry in dto.conferenceData?.entryPoints ?? [] where entry.entryPointType == "video" {
            guard let text = entry.uri, let url = URL(string: text) else { continue }
            structured.append(ConferenceInfo(url: url, provider: provider(of: url, solution: solution)))
        }
        if let text = dto.hangoutLink, let url = URL(string: text) {
            structured.append(ConferenceInfo(url: url, provider: .meet))
        }
        return ConferenceDetector.conferences(structured: structured, location: dto.location, url: nil, notes: dto.description)
    }

    private static func provider(of url: URL, solution: GoogleConferenceDTO.Solution?) -> ConferenceProvider {
        if solution?.key?.type == "hangoutsMeet" { return .meet }
        if let known = ConferenceDetector.provider(of: url) { return known }
        let name = (solution?.name ?? "").lowercased()
        if name.contains("zoom") { return .zoom }
        if name.contains("teams") { return .teams }
        return .other
    }

    /// `useDefault` resolves to the calendar's own default reminders, so an event never reads as having none because
    /// it uses the defaults. An event without a `reminders` object has none.
    private static func reminders(_ dto: GoogleRemindersDTO?, calendar: CalendarDescriptor) -> [Reminder] {
        guard let dto else { return [] }
        if dto.useDefault == true { return calendar.defaultReminders ?? [] }
        return (dto.overrides ?? []).compactMap { $0.minutes.map(Reminder.init(minutesBefore:)) }
    }

    private static func series(_ dto: GoogleEventDTO, calendarZone: TimeZone) -> SeriesInfo {
        guard let id = dto.recurringEventId else { return .notRecurring }
        return .occurrence(seriesID: id, originalStart: dto.originalStartTime.flatMap { resolve($0, calendarZone: calendarZone)?.date })
    }

    /// A `self` attendee gives their response; an organizer who is you with no attendee entry (an event with no
    /// guests) counts as accepted; anything else is an event you are not on.
    static func participation(attendees: [Attendee], organizer: Attendee?) -> Participation {
        if let me = attendees.first(where: \.isSelf) { return .invited(me.response) }
        if organizer?.isSelf == true { return .invited(.accepted) }
        return .notInvited
    }
}
