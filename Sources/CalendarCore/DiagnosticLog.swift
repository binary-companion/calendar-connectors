import Foundation

/// How serious a diagnostic event is. Ordered, so a sink can filter with `event.level >= .info`.
public enum DiagnosticLevel: Int, Sendable, Comparable {
    case debug, info, notice, warning, error

    public static func < (lhs: DiagnosticLevel, rhs: DiagnosticLevel) -> Bool { lhs.rawValue < rhs.rawValue }

    /// The lower-case word used in rendered reports.
    public var label: String {
        switch self {
        case .debug: "debug"
        case .info: "info"
        case .notice: "notice"
        case .warning: "warning"
        case .error: "error"
        }
    }
}

/// One named value of a `DiagnosticEvent`. Privacy is by default: a string is private unless the caller says otherwise,
/// numbers and flags are public. A connector must never put a credential, a link, a title or any other personal text in a
/// field, private or not: `isPrivate` only decides whether a report shows the value by default.
public struct DiagnosticField: Sendable, Equatable {
    public enum Value: Sendable, Equatable {
        case int(Int)
        case double(Double)
        case bool(Bool)
        case string(String)
    }

    public var name: String
    public var value: Value
    public var isPrivate: Bool

    public init(name: String, value: Value, isPrivate: Bool) {
        self.name = name
        self.value = value
        self.isPrivate = isPrivate
    }

    public static func int(_ name: String, _ value: Int, private isPrivate: Bool = false) -> DiagnosticField {
        DiagnosticField(name: name, value: .int(value), isPrivate: isPrivate)
    }

    public static func double(_ name: String, _ value: Double, private isPrivate: Bool = false) -> DiagnosticField {
        DiagnosticField(name: name, value: .double(value), isPrivate: isPrivate)
    }

    public static func bool(_ name: String, _ value: Bool, private isPrivate: Bool = false) -> DiagnosticField {
        DiagnosticField(name: name, value: .bool(value), isPrivate: isPrivate)
    }

    /// Strings are private unless `private: false` is passed (fixed tokens and host names).
    public static func string(_ name: String, _ value: String, private isPrivate: Bool = true) -> DiagnosticField {
        DiagnosticField(name: name, value: .string(value), isPrivate: isPrivate)
    }

    /// The value as text, ignoring `isPrivate`. For tests and for a report that has already decided what to show.
    public var unredactedText: String {
        switch value {
        case .int(let number): String(number)
        case .double(let number): String(number)
        case .bool(let flag): flag ? "true" : "false"
        case .string(let text): text
        }
    }
}

/// Something worth knowing about, with a stable `category` (the connector, for example `icalsub`) and a stable camelCase
/// `name` (for example `fetchCompleted`) that tests and support can rely on.
public struct DiagnosticEvent: Sendable, Equatable {
    public var level: DiagnosticLevel
    public var category: String
    public var name: String
    public var fields: [DiagnosticField]
    public var date: Date

    public init(level: DiagnosticLevel, category: String, name: String, fields: [DiagnosticField] = [], date: Date = Date()) {
        self.level = level
        self.category = category
        self.name = name
        self.fields = fields
        self.date = date
    }

    /// The first field called `name`.
    public func field(_ name: String) -> DiagnosticField? { fields.first { $0.name == name } }
}

/// Where a connector reports what it is doing. Adapters (the OS logger, a file, an in-app report) live outside the
/// library. `record` is synchronous, never throws and must not block: it is called on the connector's own path.
public protocol DiagnosticLog: Sendable {
    func record(_ event: DiagnosticEvent)
}

extension DiagnosticLog {
    public func record(_ level: DiagnosticLevel, _ category: String, _ name: String, _ fields: [DiagnosticField] = [], date: Date = Date()) {
        record(DiagnosticEvent(level: level, category: category, name: name, fields: fields, date: date))
    }
}

/// Discards everything. The default for every connector.
public struct NullDiagnosticLog: DiagnosticLog {
    public init() {}
    public func record(_ event: DiagnosticEvent) {}
}

extension DiagnosticLog where Self == NullDiagnosticLog {
    /// `diagnostics: .none`
    public static var none: NullDiagnosticLog { NullDiagnosticLog() }
}

/// Sends each event to every log, in order.
public struct FanOutDiagnosticLog: DiagnosticLog {
    private let logs: [any DiagnosticLog]
    public init(_ logs: [any DiagnosticLog]) { self.logs = logs }
    public func record(_ event: DiagnosticEvent) {
        for log in logs { log.record(event) }
    }
}

/// Keeps the last `capacity` events in memory, for an in-app report. Thread-safe.
public final class RingBufferDiagnosticLog: DiagnosticLog, @unchecked Sendable {
    public static let defaultCapacity = 500

    private let capacity: Int
    private let lock = NSLock()
    private var events: [DiagnosticEvent] = []

    public init(capacity: Int = RingBufferDiagnosticLog.defaultCapacity) {
        self.capacity = max(capacity, 1)
    }

    public func record(_ event: DiagnosticEvent) {
        lock.withLock {
            events.append(event)
            if events.count > capacity { events.removeFirst(events.count - capacity) }
        }
    }

    /// The kept events, oldest first.
    public func snapshot() -> [DiagnosticEvent] { lock.withLock { events } }

    public func clear() { lock.withLock { events.removeAll() } }

    /// A plain-text report, one line per event, oldest first:
    /// `2026-10-06T17:00:00Z info icalsub.fetchCompleted status=200 uid=<private>`.
    /// Private fields show `<private>` unless `includePrivate`.
    public func render(includePrivate: Bool = false) -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        formatter.timeZone = TimeZone(identifier: "UTC")
        return snapshot().map { event in
            let fields = event.fields.map { field in
                "\(field.name)=\(field.isPrivate && !includePrivate ? "<private>" : Self.text(field))"
            }
            return ([formatter.string(from: event.date), event.level.label, "\(event.category).\(event.name)"] + fields).joined(separator: " ")
        }.joined(separator: "\n")
    }

    /// A string value with a space, quote, equals sign or line break is quoted so a line stays one line.
    private static func text(_ field: DiagnosticField) -> String {
        guard case .string(let value) = field.value,
              value.contains(where: { $0.isWhitespace || $0 == "\"" || $0 == "=" }) else { return field.unredactedText }
        let escaped = value.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"")
            .replacingOccurrences(of: "\n", with: "\\n").replacingOccurrences(of: "\r", with: "\\r")
        return "\"\(escaped)\""
    }
}
