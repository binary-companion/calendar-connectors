//! Port of `PatchMergeTests.swift`. `cancellationStopsTheRetryLoop` becomes a drop test: Swift's loop polls
//! `Task.checkCancellation()`; a Rust future that is dropped simply stops (conventions section 10, item 11).

use std::cell::{Cell, RefCell};

use super::support::{attendee, expect_write_error, fields, hand_patch, iso, zone};
use crate::failure::{Failure, Outcome};
use crate::model::{AttendeeRole, CalendarEvent, Reminder, ReminderType};
use crate::write::event_patch::{EventEdit, EventPatch, FieldUpdate};
use crate::write::patch_merge::{Attempt, PatchMerge};
use crate::write::types::{EventField, WriteError};

fn base() -> CalendarEvent {
    let mut bob = attendee("bob@x.com");
    bob.role = AttendeeRole::Required;
    CalendarEvent {
        notes: Some("n".into()),
        location: Some("Room".into()),
        time_zone: zone("UTC"),
        attendees: vec![bob],
        version: Some("v1".into()),
        ..CalendarEvent::new(
            "e",
            "c",
            "Planning",
            iso("2026-09-21T10:00:00Z"),
            iso("2026-09-21T11:00:00Z"),
        )
    }
}

fn patch(mutate: impl FnOnce(&mut EventEdit)) -> EventPatch {
    let mut edit = EventEdit::new(base());
    mutate(&mut edit);
    edit.patch()
}

type Fetched = std::future::Ready<Outcome<CalendarEvent>>;

fn ready_event(event: CalendarEvent) -> Fetched {
    std::future::ready(Ok(event))
}

fn done(value: &str) -> std::future::Ready<Outcome<Attempt<String>>> {
    std::future::ready(Ok(Attempt::Done(value.to_string())))
}

fn stale() -> std::future::Ready<Outcome<Attempt<String>>> {
    std::future::ready(Ok(Attempt::Stale))
}

fn title_patch() -> EventPatch {
    patch(|e| e.event.title = "New".into())
}

#[test]
fn no_conflict_when_others_changed_different_fields() {
    let mut current = base();
    current.location = Some("Elsewhere".into());
    current.version = Some("v2".into());
    assert!(PatchMerge::conflicts(&title_patch(), &current).is_empty());
}

#[test]
fn a_touched_field_that_changed_elsewhere_conflicts() {
    let mut current = base();
    current.title = "Changed".into();
    assert_eq!(
        PatchMerge::conflicts(&title_patch(), &current),
        fields(&[EventField::Title])
    );
}

#[test]
fn timing_is_compared_as_a_unit() {
    let mut current = base();
    current.end = iso("2026-09-21T11:30:00Z"); // someone moved only the end
    let mine = patch(|e| e.event.start = iso("2026-09-21T09:30:00Z"));
    assert_eq!(
        PatchMerge::conflicts(&mine, &current),
        fields(&[EventField::Timing])
    );
}

#[test]
fn attendees_conflict_only_for_the_emails_the_patch_touches() {
    let mut current = base();
    current.attendees.push(attendee("new@x.com")); // unrelated addition
    let add_dee = patch(|e| e.event.attendees.push(attendee("dee@x.com")));
    assert!(PatchMerge::conflicts(&add_dee, &current).is_empty());
    current.attendees[0].role = AttendeeRole::Optional; // someone changed bob's role
    let change_bob = patch(|e| e.event.attendees[0].role = AttendeeRole::Resource);
    assert_eq!(
        PatchMerge::conflicts(&change_bob, &current),
        fields(&[EventField::Attendees])
    );
    let remove_bob = patch(|e| e.event.attendees.clear());
    assert_eq!(
        PatchMerge::conflicts(&remove_bob, &current),
        fields(&[EventField::Attendees])
    );
}

#[test]
fn a_patch_without_a_base_conflicts_on_every_touched_field() {
    let hand_built = hand_patch(|p| {
        p.title = Some("X".into());
        p.location = FieldUpdate::Set("Y".into());
    });
    assert_eq!(
        PatchMerge::conflicts(&hand_built, &base()),
        fields(&[EventField::Title, EventField::Location])
    );
}

#[test]
fn recurrence_cannot_be_judged_so_it_conflicts() {
    let mut edit = EventEdit::new(base());
    edit.event.title = "New".into();
    let mut p = edit.patch();
    p.recurrence = FieldUpdate::Clear;
    assert_eq!(
        PatchMerge::conflicts(&p, &base()),
        fields(&[EventField::Recurrence])
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn apply_returns_the_first_successful_write() {
    let result = PatchMerge::apply(
        &hand_patch(|p| p.title = Some("X".into())),
        Some("v1".into()),
        3,
        || ready_event(base()),
        |_| done("ok"),
    )
    .await;
    assert_eq!(result.unwrap(), "ok");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn apply_retries_on_the_fresh_version_when_nothing_overlaps() {
    let mut current = base();
    current.location = Some("Elsewhere".into());
    current.version = Some("v2".into());
    let seen = RefCell::new(Vec::<Option<String>>::new());
    let result = PatchMerge::apply(
        &title_patch(),
        Some("v1".into()),
        3,
        || ready_event(current.clone()),
        |version| {
            let saved = version.as_deref() == Some("v2");
            seen.borrow_mut().push(version);
            if saved { done("saved") } else { stale() }
        },
    )
    .await;
    assert_eq!(result.unwrap(), "saved");
    assert_eq!(
        *seen.borrow(),
        vec![Some("v1".to_string()), Some("v2".to_string())]
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn apply_judges_every_second_stale_result_again() {
    let fetched = Cell::new(0);
    let result: Outcome<String> = PatchMerge::apply(
        &title_patch(),
        Some("v1".into()),
        3,
        || {
            fetched.set(fetched.get() + 1);
            let mut current = base();
            current.version = Some(format!("v{}", fetched.get() + 1));
            if fetched.get() == 2 {
                current.title = "Changed".into(); // the second concurrent edit overlaps
            }
            ready_event(current)
        },
        |_| stale(),
    )
    .await;
    expect_write_error(
        WriteError::Conflict {
            fields: fields(&[EventField::Title]),
        },
        result,
    );
    assert_eq!(fetched.get(), 2);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn apply_gives_up_after_max_attempts() {
    let writes = Cell::new(0);
    let result: Outcome<String> = PatchMerge::apply(
        &title_patch(),
        Some("v1".into()),
        3,
        || ready_event(base()),
        |_| {
            writes.set(writes.get() + 1);
            stale()
        },
    )
    .await;
    expect_write_error(
        WriteError::Conflict {
            fields: fields(&[EventField::Title]),
        },
        result,
    );
    assert_eq!(writes.get(), 3);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_baseless_patch_fails_at_the_first_stale_version() {
    let fetched = Cell::new(0);
    let result: Outcome<String> = PatchMerge::apply(
        &hand_patch(|p| p.title = Some("X".into())),
        Some("v1".into()),
        PatchMerge::DEFAULT_MAX_ATTEMPTS,
        || {
            fetched.set(fetched.get() + 1);
            ready_event(base())
        },
        |_| stale(),
    )
    .await;
    expect_write_error(
        WriteError::Conflict {
            fields: fields(&[EventField::Title]),
        },
        result,
    );
    assert_eq!(fetched.get(), 1);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_retry_without_a_fresh_version_fails_closed_when_the_original_was_versioned() {
    let mut current = base();
    current.location = Some("Elsewhere".into()); // no overlap with a title patch, so a retry would be due
    current.version = None;
    let writes = Cell::new(0);
    let result: Outcome<String> = PatchMerge::apply(
        &title_patch(),
        Some("v1".into()),
        PatchMerge::DEFAULT_MAX_ATTEMPTS,
        || ready_event(current.clone()),
        |_| {
            writes.set(writes.get() + 1);
            stale()
        },
    )
    .await;
    expect_write_error(
        WriteError::Conflict {
            fields: fields(&[EventField::Title]),
        },
        result,
    );
    assert_eq!(writes.get(), 1); // never retried unconditionally
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_nil_original_version_still_retries_without_one() {
    let mut current = base();
    current.location = Some("Elsewhere".into());
    current.version = None;
    let seen = RefCell::new(Vec::<Option<String>>::new());
    let result = PatchMerge::apply(
        &title_patch(),
        None,
        PatchMerge::DEFAULT_MAX_ATTEMPTS,
        || ready_event(current.clone()),
        |version| {
            seen.borrow_mut().push(version);
            if seen.borrow().len() == 2 {
                done("saved")
            } else {
                stale()
            }
        },
    )
    .await;
    assert_eq!(result.unwrap(), "saved");
    assert_eq!(*seen.borrow(), vec![None, None]);
}

// MARK: edge cases beyond the plan

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_non_positive_max_attempts_still_writes_once() {
    for attempts in [0, -1, i64::MIN] {
        let writes = Cell::new(0);
        let result = PatchMerge::apply(
            &hand_patch(|p| p.title = Some("X".into())),
            Some("v1".into()),
            attempts,
            || ready_event(base()),
            |_| {
                writes.set(writes.get() + 1);
                done("ok")
            },
        )
        .await;
        assert!(
            result.unwrap() == "ok" && writes.get() == 1,
            "attempts {attempts}"
        );
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn cancellation_stops_the_retry_loop() {
    // Swift cancels the task from inside `write` and the next iteration throws. Rust has no cooperative checks: the
    // caller drops the future. Here the first write yields once, then the race below drops the loop; no second
    // write may happen.
    let writes = Cell::new(0);
    let empty = EventPatch::default();
    let apply = PatchMerge::apply(
        &empty,
        Some("v1".into()),
        10,
        || ready_event(base()),
        |_| {
            writes.set(writes.get() + 1);
            async {
                tokio::task::yield_now().await;
                Ok::<Attempt<String>, Failure>(Attempt::Stale)
            }
        },
    );
    tokio::select! {
        biased;
        _ = apply => panic!("the loop must not finish before it is dropped"),
        _ = async { while writes.get() < 1 { tokio::task::yield_now().await; } } => {}
    }
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    assert_eq!(writes.get(), 1);
}

#[test]
fn attendee_already_in_the_wanted_state_is_not_a_conflict() {
    // Someone else added dee with the same role, or already removed bob: the patch's outcome already holds.
    let mut current = base();
    let mut dee = attendee("dee@x.com");
    dee.role = AttendeeRole::Required;
    current.attendees.push(dee);
    let add_dee = patch(|e| e.event.attendees.push(attendee("dee@x.com")));
    assert!(PatchMerge::conflicts(&add_dee, &current).is_empty());
    current.attendees[1].role = AttendeeRole::Optional; // ...but with a different role: real conflict
    assert_eq!(
        PatchMerge::conflicts(&add_dee, &current),
        fields(&[EventField::Attendees])
    );

    let mut gone = base();
    gone.attendees.clear();
    let remove_bob = patch(|e| e.event.attendees.clear());
    assert!(PatchMerge::conflicts(&remove_bob, &gone).is_empty());
}

#[test]
fn empty_and_missing_notes_and_location_are_the_same() {
    let mut current = base();
    current.notes = None;
    current.location = Some(String::new());
    let mut edited_base = base();
    edited_base.notes = Some(String::new());
    edited_base.location = None;
    let mut edit = EventEdit::new(edited_base);
    edit.event.notes = Some("x".into());
    edit.event.location = Some("y".into());
    assert!(PatchMerge::conflicts(&edit.patch(), &current).is_empty());
}

#[test]
fn reminder_order_is_not_a_conflict() {
    let mut base_with_reminders = base();
    base_with_reminders.reminders = Some(vec![
        Reminder::with_minutes_before(10),
        Reminder::with_minutes_before(30),
    ]);
    let mut edit = EventEdit::new(base_with_reminders.clone());
    edit.event.reminders = Some(vec![Reminder::with_minutes_before(5)]);
    let mut current = base_with_reminders;
    current.reminders = Some(vec![
        Reminder::with_minutes_before(30),
        Reminder::with_minutes_before(10),
    ]);
    assert!(PatchMerge::conflicts(&edit.patch(), &current).is_empty());
    current.reminders = Some(vec![Reminder::with_minutes_before(30)]);
    assert_eq!(
        PatchMerge::conflicts(&edit.patch(), &current),
        fields(&[EventField::Reminders])
    );
}

#[test]
fn attendee_emails_are_compared_case_insensitively() {
    // The provider stores a mixed-case address; the patch carries the normalised (lowercase) one.
    let mut mixed = base();
    let mut bob = attendee("Bob@X.com");
    bob.role = AttendeeRole::Required;
    mixed.attendees = vec![bob];
    let mut edit = EventEdit::new(mixed.clone());
    edit.event.attendees.clear();
    let remove_bob = edit.patch();
    assert_eq!(
        remove_bob.attendees.as_ref().unwrap().remove,
        vec!["bob@x.com".to_string()]
    );
    let mut current = mixed.clone();
    current.attendees[0].role = AttendeeRole::Optional; // someone changed bob's role meanwhile
    assert_eq!(
        PatchMerge::conflicts(&remove_bob, &current),
        fields(&[EventField::Attendees])
    );

    let mut upsert = EventEdit::new(mixed);
    upsert.event.attendees[0].role = AttendeeRole::Resource;
    assert_eq!(
        PatchMerge::conflicts(&upsert.patch(), &current),
        fields(&[EventField::Attendees])
    );
}

#[test]
fn a_reordered_reminder_list_is_not_a_conflict_but_a_changed_alert_type_is() {
    let mut mine = base();
    mine.reminders = Some(vec![
        Reminder::with_minutes_before(10),
        Reminder::with_minutes_before(60),
    ]);
    let mut edit = EventEdit::new(mine.clone());
    edit.event.title = "New".into();
    edit.event.reminders = Some(vec![Reminder::with_minutes_before(5)]);
    let mut current = mine;
    // reordered, flagged by the provider
    current.reminders = Some(vec![
        Reminder::before(60, ReminderType::Display, Some(false)),
        Reminder::with_minutes_before(10),
    ]);
    assert!(PatchMerge::conflicts(&edit.patch(), &current).is_empty());
    current.reminders = Some(vec![
        Reminder::before(60, ReminderType::Email { address: None }, None),
        Reminder::with_minutes_before(10),
    ]);
    assert_eq!(
        PatchMerge::conflicts(&edit.patch(), &current),
        fields(&[EventField::Reminders])
    );
}
