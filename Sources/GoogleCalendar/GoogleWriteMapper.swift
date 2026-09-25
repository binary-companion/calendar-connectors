import CalendarCore
import Foundation

/// Pure conversion between the library's write types and Google's event JSON. No I/O.
enum GoogleWriteMapper {
    typealias JSON = [String: Any]

    /// A request body plus whether the request needs `conferenceDataVersion=1`.
    struct Body {
        var json: JSON
        var needsConferenceVersion: Bool
    }

    /// Google's popup reminder range: zero minutes to four weeks.
    private static let reminderMinutes = 0...40320
    /// Google's event dates run from year 1 to 9999 (1970-based seconds).
    private static let representableSeconds = -62_135_596_800.0...253_402_300_799.0

    static func sendUpdates(_ policy: NotifyPolicy) -> String {
        switch policy {
        case .all: "all"
        case .externalOnly: "externalOnly"
        case .none: "none"
        }
    }

    /// `JSONSerialization` raises an uncatchable Objective-C exception (a crash) for a value it cannot encode, such as
    /// NaN, so check first and throw instead.
    static func data(_ json: JSON) throws -> Data {
        guard JSONSerialization.isValidJSONObject(json) else { throw WriteError.invalid("the request body is not valid JSON") }
        return try JSONSerialization.data(withJSONObject: json, options: [.sortedKeys])
    }

    static func instantText(_ date: Date) -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        return formatter.string(from: date)
    }

    private static func dayText(_ date: CalendarDate) -> String {
        String(format: "%04d-%02d-%02d", date.year, date.month, date.day)
    }

    /// The zone name Google can resolve. A fixed-offset zone (`GMT+0500`) is not an IANA name, so it yields nil; a
    /// timed event still carries its offset in `dateTime`. Darwin names the UTC zone "GMT" and Linux "UTC"; the wire
    /// format is "UTC" on both.
    private static func zoneName(_ zone: TimeZone?) -> String? {
        guard let identifier = zone?.identifier, !(identifier.hasPrefix("GMT") && identifier.count > 3) else { return nil }
        return identifier == "GMT" ? "UTC" : identifier
    }

    /// Google cannot store times outside years 1 to 9999; an out-of-range date would render as a malformed string.
    private static func requireRepresentable(_ timing: EventTiming) throws {
        for date in [timing.start, timing.end] where !representableSeconds.contains(date.timeIntervalSince1970) {
            throw WriteError.invalid("times must fall in the years 1 to 9999")
        }
    }

    /// All-day: `date` values with the exclusive end. Timed: `dateTime` (UTC) plus the zone name when there is one.
    static func timeJSON(_ timing: EventTiming) -> (start: JSON, end: JSON) {
        if timing.isAllDay {
            let days = AllDay.dates(start: timing.start, end: timing.end, in: timing.timeZone ?? TimeZone(identifier: "UTC")!)
            return (["date": dayText(days.first)], ["date": dayText(days.endExclusive)])
        }
        func stamp(_ date: Date) -> JSON {
            var json: JSON = ["dateTime": instantText(date)]
            if let name = zoneName(timing.timeZone) { json["timeZone"] = name }
            return json
        }
        return (stamp(timing.start), stamp(timing.end))
    }

    /// For PATCH: the same, plus explicit nulls for the other form's keys, so an all-day event can become timed and
    /// back (Google keeps the old `date` or `dateTime` otherwise and rejects the mixture).
    private static func patchTimeJSON(_ timing: EventTiming) -> (start: JSON, end: JSON) {
        var time = timeJSON(timing)
        let stale: [String] = timing.isAllDay ? ["dateTime", "timeZone"] : ["date"]
        for key in stale {
            time.start[key] = time.start[key] ?? NSNull()
            time.end[key] = time.end[key] ?? NSNull()
        }
        return time
    }

    private static func visibilityText(_ visibility: Visibility) -> String {
        switch visibility {
        case .default: "default"
        case .publicEvent: "public"
        case .privateEvent: "private"
        case .confidential: "confidential"
        }
    }

    static func remindersJSON(_ reminders: [Reminder]) throws -> JSON {
        guard reminders.count <= 5 else { throw WriteError.invalid("Google allows at most 5 reminders") }
        guard reminders.allSatisfy({ reminderMinutes.contains($0.minutesBefore) }) else {
            throw WriteError.invalid("reminder minutes must be between 0 and 40320")
        }
        return ["useDefault": false, "overrides": reminders.map { ["method": "popup", "minutes": $0.minutesBefore] as JSON }]
    }

    static func attendeeJSON(_ attendee: AttendeeDraft) -> JSON {
        var json: JSON = ["email": attendee.email]
        if let name = attendee.name { json["displayName"] = name }
        if attendee.role == .optional { json["optional"] = true }
        if attendee.role == .resource { json["resource"] = true }
        return json
    }

    static func conferenceRequestJSON() -> JSON {
        ["createRequest": ["requestId": UUID().uuidString, "conferenceSolutionKey": ["type": "hangoutsMeet"]] as JSON]
    }

    private static func recurrenceLines(_ rule: RecurrenceRule, allDay: Bool, zone: TimeZone?) throws -> [String] {
        try rule.validate()
        guard allDay || zoneName(zone) != nil else { throw WriteError.invalid("recurring events need a time zone") }
        return ["RRULE:" + rule.rruleString(allDay: allDay, in: zone)]
    }

    static func createBody(_ draft: EventDraft) throws -> Body {
        try draft.validate()
        try requireRepresentable(draft.timing)
        var json: JSON = ["summary": draft.title]
        if let notes = draft.notes { json["description"] = notes }
        if let location = draft.location { json["location"] = location }
        let time = timeJSON(draft.timing)
        json["start"] = time.start
        json["end"] = time.end
        json["transparency"] = draft.availability == .free ? "transparent" : "opaque"
        json["visibility"] = visibilityText(draft.visibility)
        if let reminders = draft.reminders { json["reminders"] = try remindersJSON(reminders) }
        if !draft.attendees.isEmpty { json["attendees"] = draft.attendees.map(attendeeJSON) }
        if let rule = draft.recurrence {
            json["recurrence"] = try recurrenceLines(rule, allDay: draft.timing.isAllDay, zone: draft.timing.timeZone)
        }
        if draft.conference == .generate { json["conferenceData"] = conferenceRequestJSON() }
        return Body(json: json, needsConferenceVersion: draft.conference == .generate)
    }

    /// Only the touched fields; `.clear` is JSON `null`. `currentAttendees` is required when the patch changes
    /// attendees (Google replaces the whole array on PATCH).
    static func patchBody(_ patch: EventPatch, currentAttendees: [JSON]?) throws -> Body {
        var json: JSON = [:]
        if let title = patch.title { json["summary"] = title }
        switch patch.notes { case .keep: break; case .set(let value): json["description"] = value; case .clear: json["description"] = NSNull() }
        switch patch.location { case .keep: break; case .set(let value): json["location"] = value; case .clear: json["location"] = NSNull() }
        if let timing = patch.timing {
            try timing.validate()
            try requireRepresentable(timing)
            let time = patchTimeJSON(timing)
            json["start"] = time.start
            json["end"] = time.end
        }
        if let availability = patch.availability { json["transparency"] = availability == .free ? "transparent" : "opaque" }
        if let visibility = patch.visibility { json["visibility"] = visibilityText(visibility) }
        switch patch.reminders {
        case .keep: break
        case .set(let list): json["reminders"] = try remindersJSON(list)
        case .clear: json["reminders"] = ["useDefault": true] as JSON
        }
        if let changes = patch.attendees, !changes.isEmpty {
            guard let currentAttendees else { throw WriteError.invalid("attendee changes need the current attendees") }
            json["attendees"] = mergeAttendees(current: currentAttendees, changes: changes)
        }
        switch patch.recurrence {
        case .keep: break
        case .clear: json["recurrence"] = NSNull()
        case .set(let rule):
            // The series uses the time the patch sends; only without one does it keep the base's zone.
            let allDay = patch.timing?.isAllDay ?? patch.base?.isAllDay ?? false
            let zone = patch.timing.map(\.timeZone) ?? patch.base?.timeZone
            json["recurrence"] = try recurrenceLines(rule, allDay: allDay, zone: zone)
        }
        switch patch.conference {
        case nil: break
        case .generate?: json["conferenceData"] = conferenceRequestJSON()
        case .remove?: json["conferenceData"] = NSNull()
        }
        return Body(json: json, needsConferenceVersion: patch.conference != nil)
    }

    private static func normalized(_ email: String) -> String { email.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
    private static func email(of attendee: JSON) -> String { normalized(attendee["email"] as? String ?? "") }

    /// Removes `changes.remove`, then upserts `changes.add` by email (case-insensitive). Existing entries keep every
    /// other key (responses, `self`, `organizer`), so other guests' responses survive.
    static func mergeAttendees(current: [JSON], changes: AttendeeChanges) -> [JSON] {
        let removed = Set(changes.remove.map(normalized))
        var result = current.filter { !removed.contains(email(of: $0)) }
        for draft in changes.add {
            if let index = result.firstIndex(where: { email(of: $0) == normalized(draft.email) }) {
                var merged = result[index]
                if let name = draft.name { merged["displayName"] = name }
                if draft.role == .optional { merged["optional"] = true } else { merged["optional"] = nil }
                if draft.role == .resource { merged["resource"] = true } else { merged["resource"] = nil }
                result[index] = merged
            } else {
                result.append(attendeeJSON(draft))
            }
        }
        return result
    }

    static func respondAttendees(current: [JSON], response: ResponseStatus) throws -> [JSON] {
        let text: String
        switch response {
        case .accepted: text = "accepted"
        case .tentative: text = "tentative"
        case .declined: text = "declined"
        case .needsAction: throw WriteError.invalid("cannot respond with needsAction")
        }
        guard let index = current.firstIndex(where: { $0["self"] as? Bool == true }) else {
            throw WriteError.invalid("you are not an attendee of this event")
        }
        var result = current
        result[index]["responseStatus"] = text
        return result
    }
}

// MARK: Series splitting

extension GoogleWriteMapper {
    /// Fields a new series must not copy from the master: identity, bookkeeping, and the old conference (a new one is
    /// requested instead).
    static let outputOnlyKeys: Set<String> = [
        "id", "etag", "iCalUID", "htmlLink", "created", "updated", "sequence", "creator", "organizer", "recurringEventId",
        "originalStartTime", "conferenceData", "hangoutLink", "kind", "status", "recurrence",
    ]

    private static func isRRule(_ line: String) -> Bool { line.uppercased().hasPrefix("RRULE:") }

    private static func parts(of line: String) -> [String] {
        line.dropFirst("RRULE:".count).split(separator: ";").map(String.init)
    }

    /// The `recurrence` lines with every RRULE cut off just before `split` (`COUNT` and `UNTIL` replaced by an `UNTIL`
    /// that is a date for all-day series and a UTC date-time, one second earlier, for timed ones). Other lines
    /// (EXDATE, RDATE) keep only the dates before `split`: later ones now belong to the new series (see `carriedOver`),
    /// so the master must not keep them too (a later RDATE would show up twice, and survive a delete). A value that
    /// cannot be read stays with the master. Works on the raw text, so rules outside the authorable subset are fine.
    static func truncated(_ lines: [String], before split: Date, allDay: Bool, zone: TimeZone?) -> [String] {
        let until: String
        if allDay {
            until = RecurrenceRule.dateText(AllDay.date(of: split, in: zone ?? TimeZone(identifier: "UTC")!).adding(days: -1))
        } else {
            until = RecurrenceRule.untilText(split.addingTimeInterval(-1), allDay: false, zone: nil)
        }
        return lines.compactMap { line in
            guard isRRule(line) else {
                guard isDateLine(line) else { return line }
                return dateLine(line, zone: zone ?? TimeZone(identifier: "UTC")!) { $0.map { $0 < split } ?? true }
            }
            var kept = parts(of: line).filter { !$0.uppercased().hasPrefix("COUNT=") && !$0.uppercased().hasPrefix("UNTIL=") }
            kept.append("UNTIL=" + until)
            return "RRULE:" + kept.joined(separator: ";")
        }
    }

    /// The `COUNT` of the first RRULE, if any.
    static func count(in lines: [String]) -> Int? {
        for line in lines where isRRule(line) {
            for part in parts(of: line) where part.uppercased().hasPrefix("COUNT=") { return Int(part.dropFirst("COUNT=".count)) }
        }
        return nil
    }

    /// Google allows a single RRULE per event, so every RRULE line is the one rule.
    static func replacingCount(_ lines: [String], with count: Int) -> [String] {
        lines.map { line in
            guard isRRule(line) else { return line }
            return "RRULE:" + parts(of: line).map { $0.uppercased().hasPrefix("COUNT=") ? "COUNT=\(count)" : $0 }.joined(separator: ";")
        }
    }

    static func rruleLines(_ lines: [String]) -> [String] { lines.filter(isRRule) }

    /// The EXDATE and RDATE lines cut down to the dates at or after `split`, so deleted or added occurrences that
    /// belong to the new series follow it (`truncated` removes the same dates from the master). A line left with no
    /// values is dropped, and a value that cannot be read stays with the master. `zone` reads floating times.
    static func carriedOver(_ lines: [String], from split: Date, zone: TimeZone) -> [String] {
        lines.filter(isDateLine).compactMap { line in dateLine(line, zone: zone) { $0.map { $0 >= split } ?? false } }
    }

    private static func isDateLine(_ line: String) -> Bool {
        let name = line.prefix { $0 != ";" && $0 != ":" }.uppercased()
        return (name == "EXDATE" || name == "RDATE") && line.contains(":")
    }

    /// An EXDATE or RDATE `line` reduced to the values for which `keep` (given the value's instant, nil if unreadable)
    /// is true; nil when none remain.
    private static func dateLine(_ line: String, zone: TimeZone, keep: (Date?) -> Bool) -> String? {
        guard let colon = line.firstIndex(of: ":") else { return nil }
        let head = String(line[..<colon])
        let tzid = head.split(separator: ";").dropFirst().compactMap { param -> String? in
            param.uppercased().hasPrefix("TZID=") ? String(param.dropFirst("TZID=".count)) : nil
        }.first
        let lineZone = tzid.flatMap { TimeZone(identifier: $0) } ?? zone
        let values = line[line.index(after: colon)...].split(separator: ",").map(String.init)
        let kept = values.filter { keep(icalInstant($0, zone: lineZone)) }
        return kept.isEmpty ? nil : head + ":" + kept.joined(separator: ",")
    }

    /// `yyyyMMdd` (start of that day in `zone`), `yyyyMMdd'T'HHmmss` (in `zone`) or the same with a trailing `Z` (UTC).
    /// A PERIOD value (`start/end`) reads as its start.
    private static func icalInstant(_ text: String, zone: TimeZone) -> Date? {
        var value = (text.split(separator: "/").first.map(String.init) ?? text).trimmingCharacters(in: .whitespaces).uppercased()
        let isUTC = value.hasSuffix("Z")
        if isUTC { value.removeLast() }
        let pieces = value.split(separator: "T", omittingEmptySubsequences: false).map(String.init)
        guard let day = pieces.first, day.count == 8, let year = Int(day.prefix(4)), let month = Int(day.dropFirst(4).prefix(2)),
              let dayOfMonth = Int(day.suffix(2))
        else { return nil }
        let effectiveZone = isUTC ? TimeZone(identifier: "UTC")! : zone
        if pieces.count == 1 { return AllDay.startOfDay(CalendarDate(year: year, month: month, day: dayOfMonth), in: effectiveZone) }
        guard pieces.count == 2, pieces[1].count == 6, let hour = Int(pieces[1].prefix(2)), let minute = Int(pieces[1].dropFirst(2).prefix(2)),
              let second = Int(pieces[1].suffix(2))
        else { return nil }
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = effectiveZone
        return calendar.date(from: DateComponents(year: year, month: month, day: dayOfMonth, hour: hour, minute: minute, second: second))
    }

    private static func hasMeetLink(_ json: JSON) -> Bool {
        let key = ((json["conferenceData"] as? JSON)?["conferenceSolution"] as? JSON)?["key"] as? JSON
        return key?["type"] as? String == "hangoutsMeet" || json["hangoutLink"] != nil
    }

    /// The zone name for a fallback identifier, spelled as Google's wire format wants it. An identifier this platform
    /// does not know is passed through unchanged (it came from Google in the first place).
    private static func wireName(_ identifier: String) -> String? {
        guard let zone = TimeZone(identifier: identifier) else { return identifier }
        return zoneName(zone)
    }

    /// The days between two `date` values of an all-day master.
    private static func dayCount(_ first: String, _ last: String) -> Int? {
        func day(_ text: String) -> Date? {
            let parts = text.split(separator: "-").compactMap { Int($0) }
            guard parts.count == 3 else { return nil }
            return AllDay.startOfDay(CalendarDate(year: parts[0], month: parts[1], day: parts[2]), in: TimeZone(identifier: "UTC")!)
        }
        guard let a = day(first), let b = day(last) else { return nil }
        return Int((b.timeIntervalSince(a) / 86_400).rounded())
    }

    /// The start and end of the new series: the occurrence's original `slot` with the master's length, in the master's
    /// own form (`dateTime` with its zone name, or all-day `date`s), so following occurrences keep the series' time even
    /// when this one was moved. `fallbackZone` reads an all-day slot's day and names the zone when the master has none.
    private static func seriesTimes(master: JSON, slot: Date, fallbackZone: String?) throws -> (start: JSON, end: JSON) {
        let unreadable = SourceError.invalidResponse("google: unreadable event")
        guard let start = master["start"] as? JSON, let end = master["end"] as? JSON else { throw unreadable }
        let fallback = fallbackZone.flatMap { TimeZone(identifier: $0) }
        if let first = start["date"] as? String, let last = end["date"] as? String {
            guard let days = dayCount(first, last), days >= 1 else { throw unreadable }
            let day = AllDay.date(of: slot, in: fallback ?? TimeZone(identifier: "UTC")!)
            return (["date": dayText(day)], ["date": dayText(day.adding(days: days))])
        }
        guard let startText = start["dateTime"] as? String, let endText = end["dateTime"] as? String,
              let masterStart = GoogleEventMapper.parseInstant(startText), let masterEnd = GoogleEventMapper.parseInstant(endText),
              masterEnd > masterStart
        else { throw unreadable }
        let zone = (start["timeZone"] as? String).flatMap { TimeZone(identifier: $0) } ?? fallback
        return timeJSON(EventTiming(start: slot, end: slot.addingTimeInterval(masterEnd.timeIntervalSince(masterStart)), timeZone: zone, isAllDay: false))
    }

    /// The insert body for the new series: the whole master resource (so unmodeled fields such as color, attachments and
    /// extended properties carry over) minus output-only fields, starting at the occurrence's original `slot` with the
    /// master's length (unless the patch sets the time), guests' responses reset, the patch applied, and a new Meet link
    /// requested when the master had one (unless the patch removes the conference).
    static func newSeriesBody(master: JSON, slot: Date, patch: EventPatch, recurrence: [String], fallbackZone: String?) throws -> Body {
        var json = master.filter { !outputOnlyKeys.contains($0.key) }
        if let attendees = json["attendees"] as? [JSON] {
            json["attendees"] = attendees.map { attendee -> JSON in
                var copy = attendee
                copy["responseStatus"] = nil
                copy["self"] = nil
                copy["organizer"] = nil
                return copy
            }
        }
        let times = try seriesTimes(master: master, slot: slot, fallbackZone: fallbackZone)
        json["start"] = times.start
        json["end"] = times.end
        let changes = try patchBody(patch, currentAttendees: json["attendees"] as? [JSON] ?? [])
        for (key, value) in changes.json {
            if value is NSNull {
                json[key] = nil
            } else if let nested = value as? JSON {
                // A PATCH clears the other form of a date with a null; an insert has nothing to clear.
                json[key] = nested.filter { !($0.value is NSNull) }
            } else {
                json[key] = value
            }
        }
        // A recurring event's times need a zone name for its rule to be read in.
        for key in ["start", "end"] {
            if var time = json[key] as? JSON, time["dateTime"] != nil, time["timeZone"] == nil, let zone = fallbackZone.flatMap(wireName) {
                time["timeZone"] = zone
                json[key] = time
            }
        }
        if patch.recurrence == .keep { json["recurrence"] = recurrence }
        var needsConferenceVersion = changes.needsConferenceVersion
        if patch.conference == nil, hasMeetLink(master) {
            json["conferenceData"] = conferenceRequestJSON()
            needsConferenceVersion = true
        }
        return Body(json: json, needsConferenceVersion: needsConferenceVersion)
    }
}
