import Foundation

public typealias Sleeper = @Sendable (Duration) async throws -> Void
public let defaultSleeper: Sleeper = { try await Task.sleep(for: $0) }

/// Turns a source's cheap `checkForChanges()` into a `changes()` stream by polling.
/// Sleeping goes through `sleep`, so tests drive it without real time. Use one subscriber per source
/// (each `changes(polling:)` call starts its own poller).
public struct ChangeMonitor: Sendable {
    public var interval: Duration
    public var maxBackoff: Duration
    private let sleep: Sleeper

    public init(interval: Duration = .seconds(60), maxBackoff: Duration = .seconds(900), sleep: @escaping Sleeper = defaultSleeper) {
        self.interval = interval
        self.maxBackoff = maxBackoff
        self.sleep = sleep
    }

    /// Polling starts as soon as the stream is created, and stops only when the consuming task is cancelled
    /// (or the stream finishes, e.g. on `authExpired`). Dropping the stream without cancelling does NOT stop it.
    public func changes(polling source: some PollingCalendarSource) -> AsyncStream<CalendarChange> {
        let interval = interval, maxBackoff = maxBackoff, sleep = sleep
        return AsyncStream { continuation in
            let task = Task {
                var failures = 0
                while !Task.isCancelled {
                    do {
                        if let change = try await source.checkForChanges() { continuation.yield(change) }
                        failures = 0
                    } catch is CancellationError {
                        break
                    } catch SourceError.authExpired {
                        continuation.yield(.sourceFailed(.authExpired))
                        break
                    } catch {
                        failures += 1
                        // Clamp the exponent before multiplying so a long outage cannot overflow.
                        let delay = min(maxBackoff, interval * (1 << min(failures, 10)))
                        // A sleeper error is not a source failure: it just ends the monitor.
                        do { try await sleep(delay) } catch { break }
                        continue
                    }
                    do { try await sleep(interval) } catch { break }
                }
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}
