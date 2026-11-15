//! Opt-in, interactive live write smoke test against a real Google account (port of `GoogleLiveWriteSmokeTests.swift`).
//!
//! It is `#[ignore]`d and also needs `TIMETUG_LIVE_GOOGLE=1`, so a plain `cargo test` never touches an account:
//!
//! ```text
//! TIMETUG_LIVE_GOOGLE=1 GOOGLE_OAUTH_CLIENT_ID=... GOOGLE_OAUTH_CLIENT_SECRET=... \
//!   cargo test -p calendar-google live_write_smoke -- --ignored --nocapture
//! ```
//!
//! Signs in through the browser, creates events named "TimeTug write smoke" in the primary calendar (no attendees,
//! nothing is emailed), and deletes them at the end, including when it fails half way. Nothing without that title
//! prefix is ever deleted.
//!
//! What it settles (the unit tests only use a fake transport for these), printed as `LIVE ...` lines to record in the
//! spec:
//! - `If-Match` etag behaviour on PATCH: the stale-edit merge and conflict below.
//! - The recurring series' zone: the main series uses an IANA zone (`Europe/London`). A separate UTC-zoned recurring
//!   event exercises the "UTC" spelling (the zone `UTC` is reported as `GMT`, which the mapper sends as "UTC"); it
//!   prints what Google stored as `LIVE UTC ...`, and a request error as `LIVE UTC failure: ...`.
//! - The `.thisAndFollowing` split and its `COUNT` arithmetic (old series keeps the earlier occurrences, the new series
//!   gets `total - prior`): printed as the instance count per series after the split (expected `[2, 2]`).
//! - The Meet re-request on the new series: printed as each instance's conference provider before and after the split.
//! - A split when the series already has exceptions AFTER the split point (one occurrence modified, another cancelled
//!   through the API): what the calendar shows afterwards is printed as `LIVE split with later exceptions ...`.
//! - `supportsAttachments`: NOT exercised. The series has no attachments, so the split never sends that parameter;
//!   confirming it needs an event with a Drive attachment, added by hand.
//!
//! The HTTP is the system `curl` (the library has no HTTP client) and the sign-in listens on a loopback port; both are
//! in `calendar_test_support::live`. Unix only.

#![cfg(unix)]
#![allow(clippy::print_stdout)]

use std::collections::BTreeMap;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use calendar_core::connection::{ConnectorKind, InMemoryCredentialStore, InMemorySyncStateStore};
use calendar_core::failure::Failure;
use calendar_core::model::CalendarEvent;
use calendar_core::recurrence::rule::{Frequency, RecurrenceRule, RuleEnd};
use calendar_core::source_types::CalendarSource;
use calendar_core::time::{DateInterval, Zone, add_seconds};
use calendar_core::write::event_draft::{EventDraft, EventTiming};
use calendar_core::write::event_patch::{EventPatch, FieldUpdate};
use calendar_core::write::types::{
    ConferenceRequest, EventField, EventRef, NotifyPolicy, RecurrenceScope, WriteError,
};
use calendar_core::write::writable_source::WritableCalendarSource;
use calendar_test_support::hand_patch;
use calendar_test_support::live::{
    BrowserInteraction, CurlTransport, SmokeCleaner, live_now, live_sleeper, require_env, switch_on,
};
use jiff::Timestamp;

use crate::connector_kind::{GoogleConnectorKind, GoogleOAuthConfig};

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

fn providers(list: &[CalendarEvent]) -> Vec<String> {
    list.iter()
        .map(|event| {
            event
                .conference()
                .map_or("-".to_string(), |c| format!("{:?}", c.provider))
        })
        .collect()
}

fn titled<'a>(list: &'a [CalendarEvent], prefix: &str) -> Vec<&'a CalendarEvent> {
    list.iter()
        .filter(|e| e.title.starts_with(prefix))
        .collect()
}

fn writable(source: &Arc<dyn CalendarSource>) -> Res<&dyn WritableCalendarSource> {
    source
        .as_writable()
        .ok_or_else(|| "the Google source is not writable".into())
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "live: writes to a real Google account; set TIMETUG_LIVE_GOOGLE=1"]
async fn google_write_smoke() {
    if !switch_on("TIMETUG_LIVE_GOOGLE") {
        println!("TIMETUG_LIVE_GOOGLE is not 1; the live Google test does nothing");
        return;
    }
    let client_id = require_env("GOOGLE_OAUTH_CLIENT_ID");
    let client_secret = require_env("GOOGLE_OAUTH_CLIENT_SECRET");
    let kind = GoogleConnectorKind::new(
        GoogleOAuthConfig::new(client_id, client_secret),
        Arc::new(CurlTransport),
        live_now(),
        live_sleeper(),
    );
    let credentials = Arc::new(InMemoryCredentialStore::new());
    let connection = kind
        .authorize(&BrowserInteraction, credentials.as_ref())
        .await
        .expect("sign-in failed");
    let source = kind
        .make_source(
            &connection,
            credentials.clone(),
            Arc::new(InMemorySyncStateStore::new()),
        )
        .expect("could not make the source");
    let calendars = source.calendars().await.expect("could not list calendars");
    let primary = calendars
        .iter()
        .find(|calendar| calendar.is_default == Some(true))
        .expect("no primary calendar")
        .clone();
    println!("LIVE signed in; writing to the primary calendar");

    // The main flow uses an IANA zone; the UTC spelling is exercised separately (see the module comment).
    let zone = Zone::named("Europe/London").expect("Europe/London");
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
    // The longest series (weekly, six occurrences, from day 2) ends about day 37; the window covers all of it.
    let window = DateInterval::new(
        add_seconds(start, -3600.0),
        add_seconds(start, 86_400.0 * 40.0),
    );
    let cleaner = SmokeCleaner::new(source.clone(), SMOKE_PREFIX, &primary.id, window);

    // Leftovers from an earlier run that died before it could clean up.
    let earlier = cleaner.mine().await.expect("could not list events");
    if !earlier.is_empty() {
        println!(
            "LIVE removing {} smoke events left by an earlier run",
            earlier.len()
        );
        cleaner.clean_up().await;
    }

    let mut failures = Vec::new();
    let run = tokio::time::timeout(
        Duration::from_secs(600),
        exercise(&source, &primary.id, &zone, start, &cleaner, &mut failures),
    )
    .await;
    match run {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            println!("LIVE failure: {error}");
            failures.push(format!("the run stopped: {error}"));
        }
        Err(_) => failures.push("the run took longer than 10 minutes".to_string()),
    }
    if !failures.is_empty() {
        cleaner.clean_up().await;
    }
    match cleaner.poll(<[CalendarEvent]>::is_empty).await {
        Ok(leftovers) => {
            expect(
                &mut failures,
                leftovers.is_empty(),
                format!(
                    "smoke events were left behind: {:?}",
                    leftovers.iter().map(|e| &e.title).collect::<Vec<_>>()
                ),
            );
            if !leftovers.is_empty() {
                cleaner.clean_up().await;
            }
        }
        Err(error) => {
            failures.push(format!("could not check for leftovers: {error}"));
            cleaner.clean_up().await;
        }
    }
    assert!(failures.is_empty(), "live smoke failed: {failures:#?}");
}

/// The four flows. Events it creates are recorded in `cleaner` first, so a failure anywhere can delete them.
async fn exercise(
    source: &Arc<dyn CalendarSource>,
    primary_id: &str,
    zone: &Zone,
    start: Timestamp,
    cleaner: &SmokeCleaner,
    failures: &mut Vec<String>,
) -> Res<()> {
    let writable = writable(source)?;
    let timing = |offset_days: i64| {
        let s = add_seconds(start, 86_400.0 * offset_days as f64);
        EventTiming::new(s, add_seconds(s, 1800.0), Some(zone.clone()), false)
    };
    let scope_all = RecurrenceScope::AllInSeries;

    // 1. A single event: create, edit, then a stale edit merges and a conflicting one fails.
    let mut draft = EventDraft::new(SMOKE_PREFIX, timing(0));
    draft.location = Some("Room 1".to_string());
    let single = writable
        .create(&draft, primary_id, NotifyPolicy::None)
        .await?;
    cleaner.record_created(&single);
    expect(
        failures,
        single.title == SMOKE_PREFIX && single.source_id.as_deref() == Some(source.id().as_str()),
        "the created event has the wrong title or source id",
    );
    let mut edited = single.clone();
    edited.title = format!("{SMOKE_PREFIX} renamed");
    let renamed = writable
        .update(
            &EventRef::from_event(&single),
            &EventPatch::diff(&single, &edited),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await?;
    expect(
        failures,
        renamed.title == format!("{SMOKE_PREFIX} renamed")
            && renamed.location.as_deref() == Some("Room 1")
            && renamed.version != single.version,
        "the rename did not keep the location or change the version",
    );
    // `single` is now stale. A patch to the notes only merges; a patch to the title conflicts.
    let mut notes_edit = single.clone();
    notes_edit.notes = Some("merged".to_string());
    let merged = writable
        .update(
            &EventRef::from_event(&single),
            &EventPatch::diff(&single, &notes_edit),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await?;
    expect(
        failures,
        merged.notes.as_deref() == Some("merged")
            && merged.title == format!("{SMOKE_PREFIX} renamed"),
        "the stale notes edit did not merge",
    );
    let mut title_edit = single.clone();
    title_edit.title = format!("{SMOKE_PREFIX} mine");
    match writable
        .update(
            &EventRef::from_event(&single),
            &EventPatch::diff(&single, &title_edit),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await
    {
        Err(Failure::Write(WriteError::Conflict { fields })) => expect(
            failures,
            fields == [EventField::Title].into(),
            format!("conflict on the wrong fields: {fields:?}"),
        ),
        other => failures.push(format!("expected a conflict, got {other:?}")),
    }
    cleaner
        .remove(&merged, RecurrenceScope::ThisInstance)
        .await?;

    // 2. A weekly series of four: instance, this-and-following and whole-series edits, then delete. Anything that
    // indexes `list` is guarded by a `require`, so a short list ends the run (and the cleanup) instead of panicking.
    let series_rule = |count| RecurrenceRule {
        end: RuleEnd::Count(count),
        ..RecurrenceRule::new(Frequency::Weekly)
    };
    let series_title = format!("{SMOKE_PREFIX} series");
    let mut wanted_meet = true;
    let mut series_draft = EventDraft {
        conference: ConferenceRequest::Generate,
        recurrence: Some(series_rule(4)),
        ..EventDraft::new(&series_title, timing(2))
    };
    let master = match writable
        .create(&series_draft, primary_id, NotifyPolicy::None)
        .await
    {
        Ok(master) => master,
        Err(error) => {
            // The Meet request is the part most likely to differ by account type; keep going without it.
            println!("LIVE creating the series with a Meet link failed, retrying without: {error}");
            wanted_meet = false;
            series_draft.conference = ConferenceRequest::None;
            writable
                .create(&series_draft, primary_id, NotifyPolicy::None)
                .await?
        }
    };
    cleaner.record_created(&master);
    // Google adds a Meet link a moment after the insert, so wait for it before recording what the split does to it.
    let mut list = cleaner
        .poll(|l| l.len() == 4 && (!wanted_meet || l.iter().all(|e| e.conference().is_some())))
        .await?;
    if wanted_meet {
        if list.iter().all(|e| e.conference().is_some()) {
            println!("LIVE conference before the split: {:?}", providers(&list));
        } else {
            println!(
                "LIVE conference pending (no Meet link after 15 s): {:?}",
                providers(&list)
            );
        }
    }
    require(
        list.len() == 4,
        format!("expected 4 instances of the series, saw {}", list.len()),
    )?;
    expect(
        failures,
        list.iter()
            .all(|e| e.series_id().is_some() && e.original_start().is_some()),
        "an instance has no series id or original start",
    );
    writable
        .update(
            &EventRef::from_event(&list[1]),
            &hand_patch(|p| p.title = Some(format!("{series_title} (second)"))),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await?;
    writable
        .update(
            &EventRef::from_event(&list[2]),
            &hand_patch(|p| p.location = FieldUpdate::Set("Lab".to_string())),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await?;
    list = cleaner
        .poll(|l| {
            l.len() == 4
                && l[2].location.as_deref() == Some("Lab")
                && l[3].location.as_deref() == Some("Lab")
                && (!wanted_meet || l.iter().all(|e| e.conference().is_some()))
        })
        .await?;
    println!(
        "LIVE after split: {:?}",
        list.iter()
            .map(|e| (
                e.title.clone(),
                e.location.clone().unwrap_or("-".to_string())
            ))
            .collect::<Vec<_>>()
    );
    // COUNT arithmetic: the old series keeps the two earlier occurrences and the new series gets the other two.
    let mut per_series: BTreeMap<String, usize> = BTreeMap::new();
    for event in &list {
        *per_series
            .entry(event.series_id().unwrap_or(&event.event_id).to_string())
            .or_default() += 1;
    }
    let mut counts: Vec<usize> = per_series.values().copied().collect();
    counts.sort_unstable();
    println!("LIVE instances per series after split (expected [2, 2]): {counts:?}");
    if wanted_meet {
        if list.iter().all(|e| e.conference().is_some()) {
            println!("LIVE conference after the split: {:?}", providers(&list));
        } else {
            println!(
                "LIVE conference pending (not on every instance after the split): {:?}",
                providers(&list)
            );
        }
    }
    require(
        list.len() == 4,
        format!("expected 4 instances after the split, saw {}", list.len()),
    )?;
    expect(
        failures,
        list[0].location.is_none()
            && list[1].title.ends_with("(second)")
            && list[2].location.as_deref() == Some("Lab")
            && list[3].location.as_deref() == Some("Lab"),
        "the split did not leave the expected titles and locations",
    );
    writable
        .update(
            &EventRef::from_event(&list[3]),
            &hand_patch(|p| p.notes = FieldUpdate::Set("whole series".to_string())),
            scope_all,
            NotifyPolicy::None,
        )
        .await?;
    list = cleaner
        .poll(|l| {
            l.last()
                .is_some_and(|e| e.notes.as_deref() == Some("whole series"))
        })
        .await?;
    // Instances after the split belong to the new series, so only the series the ref points at is guaranteed to change.
    println!(
        "LIVE notes after allInSeries: {:?}",
        list.iter()
            .map(|e| e.notes.clone().unwrap_or("-".to_string()))
            .collect::<Vec<_>>()
    );
    // The split left the old and the new series both present; `remove` deletes each once.
    for event in &list {
        cleaner.remove(event, scope_all).await?;
    }

    // 3. A split with exceptions after the split point. The events of this step are told apart by title, because the
    // polls above count every smoke event.
    let later_title = format!("{SMOKE_PREFIX} later exceptions");
    let later = writable
        .create(
            &EventDraft {
                recurrence: Some(series_rule(6)),
                ..EventDraft::new(&later_title, timing(2))
            },
            primary_id,
            NotifyPolicy::None,
        )
        .await?;
    cleaner.record_created(&later);
    let all = cleaner.poll(|l| titled(l, &later_title).len() == 6).await?;
    let mut later_list: Vec<CalendarEvent> =
        titled(&all, &later_title).into_iter().cloned().collect();
    require(
        later_list.len() == 6,
        format!(
            "expected 6 instances of the later-exceptions series, saw {}",
            later_list.len()
        ),
    )?;
    let modified_slot = later_list[4].start;
    let cancelled_slot = later_list[5].start;
    writable
        .update(
            &EventRef::from_event(&later_list[4]),
            &hand_patch(|p| p.title = Some(format!("{later_title} (modified)"))),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await?;
    writable
        .delete(
            &EventRef::from_event(&later_list[5]),
            RecurrenceScope::ThisInstance,
            NotifyPolicy::None,
        )
        .await?;
    let all = cleaner
        .poll(|l| {
            let mine = titled(l, &later_title);
            mine.len() == 5 && mine.iter().any(|e| e.title.ends_with("(modified)"))
        })
        .await?;
    later_list = titled(&all, &later_title).into_iter().cloned().collect();
    println!(
        "LIVE later exceptions before the split (expected 5 shown): {}",
        later_list.len()
    );
    require(
        later_list.len() == 5,
        format!(
            "expected 5 instances before the split, saw {}",
            later_list.len()
        ),
    )?;
    writable
        .update(
            &EventRef::from_event(&later_list[2]),
            &hand_patch(|p| p.location = FieldUpdate::Set("Lab".to_string())),
            RecurrenceScope::ThisAndFollowing,
            NotifyPolicy::None,
        )
        .await?;
    // The listing settles a moment after the split; wait until the new series shows, then a little longer for the rest.
    cleaner
        .poll(|l| {
            titled(l, &later_title)
                .iter()
                .any(|e| e.location.as_deref() == Some("Lab"))
        })
        .await?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let all = cleaner.mine().await?;
    later_list = titled(&all, &later_title).into_iter().cloned().collect();
    println!(
        "LIVE split with later exceptions: {} instances shown (before the split: 5, with one cancelled and one modified)",
        later_list.len()
    );
    println!(
        "LIVE split with later exceptions: {:?}",
        later_list
            .iter()
            .map(|e| (
                e.title.get(later_title.len()..).unwrap_or("").to_string(),
                e.location.clone().unwrap_or("-".to_string()),
                e.start.to_string(),
                e.series_id().unwrap_or("-").to_string(),
            ))
            .collect::<Vec<_>>()
    );
    let mut per_series: BTreeMap<String, usize> = BTreeMap::new();
    for event in &later_list {
        *per_series
            .entry(event.series_id().unwrap_or(&event.event_id).to_string())
            .or_default() += 1;
    }
    let mut counts: Vec<usize> = per_series.values().copied().collect();
    counts.sort_unstable();
    println!(
        "LIVE split with later exceptions: cancelled occurrence reappeared = {}, modified occurrence shown {} time(s) at its slot, instances per series {counts:?}",
        later_list.iter().any(|e| e.start == cancelled_slot),
        later_list
            .iter()
            .filter(|e| e.start == modified_slot)
            .count()
    );
    for event in &later_list {
        cleaner.remove(event, scope_all).await?;
    }

    // 4. A UTC-zoned recurring event: Google must accept the zone name "UTC" (and a rule read in it). A failure here
    // is the finding, so it is printed instead of ending the run.
    let utc = Zone::named("UTC").ok_or("no UTC zone")?;
    let utc_start = add_seconds(start, 86_400.0);
    let utc_title = format!("{SMOKE_PREFIX} utc");
    let utc_draft = EventDraft {
        recurrence: Some(series_rule(2)),
        ..EventDraft::new(
            &utc_title,
            EventTiming::new(utc_start, add_seconds(utc_start, 1800.0), Some(utc), false),
        )
    };
    let utc_result: Res<()> = async {
        let utc_event = writable.create(&utc_draft, primary_id, NotifyPolicy::None).await?;
        cleaner.record_created(&utc_event);
        let all = cleaner.poll(|l| titled(l, &utc_title).len() == 2).await?;
        let utc_list = titled(&all, &utc_title);
        println!(
            "LIVE UTC event: {} instances (expected 2), zone {:?}, starts {:?} (first expected {utc_start})",
            utc_list.len(),
            utc_list.iter().map(|e| e.time_zone.identifier().to_string()).collect::<Vec<_>>(),
            utc_list.iter().map(|e| e.start.to_string()).collect::<Vec<_>>(),
        );
        for event in utc_list {
            cleaner.remove(event, scope_all).await?;
        }
        Ok(())
    }
    .await;
    if let Err(error) = utc_result {
        println!("LIVE UTC failure: {error}");
    }
    Ok(())
}
