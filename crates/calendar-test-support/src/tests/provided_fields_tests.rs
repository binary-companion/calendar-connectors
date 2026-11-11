//! The `ModelTests.swift` test `aDeclaredFieldThatIsNilIsReportedAndAnUndeclaredNilIsFine`, which uses
//! `ProvidedFieldsConformance`.

use calendar_core::model::{CalendarEvent, EventKind};
use calendar_core::source_types::{ProvidedField, SourceCapabilities};

use super::support::at;
use crate::provided_fields_conformance::ProvidedFieldsConformance;

#[test]
fn a_declared_field_that_is_nil_is_reported_and_an_undeclared_nil_is_fine() {
    let mut event = CalendarEvent::new("e", "c", "T", at(0), at(60));
    let capabilities = SourceCapabilities {
        provided_fields: [ProvidedField::Kind, ProvidedField::Reminders]
            .into_iter()
            .collect(),
        ..SourceCapabilities::default()
    };
    assert_eq!(
        ProvidedFieldsConformance::event_violations(&event, &capabilities),
        vec![
            "declared field kind is nil",
            "declared field reminders is nil"
        ]
    );
    assert!(
        ProvidedFieldsConformance::event_violations(&event, &SourceCapabilities::default())
            .is_empty()
    );
    event.kind = Some(EventKind::Standard);
    event.reminders = Some(vec![]);
    assert!(ProvidedFieldsConformance::event_violations(&event, &capabilities).is_empty());
}
