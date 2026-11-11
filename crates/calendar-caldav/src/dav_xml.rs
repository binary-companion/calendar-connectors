//! Port of `DAVXML.swift`: WebDAV and CalDAV request bodies, and the multistatus reader.

use std::collections::BTreeMap;

use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::HttpResponse;
use calendar_core::source_types::SourceError;
use calendar_ical::ICalValues;
use jiff::Timestamp;

use crate::xml_tree::XmlTree;

/// Swift's `enum DAV` (the namespaces).
pub(crate) enum Dav {}

impl Dav {
    pub(crate) const DAV: &'static str = "DAV:";
    pub(crate) const CALDAV: &'static str = "urn:ietf:params:xml:ns:caldav";
    pub(crate) const CALENDAR_SERVER: &'static str = "http://calendarserver.org/ns/";
    pub(crate) const APPLE: &'static str = "http://apple.com/ns/ical/";
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct DavProperty {
    pub namespace: String,
    pub name: String,
}

impl DavProperty {
    pub(crate) fn new(namespace: &str, name: &str) -> DavProperty {
        DavProperty {
            namespace: namespace.to_string(),
            name: name.to_string(),
        }
    }

    pub(crate) fn current_user_principal() -> DavProperty {
        DavProperty::new(Dav::DAV, "current-user-principal")
    }
    pub(crate) fn calendar_home_set() -> DavProperty {
        DavProperty::new(Dav::CALDAV, "calendar-home-set")
    }
    pub(crate) fn calendar_user_address_set() -> DavProperty {
        DavProperty::new(Dav::CALDAV, "calendar-user-address-set")
    }
    pub(crate) fn schedule_inbox_url() -> DavProperty {
        DavProperty::new(Dav::CALDAV, "schedule-inbox-URL")
    }
    pub(crate) fn schedule_default_calendar_url() -> DavProperty {
        DavProperty::new(Dav::CALDAV, "schedule-default-calendar-URL")
    }
    pub(crate) fn resource_type() -> DavProperty {
        DavProperty::new(Dav::DAV, "resourcetype")
    }
    pub(crate) fn display_name() -> DavProperty {
        DavProperty::new(Dav::DAV, "displayname")
    }
    pub(crate) fn supported_components() -> DavProperty {
        DavProperty::new(Dav::CALDAV, "supported-calendar-component-set")
    }
    pub(crate) fn privileges() -> DavProperty {
        DavProperty::new(Dav::DAV, "current-user-privilege-set")
    }
    pub(crate) fn calendar_time_zone() -> DavProperty {
        DavProperty::new(Dav::CALDAV, "calendar-timezone")
    }
    pub(crate) fn calendar_color() -> DavProperty {
        DavProperty::new(Dav::APPLE, "calendar-color")
    }
    pub(crate) fn get_ctag() -> DavProperty {
        DavProperty::new(Dav::CALENDAR_SERVER, "getctag")
    }
    pub(crate) fn sync_token() -> DavProperty {
        DavProperty::new(Dav::DAV, "sync-token")
    }
    pub(crate) fn get_etag() -> DavProperty {
        DavProperty::new(Dav::DAV, "getetag")
    }
    pub(crate) fn calendar_data() -> DavProperty {
        DavProperty::new(Dav::CALDAV, "calendar-data")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DavResponse {
    pub href: String,
    /// The response-level status (a `sync-collection` removal is 404); `None` when the response has propstats.
    pub status: Option<i64>,
    /// Properties from `200` propstats only.
    pub properties: BTreeMap<DavProperty, XmlTree>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Multistatus {
    pub responses: Vec<DavResponse>,
    pub sync_token: Option<String>,
}

pub(crate) enum DavXml {}

fn invalid(message: &str) -> Failure {
    Failure::Source(SourceError::InvalidResponse(message.to_string()))
}

impl DavXml {
    pub(crate) fn multistatus(data: &[u8]) -> Outcome<Multistatus> {
        let root = XmlTree::parse(data)?;
        if root.namespace != Dav::DAV || root.name != "multistatus" {
            return Err(invalid("expected a multistatus body"));
        }
        let mut responses = Vec::new();
        for response in root.children_named(Dav::DAV, "response") {
            let href = match response.child(Dav::DAV, "href") {
                Some(href) if !href.trimmed_text().is_empty() => href.trimmed_text().to_string(),
                _ => continue,
            };
            let mut properties = BTreeMap::new();
            for propstat in response.children_named(Dav::DAV, "propstat") {
                if DavXml::status(propstat.child(Dav::DAV, "status")) != Some(200) {
                    continue;
                }
                if let Some(prop) = propstat.child(Dav::DAV, "prop") {
                    for property in &prop.children {
                        properties.insert(
                            DavProperty::new(&property.namespace, &property.name),
                            property.clone(),
                        );
                    }
                }
            }
            responses.push(DavResponse {
                href,
                status: DavXml::status(response.child(Dav::DAV, "status")),
                properties,
            });
        }
        Ok(Multistatus {
            responses,
            sync_token: root
                .child(Dav::DAV, "sync-token")
                .map(|t| t.trimmed_text().to_string()),
        })
    }

    /// `HTTP/1.1 200 OK` gives 200.
    pub(crate) fn status(element: Option<&XmlTree>) -> Option<i64> {
        let parts: Vec<&str> = element?
            .trimmed_text()
            .split(' ')
            .filter(|p| !p.is_empty())
            .collect();
        if parts.len() < 2 {
            return None;
        }
        parts[1].parse().ok()
    }

    pub(crate) fn propfind(properties: &[DavProperty]) -> Vec<u8> {
        let elements: String = properties.iter().map(DavXml::element).collect();
        DavXml::document(&format!(
            "<d:propfind {}><d:prop>{elements}</d:prop></d:propfind>",
            DavXml::namespaces()
        ))
    }

    /// Swift `calendarQuery(from:to:)`.
    pub(crate) fn calendar_query_range(start: Timestamp, end: Timestamp) -> Vec<u8> {
        DavXml::document(&format!(
            "<c:calendar-query {}><d:prop><d:getetag/><c:calendar-data/></d:prop>\
             <c:filter><c:comp-filter name=\"VCALENDAR\"><c:comp-filter name=\"VEVENT\">\
             <c:time-range start=\"{}\" end=\"{}\"/>\
             </c:comp-filter></c:comp-filter></c:filter></c:calendar-query>",
            DavXml::namespaces(),
            ICalValues::utc_text(start),
            ICalValues::utc_text(end)
        ))
    }

    /// Swift `calendarQuery(uid:)`.
    pub(crate) fn calendar_query_uid(uid: &str) -> Vec<u8> {
        DavXml::document(&format!(
            "<c:calendar-query {}><d:prop><d:getetag/><c:calendar-data/></d:prop>\
             <c:filter><c:comp-filter name=\"VCALENDAR\"><c:comp-filter name=\"VEVENT\"><c:prop-filter name=\"UID\">\
             <c:text-match collation=\"i;octet\">{}</c:text-match>\
             </c:prop-filter></c:comp-filter></c:comp-filter></c:filter></c:calendar-query>",
            DavXml::namespaces(),
            DavXml::escape(uid)
        ))
    }

    pub(crate) fn sync_collection(token: Option<&str>) -> Vec<u8> {
        DavXml::document(&format!(
            "<d:sync-collection {}><d:sync-token>{}</d:sync-token><d:sync-level>1</d:sync-level>\
             <d:prop><d:getetag/></d:prop></d:sync-collection>",
            DavXml::namespaces(),
            DavXml::escape(token.unwrap_or(""))
        ))
    }

    /// RFC 6578: a token the server no longer accepts is a 403 (or 409 on some servers) with `DAV:valid-sync-token`.
    pub(crate) fn is_invalid_sync_token(response: &HttpResponse) -> bool {
        if response.status != 403 && response.status != 409 {
            return false;
        }
        XmlTree::parse(&response.body)
            .map(|tree| tree.first(Dav::DAV, "valid-sync-token").is_some())
            .unwrap_or(false)
    }

    fn namespaces() -> String {
        format!(
            "xmlns:d=\"DAV:\" xmlns:c=\"{}\" xmlns:cs=\"{}\" xmlns:a=\"{}\"",
            Dav::CALDAV,
            Dav::CALENDAR_SERVER,
            Dav::APPLE
        )
    }

    fn element(property: &DavProperty) -> String {
        let prefix = match property.namespace.as_str() {
            Dav::DAV => "d",
            Dav::CALDAV => "c",
            Dav::CALENDAR_SERVER => "cs",
            Dav::APPLE => "a",
            other => {
                return format!(
                    "<x:{} xmlns:x=\"{}\"/>",
                    property.name,
                    DavXml::escape(other)
                );
            }
        };
        format!("<{prefix}:{}/>", property.name)
    }

    fn document(body: &str) -> Vec<u8> {
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>{body}").into_bytes()
    }

    pub(crate) fn escape(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }
}
