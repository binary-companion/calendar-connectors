//! Port of `AlarmMapper.swift`.

use calendar_core::calendar_user_address::CalendarUserAddress;
use calendar_core::model::{
    Reminder, ReminderAnchor, ReminderProximity, ReminderTrigger, ReminderType, StructuredLocation,
};
use calendar_core::url_string::UrlString;
use calendar_core::write::types::{EventField, WriteError};

use crate::component::{ICalComponent, ICalParameter, ICalProperty};
use crate::grapheme::{drop_first, split};
use crate::ical_values::{ICalDateValue, ICalValues};
use crate::swift_number::swift_double;

pub enum AlarmMapper {}

impl AlarmMapper {
    /// Every readable `VALARM` as a `Reminder`; an alarm with an unreadable trigger is skipped. `is_calendar_default` is
    /// true for Apple's default alarms (`X-APPLE-DEFAULT-ALARM` or `X-APPLE-LOCAL-DEFAULT-ALARM` set to TRUE) and false
    /// otherwise. (Swift `reminders(from:)`.)
    pub fn reminders_from<'a>(
        alarms: impl IntoIterator<Item = &'a ICalComponent>,
    ) -> Vec<Reminder> {
        alarms
            .into_iter()
            .filter_map(AlarmMapper::reminder)
            .collect()
    }

    pub(crate) fn reminder(alarm: &ICalComponent) -> Option<Reminder> {
        let trigger_property = alarm.property("TRIGGER")?;
        let mut trigger = if trigger_property
            .parameter("VALUE")
            .map(str::to_uppercase)
            .as_deref()
            == Some("DATE-TIME")
        {
            let Some(ICalDateValue::Utc(date)) = ICalValues::date_value(trigger_property) else {
                return None;
            };
            ReminderTrigger::Absolute(date)
        } else {
            let offset = ICalValues::duration(&trigger_property.value)?;
            let anchor = if trigger_property
                .parameter("RELATED")
                .map(str::to_uppercase)
                .as_deref()
                == Some("END")
            {
                ReminderAnchor::End
            } else {
                ReminderAnchor::Start
            };
            ReminderTrigger::Relative { offset, to: anchor }
        };
        if let Some(proximity) = alarm
            .property("X-APPLE-PROXIMITY")
            .map(|p| p.value.to_uppercase())
            && (proximity == "ARRIVE" || proximity == "DEPART")
            && let Some(place) = alarm.property("X-APPLE-STRUCTURED-LOCATION")
        {
            trigger = ReminderTrigger::Location(
                AlarmMapper::structured_location(place),
                if proximity == "ARRIVE" {
                    ReminderProximity::Enter
                } else {
                    ReminderProximity::Leave
                },
            );
        }
        let action = alarm
            .property("ACTION")
            .map(|p| p.value.to_uppercase())
            .unwrap_or_else(|| "DISPLAY".to_string());
        let kind = match action.as_str() {
            "DISPLAY" => ReminderType::Display,
            "AUDIO" => ReminderType::Audio {
                sound_name: alarm.property("ATTACH").map(|p| p.value.clone()),
            },
            "EMAIL" => ReminderType::Email {
                address: alarm
                    .property("ATTENDEE")
                    .and_then(|p| CalendarUserAddress::email_from(Some(&p.value))),
            },
            "PROCEDURE" => ReminderType::Procedure {
                url: alarm
                    .property("ATTACH")
                    .and_then(|p| UrlString::parse(&p.value)),
            },
            _ => ReminderType::Other(action),
        };
        let repeat_count = alarm
            .property("REPEAT")
            .and_then(|p| p.value.parse::<i64>().ok())
            .unwrap_or(0);
        let interval = alarm
            .property("DURATION")
            .and_then(|p| ICalValues::duration(&p.value));
        let is_default = ["X-APPLE-DEFAULT-ALARM", "X-APPLE-LOCAL-DEFAULT-ALARM"]
            .iter()
            .any(|name| {
                alarm
                    .property(name)
                    .map(|p| p.value.to_uppercase())
                    .as_deref()
                    == Some("TRUE")
            });
        Some(Reminder {
            trigger,
            kind,
            repeat_count,
            repeat_interval: if repeat_count > 0 { interval } else { None },
            is_calendar_default: Some(is_default),
        })
    }

    /// `geo:lat,long` with `X-TITLE` and `X-APPLE-RADIUS` parameters.
    pub(crate) fn structured_location(property: &ICalProperty) -> StructuredLocation {
        let mut latitude = None;
        let mut longitude = None;
        if property.value.to_lowercase().starts_with("geo:") {
            let parts = split(drop_first(&property.value, 4), ',');
            if parts.len() >= 2 {
                latitude = swift_double(parts[0]);
                longitude = swift_double(parts[1]);
            }
        }
        StructuredLocation {
            title: property.parameter("X-TITLE").map(str::to_string),
            latitude,
            longitude,
            radius: property.parameter("X-APPLE-RADIUS").and_then(swift_double),
        }
    }

    /// `VALARM`s for reminders a CalDAV server stores as written: relative to the start or end, or absolute, as a
    /// display or sound alert, optionally repeating. Anything else fails with `.unsupported(fields: [.reminders])`.
    /// (Swift `alarms(from:)`.)
    pub fn alarms_from(reminders: &[Reminder]) -> Result<Vec<ICalComponent>, WriteError> {
        let unsupported = || WriteError::Unsupported {
            fields: [EventField::Reminders].into_iter().collect(),
        };
        let mut alarms = Vec::with_capacity(reminders.len());
        for reminder in reminders {
            let mut properties: Vec<ICalProperty> = Vec::new();
            match &reminder.kind {
                ReminderType::Display => {
                    properties.push(ICalProperty::with_value("ACTION", "DISPLAY"));
                    properties.push(ICalProperty::from_text(
                        "DESCRIPTION",
                        "Reminder",
                        Vec::new(),
                    ));
                }
                ReminderType::Audio { sound_name } => {
                    properties.push(ICalProperty::with_value("ACTION", "AUDIO"));
                    if let Some(sound) = sound_name {
                        properties.push(ICalProperty::new(
                            "ATTACH",
                            vec![ICalParameter::single("VALUE", "URI")],
                            sound.clone(),
                        ));
                    }
                }
                _ => return Err(unsupported()),
            }
            match &reminder.trigger {
                ReminderTrigger::Relative { offset, to } => {
                    properties.push(ICalProperty::new(
                        "TRIGGER",
                        if *to == ReminderAnchor::End {
                            vec![ICalParameter::single("RELATED", "END")]
                        } else {
                            Vec::new()
                        },
                        ICalValues::duration_text(*offset),
                    ));
                }
                ReminderTrigger::Absolute(date) => {
                    properties.push(ICalProperty::new(
                        "TRIGGER",
                        vec![ICalParameter::single("VALUE", "DATE-TIME")],
                        ICalValues::utc_text(*date),
                    ));
                }
                ReminderTrigger::Location(..) => return Err(unsupported()),
            }
            if reminder.repeat_count > 0
                && let Some(interval) = reminder.repeat_interval
            {
                properties.push(ICalProperty::with_value(
                    "REPEAT",
                    reminder.repeat_count.to_string(),
                ));
                properties.push(ICalProperty::with_value(
                    "DURATION",
                    ICalValues::duration_text(interval),
                ));
            }
            alarms.push(ICalComponent::new("VALARM", properties, Vec::new()));
        }
        Ok(alarms)
    }
}
