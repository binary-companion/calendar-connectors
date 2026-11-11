use crate::UrlString;
use crate::conference_detector::ConferenceDetector;
use crate::join_url_policy::JoinUrlPolicy;
use crate::model::{ConferenceInfo, ConferenceOrigin, ConferenceProvider};

fn url(text: &str) -> UrlString {
    UrlString::parse(text).unwrap()
}

/// `detect(location:url:notes:)` of the Swift test file; `url` that `URL(string:)` rejects is passed as nil.
fn detect_all(
    location: Option<&str>,
    link: Option<&str>,
    notes: Option<&str>,
) -> Option<UrlString> {
    let link = link.and_then(UrlString::parse);
    ConferenceDetector::detect(location, link.as_ref(), notes)
}

fn detect_in_notes(notes: &str) -> Option<UrlString> {
    detect_all(None, None, Some(notes))
}

fn detect_in_url(link: &str) -> Option<UrlString> {
    detect_all(None, Some(link), None)
}

fn host_of(found: &Option<UrlString>) -> Option<String> {
    found.as_ref().and_then(UrlString::host)
}

#[test]
fn finds_zoom_in_location() {
    let found = detect_all(Some("https://acme.zoom.us/j/123456789?pwd=abc"), None, None);
    assert_eq!(host_of(&found).as_deref(), Some("acme.zoom.us"));
}

#[test]
fn finds_meet_in_notes() {
    let found = detect_in_notes("Join: https://meet.google.com/abc-defg-hij.");
    assert_eq!(
        found.as_ref().map(UrlString::as_str),
        Some("https://meet.google.com/abc-defg-hij")
    );
}

#[test]
fn finds_teams_inside_html_with_entities() {
    let notes = r#"<p><a href="https://teams.microsoft.com/l/meetup-join/19%3ameeting?a=1&amp;b=2">Join</a></p>"#;
    let result = detect_in_notes(notes);
    assert_eq!(host_of(&result).as_deref(), Some("teams.microsoft.com"));
    assert_eq!(
        result.as_ref().and_then(UrlString::query).as_deref(),
        Some("a=1&b=2")
    );
}

#[test]
fn unwraps_outlook_safe_links() {
    let wrapped = "https://nam02.safelinks.protection.outlook.com/?url=https%3A%2F%2Fteams.microsoft.com%2Fl%2Fmeetup-join%2F0&data=05";
    let found = detect_in_notes(&format!("Click {wrapped} now"));
    assert_eq!(host_of(&found).as_deref(), Some("teams.microsoft.com"));
}

#[test]
fn unwraps_google_redirects() {
    let wrapped = "https://www.google.com/url?q=https://acme.zoom.us/j/1&sa=D";
    assert_eq!(
        host_of(&detect_in_notes(wrapped)).as_deref(),
        Some("acme.zoom.us")
    );
}

#[test]
fn accepts_zoom_mtg_scheme() {
    let found = detect_in_notes("zoommtg://zoom.us/join?confno=123");
    assert_eq!(
        found.as_ref().and_then(UrlString::scheme).as_deref(),
        Some("zoommtg")
    );
}

#[test]
fn ignores_non_provider_links_in_text() {
    assert_eq!(
        detect_in_notes("Agenda: https://docs.google.com/document/d/abc"),
        None
    );
}

#[test]
fn location_beats_notes() {
    let result = detect_all(
        Some("https://meet.google.com/aaa-bbbb-ccc"),
        None,
        Some("https://acme.zoom.us/j/1"),
    );
    assert_eq!(host_of(&result).as_deref(), Some("meet.google.com"));
}

#[test]
fn returns_first_provider_link_skipping_others() {
    let notes = "Doc https://docs.google.com/x then https://acme.zoom.us/j/9";
    assert_eq!(
        host_of(&detect_in_notes(notes)).as_deref(),
        Some("acme.zoom.us")
    );
}

#[test]
fn event_url_fallback_when_not_allowlisted() {
    let found = detect_in_url("https://example.com/meeting/42");
    assert_eq!(
        found.as_ref().map(UrlString::as_str),
        Some("https://example.com/meeting/42")
    );
}

#[test]
fn event_url_fallback_excludes_google_calendars_own_permalink() {
    // Google sets an event's `url` to a link back to its own web UI (the API's `htmlLink`) on every event,
    // even one with no real conference; that is never a join link, so it must not win the raw-url fallback.
    assert_eq!(
        detect_in_url("https://www.google.com/calendar/event?eid=abc123"),
        None
    );
    assert_eq!(
        detect_in_url("https://calendar.google.com/calendar/u/0/r/eventedit/abc123"),
        None
    );
}

#[test]
fn google_redirect_inside_event_url_still_unwraps_to_a_real_provider() {
    // The exclusion only guards the raw fallback: a genuine www.google.com/url redirect to a
    // provider (as Google sometimes wraps links in notes) is still found via the text scan.
    let found = detect_in_url("https://www.google.com/url?q=https://acme.zoom.us/j/1&sa=D");
    assert_eq!(host_of(&found).as_deref(), Some("acme.zoom.us"));
}

#[test]
fn slack_huddle_needs_huddle_path() {
    assert!(detect_in_notes("https://app.slack.com/huddle/T1/C1").is_some());
    assert!(detect_in_notes("https://app.slack.com/client/T1/C1").is_none());
}

#[test]
fn no_links_returns_nil() {
    assert_eq!(detect_all(Some("Room 4"), None, Some("Bring laptop")), None);
}

// ISSUE 1: zoommtg URL host validation
#[test]
fn zoommtg_requires_zoom_domain() {
    // Evil domain should not match
    assert_eq!(detect_in_notes("zoommtg://evil.com/join?confno=1"), None);
}

#[test]
fn zoommtg_zoom_us_still_works() {
    // Legitimate zoom.us should still work
    let found = detect_in_notes("zoommtg://zoom.us/join?confno=123");
    assert_eq!(
        found.as_ref().and_then(UrlString::scheme).as_deref(),
        Some("zoommtg")
    );
}

// ISSUE 2: SafeLinks host boundary validation
#[test]
fn safelinks_requires_dot_boundary() {
    // Evil subdomain should not be unwrapped
    let evil =
        "https://evilsafelinks.protection.outlook.com/?url=https%3A%2F%2Fmeet.google.com%2Fa-b-c";
    assert_eq!(detect_in_notes(evil), None);
}

// ISSUE 3: Slack huddle path prefix validation
#[test]
fn slack_huddle_path_must_be_exact() {
    // /huddlefoo should not match
    assert_eq!(detect_in_notes("https://app.slack.com/huddlefoo/x"), None);
}

#[test]
fn slack_huddle_path_still_works_with_slash() {
    // /huddle/ should still match
    assert!(detect_in_notes("https://app.slack.com/huddle/T1/C1").is_some());
}

// ISSUE 4: Case-insensitive regex
#[test]
fn upper_case_url_scheme_and_host_match() {
    let result = detect_in_notes("HTTPS://MEET.GOOGLE.COM/abc-defg-hij");
    assert!(result.is_some());
    assert_eq!(
        host_of(&result).map(|h| h.to_lowercase()).as_deref(),
        Some("meet.google.com")
    );
}

// The list API.

fn list(
    structured: &[ConferenceInfo],
    location: Option<&str>,
    link: Option<&str>,
    notes: Option<&str>,
) -> Vec<ConferenceInfo> {
    let link = link.and_then(UrlString::parse);
    ConferenceDetector::conferences(structured, location, link.as_ref(), notes)
}

fn providers(list: &[ConferenceInfo]) -> Vec<ConferenceProvider> {
    list.iter().map(|c| c.provider).collect()
}

fn origins(list: &[ConferenceInfo]) -> Vec<ConferenceOrigin> {
    list.iter().map(|c| c.origin).collect()
}

#[test]
fn keeps_every_provider_link_in_order_with_its_origin() {
    let result = list(
        &[],
        Some("https://meet.google.com/aaa-bbbb-ccc"),
        None,
        Some("or https://acme.zoom.us/j/9 and https://acme.webex.com/meet/x"),
    );
    use ConferenceProvider::{Meet, Webex, Zoom};
    assert_eq!(providers(&result), [Meet, Zoom, Webex]);
    assert_eq!(
        origins(&result),
        [
            ConferenceOrigin::Location,
            ConferenceOrigin::Notes,
            ConferenceOrigin::Notes
        ]
    );
}

#[test]
fn duplicates_across_fields_are_removed_first_wins() {
    let result = list(
        &[],
        Some("https://acme.zoom.us/j/9"),
        None,
        Some("https://ACME.zoom.us/j/9/ again"),
    );
    assert!(result.len() == 1 && result[0].origin == ConferenceOrigin::Location);
}

#[test]
fn structured_links_come_first_and_are_not_repeated_by_the_text_scan() {
    let structured = ConferenceInfo::new(
        url("https://acme.zoom.us/j/1"),
        ConferenceProvider::Zoom,
        ConferenceOrigin::Structured,
    );
    let result = list(
        &[structured],
        None,
        None,
        Some("https://acme.zoom.us/j/1 and https://meet.google.com/aaa-bbbb-ccc"),
    );
    assert_eq!(
        providers(&result),
        [ConferenceProvider::Zoom, ConferenceProvider::Meet]
    );
    assert!(
        result[0].origin == ConferenceOrigin::Structured
            && result[1].origin == ConferenceOrigin::Notes
    );
}

#[test]
fn the_event_url_is_added_only_when_nothing_else_is_found() {
    let alone = list(&[], None, Some("https://example.com/meeting/42"), None);
    assert!(
        alone.len() == 1
            && alone[0].origin == ConferenceOrigin::EventUrl
            && alone[0].provider == ConferenceProvider::Other
    );
    let with_notes = list(
        &[],
        None,
        Some("https://example.com/meeting/42"),
        Some("https://meet.google.com/aaa-bbbb-ccc"),
    );
    assert_eq!(origins(&with_notes), [ConferenceOrigin::Notes]);
}

#[test]
fn a_provider_event_url_is_an_ordinary_url_origin_link() {
    let result = list(&[], None, Some("https://acme.zoom.us/j/1"), None);
    assert!(
        result.len() == 1
            && result[0].origin == ConferenceOrigin::Url
            && result[0].provider == ConferenceProvider::Zoom
    );
}

#[test]
fn every_allowlisted_host_names_its_provider() {
    use ConferenceProvider::*;
    let cases = [
        ("https://acme.zoom.us/j/1", Zoom),
        ("https://meet.google.com/a-b-c", Meet),
        ("https://teams.microsoft.com/l/meetup-join/1", Teams),
        ("https://teams.live.com/meet/1", Teams),
        ("https://acme.webex.com/meet/x", Webex),
        ("https://gotomeet.me/x", GoToMeeting),
        ("https://global.gotomeeting.com/join/1", GoToMeeting),
        ("https://whereby.com/room", Whereby),
        ("https://meet.jit.si/room", Jitsi),
        ("https://app.slack.com/huddle/T1/C1", Slack),
    ];
    for (link, provider) in cases {
        assert_eq!(
            ConferenceDetector::provider_of(&url(link)),
            Some(provider),
            "{link}"
        );
    }
    assert_eq!(
        ConferenceDetector::provider_of(&url("https://example.com/x")),
        None
    );
}

#[test]
fn webex_identity_keeps_the_meeting_id() {
    let a = ConferenceInfo::new(
        url("https://acme.webex.com/acme/j.php?MTID=m111"),
        ConferenceProvider::Webex,
        ConferenceOrigin::Structured,
    );
    let b = ConferenceInfo::new(
        url("https://acme.webex.com/acme/j.php?MTID=m222"),
        ConferenceProvider::Webex,
        ConferenceOrigin::Structured,
    );
    assert_ne!(a.identity(), b.identity());
}

#[test]
fn join_url_policy_accepts_only_safe_meeting_schemes() {
    let allowed = [
        "https://meet.google.com/abc",
        "HTTP://example.com/meeting",
        "zoommtg://acme.zoom.us/join",
    ];
    let rejected = [
        "file:///tmp/payload",
        "javascript:alert(1)",
        "data:text/html,hello",
        "custom://example.com/run",
        "https://user:pass@example.com/meeting",
        "zoommtg://zoom.us.evil.example/j/1",
        "https:///missing-host",
    ];
    for raw in allowed {
        assert!(JoinUrlPolicy::is_allowed(&url(raw)), "{raw}");
    }
    for raw in rejected {
        assert!(!JoinUrlPolicy::is_allowed(&url(raw)), "{raw}");
    }
    assert_eq!(
        ConferenceDetector::provider_of(&url("https://zoom.us.evil.example/j/1")),
        None
    );
}

#[test]
fn invalid_structured_conference_falls_back_to_safe_link() {
    let unsafe_link = ConferenceInfo::new(
        url("file:///tmp/payload"),
        ConferenceProvider::Zoom,
        ConferenceOrigin::Structured,
    );
    let found = list(
        &[unsafe_link],
        None,
        None,
        Some("https://meet.google.com/abc"),
    );
    let urls: Vec<&str> = found.iter().map(|c| c.url.as_str()).collect();
    assert_eq!(urls, ["https://meet.google.com/abc"]);
}

#[test]
fn the_scanner_never_panics_on_odd_text() {
    // Not in the Swift tests: the hand-written scanner replaces a regular expression, so feed it awkward text.
    let pieces = [
        "http",
        "https://",
        "HTTPS://",
        "zoommtg://",
        "://",
        "\u{a0}",
        "\u{2028}",
        "\u{1f600}",
        "é",
        "&amp;",
        "<",
        ">",
        "\"",
        "'",
        ")",
        "]",
        ".",
        ",",
        " ",
        "\n",
        "x",
        "%",
        "%zz",
        "?",
        "#",
        "@",
        "[",
        "ſ",
        "\u{0301}",
    ];
    let mut state = 0x2545_f491_4f6c_dd1du64;
    for _ in 0..2_000 {
        let mut text = String::new();
        for _ in 0..12 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            text.push_str(pieces[(state >> 33) as usize % pieces.len()]);
        }
        let _ = ConferenceDetector::conferences(&[], Some(&text), None, Some(&text));
    }
}
