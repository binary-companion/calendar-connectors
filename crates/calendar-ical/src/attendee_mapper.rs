//! Port of `AttendeeMapper.swift`.

use std::collections::BTreeSet;

use calendar_core::calendar_user_address::CalendarUserAddress;
use calendar_core::model::{Attendee, AttendeeRole, Participation, ResponseStatus};
use calendar_core::write::event_draft::AttendeeDraft;

use crate::component::{ICalComponent, ICalParameter, ICalProperty};

pub enum AttendeeMapper {}

impl AttendeeMapper {
    pub fn read(
        vevent: &ICalComponent,
        self_addresses: &BTreeSet<String>,
    ) -> (Vec<Attendee>, Option<Attendee>) {
        let organizer = vevent.property("ORGANIZER").map(|property| {
            let email = AttendeeMapper::email_of(property);
            Attendee::new(
                property.parameter("CN").map(str::to_string),
                email.as_deref(),
                AttendeeRole::Required,
                ResponseStatus::Accepted,
                AttendeeMapper::is_self(&property.value, email.as_deref(), self_addresses),
                true,
            )
        });
        let attendees = vevent
            .properties_named("ATTENDEE")
            .into_iter()
            .map(|property| {
                let email = AttendeeMapper::email_of(property);
                let is_organizer = email.is_some()
                    && email.as_deref() == organizer.as_ref().and_then(|o| o.email());
                Attendee::new(
                    property.parameter("CN").map(str::to_string),
                    email.as_deref(),
                    AttendeeMapper::role_of(property),
                    AttendeeMapper::response_of(property),
                    AttendeeMapper::is_self(&property.value, email.as_deref(), self_addresses),
                    is_organizer,
                )
            })
            .collect();
        (attendees, organizer)
    }

    /// The address as mail: from the value (`mailto:`), else from the `EMAIL` parameter (RFC 7986).
    pub(crate) fn email_of(property: &ICalProperty) -> Option<String> {
        CalendarUserAddress::email_from(Some(&property.value))
            .or_else(|| CalendarUserAddress::email_from(property.parameter("EMAIL")))
    }

    pub fn is_self(address: &str, email: Option<&str>, self_addresses: &BTreeSet<String>) -> bool {
        if self_addresses.contains(&address.to_lowercase()) {
            return true;
        }
        let Some(email) = email else {
            return false;
        };
        self_addresses.contains(&format!("mailto:{email}"))
    }

    pub(crate) fn role_of(property: &ICalProperty) -> AttendeeRole {
        if matches!(
            property
                .parameter("CUTYPE")
                .map(str::to_uppercase)
                .as_deref(),
            Some("RESOURCE" | "ROOM")
        ) {
            return AttendeeRole::Resource;
        }
        match property.parameter("ROLE").map(str::to_uppercase).as_deref() {
            Some("OPT-PARTICIPANT" | "NON-PARTICIPANT") => AttendeeRole::Optional,
            _ => AttendeeRole::Required,
        }
    }

    pub(crate) fn response_of(property: &ICalProperty) -> ResponseStatus {
        match property
            .parameter("PARTSTAT")
            .map(str::to_uppercase)
            .as_deref()
        {
            Some("ACCEPTED") => ResponseStatus::Accepted,
            Some("TENTATIVE") => ResponseStatus::Tentative,
            Some("DECLINED") => ResponseStatus::Declined,
            _ => ResponseStatus::NeedsAction,
        }
    }

    pub(crate) fn partstat(response: ResponseStatus) -> &'static str {
        match response {
            ResponseStatus::Accepted => "ACCEPTED",
            ResponseStatus::Tentative => "TENTATIVE",
            ResponseStatus::Declined => "DECLINED",
            ResponseStatus::NeedsAction => "NEEDS-ACTION",
        }
    }

    /// The same rule as the Google and Microsoft mappers: your attendee entry's response; an organizer who is you with
    /// no attendee entry is accepted; otherwise not invited. `None` when the account's own addresses are unknown.
    pub fn participation(
        attendees: &[Attendee],
        organizer: Option<&Attendee>,
        knows_self: bool,
    ) -> Option<Participation> {
        if !knows_self {
            return None;
        }
        if let Some(me) = attendees.iter().find(|attendee| attendee.is_self) {
            return Some(Participation::Invited(me.response));
        }
        if organizer.is_some_and(|o| o.is_self) {
            return Some(Participation::Invited(ResponseStatus::Accepted));
        }
        Some(Participation::NotInvited)
    }

    /// Swift `property(for:)`.
    pub fn property_for(draft: &AttendeeDraft) -> ICalProperty {
        let mut parameters: Vec<ICalParameter> = Vec::new();
        if let Some(name) = &draft.name
            && !name.is_empty()
        {
            parameters.push(ICalParameter::single("CN", name));
        }
        match draft.role {
            AttendeeRole::Required => {
                parameters.push(ICalParameter::single("ROLE", "REQ-PARTICIPANT"))
            }
            AttendeeRole::Optional => {
                parameters.push(ICalParameter::single("ROLE", "OPT-PARTICIPANT"))
            }
            AttendeeRole::Resource => {
                parameters.push(ICalParameter::single("ROLE", "REQ-PARTICIPANT"));
                parameters.push(ICalParameter::single("CUTYPE", "RESOURCE"));
            }
        }
        parameters.push(ICalParameter::single("PARTSTAT", "NEEDS-ACTION"));
        parameters.push(ICalParameter::single("RSVP", "TRUE"));
        ICalProperty::new("ATTENDEE", parameters, format!("mailto:{}", draft.email()))
    }

    pub fn organizer_property(address: &str) -> ICalProperty {
        ICalProperty::with_value("ORGANIZER", address)
    }

    /// Sets the account's own `PARTSTAT` and clears `RSVP` on each of its `ATTENDEE` entries. Returns false when the
    /// account is not an attendee.
    pub fn set_response(
        response: ResponseStatus,
        vevent: &mut ICalComponent,
        self_addresses: &BTreeSet<String>,
    ) -> bool {
        let mut found = false;
        for index in 0..vevent.properties.len() {
            if vevent.properties[index].name != "ATTENDEE" {
                continue;
            }
            let property = vevent.properties[index].clone();
            if !AttendeeMapper::is_self(
                &property.value,
                AttendeeMapper::email_of(&property).as_deref(),
                self_addresses,
            ) {
                continue;
            }
            vevent.properties[index]
                .set_parameter("PARTSTAT", Some(AttendeeMapper::partstat(response)));
            vevent.properties[index].set_parameter("RSVP", None);
            found = true;
        }
        found
    }
}
