import CalendarCore
import Foundation

enum GraphEventMapper {
    /// The event properties every read asks for (`$select`), so a response carries what the mapper needs and no more.
    static let eventSelect = "id,iCalUId,subject,body,location,start,end,isAllDay,isCancelled,showAs,sensitivity,type,seriesMasterId,originalStart,originalStartTimeZone,attendees,organizer,isOrganizer,responseStatus,isReminderOn,reminderMinutesBeforeStart,isOnlineMeeting,onlineMeeting,changeKey,lastModifiedDateTime,createdDateTime,webLink,recurrence"

    /// Graph's `showAs` has five values; two of them read as the library's four.
    static let supportedAvailabilities: Set<Availability> = [.busy, .free, .tentative, .unavailable]

    static func descriptor(from dto: GraphCalendarDTO, accountName: String?, zone: TimeZone) -> CalendarDescriptor {
        CalendarDescriptor(
            id: dto.id, title: dto.name ?? dto.id, service: .microsoft, colorHex: dto.hexColor,
            permissions: CalendarPermissions(
                canViewDetails: true, canEdit: dto.canEdit ?? false, canShare: dto.canShare ?? false,
                canViewPrivate: dto.canViewPrivateItems ?? false),
            isDefault: dto.isDefaultCalendar ?? false, timeZone: zone, accountName: accountName, kind: .standard,
            provider: .microsoft, supportedAvailabilities: supportedAvailabilities)
    }

    /// Returns nil for cancelled events and for events whose times cannot be understood. `accountEmail` (lowercased)
    /// marks the attendee that is the account owner.
    static func map(_ dto: GraphEventDTO, calendar: CalendarDescriptor, accountEmail: String?, sourceID: String? = nil) -> CalendarEvent? {
        if dto.isCancelled == true { return nil }
        guard let times = resolve(dto, fallbackZone: calendar.timeZone ?? TimeZone(identifier: "UTC")!) else { return nil }

        let organizerEmail = dto.organizer?.emailAddress?.address?.lowercased()
        let attendees = (dto.attendees ?? []).map { attendee($0, accountEmail: accountEmail, organizerEmail: organizerEmail) }
        let organizer = dto.organizer?.emailAddress.map {
            Attendee(name: $0.name, email: $0.address, role: .required, response: .accepted,
                     isSelf: dto.isOrganizer == true || (accountEmail != nil && $0.address?.lowercased() == accountEmail), isOrganizer: true)
        }
        let notes = plainText(dto.body)
        return CalendarEvent(
            eventID: dto.id, uid: dto.iCalUId, uidScope: .global, calendarID: calendar.id, title: dto.subject ?? "(No title)",
            notes: notes, location: dto.location?.displayName.flatMap { $0.isEmpty ? nil : $0 },
            start: times.start, end: times.end, timeZone: times.zone, isAllDay: times.isAllDay, status: .confirmed,
            availability: availability(dto.showAs), visibility: visibility(dto.sensitivity), kind: .standard,
            series: series(dto, start: times.start), attendees: attendees, organizer: organizer,
            conferences: conferences(dto, notes: notes), reminders: reminders(dto),
            url: dto.webLink.flatMap(URL.init(string:)), version: dto.changeKey,
            lastModified: dto.lastModifiedDateTime.flatMap(GraphTime.parseInstant),
            created: dto.createdDateTime.flatMap(GraphTime.parseInstant),
            participation: participation(attendees: attendees, organizer: organizer), sourceID: sourceID)
    }

    struct Resolved {
        var start: Date
        var end: Date
        /// The zone the event is shown in.
        var zone: TimeZone
        var isAllDay: Bool
    }

    /// Reads the times in the zone the response names (the account zone, because reads send `Prefer: outlook.timezone`).
    /// A timed event is shown in the zone it was scheduled in (`originalStartTimeZone`) when Graph gives one.
    static func resolve(_ dto: GraphEventDTO, fallbackZone: TimeZone) -> Resolved? {
        guard let startText = dto.start?.dateTime, let endText = dto.end?.dateTime else { return nil }
        let startZone = dto.start?.timeZone.flatMap(WindowsTimeZones.timeZone(for:)) ?? fallbackZone
        let endZone = dto.end?.timeZone.flatMap(WindowsTimeZones.timeZone(for:)) ?? startZone
        if dto.isAllDay == true {
            guard let first = GraphTime.date(startText), var last = GraphTime.date(endText) else { return nil }
            if last <= first { last = first.adding(days: 1) }
            guard let range = AllDay.canonical(first: first, endExclusive: last, in: startZone) else { return nil }
            return Resolved(start: range.start, end: range.end, zone: startZone, isAllDay: true)
        }
        guard let start = GraphTime.parse(startText, in: startZone), let end = GraphTime.parse(endText, in: endZone) else { return nil }
        let shown = dto.originalStartTimeZone.flatMap(WindowsTimeZones.timeZone(for:)) ?? startZone
        return Resolved(start: start, end: max(start, end), zone: shown, isAllDay: false)
    }

    private static func availability(_ raw: String?) -> Availability {
        switch raw {
        case "free", "workingElsewhere": .free
        case "tentative": .tentative
        case "oof": .unavailable
        default: .busy
        }
    }

    private static func visibility(_ raw: String?) -> Visibility {
        switch raw {
        case "personal", "private": .privateEvent
        case "confidential": .confidential
        default: .default
        }
    }

    private static func series(_ dto: GraphEventDTO, start: Date) -> SeriesInfo {
        switch dto.type {
        case "occurrence", "exception":
            guard let master = dto.seriesMasterId else { return .notRecurring }
            return .occurrence(seriesID: master, originalStart: dto.originalStart.flatMap(GraphTime.parseInstant))
        case "seriesMaster":
            // A master is its own series, starting at its own slot, so a single-instance write on it can be refused.
            return .occurrence(seriesID: dto.id, originalStart: start)
        default:
            return .notRecurring
        }
    }

    private static func attendee(_ dto: GraphAttendeeDTO, accountEmail: String?, organizerEmail: String?) -> Attendee {
        let email = dto.emailAddress?.address?.lowercased()
        let role: AttendeeRole
        switch dto.type {
        case "optional": role = .optional
        case "resource": role = .resource
        default: role = .required
        }
        let response: ResponseStatus
        switch dto.status?.response {
        case "accepted", "organizer": response = .accepted
        case "tentativelyAccepted": response = .tentative
        case "declined": response = .declined
        default: response = .needsAction
        }
        return Attendee(
            name: dto.emailAddress?.name, email: email, role: role, response: response,
            isSelf: email != nil && email == accountEmail, isOrganizer: email != nil && email == organizerEmail)
    }

    /// A `self` attendee gives their response; an organizer who is you with no attendee entry (an event with no
    /// guests) counts as accepted; anything else is an event you are not on.
    static func participation(attendees: [Attendee], organizer: Attendee?) -> Participation {
        if let me = attendees.first(where: \.isSelf) { return .invited(me.response) }
        if organizer?.isSelf == true { return .invited(.accepted) }
        return .notInvited
    }

    /// Graph's structured Teams link, then links found in the location and notes.
    private static func conferences(_ dto: GraphEventDTO, notes: String?) -> [ConferenceInfo] {
        var structured: [ConferenceInfo] = []
        if let text = dto.onlineMeeting?.joinUrl, let url = URL(string: text) {
            structured.append(ConferenceInfo(url: url, provider: ConferenceDetector.provider(of: url) ?? .teams))
        }
        return ConferenceDetector.conferences(structured: structured, location: dto.location?.displayName, url: nil, notes: notes)
    }

    /// Graph keeps one reminder per event: on or off, and how many minutes before the start.
    private static func reminders(_ dto: GraphEventDTO) -> [Reminder] {
        guard dto.isReminderOn == true else { return [] }
        return [.before(minutes: dto.reminderMinutesBeforeStart ?? 15)]
    }

    /// Reads reach Graph with `Prefer: outlook.body-content-type="text"`, so a body is normally plain text already.
    /// An HTML body (a request that lost the preference) is reduced to text, so notes never carry markup.
    static func plainText(_ body: GraphBodyDTO?) -> String? {
        guard var text = body?.content, !text.isEmpty else { return nil }
        if body?.contentType?.lowercased() == "html" {
            text = text.replacingOccurrences(of: "<br\\s*/?>", with: "\n", options: [.regularExpression, .caseInsensitive])
            text = text.replacingOccurrences(of: "</p>", with: "\n", options: .caseInsensitive)
            text = text.replacingOccurrences(of: "<[^>]+>", with: "", options: .regularExpression)
            for (entity, character) in [("&nbsp;", " "), ("&lt;", "<"), ("&gt;", ">"), ("&quot;", "\""), ("&#39;", "'"), ("&amp;", "&")] {
                text = text.replacingOccurrences(of: entity, with: character)
            }
        }
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}
