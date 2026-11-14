//! Port of `EventPatchTests.swift`.

use super::support::{attendee, fields, hand_patch, iso, named_attendee, zone};
use crate::model::{
    AttendeeRole, Availability, CalendarEvent, ConferenceInfo, ConferenceOrigin,
    ConferenceProvider, EventStatus, Participation, Reminder, ReminderType, ResponseStatus,
    Visibility,
};
use crate::time::Zone;
use crate::url_string::UrlString;
use crate::write::event_draft::{AttendeeDraft, EventTiming};
use crate::write::event_patch::{AttendeeChanges, EventEdit, EventPatch, FieldUpdate};
use crate::write::types::{ConferenceChange, EventField};

fn utc() -> Zone {
    zone("UTC")
}

fn conference(url: &str, provider: ConferenceProvider) -> ConferenceInfo {
    ConferenceInfo::new(
        UrlString::parse(url).unwrap(),
        provider,
        ConferenceOrigin::Structured,
    )
}

fn original() -> CalendarEvent {
    let mut me = attendee("me@x.com");
    me.is_self = true;
    let mut cy = attendee("cy@x.com");
    cy.role = AttendeeRole::Optional;
    let mut organizer = attendee("me@x.com");
    organizer.is_self = true;
    organizer.is_organizer = true;
    CalendarEvent {
        uid: Some("u".into()),
        notes: Some("n".into()),
        location: Some("Room".into()),
        time_zone: utc(),
        status: EventStatus::Confirmed,
        attendees: vec![me, attendee("bob@x.com"), cy],
        organizer: Some(organizer),
        conferences: vec![conference(
            "https://meet.google.com/abc",
            ConferenceProvider::Meet,
        )],
        reminders: Some(vec![Reminder::with_minutes_before(10)]),
        url: UrlString::parse("https://x.test/e"),
        version: Some("v1".into()),
        participation: Some(Participation::Invited(ResponseStatus::Accepted)),
        ..CalendarEvent::new(
            "e",
            "c",
            "Planning",
            iso("2026-09-21T10:00:00Z"),
            iso("2026-09-21T11:00:00Z"),
        )
    }
}

fn draft_attendee(email: &str, name: Option<&str>, role: AttendeeRole) -> AttendeeDraft {
    AttendeeDraft::new(email, name, role)
}

fn changes(add: Vec<AttendeeDraft>, remove: &[&str]) -> AttendeeChanges {
    AttendeeChanges::new(add, remove.iter().map(|s| s.to_string()).collect())
}

fn index_of(event: &CalendarEvent, email: &str) -> usize {
    event
        .attendees
        .iter()
        .position(|a| a.email() == Some(email))
        .unwrap()
}

#[test]
fn an_untouched_edit_produces_an_empty_patch() {
    let edit = EventEdit::new(original());
    assert!(
        !edit.has_changes() && edit.patch().is_empty() && edit.patch().touched_fields().is_empty()
    );
}

#[test]
fn diff_captures_scalar_changes_and_clears() {
    let mut edit = EventEdit::new(original());
    edit.event.title = "Planning v2".into();
    edit.event.notes = None;
    edit.event.location = Some("Lab".into());
    edit.event.availability = Some(Availability::Free);
    edit.event.visibility = Some(Visibility::PrivateEvent);
    let patch = edit.patch();
    assert_eq!(patch.title.as_deref(), Some("Planning v2"));
    assert!(
        patch.notes == FieldUpdate::Clear && patch.location == FieldUpdate::Set("Lab".to_string())
    );
    assert!(
        patch.availability == Some(Availability::Free)
            && patch.visibility == Some(Visibility::PrivateEvent)
    );
    assert_eq!(
        patch.touched_fields(),
        fields(&[
            EventField::Title,
            EventField::Notes,
            EventField::Location,
            EventField::Availability,
            EventField::Visibility
        ])
    );
    assert_eq!(patch.base(), Some(&original()));
}

#[test]
fn diff_treats_time_as_one_unit() {
    let mut edit = EventEdit::new(original());
    edit.event.end = iso("2026-09-21T11:30:00Z");
    let patch = edit.patch();
    assert_eq!(
        patch.timing,
        Some(EventTiming::new(
            iso("2026-09-21T10:00:00Z"),
            iso("2026-09-21T11:30:00Z"),
            Some(utc()),
            false
        ))
    );
    assert_eq!(patch.touched_fields(), fields(&[EventField::Timing]));
}

#[test]
fn diff_builds_an_attendee_delta_by_email() {
    let mut edit = EventEdit::new(original());
    edit.event
        .attendees
        .retain(|a| a.email() != Some("bob@x.com")); // removed
    edit.event
        .attendees
        .push(named_attendee("Dee", Some("Dee@x.com"))); // added
    let cy = index_of(&edit.event, "cy@x.com");
    edit.event.attendees[cy].role = AttendeeRole::Required; // role changed: upsert
    let patch = edit.patch();
    let delta = patch.attendees.as_ref().unwrap();
    assert_eq!(delta.remove, vec!["bob@x.com".to_string()]);
    // Upserts follow the edited list's order (cy was already there, dee was appended).
    assert_eq!(
        delta.add,
        vec![
            draft_attendee("cy@x.com", None, AttendeeRole::Required),
            draft_attendee("dee@x.com", Some("Dee"), AttendeeRole::Required)
        ]
    );
    assert_eq!(patch.touched_fields(), fields(&[EventField::Attendees]));
}

#[test]
fn diff_ignores_provider_owned_fields_and_unwritable_conference_changes() {
    let mut edit = EventEdit::new(original());
    edit.event.status = EventStatus::Tentative;
    edit.event.participation = Some(Participation::Invited(ResponseStatus::Declined));
    edit.event.version = Some("v9".into());
    edit.event.url = None;
    edit.event.organizer = None;
    edit.event.source_id = Some("other".into());
    // changed, not removable
    edit.event.conferences = vec![conference("https://zoom.us/j/1", ConferenceProvider::Zoom)];
    assert!(edit.patch().is_empty());
    edit.event.conferences = vec![];
    assert!(edit.patch().conference == Some(ConferenceChange::Remove));
    assert_eq!(
        edit.patch().touched_fields(),
        fields(&[EventField::Conference])
    );
}

#[test]
fn diff_sets_reminders_explicitly() {
    let mut edit = EventEdit::new(original());
    edit.event.reminders = Some(vec![]);
    assert!(edit.patch().reminders == FieldUpdate::Set(vec![]));
    edit.event.reminders = Some(vec![
        Reminder::with_minutes_before(5),
        Reminder::with_minutes_before(30),
    ]);
    assert!(
        edit.patch().reminders
            == FieldUpdate::Set(vec![
                Reminder::with_minutes_before(5),
                Reminder::with_minutes_before(30)
            ])
    );
}

#[test]
fn attendees_without_an_email_are_ignored_by_the_diff() {
    let mut edit = EventEdit::new(original());
    edit.event.attendees.push(named_attendee("Ghost", None));
    assert!(edit.patch().is_empty());
}

#[test]
fn a_hand_built_patch_has_no_base_and_reports_touched_fields() {
    let patch = hand_patch(|p| {
        p.title = Some("X".into());
        p.recurrence = FieldUpdate::Clear;
        p.conference = Some(ConferenceChange::Generate);
    });
    assert!(patch.base().is_none());
    assert_eq!(
        patch.touched_fields(),
        fields(&[
            EventField::Title,
            EventField::Recurrence,
            EventField::Conference
        ])
    );
    // an empty delta touches nothing
    assert!(hand_patch(|p| p.attendees = Some(changes(vec![], &[]))).is_empty());
    assert_eq!(
        EventEdit::new(original()).patch().without_base(),
        EventPatch::default()
    );
}

#[test]
fn applied_to_rewrites_every_representable_field() {
    let patch = hand_patch(|p| {
        p.title = Some("New".into());
        p.notes = FieldUpdate::Clear;
        p.location = FieldUpdate::Set("Lab".into());
        p.timing = Some(EventTiming::new(
            iso("2026-09-22T10:00:00Z"),
            iso("2026-09-22T11:00:00Z"),
            Some(utc()),
            false,
        ));
        p.availability = Some(Availability::Free);
        p.visibility = Some(Visibility::Confidential);
        p.reminders = FieldUpdate::Set(vec![]);
        p.attendees = Some(changes(
            vec![
                draft_attendee("dee@x.com", Some("Dee"), AttendeeRole::Required),
                draft_attendee("cy@x.com", None, AttendeeRole::Required),
            ],
            &["bob@x.com"],
        ));
        p.conference = Some(ConferenceChange::Remove);
    });
    let e = patch.applied_to(&original());
    assert!(e.title == "New" && e.notes.is_none() && e.location.as_deref() == Some("Lab"));
    assert!(
        e.start == iso("2026-09-22T10:00:00Z")
            && e.availability == Some(Availability::Free)
            && e.visibility == Some(Visibility::Confidential)
    );
    assert!(e.reminders == Some(vec![]) && e.conference().is_none());
    let emails: Vec<_> = e
        .attendees
        .iter()
        .map(|a| a.email().unwrap().to_string())
        .collect();
    assert_eq!(emails, vec!["me@x.com", "cy@x.com", "dee@x.com"]);
    assert_eq!(
        e.attendees[index_of(&e, "cy@x.com")].role,
        AttendeeRole::Required
    );
}

#[test]
fn duplicate_emails_in_provider_data_do_not_trap_and_the_first_wins() {
    let mut event = original();
    let mut again = named_attendee("Bob again", Some("BOB@x.com"));
    again.role = AttendeeRole::Optional;
    event.attendees.push(again);
    let mut edit = EventEdit::new(event);
    assert!(edit.patch().is_empty());
    edit.event
        .attendees
        .retain(|a| a.email() != Some("bob@x.com"));
    assert_eq!(
        edit.patch().attendees.unwrap().remove,
        vec!["bob@x.com".to_string()]
    );
}

#[test]
fn clearing_an_attendee_name_alone_is_not_a_change() {
    // A draft cannot clear a name, so emitting an upsert would touch attendees (and may notify) and change nothing.
    let mut event = original();
    event.attendees[1].name = Some("Bob".into());
    let mut edit = EventEdit::new(event);
    edit.event.attendees[1].name = None;
    assert!(edit.patch().is_empty());
    edit.event.attendees[1].role = AttendeeRole::Optional;
    assert_eq!(
        edit.patch().attendees.unwrap().add,
        vec![draft_attendee("bob@x.com", None, AttendeeRole::Optional)]
    );
}

#[test]
fn a_name_only_change_to_a_value_is_an_upsert() {
    let mut edit = EventEdit::new(original());
    edit.event.attendees[1].name = Some("Bob".into()); // nil -> "Bob"
    let patch = edit.patch();
    let delta = patch.attendees.as_ref().unwrap();
    assert_eq!(
        delta.add,
        vec![draft_attendee(
            "bob@x.com",
            Some("Bob"),
            AttendeeRole::Required
        )]
    );
    assert!(delta.remove.is_empty() && patch.touched_fields() == fields(&[EventField::Attendees]));
    let mut renamed = original();
    renamed.attendees[1].name = Some("Bob".into());
    let mut second = EventEdit::new(renamed);
    second.event.attendees[1].name = Some("Robert".into()); // "Bob" -> "Robert"
    let patch = second.patch();
    assert_eq!(
        patch.attendees.as_ref().unwrap().add,
        vec![draft_attendee(
            "bob@x.com",
            Some("Robert"),
            AttendeeRole::Required
        )]
    );
    assert_eq!(patch.touched_fields(), fields(&[EventField::Attendees]));
}

#[test]
fn rebuilding_the_self_attendee_without_the_self_flag_is_not_an_addition() {
    let mut edit = EventEdit::new(original());
    edit.event.attendees[0] = named_attendee("Me", Some("ME@x.com"));
    assert!(edit.patch().is_empty());
}

#[test]
fn applying_a_diff_reproduces_the_edited_writable_fields() {
    let mut edit = EventEdit::new(original());
    edit.event.title = "T".into();
    edit.event
        .attendees
        .retain(|a| a.email() != Some("bob@x.com"));
    edit.event
        .attendees
        .push(named_attendee("Dee", Some("dee@x.com")));
    edit.event.attendees[1].role = AttendeeRole::Required;
    edit.event.reminders = Some(vec![]);
    edit.event.conferences = vec![];
    let result = edit.patch().applied_to(&original());
    assert!(
        result.title == "T" && result.reminders == Some(vec![]) && result.conference().is_none()
    );
    let emails = |e: &CalendarEvent| -> std::collections::BTreeSet<String> {
        e.attendees
            .iter()
            .map(|a| a.email().unwrap().to_string())
            .collect()
    };
    assert_eq!(emails(&result), emails(&edit.event));
    assert_eq!(
        result.attendees[index_of(&result, "cy@x.com")].role,
        AttendeeRole::Required
    );
}

#[test]
fn applying_a_patch_matches_attendee_emails_case_insensitively() {
    let mut event = original();
    let mut bob = attendee("Bob@X.com");
    bob.role = AttendeeRole::Required;
    event.attendees = vec![bob, attendee("Cy@X.com")];
    // `Attendee` lowercases its address on creation, so a provider's mixed-case address is already normalised:
    // an upsert updates that attendee instead of appending a duplicate.
    let upserted = hand_patch(|p| {
        p.attendees = Some(changes(
            vec![draft_attendee("bob@x.com", None, AttendeeRole::Optional)],
            &[],
        ));
    })
    .applied_to(&event);
    assert!(upserted.attendees.len() == 2 && upserted.attendees[0].role == AttendeeRole::Optional);
    // A removal by normalised address removes the mixed-case attendee.
    let removed =
        hand_patch(|p| p.attendees = Some(changes(vec![], &["CY@x.com"]))).applied_to(&event);
    let emails: Vec<_> = removed
        .attendees
        .iter()
        .map(|a| a.email().unwrap().to_string())
        .collect();
    assert_eq!(emails, vec!["bob@x.com"]);
}

#[test]
fn a_nil_availability_visibility_or_reminders_on_the_edited_copy_is_never_a_change() {
    let mut edit = EventEdit::new(original());
    edit.event.availability = None;
    edit.event.visibility = None;
    edit.event.reminders = None;
    let patch = edit.patch();
    assert!(
        patch.availability.is_none()
            && patch.visibility.is_none()
            && patch.reminders == FieldUpdate::Keep
    );
}

#[test]
fn clearing_reminders_leaves_them_unknown_on_the_in_memory_result() {
    let cleared = hand_patch(|p| p.reminders = FieldUpdate::Clear).applied_to(&original());
    assert_eq!(cleared.reminders, None);
}

#[test]
fn a_reordered_or_default_flagged_reminder_list_is_not_a_change_but_a_new_one_is() {
    let before = |m: i64| Reminder::before(m, ReminderType::Display, None);
    let mut base = original();
    base.reminders = Some(vec![before(10), before(20)]);
    let same_set = EventEdit::new(base);
    let mut reordered = same_set.clone();
    reordered.event.reminders = Some(vec![
        Reminder::before(20, ReminderType::Display, Some(false)),
        before(10),
    ]);
    assert!(reordered.patch().reminders == FieldUpdate::Keep);
    let mut changed = same_set.clone();
    changed.event.reminders = Some(vec![before(20)]);
    assert!(changed.patch().reminders == FieldUpdate::Set(vec![before(20)]));
    let mut defaults = same_set.clone();
    defaults.event.reminders = Some(vec![Reminder::before(
        10,
        ReminderType::Display,
        Some(true),
    )]);
    assert!(defaults.patch().reminders == FieldUpdate::Clear);
}

#[test]
fn removed_emails_are_ordered_by_scalar_value() {
    // Swift sorts the removals with `<` on `String`; for text in canonical (NFC) form that is scalar-value order,
    // which is also Rust's byte order. A non-ASCII address sorts after every ASCII one.
    let mut event = original();
    event.attendees = vec![
        attendee("z@x.com"),
        attendee("\u{e9}@x.com"),
        attendee("a@x.com"),
    ];
    let mut edit = EventEdit::new(event);
    edit.event.attendees.clear();
    assert_eq!(
        edit.patch().attendees.unwrap().remove,
        vec![
            "a@x.com".to_string(),
            "z@x.com".to_string(),
            "\u{e9}@x.com".to_string()
        ]
    );
}
