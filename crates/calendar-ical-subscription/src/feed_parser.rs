//! Port of `FeedParser.swift`.

use std::collections::{BTreeMap, BTreeSet};

use calendar_core::diagnostic_log::{
    DiagnosticEvent, DiagnosticField, DiagnosticLevel, DiagnosticLog,
};
use calendar_core::source_types::SourceError;
use calendar_core::time::{DateInterval, Zone};
use calendar_ical::{
    EventReadContext, EventReader, EventResource, ICalComponent, ICalParser, ICalSerializer,
};
use jiff::Timestamp;

/// One UID's events from a feed (a master plus its overrides) as the resource the shared `EventReader` expects.
#[derive(Clone, Debug)]
pub(crate) struct FeedResource {
    pub name: String,
    pub resource: EventResource,
}

/// A group's UID and a hash of its `RRULE` lines (never the rule text), so an edited rule is a new entry.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct UnreadableRule {
    pub uid: String,
    /// The UID was made up from the event's start and title (the feed gave none), so it must not be logged.
    pub is_synthetic: bool,
    pub rule_hash: String,
}

impl UnreadableRule {
    /// The diagnostic for this rule: the UID only when the feed supplied it.
    pub(crate) fn event(&self, at: Timestamp) -> DiagnosticEvent {
        DiagnosticEvent::new(
            DiagnosticLevel::Notice,
            "icalsub",
            "rruleUnreadable",
            vec![if self.is_synthetic {
                DiagnosticField::bool("syntheticUID", true)
            } else {
                DiagnosticField::string("uid", self.uid.clone())
            }],
            at,
        )
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ParsedFeed {
    pub name: Option<String>,
    pub color_hex: Option<String>,
    pub time_zone: Option<Zone>,
    pub resources: Vec<FeedResource>,
    /// The kept groups whose recurrence rule could not be read, to tell "still broken" from "newly broken".
    pub unreadable_rules: Vec<UnreadableRule>,
}

impl ParsedFeed {
    /// A deterministic form of what the user sees (header and retained events), to tell a real change from an edit
    /// outside the retention window or a feed that restamps itself on every request. Computed on demand.
    pub(crate) fn fingerprint(&self) -> Vec<u8> {
        FeedParser::fingerprint_of(self)
    }
}

pub(crate) enum FeedParser {}

const NOT_A_CALENDAR: &str = "that link did not return a calendar";

/// Properties a feed may rewrite on every request without the event having changed.
const VOLATILE: [&str; 4] = ["DTSTAMP", "LAST-MODIFIED", "CREATED", "PRODID"];

impl FeedParser {
    /// Swift `parse(_:)` with its defaults: no retention window, no diagnostics.
    #[cfg(test)]
    pub(crate) fn parse(data: &[u8]) -> Result<ParsedFeed, SourceError> {
        FeedParser::parse_with_retention(data, None)
    }

    /// Swift `parse(_:retention:)` without diagnostics.
    #[cfg(test)]
    pub(crate) fn parse_with_retention(
        data: &[u8],
        retention: Option<(DateInterval, Zone)>,
    ) -> Result<ParsedFeed, SourceError> {
        FeedParser::parse_logged(
            data,
            retention,
            &calendar_core::diagnostic_log::NullDiagnosticLog,
            Timestamp::UNIX_EPOCH,
            true,
        )
    }

    /// A feed is one `VCALENDAR` holding many events, while `EventResource` holds one UID's events. Events are grouped
    /// by `UID` (first-seen order); each group shares the calendar's header and `VTIMEZONE`s. A feed with no events is
    /// valid. With `retention`, a UID group with no occurrence overlapping the window (read in `zone`) is dropped; any
    /// other group is kept whole, and so is a group whose recurrence rule cannot be read (its occurrences are unknown,
    /// so it cannot be shown to be outside the window). `diagnostics` gets counts and fixed tokens only: never text
    /// from the feed. Swift's `date` default is not available (no clock): `at` dates every diagnostic.
    pub(crate) fn parse_logged(
        data: &[u8],
        retention: Option<(DateInterval, Zone)>,
        diagnostics: &dyn DiagnosticLog,
        at: Timestamp,
        logs_unreadable_rules: bool,
    ) -> Result<ParsedFeed, SourceError> {
        let not_a_calendar = || {
            diagnostics.record(
                DiagnosticLevel::Warning,
                "icalsub",
                "feedNotCalendar",
                Vec::new(),
                at,
            );
            SourceError::InvalidResponse(NOT_A_CALENDAR.to_string())
        };
        let Ok(calendar) = ICalParser::parse(data) else {
            return Err(not_a_calendar());
        };
        if calendar.name != "VCALENDAR" {
            return Err(not_a_calendar());
        }

        let zones: Vec<ICalComponent> = calendar
            .components_named("VTIMEZONE")
            .into_iter()
            .cloned()
            .collect();
        let mut order: Vec<String> = Vec::new();
        let mut groups: BTreeMap<String, Vec<ICalComponent>> = BTreeMap::new();
        let mut events_in_feed: i64 = 0;
        let mut without_uid: i64 = 0;
        let mut synthetic_uids: BTreeSet<String> = BTreeSet::new();
        for event in calendar.components_named("VEVENT") {
            events_in_feed += 1;
            let given = event
                .property("UID")
                .map(|property| property.text())
                .filter(|text| !text.is_empty());
            let uid = match given {
                Some(uid) => uid,
                None => {
                    without_uid += 1;
                    let uid = FeedParser::synthetic_uid(event);
                    synthetic_uids.insert(uid.clone());
                    uid
                }
            };
            if !groups.contains_key(&uid) {
                order.push(uid.clone());
            }
            groups.entry(uid).or_default().push(event.clone());
        }
        let mut resources: Vec<FeedResource> = Vec::new();
        let feed_zone = calendar
            .property("X-WR-TIMEZONE")
            .and_then(|property| Zone::named(&property.value));
        let mut dropped: i64 = 0;
        let mut unreadable: Vec<UnreadableRule> = Vec::new();
        for uid in &order {
            let mut components = zones.clone();
            components.extend(groups.get(uid).cloned().unwrap_or_default());
            let wrapper = ICalComponent::new("VCALENDAR", calendar.properties.clone(), components);
            let Ok(resource) = EventResource::new(wrapper) else {
                continue;
            };
            let item = FeedResource {
                name: FeedParser::resource_name(uid),
                resource,
            };
            let zone = feed_zone
                .clone()
                .or_else(|| retention.as_ref().map(|(_, zone)| zone.clone()))
                .unwrap_or_else(Zone::utc);
            let is_unreadable = FeedParser::has_unreadable_rule(&item, &zone);
            if is_unreadable {
                let rule = UnreadableRule {
                    uid: uid.clone(),
                    is_synthetic: synthetic_uids.contains(uid),
                    rule_hash: FeedParser::fnv1a_hex(&FeedParser::rule_text(&item)),
                };
                if logs_unreadable_rules {
                    diagnostics.log(rule.event(at));
                }
                unreadable.push(rule);
            }
            if let Some((window, _)) = &retention
                && !is_unreadable
                && !FeedParser::is_kept(&item, window, &zone)
            {
                dropped += 1;
                continue;
            }
            resources.push(item);
        }
        if without_uid > 0 {
            diagnostics.record(
                DiagnosticLevel::Info,
                "icalsub",
                "syntheticUID",
                vec![DiagnosticField::int("count", without_uid)],
                at,
            );
        }
        diagnostics.record(
            DiagnosticLevel::Info,
            "icalsub",
            "feedParsed",
            vec![
                DiagnosticField::int("eventsInFeed", events_in_feed),
                DiagnosticField::int("groupsKept", resources.len() as i64),
                DiagnosticField::int("groupsDropped", dropped),
                DiagnosticField::int("unreadableRules", unreadable.len() as i64),
                DiagnosticField::bool("retentionActive", retention.is_some()),
            ],
            at,
        );
        let color = calendar
            .property("X-APPLE-CALENDAR-COLOR")
            .map(|property| property.value.clone())
            .or_else(|| {
                calendar
                    .property("COLOR")
                    .map(|property| property.value.clone())
            });
        Ok(ParsedFeed {
            name: calendar
                .property("X-WR-CALNAME")
                .map(|property| property.text())
                .filter(|text| !text.is_empty()),
            color_hex: color.map(|raw| FeedParser::color_text(&raw)),
            time_zone: feed_zone,
            resources,
            unreadable_rules: unreadable,
        })
    }

    /// A group is kept when the shared reader finds at least one event or occurrence overlapping the window. The
    /// reader expands rules, dates and exceptions only inside the window (skipping straight to it, with an instance
    /// limit), applies overrides, and decides zero-length and all-day events, so an unbounded series stays cheap.
    fn is_kept(item: &FeedResource, window: &DateInterval, zone: &Zone) -> bool {
        let context = EventReadContext::new(
            "feed",
            item.name.clone(),
            None,
            Some(String::new()),
            zone.clone(),
            Default::default(),
        );
        !EventReader::events_in(&item.resource, window, &context).is_empty()
    }

    /// The group's `RRULE` lines, to tell an edited rule from an unchanged one.
    fn rule_text(item: &FeedResource) -> String {
        item.resource
            .master()
            .map(|master| {
                master
                    .properties_named("RRULE")
                    .into_iter()
                    .map(ICalSerializer::content_line)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    }

    /// True when the group's series master has an `RRULE` the shared reader could not read. A group with no usable
    /// `DTSTART` has no timing to expand from, so it is never checked (and never reported).
    fn has_unreadable_rule(item: &FeedResource, zone: &Zone) -> bool {
        let Some(master) = item.resource.master() else {
            return false;
        };
        let resolver = item.resource.resolver();
        let Some(timing) = EventReader::timing_of(master, &resolver, zone) else {
            return false;
        };
        EventReader::recurrence_set_of(master, &timing.zone, timing.is_all_day, &resolver)
            .is_some_and(|set| set.has_unreadable_rule())
    }

    pub(crate) fn fingerprint_of(feed: &ParsedFeed) -> Vec<u8> {
        let mut data = format!(
            "{}|{}|{}\n",
            feed.name.as_deref().unwrap_or(""),
            feed.color_hex.as_deref().unwrap_or(""),
            feed.time_zone.as_ref().map_or("", |zone| zone.identifier()),
        )
        .into_bytes();
        let mut items: Vec<&FeedResource> = feed.resources.iter().collect();
        items.sort_by(|a, b| a.name.cmp(&b.name));
        for item in items {
            data.extend(format!("#{}\n", item.name).into_bytes());
            let mut events: Vec<ICalComponent> = item
                .resource
                .events()
                .into_iter()
                .map(|event| {
                    ICalComponent::new(
                        &event.name,
                        event
                            .properties
                            .iter()
                            .filter(|property| !VOLATILE.contains(&property.name.as_str()))
                            .cloned()
                            .collect(),
                        event.components.clone(),
                    )
                })
                .collect();
            let key = |event: &ICalComponent| {
                event
                    .property("RECURRENCE-ID")
                    .map(|property| property.value.clone())
                    .unwrap_or_default()
            };
            events.sort_by_cached_key(key);
            for event in &events {
                data.extend(ICalSerializer::serialize(event).into_bytes());
            }
        }
        data
    }

    /// The UID as one URL-safe path segment: it is the base of every event id, which joins an occurrence's start with
    /// `#`. ASCII letters and digits and `-._~@` stay; every other scalar is percent-encoded as UTF-8, which is what
    /// `addingPercentEncoding(withAllowedCharacters:)` does to non-ASCII letters too (probed).
    pub(crate) fn resource_name(uid: &str) -> String {
        let mut out = String::with_capacity(uid.len());
        for &b in uid.as_bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'@') {
                out.push(char::from(b));
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        out
    }

    /// A UID for an event that has none: stable across reads because it comes only from the event's start and title.
    pub(crate) fn synthetic_uid(event: &ICalComponent) -> String {
        let value = |name: &str| {
            event
                .property(name)
                .map(|property| property.value.clone())
                .unwrap_or_default()
        };
        format!(
            "feed-{}",
            FeedParser::fnv1a_hex(&format!("{}|{}", value("DTSTART"), value("SUMMARY")))
        )
    }

    /// FNV-1a, 64 bit. Not for security: it only keeps a made-up UID the same from one read to the next.
    pub(crate) fn fnv1a_hex(text: &str) -> String {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in text.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("{hash:x}")
    }

    /// `#RRGGBBAA` (Apple) is trimmed to `#RRGGBB`; the descriptor validates the rest. (The length is counted in
    /// `char`s, Swift counts `Character`s: the two differ only for text with combining marks, never a colour.)
    fn color_text(raw: &str) -> String {
        if raw.starts_with('#') && raw.chars().count() == 9 {
            raw.chars().take(7).collect()
        } else {
            raw.to_string()
        }
    }
}
