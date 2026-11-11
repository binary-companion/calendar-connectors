use std::cell::RefCell;
use std::collections::BTreeSet;

use crate::connection::{InMemorySyncStateStore, SyncStateStore};
use crate::failure::{Failure, Outcome};
use crate::model::{Attendee, AttendeeRole, Participation, ResponseStatus};
use crate::shared_connector_helpers::{decode_response, detect_calendar_changes};
use crate::source_types::{CalendarChange, SourceError};
use crate::write::types::{EventRef, RecurrenceScope};

fn attendee(email: &str, response: ResponseStatus, is_self: bool) -> Attendee {
    Attendee::new(
        None,
        Some(email),
        AttendeeRole::Required,
        response,
        is_self,
        false,
    )
}

#[test]
fn participation_uses_the_self_attendees_response() {
    let me = attendee("me@x.com", ResponseStatus::Tentative, true);
    let other = attendee("o@x.com", ResponseStatus::Accepted, false);
    assert_eq!(
        Participation::resolving(&[other.clone(), me], Some(&other)),
        Participation::Invited(ResponseStatus::Tentative)
    );
}

#[test]
fn participation_counts_a_self_organizer_with_no_attendee_entry_as_accepted() {
    let me = attendee("me@x.com", ResponseStatus::NeedsAction, true);
    assert_eq!(
        Participation::resolving(&[], Some(&me)),
        Participation::Invited(ResponseStatus::Accepted)
    );
}

#[test]
fn participation_is_not_invited_when_you_are_neither_attendee_nor_organizer() {
    let other = attendee("o@x.com", ResponseStatus::NeedsAction, false);
    assert_eq!(
        Participation::resolving(std::slice::from_ref(&other), Some(&other)),
        Participation::NotInvited
    );
    assert_eq!(
        Participation::resolving(&[], None),
        Participation::NotInvited
    );
}

#[test]
fn write_target_is_the_instance_when_there_is_no_series() {
    let reference = EventRef::new("c", "e1");
    for scope in RecurrenceScope::ALL {
        let t = reference.write_target(*scope);
        assert!(t.id == "e1" && t.use_version, "{scope:?}");
    }
}

#[test]
fn write_target_addresses_the_master_for_all_in_series_without_the_instance_version() {
    let reference = EventRef::new("c", "e1_20260101").with_series_id(Some("e1"));
    let all = reference.write_target(RecurrenceScope::AllInSeries);
    assert!(all.id == "e1" && !all.use_version);
    let one = reference.write_target(RecurrenceScope::ThisInstance);
    assert!(one.id == "e1_20260101" && one.use_version);
}

#[test]
fn write_target_keeps_the_lock_when_the_ref_is_the_series_master() {
    let reference = EventRef::new("c", "e1").with_series_id(Some("e1"));
    let t = reference.write_target(RecurrenceScope::AllInSeries);
    assert!(t.id == "e1" && t.use_version);
}

// The empty-series-id normalisation that `write_target` relies on (Swift `EventRef.init` and `didSet`).
#[test]
fn an_empty_series_id_means_no_series() {
    let reference = EventRef::new("c", "e1").with_series_id(Some(""));
    assert_eq!(reference.series_id(), None);
    let mut other = EventRef::new("c", "e1").with_series_id(Some("s"));
    other.set_series_id(Some(String::new()));
    assert_eq!(other.series_id(), None);
    let t = reference.write_target(RecurrenceScope::AllInSeries);
    assert!(t.id == "e1" && t.use_version);
}

#[derive(Debug, PartialEq, serde::Deserialize)]
struct Item {
    n: i64,
}

#[test]
fn decode_response_decodes_valid_json() {
    assert_eq!(
        decode_response::<Item>(br#"{"n":3}"#).unwrap(),
        Item { n: 3 }
    );
}

#[test]
fn decode_response_throws_invalid_response_naming_the_type() {
    assert_eq!(
        decode_response::<Item>(b"nope").unwrap_err(),
        Failure::Source(SourceError::InvalidResponse(
            "could not decode Item".to_string()
        ))
    );
}

#[test]
fn decode_response_names_generic_types_without_module_paths() {
    assert_eq!(
        decode_response::<Vec<Item>>(b"nope").unwrap_err(),
        Failure::Source(SourceError::InvalidResponse(
            "could not decode Vec<Item>".to_string()
        ))
    );
}

#[test]
fn decode_response_never_panics_on_garbage() {
    let mut seed: u64 = 42;
    for _ in 0..2000 {
        let mut bytes = Vec::new();
        for _ in 0..(seed % 40) {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            bytes.push((seed >> 33) as u8);
        }
        assert!(decode_response::<Item>(&bytes).is_err());
    }
    let sample = br#"{"n":3}"#;
    for end in 0..sample.len() {
        assert!(decode_response::<Item>(&sample[..end]).is_err());
    }
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn first_check_takes_a_baseline_and_reports_nothing() {
    let state = InMemorySyncStateStore::new();
    let seen = RefCell::new(Vec::<String>::new());
    let change = detect_calendar_changes(&ids(&["b", "a"]), "set", "c1", &state, |id| {
        seen.borrow_mut().push(id);
        async { Ok(false) }
    })
    .await
    .unwrap();
    assert_eq!(change, None);
    assert_eq!(*seen.borrow(), ids(&["a", "b"]));
    assert_eq!(state.token("c1", "set").await.as_deref(), Some("a\nb"));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn reports_which_calendars_changed() {
    let state = InMemorySyncStateStore::new();
    state.set_token(Some("a\nb"), "c1", "set").await;
    let change = detect_calendar_changes(&ids(&["a", "b"]), "set", "c1", &state, |id| {
        let changed = id == "b";
        async move { Ok(changed) }
    })
    .await
    .unwrap();
    assert_eq!(
        change,
        Some(CalendarChange::EventsChanged {
            calendar_ids: Some(BTreeSet::from(["b".to_string()]))
        })
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_changed_calendar_set_wins_and_forgets_removed_calendars() {
    let state = InMemorySyncStateStore::new();
    state.set_token(Some("a\nb"), "c1", "set").await;
    state.set_token(Some("tok-b"), "c1", "b").await;
    let change = detect_calendar_changes(&ids(&["a"]), "set", "c1", &state, |_| async { Ok(true) })
        .await
        .unwrap();
    assert_eq!(change, Some(CalendarChange::CalendarsChanged));
    assert_eq!(state.token("c1", "b").await, None);
    assert_eq!(state.token("c1", "set").await.as_deref(), Some("a"));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn nothing_changed_returns_nil() {
    let state = InMemorySyncStateStore::new();
    state.set_token(Some("a"), "c1", "set").await;
    let change =
        detect_calendar_changes(&ids(&["a"]), "set", "c1", &state, |_| async { Ok(false) })
            .await
            .unwrap();
    assert_eq!(change, None);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_throwing_check_propagates() {
    let state = InMemorySyncStateStore::new();
    let result: Outcome<Option<CalendarChange>> =
        detect_calendar_changes(&ids(&["a"]), "set", "c1", &state, |_| async {
            Err(Failure::Source(SourceError::AuthExpired))
        })
        .await;
    assert_eq!(result, Err(Failure::Source(SourceError::AuthExpired)));
    // The failed check leaves the stored set untouched.
    assert_eq!(state.token("c1", "set").await, None);
}
