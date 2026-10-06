import CalendarCore
import Foundation
import ICalendar

/// What the source has loaded: the parsed feed, the bytes it came from (to spot a change) and what the server said
/// about that version.
actor FeedState {
    private(set) var feed: ParsedFeed?
    private(set) var body: Data?
    /// What the retained part of the feed looks like; this, not the raw body, decides whether anything changed.
    private(set) var fingerprint: Data?
    private(set) var validators: FeedValidators?
    private(set) var fetchedAt: Date?
    private var issued = 0
    private var storedGeneration = 0
    /// Newest fetch that found the feed unchanged. Kept apart from `storedGeneration` so that a newer "unchanged" answer
    /// does not stop an older fetch that saw genuinely different content from being stored (see `report`).
    private var unchangedGeneration = 0
    private var reportedBody: Data?
    private var previousReportedBody: Data?
    private var reportedGeneration = 0
    private var reportedUnreadable: Set<UnreadableRule> = []
    private var unreadableGeneration = 0

    /// The unreadable-rule keys not reported yet; the set then becomes `current`, so a rule that was fixed and breaks
    /// again, or is edited, is new again.
    /// A fetch older than one already applied changes nothing, so the set never moves backwards.
    func newUnreadable(_ current: [UnreadableRule], generation: Int) -> Set<UnreadableRule> {
        guard generation > unreadableGeneration else { return [] }
        unreadableGeneration = generation
        let fresh = Set(current).subtracting(reportedUnreadable)
        reportedUnreadable = Set(current)
        return fresh
    }

    /// Taken when a fetch starts, so a fetch that finishes after a newer one can be told apart.
    func nextGeneration() -> Int {
        issued += 1
        return issued
    }

    /// Keeps the feed only if no newer fetch has stored one already.
    func store(feed: ParsedFeed, body: Data, fingerprint: Data, validators: FeedValidators, generation: Int, at date: Date) {
        guard generation > storedGeneration else { return }
        storedGeneration = generation
        self.feed = feed
        self.body = body
        self.fingerprint = fingerprint
        self.validators = validators
        fetchedAt = date
    }

    /// What `checkForChanges` last saw, independent of what `events(in:)` loaded. The first report has no earlier one, so
    /// `prior` (the body held when the check began) stands in for it. A check whose fetch is older than one already
    /// reported says nothing, so overlapping checks report a change once and never report one backwards.
    func report(_ body: Data, generation: Int, prior: Data?) -> Report {
        guard generation > reportedGeneration else {
            // An older fetch can still be the first to see a change (the server answered a newer request with the old
            // feed). Report a body that differs from the last two reported ones, once; never move the generation back.
            guard let reportedBody, body != reportedBody, body != previousReportedBody else { return .stale }
            previousReportedBody = reportedBody
            self.reportedBody = body
            return .changed
        }
        reportedGeneration = generation
        let last = reportedBody ?? prior
        if let reportedBody, reportedBody != body { previousReportedBody = reportedBody }
        reportedBody = body
        return last != nil && last != body ? .changed : .unchanged
    }

    enum Report { case changed, unchanged, stale }

    /// What a fetch needs to know about the held feed, read together.
    func snapshot() -> (body: Data?, fingerprint: Data?, validators: FeedValidators?) { (body, fingerprint, validators) }

    /// The feed is unchanged: keep what is held and restart its age. A fetch older than the stored feed is ignored: what it
    /// found is no longer news, and its validators would replace newer ones.
    func unchanged(validators: FeedValidators?, body: Data?, generation: Int, at date: Date) {
        guard generation > max(storedGeneration, unchangedGeneration) else { return }
        unchangedGeneration = generation
        if let body { self.body = body }
        if let validators, validators.etag != nil || validators.lastModified != nil { self.validators = validators }
        fetchedAt = date
    }
}

/// One iCal subscription link: a read-only calendar of whatever the feed holds, re-read when it is older than `maxAge`.
public final class ICalSubscriptionSource: PollingCalendarSource {
    static let calendarID = "feed"

    private let connection: Connection
    private let link: @Sendable () async throws -> URL
    private let fetcher: FeedFetcher
    private let monitor: ChangeMonitor
    private let maxAge: TimeInterval
    private let now: @Sendable () -> Date
    private let defaultZone: TimeZone
    private let retention: RetentionWindow?
    private let diagnostics: any DiagnosticLog
    private let state = FeedState()

    /// `link` is read on every fetch, so a replaced link (after `reauthorize`) is picked up without rebuilding the source.
    init(
        connection: Connection, link: @escaping @Sendable () async throws -> URL, transport: any HTTPTransport,
        monitor: ChangeMonitor, maxAge: TimeInterval, now: @escaping @Sendable () -> Date, defaultZone: TimeZone,
        retention: RetentionWindow? = nil, diagnostics: any DiagnosticLog = NullDiagnosticLog()
    ) {
        self.retention = retention
        self.diagnostics = diagnostics
        self.connection = connection
        self.link = link
        self.fetcher = FeedFetcher(transport: transport, diagnostics: diagnostics, now: now)
        self.monitor = monitor
        self.maxAge = maxAge
        self.now = now
        self.defaultZone = defaultZone
    }

    public var id: String { connection.sourceID }
    public var displayName: String { connection.displayName }

    public var capabilities: SourceCapabilities {
        SourceCapabilities(providedFields: [.series, .uidScope, .provider, .calendarTimeZone], syncKind: .token)
    }

    public func calendars() async throws -> [CalendarDescriptor] {
        let feed = try await currentFeed()
        return [CalendarDescriptor(
            id: Self.calendarID, title: feed.name ?? connection.displayName, service: .iCalSubscription, colorHex: feed.colorHex,
            permissions: CalendarPermissions(canViewDetails: true, canEdit: false), isDefault: false,
            timeZone: feed.timeZone ?? defaultZone, accountName: connection.displayName, kind: .subscribed, provider: .subscription)]
    }

    public func events(in interval: DateInterval) async throws -> [CalendarEvent] {
        let feed = try await currentFeed()
        let zone = feed.timeZone ?? defaultZone
        var events: [CalendarEvent] = []
        for item in feed.resources {
            let context = EventReadContext(
                calendarID: Self.calendarID, resourceName: item.name, etag: nil, sourceID: id, calendarZone: zone, selfAddresses: [])
            events += EventReader.events(in: item.resource, overlapping: interval, context: context)
        }
        return events.sorted { ($0.start, $0.id) < ($1.start, $1.id) }
    }

    public func changes() -> AsyncStream<CalendarChange> { monitor.changes(polling: self) }

    /// Repeating events in the held feed whose rule could not be read (they show only their first occurrence and
    /// exceptions). Empty when none, and empty before the first load.
    public func notices() async -> [SourceNotice] {
        let count = await state.feed?.unreadableRules.count ?? 0
        return count > 0 ? [SourceNotice(kind: .unreadableRecurrence, count: count)] : []
    }

    /// Re-reads the feed. The first check with nothing loaded yet is the baseline and reports nothing; a feed loaded
    /// earlier by `events(in:)` counts as the baseline, so a change since then is reported.
    public func checkForChanges() async throws -> CalendarChange? {
        let prior = await state.fingerprint
        let fetched = try await refresh()
        let outcome = await state.report(fetched.fingerprint, generation: fetched.generation, prior: prior)
        switch outcome {
        case .changed:
            diagnostics.record(.info, "icalsub", "changeReported", [.bool("bodyChanged", fetched.bodyChanged)])
        case .unchanged where fetched.bodyChanged:
            diagnostics.record(.info, "icalsub", "changeSuppressed", [.string("reason", "unchangedRetained", private: false)])
        case .stale where fetched.bodyChanged:
            diagnostics.record(.info, "icalsub", "changeSuppressed", [.string("reason", "staleFetch", private: false)])
        default: break
        }
        return outcome == .changed ? .eventsChanged(calendarIDs: [Self.calendarID]) : nil
    }

    private func currentFeed() async throws -> ParsedFeed {
        if let feed = await state.feed, let fetchedAt = await state.fetchedAt, now().timeIntervalSince(fetchedAt) < maxAge { return feed }
        try await refresh()
        guard let feed = await state.feed else { throw SourceError.invalidResponse("the feed has not loaded") }
        return feed
    }

    /// Fetches the feed (conditionally when the server gave validators), keeps what the retention window allows, and returns
    /// a fingerprint of that with the generation the fetch started under. A 304 re-reads the held body, so an event that
    /// has come into the window since is noticed. A body that is not a calendar throws and leaves the last good feed in
    /// place; a fetch that finishes after a newer one does not replace what that one stored.
    @discardableResult
    private func refresh() async throws -> (fingerprint: Data, generation: Int, bodyChanged: Bool) {
        let url = try await link()
        let generation = await state.nextGeneration()
        let (previousBody, previousFingerprint, heldValidators) = await state.snapshot()
        let validators = heldValidators.flatMap { $0.etag != nil || $0.lastModified != nil ? $0 : nil }
        let data: Data
        var newValidators: FeedValidators?
        var newBody: Data?
        switch try await fetcher.fetch(url, validators: previousBody == nil ? nil : validators) {
        case .notModified:
            guard let previousBody else { throw SourceError.invalidResponse("the feed has not loaded") }
            data = previousBody
        case .body(let body, let received):
            data = body
            newBody = body
            newValidators = received
        }
        let at = now()
        let feed = try FeedParser.parse(data, retention: retention.map { ($0.interval(at: at), defaultZone) }, diagnostics: diagnostics,
                                     logsUnreadableRules: false)
        let current = feed.unreadableRules
        for rule in await state.newUnreadable(current, generation: generation).sorted(by: { ($0.uid, $0.ruleHash) < ($1.uid, $1.ruleHash) }) {
            diagnostics.record(rule.event)
        }
        let fingerprint = feed.fingerprint
        if fingerprint == previousFingerprint {
            await state.unchanged(validators: newValidators, body: newBody, generation: generation, at: at)
        } else {
            let kept = await state.validators ?? FeedValidators()
            await state.store(feed: feed, body: data, fingerprint: fingerprint, validators: newValidators ?? kept, generation: generation, at: at)
        }
        let bodyChanged = previousBody != nil && newBody != nil && newBody != previousBody
        return (fingerprint, generation, bodyChanged)
    }
}
