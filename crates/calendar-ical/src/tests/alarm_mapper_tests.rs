//! Port of `AlarmMapperTests.swift`.

use calendar_core::model::{
    Reminder, ReminderAnchor, ReminderProximity, ReminderTrigger, ReminderType, StructuredLocation,
};

use super::fixtures::at;
use crate::alarm_mapper::AlarmMapper;
use crate::component::ICalComponent;
use crate::content_line::ICalParser;

fn alarm(lines: &[&str]) -> ICalComponent {
    let mut all = vec!["BEGIN:VALARM"];
    all.extend_from_slice(lines);
    all.push("END:VALARM");
    ICalParser::parse_text(&all.join("\r\n")).unwrap()
}

fn not_default(trigger: ReminderTrigger) -> Reminder {
    Reminder {
        is_calendar_default: Some(false),
        ..Reminder::new(trigger)
    }
}

#[test]
fn reads_relative_absolute_repeating_and_end_alarms() {
    let alarms = [
        alarm(&["ACTION:DISPLAY", "TRIGGER:-PT10M"]),
        alarm(&[
            "ACTION:AUDIO",
            "TRIGGER;RELATED=END:PT0S",
            "ATTACH;VALUE=URI:Chord",
        ]),
        alarm(&[
            "ACTION:DISPLAY",
            "TRIGGER;VALUE=DATE-TIME:20260927T150000Z",
            "REPEAT:2",
            "DURATION:PT5M",
        ]),
        alarm(&["ACTION:EMAIL", "TRIGGER:-P1D", "ATTENDEE:mailto:me@x.test"]),
        alarm(&["ACTION:DISPLAY", "TRIGGER:not-a-duration"]),
    ];
    let reminders = AlarmMapper::reminders_from(&alarms);
    assert_eq!(
        reminders,
        vec![
            not_default(ReminderTrigger::Relative {
                offset: -600.0,
                to: ReminderAnchor::Start
            }),
            Reminder {
                kind: ReminderType::Audio {
                    sound_name: Some("Chord".to_string())
                },
                ..not_default(ReminderTrigger::Relative {
                    offset: 0.0,
                    to: ReminderAnchor::End
                })
            },
            Reminder {
                repeat_count: 2,
                repeat_interval: Some(300.0),
                ..not_default(ReminderTrigger::Absolute(at(1_790_521_200)))
            },
            Reminder {
                kind: ReminderType::Email {
                    address: Some("me@x.test".to_string())
                },
                ..not_default(ReminderTrigger::Relative {
                    offset: -86_400.0,
                    to: ReminderAnchor::Start
                })
            },
        ]
    );
}

#[test]
fn reads_apple_proximity_alarms() {
    let alarms = [alarm(&[
        "ACTION:DISPLAY",
        "TRIGGER;VALUE=DATE-TIME:19760401T005545Z",
        "X-APPLE-PROXIMITY:ARRIVE",
        "X-APPLE-STRUCTURED-LOCATION;VALUE=URI;X-APPLE-RADIUS=100;X-TITLE=Office:geo:37.33,-122.03",
    ])];
    assert_eq!(
        AlarmMapper::reminders_from(&alarms),
        vec![not_default(ReminderTrigger::Location(
            StructuredLocation {
                title: Some("Office".to_string()),
                latitude: Some(37.33),
                longitude: Some(-122.03),
                radius: Some(100.0),
            },
            ReminderProximity::Enter,
        ))]
    );
}
