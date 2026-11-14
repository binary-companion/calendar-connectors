//! Port of `ChangeDetectionTests.swift`.

use std::collections::BTreeSet;

use calendar_core::connection::CredentialStore;
use calendar_core::failure::Failure;
use calendar_core::source_types::{
    CalendarChange, CalendarSource, PollingCalendarSource, SourceError,
};

use super::support::{CalDavHarness, Collection, single_ics, single_ics_with};

fn events_changed(ids: &[&str]) -> Option<CalendarChange> {
    Some(CalendarChange::EventsChanged {
        calendar_ids: Some(ids.iter().map(|i| i.to_string()).collect::<BTreeSet<_>>()),
    })
}

#[tokio::test]
async fn first_check_is_a_baseline_and_a_quiet_one_is_nil() {
    let h = CalDavHarness::new();
    let source = h.source().await;
    assert_eq!(source.check_for_changes().await.unwrap(), None);
    assert_eq!(source.check_for_changes().await.unwrap(), None);
    // Unchanged tokens need no sync-collection.
    assert!(h.server.requests("REPORT").is_empty());
}

#[tokio::test]
async fn another_clients_edit_and_removal_are_reported() {
    let h = CalDavHarness::new();
    let source = h.source().await;
    source.check_for_changes().await.unwrap();
    h.server.store("home", "single.ics", &single_ics());
    assert_eq!(
        source.check_for_changes().await.unwrap(),
        events_changed(&["home"])
    );
    assert_eq!(source.check_for_changes().await.unwrap(), None);
    h.server.remove("home", "single.ics");
    assert_eq!(
        source.check_for_changes().await.unwrap(),
        events_changed(&["home"])
    );
}

#[tokio::test]
async fn a_new_calendar_is_calendars_changed() {
    let h = CalDavHarness::new();
    let source = h.source().await;
    source.check_for_changes().await.unwrap();
    h.server.configure(|s| {
        s.collections
            .insert("work".to_string(), Collection::new("Work"));
    });
    assert_eq!(
        source.check_for_changes().await.unwrap(),
        Some(CalendarChange::CalendarsChanged)
    );
    assert_eq!(source.check_for_changes().await.unwrap(), None);
    h.server.configure(|s| {
        s.collections.get_mut("work").unwrap().display_name = "Work (renamed)".to_string()
    });
    assert_eq!(
        source.check_for_changes().await.unwrap(),
        Some(CalendarChange::CalendarsChanged)
    );
}

#[tokio::test]
async fn an_expired_token_rebaselines_and_reports_the_calendar() {
    let h = CalDavHarness::new();
    let source = h.source().await;
    source.check_for_changes().await.unwrap();
    h.server.expire_sync_tokens();
    h.server.store("home", "single.ics", &single_ics());
    assert_eq!(
        source.check_for_changes().await.unwrap(),
        events_changed(&["home"])
    );
    assert_eq!(source.check_for_changes().await.unwrap(), None);
}

#[tokio::test]
async fn a_server_without_sync_tokens_falls_back_to_the_ctag() {
    let h = CalDavHarness::new();
    h.server.configure(|s| s.supports_sync = false);
    let source = h.source().await;
    source.check_for_changes().await.unwrap();
    h.server.store("home", "single.ics", &single_ics());
    assert_eq!(
        source.check_for_changes().await.unwrap(),
        events_changed(&["home"])
    );
    assert!(h.server.requests("REPORT").is_empty());
}

#[tokio::test]
async fn auth_expired_ends_the_stream() {
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
    let mut changes = source.changes();
    let mut received = Vec::new();
    while let Some(change) = changes.recv().await {
        received.push(change);
    }
    assert_eq!(
        received,
        [CalendarChange::SourceFailed(SourceError::AuthExpired)]
    );
}

#[tokio::test]
async fn a_failure_on_one_calendar_does_not_swallow_anothers_change() {
    let h = CalDavHarness::new();
    h.server.configure(|s| {
        s.collections
            .insert("work".to_string(), Collection::new("Work"));
    });
    let source = h.source().await;
    source.check_for_changes().await.unwrap();
    h.server
        .store("home", "a.ics", &single_ics_with("a", "Dentist", &[]));
    h.server
        .store("work", "b.ics", &single_ics_with("b", "Dentist", &[]));
    h.server.fail("REPORT", "/work/", 500);
    assert_eq!(
        source.check_for_changes().await.unwrap_err(),
        Failure::Source(SourceError::Server { status: 500 })
    );
    // `home` was found changed before `work` failed; the retry must still report both.
    assert_eq!(
        source.check_for_changes().await.unwrap(),
        events_changed(&["home", "work"])
    );
    assert_eq!(source.check_for_changes().await.unwrap(), None);
}

#[tokio::test]
async fn switching_the_default_calendar_is_calendars_changed() {
    let h = CalDavHarness::new();
    h.server.configure(|s| {
        s.collections
            .insert("work".to_string(), Collection::new("Work"));
    });
    let source = h.source().await;
    source.check_for_changes().await.unwrap();
    assert_eq!(source.check_for_changes().await.unwrap(), None);
    h.server
        .configure(|s| s.default_calendar = Some("work".to_string()));
    assert_eq!(
        source.check_for_changes().await.unwrap(),
        Some(CalendarChange::CalendarsChanged)
    );
    assert_eq!(source.check_for_changes().await.unwrap(), None);
}
