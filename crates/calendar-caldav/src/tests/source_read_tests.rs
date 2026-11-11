//! Port of `SourceReadTests.swift`.

use calendar_core::connection::CredentialStore;
use calendar_core::failure::Failure;
use calendar_core::model::{
    Availability, CalendarKind, CalendarProvider, CalendarService, Participation, ResponseStatus,
    SeriesInfo,
};
use calendar_core::recurrence::rule::Frequency;
use calendar_core::source_types::{CalendarSource, SeriesSource, SourceError, SyncKind};
use calendar_core::write::types::{EventField, RecurrenceScope};
use calendar_test_support::all_day_conformance::AllDayConformance;
use calendar_test_support::provided_fields_conformance::ProvidedFieldsConformance;

use super::support::{
    CalDavHarness, Collection, harness_default_zone, pacific, pt, pt10, september, single_ics,
    weekly_ics,
};

fn add_collections(h: &CalDavHarness) {
    h.server.configure(|s| {
        let mut tasks = Collection::new("Reminders");
        tasks.components = Some(vec!["VTODO".to_string()]);
        s.collections.insert("tasks".to_string(), tasks);
        let mut shared = Collection::new("Shared");
        shared.color = None;
        shared.privileges = vec!["read".to_string()];
        shared.time_zone_id = None;
        s.collections.insert("shared".to_string(), shared);
        let mut holidays = Collection::new("Holidays");
        holidays.privileges = vec!["read".to_string()];
        holidays.subscribed = true;
        s.collections.insert("holidays".to_string(), holidays);
    });
}

#[tokio::test]
async fn lists_event_calendars_only() {
    let h = CalDavHarness::new();
    add_collections(&h);
    let source = h.source().await;
    let calendars = source.calendars().await.unwrap();
    let ids: Vec<&str> = calendars.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["holidays", "home", "shared"]);
    let home = calendars.iter().find(|c| c.id == "home").unwrap();
    assert!(home.title == "Home" && home.color_hex() == Some("#1BADF8"));
    assert!(home.permissions.can_edit && home.permissions.can_view_details);
    assert_eq!(home.is_default, Some(true));
    assert_eq!(
        home.time_zone.as_ref().map(|z| z.identifier()),
        Some("America/Los_Angeles")
    );
    assert!(
        home.service == CalendarService::cal_dav()
            && home.provider == Some(CalendarProvider::icloud())
    );
    assert_eq!(home.kind, CalendarKind::Standard);
    assert_eq!(home.account_name.as_deref(), Some("me@icloud.test"));
    assert_eq!(
        home.supported_availabilities,
        Some(
            [Availability::Busy, Availability::Free]
                .into_iter()
                .collect()
        )
    );
    let shared = calendars.iter().find(|c| c.id == "shared").unwrap();
    assert!(
        !shared.permissions.can_edit
            && shared.is_default == Some(false)
            && shared.color_hex().is_none()
    );
    assert_eq!(shared.time_zone, Some(harness_default_zone()));
    let holidays = calendars.iter().find(|c| c.id == "holidays").unwrap();
    assert!(holidays.kind == CalendarKind::Subscribed && !holidays.permissions.can_edit);
    for calendar in &calendars {
        assert_eq!(
            ProvidedFieldsConformance::calendar_violations(calendar, &source.capabilities()),
            Vec::<String>::new()
        );
    }
}

#[tokio::test]
async fn default_is_nil_when_the_server_does_not_say() {
    let h = CalDavHarness::new();
    h.server.configure(|s| s.default_calendar = None);
    assert!(
        h.source()
            .await
            .calendars()
            .await
            .unwrap()
            .iter()
            .all(|c| c.is_default.is_none())
    );
}

#[tokio::test]
async fn reads_events_from_every_visible_calendar() {
    let h = CalDavHarness::new();
    add_collections(&h);
    h.server.store("home", "single.ics", &single_ics());
    h.server.store("shared", "weekly.ics", &weekly_ics());
    let source = h.source().await;
    let events = source.events_in(september()).await.unwrap();
    let titles: Vec<&str> = events.iter().map(|e| e.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Team sync",
            "Dentist",
            "Team sync (moved)",
            "Team sync",
            "Team sync"
        ]
    );
    let dentist = events.iter().find(|e| e.title == "Dentist").unwrap();
    assert!(dentist.event_id == "single.ics" && dentist.calendar_id == "home");
    assert_eq!(dentist.series, Some(SeriesInfo::NotRecurring));
    assert_eq!(dentist.version, h.server.etag("home", "single.ics"));
    assert_eq!(dentist.source_id.as_deref(), Some("icloud-c1"));
    let moved = events
        .iter()
        .find(|e| e.title == "Team sync (moved)")
        .unwrap();
    assert_eq!(moved.calendar_id, "shared");
    assert_eq!(
        moved.series,
        Some(SeriesInfo::Occurrence {
            series_id: "weekly.ics".to_string(),
            original_start: Some(pt10(2026, 9, 15))
        })
    );
    assert_eq!(
        moved.participation,
        Some(Participation::Invited(ResponseStatus::NeedsAction))
    );
    for event in &events {
        assert_eq!(
            ProvidedFieldsConformance::event_violations(event, &source.capabilities()),
            Vec::<String>::new()
        );
        assert_eq!(AllDayConformance::violations(event), Vec::<String>::new());
    }
    let query = h.server.requests("REPORT").into_iter().next().unwrap();
    let body = String::from_utf8_lossy(query.body.as_deref().unwrap_or(&[])).into_owned();
    assert!(body.contains("start=\"20260901T070000Z\""));
}

#[tokio::test]
async fn without_addresses_participation_is_not_declared() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics());
    let source = h.source_with(true, &[], None).await;
    let caps = source.capabilities();
    assert!(
        !caps
            .provided_fields
            .contains(&calendar_core::source_types::ProvidedField::Participation)
    );
    assert!(!caps.can_respond_to_invite);
    let events = source.events_in(september()).await.unwrap();
    assert!(
        events
            .iter()
            .all(|e| e.participation.is_none() && e.attendees.iter().all(|a| !a.is_self))
    );
}

#[tokio::test]
async fn skips_a_calendar_that_disappeared_and_unreadable_resources() {
    let h = CalDavHarness::new();
    add_collections(&h);
    h.server.store("home", "single.ics", &single_ics());
    h.server
        .store("home", "broken.ics", "this is not iCalendar");
    h.server.store("shared", "weekly.ics", &weekly_ics());
    let source = h.source().await;
    source.calendars().await.unwrap();
    h.server.fail("REPORT", "/shared/", 404);
    let titles: Vec<String> = source
        .events_in(september())
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.title)
        .collect();
    assert_eq!(titles, ["Dentist"]);
}

#[tokio::test]
async fn free_busy_only_calendars_are_not_queried() {
    let h = CalDavHarness::new();
    h.server.configure(|s| {
        let mut busy = Collection::new("Busy");
        busy.privileges = vec!["read-free-busy".to_string()];
        s.collections.insert("busy".to_string(), busy);
    });
    let source = h.source().await;
    let calendars = source.calendars().await.unwrap();
    let busy = calendars.iter().find(|c| c.id == "busy").unwrap();
    assert!(!busy.permissions.can_view_details);
    source.events_in(september()).await.unwrap();
    assert!(
        h.server
            .requests("REPORT")
            .iter()
            .all(|r| !r.url.path().contains("/busy"))
    );
}

#[tokio::test]
async fn wrong_stored_password_is_auth_expired() {
    let h = CalDavHarness::new();
    let source = h.source().await;
    h.credentials
        .set_secrets(
            [("username", "me@icloud.test"), ("password", "revoked")]
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            "c1",
        )
        .await
        .unwrap();
    assert_eq!(
        source.calendars().await.unwrap_err(),
        Failure::Source(SourceError::AuthExpired)
    );
}

#[tokio::test]
async fn capabilities_follow_auto_schedule() {
    let h = CalDavHarness::new();
    let scheduling = h
        .source_with(true, &super::support::DEFAULT_ADDRESSES, None)
        .await
        .capabilities();
    assert!(
        scheduling.can_edit_attendees
            && scheduling.can_respond_to_invite
            && scheduling.writable_fields.contains(&EventField::Attendees)
    );
    let plain = h
        .source_with(false, &super::support::DEFAULT_ADDRESSES, None)
        .await
        .capabilities();
    assert!(
        !plain.can_edit_attendees
            && !plain.can_respond_to_invite
            && !plain.writable_fields.contains(&EventField::Attendees)
    );
    for caps in [scheduling, plain] {
        assert_eq!(
            caps.can_edit_attendees,
            caps.writable_fields.contains(&EventField::Attendees)
        );
        assert!(
            caps.can_write && !caps.controls_notifications && caps.sync_kind == SyncKind::Token
        );
        assert_eq!(
            caps.recurrence_scopes,
            RecurrenceScope::ALL.iter().copied().collect()
        );
        assert!(!caps.writable_fields.contains(&EventField::Conference));
    }
}

#[tokio::test]
async fn series_returns_the_rules_or_not_found() {
    let h = CalDavHarness::new();
    h.server.store("home", "weekly.ics", &weekly_ics());
    h.server.store("home", "single.ics", &single_ics());
    let source = h.source().await;
    let series = source.series("weekly.ics", "home").await.unwrap();
    assert!(
        series.series_id == "weekly.ics"
            && series.start == pt(2026, 9, 1, 10, 0)
            && series.time_zone == pacific()
    );
    assert_eq!(
        series.recurrence.rules.first().map(|r| r.frequency),
        Some(Frequency::Weekly)
    );
    assert_eq!(
        series.recurrence.excluded_dates,
        Some(vec![pt(2026, 9, 8, 10, 0)])
    );
    assert_eq!(
        source.series("single.ics", "home").await.unwrap_err(),
        Failure::Source(SourceError::NotFound)
    );
    assert_eq!(
        source.series("nope.ics", "home").await.unwrap_err(),
        Failure::Source(SourceError::NotFound)
    );
    assert_eq!(
        ProvidedFieldsConformance::source_violations(source.as_ref()),
        Vec::<String>::new()
    );
}
