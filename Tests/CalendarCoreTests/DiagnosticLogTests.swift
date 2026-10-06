import CalendarCore
import CalendarTestSupport
import Foundation
import Testing

private let epoch = Date(timeIntervalSince1970: 1_800_000_000)

private func event(_ name: String, fields: [DiagnosticField] = [], level: DiagnosticLevel = .info) -> DiagnosticEvent {
    DiagnosticEvent(level: level, category: "test", name: name, fields: fields, date: epoch)
}

@Test func levelsAreOrdered() {
    let levels: [DiagnosticLevel] = [.debug, .info, .notice, .warning, .error]
    #expect(levels == levels.sorted() && levels.first! < levels.last!)
}

@Test func stringFieldsArePrivateByDefaultAndNumbersAreNot() {
    #expect(DiagnosticField.string("uid", "abc").isPrivate)
    #expect(!DiagnosticField.string("host", "www.example.test", private: false).isPrivate)
    #expect(!DiagnosticField.int("events", 12).isPrivate)
    #expect(!DiagnosticField.bool("ok", true).isPrivate)
    #expect(!DiagnosticField.double("seconds", 1.5).isPrivate)
    #expect(DiagnosticField.int("events", 12).value == .int(12))
}

@Test func theRingBufferKeepsTheLastEventsOldestFirst() {
    let log = RingBufferDiagnosticLog(capacity: 3)
    for index in 1...5 { log.record(event("e\(index)")) }
    #expect(log.snapshot().map(\.name) == ["e3", "e4", "e5"])
    log.clear()
    #expect(log.snapshot().isEmpty)
}

@Test func theRingBufferDefaultsToFiveHundred() {
    let log = RingBufferDiagnosticLog()
    for index in 0..<520 { log.record(event("e\(index)")) }
    let names = log.snapshot().map(\.name)
    #expect(names.count == 500 && names.first == "e20" && names.last == "e519")
}

@Test func theRingBufferSurvivesConcurrentWriters() async {
    let log = RingBufferDiagnosticLog(capacity: 100)
    await withTaskGroup(of: Void.self) { group in
        for task in 0..<8 {
            group.addTask {
                for index in 0..<250 { log.record(event("t\(task)-\(index)")) }
                _ = log.snapshot()
            }
        }
    }
    #expect(log.snapshot().count == 100)
}

@Test func renderingHidesPrivateFieldsUnlessAsked() {
    let log = RingBufferDiagnosticLog()
    log.record(event("rruleUnreadable", fields: [.string("uid", "secret@example.test"), .int("n", 2), .string("host", "www.example.test", private: false)], level: .notice))
    let hidden = log.render()
    #expect(hidden == "2027-01-15T08:00:00Z notice test.rruleUnreadable uid=<private> n=2 host=www.example.test")
    #expect(!hidden.contains("secret@example.test"))
    #expect(log.render(includePrivate: true).contains("uid=secret@example.test"))
}

@Test func renderingIsOneLinePerEventAndQuotesAwkwardStrings() {
    let log = RingBufferDiagnosticLog()
    log.record(event("a", fields: [.string("note", "two words\nnext", private: false)]))
    log.record(event("b"))
    let lines = log.render().split(separator: "\n")
    #expect(lines.count == 2)
    #expect(lines[0].hasSuffix("test.a note=\"two words\\nnext\""))
}

@Test func fanOutSendsToEveryLogAndNullDiscards() {
    let first = CollectingDiagnosticLog(), second = RingBufferDiagnosticLog()
    let fan = FanOutDiagnosticLog([first, second, .none])
    fan.record(event("x"))
    fan.record(.info, "test", "y", [.int("n", 1)], date: epoch)
    #expect(first.events.map(\.name) == ["x", "y"] && second.snapshot().map(\.name) == ["x", "y"])
    #expect(first.events[1].field("n")?.value == .int(1) && first.events[1].date == epoch)
}
