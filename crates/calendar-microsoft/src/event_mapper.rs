//! Port of `GraphEventMapper.swift`: Graph's calendars and event resources as the library's model.

use std::collections::BTreeSet;

use calendar_core::all_day::AllDay;
use calendar_core::conference_detector::ConferenceDetector;
use calendar_core::model::{
    Attendee, AttendeeRole, Availability, CalendarDescriptor, CalendarEvent, CalendarKind,
    CalendarPermissions, CalendarProvider, CalendarService, ConferenceInfo, ConferenceOrigin,
    ConferenceProvider, EventKind, EventStatus, Participation, Reminder, ReminderType,
    ResponseStatus, SeriesInfo, UidScope, Visibility,
};
use calendar_core::time::Zone;
use calendar_core::url_string::UrlString;
use calendar_core::windows_time_zones::WindowsTimeZones;
use jiff::Timestamp;

use crate::dtos::{GraphAttendeeDTO, GraphBodyDTO, GraphCalendarDTO, GraphEventDTO};
use crate::graph_time::GraphTime;

pub(crate) enum GraphEventMapper {}

/// Swift's nested `Resolved`.
#[derive(Clone, Debug)]
pub(crate) struct Resolved {
    pub start: Timestamp,
    pub end: Timestamp,
    /// The zone the event is shown in.
    pub zone: Zone,
    pub is_all_day: bool,
}

impl GraphEventMapper {
    /// The event properties every read asks for (`$select`), so a response carries what the mapper needs and no more.
    pub(crate) const EVENT_SELECT: &'static str = "id,iCalUId,subject,body,location,start,end,isAllDay,isCancelled,showAs,sensitivity,type,seriesMasterId,originalStart,originalStartTimeZone,attendees,organizer,isOrganizer,responseStatus,isReminderOn,reminderMinutesBeforeStart,isOnlineMeeting,onlineMeeting,changeKey,lastModifiedDateTime,createdDateTime,webLink,recurrence";

    /// Graph's `showAs` has five values; two of them read as the library's four.
    pub(crate) fn supported_availabilities() -> BTreeSet<Availability> {
        BTreeSet::from([
            Availability::Busy,
            Availability::Free,
            Availability::Tentative,
            Availability::Unavailable,
        ])
    }

    pub(crate) fn descriptor(
        dto: &GraphCalendarDTO,
        account_name: Option<&str>,
        zone: &Zone,
    ) -> CalendarDescriptor {
        let mut descriptor = CalendarDescriptor::new(
            dto.id.clone(),
            dto.name.clone().unwrap_or_else(|| dto.id.clone()),
            CalendarService::microsoft(),
        )
        .with_color_hex(dto.hex_color.as_deref());
        descriptor.permissions = CalendarPermissions {
            can_view_details: true,
            can_edit: dto.can_edit.unwrap_or(false),
            can_share: Some(dto.can_share.unwrap_or(false)),
            can_view_private: Some(dto.can_view_private_items.unwrap_or(false)),
        };
        descriptor.is_default = Some(dto.is_default_calendar.unwrap_or(false));
        descriptor.time_zone = Some(zone.clone());
        descriptor.account_name = account_name.map(str::to_string);
        descriptor.kind = CalendarKind::Standard;
        descriptor.provider = Some(CalendarProvider::microsoft());
        descriptor.supported_availabilities = Some(Self::supported_availabilities());
        descriptor
    }

    /// Returns `None` for cancelled events and for events whose times cannot be understood. `account_email`
    /// (lowercased) marks the attendee that is the account owner.
    pub(crate) fn map(
        dto: &GraphEventDTO,
        calendar: &CalendarDescriptor,
        account_email: Option<&str>,
        source_id: Option<&str>,
    ) -> Option<CalendarEvent> {
        if dto.is_cancelled == Some(true) {
            return None;
        }
        let times = Self::resolve(dto, &calendar.time_zone.clone().unwrap_or_else(Zone::utc))?;

        let organizer_email = dto
            .organizer
            .as_ref()
            .and_then(|organizer| organizer.email_address.as_ref())
            .and_then(|email| email.address.as_deref())
            .map(str::to_lowercase);
        let attendees: Vec<Attendee> = dto
            .attendees
            .iter()
            .flatten()
            .map(|attendee| Self::attendee(attendee, account_email, organizer_email.as_deref()))
            .collect();
        let organizer = dto
            .organizer
            .as_ref()
            .and_then(|organizer| organizer.email_address.as_ref())
            .map(|email| {
                Attendee::new(
                    email.name.clone(),
                    email.address.as_deref(),
                    AttendeeRole::Required,
                    ResponseStatus::Accepted,
                    dto.is_organizer == Some(true)
                        || (account_email.is_some()
                            && email.address.as_deref().map(str::to_lowercase).as_deref()
                                == account_email),
                    true,
                )
            });
        let notes = Self::plain_text(dto.body.as_ref());
        let mut event = CalendarEvent::new(
            dto.id.clone(),
            calendar.id.clone(),
            dto.subject
                .clone()
                .unwrap_or_else(|| "(No title)".to_string()),
            times.start,
            times.end,
        );
        event.uid = dto.ical_uid.clone();
        event.uid_scope = Some(UidScope::Global);
        event.location = dto
            .location
            .as_ref()
            .and_then(|location| location.display_name.clone())
            .filter(|name| !name.is_empty());
        event.time_zone = times.zone.clone();
        event.is_all_day = times.is_all_day;
        event.status = EventStatus::Confirmed;
        event.availability = Some(Self::availability(dto.show_as.as_deref()));
        event.visibility = Some(Self::visibility(dto.sensitivity.as_deref()));
        event.kind = Some(EventKind::Standard);
        event.series = Some(Self::series(dto, times.start));
        event.conferences = Self::conferences(dto, notes.as_deref());
        event.notes = notes;
        event.reminders = Some(Self::reminders(dto));
        event.url = dto.web_link.as_deref().and_then(UrlString::parse);
        event.version = dto.change_key.clone();
        event.last_modified = dto
            .last_modified_date_time
            .as_deref()
            .and_then(GraphTime::parse_instant);
        event.created = dto
            .created_date_time
            .as_deref()
            .and_then(GraphTime::parse_instant);
        event.participation = Some(Participation::resolving(&attendees, organizer.as_ref()));
        event.attendees = attendees;
        event.organizer = organizer;
        event.source_id = source_id.map(str::to_string);
        Some(event)
    }

    /// Reads the times in the zone the response names (the account zone, because reads send
    /// `Prefer: outlook.timezone`). A timed event is shown in the zone it was scheduled in (`originalStartTimeZone`)
    /// when Graph gives one.
    pub(crate) fn resolve(dto: &GraphEventDTO, fallback_zone: &Zone) -> Option<Resolved> {
        let start_text = dto.start.as_ref()?.date_time.as_deref()?;
        let end_text = dto.end.as_ref()?.date_time.as_deref()?;
        let start_zone = dto
            .start
            .as_ref()
            .and_then(|start| start.time_zone.as_deref())
            .and_then(WindowsTimeZones::time_zone_for)
            .unwrap_or_else(|| fallback_zone.clone());
        let end_zone = dto
            .end
            .as_ref()
            .and_then(|end| end.time_zone.as_deref())
            .and_then(WindowsTimeZones::time_zone_for)
            .unwrap_or_else(|| start_zone.clone());
        if dto.is_all_day == Some(true) {
            let first = GraphTime::date(start_text)?;
            let mut last = GraphTime::date(end_text)?;
            if last <= first {
                last = first.adding_days(1);
            }
            let (start, end) = AllDay::canonical(&first, &last, &start_zone)?;
            return Some(Resolved {
                start,
                end,
                zone: start_zone,
                is_all_day: true,
            });
        }
        let start = GraphTime::parse(start_text, &start_zone)?;
        let end = GraphTime::parse(end_text, &end_zone)?;
        let shown = dto
            .original_start_time_zone
            .as_deref()
            .and_then(WindowsTimeZones::time_zone_for)
            .unwrap_or(start_zone);
        Some(Resolved {
            start,
            end: start.max(end),
            zone: shown,
            is_all_day: false,
        })
    }

    fn availability(raw: Option<&str>) -> Availability {
        match raw {
            Some("free" | "workingElsewhere") => Availability::Free,
            Some("tentative") => Availability::Tentative,
            Some("oof") => Availability::Unavailable,
            _ => Availability::Busy,
        }
    }

    fn visibility(raw: Option<&str>) -> Visibility {
        match raw {
            Some("personal" | "private") => Visibility::PrivateEvent,
            Some("confidential") => Visibility::Confidential,
            _ => Visibility::Default,
        }
    }

    fn series(dto: &GraphEventDTO, start: Timestamp) -> SeriesInfo {
        match dto.kind.as_deref() {
            Some("occurrence" | "exception") => {
                let Some(master) = &dto.series_master_id else {
                    return SeriesInfo::NotRecurring;
                };
                SeriesInfo::Occurrence {
                    series_id: master.clone(),
                    original_start: dto
                        .original_start
                        .as_deref()
                        .and_then(GraphTime::parse_instant),
                }
            }
            // A master is its own series, starting at its own slot, so a single-instance write on it can be refused.
            Some("seriesMaster") => SeriesInfo::Occurrence {
                series_id: dto.id.clone(),
                original_start: Some(start),
            },
            _ => SeriesInfo::NotRecurring,
        }
    }

    fn attendee(
        dto: &GraphAttendeeDTO,
        account_email: Option<&str>,
        organizer_email: Option<&str>,
    ) -> Attendee {
        let email = dto
            .email_address
            .as_ref()
            .and_then(|email| email.address.as_deref())
            .map(str::to_lowercase);
        let role = match dto.kind.as_deref() {
            Some("optional") => AttendeeRole::Optional,
            Some("resource") => AttendeeRole::Resource,
            _ => AttendeeRole::Required,
        };
        let response = match dto
            .status
            .as_ref()
            .and_then(|status| status.response.as_deref())
        {
            Some("accepted" | "organizer") => ResponseStatus::Accepted,
            Some("tentativelyAccepted") => ResponseStatus::Tentative,
            Some("declined") => ResponseStatus::Declined,
            _ => ResponseStatus::NeedsAction,
        };
        Attendee::new(
            dto.email_address
                .as_ref()
                .and_then(|email| email.name.clone()),
            email.as_deref(),
            role,
            response,
            email.is_some() && email.as_deref() == account_email,
            email.is_some() && email.as_deref() == organizer_email,
        )
    }

    /// Graph's structured Teams link, then links found in the location and notes.
    fn conferences(dto: &GraphEventDTO, notes: Option<&str>) -> Vec<ConferenceInfo> {
        let mut structured: Vec<ConferenceInfo> = Vec::new();
        if let Some(url) = dto
            .online_meeting
            .as_ref()
            .and_then(|meeting| meeting.join_url.as_deref())
            .and_then(UrlString::parse)
        {
            let provider =
                ConferenceDetector::provider_of(&url).unwrap_or(ConferenceProvider::Teams);
            structured.push(ConferenceInfo::new(
                url,
                provider,
                ConferenceOrigin::Structured,
            ));
        }
        ConferenceDetector::conferences(
            &structured,
            dto.location
                .as_ref()
                .and_then(|location| location.display_name.as_deref()),
            None,
            notes,
        )
    }

    /// Graph keeps one reminder per event: on or off, and how many minutes before the start.
    fn reminders(dto: &GraphEventDTO) -> Vec<Reminder> {
        if dto.is_reminder_on != Some(true) {
            return Vec::new();
        }
        vec![Reminder::before(
            dto.reminder_minutes_before_start.unwrap_or(15),
            ReminderType::Display,
            None,
        )]
    }

    /// Reads reach Graph with `Prefer: outlook.body-content-type="text"`, so a body is normally plain text already.
    /// An HTML body (a request that lost the preference) is reduced to text, so notes never carry markup.
    pub(crate) fn plain_text(body: Option<&GraphBodyDTO>) -> Option<String> {
        let body = body?;
        let mut text = body.content.clone().filter(|content| !content.is_empty())?;
        if body
            .content_type
            .as_deref()
            .map(str::to_lowercase)
            .as_deref()
            == Some("html")
        {
            text = replace_line_breaks(&text);
            text = text_without_tags(&text);
            for (entity, character) in [
                ("&nbsp;", " "),
                ("&lt;", "<"),
                ("&gt;", ">"),
                ("&quot;", "\""),
                ("&#39;", "'"),
                ("&amp;", "&"),
            ] {
                text = text.replace(entity, character);
            }
        }
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }
}

/// `<br\s*/?>` and `</p>` (both case-insensitive) become a newline.
fn replace_line_breaks(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<'
            && let Some(length) = line_break_length(&chars[i..])
        {
            out.push('\n');
            i += length;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// The length of a `<br\s*/?>` or `</p>` tag at the start of `chars`.
fn line_break_length(chars: &[char]) -> Option<usize> {
    let is = |index: usize, wanted: char| {
        chars
            .get(index)
            .is_some_and(|c| c.eq_ignore_ascii_case(&wanted))
    };
    if is(1, 'b') && is(2, 'r') {
        let mut at = 3;
        while chars.get(at).is_some_and(|c| c.is_whitespace()) {
            at += 1;
        }
        if chars.get(at) == Some(&'/') {
            at += 1;
        }
        if chars.get(at) == Some(&'>') {
            return Some(at + 1);
        }
    }
    if chars.get(1) == Some(&'/') && is(2, 'p') && chars.get(3) == Some(&'>') {
        return Some(4);
    }
    None
}

/// `<[^>]+>` replaced by nothing: from a `<`, up to the next `>` when at least one character lies between.
fn text_without_tags(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<' {
            match chars[i + 1..].iter().position(|c| *c == '>') {
                Some(close) if close > 0 => {
                    i += close + 2;
                    continue;
                }
                // `<>`: nothing between, so no tag starts here.
                Some(_) => {}
                // No `>` is left, so no tag can start anywhere after this point either.
                None => {
                    out.extend(&chars[i..]);
                    break;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}
