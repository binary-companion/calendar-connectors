import CalendarCore
import Foundation

enum GoogleEventMapper {
    static func descriptor(from dto: GoogleCalendarListEntryDTO, accountName: String?) -> CalendarDescriptor? {
        if dto.deleted == true || dto.hidden == true { return nil }
        let role: AccessRole
        switch dto.accessRole {
        case "owner": role = .owner
        case "writer": role = .writer
        case "freeBusyReader": role = .freeBusyReader
        default: role = .reader
        }
        return CalendarDescriptor(
            id: dto.id, title: dto.summaryOverride ?? dto.summary ?? dto.id, colorHex: dto.backgroundColor,
            accessRole: role, isPrimary: dto.primary ?? false,
            timeZone: dto.timeZone.flatMap { TimeZone(identifier: $0) }, accountName: accountName)
    }

    /// Returns nil for cancelled events and for events whose times cannot be understood.
    static func map(_ dto: GoogleEventDTO, calendar: CalendarDescriptor) -> CalendarEvent? {
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
            eventID: dto.id, uid: dto.iCalUID, calendarID: calendar.id, title: dto.summary ?? "",
            notes: dto.description, location: dto.location, start: start.date, end: end.date,
            timeZone: start.zone, isAllDay: start.isAllDay, status: dto.status == "tentative" ? .tentative : .confirmed,
            availability: dto.transparency == "transparent" ? .free : .busy,
            visibility: visibility(dto.visibility), kind: kind(dto.eventType),
            seriesID: dto.recurringEventId,
            originalStart: dto.originalStartTime.flatMap { resolve($0, calendarZone: calendarZone)?.date },
            attendees: attendees, organizer: organizer, conference: conference(dto),
            reminders: reminders(dto.reminders), url: dto.htmlLink.flatMap(URL.init(string:)),
            version: dto.etag, myResponse: attendees.first(where: \.isSelf)?.response)
    }

    private struct Resolved {
        var date: Date
        var zone: TimeZone?
        var isAllDay: Bool
    }

    private static func resolve(_ time: GoogleTimeDTO?, calendarZone: TimeZone) -> Resolved? {
        guard let time else { return nil }
        if let day = time.date {
            let parts = day.split(separator: "-").compactMap { Int($0) }
            var calendar = Calendar(identifier: .gregorian)
            calendar.timeZone = calendarZone
            guard parts.count == 3,
                  let date = calendar.date(from: DateComponents(year: parts[0], month: parts[1], day: parts[2]))
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

    private static func conference(_ dto: GoogleEventDTO) -> ConferenceInfo? {
        if let video = dto.conferenceData?.entryPoints?.first(where: { $0.entryPointType == "video" }),
           let text = video.uri, let url = URL(string: text)
        {
            let solution = dto.conferenceData?.conferenceSolution
            let name = (solution?.name ?? "").lowercased()
            let host = (url.host ?? "").lowercased()
            let provider: ConferenceProvider
            if solution?.key?.type == "hangoutsMeet" || host.contains("meet.google.com") {
                provider = .meet
            } else if name.contains("zoom") || host.contains("zoom.") {
                provider = .zoom
            } else if name.contains("teams") || host.contains("teams.microsoft") {
                provider = .teams
            } else {
                provider = .other
            }
            return ConferenceInfo(url: url, provider: provider)
        }
        if let text = dto.hangoutLink, let url = URL(string: text) {
            return ConferenceInfo(url: url, provider: .meet)
        }
        return nil
    }

    private static func reminders(_ dto: GoogleRemindersDTO?) -> [Reminder] {
        guard let dto, dto.useDefault != true else { return [] }
        return (dto.overrides ?? []).compactMap { $0.minutes.map(Reminder.init(minutesBefore:)) }
    }
}
