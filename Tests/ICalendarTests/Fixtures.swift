import CalendarCore
import Foundation
@testable import ICalendar

let la = TimeZone(identifier: "America/Los_Angeles")!

func laTime(_ y: Int, _ mo: Int, _ d: Int, _ h: Int = 10, _ mi: Int = 0) -> Date {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = la
    return calendar.date(from: DateComponents(year: y, month: mo, day: d, hour: h, minute: mi))!
}

func context(_ name: String = "4F2A.ics", selfAddresses: Set<String> = ["mailto:me@icloud.test"]) -> EventReadContext {
    EventReadContext(calendarID: "home", resourceName: name, etag: "\"e1\"", sourceID: "icloud-conn",
                     calendarZone: la, selfAddresses: selfAddresses)
}

func resource(_ text: String) throws -> EventResource {
    try EventResource(data: Data(text.replacingOccurrences(of: "\n", with: "\r\n").utf8))
}

/// Shaped like an iCloud export (scrubbed): a weekly Tuesday 10:00 series in Los Angeles from 2026-09-01, one excluded
/// date, one occurrence moved to Wednesday 14:00, attendees (the organizer's own address is a principal URL), a default alarm.
let weeklySeries = """
BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Apple Inc.//iCloud//EN
BEGIN:VTIMEZONE
TZID:America/Los_Angeles
BEGIN:DAYLIGHT
DTSTART:20070311T020000
RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU
TZOFFSETFROM:-0800
TZOFFSETTO:-0700
END:DAYLIGHT
BEGIN:STANDARD
DTSTART:20071104T020000
RRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU
TZOFFSETFROM:-0700
TZOFFSETTO:-0800
END:STANDARD
END:VTIMEZONE
BEGIN:VEVENT
UID:4F2A-UID
DTSTAMP:20260901T000000Z
CREATED:20260801T120000Z
LAST-MODIFIED:20260902T120000Z
SEQUENCE:2
SUMMARY:Team sync
DESCRIPTION:Agenda\\nhttps://zoom.us/j/123456789
LOCATION:Room 4
DTSTART;TZID=America/Los_Angeles:20260901T100000
DTEND;TZID=America/Los_Angeles:20260901T103000
RRULE:FREQ=WEEKLY;BYDAY=TU
EXDATE;TZID=America/Los_Angeles:20260908T100000
TRANSP:OPAQUE
CLASS:PRIVATE
STATUS:CONFIRMED
ORGANIZER;CN=Me:mailto:me@icloud.test
ATTENDEE;CN=Me;PARTSTAT=ACCEPTED;ROLE=CHAIR:mailto:me@icloud.test
ATTENDEE;CN=Ann;PARTSTAT=TENTATIVE;ROLE=REQ-PARTICIPANT:mailto:Ann@Example.test
ATTENDEE;CN=Room;CUTYPE=ROOM;PARTSTAT=ACCEPTED:mailto:room@example.test
ATTENDEE;CN=Bo;ROLE=OPT-PARTICIPANT;PARTSTAT=NEEDS-ACTION:urn:uuid:1234
X-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC
BEGIN:VALARM
ACTION:DISPLAY
DESCRIPTION:Reminder
TRIGGER:-PT15M
X-APPLE-DEFAULT-ALARM:TRUE
END:VALARM
END:VEVENT
BEGIN:VEVENT
UID:4F2A-UID
DTSTAMP:20260901T000000Z
RECURRENCE-ID;TZID=America/Los_Angeles:20260915T100000
SUMMARY:Team sync (moved)
DTSTART;TZID=America/Los_Angeles:20260916T140000
DTEND;TZID=America/Los_Angeles:20260916T143000
ORGANIZER;CN=Me:mailto:me@icloud.test
ATTENDEE;CN=Me;PARTSTAT=ACCEPTED:mailto:me@icloud.test
END:VEVENT
END:VCALENDAR
"""
