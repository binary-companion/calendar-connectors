//! Port of `WriteTypesTests.swift`.

use jiff::Timestamp;

use super::support::{at, expect_write_error, fields};
use crate::model::{CalendarEvent, SeriesInfo};
use crate::source_types::SourceCapabilities;
use crate::write::types::{EventField, EventRef, WriteError, WriteValidation};

fn event(series: Option<&str>, original: Option<Timestamp>) -> CalendarEvent {
    let mut event = CalendarEvent::new("e1", "cal", "T", at(1000), at(2000));
    event.series = series.map(|id| SeriesInfo::Occurrence {
        series_id: id.to_string(),
        original_start: original,
    });
    event.version = Some("v1".to_string());
    event
}

#[test]
fn event_ref_copies_the_fields_a_write_needs() {
    let original = at(900);
    let reference = EventRef::from_event(&event(Some("s1"), Some(original)));
    let mut expected = EventRef::new("cal", "e1").with_series_id(Some("s1"));
    expected.version = Some("v1".to_string());
    expected.original_start = Some(original);
    assert_eq!(reference, expected);
}

#[test]
fn capabilities_default_to_nothing_writable() {
    let c = SourceCapabilities::default();
    assert!(
        c.writable_fields.is_empty() && c.recurrence_scopes.is_empty() && !c.controls_notifications
    );
}

#[test]
fn events_have_no_source_id_until_a_source_stamps_one() {
    assert_eq!(event(None, None).source_id, None);
    let mut stamped = event(None, None);
    stamped.source_id = Some("google-1".to_string());
    assert!(stamped.source_id.as_deref() == Some("google-1") && stamped.id() == "cal/e1");
}

#[test]
fn require_writable_names_every_missing_field() {
    let caps = SourceCapabilities {
        can_write: true,
        writable_fields: fields(&[EventField::Title, EventField::Timing]),
        ..SourceCapabilities::default()
    };
    assert!(WriteValidation::require_writable(&fields(&[EventField::Title]), &caps).is_ok());
    expect_write_error(
        WriteError::Unsupported {
            fields: fields(&[EventField::Attendees, EventField::Visibility]),
        },
        WriteValidation::require_writable(
            &fields(&[
                EventField::Title,
                EventField::Attendees,
                EventField::Visibility,
            ]),
            &caps,
        ),
    );
}

#[test]
fn write_errors_compare_by_payload() {
    let conflict = |f: EventField| WriteError::Conflict {
        fields: fields(&[f]),
    };
    assert_eq!(conflict(EventField::Title), conflict(EventField::Title));
    assert_ne!(conflict(EventField::Title), conflict(EventField::Notes));
    assert_ne!(
        WriteError::Forbidden(None),
        WriteError::Forbidden(Some("read-only calendar".to_string()))
    );
}

#[test]
fn an_empty_series_id_is_no_series() {
    assert_eq!(
        EventRef::new("cal", "e1")
            .with_series_id(Some(""))
            .series_id(),
        None
    );
    assert_eq!(
        EventRef::new("cal", "e1")
            .with_series_id(Some("s1"))
            .series_id(),
        Some("s1")
    );
    assert_eq!(
        EventRef::from_event(&event(Some(""), None)).series_id(),
        None
    );
}

#[test]
fn setting_the_series_id_to_empty_later_is_no_series_too() {
    let mut reference = EventRef::new("cal", "e1").with_series_id(Some("s1"));
    reference.set_series_id(Some(String::new()));
    assert_eq!(reference.series_id(), None);
    reference.set_series_id(Some("s2".to_string()));
    assert_eq!(reference.series_id(), Some("s2"));
}
