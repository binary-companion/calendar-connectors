//! Opt-in, interactive live write smoke test against a real Microsoft account (port of
//! `MicrosoftLiveWriteSmokeTests.swift`).
//!
//! It is `#[ignore]`d and also needs `TIMETUG_LIVE_MICROSOFT=1`, so a plain `cargo test` never touches an account:
//!
//! ```text
//! TIMETUG_LIVE_MICROSOFT=1 MICROSOFT_OAUTH_CLIENT_ID=<id> \
//!   cargo test -p calendar-microsoft live_write_smoke -- --ignored --nocapture
//! ```
//!
//! (read the id from your xcconfig; do not paste it into shared logs). Every event it creates starts with
//! "TimeTug write smoke" and nothing without that prefix is ever deleted. Prints `LIVE ...` lines; record the answers
//! in the Phase 4 spec:
//! - the sign-in with the `http://localhost` redirect and the account's identity;
//! - the calendars and their permissions, and the account's zone (`LIVE calendar ...`);
//! - a created single event's version, uid scope and stored zone, and whether an edit changes the version;
//! - a Teams meeting on `Generate` (a personal account may refuse it: `LIVE teams: ...`);
//! - a weekly numbered series: the occurrences read back, and `ThisAndFollowing` on the third occurrence, printed as the
//!   instance count per series after the split (expected `[2, 2]`), including the numbered-count arithmetic;
//! - `original_start` on each occurrence (`LIVE occurrence ...`), which the split depends on.
//!
//! The HTTP is the system `curl` (the library has no HTTP client) and the sign-in listens on a loopback port; both are
//! in `calendar_test_support::live`. Unix only.

#![cfg(unix)]
#![allow(clippy::print_stdout)]

use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use calendar_core::connection::{ConnectorKind, InMemoryCredentialStore, InMemorySyncStateStore};
use calendar_core::model::CalendarEvent;
use calendar_core::recurrence::rule::{Frequency, RecurrenceRule, RuleEnd};
use calendar_core::source_types::CalendarSource;
use calendar_core::time::{DateInterval, Zone, add_seconds};
use calendar_core::write::event_draft::{EventDraft, EventTiming};
use calendar_core::write::types::{ConferenceRequest, EventRef, NotifyPolicy, RecurrenceScope};
use calendar_test_support::hand_patch;
use calendar_test_support::live::{
    BrowserInteraction, CurlTransport, SmokeCleaner, live_now, live_sleeper, require_env, switch_on,
};
use jiff::Timestamp;

use crate::connector_kind::{MicrosoftConnectorKind, MicrosoftOAuthConfig};

/// Every event this test creates starts with this prefix, and nothing without it is ever deleted.
const SMOKE_PREFIX: &str = "TimeTug write smoke";

type Res<T> = Result<T, Box<dyn Error>>;

/// A condition the test cannot go on without (Swift `try #require`).
fn require(condition: bool, message: impl Into<String>) -> Res<()> {
    if condition {
        Ok(())
    } else {
        Err(message.into().into())
    }
}

/// Records a failed expectation and goes on (Swift `#expect`); the test fails at the end, after the cleanup.
fn expect(failures: &mut Vec<String>, condition: bool, message: impl Into<String>) {
    if !condition {
        failures.push(message.into());
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "live: writes to a real Microsoft account; set TIMETUG_LIVE_MICROSOFT=1"]
async fn microsoft_write_smoke() {
    if !switch_on("TIMETUG_LIVE_MICROSOFT") {
        println!("TIMETUG_LIVE_MICROSOFT is not 1; the live Microsoft test does nothing");
        return;
    }
    let client_id = require_env("MICROSOFT_OAUTH_CLIENT_ID");
    let kind = MicrosoftConnectorKind::new(
        MicrosoftOAuthConfig::new(client_id),
        Arc::new(CurlTransport),
        live_now(),
        live_sleeper(),
    );
    let credentials = Arc::new(InMemoryCredentialStore::new());
    let connection = kind
        .authorize(&BrowserInteraction, credentials.as_ref())
        .await
        .expect("sign-in failed");
    println!(
        "LIVE signed in ({})",
        if connection.display_name.contains('@') {
            "identity read"
        } else {
            "no identity"
        }
    );
    let source = kind
        .make_source(
            &connection,
            credentials.clone(),
            Arc::new(InMemorySyncStateStore::new()),
        )
        .expect("could not make the source");
    let calendars = source.calendars().await.expect("could not list calendars");
    for calendar in &calendars {
        println!(
            "LIVE calendar default={} canEdit={} zone={}",
            calendar.is_default == Some(true),
            calendar.permissions.can_edit,
            calendar
                .time_zone
                .as_ref()
                .map_or("nil", |z| z.identifier()),
        );
    }
    let primary = calendars
        .iter()
        .find(|calendar| calendar.is_default == Some(true))
        .expect("no primary calendar")
        .clone();
    // A missing zone here means the connector did not read one.
    let zone = primary
        .time_zone
        .clone()
        .expect("the primary calendar has no zone");
    let tomorrow = Timestamp::now()
        .to_zoned(zone.as_jiff().clone())
        .tomorrow()
        .expect("tomorrow");
    let start = tomorrow
        .with()
        .hour(3)
        .minute(0)
        .second(0)
        .subsec_nanosecond(0)
        .build()
        .expect("03:00 tomorrow")
        .timestamp();
    // The series below ends about day 23 (weekly, four occurrences from day 2).
    let window = DateInterval::new(
        add_seconds(start, -3600.0),
        add_seconds(start, 86_400.0 * 30.0),
    );
    let cleaner = SmokeCleaner::new(source.clone(), SMOKE_PREFIX, &primary.id, window);
    cleaner.clean_up().await; // leftovers from an earlier run

    let mut failures = Vec::new();
    let run = tokio::time::timeout(
        Duration::from_secs(600),
        exercise(&source, &primary.id, &zone, start, &cleaner, &mut failures),
    )
    .await;
    match run {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            println!("LIVE smoke failed: {error}");
            failures.push(format!("the run stopped: {error}"));
        }
        Err(_) => failures.push("the run took longer than 10 minutes".to_string()),
    }
    cleaner.clean_up().await;
    assert!(failures.is_empty(), "live smoke failed: {failures:#?}");
}

/// The three flows. Events it creates are recorded in `cleaner` first, so a failure anywhere can delete them.
async fn exercise(
    source: &Arc<dyn CalendarSource>,
    primary_id: &str,
    zone: &Zone,
    start: Timestamp,
    cleaner: &SmokeCleaner,
    failures: &mut Vec<String>,
) -> Res<()> {
    let writable = source
        .as_writable()
        .ok_or("the Microsoft source is not writable")?;
    let timing = |offset_days: i64| {
        let s = add_seconds(start, 86_400.0 * offset_days as f64);
        EventTiming::new(s, add_seconds(s, 1800.0), Some(zone.clone()), false)
    };

    // 1. A single event: create, edit, and see whether the version changes.
    let mut draft = EventDraft::new(SMOKE_PREFIX, timing(0));
    draft.location = Some("Room 1".to_string());
    let single = writable
        .create(&draft, primary_id, NotifyPolicy::None)
        .await?;
    cleaner.record_created(&single);
    println!(
        "LIVE single uidScope={:?} zone={} version={}",
        single.uid_scope,
        single.time_zone.identifier(),
        single.version.is_some()
    );
    let renamed = writable
        .update(
            &EventRef::from_event(&single),
            &hand_patch(|p| p.title = Some(format!("{SMOKE_PREFIX} renamed"))),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await?;
    expect(
        failures,
        renamed.title == format!("{SMOKE_PREFIX} renamed")
            && renamed.location.as_deref() == Some("Room 1"),
        "the rename did not keep the location",
    );
    println!(
        "LIVE edit changed the version: {}",
        renamed.version != single.version
    );
    cleaner
        .remove(&renamed, RecurrenceScope::ThisInstance)
        .await?;

    // 2. Teams on generate. A personal account may refuse it; either answer is recorded.
    let teams_draft = EventDraft {
        conference: ConferenceRequest::Generate,
        ..EventDraft::new(format!("{SMOKE_PREFIX} teams"), timing(1))
    };
    match writable
        .create(&teams_draft, primary_id, NotifyPolicy::None)
        .await
    {
        Ok(meeting) => {
            cleaner.record_created(&meeting);
            println!(
                "LIVE teams: created, conferences={:?}",
                meeting
                    .conferences
                    .iter()
                    .map(|c| format!("{:?}", c.provider))
                    .collect::<Vec<_>>()
            );
            cleaner
                .remove(&meeting, RecurrenceScope::ThisInstance)
                .await?;
        }
        Err(error) => println!("LIVE teams: {error}"),
    }

    // 3. A weekly numbered series, read back, then split at the third occurrence.
    let series_title = format!("{SMOKE_PREFIX} series");
    let master = writable
        .create(
            &EventDraft {
                recurrence: Some(RecurrenceRule {
                    end: RuleEnd::Count(4),
                    ..RecurrenceRule::new(Frequency::Weekly)
                }),
                ..EventDraft::new(&series_title, timing(2))
            },
            primary_id,
            NotifyPolicy::None,
        )
        .await?;
    cleaner.record_created(&master);
    let exactly = |l: &[CalendarEvent], title: &str| l.iter().filter(|e| e.title == title).count();
    let list = cleaner.poll(|l| exactly(l, &series_title) == 4).await?;
    let occurrences: Vec<&CalendarEvent> =
        list.iter().filter(|e| e.title == series_title).collect();
    for occurrence in &occurrences {
        println!(
            "LIVE occurrence start={} originalStart={:?} seriesID={}",
            occurrence.start,
            occurrence.original_start(),
            occurrence.series_id().is_some()
        );
    }
    require(
        occurrences.len() == 4,
        format!("expected 4 occurrences, saw {}", occurrences.len()),
    )?;
    let third = occurrences[2];
    let new_title = format!("{SMOKE_PREFIX} series 2");
    writable
        .update(
            &EventRef::from_event(third),
            &hand_patch(|p| p.title = Some(new_title.clone())),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await?;
    let counts_of = |l: &[CalendarEvent]| [exactly(l, &series_title), exactly(l, &new_title)];
    let now = cleaner.poll(|l| counts_of(l) == [2, 2]).await?;
    let counts = counts_of(&now);
    println!("LIVE split instance counts [old, new] = {counts:?}");
    expect(
        failures,
        counts == [2, 2],
        format!("split counts were {counts:?}"),
    );
    Ok(())
}
