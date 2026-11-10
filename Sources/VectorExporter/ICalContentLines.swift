import Foundation
@testable import ICalendar

// ical/content-lines.json
//
// Entry points: ICalParser.unfold(_ text: String) -> [String]  (internal, read with @testable),
// ICalParser.parseLine(_ line: String) throws -> ICalProperty  (internal), ICalSerializer.contentLine(_ property:) -> String.
// A case has an "fn".
//
//   fn "unfold":      input { "fn", "text": "<text>" }
//                     expected { "lines": ["<line>"] }
//   fn "parseLine":   input { "fn", "line": "<content line without its line end>" }
//                     expected { "ok": true, "name": "<UPPERCASE>", "parameters": [ { "name": "<UPPERCASE>", "values": ["<unquoted>"] } ],
//                                "value": "<text after the colon>" }
//                            or { "ok": false, "message": "<ICalError.malformed payload>" }
//   fn "contentLine": input { "fn", "name": "<UPPERCASE>", "parameters": [ { "name": "<UPPERCASE>", "values": ["<value>"] } ], "value": "<value>" }
//                     expected { "text": "<name;PARAM=v,w:value>" }
//
// 300 cases: 100 of each. The texts include folded lines (a space or a tab after a CRLF, a bare LF or a bare CR),
// quoted parameters with commas, semicolons and colons, escaped text, non-ASCII and emoji, CRLF, bare LF, a
// combining mark after the colon (Swift reads a Character, so the colon is not found), and malformed lines.
// parseLine returns an ICalProperty, whose constructor uppercases the property name and the parameter names;
// the contentLine inputs are uppercase already.

func exportICalContentLines() -> [VectorCase] {
    var rng = LCG(state: 0x1CA1_C0DE_0004)
    var cases: [VectorCase] = []
    func add(_ input: [String: Any], _ expected: [String: Any]) {
        let fn = input["fn"] as! String
        cases.append(VectorCase(name: "content-\(fn)-\(fourDigits(cases.count + 1))", input: input, expected: expected))
    }
    func unfold(_ text: String) { add(["fn": "unfold", "text": text], ["lines": ICalParser.unfold(text)]) }
    func parseLine(_ line: String) {
        let expected: [String: Any]
        do {
            let p = try ICalParser.parseLine(line)
            expected = ["ok": true, "name": p.name, "parameters": p.parameters.map { ["name": $0.name, "values": $0.values] as [String: Any] }, "value": p.value]
        } catch ICalError.malformed(let message) {
            expected = ["ok": false, "message": message]
        } catch { fatalError("unexpected \(error)") }
        add(["fn": "parseLine", "line": line], expected)
    }
    func contentLine(_ name: String, _ parameters: [(String, [String])], _ value: String) {
        let property = ICalProperty(name: name, parameters: parameters.map { ICalParameter(name: $0.0, values: $0.1) }, value: value)
        add(["fn": "contentLine", "name": name, "parameters": parameters.map { ["name": $0.0, "values": $0.1] as [String: Any] }, "value": value],
            ["text": ICalSerializer.contentLine(property)])
    }

    // unfold: hand-written.
    for text in [
        "", "A:1", "A:1\r\nB:2", "A:1\nB:2", "A:1\rB:2", "A:1\r\n B:2", "A:long\r\n  folded", "A:long\r\n\tfolded", "A:long\n folded\n more",
        "A:1\r\n\r\nB:2", "A:1\r\n", "\r\nA:1", " A:1", "A:1\r\n\r\n x", "A:\u{00E9}\r\n \u{00E8}", "A:\u{1F642}\r\n \u{1F642}", "SUMMARY:caf\u{0065}\u{0301}\r\n \u{0301}",
        "A:1\r\r\nB:2", "A:1\n\rB:2", "A:x\r\n y\r\n z\r\nB:2", "\u{FEFF}A:1", "A:1\u{2028}B:2", "A:1\u{0085}B:2",
    ] { unfold(text) }
    // unfold: generated.
    let pieces = ["BEGIN:VEVENT", "SUMMARY:Planning, part 2", "X-LONG:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                  "DESCRIPTION:caf\u{00E9} \u{1F642} \u{4E2D}\u{6587}", "ATTENDEE;CN=\"Doe, Jane\":mailto:jane@example.test", "END:VEVENT", ""]
    let breaks = ["\r\n", "\n", "\r", "\r\n", "\r\n"]
    while cases.count < 100 {
        var text = ""
        for _ in 0..<rng.int(1...5) {
            var piece = rng.pick(pieces)
            if rng.chance(40), piece.count > 4 {                // fold the piece
                let at = piece.index(piece.startIndex, offsetBy: rng.int(1...(piece.count - 1)))
                piece = String(piece[..<at]) + rng.pick(breaks) + rng.pick([" ", "\t", "  "]) + String(piece[at...])
            }
            text += piece + rng.pick(breaks)
        }
        if rng.chance(25) { text = String(text.dropLast(rng.int(1...2))) }
        unfold(text)
    }

    // parseLine: hand-written.
    for line in [
        "SUMMARY:Planning\\, part 2\\nwith notes", "ATTENDEE;CN=\"Doe, Jane\";ROLE=REQ-PARTICIPANT;DELEGATED-FROM=\"mailto:a@x.test\",\"mailto:b@x.test\":mailto:jane@x.test",
        "URL:https://x.test:8443/a;b", "X-CUSTOM;X-FLAG=1:kept", "DTSTART;TZID=America/New_York:20260102T090000", "DTSTART;VALUE=DATE:20260102",
        "A:", "A::", "A:;,:", "a:lower", "X-A;P=:v", "X-A;P=\"\":v", "X-A;P=\"a;b:c,d\":v", "X-A;P=a,b,c:v", "X-A;P=a;Q=b:v", "X-A;P=\"a\"\"b\":v",
        "SUMMARY:caf\u{00E9} \u{1F642}", "SUMMARY:e\u{0301}", "SUMMARY:\u{0301}x", "SUMMARY:\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
        "SUMMARY\u{0301}:x", "SUMMARY;\u{0301}P=1:x", "A:\u{0301}", "A\u{0301}B:c",
        // malformed
        "", ":", ":v", ";P=1:v", "NOCOLON", "A;P", "A;P=", "A;=v:x", "A;P=\"unterminated:v", "A;P=v", "A;P=v;Q", "A;P=\"x\"y:v", "A;:v", "A;P=1,:v",
    ] { parseLine(line) }
    // parseLine: generated.
    let names = ["SUMMARY", "DESCRIPTION", "ATTENDEE", "ORGANIZER", "DTSTART", "DTEND", "RRULE", "EXDATE", "X-WR-CALNAME", "X-APPLE-STRUCTURED-LOCATION", "Location", "uid"]
    let paramNames = ["CN", "TZID", "VALUE", "ROLE", "PARTSTAT", "DELEGATED-FROM", "X-NUM-GUESTS", "LANGUAGE", "SENT-BY"]
    let paramValues = ["Doe, Jane", "America/New_York", "DATE", "REQ-PARTICIPANT", "mailto:a@x.test", "a;b", "plain", "caf\u{00E9}", "\u{1F642}", "x:y", "", "1"]
    let values = ["", "Planning\\, part 2", "mailto:jane@example.test", "20260102T090000Z", "FREQ=WEEKLY;BYDAY=MO,WE", "https://example.test:8443/a;b?q=1,2",
                  "caf\u{00E9} \u{1F642}", "a:b:c", "a;b;c", "\u{4E2D}\u{6587}", "x\\ny"]
    while cases.count < 200 {
        var line = rng.pick(names)
        for _ in 0..<rng.below(4) {
            line += ";" + rng.pick(paramNames) + "="
            line += (0..<rng.int(1...3)).map { _ -> String in
                let v = rng.pick(paramValues)
                return (v.contains(where: { ";:,".contains($0) }) || rng.chance(15)) ? "\"\(v)\"" : v
            }.joined(separator: ",")
        }
        line += ":" + rng.pick(values)
        parseLine(line)
    }

    // contentLine.
    contentLine("SUMMARY", [], "Planning")
    contentLine("ATTENDEE", [("CN", ["Doe, Jane"]), ("ROLE", ["REQ-PARTICIPANT"])], "mailto:jane@x.test")
    contentLine("ATTENDEE", [("DELEGATED-FROM", ["mailto:a@x.test", "mailto:b@x.test"])], "mailto:jane@x.test")
    contentLine("X-A", [("P", [""])], "")
    contentLine("X-A", [("P", ["a;b"]), ("Q", ["c:d"]), ("R", ["e,f"])], "v")
    contentLine("X-A", [("P", [])], "v")
    while cases.count < 300 {
        let params: [(String, [String])] = (0..<rng.below(4)).map { _ in
            (rng.pick(paramNames), (0..<rng.int(1...3)).map { _ in rng.pick(paramValues) })
        }
        contentLine(rng.pick(names).uppercased(), params, rng.pick(values))
    }
    return cases
}
