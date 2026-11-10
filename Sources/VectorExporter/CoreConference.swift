import CalendarCore
import Foundation

// core/conference.json
//
// Entry points: ConferenceDetector.provider(of:), ConferenceDetector.conferences(structured:location:url:notes:),
// ConferenceDetector.detect(location:url:notes:) and JoinURLPolicy.isAllowed(_:). A case has an "fn". Every URL
// field is text; as in ConferenceDetectorTests, a url that URL(string:) rejects is passed as nil (the exporter
// only writes urls that URL(string:) accepts, so this never happens in the file).
//
//   fn "provider":    input { "fn", "url": "<text>" }                       expected { "provider": "<ConferenceProvider raw value>" | null }
//   fn "isAllowed":   input { "fn", "url": "<text>" }                       expected { "allowed": <Bool> }
//   fn "detect":      input { "fn", "location": <text> | null, "url": <text> | null, "notes": <text> | null }
//                                                                           expected { "url": "<absoluteString>" | null }
//   fn "conferences": input { "fn", "structured": [ { "url", "provider", "origin" } ], "location", "url", "notes" }
//                                                                           expected { "conferences": [ { "url": "<absoluteString>", "provider", "origin" } ] }
//
// Provider raw values: meet, teams, zoom, webex, goToMeeting, whereby, jitsi, slack, other. Origin raw values:
// structured, location, url, notes, eventURL. 400 cases: 60 provider, 100 isAllowed, 100 detect, 140 conferences,
// taken from the strings in ConferenceDetectorTests plus generated variations (Outlook SafeLinks and Google
// redirect wrappers, trailing punctuation, upper-case hosts and schemes, HTML with &amp;, lookalike hosts).
// The hosts are examples (acme.zoom.us, example.com); there are no real meeting links.

private func percentEncode(_ text: String) -> String {
    var out = ""
    for byte in text.utf8 {
        let c = Character(UnicodeScalar(byte))
        if c.isASCII, c.isLetter || c.isNumber || "-._~".contains(c) { out.append(c) } else { out += String(format: "%%%02X", byte) }
    }
    return out
}

func exportCoreConference() -> [VectorCase] {
    var rng = LCG(state: 0xC0FF_E000_0007)
    var cases: [VectorCase] = []

    func conferenceJSON(_ c: ConferenceInfo) -> [String: Any] { ["url": c.url.absoluteString, "provider": c.provider.rawValue, "origin": c.origin.rawValue] }
    func add(_ fn: String, _ input: [String: Any], _ expected: [String: Any]) {
        var input = input
        input["fn"] = fn
        cases.append(VectorCase(name: "conference-\(fn)-\(fourDigits(cases.count + 1))", input: input, expected: expected))
    }
    func provider(_ text: String) {
        guard let url = URL(string: text) else { return }
        add("provider", ["url": text], ["provider": orNull(ConferenceDetector.provider(of: url)?.rawValue)])
    }
    func allowed(_ text: String) {
        guard let url = URL(string: text) else { return }
        add("isAllowed", ["url": text], ["allowed": JoinURLPolicy.isAllowed(url)])
    }
    func detect(location: String? = nil, url: String? = nil, notes: String? = nil) {
        if let url, URL(string: url) == nil { return }
        add("detect", ["location": orNull(location), "url": orNull(url), "notes": orNull(notes)],
            ["url": orNull(ConferenceDetector.detect(location: location, url: url.flatMap(URL.init(string:)), notes: notes)?.absoluteString)])
    }
    func conferences(structured: [(String, ConferenceProvider)] = [], location: String? = nil, url: String? = nil, notes: String? = nil) {
        if let url, URL(string: url) == nil { return }
        let infos = structured.compactMap { text, p in URL(string: text).map { ConferenceInfo(url: $0, provider: p) } }
        let list = ConferenceDetector.conferences(structured: infos, location: location, url: url.flatMap(URL.init(string:)), notes: notes)
        add("conferences",
            ["structured": infos.map(conferenceJSON), "location": orNull(location), "url": orNull(url), "notes": orNull(notes)],
            ["conferences": list.map(conferenceJSON)])
    }

    let providerLinks = [
        "https://acme.zoom.us/j/123456789?pwd=abc", "https://zoom.us/j/1", "https://us02web.zoom.com/j/42", "https://meet.google.com/abc-defg-hij",
        "https://teams.microsoft.com/l/meetup-join/19%3ameeting?a=1&b=2", "https://teams.live.com/meet/1", "https://acme.webex.com/meet/x",
        "https://acme.webex.com/acme/j.php?MTID=m111", "https://gotomeet.me/x", "https://global.gotomeeting.com/join/1", "https://whereby.com/room",
        "https://meet.jit.si/room", "https://app.slack.com/huddle/T1/C1", "zoommtg://zoom.us/join?confno=123", "zoommtg://acme.zoom.us/join",
    ]
    let notProviderLinks = [
        "https://docs.google.com/document/d/abc", "https://example.com/meeting/42", "https://app.slack.com/client/T1/C1", "https://app.slack.com/huddlefoo/x",
        "https://zoom.us.evil.example/j/1", "https://evilzoom.us/j/1", "https://notmeet.google.com.example/abc", "zoommtg://evil.com/join?confno=1",
        "zoommtg://zoom.us.evil.example/j/1", "https://www.google.com/calendar/event?eid=abc123", "https://calendar.google.com/calendar/u/0/r/eventedit/abc123",
        "https://evilsafelinks.protection.outlook.com/?url=https%3A%2F%2Fmeet.google.com%2Fa-b-c", "ftp://acme.zoom.us/j/1", "https://example.com/zoom.us/j/1",
    ]
    let unsafeLinks = ["file:///tmp/payload", "javascript:alert(1)", "data:text/html,hello", "custom://example.com/run", "https://user:pass@example.com/meeting",
                       "https:///missing-host", "HTTP://example.com/meeting", "HTTPS://MEET.GOOGLE.COM/abc-defg-hij", "http://localhost:8080/", "mailto:a@example.test",
                       "tel:+15555550100", "https://user@example.com/", "zoommtg://user@zoom.us/join", "ZOOMMTG://ZOOM.US/join", "https://[::1]/x", "https://127.0.0.1/x"]

    // provider.
    for link in providerLinks + notProviderLinks + unsafeLinks { provider(link) }
    while cases.count < 60 {
        let base = rng.pick(providerLinks)
        provider(rng.chance(50) ? base.uppercased() : base.replacingOccurrences(of: "acme.", with: rng.pick(["a.b.", "", "x-y."])))
    }

    // isAllowed.
    for link in providerLinks + notProviderLinks + unsafeLinks { allowed(link) }
    while cases.count < 60 + 100 {
        let base = rng.pick(providerLinks + notProviderLinks + unsafeLinks)
        allowed(rng.pick([base, base.uppercased(), base + "/extra", base.replacingOccurrences(of: "://", with: "://u:p@"), base + "#frag", base + "?q=1"]))
    }

    // detect: the tests' inputs first.
    detect(location: "https://acme.zoom.us/j/123456789?pwd=abc")
    detect(notes: "Join: https://meet.google.com/abc-defg-hij.")
    detect(notes: #"<p><a href="https://teams.microsoft.com/l/meetup-join/19%3ameeting?a=1&amp;b=2">Join</a></p>"#)
    detect(notes: "Click https://nam02.safelinks.protection.outlook.com/?url=https%3A%2F%2Fteams.microsoft.com%2Fl%2Fmeetup-join%2F0&data=05 now")
    detect(notes: "https://www.google.com/url?q=https://acme.zoom.us/j/1&sa=D")
    detect(notes: "zoommtg://zoom.us/join?confno=123")
    detect(notes: "Agenda: https://docs.google.com/document/d/abc")
    detect(location: "https://meet.google.com/aaa-bbbb-ccc", notes: "https://acme.zoom.us/j/1")
    detect(notes: "Doc https://docs.google.com/x then https://acme.zoom.us/j/9")
    detect(url: "https://example.com/meeting/42")
    detect(url: "https://www.google.com/calendar/event?eid=abc123")
    detect(url: "https://calendar.google.com/calendar/u/0/r/eventedit/abc123")
    detect(url: "https://www.google.com/url?q=https://acme.zoom.us/j/1&sa=D")
    detect(notes: "https://app.slack.com/huddle/T1/C1")
    detect(notes: "https://app.slack.com/client/T1/C1")
    detect(location: "Room 4", notes: "Bring laptop")
    detect(notes: "zoommtg://evil.com/join?confno=1")
    detect(notes: "https://evilsafelinks.protection.outlook.com/?url=https%3A%2F%2Fmeet.google.com%2Fa-b-c")
    detect(notes: "https://app.slack.com/huddlefoo/x")
    detect(notes: "HTTPS://MEET.GOOGLE.COM/abc-defg-hij")
    detect(); detect(location: "", url: "", notes: "")

    // Generated texts around a link.
    func wrap(_ link: String) -> String {
        switch rng.below(9) {
        case 0: return "https://nam02.safelinks.protection.outlook.com/?url=\(percentEncode(link))&data=05%7C01"
        case 1: return "https://www.google.com/url?q=\(link)&sa=D"
        case 2: return "https://www.google.com/url?url=\(percentEncode(link))&sa=D"
        case 3: return link + rng.pick([".", ",", ";", ":", "!", "?", ")", ".)", "]"])
        case 4: return "<a href=\"\(link.replacingOccurrences(of: "&", with: "&amp;"))\">Join</a>"
        case 5: return link.uppercased()
        case 6: return "(\(link))"
        case 7: return "https://nam02.safelinks.protection.outlook.com/?url=\(percentEncode("https://nam02.safelinks.protection.outlook.com/?url=\(percentEncode(link))"))"
        default: return link
        }
    }
    func sentence(_ link: String) -> String {
        rng.pick(["Join: \(link)", "\(link) is the room", "Meeting at \(link) today.", "see\n\(link)\nfor details", "<p>\(link)</p>", "a \(link) b \(rng.pick(providerLinks)) c", link])
    }
    let providerOrNot = providerLinks + providerLinks + notProviderLinks
    while cases.count < 60 + 100 + 100 {
        let link = wrap(rng.pick(providerOrNot))
        switch rng.below(3) {
        case 0: detect(location: sentence(link))
        case 1: detect(notes: sentence(link))
        default: detect(location: rng.chance(30) ? "Room 4" : nil, url: rng.chance(50) ? rng.pick(providerOrNot) : nil, notes: sentence(link))
        }
    }

    // conferences: the tests' inputs first.
    conferences(location: "https://meet.google.com/aaa-bbbb-ccc", notes: "or https://acme.zoom.us/j/9 and https://acme.webex.com/meet/x")
    conferences(location: "https://acme.zoom.us/j/9", notes: "https://ACME.zoom.us/j/9/ again")
    conferences(structured: [("https://acme.zoom.us/j/1", .zoom)], notes: "https://acme.zoom.us/j/1 and https://meet.google.com/aaa-bbbb-ccc")
    conferences(url: "https://example.com/meeting/42")
    conferences(url: "https://example.com/meeting/42", notes: "https://meet.google.com/aaa-bbbb-ccc")
    conferences(url: "https://acme.zoom.us/j/1")
    conferences(structured: [("file:///tmp/payload", .zoom)], notes: "https://meet.google.com/abc")
    conferences(structured: [("https://acme.webex.com/acme/j.php?MTID=m111", .webex), ("https://acme.webex.com/acme/j.php?MTID=m222", .webex)])
    conferences(structured: [("https://acme.webex.com/acme/j.php?MTID=m111", .webex)], notes: "https://acme.webex.com/acme/j.php?MTID=m111 https://acme.webex.com/acme/j.php?MTID=m222")
    conferences(); conferences(location: "", notes: "")
    while cases.count < 400 {
        let structured: [(String, ConferenceProvider)] = rng.chance(25) ? [(rng.pick(providerLinks), .other)] : []
        let loc: String? = rng.chance(60) ? sentence(wrap(rng.pick(providerOrNot))) : nil
        let notes: String? = rng.chance(70) ? sentence(wrap(rng.pick(providerOrNot))) : nil
        let url: String? = rng.chance(35) ? rng.pick(providerOrNot) : nil
        conferences(structured: structured, location: loc, url: url, notes: notes)
    }
    return cases
}
