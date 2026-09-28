import CalendarCore
import Foundation
import Testing
@testable import ICalendar

private func alarm(_ lines: [String]) throws -> ICalComponent {
    try ICalParser.parse((["BEGIN:VALARM"] + lines + ["END:VALARM"]).joined(separator: "\r\n"))
}

@Test func readsRelativeAbsoluteRepeatingAndEndAlarms() throws {
    let reminders = AlarmMapper.reminders(from: [
        try alarm(["ACTION:DISPLAY", "TRIGGER:-PT10M"]),
        try alarm(["ACTION:AUDIO", "TRIGGER;RELATED=END:PT0S", "ATTACH;VALUE=URI:Chord"]),
        try alarm(["ACTION:DISPLAY", "TRIGGER;VALUE=DATE-TIME:20260927T150000Z", "REPEAT:2", "DURATION:PT5M"]),
        try alarm(["ACTION:EMAIL", "TRIGGER:-P1D", "ATTENDEE:mailto:me@x.test"]),
        try alarm(["ACTION:DISPLAY", "TRIGGER:not-a-duration"]),
    ])
    #expect(reminders == [
        Reminder(trigger: .relative(offset: -600, to: .start), isCalendarDefault: false),
        Reminder(trigger: .relative(offset: 0, to: .end), type: .audio(soundName: "Chord"), isCalendarDefault: false),
        Reminder(trigger: .absolute(Date(timeIntervalSince1970: 1_790_521_200)), repeatCount: 2, repeatInterval: 300, isCalendarDefault: false),
        Reminder(trigger: .relative(offset: -86_400, to: .start), type: .email(address: "me@x.test"), isCalendarDefault: false),
    ])
}

@Test func readsAppleProximityAlarms() throws {
    let reminders = AlarmMapper.reminders(from: [try alarm([
        "ACTION:DISPLAY", "TRIGGER;VALUE=DATE-TIME:19760401T005545Z", "X-APPLE-PROXIMITY:ARRIVE",
        "X-APPLE-STRUCTURED-LOCATION;VALUE=URI;X-APPLE-RADIUS=100;X-TITLE=Office:geo:37.33,-122.03",
    ])])
    #expect(reminders == [Reminder(trigger: .location(StructuredLocation(title: "Office", latitude: 37.33, longitude: -122.03, radius: 100), .enter),
                                   isCalendarDefault: false)])
}
