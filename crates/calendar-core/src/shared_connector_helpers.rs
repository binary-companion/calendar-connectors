//! Helpers the connectors share. Port of `SharedConnectorHelpers.swift`.

use std::collections::BTreeSet;
use std::future::Future;

use serde::de::DeserializeOwned;

use crate::connection::SyncStateStore;
use crate::failure::{Failure, Outcome};
use crate::model::{Attendee, Participation, ResponseStatus};
use crate::source_types::{CalendarChange, SourceError};
use crate::write::types::{EventRef, RecurrenceScope};

impl Participation {
    /// A `self` attendee gives their response; an organizer who is you with no attendee entry (an event with no
    /// guests) counts as accepted; anything else is an event you are not on.
    pub fn resolving(attendees: &[Attendee], organizer: Option<&Attendee>) -> Participation {
        if let Some(me) = attendees.iter().find(|a| a.is_self) {
            return Participation::Invited(me.response);
        }
        if organizer.is_some_and(|o| o.is_self) {
            return Participation::Invited(ResponseStatus::Accepted);
        }
        Participation::NotInvited
    }
}

/// The result of [`EventRef::write_target`] (Swift's labelled tuple `(id:, useVersion:)`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteTarget {
    pub id: String,
    pub use_version: bool,
}

impl EventRef {
    /// The id a write addresses: the instance, or the series master for `AllInSeries` (whose version differs from the
    /// instance's, so the caller's version cannot lock it; a master ref's own version can).
    pub fn write_target(&self, scope: RecurrenceScope) -> WriteTarget {
        let Some(series) = self.series_id().filter(|s| !s.is_empty()) else {
            return WriteTarget {
                id: self.event_id.clone(),
                use_version: true,
            };
        };
        if scope == RecurrenceScope::AllInSeries {
            WriteTarget {
                id: series.to_string(),
                use_version: series == self.event_id,
            }
        } else {
            WriteTarget {
                id: self.event_id.clone(),
                use_version: true,
            }
        }
    }
}

/// `String(describing: T.self)` for a Swift type: the type's own name without module paths, generics kept.
fn short_type_name<T>() -> String {
    let full = std::any::type_name::<T>();
    let mut out = String::new();
    let mut segment = String::new();
    for ch in full.chars() {
        if ch.is_alphanumeric() || ch == '_' || ch == ':' {
            segment.push(ch);
        } else {
            out.push_str(segment.rsplit("::").next().unwrap_or(""));
            segment.clear();
            out.push(ch);
        }
    }
    out.push_str(segment.rsplit("::").next().unwrap_or(""));
    out
}

/// Decodes a provider response body; any failure becomes `SourceError::InvalidResponse` naming the type (the Rust
/// type name without its module path, so a DTO keeps the name its Swift counterpart had).
pub fn decode_response<T: DeserializeOwned>(data: &[u8]) -> Outcome<T> {
    serde_json::from_slice(data).map_err(|_| {
        Failure::Source(SourceError::InvalidResponse(format!(
            "could not decode {}",
            short_type_name::<T>()
        )))
    })
}

/// One incremental check across a connection's calendars. `check_calendar` returns whether that calendar changed; the
/// first call for a calendar takes its baseline and returns false. A change to the calendar set itself wins over
/// event changes, and calendars that left the set lose their stored tokens.
pub async fn detect_calendar_changes<F, Fut>(
    calendar_ids: &[String],
    set_scope: &str,
    connection_id: &str,
    sync_state: &dyn SyncStateStore,
    mut check_calendar: F,
) -> Outcome<Option<CalendarChange>>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Outcome<bool>>,
{
    let mut ids: Vec<String> = calendar_ids.to_vec();
    ids.sort();
    let set_key = ids.join("\n");

    let previous_key = sync_state.token(connection_id, set_scope).await;
    let set_changed = previous_key
        .as_deref()
        .is_some_and(|previous| previous != set_key);
    if let (Some(previous_key), true) = (&previous_key, set_changed) {
        let current: BTreeSet<&str> = ids.iter().map(String::as_str).collect();
        let before: BTreeSet<&str> = previous_key.split('\n').filter(|s| !s.is_empty()).collect();
        for removed in before.difference(&current) {
            sync_state.set_token(None, connection_id, removed).await;
        }
    }

    let mut changed = BTreeSet::new();
    for id in &ids {
        if check_calendar(id.clone()).await? {
            changed.insert(id.clone());
        }
    }
    sync_state
        .set_token(Some(&set_key), connection_id, set_scope)
        .await;

    if set_changed {
        return Ok(Some(CalendarChange::CalendarsChanged));
    }
    if changed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(CalendarChange::EventsChanged {
            calendar_ids: Some(changed),
        }))
    }
}
