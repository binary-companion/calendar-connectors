//! Port of `CalDAVCalendarSource+Series.swift`.

use async_trait::async_trait;
use calendar_core::failure::{Failure, Outcome};
use calendar_core::recurrence::set::CalendarSeries;
use calendar_core::source_types::{SeriesSource, SourceError};
use calendar_ical::{EventReader, EventResource};

use crate::source::CalDavCalendarSource;

fn source_error(error: SourceError) -> Failure {
    Failure::Source(error)
}

#[async_trait]
impl SeriesSource for CalDavCalendarSource {
    /// `id` is the resource name (`SeriesInfo::Occurrence`'s `series_id`). A resource that is missing or does not
    /// recur is `SourceError::NotFound`.
    async fn series(&self, id: &str, calendar_id: &str) -> Outcome<CalendarSeries> {
        let reply = self
            .client
            .send("GET", &self.resource_url(calendar_id, id))
            .await?;
        match reply.response.status {
            200 => {}
            403 | 404 | 410 => return Err(source_error(SourceError::NotFound)),
            other => {
                return Err(source_error(SourceError::InvalidResponse(format!(
                    "GET answered {other}"
                ))));
            }
        }
        let Ok(resource) = EventResource::from_data(&reply.response.body) else {
            return Err(source_error(SourceError::InvalidResponse(
                "the event could not be read".to_string(),
            )));
        };
        let zone = self.calendar_zone(calendar_id).await?;
        let context = self.context(
            calendar_id,
            &zone,
            id,
            reply.response.header("ETag").map(str::to_string),
        );
        EventReader::series_of(&resource, &context)
            .ok_or_else(|| source_error(SourceError::NotFound))
    }
}
