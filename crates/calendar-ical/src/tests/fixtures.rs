//! Port of `Fixtures.swift`.

use std::collections::BTreeSet;

use calendar_core::civil;
use calendar_core::time::Zone;
use jiff::Timestamp;

use crate::event_reader::EventReadContext;
use crate::event_resource::EventResource;

pub(super) fn la() -> Zone {
    Zone::named("America/Los_Angeles").unwrap()
}

pub(super) fn at(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).unwrap()
}

/// Swift `laTime(_ y:, _ mo:, _ d:, _ h: = 10, _ mi: = 0)`.
macro_rules! la_time {
    ($y:expr, $mo:expr, $d:expr) => {
        $crate::tests::fixtures::la_time_at($y, $mo, $d, 10, 0)
    };
    ($y:expr, $mo:expr, $d:expr, $h:expr) => {
        $crate::tests::fixtures::la_time_at($y, $mo, $d, $h, 0)
    };
    ($y:expr, $mo:expr, $d:expr, $h:expr, $mi:expr) => {
        $crate::tests::fixtures::la_time_at($y, $mo, $d, $h, $mi)
    };
}
pub(super) use la_time;

pub(super) fn la_time_at(y: i64, mo: i64, d: i64, h: i64, mi: i64) -> Timestamp {
    civil::date_from(y, mo, d, h, mi, 0, &la()).unwrap()
}

/// Swift `context(_ name: = "4F2A.ics", selfAddresses: = ["mailto:me@icloud.test"])`.
pub(super) fn context() -> EventReadContext {
    context_named("4F2A.ics", &["mailto:me@icloud.test"])
}

pub(super) fn context_named(name: &str, self_addresses: &[&str]) -> EventReadContext {
    EventReadContext::new(
        "home",
        name,
        Some("\"e1\"".to_string()),
        Some("icloud-conn".to_string()),
        la(),
        self_addresses
            .iter()
            .map(|a| a.to_string())
            .collect::<BTreeSet<String>>(),
    )
}

pub(super) fn resource(text: &str) -> EventResource {
    EventResource::from_data(text.replace('\n', "\r\n").as_bytes()).unwrap()
}

/// Shaped like an iCloud export (scrubbed): a weekly Tuesday 10:00 series in Los Angeles from 2026-09-01, one excluded
/// date, one occurrence moved to Wednesday 14:00, attendees (the organizer's own address is a principal URL), a default alarm.
pub(super) const WEEKLY_SERIES: &str = r#"BEGIN:VCALENDAR
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
DESCRIPTION:Agenda\nhttps://zoom.us/j/123456789
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
END:VCALENDAR"#;
