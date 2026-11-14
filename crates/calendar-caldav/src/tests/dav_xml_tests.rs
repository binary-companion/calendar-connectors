//! Port of `DAVXMLTests.swift`.

use std::collections::BTreeMap;

use calendar_core::failure::Failure;
use calendar_core::http::HttpResponse;
use calendar_core::source_types::SourceError;
use jiff::Timestamp;

use crate::dav_xml::{Dav, DavProperty, DavXml};
use crate::xml_tree::XmlTree;

/// Shaped like iCloud's answer to a depth-1 PROPFIND on a calendar home (scrubbed; uppercase prefixes).
pub(super) const HOME_MULTISTATUS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<multistatus xmlns="DAV:">
 <response>
  <href>/123/calendars/home/</href>
  <propstat>
   <prop>
    <displayname>Home</displayname>
    <resourcetype><collection/><calendar xmlns="urn:ietf:params:xml:ns:caldav"/></resourcetype>
    <calendar-color xmlns="http://apple.com/ns/ical/">#FF2968FF</calendar-color>
    <getctag xmlns="http://calendarserver.org/ns/">HwoQEgwAAA</getctag>
    <sync-token>https://example.test/sync/1</sync-token>
   </prop>
   <status>HTTP/1.1 200 OK</status>
  </propstat>
  <propstat>
   <prop><calendar-timezone xmlns="urn:ietf:params:xml:ns:caldav"/></prop>
   <status>HTTP/1.1 404 Not Found</status>
  </propstat>
 </response>
</multistatus>"#;

fn is_invalid(error: Failure) -> bool {
    matches!(error, Failure::Source(SourceError::InvalidResponse(_)))
}

#[test]
fn parses_multistatus_with_found_and_missing_properties() {
    let result = DavXml::multistatus(HOME_MULTISTATUS.as_bytes()).unwrap();
    let response = result.responses.first().unwrap();
    assert_eq!(response.href, "/123/calendars/home/");
    let get = |p: DavProperty| response.properties.get(&p);
    assert_eq!(
        get(DavProperty::display_name()).unwrap().trimmed_text(),
        "Home"
    );
    assert_eq!(
        get(DavProperty::calendar_color()).unwrap().trimmed_text(),
        "#FF2968FF"
    );
    assert_eq!(
        get(DavProperty::get_ctag()).unwrap().trimmed_text(),
        "HwoQEgwAAA"
    );
    assert_eq!(
        get(DavProperty::sync_token()).unwrap().trimmed_text(),
        "https://example.test/sync/1"
    );
    assert!(
        get(DavProperty::resource_type())
            .unwrap()
            .child(Dav::CALDAV, "calendar")
            .is_some()
    );
    // A 404 propstat is not a value.
    assert!(get(DavProperty::calendar_time_zone()).is_none());
}

#[test]
fn multistatus_ignores_prefixes() {
    let prefixed = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/">
      <d:response><d:href>/cal/a.ics</d:href>
        <d:propstat><d:prop><d:getetag>"7"</d:getetag><c:calendar-data>BEGIN:VCALENDAR</c:calendar-data></d:prop>
        <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
      <d:response><d:href>/cal/gone.ics</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response>
      <d:sync-token>tok-2</d:sync-token>
    </d:multistatus>"#;
    let result = DavXml::multistatus(prefixed.as_bytes()).unwrap();
    let hrefs: Vec<&str> = result.responses.iter().map(|r| r.href.as_str()).collect();
    assert_eq!(hrefs, ["/cal/a.ics", "/cal/gone.ics"]);
    assert_eq!(
        result.responses[0].properties[&DavProperty::get_etag()].trimmed_text(),
        "\"7\""
    );
    assert_eq!(
        result.responses[0].properties[&DavProperty::calendar_data()].text,
        "BEGIN:VCALENDAR"
    );
    assert_eq!(result.responses[1].status, Some(404));
    assert_eq!(result.sync_token.as_deref(), Some("tok-2"));
}

#[test]
fn rejects_non_xml() {
    assert!(is_invalid(
        DavXml::multistatus(b"<html><body>oops").unwrap_err()
    ));
    assert!(is_invalid(
        DavXml::multistatus(b"<other xmlns=\"DAV:\"/>").unwrap_err()
    ));
}

#[test]
fn builds_request_bodies() {
    let propfind = XmlTree::parse(&DavXml::propfind(&[
        DavProperty::display_name(),
        DavProperty::get_ctag(),
    ]))
    .unwrap();
    assert!(propfind.namespace == Dav::DAV && propfind.name == "propfind");
    assert!(
        propfind
            .child(Dav::DAV, "prop")
            .unwrap()
            .child(Dav::CALENDAR_SERVER, "getctag")
            .is_some()
    );

    let start = Timestamp::from_second(1_790_521_200).unwrap();
    let end = Timestamp::from_second(1_790_521_200 + 86_400).unwrap();
    let query = XmlTree::parse(&DavXml::calendar_query_range(start, end)).unwrap();
    assert_eq!(query.name, "calendar-query");
    let range = query.first(Dav::CALDAV, "time-range").unwrap();
    assert!(
        range.attributes["start"] == "20260927T150000Z"
            && range.attributes["end"] == "20260928T150000Z"
    );
    assert!(
        query.first(Dav::CALDAV, "calendar-data").is_some()
            && query.first(Dav::DAV, "getetag").is_some()
    );

    let by_uid = XmlTree::parse(&DavXml::calendar_query_uid("a<b&c")).unwrap();
    assert_eq!(
        by_uid
            .first(Dav::CALDAV, "text-match")
            .unwrap()
            .trimmed_text(),
        "a<b&c"
    );

    let sync = XmlTree::parse(&DavXml::sync_collection(None)).unwrap();
    assert!(
        sync.name == "sync-collection"
            && sync.child(Dav::DAV, "sync-token").unwrap().trimmed_text() == ""
    );
    assert_eq!(
        sync.child(Dav::DAV, "sync-level").unwrap().trimmed_text(),
        "1"
    );
}

#[test]
fn recognizes_an_invalid_sync_token() {
    let body = b"<error xmlns=\"DAV:\"><valid-sync-token/></error>".to_vec();
    let answer =
        |status: i64, body: &[u8]| HttpResponse::new(status, BTreeMap::new(), body.to_vec());
    assert!(DavXml::is_invalid_sync_token(&answer(403, &body)));
    assert!(DavXml::is_invalid_sync_token(&answer(409, &body)));
    assert!(!DavXml::is_invalid_sync_token(&answer(
        403,
        b"<error xmlns=\"DAV:\"><need-privileges/></error>"
    )));
}
