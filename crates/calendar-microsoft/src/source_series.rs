//! Port of `MicrosoftCalendarSource+Series.swift`.

use async_trait::async_trait;
use calendar_core::failure::Outcome;
use calendar_core::recurrence::set::{CalendarSeries, RecurrenceSet};
use calendar_core::shared_connector_helpers::decode_response;
use calendar_core::source_types::{SeriesSource, SourceError};

use crate::api_client::{ApiResult, GraphApiClient, GraphApiError, GraphError};
use crate::dtos::GraphEventDTO;
use crate::event_mapper::GraphEventMapper;
use crate::recurrence_mapper::GraphRecurrenceMapper;
use crate::source::MicrosoftCalendarSource;

impl MicrosoftCalendarSource {
    async fn series_of(&self, id: &str, calendar_id: &str) -> ApiResult<CalendarSeries> {
        let zone = self.account_zone(false).await?;
        let url = GraphApiClient::url(&GraphApiClient::event_path(calendar_id, id), &[])?;
        let data = self.api.get(&url, &zone.read_preferences()).await?;
        let dto = decode_response::<GraphEventDTO>(&data)?;
        let (true, false, Some(recurrence)) = (
            dto.kind.as_deref() == Some("seriesMaster"),
            dto.is_cancelled == Some(true),
            dto.recurrence.as_ref(),
        ) else {
            return Err(SourceError::NotFound.into());
        };
        let Some(resolved) = GraphEventMapper::resolve(&dto, &zone.zone) else {
            return Err(SourceError::NotFound.into());
        };
        let rule = GraphRecurrenceMapper::rule(recurrence, &resolved.zone);
        let unknown: Vec<String> = if rule.is_none() {
            vec![format!(
                "X-MS-RECURRENCE:{}",
                recurrence
                    .pattern
                    .as_ref()
                    .and_then(|pattern| pattern.kind.as_deref())
                    .unwrap_or("unknown")
            )]
        } else {
            Vec::new()
        };
        Ok(CalendarSeries::new(
            id,
            calendar_id,
            resolved.start,
            resolved.zone,
            resolved.is_all_day,
            RecurrenceSet {
                rules: rule.into_iter().collect(),
                extra_dates: None,
                excluded_dates: None,
                unparsed: unknown,
            },
        ))
    }
}

#[async_trait]
impl SeriesSource for MicrosoftCalendarSource {
    /// The series master's rule and anchor time. Reads expand series, so an instance's `series_id` is the master's
    /// event id. An unknown id, and an id that is not a recurring series master, are `SourceError::NotFound`. Skipped
    /// and extra dates (`excluded_dates`, `extra_dates`) are `None`: Graph v1.0 does not list them on the master.
    async fn series(&self, id: &str, calendar_id: &str) -> Outcome<CalendarSeries> {
        match self.series_of(id, calendar_id).await {
            Ok(series) => Ok(series),
            Err(GraphError::Api(GraphApiError::NotFound | GraphApiError::Gone)) => {
                Err(SourceError::NotFound.into())
            }
            Err(error) => Err(error.into_failure()),
        }
    }
}
