import Foundation

/// A `SyncStateStore` backed by one JSON file. A missing or unreadable file means "no tokens", so a source does
/// a full bootstrap; write failures leave the in-memory state (and the next launch re-bootstraps).
public actor FileSyncStateStore: SyncStateStore {
    private let url: URL
    private var storage: [ConnectionID: [String: String]]?

    public init(url: URL) { self.url = url }

    private func current() -> [ConnectionID: [String: String]] {
        if let storage { return storage }
        let loaded = (try? Data(contentsOf: url))
            .flatMap { try? JSONDecoder().decode([ConnectionID: [String: String]].self, from: $0) } ?? [:]
        storage = loaded
        return loaded
    }

    public func token(for connectionID: ConnectionID, scope: String) async -> String? {
        current()[connectionID]?[scope]
    }

    public func setToken(_ token: String?, for connectionID: ConnectionID, scope: String) async {
        var all = current()
        var scopes = all[connectionID] ?? [:]
        scopes[scope] = token
        all[connectionID] = scopes.isEmpty ? nil : scopes
        commit(all)
    }

    public func removeAll(for connectionID: ConnectionID) async {
        var all = current()
        all[connectionID] = nil
        commit(all)
    }

    private func commit(_ all: [ConnectionID: [String: String]]) {
        storage = all
        try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        if let data = try? JSONEncoder().encode(all) { try? data.write(to: url, options: .atomic) }
    }
}
