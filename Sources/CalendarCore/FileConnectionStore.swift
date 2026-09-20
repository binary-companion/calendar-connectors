import Foundation

public enum FileStoreError: Error, Equatable {
    /// The file exists but cannot be read; it is left untouched so the user's data is never overwritten.
    case unreadable
}

/// Persists the non-secret `Connection` list as JSON. Portable; the host chooses the file location.
public actor FileConnectionStore {
    public enum LoadResult: Equatable, Sendable {
        case loaded([Connection])
        case missing
        case unreadable
    }

    private let url: URL
    public init(url: URL) { self.url = url }

    public func load() -> LoadResult {
        guard FileManager.default.fileExists(atPath: url.path) else { return .missing }
        guard let data = try? Data(contentsOf: url),
              let list = try? JSONDecoder().decode([Connection].self, from: data) else { return .unreadable }
        return .loaded(list)
    }

    /// The stored connections; empty when the file is missing or unreadable (check `load()` to tell them apart).
    public func connections() -> [Connection] {
        if case .loaded(let list) = load() { return list }
        return []
    }

    public func save(_ connections: [Connection]) throws {
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        try encoder.encode(connections).write(to: url, options: .atomic)
    }

    /// Adds, or replaces the entry with the same `connectionID`.
    public func add(_ connection: Connection) throws {
        var list = try writable()
        if let index = list.firstIndex(where: { $0.connectionID == connection.connectionID }) {
            list[index] = connection
        } else {
            list.append(connection)
        }
        try save(list)
    }

    /// Removes a connection; a no-op when it is not stored.
    public func remove(connectionID: ConnectionID) throws {
        let list = try writable()
        guard list.contains(where: { $0.connectionID == connectionID }) else { return }
        try save(list.filter { $0.connectionID != connectionID })
    }

    private func writable() throws -> [Connection] {
        switch load() {
        case .loaded(let list): return list
        case .missing: return []
        case .unreadable: throw FileStoreError.unreadable
        }
    }
}
