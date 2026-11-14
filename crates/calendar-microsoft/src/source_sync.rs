//! Port of `MicrosoftCalendarSource+Sync.swift`: change detection by Graph's delta links.

use async_trait::async_trait;
use calendar_core::failure::Outcome;
use calendar_core::shared_connector_helpers::detect_calendar_changes;
use calendar_core::source_types::{CalendarChange, PollingCalendarSource, SourceError};
use calendar_core::time::{add_seconds, seconds_between};
use calendar_core::url_string::UrlString;
use jiff::Timestamp;

use crate::api_client::{ApiResult, GraphApiClient, GraphApiError, GraphError, query_item};
use crate::dtos::{GraphIDDTO, GraphListPage};
use crate::graph_time::GraphTime;
use crate::source::MicrosoftCalendarSource;

/// A calendar's delta position: the link to poll next, and when its window was baselined. Kept in the sync state
/// store as `"<baseline seconds>|<link>"` because the store holds one string. The link is opaque (the persisted-format
/// tests pin that the stored ones still parse).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeltaState {
    pub baseline_date: Timestamp,
    pub link: UrlString,
}

impl DeltaState {
    /// Swift's failable `init(_ stored:)`: the text before the first `|` is the baseline seconds, the rest is the
    /// link. A baseline that is not a finite number is not a position (Swift would trap when it stored it again).
    pub(crate) fn parse(stored: &str) -> Option<DeltaState> {
        let (seconds, link) = stored.split_once('|')?;
        let seconds = seconds.parse::<f64>().ok().filter(|s| s.is_finite())?;
        Some(DeltaState {
            baseline_date: add_seconds(Timestamp::UNIX_EPOCH, seconds),
            link: UrlString::parse(link)?,
        })
    }

    /// `"\(Int(baselineDate.timeIntervalSince1970))|\(link.absoluteString)"`.
    pub(crate) fn stored(&self) -> String {
        let seconds = seconds_between(Timestamp::UNIX_EPOCH, self.baseline_date);
        format!("{}|{}", seconds as i64, self.link.as_str())
    }
}

impl MicrosoftCalendarSource {
    pub(crate) const CALENDAR_SET_SCOPE: &'static str = "_calendars";
    /// The delta window: this far back and this far ahead of the day it was baselined.
    const WINDOW_BEFORE: f64 = 30.0 * 86_400.0;
    const WINDOW_AFTER: f64 = 365.0 * 86_400.0;
    /// A window is renewed once it is this old, so it always keeps 16 days back and 350 days ahead.
    const WINDOW_RENEWAL: f64 = 14.0 * 86_400.0;

    fn page_preference() -> Vec<String> {
        vec!["odata.maxpagesize=200".to_string()]
    }

    /// The outcome of one calendar's check: a calendar that was removed or is no longer readable counts as unchanged.
    fn skip_missing(result: ApiResult<bool>) -> Outcome<bool> {
        match result {
            Ok(changed) => Ok(changed),
            Err(GraphError::Api(GraphApiError::NotFound | GraphApiError::Forbidden)) => Ok(false),
            Err(error) => Err(error.into_failure()),
        }
    }

    async fn check_calendar(&self, id: String) -> ApiResult<bool> {
        let connection_id = &self.connection.connection_id;
        if let Some(stored) = self.sync_state.token(connection_id, &id).await
            && let Some(delta) = DeltaState::parse(&stored)
        {
            return self.poll(&id, &delta).await;
        }
        self.baseline(&id).await?;
        Ok(false)
    }

    /// Lists the window only to obtain the delta link.
    async fn baseline(&self, calendar_id: &str) -> ApiResult<()> {
        let taken = (self.now)();
        let url = GraphApiClient::url(
            &GraphApiClient::calendar_path(calendar_id, "/calendarView/delta"),
            &[
                query_item(
                    "startDateTime",
                    &GraphTime::instant_text(add_seconds(taken, -Self::WINDOW_BEFORE)),
                ),
                query_item(
                    "endDateTime",
                    &GraphTime::instant_text(add_seconds(taken, Self::WINDOW_AFTER)),
                ),
            ],
        )?;
        let mut link: Option<String> = None;
        self.api
            .pages::<GraphListPage<GraphIDDTO>, _>(&url, &Self::page_preference(), |page| {
                if let Some(delta) = &page.delta_link {
                    link = Some(delta.clone());
                }
                Ok(())
            })
            .await?;
        let Some(delta_url) = link.as_deref().and_then(UrlString::parse) else {
            return Err(SourceError::InvalidResponse("no deltaLink".to_string()).into());
        };
        let state = DeltaState {
            baseline_date: taken,
            link: delta_url,
        };
        self.sync_state
            .set_token(
                Some(&state.stored()),
                &self.connection.connection_id,
                calendar_id,
            )
            .await;
        Ok(())
    }

    /// True when anything changed since `delta`. Always walks to the last page, because only it carries the new link.
    /// A link Graph no longer accepts, and a window that has aged out, take a new baseline; both report a change,
    /// because an edit between the last poll and the new baseline would otherwise never be reported.
    async fn poll(&self, calendar_id: &str, delta: &DeltaState) -> ApiResult<bool> {
        let mut any_items = false;
        let mut new_link: Option<String> = None;
        let walked = self
            .api
            .pages::<GraphListPage<GraphIDDTO>, _>(&delta.link, &Self::page_preference(), |page| {
                // Items are only counted; an item that fails to parse is still a change (removals arrive as
                // `@removed`).
                if page.value.as_ref().is_some_and(|items| !items.is_empty()) {
                    any_items = true;
                }
                if let Some(link) = &page.delta_link {
                    new_link = Some(link.clone());
                }
                Ok(())
            })
            .await;
        match walked {
            Ok(()) => {}
            Err(GraphError::Api(GraphApiError::Gone)) => {
                self.baseline(calendar_id).await?;
                return Ok(true);
            }
            Err(error) => return Err(error),
        }
        let Some(url) = new_link.as_deref().and_then(UrlString::parse) else {
            return Err(SourceError::InvalidResponse("no deltaLink".to_string()).into());
        };
        let state = DeltaState {
            baseline_date: delta.baseline_date,
            link: url,
        };
        self.sync_state
            .set_token(
                Some(&state.stored()),
                &self.connection.connection_id,
                calendar_id,
            )
            .await;
        if seconds_between(delta.baseline_date, (self.now)()) > Self::WINDOW_RENEWAL {
            self.baseline(calendar_id).await?;
            return Ok(true);
        }
        Ok(any_items)
    }
}

#[async_trait]
impl PollingCalendarSource for MicrosoftCalendarSource {
    /// One incremental check. The first call for a calendar takes its baseline and reports nothing.
    async fn check_for_changes(&self) -> Outcome<Option<CalendarChange>> {
        let calendar_ids: Vec<String> = self
            .load_calendars(false)
            .await?
            .into_iter()
            .map(|calendar| calendar.id)
            .collect();
        let connection_id = self.connection.connection_id.clone();
        detect_calendar_changes(
            &calendar_ids,
            Self::CALENDAR_SET_SCOPE,
            &connection_id,
            self.sync_state.as_ref(),
            |id| async move { Self::skip_missing(self.check_calendar(id).await) },
        )
        .await
    }
}
