//! Port of `WritableSourceTests.swift`.

use std::collections::BTreeSet;

use async_trait::async_trait;
use calendar_core::failure::{Failure, Outcome};
use calendar_core::model::{Attendee, CalendarDescriptor, CalendarEvent, Reminder, ResponseStatus};
use calendar_core::recurrence::rule::{Frequency, RecurrenceRule};
use calendar_core::source_types::{CalendarChange, CalendarSource, SourceCapabilities};
use calendar_core::time::{DateInterval, add_seconds};
use calendar_core::write::event_draft::{AttendeeDraft, EventDraft, EventTiming};
use calendar_core::write::event_patch::{AttendeeChanges, EventEdit, EventPatch, FieldUpdate};
use calendar_core::write::types::{
    EventField, EventRef, NotifyPolicy, RecurrenceScope, WriteError,
};
use calendar_core::write::writable_source::WritableCalendarSource;
use tokio::sync::mpsc::UnboundedReceiver;

use super::support::{at, expect_write_error, zone};
use crate::fake_writable_source::FakeWritableSource;
use crate::hand_patch;
use crate::writable_source_conformance::WritableSourceConformance;

fn window() -> DateInterval {
    DateInterval::new(at(1_790_000_000), at(1_790_000_000 + 86_400 * 3))
}

fn draft(title: &str) -> EventDraft {
    let start = add_seconds(window().start, 3600.0);
    EventDraft::new(
        title,
        EventTiming::new(start, add_seconds(start, 1800.0), Some(zone("UTC")), false),
    )
}

fn fields(list: &[EventField]) -> BTreeSet<EventField> {
    list.iter().copied().collect()
}

fn fake_with_fields(list: &[EventField]) -> FakeWritableSource {
    FakeWritableSource::with_options(&["cal"], fields(list), true)
}

async fn create(source: &FakeWritableSource, draft: &EventDraft) -> Outcome<CalendarEvent> {
    source.create(draft, "cal", NotifyPolicy::None).await
}

async fn update(
    source: &FakeWritableSource,
    reference: &EventRef,
    patch: &EventPatch,
) -> Outcome<CalendarEvent> {
    source
        .update(
            reference,
            patch,
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_fake_passes_the_conformance_checks() {
    let source = FakeWritableSource::new();
    assert!(
        WritableSourceConformance::violations(&source, "cal", window())
            .await
            .is_empty()
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_fake_without_attendee_support_still_conforms() {
    let source = FakeWritableSource::with_options(
        &["cal"],
        fields(&[
            EventField::Title,
            EventField::Notes,
            EventField::Location,
            EventField::Timing,
        ]),
        false,
    );
    assert!(!source.capabilities().can_edit_attendees);
    assert!(
        WritableSourceConformance::violations(&source, "cal", window())
            .await
            .is_empty()
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_source_that_cannot_write_location_still_conforms() {
    let source = fake_with_fields(&[EventField::Title, EventField::Timing]);
    assert!(
        WritableSourceConformance::violations(&source, "cal", window())
            .await
            .is_empty()
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn create_reads_back_and_stamps_the_source() {
    let source = FakeWritableSource::new();
    let created = create(&source, &draft("Sync")).await.unwrap();
    assert!(
        created.title == "Sync"
            && created.calendar_id == "cal"
            && created.source_id.as_deref() == Some(source.id().as_str())
            && created.version.is_some()
    );
    let listed: Vec<String> = source
        .events_in(window())
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.event_id)
        .collect();
    assert_eq!(listed, vec![created.event_id.clone()]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_unknown_calendar_is_not_found_and_unwritable_fields_are_refused() {
    let source = fake_with_fields(&[EventField::Title, EventField::Timing]);
    expect_write_error(
        WriteError::NotFound,
        source
            .create(&draft("Sync"), "nope", NotifyPolicy::None)
            .await,
    );
    let mut with_notes = draft("Sync");
    with_notes.notes = Some("n".into());
    expect_write_error(
        WriteError::Unsupported {
            fields: fields(&[EventField::Notes]),
        },
        create(&source, &with_notes).await,
    );
    assert_eq!(source.write_count(), 0);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_empty_patch_writes_nothing() {
    let source = FakeWritableSource::new();
    let created = create(&source, &draft("Sync")).await.unwrap();
    let before = source.write_count();
    let same = update(
        &source,
        &EventRef::from_event(&created),
        &EventPatch::default(),
    )
    .await
    .unwrap();
    assert!(same == created && source.write_count() == before);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_empty_patch_with_a_base_returns_the_base_and_writes_nothing() {
    let source = FakeWritableSource::new();
    let created = create(&source, &draft("Sync")).await.unwrap();
    assert!(
        source.simulate_external_edit("cal", &created.event_id, |e| {
            e.location = Some("Elsewhere".into());
        })
    );
    let before = source.write_count();
    let empty = EventEdit::new(created.clone()).patch(); // nothing edited, but it carries `created` as its base
    assert!(empty.is_empty() && empty.base() == Some(&created));
    let result = update(&source, &EventRef::from_event(&created), &empty)
        .await
        .unwrap();
    assert!(result == created && source.write_count() == before);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn simulate_external_edit_reports_whether_the_event_exists() {
    let source = FakeWritableSource::new();
    let created = create(&source, &draft("Sync")).await.unwrap();
    assert!(source.simulate_external_edit("cal", &created.event_id, |e| e.title = "Theirs".into()));
    assert!(!source.simulate_external_edit("cal", "ghost", |e| e.title = "Theirs".into()));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_fake_does_not_model_recurrence_so_it_refuses_it() {
    let source = FakeWritableSource::new();
    assert!(
        !source
            .capabilities()
            .writable_fields
            .contains(&EventField::Recurrence)
    );
    let mut recurring = draft("Sync");
    recurring.recurrence = Some(RecurrenceRule::new(Frequency::Daily));
    let recurrence_only = WriteError::Unsupported {
        fields: fields(&[EventField::Recurrence]),
    };
    expect_write_error(recurrence_only.clone(), create(&source, &recurring).await);
    let created = create(&source, &draft("Sync")).await.unwrap();
    expect_write_error(
        recurrence_only,
        update(
            &source,
            &EventRef::from_event(&created),
            &hand_patch(|p| p.recurrence = FieldUpdate::Set(RecurrenceRule::new(Frequency::Daily))),
        )
        .await,
    );
    assert_eq!(source.write_count(), 1); // only the plain create
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_stale_version_merges_when_fields_do_not_overlap() {
    let source = FakeWritableSource::new();
    let created = create(&source, &draft("Sync")).await.unwrap();
    source.simulate_external_edit("cal", &created.event_id, |e| {
        e.location = Some("Elsewhere".into())
    });
    let mut edit = EventEdit::new(created.clone());
    edit.event.title = "Renamed".into();
    let updated = update(&source, &EventRef::from_event(&created), &edit.patch())
        .await
        .unwrap();
    assert!(updated.title == "Renamed" && updated.location.as_deref() == Some("Elsewhere"));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_stale_version_conflicts_when_the_same_field_changed() {
    let source = FakeWritableSource::new();
    let created = create(&source, &draft("Sync")).await.unwrap();
    source.simulate_external_edit("cal", &created.event_id, |e| e.title = "Theirs".into());
    let mut edit = EventEdit::new(created.clone());
    edit.event.title = "Mine".into();
    expect_write_error(
        WriteError::Conflict {
            fields: fields(&[EventField::Title]),
        },
        update(&source, &EventRef::from_event(&created), &edit.patch()).await,
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn respond_is_unsupported_when_the_source_cannot_rsvp() {
    let source = FakeWritableSource::with_options(
        &["cal"],
        FakeWritableSource::default_writable_fields(),
        false,
    );
    let created = create(&source, &draft("Sync")).await.unwrap();
    expect_write_error(
        WriteError::Unsupported {
            fields: fields(&[EventField::Attendees]),
        },
        source
            .respond(
                &EventRef::from_event(&created),
                ResponseStatus::Accepted,
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
    );
}

#[test]
fn capabilities_follow_the_invariants() {
    let full = FakeWritableSource::new().capabilities();
    assert!(
        full.can_write
            && full.can_edit_attendees == full.writable_fields.contains(&EventField::Attendees)
    );
    let limited = fake_with_fields(&[EventField::Title, EventField::Timing]).capabilities();
    assert!(
        !limited.can_edit_attendees
            && limited.writable_fields == fields(&[EventField::Title, EventField::Timing])
    );
}

// MARK: - Fake behaviour beyond the brief

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn respond_updates_the_self_attendee_and_refuses_needs_action() {
    let source = FakeWritableSource::new();
    let created = create(&source, &draft("Sync")).await.unwrap();
    let reference = EventRef::from_event(&created);
    let respond = |response| {
        source.respond(
            &reference,
            response,
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
    };
    expect_write_error(
        WriteError::Invalid("you are not an attendee of this event".into()),
        respond(ResponseStatus::Accepted).await,
    );
    source.simulate_external_edit("cal", &created.event_id, |e| {
        let mut me = Attendee::default();
        me = Attendee::new(
            None,
            Some("me@example.com"),
            me.role,
            me.response,
            true,
            false,
        );
        e.attendees = vec![me];
    });
    expect_write_error(
        WriteError::Invalid("cannot respond with needsAction".into()),
        respond(ResponseStatus::NeedsAction).await,
    );
    let answered = respond(ResponseStatus::Declined).await.unwrap();
    assert!(
        answered.my_response() == Some(ResponseStatus::Declined)
            && answered.attendees.first().map(|a| a.response) == Some(ResponseStatus::Declined)
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn update_and_delete_of_a_missing_event_are_not_found() {
    let source = FakeWritableSource::new();
    let reference = EventRef::new("cal", "ghost");
    expect_write_error(
        WriteError::NotFound,
        update(
            &source,
            &reference,
            &hand_patch(|p| p.title = Some("x".into())),
        )
        .await,
    );
    expect_write_error(
        WriteError::NotFound,
        source
            .delete(
                &reference,
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
    );
    assert_eq!(source.write_count(), 0);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn update_refuses_unwritable_fields_and_bad_input_before_writing() {
    let source = fake_with_fields(&[
        EventField::Title,
        EventField::Timing,
        EventField::Attendees,
        EventField::Reminders,
    ]);
    let created = create(&source, &draft("Sync")).await.unwrap();
    let reference = EventRef::from_event(&created);
    let before = source.write_count();
    expect_write_error(
        WriteError::Unsupported {
            fields: fields(&[EventField::Notes]),
        },
        update(
            &source,
            &reference,
            &hand_patch(|p| p.notes = FieldUpdate::Set("n".into())),
        )
        .await,
    );
    expect_write_error(
        WriteError::Invalid("attendee email is not valid: nobody".into()),
        update(
            &source,
            &reference,
            &hand_patch(|p| {
                p.attendees = Some(AttendeeChanges::new(
                    vec![AttendeeDraft::new(
                        "nobody",
                        None,
                        calendar_core::model::AttendeeRole::Required,
                    )],
                    vec![],
                ));
            }),
        )
        .await,
    );
    expect_write_error(
        WriteError::Invalid("reminder minutes must not be negative".into()),
        update(
            &source,
            &reference,
            &hand_patch(|p| {
                p.reminders = FieldUpdate::Set(vec![Reminder::with_minutes_before(-5)])
            }),
        )
        .await,
    );
    let backwards = EventTiming::new(created.end, created.start, Some(zone("UTC")), false);
    expect_write_error(
        WriteError::Invalid("end must be after start".into()),
        update(
            &source,
            &reference,
            &hand_patch(|p| p.timing = Some(backwards)),
        )
        .await,
    );
    assert_eq!(source.write_count(), before);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn create_refuses_an_invalid_draft() {
    let source = FakeWritableSource::new();
    let mut bad = draft("Sync");
    bad.timing.end = bad.timing.start;
    expect_write_error(
        WriteError::Invalid("end must be after start".into()),
        create(&source, &bad).await,
    );
    assert_eq!(source.write_count(), 0);
}

// MARK: - The conformance checks catch broken sources

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flaw {
    IgnoresTitleChange,
    ThrowsOnUpdate,
    DeleteKeepsEvent,
    SilentlyAcceptsAttendees,
    CanWriteFalse,
    EmptyPatchBumpsVersion,
    TitleUpdateShiftsEnd,
    TitleUpdateFlipsAllDay,
    DeleteAlwaysFails,
}

/// Forwards to a fake and lets a test break one behaviour.
struct BrokenSource {
    inner: FakeWritableSource,
    flaw: Flaw,
}

impl BrokenSource {
    fn new(flaw: Flaw) -> BrokenSource {
        BrokenSource {
            flaw,
            inner: fake_with_fields(&[EventField::Title, EventField::Location, EventField::Timing]),
        }
    }
}

#[async_trait]
impl CalendarSource for BrokenSource {
    fn id(&self) -> String {
        self.inner.id()
    }
    fn display_name(&self) -> String {
        self.inner.display_name()
    }
    fn capabilities(&self) -> SourceCapabilities {
        let mut caps = self.inner.capabilities();
        if self.flaw == Flaw::CanWriteFalse {
            caps.can_write = false;
        }
        caps
    }
    async fn calendars(&self) -> Outcome<Vec<CalendarDescriptor>> {
        self.inner.calendars().await
    }
    async fn events_in(&self, interval: DateInterval) -> Outcome<Vec<CalendarEvent>> {
        self.inner.events_in(interval).await
    }
    fn changes(&self) -> UnboundedReceiver<CalendarChange> {
        self.inner.changes()
    }
    fn as_writable(&self) -> Option<&dyn WritableCalendarSource> {
        Some(self)
    }
}

#[async_trait]
impl WritableCalendarSource for BrokenSource {
    async fn create(
        &self,
        draft: &EventDraft,
        calendar_id: &str,
        notify: NotifyPolicy,
    ) -> Outcome<CalendarEvent> {
        self.inner.create(draft, calendar_id, notify).await
    }

    async fn update(
        &self,
        reference: &EventRef,
        patch: &EventPatch,
        scope: RecurrenceScope,
        notify: NotifyPolicy,
    ) -> Outcome<CalendarEvent> {
        let empty = EventPatch::default();
        match self.flaw {
            Flaw::IgnoresTitleChange => self.inner.update(reference, &empty, scope, notify).await,
            Flaw::ThrowsOnUpdate => Err(WriteError::Forbidden(Some("no".into())).into()),
            Flaw::SilentlyAcceptsAttendees if patch.attendees.is_some() => {
                self.inner.update(reference, &empty, scope, notify).await
            }
            Flaw::EmptyPatchBumpsVersion if patch.is_empty() => {
                let mut event = self.inner.update(reference, patch, scope, notify).await?;
                event.version = Some("bumped-by-an-empty-patch".into());
                Ok(event)
            }
            Flaw::TitleUpdateShiftsEnd if patch.title.is_some() => {
                let mut event = self.inner.update(reference, patch, scope, notify).await?;
                event.end = add_seconds(event.end, 60.0);
                Ok(event)
            }
            Flaw::TitleUpdateFlipsAllDay if patch.title.is_some() => {
                let mut event = self.inner.update(reference, patch, scope, notify).await?;
                event.is_all_day = true;
                Ok(event)
            }
            _ => self.inner.update(reference, patch, scope, notify).await,
        }
    }

    async fn delete(
        &self,
        reference: &EventRef,
        scope: RecurrenceScope,
        notify: NotifyPolicy,
    ) -> Outcome<()> {
        if self.flaw == Flaw::DeleteKeepsEvent {
            return Ok(());
        }
        if self.flaw == Flaw::DeleteAlwaysFails {
            return Err(WriteError::Forbidden(Some("no delete".into())).into());
        }
        self.inner.delete(reference, scope, notify).await
    }

    async fn respond(
        &self,
        reference: &EventRef,
        response: ResponseStatus,
        scope: RecurrenceScope,
        notify: NotifyPolicy,
    ) -> Outcome<CalendarEvent> {
        self.inner.respond(reference, response, scope, notify).await
    }
}

const ALL_FLAWS: [Flaw; 9] = [
    Flaw::IgnoresTitleChange,
    Flaw::ThrowsOnUpdate,
    Flaw::DeleteKeepsEvent,
    Flaw::SilentlyAcceptsAttendees,
    Flaw::CanWriteFalse,
    Flaw::EmptyPatchBumpsVersion,
    Flaw::TitleUpdateShiftsEnd,
    Flaw::TitleUpdateFlipsAllDay,
    Flaw::DeleteAlwaysFails,
];

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_conformance_checks_report_broken_sources() {
    for flaw in ALL_FLAWS {
        let source = BrokenSource::new(flaw);
        let found = WritableSourceConformance::violations(&source, "cal", window()).await;
        assert!(!found.is_empty(), "expected a violation for {flaw:?}");
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_conformance_checks_name_the_flaw_they_found() {
    async fn found(flaw: Flaw) -> Vec<String> {
        WritableSourceConformance::violations(&BrokenSource::new(flaw), "cal", window()).await
    }
    assert!(
        found(Flaw::EmptyPatchBumpsVersion)
            .await
            .contains(&"an empty patch changed the version".to_string())
    );
    let timing = "update changed a field the patch did not touch (timing)".to_string();
    assert!(found(Flaw::TitleUpdateShiftsEnd).await.contains(&timing));
    assert!(found(Flaw::TitleUpdateFlipsAllDay).await.contains(&timing));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_conformance_checks_report_a_cleanup_that_fails() {
    let found = WritableSourceConformance::violations(
        &BrokenSource::new(Flaw::DeleteAlwaysFails),
        "cal",
        window(),
    )
    .await;
    assert!(
        found.iter().any(|v| v.starts_with("cleanup delete failed")),
        "{found:?}"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_conformance_checks_leave_nothing_behind_when_a_step_fails() {
    let source = BrokenSource::new(Flaw::ThrowsOnUpdate);
    let _ = WritableSourceConformance::violations(&source, "cal", window()).await;
    assert!(source.events_in(window()).await.unwrap().is_empty());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_conformance_checks_refuse_an_unwritable_calendar_without_crashing() {
    let found =
        WritableSourceConformance::violations(&FakeWritableSource::new(), "nope", window()).await;
    assert!(found.len() == 1 && found[0].starts_with("create threw"));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_fake_source_refuses_a_duplicate_uid_on_the_same_calendar() {
    let source = FakeWritableSource::with_options(
        &["a", "b"],
        FakeWritableSource::default_writable_fields(),
        true,
    );
    let timing = EventTiming::new(at(1000), at(2000), None, false);
    let with_uid = EventDraft {
        uid: Some("u-1".into()),
        ..EventDraft::new("T", timing)
    };
    let first = source
        .create(&with_uid, "a", NotifyPolicy::None)
        .await
        .unwrap();
    match source.create(&with_uid, "a", NotifyPolicy::None).await {
        Err(Failure::Write(WriteError::AlreadyExists(existing))) => {
            assert_eq!(existing.event_id, first.event_id)
        }
        other => panic!("expected alreadyExists, got {other:?}"),
    }
    // another calendar
    source
        .create(&with_uid, "b", NotifyPolicy::None)
        .await
        .unwrap();
}

#[test]
fn the_fake_and_a_wrapper_expose_themselves_as_writable() {
    // `CalendarSource::as_writable` is Rust's `source as? WritableCalendarSource`.
    let fake = FakeWritableSource::new();
    assert!(fake.as_writable().is_some());
    assert!(
        BrokenSource::new(Flaw::CanWriteFalse)
            .as_writable()
            .is_some()
    );
}
