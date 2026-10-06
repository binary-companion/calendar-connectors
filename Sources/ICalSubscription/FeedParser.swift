import CalendarCore
import Foundation
import ICalendar

/// One UID's events from a feed (a master plus its overrides) as the resource the shared `EventReader` expects.
struct FeedResource: Sendable {
    let name: String
    let resource: EventResource
}

/// A group's UID and a hash of its `RRULE` lines (never the rule text), so an edited rule is a new entry.
struct UnreadableRule: Hashable, Sendable {
    let uid: String
    /// The UID was made up from the event's start and title (the feed gave none), so it must not be logged.
    let isSynthetic: Bool
    let ruleHash: String

    /// The diagnostic for this rule: the UID only when the feed supplied it.
    var event: DiagnosticEvent {
        DiagnosticEvent(level: .notice, category: "icalsub", name: "rruleUnreadable",
                        fields: [isSynthetic ? .bool("syntheticUID", true) : .string("uid", uid)])
    }
}

struct ParsedFeed: Sendable {
    var name: String?
    var colorHex: String?
    var timeZone: TimeZone?
    var resources: [FeedResource]
    /// The kept groups whose recurrence rule could not be read, to tell "still broken" from "newly broken".
    var unreadableRules: [UnreadableRule] = []
    /// A deterministic form of what the user sees (header and retained events), to tell a real change from an edit outside
    /// the retention window or a feed that restamps itself on every request. Computed on demand.
    var fingerprint: Data { FeedParser.fingerprint(of: self) }
}

enum FeedParser {
    private static let notACalendar = "that link did not return a calendar"

    /// A feed is one `VCALENDAR` holding many events, while `EventResource` holds one UID's events. Events are grouped by
    /// `UID` (first-seen order); each group shares the calendar's header and `VTIMEZONE`s. A feed with no events is valid.
    /// With `retention`, a UID group with no occurrence overlapping the window (read in `zone`) is dropped; any other group
    /// is kept whole, and so is a group whose recurrence rule cannot be read (its occurrences are unknown, so it cannot be
    /// shown to be outside the window). `diagnostics` gets counts and fixed tokens only: never text from the feed.
    static func parse(
        _ data: Data, retention: (window: DateInterval, zone: TimeZone)? = nil, diagnostics: any DiagnosticLog = NullDiagnosticLog(),
        logsUnreadableRules: Bool = true
    ) throws -> ParsedFeed {
        let calendar: ICalComponent
        do { calendar = try ICalParser.parse(data) } catch {
            diagnostics.record(.warning, "icalsub", "feedNotCalendar")
            throw SourceError.invalidResponse(notACalendar)
        }
        guard calendar.name == "VCALENDAR" else {
            diagnostics.record(.warning, "icalsub", "feedNotCalendar")
            throw SourceError.invalidResponse(notACalendar)
        }

        let zones = calendar.components(named: "VTIMEZONE")
        var order: [String] = []
        var groups: [String: [ICalComponent]] = [:]
        var eventsInFeed = 0, withoutUID = 0
        var syntheticUIDs: Set<String> = []
        for event in calendar.components(named: "VEVENT") {
            eventsInFeed += 1
            let uid: String
            if let given = event.property("UID")?.text.nonEmpty { uid = given } else {
                withoutUID += 1
                uid = syntheticUID(for: event)
                syntheticUIDs.insert(uid)
            }
            if groups[uid] == nil { order.append(uid) }
            groups[uid, default: []].append(event)
        }
        var resources: [FeedResource] = []
        let feedZone = calendar.property("X-WR-TIMEZONE").flatMap { TimeZone(identifier: $0.value) }
        var dropped = 0
        var unreadable: [UnreadableRule] = []
        for uid in order {
            let wrapper = ICalComponent(name: "VCALENDAR", properties: calendar.properties, components: zones + (groups[uid] ?? []))
            guard let resource = try? EventResource(calendar: wrapper) else { continue }
            let item = FeedResource(name: resourceName(for: uid), resource: resource)
            let zone = feedZone ?? retention?.zone ?? .gmt
            let isUnreadable = hasUnreadableRule(item, zone: zone)
            if isUnreadable {
                let rule = UnreadableRule(uid: uid, isSynthetic: syntheticUIDs.contains(uid), ruleHash: fnv1aHex(ruleText(of: item)))
                unreadable.append(rule)
                if logsUnreadableRules { diagnostics.record(rule.event) }
            }
            if let retention, !isUnreadable, !isKept(item, window: retention.window, zone: zone) {
                dropped += 1
                continue
            }
            resources.append(item)
        }
        if withoutUID > 0 { diagnostics.record(.info, "icalsub", "syntheticUID", [.int("count", withoutUID)]) }
        diagnostics.record(.info, "icalsub", "feedParsed", [
            .int("eventsInFeed", eventsInFeed), .int("groupsKept", resources.count), .int("groupsDropped", dropped),
            .int("unreadableRules", unreadable.count),
            .bool("retentionActive", retention != nil),
        ])
        let feed = ParsedFeed(
            name: calendar.property("X-WR-CALNAME")?.text.nonEmpty,
            colorHex: colorText(calendar.property("X-APPLE-CALENDAR-COLOR")?.value ?? calendar.property("COLOR")?.value),
            timeZone: feedZone,
            resources: resources, unreadableRules: unreadable)
        return feed
    }

    /// A group is kept when the shared reader finds at least one event or occurrence overlapping the window. The reader
    /// expands rules, dates and exceptions only inside the window (skipping straight to it, with an instance limit), applies
    /// overrides, and decides zero-length and all-day events, so an unbounded series stays cheap.
    private static func isKept(_ item: FeedResource, window: DateInterval, zone: TimeZone) -> Bool {
        let context = EventReadContext(calendarID: "feed", resourceName: item.name, etag: nil, sourceID: "", calendarZone: zone, selfAddresses: [])
        return !EventReader.events(in: item.resource, overlapping: window, context: context).isEmpty
    }

    /// The group's `RRULE` lines, to tell an edited rule from an unchanged one.
    private static func ruleText(of item: FeedResource) -> String {
        item.resource.master?.properties(named: "RRULE").map { ICalSerializer.contentLine($0) }.joined(separator: "\n") ?? ""
    }

    /// True when the group's series master has an `RRULE` the shared reader could not read. A group with no usable
    /// `DTSTART` has no timing to expand from, so it is never checked (and never reported).
    private static func hasUnreadableRule(_ item: FeedResource, zone: TimeZone) -> Bool {
        guard let master = item.resource.master,
              let timing = EventReader.timing(of: master, resolver: item.resource.resolver, calendarZone: zone) else { return false }
        return EventReader.recurrenceSet(of: master, zone: timing.zone, isAllDay: timing.isAllDay, resolver: item.resource.resolver)?
            .hasUnreadableRule ?? false
    }

    /// Properties a feed may rewrite on every request without the event having changed.
    private static let volatile: Set<String> = ["DTSTAMP", "LAST-MODIFIED", "CREATED", "PRODID"]

    static func fingerprint(of feed: ParsedFeed) -> Data {
        var data = Data("\(feed.name ?? "")|\(feed.colorHex ?? "")|\(feed.timeZone?.identifier ?? "")\n".utf8)
        for item in feed.resources.sorted(by: { $0.name < $1.name }) {
            data.append(Data("#\(item.name)\n".utf8))
            let events = item.resource.events.map { event in
                ICalComponent(name: event.name, properties: event.properties.filter { !volatile.contains($0.name) }, components: event.components)
            }
            for event in events.sorted(by: { ($0.property("RECURRENCE-ID")?.value ?? "") < ($1.property("RECURRENCE-ID")?.value ?? "") }) {
                data.append(Data(ICalSerializer.serialize(event).utf8))
            }
        }
        return data
    }

    /// The UID as one URL-safe path segment: it is the base of every event id, which joins an occurrence's start with `#`.
    static func resourceName(for uid: String) -> String {
        var allowed = CharacterSet.alphanumerics
        allowed.insert(charactersIn: "-._~@")
        return uid.addingPercentEncoding(withAllowedCharacters: allowed) ?? uid
    }

    /// A UID for an event that has none: stable across reads because it comes only from the event's start and title.
    static func syntheticUID(for event: ICalComponent) -> String {
        let basis = (event.property("DTSTART")?.value ?? "") + "|" + (event.property("SUMMARY")?.value ?? "")
        return "feed-" + fnv1aHex(basis)
    }

    /// FNV-1a, 64 bit. Not for security: it only keeps a made-up UID the same from one read to the next.
    static func fnv1aHex(_ text: String) -> String {
        var hash: UInt64 = 0xcbf2_9ce4_8422_2325
        for byte in text.utf8 {
            hash ^= UInt64(byte)
            hash = hash &* 0x0000_0100_0000_01b3
        }
        return String(hash, radix: 16)
    }

    /// `#RRGGBBAA` (Apple) is trimmed to `#RRGGBB`; the descriptor validates the rest.
    private static func colorText(_ raw: String?) -> String? {
        guard let raw else { return nil }
        return raw.hasPrefix("#") && raw.count == 9 ? String(raw.prefix(7)) : raw
    }
}

private extension String {
    var nonEmpty: String? { isEmpty ? nil : self }
}
