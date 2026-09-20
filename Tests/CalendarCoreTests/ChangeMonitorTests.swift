import Foundation
import Testing
@testable import CalendarCore

private actor Script {
    private var results: [Result<CalendarChange?, Error>]
    private(set) var calls = 0
    init(_ results: [Result<CalendarChange?, Error>]) { self.results = results }
    func next() throws -> CalendarChange? {
        calls += 1
        guard !results.isEmpty else { throw CancellationError() }
        return try results.removeFirst().get()
    }
}

private struct ScriptedSource: PollingCalendarSource {
    let script: Script
    var id: String { "scripted" }
    var displayName: String { "Scripted" }
    var capabilities: SourceCapabilities { SourceCapabilities(syncKind: .token) }
    func calendars() async throws -> [CalendarDescriptor] { [] }
    func events(in interval: DateInterval) async throws -> [CalendarEvent] { [] }
    func changes() -> AsyncStream<CalendarChange> { AsyncStream { $0.finish() } }
    func checkForChanges() async throws -> CalendarChange? { try await script.next() }
}

private actor SleepLog {
    private(set) var durations: [Duration] = []
    func record(_ d: Duration) { durations.append(d) }
}

private func run(_ results: [Result<CalendarChange?, Error>], monitor: (@escaping @Sendable (Duration) async -> Void) -> ChangeMonitor)
    async -> (changes: [CalendarChange], sleeps: [Duration])
{
    let log = SleepLog()
    let m = monitor { await log.record($0) }
    var changes: [CalendarChange] = []
    for await change in m.changes(polling: ScriptedSource(script: Script(results))) { changes.append(change) }
    return (changes, await log.durations)
}

private func standard(_ record: @escaping @Sendable (Duration) async -> Void) -> ChangeMonitor {
    ChangeMonitor(interval: .seconds(60), maxBackoff: .seconds(900), sleep: { await record($0) })
}

@Test func yieldsChangesAndSleepsTheIntervalBetweenChecks() async {
    let result = await run([.success(nil), .success(.eventsChanged(calendarIDs: ["a"])), .success(.calendarsChanged)], monitor: standard)
    #expect(result.changes == [.eventsChanged(calendarIDs: ["a"]), .calendarsChanged])
    #expect(result.sleeps == [.seconds(60), .seconds(60), .seconds(60)])
}

@Test func backsOffExponentiallyThenResetsOnSuccess() async {
    let boom = SourceError.network("down")
    let result = await run([.failure(boom), .failure(boom), .success(nil), .success(nil)], monitor: standard)
    #expect(result.sleeps == [.seconds(120), .seconds(240), .seconds(60), .seconds(60)])
}

@Test func backoffIsCappedAndNeverOverflows() async {
    let boom = SourceError.server(status: 503)
    let result = await run(Array(repeating: .failure(boom), count: 5000), monitor: standard)
    #expect(result.sleeps.count == 5000)
    #expect(result.sleeps.max() == .seconds(900))
    #expect(result.sleeps.last == .seconds(900))
}

@Test func authExpiredYieldsSourceFailedAndFinishesWithoutSleeping() async {
    let result = await run([.failure(SourceError.authExpired)], monitor: standard)
    #expect(result.changes == [.sourceFailed(.authExpired)])
    #expect(result.sleeps.isEmpty)
}

@Test func cancellingTheConsumerStopsPolling() async {
    let script = Script(Array(repeating: .success(nil), count: 1_000_000))
    let monitor = ChangeMonitor(interval: .milliseconds(1), maxBackoff: .seconds(1))
    let consumer = Task {
        for await _ in monitor.changes(polling: ScriptedSource(script: script)) {}
    }
    try? await Task.sleep(for: .milliseconds(30))
    consumer.cancel()
    await consumer.value
    let callsAtStop = await script.calls
    try? await Task.sleep(for: .milliseconds(30))
    #expect(await script.calls <= callsAtStop + 1)
}

private struct SleeperBoom: Error {}

private actor FirstSleepThrows {
    private var thrown = false
    func sleep() throws {
        if !thrown { thrown = true; throw SleeperBoom() }
    }
}

@Test func sleeperErrorEndsTheMonitorWithoutCountingAsAFailure() async {
    // Only the first sleep throws; if the error were treated as a polling failure the loop would retry.
    let script = Script([.success(nil), .success(nil), .success(nil)])
    let gate = FirstSleepThrows()
    let monitor = ChangeMonitor(interval: .seconds(60), maxBackoff: .seconds(900), sleep: { _ in try await gate.sleep() })
    var changes: [CalendarChange] = []
    for await change in monitor.changes(polling: ScriptedSource(script: script)) { changes.append(change) }
    #expect(await script.calls == 1)
    #expect(changes.isEmpty)
}
