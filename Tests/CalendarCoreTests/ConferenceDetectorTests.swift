import Foundation
import Testing
@testable import CalendarCore

private func detect(location: String? = nil, url: String? = nil, notes: String? = nil) -> URL? {
    ConferenceDetector.detect(location: location, url: url.flatMap(URL.init(string:)), notes: notes)
}

@Test func findsZoomInLocation() {
    #expect(detect(location: "https://acme.zoom.us/j/123456789?pwd=abc")?.host == "acme.zoom.us")
}

@Test func findsMeetInNotes() {
    #expect(detect(notes: "Join: https://meet.google.com/abc-defg-hij.")?.absoluteString
        == "https://meet.google.com/abc-defg-hij")
}

@Test func findsTeamsInsideHTMLWithEntities() {
    let notes = #"<p><a href="https://teams.microsoft.com/l/meetup-join/19%3ameeting?a=1&amp;b=2">Join</a></p>"#
    let result = detect(notes: notes)
    #expect(result?.host == "teams.microsoft.com")
    #expect(result?.query == "a=1&b=2")
}

@Test func unwrapsOutlookSafeLinks() {
    let wrapped = "https://nam02.safelinks.protection.outlook.com/?url=https%3A%2F%2Fteams.microsoft.com%2Fl%2Fmeetup-join%2F0&data=05"
    #expect(detect(notes: "Click \(wrapped) now")?.host == "teams.microsoft.com")
}

@Test func unwrapsGoogleRedirects() {
    let wrapped = "https://www.google.com/url?q=https://acme.zoom.us/j/1&sa=D"
    #expect(detect(notes: wrapped)?.host == "acme.zoom.us")
}

@Test func acceptsZoomMtgScheme() {
    #expect(detect(notes: "zoommtg://zoom.us/join?confno=123")?.scheme == "zoommtg")
}

@Test func ignoresNonProviderLinksInText() {
    #expect(detect(notes: "Agenda: https://docs.google.com/document/d/abc") == nil)
}

@Test func locationBeatsNotes() {
    let result = detect(location: "https://meet.google.com/aaa-bbbb-ccc",
                        notes: "https://acme.zoom.us/j/1")
    #expect(result?.host == "meet.google.com")
}

@Test func returnsFirstProviderLinkSkippingOthers() {
    let notes = "Doc https://docs.google.com/x then https://acme.zoom.us/j/9"
    #expect(detect(notes: notes)?.host == "acme.zoom.us")
}

@Test func eventURLFallbackWhenNotAllowlisted() {
    #expect(detect(url: "https://example.com/meeting/42")?.absoluteString == "https://example.com/meeting/42")
}

@Test func eventURLFallbackExcludesGoogleCalendarsOwnPermalink() {
    // Google sets an event's `url` to a link back to its own web UI (the API's `htmlLink`) on every event,
    // even one with no real conference; that is never a join link, so it must not win the raw-url fallback.
    #expect(detect(url: "https://www.google.com/calendar/event?eid=abc123") == nil)
    #expect(detect(url: "https://calendar.google.com/calendar/u/0/r/eventedit/abc123") == nil)
}

@Test func googleRedirectInsideEventURLStillUnwrapsToARealProvider() {
    // The exclusion only guards the raw fallback: a genuine www.google.com/url redirect to a
    // provider (as Google sometimes wraps links in notes) is still found via the text scan.
    #expect(detect(url: "https://www.google.com/url?q=https://acme.zoom.us/j/1&sa=D")?.host == "acme.zoom.us")
}

@Test func slackHuddleNeedsHuddlePath() {
    #expect(detect(notes: "https://app.slack.com/huddle/T1/C1") != nil)
    #expect(detect(notes: "https://app.slack.com/client/T1/C1") == nil)
}

@Test func noLinksReturnsNil() {
    #expect(detect(location: "Room 4", notes: "Bring laptop") == nil)
}

// ISSUE 1: zoommtg URL host validation
@Test func zoommtgRequiresZoomDomain() {
    // Evil domain should not match
    #expect(detect(notes: "zoommtg://evil.com/join?confno=1") == nil)
}

@Test func zoommtgZoomUsStillWorks() {
    // Legitimate zoom.us should still work
    #expect(detect(notes: "zoommtg://zoom.us/join?confno=123")?.scheme == "zoommtg")
}

// ISSUE 2: SafeLinks host boundary validation
@Test func safelinksRequiresDotBoundary() {
    // Evil subdomain should not be unwrapped
    let evilSafelinks = "https://evilsafelinks.protection.outlook.com/?url=https%3A%2F%2Fmeet.google.com%2Fa-b-c"
    #expect(detect(notes: evilSafelinks) == nil)
}

// ISSUE 3: Slack huddle path prefix validation
@Test func slackHuddlePathMustBeExact() {
    // /huddlefoo should not match
    #expect(detect(notes: "https://app.slack.com/huddlefoo/x") == nil)
}

@Test func slackHuddlePathStillWorksWithSlash() {
    // /huddle/ should still match
    #expect(detect(notes: "https://app.slack.com/huddle/T1/C1") != nil)
}

// ISSUE 4: Case-insensitive regex
@Test func upperCaseURLSchemeAndHostMatch() {
    let result = detect(notes: "HTTPS://MEET.GOOGLE.COM/abc-defg-hij")
    #expect(result != nil)
    #expect(result?.host?.lowercased() == "meet.google.com")
}

// The list API.

private func list(structured: [ConferenceInfo] = [], location: String? = nil, url: String? = nil, notes: String? = nil) -> [ConferenceInfo] {
    ConferenceDetector.conferences(structured: structured, location: location, url: url.flatMap(URL.init(string:)), notes: notes)
}

@Test func keepsEveryProviderLinkInOrderWithItsOrigin() {
    let result = list(location: "https://meet.google.com/aaa-bbbb-ccc", notes: "or https://acme.zoom.us/j/9 and https://acme.webex.com/meet/x")
    #expect(result.map(\.provider) == [.meet, .zoom, .webex])
    #expect(result.map(\.origin) == [.location, .notes, .notes])
}

@Test func duplicatesAcrossFieldsAreRemovedFirstWins() {
    let result = list(location: "https://acme.zoom.us/j/9", notes: "https://ACME.zoom.us/j/9/ again")
    #expect(result.count == 1 && result[0].origin == .location)
}

@Test func structuredLinksComeFirstAndAreNotRepeatedByTheTextScan() {
    let structured = ConferenceInfo(url: URL(string: "https://acme.zoom.us/j/1")!, provider: .zoom)
    let result = list(structured: [structured], notes: "https://acme.zoom.us/j/1 and https://meet.google.com/aaa-bbbb-ccc")
    #expect(result.map(\.provider) == [.zoom, .meet])
    #expect(result[0].origin == .structured && result[1].origin == .notes)
}

@Test func theEventURLIsAddedOnlyWhenNothingElseIsFound() {
    let alone = list(url: "https://example.com/meeting/42")
    #expect(alone.count == 1 && alone[0].origin == .eventURL && alone[0].provider == .other)
    #expect(list(url: "https://example.com/meeting/42", notes: "https://meet.google.com/aaa-bbbb-ccc").map(\.origin) == [.notes])
}

@Test func aProviderEventURLIsAnOrdinaryUrlOriginLink() {
    let result = list(url: "https://acme.zoom.us/j/1")
    #expect(result.count == 1 && result[0].origin == .url && result[0].provider == .zoom)
}

@Test func everyAllowlistedHostNamesItsProvider() {
    let cases: [(String, ConferenceProvider)] = [
        ("https://acme.zoom.us/j/1", .zoom), ("https://meet.google.com/a-b-c", .meet),
        ("https://teams.microsoft.com/l/meetup-join/1", .teams), ("https://teams.live.com/meet/1", .teams),
        ("https://acme.webex.com/meet/x", .webex), ("https://gotomeet.me/x", .goToMeeting),
        ("https://global.gotomeeting.com/join/1", .goToMeeting), ("https://whereby.com/room", .whereby),
        ("https://meet.jit.si/room", .jitsi), ("https://app.slack.com/huddle/T1/C1", .slack),
    ]
    for (link, provider) in cases { #expect(ConferenceDetector.provider(of: URL(string: link)!) == provider) }
    #expect(ConferenceDetector.provider(of: URL(string: "https://example.com/x")!) == nil)
}

@Test func webexIdentityKeepsTheMeetingID() {
    let a = ConferenceInfo(url: URL(string: "https://acme.webex.com/acme/j.php?MTID=m111")!, provider: .webex)
    let b = ConferenceInfo(url: URL(string: "https://acme.webex.com/acme/j.php?MTID=m222")!, provider: .webex)
    #expect(a.identity != b.identity)
}
