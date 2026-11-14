//! Port of `WriteTests.swift`.

use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use calendar_core::all_day::{AllDay, CalendarDate};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::{HttpRequest, HttpResponse, HttpTransport};
use calendar_core::model::{
    AttendeeRole, CalendarEvent, Participation, Reminder, ResponseStatus, SeriesInfo,
};
use calendar_core::recurrence::rule::RecurrenceRule;
use calendar_core::source_types::CalendarSource;
use calendar_core::time::{DateInterval, Zone};
use calendar_core::write::event_draft::{AttendeeDraft, EventDraft, EventTiming};
use calendar_core::write::event_patch::{EventPatch, FieldUpdate};
use calendar_core::write::types::{
    ConferenceRequest, EventField, EventRef, NotifyPolicy, RecurrenceScope, WriteError,
};
use calendar_core::write::writable_source::WritableCalendarSource;
use calendar_ical::EventResource;
use calendar_test_support::hand_patch;
use calendar_test_support::writable_source_conformance::WritableSourceConformance;
use jiff::Timestamp;

use super::support::{
    CalDavHarness, FakeCalDavServer, pacific, pt, september, single_ics, single_ics_with,
    weekly_ics, weekly_ics_by,
};
use crate::source::CalDavCalendarSource;

pub(super) fn expect_write_error<T: Debug>(expected: WriteError, result: Outcome<T>) {
    match result {
        Err(Failure::Write(error)) => assert_eq!(error, expected),
        other => panic!("expected {expected:?}, got {other:?}"),
    }
}

fn stored(h: &CalDavHarness, name: &str) -> EventResource {
    let body = h.server.body("home", name).unwrap();
    EventResource::from_data(body.as_bytes()).unwrap()
}

async fn occurrence(source: &CalDavCalendarSource, day: i64, hour: i64) -> CalendarEvent {
    let slot = pt(2026, 9, day, hour, 0);
    source
        .events_in(september())
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.original_start() == Some(slot))
        .unwrap()
}

async fn occ(source: &CalDavCalendarSource, day: i64) -> CalendarEvent {
    occurrence(source, day, 10).await
}

fn timing() -> EventTiming {
    EventTiming::new(
        pt(2026, 9, 10, 9, 0),
        pt(2026, 9, 10, 10, 0),
        Some(pacific()),
        false,
    )
}

fn title_patch(title: &str) -> EventPatch {
    hand_patch(|p| p.title = Some(title.to_string()))
}

fn timing_patch(t: EventTiming) -> EventPatch {
    hand_patch(|p| p.timing = Some(t))
}

fn recurrence_patch(rule: RecurrenceRule) -> EventPatch {
    hand_patch(|p| p.recurrence = FieldUpdate::Set(rule))
}

fn rule(text: &str) -> RecurrenceRule {
    RecurrenceRule::from_rrule(text, None).unwrap()
}

fn rule_in(text: &str, zone: &Zone) -> RecurrenceRule {
    RecurrenceRule::from_rrule(text, Some(zone)).unwrap()
}

fn id(event: &CalendarEvent) -> EventRef {
    EventRef::from_event(event)
}

fn property(resource: &EventResource, name: &str) -> Option<String> {
    resource
        .master()
        .and_then(|m| m.property(name))
        .map(|p| p.value.clone())
}

#[tokio::test]
async fn passes_the_writable_conformance_suite() {
    for auto_schedule in [true, false] {
        let h = CalDavHarness::new();
        let source = h
            .source_with(auto_schedule, &super::support::DEFAULT_ADDRESSES, None)
            .await;
        assert_eq!(
            WritableSourceConformance::violations(source.as_ref(), "home", september()).await,
            Vec::<String>::new()
        );
    }
}

#[tokio::test]
async fn create_puts_a_new_resource_and_returns_its_version() {
    let h = CalDavHarness::new();
    let source = h.source().await;
    let mut draft = EventDraft::new("Design review", timing());
    draft.reminders = Some(vec![Reminder::before(10, Default::default(), Some(false))]);
    let created = source
        .create(&draft, "home", NotifyPolicy::None)
        .await
        .unwrap();
    assert!(created.event_id == "uuid-2.ics" && created.uid.as_deref() == Some("uuid-1"));
    assert_eq!(created.series, Some(SeriesInfo::NotRecurring));
    assert_eq!(created.version, h.server.etag("home", "uuid-2.ics"));
    let put = h.server.requests("PUT").into_iter().next().unwrap();
    assert_eq!(
        put.headers.get("If-None-Match").map(String::as_str),
        Some("*")
    );
    assert!(put.headers["Content-Type"].starts_with("text/calendar"));
    let resource = stored(&h, "uuid-2.ics");
    assert_eq!(
        resource.calendar.components_named("VTIMEZONE")[0]
            .property("TZID")
            .unwrap()
            .value,
        "America/Los_Angeles"
    );
    assert_eq!(
        created.reminders,
        Some(vec![Reminder {
            is_calendar_default: Some(false),
            ..Reminder::with_minutes_before(10)
        }])
    );
}

#[tokio::test]
async fn create_with_a_known_uid_is_already_exists() {
    let h = CalDavHarness::new();
    h.server.store(
        "home",
        "single.ics",
        &single_ics_with("shared-uid", "Dentist", &[]),
    );
    let source = h.source().await;
    let mut draft = EventDraft::new("Copy", timing());
    draft.uid = Some("shared-uid".to_string());
    match source.create(&draft, "home", NotifyPolicy::None).await {
        Err(Failure::Write(WriteError::AlreadyExists(existing))) => {
            assert!(existing.event_id == "single.ics" && existing.title == "Dentist");
        }
        other => panic!("expected alreadyExists, got {other:?}"),
    }
    assert!(h.server.requests("PUT").is_empty());
}

#[tokio::test]
async fn create_retries_a_name_clash_once() {
    let h = CalDavHarness::new();
    let source = h.source().await;
    h.server.fail("PUT", "uuid-2.ics", 412);
    let created = source
        .create(&EventDraft::new("x", timing()), "home", NotifyPolicy::None)
        .await
        .unwrap();
    assert_eq!(created.event_id, "uuid-3.ics");
}

#[tokio::test]
async fn create_recurring_returns_the_series_master() {
    let h = CalDavHarness::new();
    let source = h.source().await;
    let mut draft = EventDraft::new("Weekly", timing());
    draft.recurrence = Some(rule("FREQ=WEEKLY;COUNT=3"));
    let series = source
        .create(&draft, "home", NotifyPolicy::None)
        .await
        .unwrap();
    assert!(
        series.series_id() == Some(series.event_id.as_str())
            && series.original_start() == Some(series.start)
    );
    expect_write_error(
        WriteError::Invalid(
            "this is a recurring series; use .allInSeries or read the occurrence first".to_string(),
        ),
        source
            .update(
                &id(&series),
                &title_patch("y"),
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
    );
}

#[tokio::test]
async fn refused_fields_and_policies_write_nothing() {
    let h = CalDavHarness::new();
    let source = h.source().await;
    let guests = vec![AttendeeDraft::new(
        "ann@example.test",
        None,
        AttendeeRole::Required,
    )];
    let attendees = || EventDraft {
        attendees: guests.clone(),
        ..EventDraft::new("x", timing())
    };
    expect_write_error(
        WriteError::Unsupported {
            fields: [EventField::Attendees].into(),
        },
        source
            .create(&attendees(), "home", NotifyPolicy::None)
            .await,
    );
    let conference = EventDraft {
        conference: ConferenceRequest::Generate,
        ..EventDraft::new("x", timing())
    };
    expect_write_error(
        WriteError::Unsupported {
            fields: [EventField::Conference].into(),
        },
        source.create(&conference, "home", NotifyPolicy::All).await,
    );
    let email = EventDraft {
        reminders: Some(vec![Reminder::before(
            5,
            calendar_core::model::ReminderType::Email { address: None },
            None,
        )]),
        ..EventDraft::new("x", timing())
    };
    expect_write_error(
        WriteError::Unsupported {
            fields: [EventField::Reminders].into(),
        },
        source.create(&email, "home", NotifyPolicy::All).await,
    );
    let plain = h
        .source_with(false, &super::support::DEFAULT_ADDRESSES, None)
        .await;
    expect_write_error(
        WriteError::Unsupported {
            fields: [EventField::Attendees].into(),
        },
        plain.create(&attendees(), "home", NotifyPolicy::All).await,
    );
    assert!(h.server.requests("PUT").is_empty());
    let invited = source
        .create(
            &EventDraft {
                attendees: guests.clone(),
                ..EventDraft::new("Meet", timing())
            },
            "home",
            NotifyPolicy::All,
        )
        .await
        .unwrap();
    assert!(invited.organizer.as_ref().unwrap().is_self);
    let emails: Vec<Option<&str>> = invited.attendees.iter().map(|a| a.email()).collect();
    assert_eq!(emails, [Some("me@icloud.test"), Some("ann@example.test")]);
}

#[tokio::test]
async fn update_keeps_properties_the_model_does_not_cover() {
    let h = CalDavHarness::new();
    h.server.store("home", "single.ics", &single_ics());
    let source = h.source().await;
    let event = source.events_in(september()).await.unwrap().remove(0);
    let renamed = source
        .update(
            &id(&event),
            &title_patch("Dentist (moved)"),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert!(renamed.title == "Dentist (moved)" && renamed.version != event.version);
    let put = h.server.requests("PUT").pop().unwrap();
    assert_eq!(put.headers.get("If-Match"), event.version.as_ref());
    assert_eq!(
        property(
            &stored(&h, "single.ics"),
            "X-APPLE-TRAVEL-ADVISORY-BEHAVIOR"
        )
        .as_deref(),
        Some("AUTOMATIC")
    );
    expect_write_error(
        WriteError::Unsupported {
            fields: [EventField::Reminders].into(),
        },
        source
            .update(
                &id(&renamed),
                &hand_patch(|p| p.reminders = FieldUpdate::Clear),
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
    );
}

#[tokio::test]
async fn this_instance_adds_then_edits_an_override() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let source = h.source().await;
    let tuesday = occ(&source, 22).await;
    let edited = source
        .update(
            &id(&tuesday),
            &title_patch("Special"),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert!(edited.event_id == tuesday.event_id && edited.title == "Special");
    assert_eq!(stored(&h, "weekly.ics").overrides().len(), 2);
    let again = source
        .update(
            &id(&edited),
            &hand_patch(|p| p.location = FieldUpdate::Set("Room 9".to_string())),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert!(again.title == "Special" && again.location.as_deref() == Some("Room 9"));
    assert_eq!(stored(&h, "weekly.ics").overrides().len(), 2);
    let titles: Vec<String> = source
        .events_in(september())
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.title)
        .collect();
    assert_eq!(
        titles,
        ["Team sync", "Team sync (moved)", "Special", "Team sync"]
    );
}

#[tokio::test]
async fn all_in_series_timing_shifts_exceptions_with_the_series() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let source = h.source().await;
    let tuesday = occ(&source, 22).await;
    let later = EventTiming::new(
        pt(2026, 9, 22, 11, 0),
        pt(2026, 9, 22, 11, 30),
        Some(pacific()),
        false,
    );
    let series = source
        .update(
            &id(&tuesday),
            &timing_patch(later.clone()),
            RecurrenceScope::AllInSeries,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert!(series.event_id == "weekly.ics" && series.start == pt(2026, 9, 1, 11, 0));
    let starts: Vec<Timestamp> = source
        .events_in(september())
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.start)
        .collect();
    assert_eq!(
        starts,
        [
            pt(2026, 9, 1, 11, 0),
            pt(2026, 9, 16, 14, 0),
            pt(2026, 9, 22, 11, 0),
            pt(2026, 9, 29, 11, 0)
        ]
    );
    let mut no_slot = id(&tuesday);
    no_slot.original_start = None;
    expect_write_error(
        WriteError::Unsupported {
            fields: [EventField::Timing].into(),
        },
        source
            .update(
                &no_slot,
                &timing_patch(later),
                RecurrenceScope::AllInSeries,
                NotifyPolicy::None,
            )
            .await,
    );
    expect_write_error(
        WriteError::Invalid("needs the occurrence's original start".to_string()),
        source
            .update(
                &no_slot,
                &title_patch("x"),
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
    );
}

#[tokio::test]
async fn a_stale_version_retries_unless_the_same_field_changed() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let source = h.source().await;
    let tuesday = occ(&source, 22).await;
    // Someone else renames another occurrence: the resource's ETag moves, but not the field we touch.
    let other = occ(&source, 29).await;
    source
        .update(
            &id(&other),
            &title_patch("Other change"),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let mut with_room = tuesday.clone();
    with_room.location = Some("Room 2".to_string());
    let moved = source
        .update(
            &id(&tuesday),
            &EventPatch::diff(&tuesday, &with_room),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert!(moved.location.as_deref() == Some("Room 2") && moved.title == "Team sync");
    // Now the same occurrence's title changes under a stale base.
    let fresh = occ(&source, 22).await;
    source
        .update(
            &id(&fresh),
            &title_patch("Theirs"),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let mut mine = fresh.clone();
    mine.title = "Mine".to_string();
    expect_write_error(
        WriteError::Conflict {
            fields: [EventField::Title].into(),
        },
        source
            .update(
                &id(&fresh),
                &EventPatch::diff(&fresh, &mine),
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
    );
}

#[tokio::test]
async fn notify_policy_is_refused_when_others_would_be_told() {
    let h = CalDavHarness::new();
    h.server.store(
        "home",
        "meeting.ics",
        &single_ics_with(
            "m",
            "Meeting",
            &[
                "ORGANIZER:mailto:me@icloud.test",
                "ATTENDEE;PARTSTAT=ACCEPTED:mailto:me@icloud.test",
                "ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:ann@example.test",
            ],
        ),
    );
    let source = h.source().await;
    let meeting = source.events_in(september()).await.unwrap().remove(0);
    for policy in [NotifyPolicy::None, NotifyPolicy::ExternalOnly] {
        expect_write_error(
            WriteError::Unsupported {
                fields: [EventField::Attendees].into(),
            },
            source
                .update(
                    &id(&meeting),
                    &title_patch("x"),
                    RecurrenceScope::ThisInstance,
                    policy,
                )
                .await,
        );
        expect_write_error(
            WriteError::Unsupported {
                fields: [EventField::Attendees].into(),
            },
            source
                .delete(&id(&meeting), RecurrenceScope::ThisInstance, policy)
                .await,
        );
    }
    assert!(h.server.requests("PUT").is_empty() && h.server.requests("DELETE").is_empty());
    source
        .update(
            &id(&meeting),
            &title_patch("Renamed"),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::All,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn deletes_by_scope() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let source = h.source().await;
    let starts = |source: Arc<CalDavCalendarSource>| async move {
        source
            .events_in(september())
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.start)
            .collect::<Vec<_>>()
    };
    source
        .delete(
            &id(&occ(&source, 22).await),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert_eq!(
        starts(source.clone()).await,
        [
            pt(2026, 9, 1, 10, 0),
            pt(2026, 9, 16, 14, 0),
            pt(2026, 9, 29, 10, 0)
        ]
    );
    // The moved occurrence's slot is the 15th at 10:00, not its new time.
    source
        .delete(
            &id(&occ(&source, 15).await),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert_eq!(starts(source.clone()).await, [pt(2026, 9, 1, 10, 0)]);
    source
        .delete(
            &id(&occ(&source, 1).await),
            RecurrenceScope::AllInSeries,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert!(h.server.names("home").is_empty());
    let deletion = h.server.requests("DELETE").pop().unwrap();
    assert!(!deletion.headers.contains_key("If-Match"));
}

#[tokio::test]
async fn deleting_an_occurrence_retries_then_reports_a_conflict() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let source = h.source().await;
    let tuesday = occ(&source, 22).await;
    h.server.fail_n("PUT", "weekly.ics", 412, 2, 0);
    source
        .delete(
            &id(&tuesday),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert_eq!(h.server.requests("PUT").len(), 3);
    let thursday = occ(&source, 29).await;
    h.server.fail_n("PUT", "weekly.ics", 412, 3, 0);
    expect_write_error(
        WriteError::Conflict {
            fields: [EventField::Recurrence].into(),
        },
        source
            .delete(
                &id(&thursday),
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
    );
    expect_write_error(
        WriteError::NotFound,
        source
            .delete(
                &EventRef::new("home", "gone.ics"),
                RecurrenceScope::AllInSeries,
                NotifyPolicy::None,
            )
            .await,
    );
}

#[tokio::test]
async fn respond_sets_the_accounts_answer() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics());
    let source = h.source().await;
    let tuesday = occ(&source, 22).await;
    assert_eq!(
        tuesday.participation,
        Some(Participation::Invited(ResponseStatus::NeedsAction))
    );
    let answered = source
        .respond(
            &id(&tuesday),
            ResponseStatus::Accepted,
            RecurrenceScope::ThisInstance,
            NotifyPolicy::All,
        )
        .await
        .unwrap();
    assert_eq!(
        answered.participation,
        Some(Participation::Invited(ResponseStatus::Accepted))
    );
    assert_eq!(
        occ(&source, 29).await.participation,
        Some(Participation::Invited(ResponseStatus::NeedsAction))
    );
    let series = source
        .respond(
            &id(&tuesday),
            ResponseStatus::Declined,
            RecurrenceScope::AllInSeries,
            NotifyPolicy::All,
        )
        .await
        .unwrap();
    assert_eq!(
        series.participation,
        Some(Participation::Invited(ResponseStatus::Declined))
    );
    assert!(
        source
            .events_in(september())
            .await
            .unwrap()
            .iter()
            .all(|e| e.participation == Some(Participation::Invited(ResponseStatus::Declined)))
    );
    let unsupported = WriteError::Unsupported {
        fields: [EventField::Attendees].into(),
    };
    expect_write_error(
        unsupported.clone(),
        source
            .respond(
                &id(&tuesday),
                ResponseStatus::Accepted,
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
    );
    expect_write_error(
        unsupported.clone(),
        source
            .respond(
                &id(&tuesday),
                ResponseStatus::Accepted,
                RecurrenceScope::ThisAndFollowing,
                NotifyPolicy::All,
            )
            .await,
    );
    expect_write_error(
        WriteError::Invalid("a response must be accepted, tentative or declined".to_string()),
        source
            .respond(
                &id(&tuesday),
                ResponseStatus::NeedsAction,
                RecurrenceScope::ThisInstance,
                NotifyPolicy::All,
            )
            .await,
    );
    h.server.store("home", "single.ics", &single_ics());
    let mine = source
        .events_in(september())
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.title == "Dentist")
        .unwrap();
    expect_write_error(
        unsupported,
        source
            .respond(
                &id(&mine),
                ResponseStatus::Accepted,
                RecurrenceScope::ThisInstance,
                NotifyPolicy::All,
            )
            .await,
    );
}

#[tokio::test]
async fn forbidden_calendar_is_forbidden() {
    let h = CalDavHarness::new();
    h.server.configure(|s| {
        let mut shared = super::support::Collection::new("Shared");
        shared.privileges = vec!["read".to_string()];
        s.collections.insert("shared".to_string(), shared);
    });
    h.server.store("shared", "single.ics", &single_ics());
    let source = h.source().await;
    expect_write_error(
        WriteError::Forbidden(None),
        source
            .create(
                &EventDraft::new("x", timing()),
                "shared",
                NotifyPolicy::None,
            )
            .await,
    );
}

// MARK: Review round 1

type Hook = Box<dyn FnOnce(&FakeCalDavServer) + Send>;

/// Forwards to the server, but lets `before_first_put` (another client's edit) happen just before the first PUT, so
/// that PUT is genuinely stale.
struct InterferingTransport {
    server: Arc<FakeCalDavServer>,
    pending: Mutex<Option<Hook>>,
}

impl InterferingTransport {
    fn new(
        server: &Arc<FakeCalDavServer>,
        before_first_put: impl FnOnce(&FakeCalDavServer) + Send + 'static,
    ) -> Arc<Self> {
        Arc::new(InterferingTransport {
            server: server.clone(),
            pending: Mutex::new(Some(Box::new(before_first_put))),
        })
    }
}

#[async_trait]
impl HttpTransport for InterferingTransport {
    async fn send(&self, request: HttpRequest) -> Outcome<HttpResponse> {
        if request.method == "PUT" {
            let action = self
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(action) = action {
                action(&self.server);
            }
        }
        self.server.send(request).await
    }
}

/// An invite to single occurrences: overrides, no master.
fn overrides_only_ics(slots: &[i64]) -> String {
    let mut lines: Vec<String> = vec![
        "BEGIN:VCALENDAR".into(),
        "VERSION:2.0".into(),
        "PRODID:-//Apple Inc.//iCloud//EN".into(),
    ];
    for day in slots {
        lines.extend([
            "BEGIN:VEVENT".to_string(),
            "UID:only-uid".to_string(),
            "DTSTAMP:20260901T000000Z".to_string(),
            format!("RECURRENCE-ID;TZID=America/Los_Angeles:202609{day}T100000"),
            format!("SUMMARY:Guest spot {day}"),
            format!("DTSTART;TZID=America/Los_Angeles:202609{day}T100000"),
            format!("DTEND;TZID=America/Los_Angeles:202609{day}T103000"),
            "ORGANIZER;CN=Me:mailto:me@icloud.test".to_string(),
            "ATTENDEE;CN=Me;PARTSTAT=NEEDS-ACTION:mailto:me@icloud.test".to_string(),
            "END:VEVENT".to_string(),
        ]);
    }
    lines.push("END:VCALENDAR".into());
    lines.join("\r\n") + "\r\n"
}

#[tokio::test]
async fn an_organizer_who_is_not_the_account_is_told_even_when_no_other_attendee_is_listed() {
    let h = CalDavHarness::new();
    h.server.store(
        "home",
        "invite.ics",
        &single_ics_with(
            "i",
            "Invite",
            &[
                "ORGANIZER:mailto:boss@example.test",
                "ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:me@icloud.test",
            ],
        ),
    );
    let source = h.source().await;
    let invite = source.events_in(september()).await.unwrap().remove(0);
    let unsupported = WriteError::Unsupported {
        fields: [EventField::Attendees].into(),
    };
    for policy in [NotifyPolicy::None, NotifyPolicy::ExternalOnly] {
        expect_write_error(
            unsupported.clone(),
            source
                .delete(&id(&invite), RecurrenceScope::ThisInstance, policy)
                .await,
        );
        expect_write_error(
            unsupported.clone(),
            source
                .update(
                    &id(&invite),
                    &title_patch("x"),
                    RecurrenceScope::ThisInstance,
                    policy,
                )
                .await,
        );
    }
    assert!(h.server.requests("PUT").is_empty() && h.server.requests("DELETE").is_empty());
}

#[tokio::test]
async fn deleting_an_occurrence_of_a_series_that_became_a_single_event_is_not_found() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let source = h.source().await;
    let tuesday = occ(&source, 22).await;
    h.server.store(
        "home",
        "weekly.ics",
        &single_ics_with("weekly-uid", "Now a one-off", &[]),
    );
    expect_write_error(
        WriteError::NotFound,
        source
            .delete(
                &id(&tuesday),
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
    );
    expect_write_error(
        WriteError::NotFound,
        source
            .delete(
                &id(&tuesday),
                RecurrenceScope::ThisAndFollowing,
                NotifyPolicy::None,
            )
            .await,
    );
    assert!(h.server.requests("DELETE").is_empty() && h.server.requests("PUT").is_empty());
    assert_eq!(h.server.names("home"), ["weekly.ics"]);
}

#[tokio::test]
async fn a_whole_series_edit_is_judged_against_the_series_master() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let source = h.source().await;
    // The override: its own title, not the master's.
    let moved = occurrence(&source, 15, 10).await;
    let tuesday = occ(&source, 22).await;
    source
        .update(
            &id(&tuesday),
            &title_patch("Planning"),
            RecurrenceScope::AllInSeries,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    let mut mine = moved.clone();
    mine.title = "Planning (mine)".to_string();
    expect_write_error(
        WriteError::Conflict {
            fields: [EventField::Title].into(),
        },
        source
            .update(
                &id(&moved),
                &EventPatch::diff(&moved, &mine),
                RecurrenceScope::AllInSeries,
                NotifyPolicy::None,
            )
            .await,
    );
    assert_eq!(
        property(&stored(&h, "weekly.ics"), "SUMMARY").as_deref(),
        Some("Planning")
    );
}

#[tokio::test]
async fn an_occurrence_in_a_resource_without_a_master_can_be_deleted_updated_and_answered() {
    let h = CalDavHarness::new();
    h.server
        .store("home", "only.ics", &overrides_only_ics(&[22, 29]));
    let source = h.source().await;
    let first = occ(&source, 22).await;
    source
        .delete(
            &id(&first),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::All,
        )
        .await
        .unwrap();
    let starts: Vec<Timestamp> = source
        .events_in(september())
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.start)
        .collect();
    assert_eq!(starts, [pt(2026, 9, 29, 10, 0)]);
    let last = occ(&source, 29).await;
    let renamed = source
        .update(
            &id(&last),
            &title_patch("Renamed"),
            RecurrenceScope::AllInSeries,
            NotifyPolicy::All,
        )
        .await
        .unwrap();
    assert_eq!(renamed.title, "Renamed");
    let answered = source
        .respond(
            &id(&renamed),
            ResponseStatus::Accepted,
            RecurrenceScope::AllInSeries,
            NotifyPolicy::All,
        )
        .await
        .unwrap();
    assert_eq!(
        answered.participation,
        Some(Participation::Invited(ResponseStatus::Accepted))
    );
    source
        .delete(
            &id(&answered),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::All,
        )
        .await
        .unwrap();
    assert!(h.server.names("home").is_empty());
    let deletion = h.server.requests("DELETE").pop().unwrap();
    assert!(deletion.headers.contains_key("If-Match"));
}

#[tokio::test]
async fn delete_reruns_the_notify_check_on_every_attempt() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    // Meanwhile someone invites people.
    let transport = InterferingTransport::new(&h.server, |server| {
        server.store("home", "weekly.ics", &weekly_ics())
    });
    let source = h.source_via(transport).await;
    let tuesday = occ(&source, 22).await;
    expect_write_error(
        WriteError::Unsupported {
            fields: [EventField::Attendees].into(),
        },
        source
            .delete(
                &id(&tuesday),
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
    );
    let body = h.server.body("home", "weekly.ics").unwrap();
    assert!(!body.contains("20260922"));
}

#[tokio::test]
async fn delete_treats_a_resource_gone_on_a_retry_as_done() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let transport =
        InterferingTransport::new(&h.server, |server| server.remove("home", "weekly.ics"));
    let source = h.source_via(transport).await;
    let tuesday = occ(&source, 22).await;
    // The PUT 412s, the re-fetch 404s.
    source
        .delete(
            &id(&tuesday),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert!(h.server.names("home").is_empty());
}

fn all_day_ics(rule: &str) -> String {
    [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "PRODID:-//Apple Inc.//iCloud//EN",
        "BEGIN:VEVENT",
        "UID:allday-uid",
        "DTSTAMP:20260301T000000Z",
        "SEQUENCE:0",
        "SUMMARY:Holiday",
        "DTSTART;VALUE=DATE:20260305",
        "DTEND;VALUE=DATE:20260306",
        &format!("RRULE:{rule}"),
        "END:VEVENT",
        "END:VCALENDAR",
    ]
    .join("\r\n")
        + "\r\n"
}

fn day(zone: &Zone, y: i64, m: i64, d: i64) -> Timestamp {
    AllDay::start_of_day(&CalendarDate::new(y, m, d), zone).unwrap()
}

#[tokio::test]
async fn deleting_from_the_day_after_a_spring_forward_day_keeps_that_day() {
    let new_york = Zone::named("America/New_York").unwrap();
    let h = CalDavHarness::new();
    h.server.configure(|s| {
        s.collections.get_mut("home").unwrap().time_zone_id = Some("America/New_York".to_string())
    });
    h.server
        .store("home", "holiday.ics", &all_day_ics("FREQ=DAILY"));
    let source = h.source().await;
    let window = DateInterval::new(day(&new_york, 2026, 3, 1), day(&new_york, 2026, 3, 15));
    let slot = day(&new_york, 2026, 3, 9);
    let mar9 = source
        .events_in(window)
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.original_start() == Some(slot))
        .unwrap();
    source
        .delete(
            &id(&mar9),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    // Mar 8 is 23 hours long in New York; the series must still cover it.
    let starts: Vec<Timestamp> = source
        .events_in(window)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.start)
        .collect();
    assert_eq!(
        starts,
        (5..=8)
            .map(|d| day(&new_york, 2026, 3, d))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        property(&stored(&h, "holiday.ics"), "RRULE").as_deref(),
        Some("FREQ=DAILY;UNTIL=20260308")
    );
}

#[tokio::test]
async fn an_all_day_rule_change_keeps_its_last_day_in_the_calendar_zone() {
    let berlin = Zone::named("Europe/Berlin").unwrap();
    let h = CalDavHarness::new();
    h.server.configure(|s| {
        s.collections.get_mut("home").unwrap().time_zone_id = Some("Europe/Berlin".to_string())
    });
    h.server
        .store("home", "holiday.ics", &all_day_ics("FREQ=DAILY"));
    let source = h.source().await;
    let window = DateInterval::new(day(&berlin, 2026, 3, 1), day(&berlin, 2026, 3, 20));
    let slot = day(&berlin, 2026, 3, 5);
    let first = source
        .events_in(window)
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.original_start() == Some(slot))
        .unwrap();
    // The caller reads a date UNTIL in the calendar's zone, so this rule ends on (and includes) March 10.
    let new_rule = rule_in("FREQ=DAILY;UNTIL=20260310", &berlin);
    source
        .update(
            &id(&first),
            &recurrence_patch(new_rule),
            RecurrenceScope::AllInSeries,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert_eq!(
        property(&stored(&h, "holiday.ics"), "RRULE").as_deref(),
        Some("FREQ=DAILY;UNTIL=20260310")
    );
    let starts: Vec<Timestamp> = source
        .events_in(window)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.start)
        .collect();
    assert_eq!(
        starts,
        (5..=10)
            .map(|d| day(&berlin, 2026, 3, d))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn an_all_day_split_tail_keeps_its_rule_change_last_day_in_the_calendar_zone() {
    let berlin = Zone::named("Europe/Berlin").unwrap();
    let h = CalDavHarness::new();
    h.server.configure(|s| {
        s.collections.get_mut("home").unwrap().time_zone_id = Some("Europe/Berlin".to_string())
    });
    h.server
        .store("home", "holiday.ics", &all_day_ics("FREQ=DAILY"));
    let source = h.source().await;
    let window = DateInterval::new(day(&berlin, 2026, 3, 1), day(&berlin, 2026, 3, 20));
    let slot = day(&berlin, 2026, 3, 8);
    let eighth = source
        .events_in(window)
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.original_start() == Some(slot))
        .unwrap();
    let new_rule = rule_in("FREQ=DAILY;UNTIL=20260312", &berlin);
    source
        .update(
            &id(&eighth),
            &recurrence_patch(new_rule),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await
        .unwrap();
    assert_eq!(
        property(&stored(&h, "uuid-2.ics"), "RRULE").as_deref(),
        Some("FREQ=DAILY;UNTIL=20260312")
    );
    let starts: Vec<Timestamp> = source
        .events_in(window)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.start)
        .collect();
    assert_eq!(
        starts,
        (5..=12)
            .map(|d| day(&berlin, 2026, 3, d))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn updating_or_answering_an_occurrence_of_a_series_that_became_a_single_event_is_not_found() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let source = h.source().await;
    let tuesday = occ(&source, 22).await;
    let invite = [
        "ORGANIZER;CN=Boss:mailto:boss@example.test",
        "ATTENDEE;CN=Me;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:me@icloud.test",
    ];
    h.server.store(
        "home",
        "weekly.ics",
        &single_ics_with("weekly-uid", "Now a one-off", &invite),
    );
    let before = h.server.body("home", "weekly.ics");
    // Without a version the write goes straight at the current copy; with the stale version it is refused on the way in.
    let mut unversioned = id(&tuesday);
    unversioned.version = None;
    for reference in [unversioned, id(&tuesday)] {
        expect_write_error(
            WriteError::NotFound,
            source
                .update(
                    &reference,
                    &title_patch("x"),
                    RecurrenceScope::ThisInstance,
                    NotifyPolicy::All,
                )
                .await,
        );
        expect_write_error(
            WriteError::NotFound,
            source
                .update(
                    &reference,
                    &title_patch("x"),
                    RecurrenceScope::ThisAndFollowing,
                    NotifyPolicy::All,
                )
                .await,
        );
        expect_write_error(
            WriteError::NotFound,
            source
                .respond(
                    &reference,
                    ResponseStatus::Accepted,
                    RecurrenceScope::ThisInstance,
                    NotifyPolicy::All,
                )
                .await,
        );
    }
    let puts = h.server.requests("PUT");
    assert_eq!(h.server.body("home", "weekly.ics"), before);
    // The only PUT is the stale-version attempt on the copy the caller read, which the server refuses.
    assert!(puts.iter().all(|p| p.headers.contains_key("If-Match")) && puts.len() <= 3);
}

#[tokio::test]
async fn a_recurrence_change_on_one_occurrence_is_unsupported() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics_by(None));
    let source = h.source().await;
    let tuesday = occ(&source, 22).await;
    expect_write_error(
        WriteError::Unsupported {
            fields: [EventField::Recurrence].into(),
        },
        source
            .update(
                &id(&tuesday),
                &recurrence_patch(rule("FREQ=DAILY;COUNT=3")),
                RecurrenceScope::ThisInstance,
                NotifyPolicy::None,
            )
            .await,
    );
    assert!(h.server.requests("PUT").is_empty());
}
