//! Port of `GoogleEventMapper.swift`: Google's calendar list entries and event resources as the library's model.

use std::collections::BTreeSet;

use calendar_core::all_day::{AllDay, CalendarDate};
use calendar_core::conference_detector::ConferenceDetector;
use calendar_core::model::{
    Attendee, AttendeeRole, Availability, CalendarDescriptor, CalendarEvent, CalendarKind,
    CalendarPermissions, CalendarProvider, CalendarService, ConferenceInfo, ConferenceOrigin,
    ConferenceProvider, EventKind, EventStatus, Participation, Reminder, ReminderType,
    ResponseStatus, SeriesInfo, UidScope, Visibility,
};
use calendar_core::time::Zone;
use calendar_core::url_string::UrlString;
use jiff::Timestamp;

use crate::dtos::{
    GoogleAttendeeDTO, GoogleCalendarListEntryDTO, GoogleConferenceSolution, GoogleEventDTO,
    GoogleRemindersDTO, GoogleTimeDTO,
};

pub(crate) enum GoogleEventMapper {}

/// Swift's nested `Resolved`: a start or end time, its zone when it names one, and whether it was a date.
#[derive(Clone, Debug)]
pub(crate) struct Resolved {
    pub date: Timestamp,
    pub zone: Option<Zone>,
    pub is_all_day: bool,
}

impl GoogleEventMapper {
    pub(crate) fn descriptor(
        dto: &GoogleCalendarListEntryDTO,
        account_name: Option<&str>,
    ) -> Option<CalendarDescriptor> {
        if dto.deleted == Some(true) || dto.hidden == Some(true) {
            return None;
        }
        let calendar_kind = Self::calendar_kind(&dto.id);
        let mut descriptor = CalendarDescriptor::new(
            dto.id.clone(),
            dto.summary_override
                .clone()
                .or_else(|| dto.summary.clone())
                .unwrap_or_else(|| dto.id.clone()),
            CalendarService::google(),
        )
        .with_color_hex(dto.background_color.as_deref());
        descriptor.permissions = Self::permissions(dto.access_role.as_deref());
        descriptor.is_default = Some(dto.primary.unwrap_or(false));
        descriptor.time_zone = dto.time_zone.as_deref().and_then(Zone::named);
        descriptor.account_name = account_name.map(str::to_string);
        descriptor.kind = calendar_kind;
        descriptor.default_reminders = Some(
            dto.default_reminders
                .iter()
                .flatten()
                .filter_map(|reminder| {
                    Self::reminder(reminder.method.as_deref(), reminder.minutes, true)
                })
                .collect(),
        );
        descriptor.provider = Some(if calendar_kind == CalendarKind::Standard {
            CalendarProvider::google()
        } else {
            CalendarProvider::subscription()
        });
        descriptor.supported_availabilities = Some(Self::supported_availabilities());
        Some(descriptor)
    }

    /// Google's transparency has two values.
    pub(crate) fn supported_availabilities() -> BTreeSet<Availability> {
        BTreeSet::from([Availability::Busy, Availability::Free])
    }

    /// owner: everything; writer: edit but not share; reader: read only; freeBusyReader: no details.
    pub(crate) fn permissions(role: Option<&str>) -> CalendarPermissions {
        let (details, edit, share, private) = match role {
            Some("owner") => (true, true, true, true),
            Some("writer") => (true, true, false, true),
            Some("freeBusyReader") => (false, false, false, false),
            _ => (true, false, false, false),
        };
        CalendarPermissions {
            can_view_details: details,
            can_edit: edit,
            can_share: Some(share),
            can_view_private: Some(private),
        }
    }

    /// Google's built-in feeds have well-known ids: contacts' birthdays and the regional holiday calendars.
    fn calendar_kind(id: &str) -> CalendarKind {
        if id.ends_with("#contacts@group.v.calendar.google.com") {
            return CalendarKind::Birthdays;
        }
        if id.ends_with("#holiday@group.v.calendar.google.com") {
            return CalendarKind::Subscribed;
        }
        CalendarKind::Standard
    }

    /// Returns `None` for cancelled events and for events whose times cannot be understood.
    pub(crate) fn map(
        dto: &GoogleEventDTO,
        calendar: &CalendarDescriptor,
        source_id: Option<&str>,
    ) -> Option<CalendarEvent> {
        if dto.status.as_deref() == Some("cancelled") {
            return None;
        }
        let calendar_zone = calendar.time_zone.clone().unwrap_or_else(Zone::utc);
        let start = Self::resolve(dto.start.as_ref(), &calendar_zone)?;
        let end = Self::resolve(dto.end.as_ref(), &calendar_zone)?;

        let attendees: Vec<Attendee> = dto.attendees.iter().flatten().map(Self::attendee).collect();
        let organizer = dto.organizer.as_ref().map(|organizer| {
            Attendee::new(
                organizer.display_name.clone(),
                organizer.email.as_deref(),
                AttendeeRole::Required,
                ResponseStatus::Accepted,
                organizer.is_self.unwrap_or(false),
                true,
            )
        });
        let mut event = CalendarEvent::new(
            dto.id.clone(),
            calendar.id.clone(),
            dto.summary
                .clone()
                .unwrap_or_else(|| "(No title)".to_string()),
            start.date,
            end.date,
        );
        event.uid = dto.ical_uid.clone();
        event.uid_scope = Some(UidScope::Global);
        event.notes = dto.description.clone();
        event.location = dto.location.clone();
        event.time_zone = start.zone.clone().unwrap_or(calendar_zone.clone());
        event.is_all_day = start.is_all_day;
        event.status = if dto.status.as_deref() == Some("tentative") {
            EventStatus::Tentative
        } else {
            EventStatus::Confirmed
        };
        event.availability = Some(if dto.transparency.as_deref() == Some("transparent") {
            Availability::Free
        } else {
            Availability::Busy
        });
        event.visibility = Some(Self::visibility(dto.visibility.as_deref()));
        event.kind = Some(Self::event_kind(dto.event_type.as_deref()));
        event.series = Some(Self::series(dto, &calendar_zone));
        event.participation = Some(Participation::resolving(&attendees, organizer.as_ref()));
        event.attendees = attendees;
        event.organizer = organizer;
        event.conferences = Self::conferences(dto);
        event.reminders = Some(Self::reminders(dto.reminders.as_ref(), calendar));
        event.url = dto.html_link.as_deref().and_then(UrlString::parse);
        event.version = dto.etag.clone();
        event.last_modified = dto.updated.as_deref().and_then(Self::parse_instant);
        event.created = dto.created.as_deref().and_then(Self::parse_instant);
        event.source_id = source_id.map(str::to_string);
        Some(event)
    }

    pub(crate) fn resolve(time: Option<&GoogleTimeDTO>, calendar_zone: &Zone) -> Option<Resolved> {
        let time = time?;
        if let Some(day) = &time.date {
            let parts: Vec<i64> = day
                .split('-')
                .filter(|piece| !piece.is_empty())
                .filter_map(|piece| piece.parse::<i64>().ok())
                .collect();
            if parts.len() != 3 {
                return None;
            }
            let date = AllDay::start_of_day(
                &CalendarDate::new(parts[0], parts[1], parts[2]),
                calendar_zone,
            )?;
            return Some(Resolved {
                date,
                zone: Some(calendar_zone.clone()),
                is_all_day: true,
            });
        }
        let date = Self::parse_instant(time.date_time.as_deref()?)?;
        Some(Resolved {
            date,
            zone: time.time_zone.as_deref().and_then(Zone::named),
            is_all_day: false,
        })
    }

    /// An RFC 3339 instant: `YYYY-MM-DDTHH:MM:SS`, an optional fraction (kept to milliseconds, as
    /// `ISO8601DateFormatter` with `.withFractionalSeconds` does), then `Z` or an offset (`+hh:mm`, `+hhmm`, `+hh`).
    /// Swift tries the whole-seconds form first and then the fractional one; both are read here. Foundation is more
    /// lenient than this (it accepts surrounding spaces, text after the zone, `24:00:00` and a 30th of February);
    /// Google never sends those.
    pub(crate) fn parse_instant(text: &str) -> Option<Timestamp> {
        let bytes = text.as_bytes();
        let digits = |range: std::ops::Range<usize>| -> Option<i64> {
            let slice = bytes.get(range)?;
            if slice.iter().all(u8::is_ascii_digit) {
                std::str::from_utf8(slice).ok()?.parse().ok()
            } else {
                None
            }
        };
        let separator = |at: usize, wanted: u8| bytes.get(at) == Some(&wanted);
        if !(separator(4, b'-') && separator(7, b'-') && separator(10, b'T')) {
            return None;
        }
        if !(separator(13, b':') && separator(16, b':')) {
            return None;
        }
        let (year, month, day) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
        let (hour, minute, second) = (digits(11..13)?, digits(14..16)?, digits(17..19)?);
        if !(1..=9999).contains(&year)
            || !(1..=12).contains(&month)
            || day < 1
            || hour > 23
            || minute > 59
            || second > 59
        {
            return None;
        }
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let days_in_month = match month {
            2 if leap => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        };
        if day > days_in_month {
            return None;
        }
        let mut at = 19;
        let mut millis: i64 = 0;
        if separator(at, b'.') {
            let start = at + 1;
            let mut end = start;
            while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                end += 1;
            }
            if end == start {
                return None;
            }
            for (index, byte) in bytes[start..end].iter().take(3).enumerate() {
                millis += i64::from(byte - b'0') * [100, 10, 1][index];
            }
            at = end;
        }
        let offset: i64 = match bytes.get(at) {
            Some(b'Z' | b'z') if at + 1 == bytes.len() => 0,
            Some(sign @ (b'+' | b'-')) => {
                let hours = digits(at + 1..at + 3)?;
                let rest = &bytes[at + 3..];
                let minutes = match rest.len() {
                    0 => 0,
                    2 => digits(at + 3..at + 5)?,
                    3 if rest[0] == b':' => digits(at + 4..at + 6)?,
                    _ => return None,
                };
                if hours > 23 || minutes > 59 {
                    return None;
                }
                let seconds = hours * 3600 + minutes * 60;
                if *sign == b'-' { -seconds } else { seconds }
            }
            _ => return None,
        };
        let civil = jiff::civil::DateTime::new(
            year as i16,
            month as i8,
            day as i8,
            hour as i8,
            minute as i8,
            second as i8,
            0,
        )
        .ok()?;
        let utc = civil.to_zoned(jiff::tz::TimeZone::UTC).ok()?.timestamp();
        let nanos = utc.as_nanosecond() - i128::from(offset) * 1_000_000_000
            + i128::from(millis) * 1_000_000;
        Timestamp::from_nanosecond(nanos).ok()
    }

    fn attendee(dto: &GoogleAttendeeDTO) -> Attendee {
        let role = if dto.resource == Some(true) {
            AttendeeRole::Resource
        } else if dto.optional == Some(true) {
            AttendeeRole::Optional
        } else {
            AttendeeRole::Required
        };
        let response = match dto.response_status.as_deref() {
            Some("accepted") => ResponseStatus::Accepted,
            Some("tentative") => ResponseStatus::Tentative,
            Some("declined") => ResponseStatus::Declined,
            _ => ResponseStatus::NeedsAction,
        };
        Attendee::new(
            dto.display_name.clone(),
            dto.email.as_deref(),
            role,
            response,
            dto.is_self.unwrap_or(false),
            dto.organizer.unwrap_or(false),
        )
    }

    fn visibility(raw: Option<&str>) -> Visibility {
        match raw {
            Some("public") => Visibility::PublicEvent,
            Some("private") => Visibility::PrivateEvent,
            Some("confidential") => Visibility::Confidential,
            _ => Visibility::Default,
        }
    }

    fn event_kind(raw: Option<&str>) -> EventKind {
        match raw {
            None | Some("default") => EventKind::Standard,
            Some("focusTime") => EventKind::FocusTime,
            Some("outOfOffice") => EventKind::OutOfOffice,
            Some("workingLocation") => EventKind::WorkingLocation,
            Some("birthday") => EventKind::Birthday,
            Some(_) => EventKind::Other,
        }
    }

    /// Every video entry point, then `hangoutLink`, then links found in the description (an event imported from an
    /// invite carries its Teams or Zoom link only there). `url` is not scanned: Google's `htmlLink` is never a join
    /// link.
    fn conferences(dto: &GoogleEventDTO) -> Vec<ConferenceInfo> {
        let mut structured: Vec<ConferenceInfo> = Vec::new();
        let solution = dto
            .conference_data
            .as_ref()
            .and_then(|data| data.conference_solution.as_ref());
        for entry in dto
            .conference_data
            .iter()
            .flat_map(|data| data.entry_points.iter().flatten())
            .filter(|entry| entry.entry_point_type.as_deref() == Some("video"))
        {
            let Some(url) = entry.uri.as_deref().and_then(UrlString::parse) else {
                continue;
            };
            let provider = Self::provider(&url, solution);
            structured.push(ConferenceInfo::new(
                url,
                provider,
                ConferenceOrigin::Structured,
            ));
        }
        if let Some(url) = dto.hangout_link.as_deref().and_then(UrlString::parse) {
            structured.push(ConferenceInfo::new(
                url,
                ConferenceProvider::Meet,
                ConferenceOrigin::Structured,
            ));
        }
        ConferenceDetector::conferences(
            &structured,
            dto.location.as_deref(),
            None,
            dto.description.as_deref(),
        )
    }

    fn provider(
        url: &UrlString,
        solution: Option<&GoogleConferenceSolution>,
    ) -> ConferenceProvider {
        if solution
            .and_then(|s| s.key.as_ref())
            .and_then(|key| key.kind.as_deref())
            == Some("hangoutsMeet")
        {
            return ConferenceProvider::Meet;
        }
        if let Some(known) = ConferenceDetector::provider_of(url) {
            return known;
        }
        let name = solution
            .and_then(|s| s.name.as_deref())
            .unwrap_or("")
            .to_lowercase();
        if name.contains("zoom") {
            return ConferenceProvider::Zoom;
        }
        if name.contains("teams") {
            return ConferenceProvider::Teams;
        }
        ConferenceProvider::Other
    }

    /// `useDefault` resolves to the calendar's own default reminders, so an event never reads as having none because
    /// it uses the defaults. An event without a `reminders` object has none.
    fn reminders(dto: Option<&GoogleRemindersDTO>, calendar: &CalendarDescriptor) -> Vec<Reminder> {
        let Some(dto) = dto else {
            return Vec::new();
        };
        if dto.use_default == Some(true) {
            return calendar
                .default_reminders
                .iter()
                .flatten()
                .map(|reminder| {
                    let mut copy = reminder.clone();
                    copy.is_calendar_default = Some(true);
                    copy
                })
                .collect();
        }
        dto.overrides
            .iter()
            .flatten()
            .filter_map(|r| Self::reminder(r.method.as_deref(), r.minutes, false))
            .collect()
    }

    /// `popup` is an on-screen alert, `email` an email to the account owner (no address); another method is kept as
    /// `Other`. Google counts minutes before the start (all-day: before midnight in the calendar's zone).
    fn reminder(
        method: Option<&str>,
        minutes: Option<i64>,
        is_calendar_default: bool,
    ) -> Option<Reminder> {
        let minutes = minutes?;
        let kind = match method {
            Some("popup") | None => ReminderType::Display,
            Some("email") => ReminderType::Email { address: None },
            Some(other) => ReminderType::Other(other.to_string()),
        };
        Some(Reminder::before(minutes, kind, Some(is_calendar_default)))
    }

    fn series(dto: &GoogleEventDTO, calendar_zone: &Zone) -> SeriesInfo {
        let Some(id) = &dto.recurring_event_id else {
            return SeriesInfo::NotRecurring;
        };
        SeriesInfo::Occurrence {
            series_id: id.clone(),
            original_start: Self::resolve(dto.original_start_time.as_ref(), calendar_zone)
                .map(|resolved| resolved.date),
        }
    }
}

impl GoogleEventMapper {
    /// `ISO8601DateFormatter` with `.withInternetDateTime`: UTC, whole seconds (the fraction is dropped, rounding
    /// down), `Z`. Used for the request windows and by the write mapper.
    pub(crate) fn instant_text(date: Timestamp) -> String {
        let seconds = date.as_nanosecond().div_euclid(1_000_000_000);
        let days = seconds.div_euclid(86_400);
        let of_day = seconds.rem_euclid(86_400);
        // Civil date from a day count (Howard Hinnant's algorithm), proleptic Gregorian.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i128::from(month <= 2);
        format!(
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
            of_day / 3600,
            of_day % 3600 / 60,
            of_day % 60
        )
    }
}
