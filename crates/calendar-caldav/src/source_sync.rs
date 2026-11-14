//! Port of `CalDAVCalendarSource+Sync.swift`: change detection by sync token (RFC 6578) or ctag.

use std::collections::BTreeSet;

use async_trait::async_trait;
use calendar_core::failure::{Failure, Outcome};
use calendar_core::source_types::{CalendarChange, PollingCalendarSource, SourceError};

use crate::dav_xml::DavXml;
use crate::source::{CalDavCalendarInfo, CalDavCalendarSource};

impl CalDavCalendarSource {
    pub(crate) const CALENDARS_SCOPE: &'static str = "calendars";

    pub(crate) fn calendar_scope(id: &str) -> String {
        format!("calendar:{id}")
    }

    /// What a consumer must reload the calendar list for: calendars added or removed, renamed, recoloured, access,
    /// zone or default status changed. Event changes (the ctag) are not part of it.
    pub(crate) fn signature(calendars: &[CalDavCalendarInfo]) -> String {
        let mut lines: Vec<String> = calendars
            .iter()
            .map(|calendar| {
                let d = &calendar.descriptor;
                [
                    d.id.clone(),
                    d.title.clone(),
                    d.color_hex().unwrap_or("").to_string(),
                    d.permissions.can_edit.to_string(),
                    d.permissions.can_view_details.to_string(),
                    calendar.zone.identifier().to_string(),
                    d.is_default.map(|b| b.to_string()).unwrap_or_default(),
                ]
                .join("\u{1F}")
            })
            .collect();
        // Known limitation: Swift orders non-ASCII text by canonical equivalence, Rust by bytes.
        lines.sort();
        lines.join("\u{1E}")
    }

    /// `sync:<token>` when the calendar has a sync token (RFC 6578), else `ctag:<ctag>`.
    pub(crate) fn marker(calendar: &CalDavCalendarInfo) -> String {
        match &calendar.sync_token {
            Some(token) if !token.is_empty() => format!("sync:{token}"),
            _ => format!("ctag:{}", calendar.ctag.as_deref().unwrap_or("")),
        }
    }

    /// Whether the calendar changed, and the marker to store once the whole check has succeeded (`None`: keep the old one).
    async fn check(&self, calendar: &CalDavCalendarInfo) -> Outcome<(bool, Option<String>)> {
        let owner = &self.connection.connection_id;
        let current = Self::marker(calendar);
        let stored = self
            .sync_state
            .token(owner, &Self::calendar_scope(&calendar.descriptor.id))
            .await;
        if stored.as_deref() == Some(current.as_str()) {
            return Ok((false, None));
        }
        // A ctag server (or a calendar that changed how it reports) has nothing finer to ask.
        let Some(stored) =
            stored.filter(|s| s.starts_with("sync:") && current.starts_with("sync:"))
        else {
            return Ok((true, Some(current)));
        };
        let reply = self
            .client
            .report(
                &calendar.url,
                1,
                DavXml::sync_collection(Some(&stored["sync:".len()..])),
            )
            .await?;
        if DavXml::is_invalid_sync_token(&reply.response) {
            return Ok((true, Some(current)));
        }
        match reply.response.status {
            207 => {}
            // Removed or lost access since the list was read: the next list signature reports it.
            403 | 404 | 410 => return Ok((false, None)),
            other => {
                return Err(Failure::Source(SourceError::InvalidResponse(format!(
                    "sync-collection answered {other}"
                ))));
            }
        }
        let status = DavXml::multistatus(&reply.response.body)?;
        for response in &status.responses {
            if let Ok(url) = self.client.resolve(&response.href, &reply.url) {
                self.state.forget(&url);
            }
        }
        let marker = status
            .sync_token
            .map(|t| format!("sync:{t}"))
            .unwrap_or(current);
        Ok((!status.responses.is_empty(), Some(marker)))
    }
}

#[async_trait]
impl PollingCalendarSource for CalDavCalendarSource {
    /// One `PROPFIND` on the home; a `sync-collection` only for calendars whose token moved, so a token bump without a
    /// change is not reported. The first call stores the baseline and returns `None`.
    async fn check_for_changes(&self) -> Outcome<Option<CalendarChange>> {
        let calendars = self.load_calendars().await?;
        let owner = self.connection.connection_id.clone();
        let signature = Self::signature(&calendars);
        let stored = self.sync_state.token(&owner, Self::CALENDARS_SCOPE).await;
        if stored.as_deref() != Some(signature.as_str()) {
            self.sync_state
                .set_token(Some(&signature), &owner, Self::CALENDARS_SCOPE)
                .await;
            for calendar in &calendars {
                self.sync_state
                    .set_token(
                        Some(&Self::marker(calendar)),
                        &owner,
                        &Self::calendar_scope(&calendar.descriptor.id),
                    )
                    .await;
            }
            return Ok(if stored.is_none() {
                None
            } else {
                Some(CalendarChange::CalendarsChanged)
            });
        }
        // New markers are written only after every calendar was checked, so an error on one calendar cannot swallow the
        // change already found on another: the retry finds both.
        let mut changed: BTreeSet<String> = BTreeSet::new();
        let mut pending: Vec<(String, String)> = Vec::new();
        for calendar in &calendars {
            let (did_change, marker) = self.check(calendar).await?;
            if did_change {
                changed.insert(calendar.descriptor.id.clone());
            }
            if let Some(marker) = marker {
                pending.push((Self::calendar_scope(&calendar.descriptor.id), marker));
            }
        }
        for (scope, marker) in pending {
            self.sync_state
                .set_token(Some(&marker), &owner, &scope)
                .await;
        }
        Ok(
            (!changed.is_empty()).then_some(CalendarChange::EventsChanged {
                calendar_ids: Some(changed),
            }),
        )
    }
}
