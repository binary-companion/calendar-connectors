//! Port of `CalendarTestSupport/ProvidedFieldsConformance.swift`.

use calendar_core::model::{CalendarDescriptor, CalendarEvent};
use calendar_core::source_types::{CalendarSource, ProvidedField, SourceCapabilities};

/// Every connector's mapper tests run their fixtures through this: a field a source lists in `provided_fields` must
/// never be `None` on its events or calendars. Fields that are not listed may be `None`.
///
/// Swift has three overloads of `violations`; they are [`event_violations`](Self::event_violations),
/// [`source_violations`](Self::source_violations) and [`calendar_violations`](Self::calendar_violations).
pub enum ProvidedFieldsConformance {}

/// The declared fields in raw-value order, which is the order violations are reported in.
fn sorted_fields(capabilities: &SourceCapabilities) -> Vec<ProvidedField> {
    let mut fields: Vec<ProvidedField> = capabilities.provided_fields.iter().copied().collect();
    fields.sort_by_key(|field| field.as_str());
    fields
}

impl ProvidedFieldsConformance {
    /// The declared event fields that are missing on `event`. An empty result means the event conforms.
    pub fn event_violations(
        event: &CalendarEvent,
        capabilities: &SourceCapabilities,
    ) -> Vec<String> {
        let mut found = Vec::new();
        for field in sorted_fields(capabilities) {
            let missing = match field {
                ProvidedField::Kind => event.kind.is_none(),
                ProvidedField::Visibility => event.visibility.is_none(),
                ProvidedField::Availability => event.availability.is_none(),
                ProvidedField::Reminders => event.reminders.is_none(),
                ProvidedField::Series => event.series.is_none(),
                ProvidedField::Participation => event.participation.is_none(),
                ProvidedField::Version => event.version.is_none(),
                ProvidedField::LastModified => event.last_modified.is_none(),
                ProvidedField::Created => event.created.is_none(),
                ProvidedField::UidScope => event.uid_scope.is_none(),
                // A source-level capability.
                ProvidedField::RecurrenceRules => false,
                // A source without a conference has none to provide.
                ProvidedField::StructuredConference => false,
                // Calendar fields.
                ProvidedField::IsDefault
                | ProvidedField::CalendarTimeZone
                | ProvidedField::DefaultReminders
                | ProvidedField::Provider
                | ProvidedField::SupportedAvailabilities
                | ProvidedField::PermissionDetails => false,
            };
            if missing {
                found.push(format!("declared field {} is nil", field.as_str()));
            }
        }
        found
    }

    /// A source declares `RecurrenceRules` exactly when it conforms to `SeriesSource`.
    pub fn source_violations(source: &dyn CalendarSource) -> Vec<String> {
        let declares = source
            .capabilities()
            .provided_fields
            .contains(&ProvidedField::RecurrenceRules);
        let conforms = source.as_series().is_some();
        if declares == conforms {
            return Vec::new();
        }
        vec![
            if declares {
                "declares recurrenceRules but does not conform to SeriesSource"
            } else {
                "conforms to SeriesSource but does not declare recurrenceRules"
            }
            .to_string(),
        ]
    }

    /// The declared calendar fields that are missing on `calendar`.
    pub fn calendar_violations(
        calendar: &CalendarDescriptor,
        capabilities: &SourceCapabilities,
    ) -> Vec<String> {
        let mut found = Vec::new();
        for field in sorted_fields(capabilities) {
            let missing = match field {
                ProvidedField::IsDefault => calendar.is_default.is_none(),
                ProvidedField::CalendarTimeZone => calendar.time_zone.is_none(),
                ProvidedField::DefaultReminders => calendar.default_reminders.is_none(),
                ProvidedField::Provider => calendar.provider.is_none(),
                ProvidedField::SupportedAvailabilities => {
                    calendar.supported_availabilities.is_none()
                }
                ProvidedField::PermissionDetails => {
                    calendar.permissions.can_share.is_none()
                        || calendar.permissions.can_view_private.is_none()
                }
                // Event fields.
                _ => false,
            };
            if missing {
                found.push(format!("declared field {} is nil", field.as_str()));
            }
        }
        found
    }
}
