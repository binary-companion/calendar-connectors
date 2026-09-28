import CalendarCore
import Foundation
import Testing
@testable import ICalendar

private let sample = """
BEGIN:VCALENDAR\r
VERSION:2.0\r
PRODID:-//Example//EN\r
BEGIN:VEVENT\r
UID:abc-123\r
SUMMARY:Planning\\, part 2\\nwith notes\r
ATTENDEE;CN="Doe, Jane";ROLE=REQ-PARTICIPANT;DELEGATED-FROM="mailto:a@x.test","mailto:b@x.test":mailto:jane@x.test\r
URL:https://x.test:8443/a;b\r
X-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC\r
X-CUSTOM;X-FLAG=1:kept\r
BEGIN:VALARM\r
ACTION:DISPLAY\r
TRIGGER:-PT15M\r
END:VALARM\r
END:VEVENT\r
END:VCALENDAR\r

"""

@Test func parsesNestedComponents() throws {
    let root = try ICalParser.parse(sample)
    #expect(root.name == "VCALENDAR")
    let event = try #require(root.components(named: "VEVENT").first)
    #expect(event.property("UID")?.value == "abc-123")
    #expect(event.components(named: "VALARM").first?.property("TRIGGER")?.value == "-PT15M")
}

@Test func parsesQuotedParametersAndMultipleValues() throws {
    let event = try #require(try ICalParser.parse(sample).components(named: "VEVENT").first)
    let attendee = try #require(event.property("ATTENDEE"))
    #expect(attendee.parameter("CN") == "Doe, Jane")
    #expect(attendee.parameters.first { $0.name == "DELEGATED-FROM" }?.values == ["mailto:a@x.test", "mailto:b@x.test"])
    #expect(attendee.value == "mailto:jane@x.test")
}

@Test func valueMayContainColonsAndSemicolons() throws {
    let event = try #require(try ICalParser.parse(sample).components(named: "VEVENT").first)
    #expect(event.property("URL")?.value == "https://x.test:8443/a;b")
}

@Test func unescapesText() throws {
    let event = try #require(try ICalParser.parse(sample).components(named: "VEVENT").first)
    #expect(event.property("SUMMARY")?.text == "Planning, part 2\nwith notes")
    #expect(ICalText.escape("a;b,c\\d\ne") == "a\\;b\\,c\\\\d\\ne")
    #expect(ICalText.unescape(ICalText.escape("x;y,z\\\n")) == "x;y,z\\\n")
}

@Test func foldsAt75OctetsWithoutSplittingUTF8() throws {
    let long = String(repeating: "é🙂a", count: 40)
    let component = ICalComponent(name: "VEVENT", properties: [ICalProperty(name: "SUMMARY", text: long)])
    let text = ICalSerializer.serialize(component)
    for line in text.components(separatedBy: "\r\n") { #expect(line.utf8.count <= 75) }
    #expect(text.hasSuffix("END:VEVENT\r\n"))
    #expect(try ICalParser.parse(text).property("SUMMARY")?.text == long)
}

@Test func unfoldsAFoldThatSplitsAUTF8Sequence() throws {
    // "é" is C3 A9; a careless server folds between the two bytes.
    var data = Data("BEGIN:VEVENT\r\nSUMMARY:caf".utf8)
    data.append(0xC3)
    data.append(contentsOf: Array("\r\n ".utf8))
    data.append(0xA9)
    data.append(contentsOf: Array("\r\nEND:VEVENT\r\n".utf8))
    #expect(try ICalParser.parse(data).property("SUMMARY")?.text == "café")
}

@Test func parsesLFOnlyAndMissingFinalNewline() throws {
    let text = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\nSUMMARY:long\n  folded\nEND:VEVENT\nEND:VCALENDAR"
    let event = try #require(try ICalParser.parse(text).components(named: "VEVENT").first)
    #expect(event.property("SUMMARY")?.text == "long folded")
}

@Test func roundTripKeepsUnknownPropertiesAndParameters() throws {
    let first = try ICalParser.parse(sample)
    let again = try ICalParser.parse(ICalSerializer.serialize(first))
    #expect(again == first)
    let event = try #require(again.components(named: "VEVENT").first)
    #expect(event.property("X-CUSTOM")?.parameter("X-FLAG") == "1")
    #expect(event.property("X-APPLE-TRAVEL-ADVISORY-BEHAVIOR")?.value == "AUTOMATIC")
}

@Test func rejectsUnbalancedAndUnterminatedInput() {
    #expect(throws: ICalError.self) { try ICalParser.parse("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nEND:VCALENDAR\r\n") }
    #expect(throws: ICalError.self) { try ICalParser.parse("BEGIN:VCALENDAR\r\n") }
    #expect(throws: ICalError.self) { try ICalParser.parse("SUMMARY:outside\r\n") }
}

@Test func setReplacesAtTheFirstPosition() {
    var component = ICalComponent(name: "VEVENT", properties: [
        ICalProperty(name: "UID", value: "1"), ICalProperty(name: "SUMMARY", value: "a"),
        ICalProperty(name: "LOCATION", value: "x"), ICalProperty(name: "SUMMARY", value: "b"),
    ])
    component.set(ICalProperty(name: "summary", value: "c"))
    #expect(component.properties.map(\.name) == ["UID", "SUMMARY", "LOCATION"])
    #expect(component.property("SUMMARY")?.value == "c")
    component.setText("LOCATION", nil)
    #expect(component.property("LOCATION") == nil)
}
