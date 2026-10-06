import CalendarCore
import Foundation

/// A `DiagnosticLog` for tests: keeps every event it is given.
public final class CollectingDiagnosticLog: DiagnosticLog, @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [DiagnosticEvent] = []

    public init() {}

    public func record(_ event: DiagnosticEvent) { lock.withLock { recorded.append(event) } }

    public var events: [DiagnosticEvent] { lock.withLock { recorded } }

    public func events(named name: String) -> [DiagnosticEvent] { events.filter { $0.name == name } }

    public func clear() { lock.withLock { recorded.removeAll() } }

    /// Every event as text, private fields included: what a leak test searches.
    public var transcript: String {
        events.map { event in
            ([event.category, event.name, event.level.label] + event.fields.map { "\($0.name)=\($0.unredactedText)" }).joined(separator: " ")
        }.joined(separator: "\n")
    }
}
