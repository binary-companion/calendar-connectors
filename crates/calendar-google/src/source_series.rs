//! Port of `GoogleCalendarSource+Series.swift`.

use async_trait::async_trait;
use calendar_core::failure::Outcome;
use calendar_core::recurrence::set::{CalendarSeries, RecurrenceSet};
use calendar_core::shared_connector_helpers::decode_response;
use calendar_core::source_types::{SeriesSource, SourceError};
use calendar_core::time::Zone;

use crate::api_client::{ApiResult, GoogleApiClient, GoogleApiError, GoogleError, query_item};
use crate::dtos::GoogleEventDTO;
use crate::event_mapper::GoogleEventMapper;
use crate::source::GoogleCalendarSource;

impl GoogleCalendarSource {
    async fn series_of(&self, id: &str, calendar_id: &str) -> ApiResult<CalendarSeries> {
        let data = self
            .api
            .get(
                &GoogleApiClient::event_path(calendar_id, id),
                &[query_item("fields", "id,status,recurrence,start,end")],
            )
            .await?;
        let dto = decode_response::<GoogleEventDTO>(&data)?;
        let lines = match &dto.recurrence {
            Some(lines) if dto.status.as_deref() != Some("cancelled") && !lines.is_empty() => lines,
            _ => return Err(SourceError::NotFound.into()),
        };
        let calendar_zone = self.zone_of_calendar(calendar_id).await?;
        let Some(start) = GoogleEventMapper::resolve(dto.start.as_ref(), &calendar_zone) else {
            return Err(
                SourceError::InvalidResponse("google: unreadable series".to_string()).into(),
            );
        };
        let zone = start.zone.clone().unwrap_or(calendar_zone);
        Ok(CalendarSeries::new(
            id,
            calendar_id,
            start.date,
            zone.clone(),
            start.is_all_day,
            RecurrenceSet::from_icalendar_lines(lines, &zone, start.is_all_day),
        ))
    }

    async fn zone_of_calendar(&self, calendar_id: &str) -> Outcome<Zone> {
        use calendar_core::source_types::CalendarSource;
        let calendars = match self.calendar_list.list() {
            Some(stored) => stored,
            None => self.calendars().await?,
        };
        Ok(calendars
            .iter()
            .find(|calendar| calendar.id == calendar_id)
            .and_then(|calendar| calendar.time_zone.clone())
            .unwrap_or_else(Zone::utc))
    }
}

#[async_trait]
impl SeriesSource for GoogleCalendarSource {
    /// The series master's rule lines (`RRULE`, `EXDATE`, `RDATE`, ...) and anchor time. Reads expand series, so an
    /// instance's `series_id` is the master's event id. An id that is not a recurring series (no `recurrence`, or a
    /// cancelled master) and an unknown id are `SourceError::NotFound`.
    async fn series(&self, id: &str, calendar_id: &str) -> Outcome<CalendarSeries> {
        match self.series_of(id, calendar_id).await {
            Ok(series) => Ok(series),
            Err(GoogleError::Api(GoogleApiError::NotFound | GoogleApiError::Gone)) => {
                Err(SourceError::NotFound.into())
            }
            Err(error) => Err(error.into_failure()),
        }
    }
}
