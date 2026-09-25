# Calendar connectors Phase 3 (write capabilities) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an optional, provider-neutral write API (create, update, delete, respond, with recurrence scopes and an explicit notification policy) to the connector library, and implement it for Google and EventKit.

**Architecture:** New pure-Swift write types live in `CalendarCore` (`WritableCalendarSource`, `EventDraft`, `EventPatch`, `RecurrenceRule`, `PatchMerge`, `WriteError`, ...). A source opts in by adopting `WritableCalendarSource` and declaring `writableFields`, `controlsNotifications` and `recurrenceScopes` in its `SourceCapabilities`. Updates send only changed fields; a stale version is judged per field against the patch's `base` (three-way merge). Google implements it over REST (PATCH/POST/DELETE with `If-Match`), EventKit over `EKEventStore.save/remove` with spans.

**Tech Stack:** Swift 6 (library packages; `EventKitSource` and `CalendarApple` in Swift 5 mode), Swift Testing, `FakeTransport` for Google, EventKit (macOS).

Spec: `docs/superpowers/specs/2026-09-23-calendar-connectors-phase3-design.md` (authoritative). API summary: `docs/calendar-connectors-api.md` part 10. Read both and `AGENTS.md` first.

## Global Constraints

- Everything new in `CalendarCore` and `CalendarTestSupport` is pure Swift, Foundation only (no EventKit, CoreGraphics, Security, AppKit), so the `core-linux` job (`swift:6.0`) keeps building it. `CalendarConnectors` keeps zero external dependencies.
- Writing is opt-in by protocol: `WritableCalendarSource: CalendarSource`. Invariant checked in tests: `capabilities.canWrite == (source is WritableCalendarSource)`; `canEditAttendees == writableFields.contains(.attendees)`.
- `NotifyPolicy` has no default anywhere; every write call passes it.
- Anything a connector cannot write throws `WriteError.unsupported(fields:)` before any request or store mutation; there are no silent partial writes. Only `EventDraft(copying:for:)` is lenient.
- Updates send only changed fields (`EventPatch`), never a whole event. A stale version is judged per field against `patch.base`; a patch with no `base` conflicts on any stale version.
- All-day events keep the library's canonical form (midnight of the first day in `timeZone`, `end` exclusive), built and read only through `AllDay`.
- Reads must behave exactly as before: the `GoogleAPIClient.send` refactor keeps 410 as `GoogleAPIError.gone`, 404 `.notFound`, plain 403 `.forbidden`, `insufficientPermissions` as `SourceError.authExpired`.
- `TimeTugCore` and `CalendarCore` both define `CalendarSource` and `SourceError`; qualify (`CalendarCore.SourceError`) outside Core.
- Every behaviour gets a failing Swift Testing test first. Live tests are opt-in, gated by an environment variable, never run in CI, and clean up what they create.
- Never run `pkill -x TimeTug`. `xcodegen generate` rewrites the two checked-in Info.plists: `git checkout -- Apps/macOS/Sources/Info.plist Apps/macOS/Widgets/Info.plist` before committing. Never commit or print `~/.config/timetug/google-oauth.xcconfig` values.
- Every commit message ends with `Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>`. Work on branch `claude/phase-3-definition-f7c388`; land through a PR into `master` (never merge locally). Do not merge the PR until the user says so.
- Use the project index for discovery if it is indexed (`~/.claude/skills/index/SKILL.md`); otherwise grep.
- Per the user's standing rule, run the `deepseek-review` skill on the full diff (with the spec and tests) before opening the PR and verify each Critical or Important finding against the repo.

## File map

Library (`Packages/CalendarConnectors`):

| File | Responsibility |
|---|---|
| `Sources/CalendarCore/Write/WriteTypes.swift` (new) | `EventField`, `NotifyPolicy`, `RecurrenceScope`, `ConferenceRequest`, `ConferenceChange`, `WriteError`, `EventRef`, `WriteValidation` |
| `Sources/CalendarCore/Write/RecurrenceRule.swift` (new) | structured RRULE subset, parse and render |
| `Sources/CalendarCore/Write/EventDraft.swift` (new) | `EventTiming`, `AttendeeDraft`, `EventDraft` (+ `copying`) |
| `Sources/CalendarCore/Write/EventPatch.swift` (new) | `FieldUpdate`, `AttendeeChanges`, `EventPatch` (+ diff, `applied(to:)`), `EventEdit` |
| `Sources/CalendarCore/Write/PatchMerge.swift` (new) | field-level conflict judgment and the retry loop |
| `Sources/CalendarCore/Write/WritableCalendarSource.swift` (new) | the protocol |
| `Sources/CalendarCore/SourceTypes.swift`, `Model.swift` (edit) | capability fields, `CalendarEvent.sourceID` |
| `Sources/CalendarTestSupport/FakeWritableSource.swift`, `WritableSourceConformance.swift` (new) | in-memory writable source and the conformance checks |
| `Sources/GoogleCalendar/GoogleAPIClient.swift` (edit) | `send`, new provider outcomes |
| `Sources/GoogleCalendar/GoogleWriteMapper.swift` (new) | pure JSON building and series helpers |
| `Sources/GoogleCalendar/GoogleCalendarSource+Write.swift` (new) | `WritableCalendarSource` for Google |
| `Sources/GoogleCalendar/GoogleEventMapper.swift`, `GoogleDTOs.swift`, `GoogleCalendarSource.swift` (edit) | `sourceID`, `recurrence`, capabilities |

`Packages/EventKitSource`: `EventKitWriteMapping.swift` (new, pure), `EventKitSource+Write.swift` (new), `EventKitSource.swift` (edit: read fields, internal access), live tests. `Packages/CalendarApple/Tests`: Google live smoke. Docs: API contract, ADR 0012, `AGENTS.md`, manual test doc, spec reconciliation.

Test commands: `swift test --package-path Packages/CalendarConnectors [--filter Name]`, `swift test --package-path Packages/EventKitSource`, `swift test --package-path Packages/CalendarBridge`, `swift test --package-path Packages/TimeTugCore`.

---

## Task 1: EventKit spike (user-run; findings adjust the spec)

The spec's Risk 1 and 5 are unverified EventKit assumptions. Run this once, early, in parallel with Tasks 2 to 8 (it needs the user's Mac and calendar permission). EventKit Tasks 12 and 13 depend on its findings.

**Files:**
- Create: `Packages/EventKitSource/Tests/EventKitSourceTests/LiveSupport.swift`
- Create: `Packages/EventKitSource/Tests/EventKitSourceTests/EventKitLiveSpikeTests.swift`

**Interfaces:**
- Produces: `withScratchCalendar(_ body: (EKEventStore, EKCalendar) async throws -> Void) async throws` (used again in Task 13), and `liveEventKit: Bool`.

- [ ] **Step 1: Write the live support helper**

```swift
// LiveSupport.swift
import EventKit
import Foundation
import Testing

/// Live EventKit tests are opt-in: `TIMETUG_LIVE_EVENTKIT=1 swift test --package-path Packages/EventKitSource --filter Live`.
let liveEventKit = ProcessInfo.processInfo.environment["TIMETUG_LIVE_EVENTKIT"] == "1"

/// Creates a scratch calendar in the local ("On My Mac") source, runs `body`, then deletes the calendar and
/// everything in it. Existing calendars are never touched.
func withScratchCalendar(_ body: (EKEventStore, EKCalendar) async throws -> Void) async throws {
    let store = EKEventStore()
    guard try await store.requestFullAccessToEvents() else {
        Issue.record("calendar access was denied")
        return
    }
    guard let source = store.sources.first(where: { $0.sourceType == .local }) else {
        Issue.record("no local calendar source; enable On My Mac in Calendar settings")
        return
    }
    let calendar = EKCalendar(for: .event, eventStore: store)
    calendar.title = "TimeTug live test \(UUID().uuidString.prefix(6))"
    calendar.source = source
    try store.saveCalendar(calendar, commit: true)
    defer { try? store.removeCalendar(calendar, commit: true) }
    try await body(store, calendar)
}

func nextHour(daysAhead: Int = 2) -> Date {
    let calendar = Calendar.current
    let day = calendar.date(byAdding: .day, value: daysAhead, to: Date())!
    return calendar.date(bySettingHour: 10, minute: 0, second: 0, of: day)!
}
```

- [ ] **Step 2: Write the spike**

```swift
// EventKitLiveSpikeTests.swift
import EventKit
import Foundation
import Testing

@Test(.enabled(if: liveEventKit)) func eventKitRecurringSpike() async throws {
    try await withScratchCalendar { store, calendar in
        let start = nextHour()
        let event = EKEvent(eventStore: store)
        event.calendar = calendar
        event.title = "Spike weekly"
        event.startDate = start
        event.endDate = start.addingTimeInterval(1800)
        event.addRecurrenceRule(EKRecurrenceRule(recurrenceWith: .weekly, interval: 1, end: EKRecurrenceEnd(occurrenceCount: 4)))
        try store.save(event, span: .thisEvent, commit: true)

        func occurrences() -> [EKEvent] {
            let predicate = store.predicateForEvents(withStart: start.addingTimeInterval(-86_400), end: start.addingTimeInterval(86_400 * 40), calendars: [calendar])
            return store.events(matching: predicate).sorted { $0.startDate < $1.startDate }
        }
        var list = occurrences()
        print("SPIKE1 occurrences=\(list.count) (expect 4)")
        print("SPIKE1 sharedEventIdentifier=\(Set(list.map { $0.eventIdentifier }).count == 1)")
        print("SPIKE1 occurrenceDateEqualsStart=\(list.allSatisfy { abs($0.occurrenceDate.timeIntervalSince($0.startDate)) < 1 })")
        print("SPIKE1 hasRecurrenceRulesOnEveryOccurrence=\(list.allSatisfy { $0.hasRecurrenceRules })")

        let id = list[0].eventIdentifier!
        let byID = store.event(withIdentifier: id)
        print("SPIKE2 eventWithIdentifierIsFirstOccurrence=\(byID.map { abs($0.startDate.timeIntervalSince(start)) < 1 } ?? false)")

        // Move the third occurrence: it becomes detached and keeps its original occurrenceDate.
        let third = list[2]
        let originalSlot = third.occurrenceDate!
        third.startDate = third.startDate.addingTimeInterval(3600)
        third.endDate = third.endDate.addingTimeInterval(3600)
        try store.save(third, span: .thisEvent, commit: true)
        list = occurrences()
        let moved = list[2]
        print("SPIKE3 movedIsDetached=\(moved.isDetached) startMoved=\(abs(moved.startDate.timeIntervalSince(originalSlot)) > 1) occurrenceDateKept=\(abs(moved.occurrenceDate.timeIntervalSince(originalSlot)) < 1)")
        print("SPIKE3 movedSharesIdentifier=\(moved.eventIdentifier == id) movedHasRecurrenceRules=\(moved.hasRecurrenceRules)")

        // Edit the whole series through a LATER occurrence's first-occurrence lookup.
        let first = store.event(withIdentifier: id)!
        first.title = "Spike renamed"
        try store.save(first, span: .futureEvents, commit: true)
        list = occurrences()
        print("SPIKE4 allTitles=\(list.map { $0.title ?? "-" })")

        // refresh() on a removed event.
        let victim = list[3]
        try store.remove(victim, span: .thisEvent, commit: true)
        print("SPIKE5 refreshAfterRemove=\(victim.refresh())")
        print("SPIKE5 lastModifiedDateSet=\(first.lastModifiedDate != nil)")

        // Two saves in the same second: does lastModifiedDate distinguish them?
        let a = first.lastModifiedDate
        first.notes = "one"
        try store.save(first, span: .thisEvent, commit: true)
        let b = first.lastModifiedDate
        first.notes = "two"
        try store.save(first, span: .thisEvent, commit: true)
        let c = first.lastModifiedDate
        print("SPIKE6 lastModifiedChangesEachSave=\(a != b && b != c) a=\(String(describing: a)) b=\(String(describing: b)) c=\(String(describing: c))")
    }
}
```

- [ ] **Step 3: Run it (user machine, permission prompt)**

Run: `TIMETUG_LIVE_EVENTKIT=1 swift test --package-path Packages/EventKitSource --filter eventKitRecurringSpike 2>&1 | grep -E "SPIKE|error|Issue"`
Expected: lines `SPIKE1` to `SPIKE6`. If macOS asks for calendar access for the terminal, allow it and re-run. If it cannot run (no permission, no local source), tell the controller; continue with Tasks 2 to 11 and gate Tasks 12 and 13 on the result.

- [ ] **Step 4: Record the findings in the spec**

Replace Risks item 1 and the EventKit part of item 5 in `docs/superpowers/specs/2026-09-23-calendar-connectors-phase3-design.md` with the observed answers (shared identifier yes/no, `occurrenceDate` reliable, first occurrence via `event(withIdentifier:)`, `.futureEvents` on it edits every occurrence, `refresh()` false after remove, `lastModifiedDate` distinct per save). If `.allInSeries` does not work, change the EventKit rows in the capabilities table and scopes text to drop it from `recurrenceScopes`, and in Task 13 remove `.allInSeries` from the capability set and make `locateTarget` throw `WriteError.unsupported(fields: [.recurrence])` for it. If the identifier is NOT shared, keep `originalStart` matching anyway (it is still correct).

- [ ] **Step 5: Commit**

```bash
git add Packages/EventKitSource/Tests docs/superpowers/specs/2026-09-23-calendar-connectors-phase3-design.md
git commit -m "test: EventKit live spike for recurring occurrences and versions" -m "Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 2: Core write vocabulary, capabilities and `CalendarEvent.sourceID`

**Files:**
- Create: `Packages/CalendarConnectors/Sources/CalendarCore/Write/WriteTypes.swift`
- Modify: `Packages/CalendarConnectors/Sources/CalendarCore/SourceTypes.swift`, `Model.swift`
- Test: `Packages/CalendarConnectors/Tests/CalendarCoreTests/WriteTypesTests.swift`, `Packages/CalendarConnectors/Tests/CalendarCoreTests/TestHelpers.swift`

**Interfaces:**
- Produces: `EventField` (`CaseIterable`), `NotifyPolicy`, `RecurrenceScope` (`CaseIterable`), `ConferenceRequest`, `ConferenceChange`, `WriteError`, `EventRef`, `WriteValidation.requireWritable(_:_:)`; `SourceCapabilities.writableFields/controlsNotifications/recurrenceScopes`; `CalendarEvent.sourceID`.

- [ ] **Step 1: Write the failing tests**

```swift
// TestHelpers.swift
import Testing
@testable import CalendarCore

/// Runs `body` and records an issue unless it throws exactly `expected`.
func expectWriteError(_ expected: WriteError, _ body: () async throws -> Void) async {
    do {
        try await body()
        Issue.record("expected \(expected), but nothing was thrown")
    } catch let error as WriteError {
        #expect(error == expected)
    } catch {
        Issue.record("expected \(expected), got \(error)")
    }
}
```

```swift
// WriteTypesTests.swift
import Foundation
import Testing
@testable import CalendarCore

private func event(series: String? = nil, original: Date? = nil) -> CalendarEvent {
    CalendarEvent(eventID: "e1", calendarID: "cal", title: "T", start: Date(timeIntervalSince1970: 1000),
                  end: Date(timeIntervalSince1970: 2000), seriesID: series, originalStart: original, version: "v1")
}

@Test func eventRefCopiesTheFieldsAWriteNeeds() {
    let original = Date(timeIntervalSince1970: 900)
    let ref = EventRef(event(series: "s1", original: original))
    #expect(ref == EventRef(calendarID: "cal", eventID: "e1", version: "v1", seriesID: "s1", originalStart: original))
}

@Test func capabilitiesDefaultToNothingWritable() {
    let c = SourceCapabilities()
    #expect(c.writableFields.isEmpty && c.recurrenceScopes.isEmpty && !c.controlsNotifications)
}

@Test func eventsHaveNoSourceIDUntilASourceStampsOne() {
    #expect(event().sourceID == nil)
    var stamped = event()
    stamped.sourceID = "google-1"
    #expect(stamped.sourceID == "google-1" && stamped.id == "cal/e1")
}

@Test func requireWritableNamesEveryMissingField() async {
    let caps = SourceCapabilities(canWrite: true, writableFields: [.title, .timing])
    #expect(throws: Never.self) { try WriteValidation.requireWritable([.title], caps) }
    await expectWriteError(.unsupported(fields: [.attendees, .visibility])) {
        try WriteValidation.requireWritable([.title, .attendees, .visibility], caps)
    }
}

@Test func writeErrorsCompareByPayload() {
    #expect(WriteError.conflict(fields: [.title]) == .conflict(fields: [.title]))
    #expect(WriteError.conflict(fields: [.title]) != .conflict(fields: [.notes]))
    #expect(WriteError.forbidden(nil) != .forbidden("read-only calendar"))
}
```

- [ ] **Step 2: Run to verify failure**

Run: `swift test --package-path Packages/CalendarConnectors --filter CalendarCoreTests`
Expected: FAIL to compile (`cannot find 'EventRef' in scope`, and similar).

- [ ] **Step 3: Implement**

```swift
// Sources/CalendarCore/Write/WriteTypes.swift
import Foundation

/// A part of an event that a write can touch. Used for validation, conflict reports and capabilities.
public enum EventField: String, Sendable, Hashable, CaseIterable {
    case title, notes, location, timing, availability, visibility, reminders, attendees, recurrence, conference
}

/// Whether the provider may email attendees about a write. Callers must choose; there is no default.
public enum NotifyPolicy: Sendable, Hashable { case all, externalOnly, none }

/// Which occurrences of a recurring event a write applies to. Ignored for an event that is not part of a series.
public enum RecurrenceScope: Sendable, Hashable, CaseIterable { case thisInstance, thisAndFollowing, allInSeries }

/// On a draft: whether to ask the provider for a generated conference link.
public enum ConferenceRequest: Sendable, Hashable { case none, generate }
/// On a patch: generate a link or remove the existing one.
public enum ConferenceChange: Sendable, Hashable { case generate, remove }

public enum WriteError: Error, Sendable, Equatable {
    /// The connector cannot write these fields (or the operation; RSVP is reported as `.attendees`).
    case unsupported(fields: Set<EventField>)
    /// Someone else changed a field this patch touches.
    case conflict(fields: Set<EventField>)
    /// The event or calendar is gone.
    case notFound
    /// A read-only calendar, or no permission on this event.
    case forbidden(String?)
    /// Malformed input: end before start, RSVP of `.needsAction`, a bad recurrence rule, ...
    case invalid(String)
    /// A multi-step write stopped half way (Google `.thisAndFollowing`: series truncated, new series not created).
    case partial(String)
}

/// What a write needs to find an event. Build one from the event you read.
public struct EventRef: Hashable, Sendable {
    public var calendarID: String
    public var eventID: String
    /// The version the caller last saw; used for optimistic locking.
    public var version: String?
    public var seriesID: String?
    /// The occurrence's slot in its series (Google `originalStartTime`, EventKit `occurrenceDate`); differs from
    /// `start` for a moved occurrence. It identifies the occurrence when `eventID` is shared and is the split
    /// point for `.thisAndFollowing`.
    public var originalStart: Date?

    public init(calendarID: String, eventID: String, version: String? = nil, seriesID: String? = nil, originalStart: Date? = nil) {
        self.calendarID = calendarID
        self.eventID = eventID
        self.version = version
        self.seriesID = seriesID
        self.originalStart = originalStart
    }

    public init(_ event: CalendarEvent) {
        self.init(calendarID: event.calendarID, eventID: event.eventID, version: event.version,
                  seriesID: event.seriesID, originalStart: event.originalStart)
    }
}

public enum WriteValidation {
    /// Throws `.unsupported` naming every field in `fields` that `capabilities.writableFields` lacks.
    public static func requireWritable(_ fields: Set<EventField>, _ capabilities: SourceCapabilities) throws {
        let missing = fields.subtracting(capabilities.writableFields)
        if !missing.isEmpty { throw WriteError.unsupported(fields: missing) }
    }
}
```

In `SourceTypes.swift`, replace the `SourceCapabilities` struct with:

```swift
public struct SourceCapabilities: Equatable, Sendable {
    public var canWrite: Bool
    public var canEditAttendees: Bool
    public var canRespondToInvite: Bool
    public var providesConference: Bool
    public var syncKind: SyncKind
    public var supportsPush: Bool
    /// The fields create/update can write; drives validation and `EventDraft(copying:for:)`. Empty when read-only.
    public var writableFields: Set<EventField>
    /// Honors `NotifyPolicy` (Google: `sendUpdates`); false means the server decides.
    public var controlsNotifications: Bool
    /// Scopes accepted for update/delete on a recurring series. Empty when read-only.
    public var recurrenceScopes: Set<RecurrenceScope>

    public init(
        canWrite: Bool = false, canEditAttendees: Bool = false, canRespondToInvite: Bool = false,
        providesConference: Bool = false, syncKind: SyncKind = .none, supportsPush: Bool = false,
        writableFields: Set<EventField> = [], controlsNotifications: Bool = false,
        recurrenceScopes: Set<RecurrenceScope> = []
    ) {
        self.canWrite = canWrite
        self.canEditAttendees = canEditAttendees
        self.canRespondToInvite = canRespondToInvite
        self.providesConference = providesConference
        self.syncKind = syncKind
        self.supportsPush = supportsPush
        self.writableFields = writableFields
        self.controlsNotifications = controlsNotifications
        self.recurrenceScopes = recurrenceScopes
    }
}
```

In `Model.swift`, add to `CalendarEvent` (after `myResponse`) `/// The source that produced this event (`Connection.sourceID`; "eventkit" for EventKit). Stamped by the source. public var sourceID: String?`, add `sourceID: String? = nil` as the last initializer parameter and `self.sourceID = sourceID` at the end of the initializer body. Update the comment on `version` to "Opaque provider version (Google etag, EventKit modification date); the base of optimistic writes."

- [ ] **Step 4: Run to verify pass**

Run: `swift test --package-path Packages/CalendarConnectors`
Expected: PASS (all existing tests plus the five new ones).

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "feat(core): write vocabulary, capability fields and CalendarEvent.sourceID" -m "Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 3: `RecurrenceRule` (RRULE subset)

**Files:**
- Create: `Packages/CalendarConnectors/Sources/CalendarCore/Write/RecurrenceRule.swift`
- Test: `Packages/CalendarConnectors/Tests/CalendarCoreTests/RecurrenceRuleTests.swift`

**Interfaces:**
- Consumes: `WriteError` (Task 2).
- Produces: `RecurrenceRule` with `Frequency`, `Weekday`, `WeekdayOccurrence`, `End`, stored properties `frequency/interval/weekdays/monthDays/months/end`, `init(frequency:interval:weekdays:monthDays:months:end:)`, `validate() throws`, `init(rrule:in:) throws`, `rruleString(allDay:in:) -> String`, `static untilText(_:allDay:zone:) -> String`, `static dateText(_ CalendarDate) -> String`.

- [ ] **Step 1: Write the failing tests**

```swift
import Foundation
import Testing
@testable import CalendarCore

private let utc = TimeZone(identifier: "UTC")!
private let tokyo = TimeZone(identifier: "Asia/Tokyo")!
private func instant(_ s: String) -> Date { ISO8601DateFormatter().date(from: s)! }

@Test func parsesAWeeklyRuleWithDaysAndCount() throws {
    let rule = try RecurrenceRule(rrule: "RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=6")
    #expect(rule.frequency == .weekly && rule.interval == 2)
    #expect(rule.weekdays == [.init(.monday), .init(.wednesday)])
    #expect(rule.end == .count(6))
}

@Test func parsesOrdinalWeekdaysMonthDaysAndMonths() throws {
    let monthly = try RecurrenceRule(rrule: "FREQ=MONTHLY;BYDAY=2TU,-1FR")
    #expect(monthly.weekdays == [.init(.tuesday, ordinal: 2), .init(.friday, ordinal: -1)])
    let byDay = try RecurrenceRule(rrule: "FREQ=MONTHLY;BYMONTHDAY=1,15,-1")
    #expect(byDay.monthDays == [1, 15, -1])
    let yearly = try RecurrenceRule(rrule: "FREQ=YEARLY;BYMONTH=3,9;BYMONTHDAY=5")
    #expect(yearly.months == [3, 9] && yearly.monthDays == [5])
}

@Test func rendersInCanonicalOrder() {
    let rule = RecurrenceRule(frequency: .monthly, interval: 3, weekdays: [.init(.tuesday, ordinal: 2)], end: .count(4))
    #expect(rule.rruleString(allDay: false, in: nil) == "FREQ=MONTHLY;INTERVAL=3;BYDAY=2TU;COUNT=4")
    #expect(RecurrenceRule(frequency: .daily).rruleString(allDay: false, in: nil) == "FREQ=DAILY")
}

@Test func untilRendersAsUTCDateTimeForTimedAndDateInZoneForAllDay() {
    let until = instant("2026-10-05T15:00:00Z")   // 2026-10-06 00:00 in Tokyo
    let rule = RecurrenceRule(frequency: .daily, end: .until(until))
    #expect(rule.rruleString(allDay: false, in: tokyo) == "FREQ=DAILY;UNTIL=20261005T150000Z")
    #expect(rule.rruleString(allDay: true, in: tokyo) == "FREQ=DAILY;UNTIL=20261006")
}

@Test func untilRoundTripsThroughParseAndRender() throws {
    let timed = try RecurrenceRule(rrule: "FREQ=WEEKLY;UNTIL=20261005T150000Z")
    #expect(timed.end == .until(instant("2026-10-05T15:00:00Z")))
    #expect(timed.rruleString(allDay: false, in: nil) == "FREQ=WEEKLY;UNTIL=20261005T150000Z")
    let allDay = try RecurrenceRule(rrule: "FREQ=WEEKLY;UNTIL=20261006", in: tokyo)
    #expect(allDay.end == .until(instant("2026-10-05T15:00:00Z")))
    #expect(allDay.rruleString(allDay: true, in: tokyo) == "FREQ=WEEKLY;UNTIL=20261006")
}

@Test func rejectsWhatIsOutsideTheSubset() async {
    for text in ["FREQ=MONTHLY;BYSETPOS=1;BYDAY=MO", "FREQ=HOURLY", "FREQ=DAILY;BYHOUR=9", "FREQ=DAILY;COUNT=3;UNTIL=20261005",
                 "FREQ=WEEKLY;WKST=SU", "FREQ=WEEKLY;BYYEARDAY=3", "FREQ=WEEKLY;UNTIL=20261005T150000"] {
        await expectWriteError(.unsupported(fields: [.recurrence])) { _ = try RecurrenceRule(rrule: text) }
    }
}

@Test func rejectsMalformedText() async {
    await expectWriteError(.invalid("RRULE has no FREQ")) { _ = try RecurrenceRule(rrule: "INTERVAL=2") }
    await expectWriteError(.invalid("bad INTERVAL: x")) { _ = try RecurrenceRule(rrule: "FREQ=DAILY;INTERVAL=x") }
}

@Test func validationCatchesImpossibleRules() async {
    await expectWriteError(.invalid("recurrence interval must be at least 1")) {
        try RecurrenceRule(frequency: .daily, interval: 0).validate()
    }
    await expectWriteError(.invalid("an ordinal weekday needs a monthly or yearly rule")) {
        try RecurrenceRule(frequency: .weekly, weekdays: [.init(.monday, ordinal: 1)]).validate()
    }
    await expectWriteError(.invalid("weekday ordinal must be 1...5 or -5...-1")) {
        try RecurrenceRule(frequency: .monthly, weekdays: [.init(.monday, ordinal: 6)]).validate()
    }
    await expectWriteError(.invalid("month days apply to monthly and yearly rules")) {
        try RecurrenceRule(frequency: .weekly, monthDays: [1]).validate()
    }
    await expectWriteError(.invalid("months apply to yearly rules only")) {
        try RecurrenceRule(frequency: .monthly, months: [3]).validate()
    }
    await expectWriteError(.invalid("recurrence count must be at least 1")) {
        try RecurrenceRule(frequency: .daily, end: .count(0)).validate()
    }
    #expect(throws: Never.self) { try RecurrenceRule(frequency: .weekly, weekdays: [.init(.monday)], end: .count(3)).validate() }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `swift test --package-path Packages/CalendarConnectors --filter RecurrenceRuleTests`
Expected: FAIL to compile (`cannot find 'RecurrenceRule' in scope`).

- [ ] **Step 3: Implement**

```swift
// Sources/CalendarCore/Write/RecurrenceRule.swift
import Foundation

/// A recurrence rule in the RFC 5545 subset every connector can express. Rules outside it are rejected, not mangled.
/// EXDATE and RDATE are not authorable: delete one occurrence with `RecurrenceScope.thisInstance`.
public struct RecurrenceRule: Hashable, Sendable {
    public enum Frequency: String, Sendable, Hashable { case daily = "DAILY", weekly = "WEEKLY", monthly = "MONTHLY", yearly = "YEARLY" }
    public enum Weekday: String, Sendable, Hashable, CaseIterable {
        case monday = "MO", tuesday = "TU", wednesday = "WE", thursday = "TH", friday = "FR", saturday = "SA", sunday = "SU"
    }
    public struct WeekdayOccurrence: Hashable, Sendable {
        public var weekday: Weekday
        /// 1...5 or -5...-1 ("second Tuesday", "last Friday"); monthly and yearly rules only.
        public var ordinal: Int?
        public init(_ weekday: Weekday, ordinal: Int? = nil) {
            self.weekday = weekday
            self.ordinal = ordinal
        }
    }
    public enum End: Hashable, Sendable {
        case never, count(Int)
        /// An instant. Rendered as a UTC date-time for timed events and as a date in the event's zone for all-day ones.
        case until(Date)
    }

    public var frequency: Frequency
    public var interval: Int
    public var weekdays: [WeekdayOccurrence]
    public var monthDays: [Int]
    public var months: [Int]
    public var end: End

    public init(frequency: Frequency, interval: Int = 1, weekdays: [WeekdayOccurrence] = [], monthDays: [Int] = [],
                months: [Int] = [], end: End = .never) {
        self.frequency = frequency
        self.interval = interval
        self.weekdays = weekdays
        self.monthDays = monthDays
        self.months = months
        self.end = end
    }

    public func validate() throws {
        guard interval >= 1 else { throw WriteError.invalid("recurrence interval must be at least 1") }
        if case .count(let n) = end, n < 1 { throw WriteError.invalid("recurrence count must be at least 1") }
        if frequency == .daily && !weekdays.isEmpty { throw WriteError.unsupported(fields: [.recurrence]) }
        for occurrence in weekdays {
            guard let ordinal = occurrence.ordinal else { continue }
            guard frequency == .monthly || frequency == .yearly else {
                throw WriteError.invalid("an ordinal weekday needs a monthly or yearly rule")
            }
            guard ordinal != 0, abs(ordinal) <= 5 else { throw WriteError.invalid("weekday ordinal must be 1...5 or -5...-1") }
        }
        if !monthDays.isEmpty {
            guard frequency == .monthly || frequency == .yearly else {
                throw WriteError.invalid("month days apply to monthly and yearly rules")
            }
            guard monthDays.allSatisfy({ $0 != 0 && abs($0) <= 31 }) else { throw WriteError.invalid("month days must be 1...31 or -31...-1") }
        }
        if !months.isEmpty {
            guard frequency == .yearly else { throw WriteError.invalid("months apply to yearly rules only") }
            guard months.allSatisfy({ (1...12).contains($0) }) else { throw WriteError.invalid("months must be 1...12") }
        }
    }

    // MARK: Parsing

    /// Accepts an optional `RRULE:` prefix. A date-only `UNTIL` is read as the start of that day in `zone` (default UTC).
    /// Anything outside the subset throws `.unsupported([.recurrence])`; malformed text throws `.invalid`.
    public init(rrule text: String, in zone: TimeZone? = nil) throws {
        var body = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if body.uppercased().hasPrefix("RRULE:") { body = String(body.dropFirst(6)) }
        var parts: [String: String] = [:]
        for piece in body.split(separator: ";") {
            let pair = piece.split(separator: "=", maxSplits: 1).map(String.init)
            guard pair.count == 2, !pair[1].isEmpty else { throw WriteError.invalid("malformed RRULE part: \(piece)") }
            parts[pair[0].uppercased()] = pair[1]
        }
        let supported: Set<String> = ["FREQ", "INTERVAL", "COUNT", "UNTIL", "BYDAY", "BYMONTHDAY", "BYMONTH", "WKST"]
        guard Set(parts.keys).isSubset(of: supported) else { throw WriteError.unsupported(fields: [.recurrence]) }
        guard let freqText = parts["FREQ"] else { throw WriteError.invalid("RRULE has no FREQ") }
        guard let frequency = Frequency(rawValue: freqText.uppercased()) else { throw WriteError.unsupported(fields: [.recurrence]) }
        if let wkst = parts["WKST"], wkst.uppercased() != "MO" { throw WriteError.unsupported(fields: [.recurrence]) }
        if parts["COUNT"] != nil && parts["UNTIL"] != nil { throw WriteError.unsupported(fields: [.recurrence]) }

        var interval = 1
        if let text = parts["INTERVAL"] {
            guard let value = Int(text) else { throw WriteError.invalid("bad INTERVAL: \(text)") }
            interval = value
        }
        var end = End.never
        if let text = parts["COUNT"] {
            guard let value = Int(text) else { throw WriteError.invalid("bad COUNT: \(text)") }
            end = .count(value)
        }
        if let text = parts["UNTIL"] { end = .until(try Self.parseUntil(text, zone: zone)) }

        self.init(
            frequency: frequency, interval: interval,
            weekdays: try parts["BYDAY"].map { try $0.split(separator: ",").map { try Self.parseWeekday(String($0)) } } ?? [],
            monthDays: try parts["BYMONTHDAY"].map { try Self.parseInts($0, name: "BYMONTHDAY") } ?? [],
            months: try parts["BYMONTH"].map { try Self.parseInts($0, name: "BYMONTH") } ?? [],
            end: end)
        try validate()
    }

    private static func parseInts(_ text: String, name: String) throws -> [Int] {
        try text.split(separator: ",").map {
            guard let value = Int($0) else { throw WriteError.invalid("bad \(name) value: \($0)") }
            return value
        }
    }

    private static func parseWeekday(_ token: String) throws -> WeekdayOccurrence {
        let upper = token.uppercased()
        guard upper.count >= 2, let weekday = Weekday(rawValue: String(upper.suffix(2))) else {
            throw WriteError.invalid("bad BYDAY value: \(token)")
        }
        let prefix = String(upper.dropLast(2))
        if prefix.isEmpty { return WeekdayOccurrence(weekday) }
        guard let ordinal = Int(prefix) else { throw WriteError.invalid("bad BYDAY value: \(token)") }
        return WeekdayOccurrence(weekday, ordinal: ordinal)
    }

    private static let utc = TimeZone(identifier: "UTC")!

    private static func parseUntil(_ text: String, zone: TimeZone?) throws -> Date {
        let upper = text.uppercased()
        var calendar = Calendar(identifier: .gregorian)
        func number(_ range: Range<Int>) -> Int? {
            let chars = Array(upper)
            return Int(String(chars[range]))
        }
        if upper.count == 8, upper.allSatisfy(\.isNumber) {
            calendar.timeZone = zone ?? utc
            guard let date = calendar.date(from: DateComponents(year: number(0..<4), month: number(4..<6), day: number(6..<8))) else {
                throw WriteError.invalid("bad UNTIL: \(text)")
            }
            return date
        }
        if upper.count == 16, upper.hasSuffix("Z"), Array(upper)[8] == "T" {
            calendar.timeZone = utc
            let parts = DateComponents(year: number(0..<4), month: number(4..<6), day: number(6..<8),
                                       hour: number(9..<11), minute: number(11..<13), second: number(13..<15))
            guard let date = calendar.date(from: parts) else { throw WriteError.invalid("bad UNTIL: \(text)") }
            return date
        }
        throw WriteError.unsupported(fields: [.recurrence])   // a floating date-time has no zone to resolve against
    }

    // MARK: Rendering

    /// The RRULE value without the `RRULE:` prefix, in a fixed order.
    public func rruleString(allDay: Bool, in zone: TimeZone?) -> String {
        var parts = ["FREQ=\(frequency.rawValue)"]
        if interval > 1 { parts.append("INTERVAL=\(interval)") }
        if !weekdays.isEmpty {
            parts.append("BYDAY=" + weekdays.map { ($0.ordinal.map(String.init) ?? "") + $0.weekday.rawValue }.joined(separator: ","))
        }
        if !monthDays.isEmpty { parts.append("BYMONTHDAY=" + monthDays.map(String.init).joined(separator: ",")) }
        if !months.isEmpty { parts.append("BYMONTH=" + months.map(String.init).joined(separator: ",")) }
        switch end {
        case .never: break
        case .count(let n): parts.append("COUNT=\(n)")
        case .until(let date): parts.append("UNTIL=" + Self.untilText(date, allDay: allDay, zone: zone))
        }
        return parts.joined(separator: ";")
    }

    /// `yyyyMMdd` in `zone` for all-day events, `yyyyMMdd'T'HHmmss'Z'` in UTC for timed ones.
    public static func untilText(_ date: Date, allDay: Bool, zone: TimeZone?) -> String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = allDay ? (zone ?? utc) : utc
        let c = calendar.dateComponents([.year, .month, .day, .hour, .minute, .second], from: date)
        if allDay { return String(format: "%04d%02d%02d", c.year!, c.month!, c.day!) }
        return String(format: "%04d%02d%02dT%02d%02d%02dZ", c.year!, c.month!, c.day!, c.hour!, c.minute!, c.second!)
    }

    public static func dateText(_ date: CalendarDate) -> String {
        String(format: "%04d%02d%02d", date.year, date.month, date.day)
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `swift test --package-path Packages/CalendarConnectors --filter RecurrenceRuleTests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "feat(core): RecurrenceRule, a structured RRULE subset" -m "Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 4: `EventTiming`, `AttendeeDraft`, `EventDraft` (+ `copying`)

**Files:**
- Create: `Packages/CalendarConnectors/Sources/CalendarCore/Write/EventDraft.swift`
- Test: `Packages/CalendarConnectors/Tests/CalendarCoreTests/EventDraftTests.swift`

**Interfaces:**
- Consumes: `RecurrenceRule` (Task 3), `EventField`, `ConferenceRequest`, `WriteError`, `SourceCapabilities` (Task 2), `AllDay`.
- Produces: `EventTiming(start:end:timeZone:isAllDay:)` + `validate()`; `AttendeeDraft(email:name:role:)`; `EventDraft(title:timing:notes:location:availability:visibility:reminders:attendees:conference:recurrence:)` + `validate()`, `usedFields`, `init(copying:for:)`.

- [ ] **Step 1: Write the failing tests**

```swift
import Foundation
import Testing
@testable import CalendarCore

private let utc = TimeZone(identifier: "UTC")!
private let newYork = TimeZone(identifier: "America/New_York")!
private func instant(_ s: String) -> Date { ISO8601DateFormatter().date(from: s)! }
private func timed() -> EventTiming {
    EventTiming(start: instant("2026-09-21T10:00:00Z"), end: instant("2026-09-21T11:00:00Z"), timeZone: utc, isAllDay: false)
}

@Test func timingRejectsEndBeforeStartAndNonCanonicalAllDay() async {
    await expectWriteError(.invalid("end must be after start")) {
        try EventTiming(start: instant("2026-09-21T11:00:00Z"), end: instant("2026-09-21T10:00:00Z"), timeZone: utc, isAllDay: false).validate()
    }
    await expectWriteError(.invalid("an all-day event needs a time zone")) {
        try EventTiming(start: instant("2026-09-21T00:00:00Z"), end: instant("2026-09-22T00:00:00Z"), timeZone: nil, isAllDay: true).validate()
    }
    await expectWriteError(.invalid("all-day times must be midnight in America/New_York")) {
        try EventTiming(start: instant("2026-09-21T00:00:00Z"), end: instant("2026-09-22T00:00:00Z"), timeZone: newYork, isAllDay: true).validate()
    }
    #expect(throws: Never.self) {
        try EventTiming(start: instant("2026-09-21T04:00:00Z"), end: instant("2026-09-22T04:00:00Z"), timeZone: newYork, isAllDay: true).validate()
    }
}

@Test func attendeeDraftNormalizesEmail() {
    #expect(AttendeeDraft(email: "  Ann@Example.COM ").email == "ann@example.com")
}

@Test func draftValidationChecksTimingAttendeesRemindersAndRecurrence() async {
    let base = EventDraft(title: "T", timing: timed())
    #expect(throws: Never.self) { try base.validate() }
    var noAt = base; noAt.attendees = [AttendeeDraft(email: "nobody")]
    await expectWriteError(.invalid("attendee email is not valid: nobody")) { try noAt.validate() }
    var negative = base; negative.reminders = [Reminder(minutesBefore: -5)]
    await expectWriteError(.invalid("reminder minutes must not be negative")) { try negative.validate() }
    var badRule = base; badRule.recurrence = RecurrenceRule(frequency: .daily, interval: 0)
    await expectWriteError(.invalid("recurrence interval must be at least 1")) { try badRule.validate() }
}

@Test func usedFieldsListsWhatTheDraftSetsBeyondDefaults() {
    #expect(EventDraft(title: "T", timing: timed()).usedFields == [.title, .timing])
    var rich = EventDraft(title: "T", timing: timed(), notes: "n", location: "l", availability: .free, visibility: .privateEvent,
                          reminders: [], attendees: [AttendeeDraft(email: "a@b.c")], conference: .generate,
                          recurrence: RecurrenceRule(frequency: .daily))
    #expect(rich.usedFields == Set(EventField.allCases))
    rich.reminders = nil
    #expect(!rich.usedFields.contains(.reminders))
}

private func source() -> CalendarEvent {
    CalendarEvent(
        eventID: "e", calendarID: "c", title: "Planning", notes: "n", location: "Room",
        start: instant("2026-09-21T10:00:00Z"), end: instant("2026-09-21T11:00:00Z"), timeZone: utc,
        availability: .free, visibility: .privateEvent, seriesID: "s",
        attendees: [Attendee(email: "me@x.com", isSelf: true), Attendee(email: "Bob@x.com", role: .optional),
                    Attendee(name: "No Email")],
        conference: ConferenceInfo(url: URL(string: "https://meet.google.com/abc")!, provider: .meet),
        reminders: [Reminder(minutesBefore: 10)])
}

@Test func copyingToAFullyWritableTargetKeepsEverythingRepresentable() {
    let caps = SourceCapabilities(canWrite: true, canEditAttendees: true, writableFields: Set(EventField.allCases))
    let draft = EventDraft(copying: source(), for: caps)
    #expect(draft.title == "Planning" && draft.notes == "n" && draft.location == "Room")
    #expect(draft.timing == EventTiming(start: instant("2026-09-21T10:00:00Z"), end: instant("2026-09-21T11:00:00Z"), timeZone: utc, isAllDay: false))
    #expect(draft.availability == .free && draft.visibility == .privateEvent)
    #expect(draft.reminders == [Reminder(minutesBefore: 10)])
    #expect(draft.attendees == [AttendeeDraft(email: "bob@x.com", role: .optional)])   // self and email-less attendees dropped
    #expect(draft.conference == .generate)   // only a Meet link is re-requested
    #expect(draft.recurrence == nil)
}

@Test func copyingDropsFieldsTheTargetCannotWrite() {
    let caps = SourceCapabilities(canWrite: true, writableFields: [.title, .notes, .location, .timing, .availability, .reminders, .recurrence])
    let draft = EventDraft(copying: source(), for: caps)
    #expect(draft.attendees.isEmpty && draft.visibility == .default && draft.conference == .none)
    #expect(draft.notes == "n" && draft.availability == .free)
}

@Test func copyingTurnsEmptyRemindersIntoCalendarDefaults() {
    var event = source()
    event.reminders = []
    let draft = EventDraft(copying: event, for: SourceCapabilities(canWrite: true, writableFields: Set(EventField.allCases)))
    #expect(draft.reminders == nil)
}

@Test func copyingToAReadOnlyTargetKeepsOnlyTheRequiredParts() {
    let draft = EventDraft(copying: source(), for: SourceCapabilities())
    #expect(draft.title == "Planning" && draft.notes == nil && draft.location == nil && draft.reminders == nil)
}
```

- [ ] **Step 2: Run to verify failure**

Run: `swift test --package-path Packages/CalendarConnectors --filter EventDraftTests`
Expected: FAIL to compile (`cannot find 'EventTiming' in scope`).

- [ ] **Step 3: Implement**

```swift
// Sources/CalendarCore/Write/EventDraft.swift
import Foundation

/// Start, end, zone and all-day as one unit, so a write can never carry half a time range.
/// All-day timings use the library's canonical form (midnight of the first day in `timeZone`, `end` exclusive).
public struct EventTiming: Sendable, Equatable {
    public var start: Date
    public var end: Date
    public var timeZone: TimeZone?
    public var isAllDay: Bool

    public init(start: Date, end: Date, timeZone: TimeZone?, isAllDay: Bool) {
        self.start = start
        self.end = end
        self.timeZone = timeZone
        self.isAllDay = isAllDay
    }

    /// Every write calls this before any request or store mutation.
    public func validate() throws {
        guard end > start else { throw WriteError.invalid("end must be after start") }
        guard isAllDay else { return }
        guard let zone = timeZone else { throw WriteError.invalid("an all-day event needs a time zone") }
        for instant in [start, end] where AllDay.startOfDay(AllDay.date(of: instant, in: zone), in: zone) != instant {
            throw WriteError.invalid("all-day times must be midnight in \(zone.identifier)")
        }
    }
}

public struct AttendeeDraft: Sendable, Hashable {
    public var name: String?
    public private(set) var email: String
    public var role: AttendeeRole

    public init(email: String, name: String? = nil, role: AttendeeRole = .required) {
        self.email = email.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        self.name = name
        self.role = role
    }
}

/// Everything needed to create an event. `EventDraft(copying:for:)` is the one lenient path.
public struct EventDraft: Sendable, Equatable {
    public var title: String
    public var notes: String?
    public var location: String?
    public var timing: EventTiming
    public var availability: Availability
    public var visibility: Visibility
    /// nil = the calendar's default reminders; empty = none.
    public var reminders: [Reminder]?
    public var attendees: [AttendeeDraft]
    public var conference: ConferenceRequest
    public var recurrence: RecurrenceRule?

    public init(
        title: String, timing: EventTiming, notes: String? = nil, location: String? = nil,
        availability: Availability = .busy, visibility: Visibility = .default, reminders: [Reminder]? = nil,
        attendees: [AttendeeDraft] = [], conference: ConferenceRequest = .none, recurrence: RecurrenceRule? = nil
    ) {
        self.title = title
        self.timing = timing
        self.notes = notes
        self.location = location
        self.availability = availability
        self.visibility = visibility
        self.reminders = reminders
        self.attendees = attendees
        self.conference = conference
        self.recurrence = recurrence
    }

    public func validate() throws {
        try timing.validate()
        try recurrence?.validate()
        for attendee in attendees where !attendee.email.contains("@") {
            throw WriteError.invalid("attendee email is not valid: \(attendee.email)")
        }
        if let reminders, reminders.contains(where: { $0.minutesBefore < 0 }) {
            throw WriteError.invalid("reminder minutes must not be negative")
        }
    }

    /// The fields this draft sets beyond their defaults; connectors check them against `writableFields`.
    public var usedFields: Set<EventField> {
        var fields: Set<EventField> = [.title, .timing]
        if notes != nil { fields.insert(.notes) }
        if location != nil { fields.insert(.location) }
        if availability != .busy { fields.insert(.availability) }
        if visibility != .default { fields.insert(.visibility) }
        if reminders != nil { fields.insert(.reminders) }
        if !attendees.isEmpty { fields.insert(.attendees) }
        if conference != .none { fields.insert(.conference) }
        if recurrence != nil { fields.insert(.recurrence) }
        return fields
    }

    /// A best-effort copy of `event` for a target with `capabilities`: only fields in `writableFields` are carried
    /// over. Self and email-less attendees are dropped, an empty reminder list becomes "calendar defaults" (reads
    /// cannot tell the two apart), only a Meet link is re-requested, and recurrence is never copied (reads carry none).
    public init(copying event: CalendarEvent, for capabilities: SourceCapabilities) {
        let writable = capabilities.writableFields
        self.init(
            title: event.title,
            timing: EventTiming(start: event.start, end: event.end, timeZone: event.timeZone, isAllDay: event.isAllDay),
            notes: writable.contains(.notes) ? event.notes : nil,
            location: writable.contains(.location) ? event.location : nil,
            availability: writable.contains(.availability) ? event.availability : .busy,
            visibility: writable.contains(.visibility) ? event.visibility : .default,
            reminders: writable.contains(.reminders) && !event.reminders.isEmpty ? event.reminders : nil,
            attendees: writable.contains(.attendees)
                ? event.attendees.filter { !$0.isSelf }.compactMap { a in a.email.map { AttendeeDraft(email: $0, name: a.name, role: a.role) } }
                : [],
            conference: writable.contains(.conference) && event.conference?.provider == .meet ? .generate : .none,
            recurrence: nil)
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `swift test --package-path Packages/CalendarConnectors --filter EventDraftTests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "feat(core): EventTiming, AttendeeDraft and EventDraft with best-effort copying" -m "Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 5: `EventPatch`, `FieldUpdate`, `AttendeeChanges`, `EventEdit`

**Files:**
- Create: `Packages/CalendarConnectors/Sources/CalendarCore/Write/EventPatch.swift`
- Test: `Packages/CalendarConnectors/Tests/CalendarCoreTests/EventPatchTests.swift`

**Interfaces:**
- Consumes: `EventTiming`, `AttendeeDraft` (Task 4), `RecurrenceRule` (Task 3), `EventField`, `ConferenceChange` (Task 2).
- Produces: `FieldUpdate<Value>` (`.keep/.set/.clear`); `AttendeeChanges(add:remove:)` + `isEmpty`; `EventPatch` with public init (all parameters default), `touchedFields`, `isEmpty`, `base`, `withoutBase()`, `init(from:to:)`, `applied(to:)`; `EventEdit(_:)` with `event`, `patch`, `hasChanges`.

- [ ] **Step 1: Write the failing tests**

```swift
import Foundation
import Testing
@testable import CalendarCore

private let utc = TimeZone(identifier: "UTC")!
private func instant(_ s: String) -> Date { ISO8601DateFormatter().date(from: s)! }

private func original() -> CalendarEvent {
    CalendarEvent(
        eventID: "e", uid: "u", calendarID: "c", title: "Planning", notes: "n", location: "Room",
        start: instant("2026-09-21T10:00:00Z"), end: instant("2026-09-21T11:00:00Z"), timeZone: utc,
        status: .confirmed, attendees: [Attendee(email: "me@x.com", isSelf: true), Attendee(email: "bob@x.com"),
                                        Attendee(email: "cy@x.com", role: .optional)],
        organizer: Attendee(email: "me@x.com", isSelf: true, isOrganizer: true),
        conference: ConferenceInfo(url: URL(string: "https://meet.google.com/abc")!, provider: .meet),
        reminders: [Reminder(minutesBefore: 10)], url: URL(string: "https://x.test/e")!, version: "v1", myResponse: .accepted)
}

@Test func anUntouchedEditProducesAnEmptyPatch() {
    let edit = EventEdit(original())
    #expect(!edit.hasChanges && edit.patch.isEmpty && edit.patch.touchedFields.isEmpty)
}

@Test func diffCapturesScalarChangesAndClears() {
    var edit = EventEdit(original())
    edit.event.title = "Planning v2"
    edit.event.notes = nil
    edit.event.location = "Lab"
    edit.event.availability = .free
    edit.event.visibility = .privateEvent
    let patch = edit.patch
    #expect(patch.title == "Planning v2")
    #expect(patch.notes == .clear && patch.location == .set("Lab"))
    #expect(patch.availability == .free && patch.visibility == .privateEvent)
    #expect(patch.touchedFields == [.title, .notes, .location, .availability, .visibility])
    #expect(patch.base == original())
}

@Test func diffTreatsTimeAsOneUnit() {
    var edit = EventEdit(original())
    edit.event.end = instant("2026-09-21T11:30:00Z")
    let patch = edit.patch
    #expect(patch.timing == EventTiming(start: instant("2026-09-21T10:00:00Z"), end: instant("2026-09-21T11:30:00Z"), timeZone: utc, isAllDay: false))
    #expect(patch.touchedFields == [.timing])
}

@Test func diffBuildsAnAttendeeDeltaByEmail() throws {
    var edit = EventEdit(original())
    edit.event.attendees.removeAll { $0.email == "bob@x.com" }                    // removed
    edit.event.attendees.append(Attendee(name: "Dee", email: "Dee@x.com"))        // added
    if let i = edit.event.attendees.firstIndex(where: { $0.email == "cy@x.com" }) { edit.event.attendees[i].role = .required }   // role changed: upsert
    let changes = try #require(edit.patch.attendees)
    #expect(changes.remove == ["bob@x.com"])
    #expect(changes.add == [AttendeeDraft(email: "cy@x.com", role: .required), AttendeeDraft(email: "dee@x.com", name: "Dee")]
        || changes.add == [AttendeeDraft(email: "dee@x.com", name: "Dee"), AttendeeDraft(email: "cy@x.com", role: .required)])
    #expect(edit.patch.touchedFields == [.attendees])
}

@Test func diffIgnoresProviderOwnedFieldsAndUnwritableConferenceChanges() {
    var edit = EventEdit(original())
    edit.event.status = .tentative
    edit.event.myResponse = .declined
    edit.event.version = "v9"
    edit.event.url = nil
    edit.event.organizer = nil
    edit.event.sourceID = "other"
    edit.event.conference = ConferenceInfo(url: URL(string: "https://zoom.us/j/1")!, provider: .zoom)   // changed, not removable
    #expect(edit.patch.isEmpty)
    edit.event.conference = nil
    #expect(edit.patch.conference == .remove && edit.patch.touchedFields == [.conference])
}

@Test func diffSetsRemindersExplicitly() {
    var edit = EventEdit(original())
    edit.event.reminders = []
    #expect(edit.patch.reminders == .set([]))
    edit.event.reminders = [Reminder(minutesBefore: 5), Reminder(minutesBefore: 30)]
    #expect(edit.patch.reminders == .set([Reminder(minutesBefore: 5), Reminder(minutesBefore: 30)]))
}

@Test func attendeesWithoutAnEmailAreIgnoredByTheDiff() {
    var edit = EventEdit(original())
    edit.event.attendees.append(Attendee(name: "Ghost"))
    #expect(edit.patch.isEmpty)
}

@Test func aHandBuiltPatchHasNoBaseAndReportsTouchedFields() {
    let patch = EventPatch(title: "X", recurrence: .clear, conference: .generate)
    #expect(patch.base == nil && patch.touchedFields == [.title, .recurrence, .conference])
    #expect(EventPatch(attendees: AttendeeChanges(add: [], remove: [])).isEmpty)   // an empty delta touches nothing
    #expect(EventEdit(original()).patch.withoutBase() == EventPatch())
}

@Test func appliedToRewritesEveryRepresentableField() {
    let patch = EventPatch(
        title: "New", notes: .clear, location: .set("Lab"),
        timing: EventTiming(start: instant("2026-09-22T10:00:00Z"), end: instant("2026-09-22T11:00:00Z"), timeZone: utc, isAllDay: false),
        availability: .free, visibility: .confidential, reminders: .set([]),
        attendees: AttendeeChanges(add: [AttendeeDraft(email: "dee@x.com", name: "Dee"), AttendeeDraft(email: "cy@x.com", role: .required)], remove: ["bob@x.com"]),
        conference: .remove)
    let e = patch.applied(to: original())
    #expect(e.title == "New" && e.notes == nil && e.location == "Lab")
    #expect(e.start == instant("2026-09-22T10:00:00Z") && e.availability == .free && e.visibility == .confidential)
    #expect(e.reminders.isEmpty && e.conference == nil)
    #expect(e.attendees.map(\.email) == ["me@x.com", "cy@x.com", "dee@x.com"])
    #expect(e.attendees.first { $0.email == "cy@x.com" }?.role == .required)
}
```

- [ ] **Step 2: Run to verify failure**

Run: `swift test --package-path Packages/CalendarConnectors --filter EventPatchTests`
Expected: FAIL to compile (`cannot find 'EventPatch' in scope`).

- [ ] **Step 3: Implement**

```swift
// Sources/CalendarCore/Write/EventPatch.swift
import Foundation

/// A field change where "leave alone" and "remove" are different things.
public enum FieldUpdate<Value: Sendable & Equatable>: Sendable, Equatable {
    case keep, set(Value), clear
}

/// A delta on an event's attendees, never a replacement list, so an edit cannot wipe other people's responses.
/// `add` upserts by email: an existing attendee keeps their response and takes the new name and role.
public struct AttendeeChanges: Sendable, Equatable {
    public var add: [AttendeeDraft]
    /// Normalized (trimmed, lowercased) emails.
    public var remove: [String]

    public init(add: [AttendeeDraft] = [], remove: [String] = []) {
        self.add = add
        self.remove = remove.map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
    }

    public var isEmpty: Bool { add.isEmpty && remove.isEmpty }
}

/// The changes to make to an existing event. Only these fields are sent, so data the library does not model
/// (attachments, extended properties, colors) is never disturbed.
public struct EventPatch: Sendable, Equatable {
    public var title: String?
    public var notes: FieldUpdate<String>
    public var location: FieldUpdate<String>
    /// Time is one unit: start, end, zone and all-day together.
    public var timing: EventTiming?
    public var availability: Availability?
    public var visibility: Visibility?
    /// `.clear` = the calendar's default reminders; `.set([])` = none.
    public var reminders: FieldUpdate<[Reminder]>
    public var attendees: AttendeeChanges?
    public var recurrence: FieldUpdate<RecurrenceRule>
    public var conference: ConferenceChange?
    /// The original the patch was diffed from; what conflicts are judged against. nil for a hand-built patch,
    /// which conflicts on any stale version.
    public private(set) var base: CalendarEvent?

    public init(
        title: String? = nil, notes: FieldUpdate<String> = .keep, location: FieldUpdate<String> = .keep,
        timing: EventTiming? = nil, availability: Availability? = nil, visibility: Visibility? = nil,
        reminders: FieldUpdate<[Reminder]> = .keep, attendees: AttendeeChanges? = nil,
        recurrence: FieldUpdate<RecurrenceRule> = .keep, conference: ConferenceChange? = nil
    ) {
        self.title = title
        self.notes = notes
        self.location = location
        self.timing = timing
        self.availability = availability
        self.visibility = visibility
        self.reminders = reminders
        self.attendees = attendees
        self.recurrence = recurrence
        self.conference = conference
        self.base = nil
    }

    public var touchedFields: Set<EventField> {
        var fields = Set<EventField>()
        if title != nil { fields.insert(.title) }
        if notes != .keep { fields.insert(.notes) }
        if location != .keep { fields.insert(.location) }
        if timing != nil { fields.insert(.timing) }
        if availability != nil { fields.insert(.availability) }
        if visibility != nil { fields.insert(.visibility) }
        if reminders != .keep { fields.insert(.reminders) }
        if let attendees, !attendees.isEmpty { fields.insert(.attendees) }
        if recurrence != .keep { fields.insert(.recurrence) }
        if conference != nil { fields.insert(.conference) }
        return fields
    }

    public var isEmpty: Bool { touchedFields.isEmpty }

    public func withoutBase() -> EventPatch {
        var copy = self
        copy.base = nil
        return copy
    }

    /// The minimal patch that turns `original` into `edited`. Compares title, notes, location, timing, availability,
    /// visibility, reminders and attendees (by normalized email; a changed role or name is an upsert), plus the
    /// removal of a conference. Ignores provider-owned fields (ids, uid, calendar, source, organizer, status, kind,
    /// url, version, series fields, `myResponse`, attendee responses and flags), a changed or added conference
    /// (only `.generate` and `.remove` are writable) and recurrence (reads carry none).
    public init(from original: CalendarEvent, to edited: CalendarEvent) {
        self.init()
        if edited.title != original.title { title = edited.title }
        notes = Self.update(from: original.notes, to: edited.notes)
        location = Self.update(from: original.location, to: edited.location)
        let sameTiming = original.start == edited.start && original.end == edited.end
            && original.timeZone?.identifier == edited.timeZone?.identifier && original.isAllDay == edited.isAllDay
        if !sameTiming {
            timing = EventTiming(start: edited.start, end: edited.end, timeZone: edited.timeZone, isAllDay: edited.isAllDay)
        }
        if edited.availability != original.availability { availability = edited.availability }
        if edited.visibility != original.visibility { visibility = edited.visibility }
        if edited.reminders != original.reminders { reminders = .set(edited.reminders) }
        attendees = Self.attendeeChanges(from: original.attendees, to: edited.attendees)
        if original.conference != nil && edited.conference == nil { conference = .remove }
        base = original
    }

    private static func update(from old: String?, to new: String?) -> FieldUpdate<String> {
        guard old != new else { return .keep }
        return new.map { .set($0) } ?? .clear
    }

    private static func attendeeChanges(from original: [Attendee], to edited: [Attendee]) -> AttendeeChanges? {
        let before = Dictionary(original.filter { !$0.isSelf }.compactMap { a in a.email.map { ($0, a) } },
                                uniquingKeysWith: { first, _ in first })
        var add: [AttendeeDraft] = []
        var seen = Set<String>()
        for attendee in edited where !attendee.isSelf {
            guard let email = attendee.email, seen.insert(email).inserted else { continue }
            if let old = before[email], old.role == attendee.role, old.name == attendee.name { continue }
            add.append(AttendeeDraft(email: email, name: attendee.name, role: attendee.role))
        }
        let remove = before.keys.filter { !seen.contains($0) }.sorted()
        let changes = AttendeeChanges(add: add, remove: remove)
        return changes.isEmpty ? nil : changes
    }

    /// The event with this patch applied. Recurrence and a generated conference are not representable on
    /// `CalendarEvent`; `.remove` clears the conference. Used by the in-memory test source.
    public func applied(to event: CalendarEvent) -> CalendarEvent {
        var e = event
        if let title { e.title = title }
        switch notes { case .keep: break; case .set(let v): e.notes = v; case .clear: e.notes = nil }
        switch location { case .keep: break; case .set(let v): e.location = v; case .clear: e.location = nil }
        if let timing {
            e.start = timing.start
            e.end = timing.end
            e.timeZone = timing.timeZone
            e.isAllDay = timing.isAllDay
        }
        if let availability { e.availability = availability }
        if let visibility { e.visibility = visibility }
        switch reminders { case .keep: break; case .set(let v): e.reminders = v; case .clear: e.reminders = [] }
        if let attendees {
            let removed = Set(attendees.remove)
            e.attendees.removeAll { $0.email.map(removed.contains) ?? false }
            for draft in attendees.add {
                if let i = e.attendees.firstIndex(where: { $0.email == draft.email }) {
                    if let name = draft.name { e.attendees[i].name = name }
                    e.attendees[i].role = draft.role
                } else {
                    e.attendees.append(Attendee(name: draft.name, email: draft.email, role: draft.role))
                }
            }
        }
        if conference == .remove { e.conference = nil }
        return e
    }
}

/// A tracked edit: the event as read plus a working copy. `patch` is what changed, `hasChanges` whether anything did.
public struct EventEdit: Sendable {
    public let original: CalendarEvent
    public var event: CalendarEvent

    public init(_ original: CalendarEvent) {
        self.original = original
        self.event = original
    }

    public var patch: EventPatch { EventPatch(from: original, to: event) }
    public var hasChanges: Bool { !patch.isEmpty }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `swift test --package-path Packages/CalendarConnectors --filter EventPatchTests`
Expected: PASS. (In `diffBuildsAnAttendeeDeltaByEmail` the order of `add` follows `edited`'s order, which puts `cy@x.com` first; both orders are accepted by the assertion.)

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "feat(core): EventPatch, attendee deltas, diffing and EventEdit" -m "Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 6: `PatchMerge` (field-level conflicts and the retry loop)

**Files:**
- Create: `Packages/CalendarConnectors/Sources/CalendarCore/Write/PatchMerge.swift`
- Test: `Packages/CalendarConnectors/Tests/CalendarCoreTests/PatchMergeTests.swift`

**Interfaces:**
- Consumes: `EventPatch` (Task 5), `WriteError`, `EventField`.
- Produces: `PatchMerge.conflicts(patch:current:) -> Set<EventField>`; `PatchMerge.Attempt<Result>` (`.done(Result)`, `.stale`); `PatchMerge.apply(patch:version:maxAttempts:fetchCurrent:write:) async throws -> Result`.

- [ ] **Step 1: Write the failing tests**

```swift
import Foundation
import Testing
@testable import CalendarCore

private let utc = TimeZone(identifier: "UTC")!
private func instant(_ s: String) -> Date { ISO8601DateFormatter().date(from: s)! }

private func base() -> CalendarEvent {
    CalendarEvent(eventID: "e", calendarID: "c", title: "Planning", notes: "n", location: "Room",
                  start: instant("2026-09-21T10:00:00Z"), end: instant("2026-09-21T11:00:00Z"), timeZone: utc,
                  attendees: [Attendee(email: "bob@x.com", role: .required)], version: "v1")
}
private func patch(_ mutate: (inout EventEdit) -> Void) -> EventPatch {
    var edit = EventEdit(base())
    mutate(&edit)
    return edit.patch
}

@Test func noConflictWhenOthersChangedDifferentFields() {
    var current = base()
    current.location = "Elsewhere"
    current.version = "v2"
    #expect(PatchMerge.conflicts(patch: patch { $0.event.title = "New" }, current: current).isEmpty)
}

@Test func aTouchedFieldThatChangedElsewhereConflicts() {
    var current = base()
    current.title = "Changed"
    #expect(PatchMerge.conflicts(patch: patch { $0.event.title = "New" }, current: current) == [.title])
}

@Test func timingIsComparedAsAUnit() {
    var current = base()
    current.end = instant("2026-09-21T11:30:00Z")   // someone moved only the end
    #expect(PatchMerge.conflicts(patch: patch { $0.event.start = instant("2026-09-21T09:30:00Z") }, current: current) == [.timing])
}

@Test func attendeesConflictOnlyForTheEmailsThePatchTouches() {
    var current = base()
    current.attendees.append(Attendee(email: "new@x.com"))          // unrelated addition
    let addDee = patch { $0.event.attendees.append(Attendee(email: "dee@x.com")) }
    #expect(PatchMerge.conflicts(patch: addDee, current: current).isEmpty)
    current.attendees[0].role = .optional                            // someone changed bob's role
    let changeBob = patch { $0.event.attendees[0].role = .resource }
    #expect(PatchMerge.conflicts(patch: changeBob, current: current) == [.attendees])
    let removeBob = patch { $0.event.attendees.removeAll() }
    #expect(PatchMerge.conflicts(patch: removeBob, current: current) == [.attendees])
}

@Test func aPatchWithoutABaseConflictsOnEveryTouchedField() {
    let handBuilt = EventPatch(title: "X", location: .set("Y"))
    #expect(PatchMerge.conflicts(patch: handBuilt, current: base()) == [.title, .location])
}

@Test func recurrenceCannotBeJudgedSoItConflicts() {
    var edit = EventEdit(base())
    edit.event.title = "New"
    var p = edit.patch
    p.recurrence = .clear
    #expect(PatchMerge.conflicts(patch: p, current: base()) == [.recurrence])
}

@Test func applyReturnsTheFirstSuccessfulWrite() async throws {
    let result = try await PatchMerge.apply(patch: EventPatch(title: "X"), version: "v1",
                                            fetchCurrent: { base() }, write: { _ in .done("ok") })
    #expect(result == "ok")
}

@Test func applyRetriesOnTheFreshVersionWhenNothingOverlaps() async throws {
    var current = base()
    current.location = "Elsewhere"
    current.version = "v2"
    let seen = Box<[String?]>([])
    let result = try await PatchMerge.apply(
        patch: patch { $0.event.title = "New" }, version: "v1", fetchCurrent: { current },
        write: { version -> Attempt in
            seen.value.append(version)
            return version == "v2" ? .done("saved") : .stale
        })
    #expect(result == "saved" && seen.value == ["v1", "v2"])
}

@Test func applyJudgesEverySecondStaleResultAgain() async {
    let fetched = Box(0)
    await expectWriteError(.conflict(fields: [.title])) {
        _ = try await PatchMerge.apply(
            patch: patch { $0.event.title = "New" }, version: "v1",
            fetchCurrent: {
                fetched.value += 1
                var current = base()
                current.version = "v\(fetched.value + 1)"
                if fetched.value == 2 { current.title = "Changed" }   // the second concurrent edit overlaps
                return current
            },
            write: { _ in Attempt.stale })
    }
    #expect(fetched.value == 2)
}

@Test func applyGivesUpAfterMaxAttempts() async {
    await expectWriteError(.conflict(fields: [.title])) {
        _ = try await PatchMerge.apply(patch: patch { $0.event.title = "New" }, version: "v1", maxAttempts: 3,
                                       fetchCurrent: { base() }, write: { _ in Attempt.stale })
    }
}

@Test func aBaselessPatchFailsAtTheFirstStaleVersion() async {
    let fetched = Box(0)
    await expectWriteError(.conflict(fields: [.title])) {
        _ = try await PatchMerge.apply(patch: EventPatch(title: "X"), version: "v1",
                                       fetchCurrent: { fetched.value += 1; return base() }, write: { _ in Attempt.stale })
    }
    #expect(fetched.value == 1)
}

private typealias Attempt = PatchMerge.Attempt<String>

private final class Box<T>: @unchecked Sendable {
    var value: T
    init(_ value: T) { self.value = value }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `swift test --package-path Packages/CalendarConnectors --filter PatchMergeTests`
Expected: FAIL to compile (`cannot find 'PatchMerge' in scope`).

- [ ] **Step 3: Implement**

```swift
// Sources/CalendarCore/Write/PatchMerge.swift
import Foundation

/// Field-level conflict handling shared by every connector that has a version to compare (Google etag, EventKit
/// modification date). A stale version is not an error by itself: it is a conflict only when someone changed a
/// field this patch touches.
public enum PatchMerge {
    public enum Attempt<Result> {
        case done(Result)
        /// The provider rejected the write because the version is out of date.
        case stale
    }

    /// The touched fields whose value in `current` differs from `patch.base`. A patch without a base cannot be
    /// compared, so every touched field is a conflict. Attendees are compared only for the emails the patch adds or
    /// removes (a concurrent change to someone else is not a conflict). Recurrence cannot be read back, so a patch
    /// that touches it always conflicts on a stale version.
    public static func conflicts(patch: EventPatch, current: CalendarEvent) -> Set<EventField> {
        guard let base = patch.base else { return patch.touchedFields }
        var found = Set<EventField>()
        for field in patch.touchedFields {
            switch field {
            case .title: if current.title != base.title { found.insert(field) }
            case .notes: if current.notes != base.notes { found.insert(field) }
            case .location: if current.location != base.location { found.insert(field) }
            case .timing:
                let same = current.start == base.start && current.end == base.end
                    && current.timeZone?.identifier == base.timeZone?.identifier && current.isAllDay == base.isAllDay
                if !same { found.insert(field) }
            case .availability: if current.availability != base.availability { found.insert(field) }
            case .visibility: if current.visibility != base.visibility { found.insert(field) }
            case .reminders: if current.reminders != base.reminders { found.insert(field) }
            case .attendees: if attendeesDiffer(patch.attendees, base: base, current: current) { found.insert(field) }
            case .recurrence: found.insert(field)
            case .conference: if current.conference?.url != base.conference?.url { found.insert(field) }
            }
        }
        return found
    }

    private static func attendeesDiffer(_ changes: AttendeeChanges?, base: CalendarEvent, current: CalendarEvent) -> Bool {
        guard let changes else { return false }
        let emails = Set(changes.add.map(\.email)).union(changes.remove)
        func role(_ email: String, in event: CalendarEvent) -> AttendeeRole? {
            event.attendees.first { $0.email == email }?.role
        }
        return emails.contains { role($0, in: base) != role($0, in: current) }
    }

    /// Runs `write` with `version`. When it reports `.stale`, fetches the current event and judges the patch against
    /// it: a real overlap throws `.conflict(fields:)`; otherwise the write is retried on the fresh version, and a
    /// further stale result is judged the same way. After `maxAttempts` writes it throws
    /// `.conflict(fields: patch.touchedFields)` (the event is being edited concurrently).
    public static func apply<Result>(
        patch: EventPatch, version: String?, maxAttempts: Int = 3,
        fetchCurrent: () async throws -> CalendarEvent,
        write: (String?) async throws -> Attempt<Result>
    ) async throws -> Result {
        var version = version
        for _ in 0..<maxAttempts {
            switch try await write(version) {
            case .done(let result):
                return result
            case .stale:
                let current = try await fetchCurrent()
                let overlapping = conflicts(patch: patch, current: current)
                if !overlapping.isEmpty { throw WriteError.conflict(fields: overlapping) }
                version = current.version
            }
        }
        throw WriteError.conflict(fields: patch.touchedFields)
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `swift test --package-path Packages/CalendarConnectors --filter PatchMergeTests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "feat(core): PatchMerge, field-level conflict judgment and retry loop" -m "Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 7: `WritableCalendarSource`, `FakeWritableSource` and the conformance checks

**Files:**
- Create: `Packages/CalendarConnectors/Sources/CalendarCore/Write/WritableCalendarSource.swift`
- Create: `Packages/CalendarConnectors/Sources/CalendarTestSupport/FakeWritableSource.swift`, `WritableSourceConformance.swift`
- Test: `Packages/CalendarConnectors/Tests/CalendarCoreTests/WritableSourceTests.swift`

**Interfaces:**
- Consumes: everything from Tasks 2 to 6.
- Produces: `WritableCalendarSource`; `FakeWritableSource(calendarIDs:writableFields:canRespond:)` with `writeCount`, `simulateExternalEdit(calendarID:eventID:_:)`; `WritableSourceConformance.violations(of:calendarID:window:) async -> [String]`.

- [ ] **Step 1: Write the failing tests**

```swift
import CalendarTestSupport
import Foundation
import Testing
@testable import CalendarCore

private let utc = TimeZone(identifier: "UTC")!
private let window = DateInterval(start: Date(timeIntervalSince1970: 1_790_000_000), end: Date(timeIntervalSince1970: 1_790_000_000 + 86_400 * 3))

private func draft(_ title: String = "Sync") -> EventDraft {
    let start = window.start.addingTimeInterval(3600)
    return EventDraft(title: title, timing: EventTiming(start: start, end: start.addingTimeInterval(1800), timeZone: utc, isAllDay: false))
}

@Test func theFakePassesTheConformanceChecks() async {
    let source = FakeWritableSource()
    #expect(await WritableSourceConformance.violations(of: source, calendarID: "cal", window: window).isEmpty)
}

@Test func aFakeWithoutAttendeeSupportStillConforms() async {
    let source = FakeWritableSource(writableFields: [.title, .notes, .location, .timing], canRespond: false)
    #expect(!source.capabilities.canEditAttendees)
    #expect(await WritableSourceConformance.violations(of: source, calendarID: "cal", window: window).isEmpty)
}

@Test func createReadsBackAndStampsTheSource() async throws {
    let source = FakeWritableSource()
    let created = try await source.create(draft(), in: "cal", notify: .none)
    #expect(created.title == "Sync" && created.calendarID == "cal" && created.sourceID == source.id && created.version != nil)
    #expect(try await source.events(in: window).map(\.eventID) == [created.eventID])
}

@Test func anUnknownCalendarIsNotFoundAndUnwritableFieldsAreRefused() async {
    let source = FakeWritableSource(writableFields: [.title, .timing])
    await expectWriteError(.notFound) { _ = try await source.create(draft(), in: "nope", notify: .none) }
    var withNotes = draft(); withNotes.notes = "n"
    await expectWriteError(.unsupported(fields: [.notes])) { _ = try await source.create(withNotes, in: "cal", notify: .none) }
    #expect(source.writeCount == 0)
}

@Test func anEmptyPatchWritesNothing() async throws {
    let source = FakeWritableSource()
    let created = try await source.create(draft(), in: "cal", notify: .none)
    let before = source.writeCount
    let same = try await source.update(EventRef(created), EventPatch(), scope: .thisInstance, notify: .none)
    #expect(same == created && source.writeCount == before)
}

@Test func aStaleVersionMergesWhenFieldsDoNotOverlap() async throws {
    let source = FakeWritableSource()
    let created = try await source.create(draft(), in: "cal", notify: .none)
    source.simulateExternalEdit(calendarID: "cal", eventID: created.eventID) { $0.location = "Elsewhere" }
    var edit = EventEdit(created)
    edit.event.title = "Renamed"
    let updated = try await source.update(EventRef(created), edit.patch, scope: .thisInstance, notify: .none)
    #expect(updated.title == "Renamed" && updated.location == "Elsewhere")
}

@Test func aStaleVersionConflictsWhenTheSameFieldChanged() async throws {
    let source = FakeWritableSource()
    let created = try await source.create(draft(), in: "cal", notify: .none)
    source.simulateExternalEdit(calendarID: "cal", eventID: created.eventID) { $0.title = "Theirs" }
    var edit = EventEdit(created)
    edit.event.title = "Mine"
    await expectWriteError(.conflict(fields: [.title])) {
        _ = try await source.update(EventRef(created), edit.patch, scope: .thisInstance, notify: .none)
    }
}

@Test func respondIsUnsupportedWhenTheSourceCannotRSVP() async throws {
    let source = FakeWritableSource(canRespond: false)
    let created = try await source.create(draft(), in: "cal", notify: .none)
    await expectWriteError(.unsupported(fields: [.attendees])) {
        _ = try await source.respond(to: EventRef(created), .accepted, scope: .thisInstance, notify: .none)
    }
}

@Test func capabilitiesFollowTheInvariants() {
    let full = FakeWritableSource().capabilities
    #expect(full.canWrite && full.canEditAttendees == full.writableFields.contains(.attendees))
    let limited = FakeWritableSource(writableFields: [.title, .timing]).capabilities
    #expect(!limited.canEditAttendees && limited.writableFields == [.title, .timing])
}
```

- [ ] **Step 2: Run to verify failure**

Run: `swift test --package-path Packages/CalendarConnectors --filter WritableSourceTests`
Expected: FAIL to compile (`cannot find 'FakeWritableSource' in scope`).

- [ ] **Step 3: Implement**

```swift
// Sources/CalendarCore/Write/WritableCalendarSource.swift
import Foundation

/// A source that can also write. Read-only connectors never adopt it; callers check `source as? WritableCalendarSource`.
/// What a conforming source can write is in `capabilities` (`writableFields`, `controlsNotifications`,
/// `recurrenceScopes`, `canEditAttendees`, `canRespondToInvite`); which calendars are writable is each
/// `CalendarDescriptor.accessRole`. An operation the source cannot perform throws `WriteError.unsupported` before
/// changing anything. Writes return the event in the provider's resulting form, including its new `version`.
public protocol WritableCalendarSource: CalendarSource {
    func create(_ draft: EventDraft, in calendarID: String, notify: NotifyPolicy) async throws -> CalendarEvent
    /// An empty patch writes nothing and returns the patch's base event, or the current event when it has none.
    func update(_ ref: EventRef, _ patch: EventPatch, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent
    func delete(_ ref: EventRef, scope: RecurrenceScope, notify: NotifyPolicy) async throws
    /// `response` must be `.accepted`, `.tentative` or `.declined`.
    func respond(to ref: EventRef, _ response: ResponseStatus, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent
}
```

```swift
// Sources/CalendarTestSupport/FakeWritableSource.swift
import CalendarCore
import Foundation

/// An in-memory `WritableCalendarSource` for tests. Non-recurring events only; the scope is ignored.
public final class FakeWritableSource: WritableCalendarSource, @unchecked Sendable {
    public let id = "fake-writable"
    public let displayName = "Fake writable"
    public let capabilities: SourceCapabilities

    private let lock = NSLock()
    private let calendarIDs: Set<String>
    private var events: [String: CalendarEvent] = [:]   // by `CalendarEvent.id`
    private var counter = 0
    private var writes = 0

    public init(calendarIDs: [String] = ["cal"], writableFields: Set<EventField> = Set(EventField.allCases), canRespond: Bool = true) {
        self.calendarIDs = Set(calendarIDs)
        self.capabilities = SourceCapabilities(
            canWrite: true, canEditAttendees: writableFields.contains(.attendees), canRespondToInvite: canRespond,
            writableFields: writableFields, controlsNotifications: true, recurrenceScopes: Set(RecurrenceScope.allCases))
    }

    /// Number of successful writes; lets tests assert that a call wrote nothing.
    public var writeCount: Int { lock.withLock { writes } }

    /// Simulates another writer: applies `mutate` and bumps the version.
    public func simulateExternalEdit(calendarID: String, eventID: String, _ mutate: (inout CalendarEvent) -> Void) {
        lock.withLock {
            let key = "\(calendarID)/\(eventID)"
            guard var event = events[key] else { return }
            mutate(&event)
            counter += 1
            event.version = "v\(counter)"
            events[key] = event
        }
    }

    public func calendars() async throws -> [CalendarDescriptor] {
        calendarIDs.sorted().map { CalendarDescriptor(id: $0, title: $0, accessRole: .owner) }
    }

    public func events(in interval: DateInterval) async throws -> [CalendarEvent] {
        lock.withLock { events.values.filter { $0.start < interval.end && $0.end > interval.start }.sorted { ($0.start, $0.id) < ($1.start, $1.id) } }
    }

    public func changes() -> AsyncStream<CalendarChange> { AsyncStream { $0.finish() } }

    public func create(_ draft: EventDraft, in calendarID: String, notify: NotifyPolicy) async throws -> CalendarEvent {
        try draft.validate()
        try WriteValidation.requireWritable(draft.usedFields, capabilities)
        guard calendarIDs.contains(calendarID) else { throw WriteError.notFound }
        return lock.withLock {
            counter += 1
            writes += 1
            var event = CalendarEvent(
                eventID: "ev\(counter)", calendarID: calendarID, title: draft.title, notes: draft.notes, location: draft.location,
                start: draft.timing.start, end: draft.timing.end, timeZone: draft.timing.timeZone, isAllDay: draft.timing.isAllDay,
                availability: draft.availability, visibility: draft.visibility,
                attendees: draft.attendees.map { Attendee(name: $0.name, email: $0.email, role: $0.role) },
                reminders: draft.reminders ?? [], version: "v\(counter)", sourceID: id)
            if draft.conference == .generate {
                event.conference = ConferenceInfo(url: URL(string: "https://meet.example/\(counter)")!, provider: .meet)
            }
            events[event.id] = event
            return event
        }
    }

    public func update(_ ref: EventRef, _ patch: EventPatch, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent {
        try WriteValidation.requireWritable(patch.touchedFields, capabilities)
        try patch.timing?.validate()
        let key = "\(ref.calendarID)/\(ref.eventID)"
        guard let existing = lock.withLock({ events[key] }) else { throw WriteError.notFound }
        if patch.isEmpty { return patch.base ?? existing }
        return try await PatchMerge.apply(
            patch: patch, version: ref.version,
            fetchCurrent: { self.lock.withLock { self.events[key] } ?? existing },
            write: { version in try self.write(key, version, patch) })
    }

    private func write(_ key: String, _ version: String?, _ patch: EventPatch) throws -> PatchMerge.Attempt<CalendarEvent> {
        try lock.withLock {
            guard let current = events[key] else { throw WriteError.notFound }
            if let version, current.version != version { return .stale }
            var updated = patch.applied(to: current)
            if patch.conference == .generate {
                updated.conference = ConferenceInfo(url: URL(string: "https://meet.example/\(counter + 1)")!, provider: .meet)
            }
            counter += 1
            writes += 1
            updated.version = "v\(counter)"
            events[key] = updated
            return .done(updated)
        }
    }

    public func delete(_ ref: EventRef, scope: RecurrenceScope, notify: NotifyPolicy) async throws {
        try lock.withLock {
            guard events.removeValue(forKey: "\(ref.calendarID)/\(ref.eventID)") != nil else { throw WriteError.notFound }
            writes += 1
        }
    }

    public func respond(to ref: EventRef, _ response: ResponseStatus, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent {
        guard capabilities.canRespondToInvite else { throw WriteError.unsupported(fields: [.attendees]) }
        guard response != .needsAction else { throw WriteError.invalid("cannot respond with needsAction") }
        let key = "\(ref.calendarID)/\(ref.eventID)"
        return try lock.withLock {
            guard var event = events[key] else { throw WriteError.notFound }
            guard let index = event.attendees.firstIndex(where: \.isSelf) else { throw WriteError.invalid("you are not an attendee of this event") }
            event.attendees[index].response = response
            event.myResponse = response
            counter += 1
            writes += 1
            event.version = "v\(counter)"
            events[key] = event
            return event
        }
    }
}
```

```swift
// Sources/CalendarTestSupport/WritableSourceConformance.swift
import CalendarCore
import Foundation

/// Behaviour every `WritableCalendarSource` must have. Run it against a source and a scratch calendar (the fake
/// here, the real EventKit source in the live tests). An empty result means the source conforms.
public enum WritableSourceConformance {
    public static func violations(of source: any WritableCalendarSource, calendarID: String, window: DateInterval) async -> [String] {
        var found: [String] = []
        let caps = source.capabilities
        if !caps.canWrite { found.append("canWrite is false on a WritableCalendarSource") }
        if caps.canEditAttendees != caps.writableFields.contains(.attendees) {
            found.append("canEditAttendees does not match writableFields.contains(.attendees)")
        }
        let utc = TimeZone(identifier: "UTC")!
        let start = window.start.addingTimeInterval(3600)
        let draft = EventDraft(
            title: "Conformance A", timing: EventTiming(start: start, end: start.addingTimeInterval(1800), timeZone: utc, isAllDay: false),
            location: "Room 1")
        var created: CalendarEvent
        do {
            created = try await source.create(draft, in: calendarID, notify: .none)
        } catch {
            return found + ["create threw \(error)"]
        }
        let ref = EventRef(created)
        if created.title != "Conformance A" || created.calendarID != calendarID { found.append("create returned a different title or calendar") }

        do {
            let listed = try await source.events(in: window)
            if !listed.contains(where: { $0.eventID == created.eventID }) {
                found.append("the created event is not returned by events(in:)")
            }
            let renamed = try await source.update(ref, EventPatch(title: "Conformance B"), scope: .thisInstance, notify: .none)
            if renamed.title != "Conformance B" { found.append("update did not change the title") }
            if renamed.location != "Room 1" { found.append("update changed a field the patch did not touch") }
            if renamed.version != nil, renamed.version == created.version { found.append("update did not change the version") }

            let unchanged = try await source.update(EventRef(renamed), EventPatch(), scope: .thisInstance, notify: .none)
            if unchanged.title != "Conformance B" { found.append("an empty patch changed the event") }

            if !caps.writableFields.contains(.attendees) {
                do {
                    _ = try await source.update(EventRef(renamed), EventPatch(attendees: AttendeeChanges(add: [AttendeeDraft(email: "a@b.c")])),
                                                scope: .thisInstance, notify: .none)
                    found.append("an unwritable field (attendees) was accepted")
                } catch WriteError.unsupported {
                } catch {
                    found.append("an unwritable field threw \(error) instead of .unsupported")
                }
            }

            try await source.delete(EventRef(renamed), scope: .thisInstance, notify: .none)
            let remaining = try await source.events(in: window)
            if remaining.contains(where: { $0.eventID == created.eventID }) {
                found.append("the event is still present after delete")
            }
            do {
                try await source.delete(EventRef(renamed), scope: .thisInstance, notify: .none)
                found.append("deleting a missing event did not throw")
            } catch WriteError.notFound {
            } catch {
                found.append("deleting a missing event threw \(error) instead of .notFound")
            }
        } catch {
            found.append("threw \(error)")
        }
        return found
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `swift test --package-path Packages/CalendarConnectors`
Expected: PASS (whole library suite).

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "feat(core): WritableCalendarSource, in-memory fake and conformance checks" -m "Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 8: `GoogleAPIClient.send` (write requests, new outcomes, reads unchanged)

**Files:**
- Modify: `Packages/CalendarConnectors/Sources/GoogleCalendar/GoogleAPIClient.swift`
- Test: `Packages/CalendarConnectors/Tests/GoogleCalendarTests/WriteClientTests.swift`

**Interfaces:**
- Produces: `GoogleAPIError.preconditionFailed`, `.badRequest(String)`; `GoogleRequestMode` (`.read`, `.write`); `GoogleAPIClient.send(method:path:query:body:headers:mode:) async throws -> Data`; `GoogleAPIClient.percentEncode(_:)`, `.eventPath(_:_:)`. `get(path:query:)` stays as a thin wrapper with unchanged behaviour.

- [ ] **Step 1: Write the failing tests**

```swift
import CalendarCore
import CalendarOAuth
import CalendarTestSupport
import Foundation
import Testing
@testable import GoogleCalendar

private func makeClient(_ transport: FakeTransport) async throws -> GoogleAPIClient {
    let store = InMemoryCredentialStore()
    try await store.setSecrets([AccessTokenProvider.refreshTokenKey: "rt"], for: "c1")
    let now = TestNow()
    let provider = AccessTokenProvider(
        connectionID: "c1", credentials: store,
        refresh: { _ in OAuthTokens(accessToken: "at", expiresAt: now.date.addingTimeInterval(3600)) }, now: now.provider)
    return GoogleAPIClient(transport: transport, tokens: provider, sleep: { _ in })
}

private func thrown(_ body: () async throws -> Void) async -> Error? {
    do { try await body(); return nil } catch { return error }
}

private func errorBody(_ reason: String, message: String = "m", status: Int) -> HTTPResponse {
    .json(["error": ["errors": [["reason": reason]], "message": message]], status: status)
}

@Test func sendPostsABodyWithJsonHeadersAndExtraHeaders() async throws {
    let transport = FakeTransport()
    await transport.route("events", [.json(["id": "x"])])
    let client = try await makeClient(transport)
    _ = try await client.send(method: "PATCH", path: "/calendars/c/events/e", query: [URLQueryItem(name: "sendUpdates", value: "none")],
                              body: Data(#"{"summary":"S"}"#.utf8), headers: ["If-Match": "e1"], mode: .write)
    let request = try #require(await transport.requests.last)
    #expect(request.method == "PATCH" && request.url.absoluteString.hasSuffix("/calendars/c/events/e?sendUpdates=none"))
    #expect(request.headers["Content-Type"] == "application/json" && request.headers["If-Match"] == "e1")
    #expect(request.headers["Authorization"] == "Bearer at" && String(decoding: request.body ?? Data(), as: UTF8.self) == #"{"summary":"S"}"#)
}

@Test func aStaleIfMatchSurfacesAsPreconditionFailed() async throws {
    let transport = FakeTransport()
    await transport.route("events", [errorBody("conditionNotMet", status: 412)])
    let client = try await makeClient(transport)
    let error = await thrown { _ = try await client.send(method: "PATCH", path: "/calendars/c/events/e", mode: .write) }
    #expect(error as? GoogleAPIError == .preconditionFailed)
}

@Test func writeModeMapsAnyOtherForbiddenReasonButReadModeKeepsItsBehaviour() async throws {
    let transport = FakeTransport()
    await transport.route("events", [errorBody("forbiddenForNonOrganizer", status: 403)])
    let client = try await makeClient(transport)
    let write = await thrown { _ = try await client.send(method: "PATCH", path: "/calendars/c/events/e", mode: .write) }
    #expect(write as? GoogleAPIError == .forbidden)
    let read = await thrown { _ = try await client.get(path: "/calendars/c/events/e", query: []) }
    #expect(read as? SourceError == .invalidResponse("HTTP 403: forbiddenForNonOrganizer"))
}

@Test func insufficientPermissionsIsStillAuthExpiredInWriteMode() async throws {
    let transport = FakeTransport()
    await transport.route("events", [errorBody("insufficientPermissions", status: 403)])
    let client = try await makeClient(transport)
    let error = await thrown { _ = try await client.send(method: "POST", path: "/calendars/c/events", mode: .write) }
    #expect(error as? SourceError == .authExpired)
}

@Test func writeModeMapsBadRequestWithItsMessage() async throws {
    let transport = FakeTransport()
    await transport.route("events", [errorBody("invalid", message: "Invalid start time", status: 400)])
    let client = try await makeClient(transport)
    let error = await thrown { _ = try await client.send(method: "POST", path: "/calendars/c/events", mode: .write) }
    #expect(error as? GoogleAPIError == .badRequest("Invalid start time"))
}

@Test func goneAndNotFoundKeepTheirMeaning() async throws {
    let transport = FakeTransport()
    await transport.route("gone", [errorBody("deleted", status: 410)])
    await transport.route("missing", [errorBody("notFound", status: 404)])
    let client = try await makeClient(transport)
    #expect(await thrown { _ = try await client.get(path: "/gone", query: []) } as? GoogleAPIError == .gone)
    #expect(await thrown { _ = try await client.send(method: "DELETE", path: "/missing", mode: .write) } as? GoogleAPIError == .notFound)
}

@Test func aNoContentResponseSucceeds() async throws {
    let transport = FakeTransport()
    await transport.route("events", [HTTPResponse(status: 204)])
    let client = try await makeClient(transport)
    let data = try await client.send(method: "DELETE", path: "/calendars/c/events/e", mode: .write)
    #expect(data.isEmpty)
}

@Test func eventPathEncodesBothIds() {
    #expect(GoogleAPIClient.eventPath("me@x.com", "a_b/c") == "/calendars/me%40x.com/events/a_b%2Fc")
}
```

- [ ] **Step 2: Run to verify failure**

Run: `swift test --package-path Packages/CalendarConnectors --filter WriteClientTests`
Expected: FAIL to compile (`value of type 'GoogleAPIClient' has no member 'send'`).

- [ ] **Step 3: Implement**

In `GoogleAPIClient.swift`, replace everything from the top of the file through the end of `isRateLimit` (leaving `decode`, `pages` and `calendarList` unchanged) with:

```swift
import CalendarCore
import CalendarOAuth
import Foundation

/// Provider-specific outcomes that callers handle; never leaves the `GoogleCalendar` module.
enum GoogleAPIError: Error, Equatable {
    case gone       // 410: the sync token is no longer valid (or the event was already deleted)
    case notFound   // 404
    case forbidden  // 403 without a rate-limit reason
    case preconditionFailed   // 412: the `If-Match` version is stale
    case badRequest(String)   // 400 (write mode only), with Google's message

    /// What a public `CalendarSource` method throws if it cannot handle the case itself.
    var sourceError: SourceError { .invalidResponse("google: \(self)") }
}

/// Reads keep the original error mapping. Writes additionally map a 400 to `badRequest` and treat every non-rate-limit
/// 403 (other than `insufficientPermissions`) as `forbidden`.
enum GoogleRequestMode: Sendable { case read, write }

struct GoogleAPIClient: Sendable {
    static let base = "https://www.googleapis.com/calendar/v3"
    private static let maxRateLimitRetries = 3

    let transport: any HTTPTransport
    let tokens: AccessTokenProvider
    let sleep: Sleeper

    static func percentEncode(_ text: String) -> String {
        let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-._~"))
        return text.addingPercentEncoding(withAllowedCharacters: allowed) ?? text
    }

    /// `path` must already be percent-encoded, e.g. `/calendars/me%40x.com/events`.
    static func calendarPath(_ calendarID: String, _ tail: String) -> String {
        "/calendars/\(percentEncode(calendarID))\(tail)"
    }

    static func eventPath(_ calendarID: String, _ eventID: String) -> String {
        calendarPath(calendarID, "/events/\(percentEncode(eventID))")
    }

    func get(path: String, query: [URLQueryItem]) async throws -> Data {
        try await send(method: "GET", path: path, query: query)
    }

    /// One request with the shared 401-refresh, rate-limit backoff and 5xx handling. `path` is percent-encoded.
    func send(
        method: String, path: String, query: [URLQueryItem] = [], body: Data? = nil,
        headers extraHeaders: [String: String] = [:], mode: GoogleRequestMode = .read
    ) async throws -> Data {
        var rateLimitRetries = 0
        var refreshedAfter401 = false
        while true {
            try Task.checkCancellation()
            let token = try await tokens.accessToken()
            var components = URLComponents(string: Self.base)!
            components.percentEncodedPath += path
            components.queryItems = query.isEmpty ? nil : query
            var headers = ["Authorization": "Bearer \(token)", "Accept": "application/json"]
            if body != nil { headers["Content-Type"] = "application/json" }
            for (name, value) in extraHeaders { headers[name] = value }
            let response = try await transport.send(HTTPRequest(url: components.url!, method: method, headers: headers, body: body))
            switch response.status {
            case 200..<300:
                return response.body
            case 401:
                if refreshedAfter401 { throw SourceError.authExpired }
                refreshedAfter401 = true
                await tokens.invalidate()
            case 400 where mode == .write:
                throw GoogleAPIError.badRequest(Self.message(response))
            case 410:
                throw GoogleAPIError.gone
            case 404:
                throw GoogleAPIError.notFound
            case 412:
                throw GoogleAPIError.preconditionFailed
            case 403, 429:
                guard Self.isRateLimit(response) else {
                    if response.status == 403 { throw Self.classifyForbidden(response, mode: mode) }
                    throw SourceError.invalidResponse("HTTP \(response.status)")
                }
                let retryAfter = response.header("retry-after").flatMap(TimeInterval.init)
                rateLimitRetries += 1
                if rateLimitRetries > Self.maxRateLimitRetries { throw SourceError.rateLimited(retryAfter: retryAfter) }
                // Honor Retry-After exactly; otherwise exponential backoff with jitter so several clients de-synchronize.
                let fallback = Double(1 << (rateLimitRetries - 1)) * Double.random(in: 0.5...1.0)
                try await sleep(.seconds(retryAfter ?? fallback))
            case 500...:
                throw SourceError.server(status: response.status)
            default:
                throw SourceError.invalidResponse("HTTP \(response.status)")
            }
        }
    }

    private struct ErrorBody: Decodable {
        struct Detail: Decodable { let reason: String? }
        struct Inner: Decodable {
            let errors: [Detail]?
            let message: String?
        }
        let error: Inner?
    }

    private static func message(_ response: HTTPResponse) -> String {
        (try? JSONDecoder().decode(ErrorBody.self, from: response.body))?.error?.message ?? "HTTP \(response.status)"
    }

    /// A non-rate-limit 403. Reads: only reason `forbidden` (one unreadable calendar) is skippable and the rest affect
    /// every calendar. Writes: any reason but `insufficientPermissions` is a permission problem on this event.
    private static func classifyForbidden(_ response: HTTPResponse, mode: GoogleRequestMode) -> Error {
        let reason = (try? JSONDecoder().decode(ErrorBody.self, from: response.body))?.error?.errors?.first?.reason
        switch reason {
        case "forbidden": return GoogleAPIError.forbidden
        case "insufficientPermissions": return SourceError.authExpired
        default:
            if mode == .write { return GoogleAPIError.forbidden }
            return SourceError.invalidResponse("HTTP 403: \(reason ?? "unknown")")
        }
    }

    private static func isRateLimit(_ response: HTTPResponse) -> Bool {
        if response.status == 429 { return true }
        let body = String(decoding: response.body, as: UTF8.self)
        return body.contains("rateLimitExceeded") || body.contains("userRateLimitExceeded")
    }
```

- [ ] **Step 4: Run to verify pass (new tests and every existing Google test)**

Run: `swift test --package-path Packages/CalendarConnectors --filter GoogleCalendarTests`
Expected: PASS. In particular the existing 410-reset, 401-refresh and rate-limit tests still pass, proving reads are unchanged.

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "feat(google): shared send() for write requests; 412 and 400 outcomes" -m "Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 9: `GoogleWriteMapper` (pure JSON building)

**Files:**
- Create: `Packages/CalendarConnectors/Sources/GoogleCalendar/GoogleWriteMapper.swift`
- Test: `Packages/CalendarConnectors/Tests/GoogleCalendarTests/WriteMapperTests.swift`

**Interfaces:**
- Consumes: `EventDraft`, `EventPatch`, `EventTiming`, `AttendeeChanges`, `RecurrenceRule`, `NotifyPolicy`, `ResponseStatus`, `AllDay`, `WriteError`.
- Produces (all `static` on internal `enum GoogleWriteMapper`): `typealias JSON = [String: Any]`; `struct Body { json: JSON; needsConferenceVersion: Bool }`; `sendUpdates(_:) -> String`; `data(_:) throws -> Data`; `instantText(_:)`; `timeJSON(_:) -> (start: JSON, end: JSON)`; `createBody(_:) throws -> Body`; `patchBody(_:currentAttendees:) throws -> Body`; `mergeAttendees(current:changes:) -> [JSON]`; `respondAttendees(current:response:) throws -> [JSON]`; `conferenceRequestJSON() -> JSON`.

- [ ] **Step 1: Write the failing tests**

```swift
import CalendarCore
import Foundation
import Testing
@testable import GoogleCalendar

private let utc = TimeZone(identifier: "UTC")!
private let newYork = TimeZone(identifier: "America/New_York")!
private func instant(_ s: String) -> Date { ISO8601DateFormatter().date(from: s)! }
private func timed(_ zone: TimeZone? = utc) -> EventTiming {
    EventTiming(start: instant("2026-09-21T10:00:00Z"), end: instant("2026-09-21T10:30:00Z"), timeZone: zone, isAllDay: false)
}
private func dict(_ any: Any?) -> [String: Any] { any as? [String: Any] ?? [:] }
private func same(_ a: Any, _ b: Any) -> Bool { (a as AnyObject).isEqual(b) }

@Test func createBodyForATimedEvent() throws {
    let draft = EventDraft(
        title: "Sync", timing: timed(), notes: "n", location: "Room", availability: .free, visibility: .privateEvent,
        reminders: [Reminder(minutesBefore: 10)],
        attendees: [AttendeeDraft(email: "a@x.com", name: "A", role: .optional), AttendeeDraft(email: "r@x.com", role: .resource)])
    let body = try GoogleWriteMapper.createBody(draft)
    let json = body.json
    #expect(json["summary"] as? String == "Sync" && json["description"] as? String == "n" && json["location"] as? String == "Room")
    #expect(dict(json["start"])["dateTime"] as? String == "2026-09-21T10:00:00Z" && dict(json["start"])["timeZone"] as? String == "UTC")
    #expect(dict(json["end"])["dateTime"] as? String == "2026-09-21T10:30:00Z")
    #expect(json["transparency"] as? String == "transparent" && json["visibility"] as? String == "private")
    #expect(same(json["reminders"]!, ["useDefault": false, "overrides": [["method": "popup", "minutes": 10]]] as NSDictionary))
    let attendees = try #require(json["attendees"] as? [[String: Any]])
    #expect(attendees.count == 2 && attendees[0]["optional"] as? Bool == true && attendees[0]["displayName"] as? String == "A")
    #expect(attendees[1]["resource"] as? Bool == true)
    #expect(!body.needsConferenceVersion && json["recurrence"] == nil && json["conferenceData"] == nil)
}

@Test func allDayUsesDatesWithTheExclusiveEnd() throws {
    let timing = EventTiming(start: instant("2026-09-18T04:00:00Z"), end: instant("2026-09-20T04:00:00Z"), timeZone: newYork, isAllDay: true)
    let json = try GoogleWriteMapper.createBody(EventDraft(title: "Trip", timing: timing)).json
    #expect(same(json["start"]!, ["date": "2026-09-18"] as NSDictionary) && same(json["end"]!, ["date": "2026-09-20"] as NSDictionary))
}

@Test func nilRemindersAreOmittedAndGenerateRequestsAMeetLink() throws {
    var draft = EventDraft(title: "Sync", timing: timed())
    #expect(try GoogleWriteMapper.createBody(draft).json["reminders"] == nil)
    draft.conference = .generate
    let body = try GoogleWriteMapper.createBody(draft)
    let request = dict(dict(body.json["conferenceData"])["createRequest"])
    #expect(body.needsConferenceVersion && dict(request["conferenceSolutionKey"])["type"] as? String == "hangoutsMeet")
    #expect((request["requestId"] as? String)?.isEmpty == false)
}

@Test func recurrenceRendersAnRruleAndNeedsAZoneUnlessAllDay() async throws {
    var draft = EventDraft(title: "Weekly", timing: timed(), recurrence: RecurrenceRule(frequency: .weekly, weekdays: [.init(.monday)], end: .count(4)))
    #expect(try GoogleWriteMapper.createBody(draft).json["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;BYDAY=MO;COUNT=4"])
    draft.timing = timed(nil)
    await expectWriteError(.invalid("recurring events need a time zone")) { _ = try GoogleWriteMapper.createBody(draft) }
}

@Test func tooManyRemindersAreRejected() async {
    let many = (1...6).map { Reminder(minutesBefore: $0) }
    await expectWriteError(.invalid("Google allows at most 5 reminders")) {
        _ = try GoogleWriteMapper.createBody(EventDraft(title: "T", timing: timed(), reminders: many))
    }
}

@Test func patchBodyContainsOnlyTouchedFields() throws {
    let body = try GoogleWriteMapper.patchBody(EventPatch(title: "New", availability: .busy), currentAttendees: nil)
    #expect(same(body.json, ["summary": "New", "transparency": "opaque"] as NSDictionary) && !body.needsConferenceVersion)
    #expect(try GoogleWriteMapper.patchBody(EventPatch(), currentAttendees: nil).json.isEmpty)
}

@Test func patchBodyClearsWithNullAndResetsRemindersToDefaults() throws {
    let patch = EventPatch(notes: .clear, location: .clear, reminders: .clear, recurrence: .clear, conference: .remove)
    let json = try GoogleWriteMapper.patchBody(patch, currentAttendees: nil).json
    #expect(json["description"] is NSNull && json["location"] is NSNull && json["recurrence"] is NSNull && json["conferenceData"] is NSNull)
    #expect(same(json["reminders"]!, ["useDefault": true] as NSDictionary))
}

@Test func patchBodySendsTimeAsAPairAndRecurrenceUsesThePatchTiming() throws {
    let patch = EventPatch(timing: timed(newYork), recurrence: .set(RecurrenceRule(frequency: .daily, end: .count(3))))
    let json = try GoogleWriteMapper.patchBody(patch, currentAttendees: nil).json
    #expect(dict(json["start"])["timeZone"] as? String == "America/New_York" && dict(json["end"])["dateTime"] as? String == "2026-09-21T10:30:00Z")
    #expect(json["recurrence"] as? [String] == ["RRULE:FREQ=DAILY;COUNT=3"])
}

@Test func attendeeChangesMergeIntoTheCurrentArrayAndKeepTheRest() throws {
    let current: [[String: Any]] = [
        ["email": "me@x.com", "self": true, "responseStatus": "accepted"],
        ["email": "Bob@x.com", "responseStatus": "accepted", "optional": true],
        ["email": "cy@x.com", "responseStatus": "declined"],
    ]
    let changes = AttendeeChanges(add: [AttendeeDraft(email: "bob@x.com", name: "Bob", role: .required), AttendeeDraft(email: "dee@x.com")], remove: ["cy@x.com"])
    let json = try GoogleWriteMapper.patchBody(EventPatch(attendees: changes), currentAttendees: current).json
    let merged = try #require(json["attendees"] as? [[String: Any]])
    #expect(merged.map { $0["email"] as? String } == ["me@x.com", "Bob@x.com", "dee@x.com"])
    #expect(merged[1]["responseStatus"] as? String == "accepted" && merged[1]["optional"] == nil && merged[1]["displayName"] as? String == "Bob")
    #expect(merged[0]["self"] as? Bool == true)
}

@Test func attendeeChangesNeedTheCurrentAttendees() async {
    await expectWriteError(.invalid("attendee changes need the current attendees")) {
        _ = try GoogleWriteMapper.patchBody(EventPatch(attendees: AttendeeChanges(add: [AttendeeDraft(email: "a@b.c")])), currentAttendees: nil)
    }
}

@Test func respondingSetsTheSelfAttendeeOnly() async throws {
    let current: [[String: Any]] = [["email": "me@x.com", "self": true, "responseStatus": "needsAction"], ["email": "bob@x.com", "responseStatus": "accepted"]]
    let updated = try GoogleWriteMapper.respondAttendees(current: current, response: .tentative)
    #expect(updated[0]["responseStatus"] as? String == "tentative" && updated[1]["responseStatus"] as? String == "accepted")
    await expectWriteError(.invalid("cannot respond with needsAction")) { _ = try GoogleWriteMapper.respondAttendees(current: current, response: .needsAction) }
    await expectWriteError(.invalid("you are not an attendee of this event")) {
        _ = try GoogleWriteMapper.respondAttendees(current: [["email": "bob@x.com"]], response: .accepted)
    }
}

@Test func sendUpdatesMapsEveryPolicy() {
    #expect(GoogleWriteMapper.sendUpdates(.all) == "all" && GoogleWriteMapper.sendUpdates(.externalOnly) == "externalOnly" && GoogleWriteMapper.sendUpdates(.none) == "none")
}
```

Create `Packages/CalendarConnectors/Tests/GoogleCalendarTests/WriteHelpers.swift` (used here and in Tasks 10 and 11):

```swift
import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import GoogleCalendar

/// Runs `body` and records an issue unless it throws exactly `expected`.
func expectWriteError(_ expected: WriteError, _ body: () async throws -> Void) async {
    do {
        try await body()
        Issue.record("expected \(expected), but nothing was thrown")
    } catch let error as WriteError {
        #expect(error == expected)
    } catch {
        Issue.record("expected \(expected), got \(error)")
    }
}

/// A minimal Google event resource; `extra` overrides or adds keys.
func googleEvent(id: String, etag: String = "e1", summary: String = "Standup", extra: [String: Any] = [:]) -> [String: Any] {
    var json: [String: Any] = [
        "id": id, "etag": etag, "summary": summary,
        "start": ["dateTime": "2026-09-21T10:00:00Z", "timeZone": "UTC"], "end": ["dateTime": "2026-09-21T10:30:00Z", "timeZone": "UTC"],
    ]
    for (key, value) in extra { json[key] = value }
    return json
}

func bodyJSON(_ request: HTTPRequest) -> [String: Any] {
    (try? JSONSerialization.jsonObject(with: request.body ?? Data())) as? [String: Any] ?? [:]
}

func googleError(_ reason: String, message: String = "m", status: Int) -> HTTPResponse {
    .json(["error": ["errors": [["reason": reason]], "message": message]], status: status)
}
```

- [ ] **Step 2: Run to verify failure**

Run: `swift test --package-path Packages/CalendarConnectors --filter WriteMapperTests`
Expected: FAIL to compile (`cannot find 'GoogleWriteMapper' in scope`).

- [ ] **Step 3: Implement**

```swift
// Sources/GoogleCalendar/GoogleWriteMapper.swift
import CalendarCore
import Foundation

/// Pure conversion between the library's write types and Google's event JSON. No I/O.
enum GoogleWriteMapper {
    typealias JSON = [String: Any]

    /// A request body plus whether the request needs `conferenceDataVersion=1`.
    struct Body {
        var json: JSON
        var needsConferenceVersion: Bool
    }

    static func sendUpdates(_ policy: NotifyPolicy) -> String {
        switch policy {
        case .all: "all"
        case .externalOnly: "externalOnly"
        case .none: "none"
        }
    }

    static func data(_ json: JSON) throws -> Data {
        try JSONSerialization.data(withJSONObject: json, options: [.sortedKeys])
    }

    static func instantText(_ date: Date) -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        return formatter.string(from: date)
    }

    private static func dayText(_ date: CalendarDate) -> String {
        String(format: "%04d-%02d-%02d", date.year, date.month, date.day)
    }

    /// All-day: `date` values with the exclusive end. Timed: `dateTime` (UTC) plus the zone name when there is one.
    static func timeJSON(_ timing: EventTiming) -> (start: JSON, end: JSON) {
        if timing.isAllDay, let zone = timing.timeZone {
            let days = AllDay.dates(start: timing.start, end: timing.end, in: zone)
            return (["date": dayText(days.first)], ["date": dayText(days.endExclusive)])
        }
        func stamp(_ date: Date) -> JSON {
            var json: JSON = ["dateTime": instantText(date)]
            if let zone = timing.timeZone { json["timeZone"] = zone.identifier }
            return json
        }
        return (stamp(timing.start), stamp(timing.end))
    }

    private static func visibilityText(_ visibility: Visibility) -> String {
        switch visibility {
        case .default: "default"
        case .publicEvent: "public"
        case .privateEvent: "private"
        case .confidential: "confidential"
        }
    }

    static func remindersJSON(_ reminders: [Reminder]) throws -> JSON {
        guard reminders.count <= 5 else { throw WriteError.invalid("Google allows at most 5 reminders") }
        return ["useDefault": false, "overrides": reminders.map { ["method": "popup", "minutes": $0.minutesBefore] as JSON }]
    }

    static func attendeeJSON(_ attendee: AttendeeDraft) -> JSON {
        var json: JSON = ["email": attendee.email]
        if let name = attendee.name { json["displayName"] = name }
        if attendee.role == .optional { json["optional"] = true }
        if attendee.role == .resource { json["resource"] = true }
        return json
    }

    static func conferenceRequestJSON() -> JSON {
        ["createRequest": ["requestId": UUID().uuidString, "conferenceSolutionKey": ["type": "hangoutsMeet"]] as JSON]
    }

    private static func recurrenceLines(_ rule: RecurrenceRule, allDay: Bool, zone: TimeZone?) throws -> [String] {
        try rule.validate()
        guard allDay || zone != nil else { throw WriteError.invalid("recurring events need a time zone") }
        return ["RRULE:" + rule.rruleString(allDay: allDay, in: zone)]
    }

    static func createBody(_ draft: EventDraft) throws -> Body {
        try draft.validate()
        var json: JSON = ["summary": draft.title]
        if let notes = draft.notes { json["description"] = notes }
        if let location = draft.location { json["location"] = location }
        let time = timeJSON(draft.timing)
        json["start"] = time.start
        json["end"] = time.end
        json["transparency"] = draft.availability == .free ? "transparent" : "opaque"
        json["visibility"] = visibilityText(draft.visibility)
        if let reminders = draft.reminders { json["reminders"] = try remindersJSON(reminders) }
        if !draft.attendees.isEmpty { json["attendees"] = draft.attendees.map(attendeeJSON) }
        if let rule = draft.recurrence {
            json["recurrence"] = try recurrenceLines(rule, allDay: draft.timing.isAllDay, zone: draft.timing.timeZone)
        }
        if draft.conference == .generate { json["conferenceData"] = conferenceRequestJSON() }
        return Body(json: json, needsConferenceVersion: draft.conference == .generate)
    }

    /// Only the touched fields; `.clear` is JSON `null`. `currentAttendees` is required when the patch changes
    /// attendees (Google replaces the whole array on PATCH).
    static func patchBody(_ patch: EventPatch, currentAttendees: [JSON]?) throws -> Body {
        var json: JSON = [:]
        if let title = patch.title { json["summary"] = title }
        switch patch.notes { case .keep: break; case .set(let value): json["description"] = value; case .clear: json["description"] = NSNull() }
        switch patch.location { case .keep: break; case .set(let value): json["location"] = value; case .clear: json["location"] = NSNull() }
        if let timing = patch.timing {
            try timing.validate()
            let time = timeJSON(timing)
            json["start"] = time.start
            json["end"] = time.end
        }
        if let availability = patch.availability { json["transparency"] = availability == .free ? "transparent" : "opaque" }
        if let visibility = patch.visibility { json["visibility"] = visibilityText(visibility) }
        switch patch.reminders {
        case .keep: break
        case .set(let list): json["reminders"] = try remindersJSON(list)
        case .clear: json["reminders"] = ["useDefault": true] as JSON
        }
        if let changes = patch.attendees, !changes.isEmpty {
            guard let currentAttendees else { throw WriteError.invalid("attendee changes need the current attendees") }
            json["attendees"] = mergeAttendees(current: currentAttendees, changes: changes)
        }
        switch patch.recurrence {
        case .keep: break
        case .clear: json["recurrence"] = NSNull()
        case .set(let rule):
            let allDay = patch.timing?.isAllDay ?? patch.base?.isAllDay ?? false
            let zone = patch.timing?.timeZone ?? patch.base?.timeZone
            json["recurrence"] = try recurrenceLines(rule, allDay: allDay, zone: zone)
        }
        switch patch.conference {
        case nil: break
        case .generate?: json["conferenceData"] = conferenceRequestJSON()
        case .remove?: json["conferenceData"] = NSNull()
        }
        return Body(json: json, needsConferenceVersion: patch.conference != nil)
    }

    private static func email(of attendee: JSON) -> String { (attendee["email"] as? String ?? "").lowercased() }

    /// Removes `changes.remove`, then upserts `changes.add` by email. Existing entries keep every other key
    /// (responses, `self`, `organizer`), so other guests' responses survive.
    static func mergeAttendees(current: [JSON], changes: AttendeeChanges) -> [JSON] {
        let removed = Set(changes.remove)
        var result = current.filter { !removed.contains(email(of: $0)) }
        for draft in changes.add {
            if let index = result.firstIndex(where: { email(of: $0) == draft.email }) {
                var merged = result[index]
                if let name = draft.name { merged["displayName"] = name }
                if draft.role == .optional { merged["optional"] = true } else { merged["optional"] = nil }
                if draft.role == .resource { merged["resource"] = true } else { merged["resource"] = nil }
                result[index] = merged
            } else {
                result.append(attendeeJSON(draft))
            }
        }
        return result
    }

    static func respondAttendees(current: [JSON], response: ResponseStatus) throws -> [JSON] {
        let text: String
        switch response {
        case .accepted: text = "accepted"
        case .tentative: text = "tentative"
        case .declined: text = "declined"
        case .needsAction: throw WriteError.invalid("cannot respond with needsAction")
        }
        guard let index = current.firstIndex(where: { $0["self"] as? Bool == true }) else {
            throw WriteError.invalid("you are not an attendee of this event")
        }
        var result = current
        result[index]["responseStatus"] = text
        return result
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `swift test --package-path Packages/CalendarConnectors --filter WriteMapperTests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "feat(google): pure JSON mapping for create, patch, attendee deltas and RSVP" -m "Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 10: `GoogleCalendarSource` as a `WritableCalendarSource` (instance and series scopes)

**Files:**
- Create: `Packages/CalendarConnectors/Sources/GoogleCalendar/GoogleCalendarSource+Write.swift`
- Modify: `Sources/GoogleCalendar/GoogleEventMapper.swift`, `Sources/GoogleCalendar/GoogleCalendarSource.swift`, `Tests/GoogleCalendarTests/SourceTests.swift` (the capabilities assertion)
- Test: `Packages/CalendarConnectors/Tests/GoogleCalendarTests/WriteSourceTests.swift`

**Interfaces:**
- Consumes: `GoogleAPIClient.send/eventPath` (Task 8), `GoogleWriteMapper` (Task 9), `PatchMerge` (Task 6), `WriteValidation`, `Harness` (existing, in `SourceTests.swift`; its `source` is the `GoogleCalendarSource` for connection `google`/`c1`, so `sourceID == "google-c1"`, calendars `me@x.com` (owner) and `team@group.calendar.google.com` (reader)).
- Produces: `GoogleCalendarSource: WritableCalendarSource`; `GoogleEventMapper.map(_:calendar:sourceID:)` stamping `sourceID`; capabilities `canWrite/canEditAttendees/canRespondToInvite`, all fields, `controlsNotifications`, scopes `[.thisInstance, .allInSeries]` (Task 11 adds `.thisAndFollowing`).

- [ ] **Step 1: Write the failing tests**

```swift
import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import GoogleCalendar

private let utc = TimeZone(identifier: "UTC")!
private func instant(_ s: String) -> Date { ISO8601DateFormatter().date(from: s)! }
private let cal = "me@x.com"
private let calPath = "calendars/me%40x.com/events"

private func timing() -> EventTiming {
    EventTiming(start: instant("2026-09-21T10:00:00Z"), end: instant("2026-09-21T10:30:00Z"), timeZone: utc, isAllDay: false)
}
private func base(title: String = "Standup", version: String = "e1", attendees: [Attendee] = []) -> CalendarEvent {
    CalendarEvent(eventID: "ev1", calendarID: cal, title: title, start: instant("2026-09-21T10:00:00Z"), end: instant("2026-09-21T10:30:00Z"),
                  timeZone: utc, attendees: attendees, version: version)
}

@Test func googleDeclaresItsWriteCapabilities() async throws {
    let h = try await Harness()
    let c = h.source.capabilities
    #expect(c.canWrite && c.canEditAttendees && c.canRespondToInvite && c.controlsNotifications)
    #expect(c.writableFields == Set(EventField.allCases) && c.recurrenceScopes == [.thisInstance, .allInSeries])
}

@Test func createPostsToTheCalendarAndMapsTheResult() async throws {
    let h = try await Harness()
    await h.transport.route(calPath, [.json(googleEvent(id: "new1", etag: "e9", summary: "Sync"))])
    let draft = EventDraft(title: "Sync", timing: timing(), attendees: [AttendeeDraft(email: "bob@x.com")], conference: .generate)
    let created = try await h.source.create(draft, in: cal, notify: .all)
    let post = try #require(await h.transport.requests(matching: calPath).last)
    #expect(post.method == "POST" && post.url.absoluteString.contains("sendUpdates=all") && post.url.absoluteString.contains("conferenceDataVersion=1"))
    #expect(bodyJSON(post)["summary"] as? String == "Sync")
    #expect(created.eventID == "new1" && created.version == "e9" && created.sourceID == "google-c1" && created.calendarID == cal)
}

@Test func writesToAReadOnlyCalendarAreForbiddenBeforeAnyEventRequest() async throws {
    let h = try await Harness()
    await expectWriteError(.forbidden("read-only calendar")) {
        _ = try await h.source.create(EventDraft(title: "T", timing: timing()), in: "team@group.calendar.google.com", notify: .none)
    }
    await expectWriteError(.notFound) { _ = try await h.source.create(EventDraft(title: "T", timing: timing()), in: "nope", notify: .none) }
    #expect(await h.transport.requests(matching: "/events").isEmpty)
}

@Test func createRefusesAnInvalidDraftBeforeAnyRequest() async throws {
    let h = try await Harness()
    var draft = EventDraft(title: "T", timing: timing())
    draft.reminders = [Reminder(minutesBefore: -1)]
    await expectWriteError(.invalid("reminder minutes must not be negative")) { _ = try await h.source.create(draft, in: cal, notify: .none) }
    #expect(await h.transport.requests.isEmpty)
}

@Test func updateSendsOnlyTheChangedFieldsWithIfMatch() async throws {
    let h = try await Harness()
    await h.transport.route("\(calPath)/ev1", [.json(googleEvent(id: "ev1", etag: "e2", summary: "New"))])
    let updated = try await h.source.update(EventRef(calendarID: cal, eventID: "ev1", version: "e1"), EventPatch(title: "New"),
                                            scope: .thisInstance, notify: .none)
    let patch = try #require(await h.transport.requests(matching: "\(calPath)/ev1").last)
    #expect(patch.method == "PATCH" && patch.headers["If-Match"] == "e1" && patch.url.absoluteString.contains("sendUpdates=none"))
    #expect((bodyJSON(patch) as NSDictionary).isEqual(["summary": "New"] as NSDictionary))
    #expect(updated.title == "New" && updated.version == "e2")
}

@Test func aStaleVersionWithNoOverlapIsRetriedOnTheFreshEtag() async throws {
    let h = try await Harness()
    var edit = EventEdit(base())
    edit.event.title = "New"
    await h.transport.route("\(calPath)/ev1", [
        googleError("conditionNotMet", status: 412),
        .json(googleEvent(id: "ev1", etag: "e2", summary: "Standup", extra: ["location": "Elsewhere"])),   // someone else set the location
        .json(googleEvent(id: "ev1", etag: "e3", summary: "New", extra: ["location": "Elsewhere"])),
    ])
    let updated = try await h.source.update(EventRef(base()), edit.patch, scope: .thisInstance, notify: .none)
    let sent = await h.transport.requests(matching: "\(calPath)/ev1")
    #expect(sent.map(\.method) == ["PATCH", "GET", "PATCH"] && sent[0].headers["If-Match"] == "e1" && sent[2].headers["If-Match"] == "e2")
    #expect(updated.title == "New" && updated.location == "Elsewhere")
}

@Test func aStaleVersionWithAnOverlapIsAConflict() async throws {
    let h = try await Harness()
    var edit = EventEdit(base())
    edit.event.title = "Mine"
    await h.transport.route("\(calPath)/ev1", [googleError("conditionNotMet", status: 412), .json(googleEvent(id: "ev1", etag: "e2", summary: "Theirs"))])
    await expectWriteError(.conflict(fields: [.title])) { _ = try await h.source.update(EventRef(base()), edit.patch, scope: .thisInstance, notify: .none) }
}

@Test func attendeeChangesFetchThenPatchTheFullArrayWithTheFetchedEtag() async throws {
    let h = try await Harness()
    let attendees: [[String: Any]] = [["email": "me@x.com", "self": true, "responseStatus": "accepted"], ["email": "bob@x.com", "responseStatus": "accepted"]]
    await h.transport.route("\(calPath)/ev1", [
        .json(googleEvent(id: "ev1", etag: "e1", extra: ["attendees": attendees])),
        .json(googleEvent(id: "ev1", etag: "e2")),
    ])
    let patch = EventPatch(attendees: AttendeeChanges(add: [AttendeeDraft(email: "dee@x.com")], remove: ["bob@x.com"]))
    _ = try await h.source.update(EventRef(calendarID: cal, eventID: "ev1", version: "e1"), patch, scope: .thisInstance, notify: .externalOnly)
    let sent = await h.transport.requests(matching: "\(calPath)/ev1")
    #expect(sent.map(\.method) == ["GET", "PATCH"] && sent[1].headers["If-Match"] == "e1" && sent[1].url.absoluteString.contains("sendUpdates=externalOnly"))
    let merged = try #require(bodyJSON(sent[1])["attendees"] as? [[String: Any]])
    #expect(merged.map { $0["email"] as? String } == ["me@x.com", "dee@x.com"])
}

@Test func anAttendeeChangeConflictsWhenSomeoneElseChangedTheSameAttendee() async throws {
    let h = try await Harness()
    var edit = EventEdit(base(version: "e5", attendees: [Attendee(email: "bob@x.com", role: .required)]))
    edit.event.attendees[0].role = .resource
    await h.transport.route("\(calPath)/ev1", [.json(googleEvent(id: "ev1", etag: "e6", extra: ["attendees": [["email": "bob@x.com", "optional": true]]]))])
    await expectWriteError(.conflict(fields: [.attendees])) {
        _ = try await h.source.update(EventRef(calendarID: cal, eventID: "ev1", version: "e5"), edit.patch, scope: .thisInstance, notify: .none)
    }
    #expect(await h.transport.requests(matching: "\(calPath)/ev1").allSatisfy { $0.method == "GET" })
}

@Test func anEmptyPatchReturnsTheBaseWithoutAnyRequest() async throws {
    let h = try await Harness()
    let edit = EventEdit(base())
    let result = try await h.source.update(EventRef(base()), edit.patch, scope: .thisInstance, notify: .none)
    let requestCount = await h.transport.requests.count
    #expect(result == base() && requestCount == 1)   // only the calendar list used for the access check
}

@Test func instanceAndSeriesScopesTargetTheInstanceOrTheMaster() async throws {
    let h = try await Harness()
    await h.transport.route("\(calPath)/master1", [.json(googleEvent(id: "master1", etag: "m2", summary: "All"))])
    await h.transport.route("\(calPath)/master1_20260921T100000Z", [.json(googleEvent(id: "master1_20260921T100000Z", etag: "i2", summary: "One"))])
    let ref = EventRef(calendarID: cal, eventID: "master1_20260921T100000Z", version: "i1", seriesID: "master1", originalStart: instant("2026-09-21T10:00:00Z"))
    _ = try await h.source.update(ref, EventPatch(title: "One"), scope: .thisInstance, notify: .none)
    _ = try await h.source.update(ref, EventPatch(title: "All"), scope: .allInSeries, notify: .none)
    let instance = try #require(await h.transport.requests(matching: "master1_2026").last)
    let master = try #require(await h.transport.requests(matching: "\(calPath)/master1?").last)
    #expect(instance.headers["If-Match"] == "i1")
    #expect(master.headers["If-Match"] == nil && master.method == "PATCH")   // an instance etag cannot lock the master
}

@Test func respondPatchesOnlyTheSelfAttendeeWithTheFetchedEtag() async throws {
    let h = try await Harness()
    let attendees: [[String: Any]] = [["email": "me@x.com", "self": true, "responseStatus": "needsAction"], ["email": "bob@x.com", "responseStatus": "accepted"]]
    await h.transport.route("\(calPath)/ev1", [.json(googleEvent(id: "ev1", etag: "e4", extra: ["attendees": attendees])), .json(googleEvent(id: "ev1", etag: "e5"))])
    _ = try await h.source.respond(to: EventRef(calendarID: cal, eventID: "ev1", version: "e1"), .declined, scope: .thisInstance, notify: .all)
    let patch = try #require(await h.transport.requests(matching: "\(calPath)/ev1").last)
    #expect(patch.method == "PATCH" && patch.headers["If-Match"] == "e4" && patch.url.absoluteString.contains("sendUpdates=all"))
    let sent = try #require(bodyJSON(patch)["attendees"] as? [[String: Any]])
    #expect(sent[0]["responseStatus"] as? String == "declined" && sent[1]["responseStatus"] as? String == "accepted")
    await expectWriteError(.unsupported(fields: [.attendees])) {
        _ = try await h.source.respond(to: EventRef(calendarID: cal, eventID: "ev1", seriesID: "m", originalStart: Date()), .accepted, scope: .thisAndFollowing, notify: .none)
    }
}

@Test func deleteSendsDeleteWithTheNotifyPolicyAndMapsGoneToNotFound() async throws {
    let h = try await Harness()
    await h.transport.route("\(calPath)/ev1", [HTTPResponse(status: 204)])
    try await h.source.delete(EventRef(calendarID: cal, eventID: "ev1"), scope: .thisInstance, notify: .all)
    let request = try #require(await h.transport.requests(matching: "\(calPath)/ev1").last)
    #expect(request.method == "DELETE" && request.url.absoluteString.contains("sendUpdates=all"))
    await h.transport.route("\(calPath)/ev2", [googleError("deleted", status: 410)])
    await expectWriteError(.notFound) { try await h.source.delete(EventRef(calendarID: cal, eventID: "ev2"), scope: .thisInstance, notify: .none) }
}

@Test func aForbiddenWriteIsReportedAsForbidden() async throws {
    let h = try await Harness()
    await h.transport.route("\(calPath)/ev1", [googleError("forbiddenForNonOrganizer", status: 403)])
    await expectWriteError(.forbidden(nil)) { _ = try await h.source.update(EventRef(calendarID: cal, eventID: "ev1", version: "e1"), EventPatch(title: "X"), scope: .thisInstance, notify: .none) }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `swift test --package-path Packages/CalendarConnectors --filter WriteSourceTests`
Expected: FAIL to compile (`value of type 'GoogleCalendarSource' has no member 'create'`).

- [ ] **Step 3: Implement**

`GoogleEventMapper.swift`: change the signature and stamp the source. Replace `static func map(_ dto: GoogleEventDTO, calendar: CalendarDescriptor) -> CalendarEvent? {` with `static func map(_ dto: GoogleEventDTO, calendar: CalendarDescriptor, sourceID: String? = nil) -> CalendarEvent? {`, and change the end of the returned initializer from `version: dto.etag, myResponse: attendees.first(where: \.isSelf)?.response)` to `version: dto.etag, myResponse: attendees.first(where: \.isSelf)?.response, sourceID: sourceID)`. Also make the resolver visible to the write code: change `private struct Resolved {` to `struct Resolved {` and `private static func resolve(` to `static func resolve(`.

`GoogleCalendarSource.swift`: replace the `capabilities` property with

```swift
    public var capabilities: SourceCapabilities {
        SourceCapabilities(
            canWrite: true, canEditAttendees: true, canRespondToInvite: true, providesConference: true, syncKind: .token,
            writableFields: Set(EventField.allCases), controlsNotifications: true, recurrenceScopes: [.thisInstance, .allInSeries])
    }
```

and in `events(for:in:)` change the mapping closure to `GoogleEventMapper.map($0, calendar: calendar, sourceID: connection.sourceID)`, i.e. `handle: { events += ($0.items ?? []).compactMap(\.value).compactMap { GoogleEventMapper.map($0, calendar: calendar, sourceID: self.connection.sourceID) } })`.

`SourceTests.swift` line 177: change `#expect(!c.canWrite && c.providesConference && c.syncKind == .token && !c.supportsPush)` to `#expect(c.canWrite && c.providesConference && c.syncKind == .token && !c.supportsPush)`.

Create the write implementation:

```swift
// Sources/GoogleCalendar/GoogleCalendarSource+Write.swift
import CalendarCore
import Foundation

extension GoogleCalendarSource: WritableCalendarSource {
    public func create(_ draft: EventDraft, in calendarID: String, notify: NotifyPolicy) async throws -> CalendarEvent {
        try await translated {
            try draft.validate()
            try WriteValidation.requireWritable(draft.usedFields, capabilities)
            let calendar = try await writableCalendar(calendarID)
            let body = try GoogleWriteMapper.createBody(draft)
            let data = try await api.send(
                method: "POST", path: GoogleAPIClient.calendarPath(calendarID, "/events"),
                query: query(notify, conference: body.needsConferenceVersion), body: try GoogleWriteMapper.data(body.json), mode: .write)
            return try mapped(data, calendar: calendar)
        }
    }

    public func update(_ ref: EventRef, _ patch: EventPatch, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent {
        try await translated {
            try WriteValidation.requireWritable(patch.touchedFields, capabilities)
            try patch.timing?.validate()
            let calendar = try await writableCalendar(ref.calendarID)
            if patch.isEmpty {
                if let base = patch.base { return base }
                return try mapped(try await fetchRaw(ref.calendarID, ref.eventID).data, calendar: calendar)
            }
            if scope == .thisAndFollowing, ref.seriesID != nil {
                guard let event = try await splitSeries(ref, calendar: calendar, patch: patch, notify: notify) else {
                    throw SourceError.invalidResponse("google: the split returned no event")
                }
                return event
            }
            let (targetID, useVersion) = target(ref, scope: scope)
            return try await PatchMerge.apply(
                patch: patch, version: useVersion ? ref.version : nil,
                fetchCurrent: { try self.mapped(try await self.fetchRaw(ref.calendarID, targetID).data, calendar: calendar) },
                write: { expected -> PatchMerge.Attempt<CalendarEvent> in
                    var ifMatch = expected
                    var attendees: [[String: Any]]?
                    if let changes = patch.attendees, !changes.isEmpty {
                        // Google replaces the whole array, so start from a fresh copy and judge staleness against the caller's version.
                        let raw = try await self.fetchRaw(ref.calendarID, targetID)
                        if let expected, let etag = raw.etag, etag != expected { return .stale }
                        attendees = raw.attendees
                        ifMatch = raw.etag ?? expected
                    }
                    let body = try GoogleWriteMapper.patchBody(patch, currentAttendees: attendees)
                    var headers: [String: String] = [:]
                    if let ifMatch { headers["If-Match"] = ifMatch }
                    do {
                        let data = try await self.api.send(
                            method: "PATCH", path: GoogleAPIClient.eventPath(ref.calendarID, targetID),
                            query: self.query(notify, conference: body.needsConferenceVersion), body: try GoogleWriteMapper.data(body.json),
                            headers: headers, mode: .write)
                        return .done(try self.mapped(data, calendar: calendar))
                    } catch GoogleAPIError.preconditionFailed {
                        return .stale
                    }
                })
        }
    }

    public func delete(_ ref: EventRef, scope: RecurrenceScope, notify: NotifyPolicy) async throws {
        try await translated {
            let calendar = try await writableCalendar(ref.calendarID)
            if scope == .thisAndFollowing, ref.seriesID != nil {
                _ = try await splitSeries(ref, calendar: calendar, patch: nil, notify: notify)
                return
            }
            let (targetID, _) = target(ref, scope: scope)
            _ = try await api.send(method: "DELETE", path: GoogleAPIClient.eventPath(ref.calendarID, targetID), query: query(notify), mode: .write)
        }
    }

    public func respond(to ref: EventRef, _ response: ResponseStatus, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent {
        try await translated {
            // Splitting a series only to change one guest's response is not offered.
            if scope == .thisAndFollowing, ref.seriesID != nil { throw WriteError.unsupported(fields: [.attendees]) }
            let calendar = try await writableCalendar(ref.calendarID)
            let (targetID, _) = target(ref, scope: scope)
            let raw = try await fetchRaw(ref.calendarID, targetID)
            let attendees = try GoogleWriteMapper.respondAttendees(current: raw.attendees, response: response)
            var headers: [String: String] = [:]
            if let etag = raw.etag { headers["If-Match"] = etag }
            let data = try await api.send(
                method: "PATCH", path: GoogleAPIClient.eventPath(ref.calendarID, targetID), query: query(notify),
                body: try GoogleWriteMapper.data(["attendees": attendees]), headers: headers, mode: .write)
            return try mapped(data, calendar: calendar)
        }
    }

    // MARK: Helpers

    /// Translates provider outcomes into `WriteError`; `SourceError` and `WriteError` pass through unchanged.
    private func translated<T>(_ body: () async throws -> T) async throws -> T {
        do { return try await body() }
        catch let error as GoogleAPIError {
            switch error {
            case .gone, .notFound: throw WriteError.notFound
            case .forbidden: throw WriteError.forbidden(nil)
            case .badRequest(let message): throw WriteError.invalid(message)
            case .preconditionFailed: throw WriteError.conflict(fields: [])
            }
        }
    }

    private func query(_ notify: NotifyPolicy, conference: Bool = false) -> [URLQueryItem] {
        var items = [URLQueryItem(name: "sendUpdates", value: GoogleWriteMapper.sendUpdates(notify))]
        if conference { items.append(URLQueryItem(name: "conferenceDataVersion", value: "1")) }
        return items
    }

    /// The calendar, provided it exists and the account can write to it.
    private func writableCalendar(_ calendarID: String) async throws -> CalendarDescriptor {
        guard let calendar = try await calendars().first(where: { $0.id == calendarID }) else { throw WriteError.notFound }
        guard calendar.accessRole == .owner || calendar.accessRole == .writer else { throw WriteError.forbidden("read-only calendar") }
        return calendar
    }

    private func mapped(_ data: Data, calendar: CalendarDescriptor) throws -> CalendarEvent {
        let dto = try api.decode(GoogleEventDTO.self, from: data)
        guard let event = GoogleEventMapper.map(dto, calendar: calendar, sourceID: id) else {
            throw SourceError.invalidResponse("google: unreadable event")
        }
        return event
    }

    struct RawEvent {
        var data: Data
        var json: [String: Any]
        var etag: String? { json["etag"] as? String }
        var attendees: [[String: Any]] { json["attendees"] as? [[String: Any]] ?? [] }
    }

    /// The full event resource (no `fields` mask), so unmodeled fields and `attendees(self)` are present.
    func fetchRaw(_ calendarID: String, _ eventID: String) async throws -> RawEvent {
        let data = try await api.send(method: "GET", path: GoogleAPIClient.eventPath(calendarID, eventID), mode: .write)
        guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw SourceError.invalidResponse("google: unreadable event")
        }
        return RawEvent(data: data, json: json)
    }

    /// The id a write addresses: the instance, or the series master for `.allInSeries` (whose etag differs from the
    /// instance's, so the caller's version cannot lock it).
    private func target(_ ref: EventRef, scope: RecurrenceScope) -> (id: String, useVersion: Bool) {
        guard let series = ref.seriesID else { return (ref.eventID, true) }
        return scope == .allInSeries ? (series, false) : (ref.eventID, true)
    }

    /// Implemented in Task 11.
    private func splitSeries(_ ref: EventRef, calendar: CalendarDescriptor, patch: EventPatch?, notify: NotifyPolicy) async throws -> CalendarEvent? {
        throw WriteError.unsupported(fields: [.recurrence])
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `swift test --package-path Packages/CalendarConnectors`
Expected: PASS (whole library, including the updated `SourceTests` capabilities assertion).

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "feat(google): create, update, delete and RSVP with instance and series scopes" -m "Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 11: Google `.thisAndFollowing` (truncate and split)

**Files:**
- Modify: `Packages/CalendarConnectors/Sources/GoogleCalendar/GoogleWriteMapper.swift` (append), `GoogleCalendarSource+Write.swift` (replace `splitSeries`), `GoogleCalendarSource.swift` (scopes)
- Test: `Packages/CalendarConnectors/Tests/GoogleCalendarTests/SplitSeriesTests.swift`

**Interfaces:**
- Consumes: Tasks 8 to 10.
- Produces: `GoogleWriteMapper.truncated(_:before:allDay:zone:)`, `.count(in:)`, `.replacingCount(_:with:)`, `.rruleLines(_:)`, `.newSeriesBody(master:instance:patch:recurrence:fallbackZone:)`; a working `splitSeries`; capability scopes now all three.

- [ ] **Step 1: Write the failing tests**

```swift
import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import GoogleCalendar

private let cal = "me@x.com"
private let calPath = "calendars/me%40x.com/events"
private let tokyo = TimeZone(identifier: "Asia/Tokyo")!
private func instant(_ s: String) -> Date { ISO8601DateFormatter().date(from: s)! }
private let split = "2026-09-15T15:00:00Z"   // the third weekly occurrence

private func master(recurrence: [String] = ["RRULE:FREQ=WEEKLY;COUNT=10"]) -> [String: Any] {
    googleEvent(id: "m1", etag: "em1", summary: "Standup", extra: [
        "start": ["dateTime": "2026-09-01T15:00:00Z", "timeZone": "UTC"], "end": ["dateTime": "2026-09-01T15:30:00Z", "timeZone": "UTC"],
        "description": "notes", "location": "Room", "recurrence": recurrence, "colorId": "5",
        "attendees": [["email": "me@x.com", "self": true, "organizer": true, "responseStatus": "accepted"], ["email": "bob@x.com", "responseStatus": "accepted"]],
        "conferenceData": ["conferenceSolution": ["key": ["type": "hangoutsMeet"]], "conferenceId": "abc"],
        "reminders": ["useDefault": false, "overrides": [["method": "popup", "minutes": 10]]],
        "htmlLink": "https://x", "iCalUID": "u1", "sequence": 3,
    ])
}
private func instanceJSON() -> [String: Any] {
    googleEvent(id: "m1_20260915T150000Z", etag: "ei3", summary: "Standup", extra: [
        "start": ["dateTime": split, "timeZone": "UTC"], "end": ["dateTime": "2026-09-15T15:30:00Z", "timeZone": "UTC"],
        "recurringEventId": "m1", "originalStartTime": ["dateTime": split]])
}
private func instancesPage() -> [String: Any] {
    let starts = (0..<10).map { i in ["originalStartTime": ["dateTime": ISO8601DateFormatter().string(from: instant("2026-09-01T15:00:00Z").addingTimeInterval(Double(i) * 604_800))]] }
    return ["items": starts]
}
private var ref: EventRef {
    EventRef(calendarID: cal, eventID: "m1_20260915T150000Z", version: "ei3", seriesID: "m1", originalStart: instant(split))
}

private func harness(masterResponses: [HTTPResponse], post: HTTPResponse = .json(googleEvent(id: "n1", etag: "en1", summary: "Standup"))) async throws -> Harness {
    let h = try await Harness()
    await h.transport.route("\(calPath)/m1", masterResponses)
    await h.transport.route("\(calPath)/m1_2026", [.json(instanceJSON())])
    await h.transport.route("\(calPath)/m1/instances", [.json(instancesPage())])
    await h.transport.route("\(calPath)?", [post])
    return h
}

// MARK: Pure helpers

@Test func truncationCutsTheRuleJustBeforeTheSplitAndKeepsOtherLines() {
    let lines = ["RRULE:FREQ=WEEKLY;COUNT=10;BYDAY=TU", "EXDATE:20260908T150000Z"]
    #expect(GoogleWriteMapper.truncated(lines, before: instant(split), allDay: false, zone: nil)
        == ["RRULE:FREQ=WEEKLY;BYDAY=TU;UNTIL=20260915T145959Z", "EXDATE:20260908T150000Z"])
    let allDay = GoogleWriteMapper.truncated(["RRULE:FREQ=DAILY;UNTIL=20261231"], before: instant("2026-09-14T15:00:00Z"), allDay: true, zone: tokyo)
    #expect(allDay == ["RRULE:FREQ=DAILY;UNTIL=20260914"])   // split is Sep 15 in Tokyo; the last kept day is Sep 14
}

@Test func countHelpersReadAndReplaceCount() {
    #expect(GoogleWriteMapper.count(in: ["RRULE:FREQ=WEEKLY;COUNT=10"]) == 10)
    #expect(GoogleWriteMapper.count(in: ["RRULE:FREQ=WEEKLY;UNTIL=20261231"]) == nil)
    #expect(GoogleWriteMapper.replacingCount(["RRULE:FREQ=WEEKLY;COUNT=10", "EXDATE:x"], with: 8) == ["RRULE:FREQ=WEEKLY;COUNT=8", "EXDATE:x"])
    #expect(GoogleWriteMapper.rruleLines(["RRULE:FREQ=WEEKLY", "EXDATE:x"]) == ["RRULE:FREQ=WEEKLY"])
}

@Test func theNewSeriesBodyIsBuiltFromTheMasterWithoutOutputOnlyFields() throws {
    let patch = EventPatch(location: .set("Lab"))
    let body = try GoogleWriteMapper.newSeriesBody(master: master(), instance: instanceJSON(), patch: patch,
                                                   recurrence: ["RRULE:FREQ=WEEKLY;COUNT=8"], fallbackZone: "UTC")
    let json = body.json
    for key in ["id", "etag", "iCalUID", "htmlLink", "sequence", "recurringEventId", "originalStartTime"] { #expect(json[key] == nil, "\(key) must not be copied") }
    #expect(json["summary"] as? String == "Standup" && json["description"] as? String == "notes" && json["colorId"] as? String == "5")
    #expect(json["location"] as? String == "Lab" && json["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;COUNT=8"])
    #expect((json["start"] as? [String: Any])?["dateTime"] as? String == split)
    let attendees = try #require(json["attendees"] as? [[String: Any]])
    #expect(attendees.allSatisfy { $0["responseStatus"] == nil && $0["self"] == nil && $0["organizer"] == nil })
    let request = ((json["conferenceData"] as? [String: Any])?["createRequest"] as? [String: Any])
    #expect(request != nil && body.needsConferenceVersion)   // the old Meet link is not copied; a new one is requested
    #expect(json["reminders"] != nil)
}

@Test func aPatchThatRemovesTheConferenceDoesNotRequestANewOne() throws {
    let body = try GoogleWriteMapper.newSeriesBody(master: master(), instance: instanceJSON(), patch: EventPatch(conference: .remove),
                                                   recurrence: ["RRULE:FREQ=WEEKLY"], fallbackZone: "UTC")
    #expect(body.json["conferenceData"] == nil)
}

// MARK: Flows

@Test func updateThisAndFollowingTruncatesTheMasterThenInsertsTheNewSeries() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(master())])
    let event = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .all)
    let masterRequests = await h.transport.requests(matching: "\(calPath)/m1?")
    let truncate = try #require(masterRequests.last)
    #expect(truncate.method == "PATCH" && truncate.headers["If-Match"] == "em1" && truncate.url.absoluteString.contains("sendUpdates=none"))
    #expect(bodyJSON(truncate)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;UNTIL=20260915T145959Z"])
    let post = try #require(await h.transport.requests(matching: "\(calPath)?").last)
    #expect(post.method == "POST" && post.url.absoluteString.contains("sendUpdates=all"))
    #expect(bodyJSON(post)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;COUNT=8"] && bodyJSON(post)["location"] as? String == "Lab")
    #expect(event.eventID == "n1")
}

@Test func splittingAtTheFirstOccurrenceIsTheWholeSeries() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(googleEvent(id: "m1", etag: "em2", summary: "All"))])
    let first = EventRef(calendarID: cal, eventID: "m1_20260901T150000Z", version: "i1", seriesID: "m1", originalStart: instant("2026-09-01T15:00:00Z"))
    _ = try await h.source.update(first, EventPatch(title: "All"), scope: .thisAndFollowing, notify: .none)
    let last = try #require(await h.transport.requests(matching: "\(calPath)/m1?").last)
    #expect(last.method == "PATCH" && last.headers["If-Match"] == nil && bodyJSON(last)["summary"] as? String == "All")
    #expect(await h.transport.requests(matching: "\(calPath)?").isEmpty)
}

@Test func aFailedInsertRestoresTheOriginalRule() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(master()), .json(master())], post: googleError("invalid", message: "bad start", status: 400))
    await expectWriteError(.invalid("bad start")) { _ = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none) }
    let restore = try #require(await h.transport.requests(matching: "\(calPath)/m1?").last)
    #expect(restore.method == "PATCH" && restore.headers["If-Match"] == nil)
    #expect(bodyJSON(restore)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;COUNT=10"])
}

@Test func aFailedInsertAndAFailedRestoreIsReportedAsPartial() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(master()), HTTPResponse(status: 500)],
                              post: googleError("invalid", message: "bad start", status: 400))
    do {
        _ = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none)
        Issue.record("expected .partial")
    } catch WriteError.partial {
    } catch {
        Issue.record("expected .partial, got \(error)")
    }
}

@Test func deleteThisAndFollowingOnlyTruncatesTheMaster() async throws {
    let h = try await harness(masterResponses: [.json(master()), .json(master())])
    try await h.source.delete(ref, scope: .thisAndFollowing, notify: .all)
    let truncate = try #require(await h.transport.requests(matching: "\(calPath)/m1?").last)
    #expect(truncate.method == "PATCH" && truncate.url.absoluteString.contains("sendUpdates=all"))
    #expect(bodyJSON(truncate)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;UNTIL=20260915T145959Z"])
    #expect(await h.transport.requests(matching: "\(calPath)?").isEmpty)
}

@Test func aRuleWithoutACountIsSplitWithoutListingInstances() async throws {
    let h = try await harness(masterResponses: [.json(master(recurrence: ["RRULE:FREQ=WEEKLY;UNTIL=20261231T000000Z"])), .json(master())])
    _ = try await h.source.update(ref, EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none)
    #expect(await h.transport.requests(matching: "/instances").isEmpty)
    let post = try #require(await h.transport.requests(matching: "\(calPath)?").last)
    #expect(bodyJSON(post)["recurrence"] as? [String] == ["RRULE:FREQ=WEEKLY;UNTIL=20261231T000000Z"])
}

@Test func splittingNeedsTheOccurrencesOriginalStart() async throws {
    let h = try await harness(masterResponses: [.json(master())])
    let noStart = EventRef(calendarID: cal, eventID: "m1_x", version: "i", seriesID: "m1")
    await expectWriteError(.invalid("this and following needs the occurrence's original start")) {
        _ = try await h.source.update(noStart, EventPatch(title: "X"), scope: .thisAndFollowing, notify: .none)
    }
}

@Test func googleNowAcceptsAllThreeScopes() async throws {
    let h = try await Harness()
    #expect(h.source.capabilities.recurrenceScopes == Set(RecurrenceScope.allCases))
}
```

- [ ] **Step 2: Run to verify failure**

Run: `swift test --package-path Packages/CalendarConnectors --filter SplitSeriesTests`
Expected: FAIL to compile (`type 'GoogleWriteMapper' has no member 'truncated'`).

- [ ] **Step 3: Implement**

Append to `GoogleWriteMapper.swift`:

```swift
// MARK: Series splitting

extension GoogleWriteMapper {
    /// Fields a new series must not copy from the master: identity, bookkeeping, and the old conference (a new one is
    /// requested instead).
    static let outputOnlyKeys: Set<String> = [
        "id", "etag", "iCalUID", "htmlLink", "created", "updated", "sequence", "creator", "organizer", "recurringEventId",
        "originalStartTime", "conferenceData", "hangoutLink", "kind", "status", "recurrence",
    ]

    private static func isRRule(_ line: String) -> Bool { line.uppercased().hasPrefix("RRULE:") }

    private static func parts(of line: String) -> [String] {
        line.dropFirst("RRULE:".count).split(separator: ";").map(String.init)
    }

    /// The `recurrence` lines with every RRULE cut off just before `split` (`COUNT` and `UNTIL` replaced by an `UNTIL`
    /// that is a date for all-day series and a UTC date-time, one second earlier, for timed ones). Other lines
    /// (EXDATE, RDATE) are kept. Works on the raw text, so rules outside the authorable subset are fine.
    static func truncated(_ lines: [String], before split: Date, allDay: Bool, zone: TimeZone?) -> [String] {
        let until: String
        if allDay {
            until = RecurrenceRule.dateText(AllDay.date(of: split, in: zone ?? TimeZone(identifier: "UTC")!).adding(days: -1))
        } else {
            until = RecurrenceRule.untilText(split.addingTimeInterval(-1), allDay: false, zone: nil)
        }
        return lines.map { line in
            guard isRRule(line) else { return line }
            var kept = parts(of: line).filter { !$0.uppercased().hasPrefix("COUNT=") && !$0.uppercased().hasPrefix("UNTIL=") }
            kept.append("UNTIL=" + until)
            return "RRULE:" + kept.joined(separator: ";")
        }
    }

    /// The `COUNT` of the first RRULE, if any.
    static func count(in lines: [String]) -> Int? {
        for line in lines where isRRule(line) {
            for part in parts(of: line) where part.uppercased().hasPrefix("COUNT=") { return Int(part.dropFirst("COUNT=".count)) }
        }
        return nil
    }

    static func replacingCount(_ lines: [String], with count: Int) -> [String] {
        lines.map { line in
            guard isRRule(line) else { return line }
            return "RRULE:" + parts(of: line).map { $0.uppercased().hasPrefix("COUNT=") ? "COUNT=\(count)" : $0 }.joined(separator: ";")
        }
    }

    static func rruleLines(_ lines: [String]) -> [String] { lines.filter(isRRule) }

    private static func hasMeetLink(_ json: JSON) -> Bool {
        let key = ((json["conferenceData"] as? JSON)?["conferenceSolution"] as? JSON)?["key"] as? JSON
        return key?["type"] as? String == "hangoutsMeet" || json["hangoutLink"] != nil
    }

    /// The insert body for the new series: the whole master resource (so unmodeled fields such as color, attachments and
    /// extended properties carry over) minus output-only fields, starting at the instance's own start and end, guests'
    /// responses reset, the patch applied, and a new Meet link requested when the master had one (unless the patch
    /// removes the conference).
    static func newSeriesBody(master: JSON, instance: JSON, patch: EventPatch, recurrence: [String], fallbackZone: String?) throws -> Body {
        var json = master.filter { !outputOnlyKeys.contains($0.key) }
        if let attendees = json["attendees"] as? [JSON] {
            json["attendees"] = attendees.map { attendee -> JSON in
                var copy = attendee
                copy["responseStatus"] = nil
                copy["self"] = nil
                copy["organizer"] = nil
                return copy
            }
        }
        json["start"] = instance["start"]
        json["end"] = instance["end"]
        for key in ["start", "end"] {
            if var time = json[key] as? JSON, time["dateTime"] != nil, time["timeZone"] == nil, let zone = fallbackZone {
                time["timeZone"] = zone
                json[key] = time
            }
        }
        let changes = try patchBody(patch, currentAttendees: json["attendees"] as? [JSON] ?? [])
        for (key, value) in changes.json {
            if value is NSNull { json[key] = nil } else { json[key] = value }
        }
        if patch.recurrence == .keep { json["recurrence"] = recurrence }
        var needsConferenceVersion = changes.needsConferenceVersion
        if patch.conference == nil, hasMeetLink(master) {
            json["conferenceData"] = conferenceRequestJSON()
            needsConferenceVersion = true
        }
        return Body(json: json, needsConferenceVersion: needsConferenceVersion)
    }
}
```

In `GoogleCalendarSource+Write.swift`, replace the `splitSeries` stub with the following (and add the two supporting members inside the same extension):

```swift
    private struct InstancePage: Decodable {
        struct Item: Decodable { var originalStartTime: GoogleTimeDTO? }
        var items: [Item]?
        var nextPageToken: String?
    }

    private static func timeDTO(_ value: Any?) -> GoogleTimeDTO? {
        guard let value, JSONSerialization.isValidJSONObject(value), let data = try? JSONSerialization.data(withJSONObject: value) else { return nil }
        return try? JSONDecoder().decode(GoogleTimeDTO.self, from: data)
    }

    /// Occurrences of the series that start before `split`, including individually deleted ones (they still count
    /// toward a `COUNT` rule).
    private func priorInstances(calendarID: String, masterID: String, before split: Date, zone: TimeZone) async throws -> Int {
        var count = 0
        try await api.pages(
            InstancePage.self, path: GoogleAPIClient.eventPath(calendarID, masterID) + "/instances",
            query: [
                URLQueryItem(name: "showDeleted", value: "true"), URLQueryItem(name: "maxResults", value: "250"),
                URLQueryItem(name: "fields", value: "nextPageToken,items(originalStartTime)"),
            ],
            next: { $0.nextPageToken },
            handle: { page in
                for item in page.items ?? [] {
                    if let resolved = GoogleEventMapper.resolve(item.originalStartTime, calendarZone: zone), resolved.date < split { count += 1 }
                }
            })
        return count
    }

    private func patchRecurrence(_ calendarID: String, _ eventID: String, _ lines: [String], etag: String?, notify: NotifyPolicy) async throws {
        var headers: [String: String] = [:]
        if let etag { headers["If-Match"] = etag }
        do {
            _ = try await api.send(
                method: "PATCH", path: GoogleAPIClient.eventPath(calendarID, eventID), query: query(notify),
                body: try GoogleWriteMapper.data(["recurrence": lines]), headers: headers, mode: .write)
        } catch GoogleAPIError.preconditionFailed {
            throw WriteError.conflict(fields: [.recurrence])
        }
    }

    /// `.thisAndFollowing`: cut the master's rule just before the occurrence. Delete (`patch == nil`) stops there and
    /// returns nil. Update also inserts a new series from the occurrence with the patch applied; if that fails the
    /// master's original rule is restored, and if restoring fails too the result is `WriteError.partial`. Splitting at
    /// the first occurrence is the same as `.allInSeries`. Truncation notifies attendees only for delete; for update
    /// the insert carries the notification, so they are not told twice.
    private func splitSeries(_ ref: EventRef, calendar: CalendarDescriptor, patch: EventPatch?, notify: NotifyPolicy) async throws -> CalendarEvent? {
        guard let masterID = ref.seriesID, let split = ref.originalStart else {
            throw WriteError.invalid("this and following needs the occurrence's original start")
        }
        let zone = calendar.timeZone ?? TimeZone(identifier: "UTC")!
        let master = try await fetchRaw(ref.calendarID, masterID)
        guard let first = Self.timeDTO(master.json["start"]).flatMap({ GoogleEventMapper.resolve($0, calendarZone: zone) }) else {
            throw SourceError.invalidResponse("google: the series has no start")
        }
        if abs(first.date.timeIntervalSince(split)) < 1 {
            if let patch { return try await update(ref, patch, scope: .allInSeries, notify: notify) }
            try await delete(ref, scope: .allInSeries, notify: notify)
            return nil
        }
        let original = master.json["recurrence"] as? [String] ?? []
        guard !GoogleWriteMapper.rruleLines(original).isEmpty else { throw WriteError.invalid("the series has no recurrence rule") }
        let truncated = GoogleWriteMapper.truncated(original, before: split, allDay: first.isAllDay, zone: first.zone ?? zone)
        try await patchRecurrence(ref.calendarID, masterID, truncated, etag: master.etag, notify: patch == nil ? notify : .none)
        guard let patch else { return nil }

        do {
            let instance = try await fetchRaw(ref.calendarID, ref.eventID)
            var lines = GoogleWriteMapper.rruleLines(original)
            if let total = GoogleWriteMapper.count(in: original) {
                let prior = try await priorInstances(calendarID: ref.calendarID, masterID: masterID, before: split, zone: zone)
                guard total - prior >= 1 else { throw WriteError.invalid("the series has no occurrences left to split off") }
                lines = GoogleWriteMapper.replacingCount(lines, with: total - prior)
            }
            let masterZone = (master.json["start"] as? [String: Any])?["timeZone"] as? String
            let body = try GoogleWriteMapper.newSeriesBody(
                master: master.json, instance: instance.json, patch: patch, recurrence: lines,
                fallbackZone: masterZone ?? calendar.timeZone?.identifier ?? "UTC")
            let data = try await api.send(
                method: "POST", path: GoogleAPIClient.calendarPath(ref.calendarID, "/events"),
                query: query(notify, conference: body.needsConferenceVersion), body: try GoogleWriteMapper.data(body.json), mode: .write)
            return try mapped(data, calendar: calendar)
        } catch {
            do {
                try await patchRecurrence(ref.calendarID, masterID, original, etag: nil, notify: .none)
            } catch {
                throw WriteError.partial("the series was cut off before this occurrence but the new series could not be created, and restoring the original rule failed")
            }
            throw error
        }
    }
```

In `GoogleCalendarSource.swift` change `recurrenceScopes: [.thisInstance, .allInSeries]` to `recurrenceScopes: Set(RecurrenceScope.allCases)`, and in `WriteSourceTests.swift` change the expectation in `googleDeclaresItsWriteCapabilities` to `c.recurrenceScopes == Set(RecurrenceScope.allCases)`. (The plan's `respond` guard still refuses `.thisAndFollowing`, as the spec reconciliation in Task 15 records.)

- [ ] **Step 4: Run to verify pass**

Run: `swift test --package-path Packages/CalendarConnectors`
Expected: PASS (whole library).

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarConnectors
git commit -m "feat(google): this-and-following via truncation and a new series, with rollback" -m "Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 12: EventKit read additions and pure write mapping

Gate: read the Task 1 findings first. If the spike showed that `occurrenceDate` or `hasRecurrenceRules` is unreliable, adjust the `map` change below accordingly (the fallback is to leave `seriesID`/`originalStart` nil and drop the recurrence scopes from EventKit's capabilities in Task 13).

**Files:**
- Create: `Packages/EventKitSource/Sources/EventKitSource/EventKitWriteMapping.swift`
- Modify: `Packages/EventKitSource/Sources/EventKitSource/EventKitSource.swift`
- Test: `Packages/EventKitSource/Tests/EventKitSourceTests/EventKitWriteMappingTests.swift`

**Interfaces:**
- Consumes: `RecurrenceRule`, `EventTiming`, `Reminder`, `NotifyPolicy`, `RecurrenceScope`, `WriteError`, `AllDay`, `EventKitMapping.canonicalAllDay` (existing).
- Produces (internal, in `enum EventKitWriteMapping`): `recurrenceRule(_:) -> EKRecurrenceRule`, `alarms(_:) -> [EKAlarm]`, `span(for:) -> EKSpan`, `version(_:) -> String?`, `checkNotify(_:hasOtherAttendees:) throws`, `floatingAllDay(_:calendar:) -> (start: Date, end: Date)?`. `EventKitSource.init(store:)`, and internal `store`, `requireAccess()`, `map(_:)`. EventKit reads now fill `version`, `sourceID`, `seriesID`, `originalStart`.

- [ ] **Step 1: Write the failing tests**

```swift
import CalendarCore
import EventKit
import Foundation
import Testing
@testable import EventKitSource

private func iso(_ s: String) -> Date { ISO8601DateFormatter().date(from: s)! }
private func newYork() -> Calendar {
    var c = Calendar(identifier: .gregorian)
    c.timeZone = TimeZone(identifier: "America/New_York")!
    return c
}

@Test func weeklyRuleMapsFrequencyIntervalDaysAndCount() {
    let rule = RecurrenceRule(frequency: .weekly, interval: 2, weekdays: [.init(.monday), .init(.thursday)], end: .count(6))
    let ek = EventKitWriteMapping.recurrenceRule(rule)
    #expect(ek.frequency == .weekly && ek.interval == 2)
    #expect(ek.daysOfTheWeek?.map(\.dayOfTheWeek) == [.monday, .thursday])
    #expect(ek.recurrenceEnd?.occurrenceCount == 6)
}

@Test func monthlyOrdinalsMonthDaysAndYearlyMonthsMap() {
    let ordinal = EventKitWriteMapping.recurrenceRule(RecurrenceRule(frequency: .monthly, weekdays: [.init(.tuesday, ordinal: 2), .init(.friday, ordinal: -1)]))
    #expect(ordinal.daysOfTheWeek?.map(\.weekNumber) == [2, -1])
    let byDay = EventKitWriteMapping.recurrenceRule(RecurrenceRule(frequency: .monthly, monthDays: [1, 15, -1]))
    #expect(byDay.daysOfTheMonth?.map(\.intValue) == [1, 15, -1])
    let yearly = EventKitWriteMapping.recurrenceRule(RecurrenceRule(frequency: .yearly, months: [3, 9]))
    #expect(yearly.monthsOfTheYear?.map(\.intValue) == [3, 9])
}

@Test func untilAndNeverEndsMap() {
    let until = iso("2026-12-31T00:00:00Z")
    #expect(EventKitWriteMapping.recurrenceRule(RecurrenceRule(frequency: .daily, end: .until(until))).recurrenceEnd?.endDate == until)
    #expect(EventKitWriteMapping.recurrenceRule(RecurrenceRule(frequency: .daily)).recurrenceEnd == nil)
}

@Test func remindersBecomeNegativeRelativeOffsets() {
    let alarms = EventKitWriteMapping.alarms([Reminder(minutesBefore: 10), Reminder(minutesBefore: 60)])
    #expect(alarms.map(\.relativeOffset) == [-600, -3600])
}

@Test func onlyThisInstanceUsesTheSingleEventSpan() {
    #expect(EventKitWriteMapping.span(for: .thisInstance) == .thisEvent)
    #expect(EventKitWriteMapping.span(for: .thisAndFollowing) == .futureEvents)
    #expect(EventKitWriteMapping.span(for: .allInSeries) == .futureEvents)
}

@Test func versionIsTheModificationDateOrNil() {
    #expect(EventKitWriteMapping.version(nil) == nil)
    #expect(EventKitWriteMapping.version(Date(timeIntervalSince1970: 1_790_000_000.5)) == "1790000000.5")
}

@Test func notificationPolicyOtherThanAllIsRefusedOnlyWhenOthersWouldBeNotified() async {
    #expect(throws: Never.self) { try EventKitWriteMapping.checkNotify(.all, hasOtherAttendees: true) }
    #expect(throws: Never.self) { try EventKitWriteMapping.checkNotify(.none, hasOtherAttendees: false) }
    do {
        try EventKitWriteMapping.checkNotify(.externalOnly, hasOtherAttendees: true)
        Issue.record("expected .unsupported")
    } catch let error as WriteError {
        #expect(error == .unsupported(fields: [.attendees]))
    } catch {
        Issue.record("wrong error \(error)")
    }
}

@Test func floatingAllDayRebuildsTheCalendarDatesInTheDeviceZoneAndRoundTrips() throws {
    // Sep 18 and 19 in New York (exclusive end Sep 20), authored in New York.
    let timing = EventTiming(start: iso("2026-09-18T04:00:00Z"), end: iso("2026-09-20T04:00:00Z"), timeZone: TimeZone(identifier: "America/New_York")!, isAllDay: true)
    let floating = try #require(EventKitWriteMapping.floatingAllDay(timing, calendar: newYork()))
    #expect(floating.start == iso("2026-09-18T04:00:00Z") && floating.end == iso("2026-09-20T03:59:59Z"))
    let canonical = EventKitMapping.canonicalAllDay(start: floating.start, end: floating.end, calendar: newYork())
    #expect(canonical.start == timing.start && canonical.end == timing.end)
}

@Test func floatingAllDayKeepsTheCalendarDatesWhenTheDeviceZoneDiffers() throws {
    // Tokyo Sep 18 (one day) written on a New York device is still Sep 18 there.
    let tokyo = TimeZone(identifier: "Asia/Tokyo")!
    let timing = EventTiming(start: iso("2026-09-17T15:00:00Z"), end: iso("2026-09-18T15:00:00Z"), timeZone: tokyo, isAllDay: true)
    let floating = try #require(EventKitWriteMapping.floatingAllDay(timing, calendar: newYork()))
    #expect(floating.start == iso("2026-09-18T04:00:00Z") && floating.end == iso("2026-09-19T03:59:59Z"))
}

@Test func floatingAllDayNeedsAZone() {
    let timing = EventTiming(start: iso("2026-09-18T00:00:00Z"), end: iso("2026-09-19T00:00:00Z"), timeZone: nil, isAllDay: true)
    #expect(EventKitWriteMapping.floatingAllDay(timing, calendar: newYork()) == nil)
}
```

- [ ] **Step 2: Run to verify failure**

Run: `swift test --package-path Packages/EventKitSource --filter EventKitWriteMappingTests`
Expected: FAIL to compile (`cannot find 'EventKitWriteMapping' in scope`).

- [ ] **Step 3: Implement**

```swift
// Sources/EventKitSource/EventKitWriteMapping.swift
import CalendarCore
import EventKit
import Foundation

/// Pure conversions for EventKit writes, kept free of `EKEventStore` so they are unit-testable.
enum EventKitWriteMapping {
    static func frequency(_ frequency: RecurrenceRule.Frequency) -> EKRecurrenceFrequency {
        switch frequency {
        case .daily: .daily
        case .weekly: .weekly
        case .monthly: .monthly
        case .yearly: .yearly
        }
    }

    static func weekday(_ weekday: RecurrenceRule.Weekday) -> EKWeekday {
        switch weekday {
        case .monday: .monday
        case .tuesday: .tuesday
        case .wednesday: .wednesday
        case .thursday: .thursday
        case .friday: .friday
        case .saturday: .saturday
        case .sunday: .sunday
        }
    }

    static func recurrenceRule(_ rule: RecurrenceRule) -> EKRecurrenceRule {
        let days = rule.weekdays.map { EKRecurrenceDayOfWeek(dayOfTheWeek: weekday($0.weekday), weekNumber: $0.ordinal ?? 0) }
        let end: EKRecurrenceEnd?
        switch rule.end {
        case .never: end = nil
        case .count(let count): end = EKRecurrenceEnd(occurrenceCount: count)
        case .until(let date): end = EKRecurrenceEnd(end: date)
        }
        return EKRecurrenceRule(
            recurrenceWith: frequency(rule.frequency), interval: rule.interval,
            daysOfTheWeek: days.isEmpty ? nil : days,
            daysOfTheMonth: rule.monthDays.isEmpty ? nil : rule.monthDays.map { NSNumber(value: $0) },
            monthsOfTheYear: rule.months.isEmpty ? nil : rule.months.map { NSNumber(value: $0) },
            weeksOfTheYear: nil, daysOfTheYear: nil, setPositions: nil, end: end)
    }

    static func alarms(_ reminders: [Reminder]) -> [EKAlarm] {
        reminders.map { EKAlarm(relativeOffset: -TimeInterval($0.minutesBefore * 60)) }
    }

    /// `.thisInstance` saves one occurrence; both other scopes save the occurrence and everything after it (for
    /// `.allInSeries` the caller starts from the series' first occurrence).
    static func span(for scope: RecurrenceScope) -> EKSpan {
        scope == .thisInstance ? .thisEvent : .futureEvents
    }

    /// EventKit has no etag; the modification date is the version.
    static func version(_ modified: Date?) -> String? {
        modified.map { String($0.timeIntervalSince1970) }
    }

    /// EventKit cannot control notifications (the server decides), so anything but `.all` is refused when other
    /// attendees would be affected. With no other attendees every policy is accepted and ignored.
    static func checkNotify(_ policy: NotifyPolicy, hasOtherAttendees: Bool) throws {
        if policy != .all && hasOtherAttendees { throw WriteError.unsupported(fields: [.attendees]) }
    }

    /// EventKit stores all-day events as floating device-local dates whose end is the end of the last day (the shape
    /// `EventKitMapping.canonicalAllDay` reads). Rebuilds the calendar dates of `timing` (in its own zone) in
    /// `calendar`'s zone. nil when the timing has no zone.
    static func floatingAllDay(_ timing: EventTiming, calendar: Calendar) -> (start: Date, end: Date)? {
        guard let zone = timing.timeZone else { return nil }
        let days = AllDay.dates(start: timing.start, end: timing.end, in: zone)
        let lastDay = days.endExclusive.adding(days: -1)
        guard let first = AllDay.startOfDay(days.first, in: calendar.timeZone),
              let last = AllDay.startOfDay(lastDay, in: calendar.timeZone),
              let end = calendar.date(byAdding: DateComponents(day: 1, second: -1), to: last)
        else { return nil }
        return (first, end)
    }
}
```

In `EventKitSource.swift`:
- replace `private let store = EKEventStore()` and `public init() {}` with

```swift
    let store: EKEventStore

    public init(store: EKEventStore = EKEventStore()) {
        self.store = store
    }
```
- change `private func requireAccess()` to `func requireAccess()` and `private func map(_ event: EKEvent)` to `func map(_ event: EKEvent)`.
- in `map`, before `return CalendarEvent(`, add `let isSeries = event.hasRecurrenceRules || event.isDetached`, and change the initializer call to pass, between `availability:` and `attendees:`, `seriesID: isSeries ? event.eventIdentifier : nil, originalStart: isSeries ? event.occurrenceDate : nil,`; and after `url: event.url,` add `version: EventKitWriteMapping.version(event.lastModifiedDate),`; and at the very end (after `myResponse: ...`) add `, sourceID: EventKitSource.sourceID`. The call then reads:

```swift
        return CalendarEvent(
            eventID: event.eventIdentifier ?? event.calendarItemIdentifier,
            uid: event.calendarItemExternalIdentifier,
            calendarID: event.calendar.calendarIdentifier,
            title: event.title ?? "(No title)",
            notes: event.notes, location: event.location, start: start, end: end, timeZone: zone,
            isAllDay: event.isAllDay, status: event.status == .tentative ? .tentative : .confirmed,
            availability: event.availability == .free ? .free : .busy,
            seriesID: isSeries ? event.eventIdentifier : nil, originalStart: isSeries ? event.occurrenceDate : nil,
            attendees: attendees, organizer: event.organizer.map { attendee($0, isOrganizer: true) },
            url: event.url, version: EventKitWriteMapping.version(event.lastModifiedDate),
            myResponse: me.flatMap { EventKitMapping.response($0.participantStatus) }, sourceID: EventKitSource.sourceID)
```

- [ ] **Step 4: Run to verify pass**

Run: `swift test --package-path Packages/EventKitSource`
Expected: PASS (the existing mapping and connector-kind tests too; `EventKitConnectorKind` still compiles because `init(source: EventKitSource = EventKitSource())` uses the defaulted initializer).

- [ ] **Step 5: Commit**

```bash
git add Packages/EventKitSource
git commit -m "feat(eventkit): series, version and source ids on reads; pure write mapping" -m "Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 13: `EventKitSource` as a `WritableCalendarSource`

**Files:**
- Create: `Packages/EventKitSource/Sources/EventKitSource/EventKitSource+Write.swift`
- Modify: `Packages/EventKitSource/Sources/EventKitSource/EventKitSource.swift` (capabilities)
- Test: `Packages/EventKitSource/Tests/EventKitSourceTests/EventKitWriteTests.swift` (unit, no store) and `EventKitLiveWriteTests.swift` (live, gated)

**Interfaces:**
- Consumes: Task 12 mapping, `PatchMerge`, `WriteValidation`, `WritableSourceConformance` (from `CalendarTestSupport`, already a test dependency), `withScratchCalendar`/`nextHour`/`liveEventKit` (Task 1).
- Produces: `EventKitSource: WritableCalendarSource` with capabilities `canWrite`, `writableFields = [.title, .notes, .location, .timing, .availability, .reminders, .recurrence]`, `controlsNotifications = false`, `recurrenceScopes` all three (drop `.allInSeries` if Task 1 said it does not work).

- [ ] **Step 1: Write the failing tests**

Unit test (declares the capability contract without touching a store):

```swift
// EventKitWriteTests.swift
import CalendarCore
import EventKit
import Foundation
import Testing
@testable import EventKitSource

@Test func eventKitDeclaresWhatItCanAndCannotWrite() {
    let caps = EventKitSource().capabilities
    #expect(caps.canWrite && !caps.canEditAttendees && !caps.canRespondToInvite && !caps.controlsNotifications)
    #expect(caps.writableFields == [.title, .notes, .location, .timing, .availability, .reminders, .recurrence])
    #expect(caps.recurrenceScopes == Set(RecurrenceScope.allCases))
    #expect(caps.canEditAttendees == caps.writableFields.contains(.attendees))
    #expect(EventKitSource() is any WritableCalendarSource)
}
```

Live tests (opt-in; they use the scratch calendar from Task 1):

```swift
// EventKitLiveWriteTests.swift
import CalendarCore
import CalendarTestSupport
import EventKit
import Foundation
import Testing
@testable import EventKitSource

@Test(.enabled(if: liveEventKit)) func eventKitPassesTheWritableConformanceChecks() async throws {
    try await withScratchCalendar { store, calendar in
        let source = EventKitSource(store: store)
        let window = DateInterval(start: nextHour(daysAhead: 1), duration: 86_400 * 3)
        let violations = await WritableSourceConformance.violations(of: source, calendarID: calendar.calendarIdentifier, window: window)
        #expect(violations.isEmpty, "\(violations)")
    }
}

@Test(.enabled(if: liveEventKit)) func eventKitRecurringScopesEditTheRightOccurrences() async throws {
    try await withScratchCalendar { store, calendar in
        let source = EventKitSource(store: store)
        let id = calendar.calendarIdentifier
        let start = nextHour(daysAhead: 3)
        let window = DateInterval(start: start.addingTimeInterval(-3600), duration: 86_400 * 40)
        let draft = EventDraft(
            title: "Series", timing: EventTiming(start: start, end: start.addingTimeInterval(1800), timeZone: .current, isAllDay: false),
            recurrence: RecurrenceRule(frequency: .weekly, end: .count(4)))
        _ = try await source.create(draft, in: id, notify: .none)

        func occurrences() async throws -> [CalendarEvent] {
            try await source.events(in: window).filter { $0.calendarID == id }.sorted { $0.start < $1.start }
        }
        var list = try await occurrences()
        #expect(list.count == 4 && list.allSatisfy { $0.seriesID != nil && $0.originalStart != nil && $0.sourceID == "eventkit" })

        _ = try await source.update(EventRef(list[1]), EventPatch(title: "Only second"), scope: .thisInstance, notify: .none)
        list = try await occurrences()
        #expect(list.map(\.title) == ["Series", "Only second", "Series", "Series"])

        _ = try await source.update(EventRef(list[2]), EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none)
        list = try await occurrences()
        #expect(list.map(\.location) == [nil, nil, "Lab", "Lab"])

        _ = try await source.update(EventRef(list[3]), EventPatch(notes: .set("all")), scope: .allInSeries, notify: .none)
        list = try await occurrences()
        #expect(list.allSatisfy { $0.notes == "all" })

        try await source.delete(EventRef(list[2]), scope: .thisAndFollowing, notify: .none)
        list = try await occurrences()
        #expect(list.count == 2)
        try await source.delete(EventRef(list[0]), scope: .allInSeries, notify: .none)
        let remaining = try await occurrences()
        #expect(remaining.isEmpty)
    }
}

@Test(.enabled(if: liveEventKit)) func eventKitStaleVersionsMergeOrConflict() async throws {
    try await withScratchCalendar { store, calendar in
        let source = EventKitSource(store: store)
        let start = nextHour(daysAhead: 2)
        let draft = EventDraft(title: "Meeting", timing: EventTiming(start: start, end: start.addingTimeInterval(1800), timeZone: .current, isAllDay: false))
        let created = try await source.create(draft, in: calendar.calendarIdentifier, notify: .none)

        // Someone else changes the location in the meantime (a separate save bumps lastModifiedDate).
        try await Task.sleep(for: .seconds(1.2))
        let external = try #require(store.event(withIdentifier: created.eventID))
        external.location = "Elsewhere"
        try store.save(external, span: .thisEvent, commit: true)

        var edit = EventEdit(created)
        edit.event.title = "Renamed"
        let merged = try await source.update(EventRef(created), edit.patch, scope: .thisInstance, notify: .none)
        #expect(merged.title == "Renamed" && merged.location == "Elsewhere")

        // Now they change the title, and our stale edit of the title must conflict.
        try await Task.sleep(for: .seconds(1.2))
        let again = try #require(store.event(withIdentifier: created.eventID))
        again.title = "Theirs"
        try store.save(again, span: .thisEvent, commit: true)
        var second = EventEdit(merged)
        second.event.title = "Mine"
        do {
            _ = try await source.update(EventRef(merged), second.patch, scope: .thisInstance, notify: .none)
            Issue.record("expected a conflict")
        } catch let error as WriteError {
            #expect(error == .conflict(fields: [.title]))
        }
    }
}

@Test(.enabled(if: liveEventKit)) func eventKitRefusesWhatItCannotWrite() async throws {
    try await withScratchCalendar { store, calendar in
        let source = EventKitSource(store: store)
        let start = nextHour(daysAhead: 2)
        let timing = EventTiming(start: start, end: start.addingTimeInterval(1800), timeZone: .current, isAllDay: false)
        var draft = EventDraft(title: "T", timing: timing, attendees: [AttendeeDraft(email: "a@b.c")])
        do {
            _ = try await source.create(draft, in: calendar.calendarIdentifier, notify: .none)
            Issue.record("expected .unsupported")
        } catch let error as WriteError {
            #expect(error == .unsupported(fields: [.attendees]))
        }
        draft.attendees = []
        let created = try await source.create(draft, in: calendar.calendarIdentifier, notify: .none)
        do {
            _ = try await source.respond(to: EventRef(created), .accepted, scope: .thisInstance, notify: .none)
            Issue.record("expected .unsupported")
        } catch let error as WriteError {
            #expect(error == .unsupported(fields: [.attendees]))
        }
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `swift test --package-path Packages/EventKitSource --filter EventKitWriteTests`
Expected: FAIL (`EventKitSource() is any WritableCalendarSource` is false: "cast from 'EventKitSource' to unrelated type" or the `canWrite` expectation fails).

- [ ] **Step 3: Implement**

In `EventKitSource.swift` replace the `capabilities` line with:

```swift
    public var capabilities: SourceCapabilities {
        SourceCapabilities(
            canWrite: true, providesConference: false, syncKind: .notification,
            writableFields: [.title, .notes, .location, .timing, .availability, .reminders, .recurrence],
            controlsNotifications: false, recurrenceScopes: Set(RecurrenceScope.allCases))
    }
```

Create the write implementation:

```swift
// Sources/EventKitSource/EventKitSource+Write.swift
import CalendarCore
import EventKit
import Foundation

extension EventKitSource: WritableCalendarSource {
    public func create(_ draft: EventDraft, in calendarID: String, notify: NotifyPolicy) async throws -> CalendarEvent {
        try requireAccess()
        try draft.validate()
        try WriteValidation.requireWritable(draft.usedFields, capabilities)
        try EventKitWriteMapping.checkNotify(notify, hasOtherAttendees: false)
        let calendar = try writableCalendar(calendarID)
        let event = EKEvent(eventStore: store)
        event.calendar = calendar
        event.title = draft.title
        event.notes = draft.notes
        event.location = draft.location
        applyTiming(draft.timing, to: event)
        event.availability = draft.availability == .free ? .free : .busy
        if let reminders = draft.reminders { event.alarms = EventKitWriteMapping.alarms(reminders) }
        if let rule = draft.recurrence { event.addRecurrenceRule(EventKitWriteMapping.recurrenceRule(rule)) }
        try save(event, span: .thisEvent)
        return map(reload(event, isSeries: draft.recurrence != nil))
    }

    public func update(_ ref: EventRef, _ patch: EventPatch, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent {
        try requireAccess()
        try WriteValidation.requireWritable(patch.touchedFields, capabilities)
        try patch.timing?.validate()
        let target = try locateTarget(ref, scope: scope)
        if patch.isEmpty { return patch.base ?? map(target.event) }
        try EventKitWriteMapping.checkNotify(notify, hasOtherAttendees: hasOtherAttendees(target.event))
        // An occurrence's modification date does not describe the whole series, so a series-wide write has no lock.
        let version = scope == .allInSeries && ref.seriesID != nil ? nil : ref.version
        return try await PatchMerge.apply(
            patch: patch, version: version,
            fetchCurrent: { self.map(try self.locateTarget(ref, scope: scope).event) },
            write: { expected -> PatchMerge.Attempt<CalendarEvent> in
                let current = try self.locateTarget(ref, scope: scope)
                if let expected, expected != EventKitWriteMapping.version(current.event.lastModifiedDate) { return .stale }
                self.apply(patch, to: current.event)
                try self.save(current.event, span: current.span)
                return .done(self.map(self.reload(current.event, isSeries: ref.seriesID != nil)))
            })
    }

    public func delete(_ ref: EventRef, scope: RecurrenceScope, notify: NotifyPolicy) async throws {
        try requireAccess()
        let target = try locateTarget(ref, scope: scope)
        try EventKitWriteMapping.checkNotify(notify, hasOtherAttendees: hasOtherAttendees(target.event))
        do { try store.remove(target.event, span: target.span, commit: true) }
        catch { throw WriteError.invalid(error.localizedDescription) }
    }

    /// EventKit does not let apps change a response.
    public func respond(to ref: EventRef, _ response: ResponseStatus, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent {
        throw WriteError.unsupported(fields: [.attendees])
    }

    // MARK: Helpers

    private func writableCalendar(_ id: String) throws -> EKCalendar {
        guard let calendar = store.calendar(withIdentifier: id) else { throw WriteError.notFound }
        guard calendar.allowsContentModifications else { throw WriteError.forbidden("read-only calendar") }
        return calendar
    }

    /// The event as stored after a save, so the returned `version` is the stored modification date. A series edit can
    /// re-identify the occurrences after it, so a series event is returned as saved.
    private func reload(_ event: EKEvent, isSeries: Bool) -> EKEvent {
        guard !isSeries, let id = event.eventIdentifier, let stored = store.event(withIdentifier: id) else { return event }
        return stored
    }

    private func hasOtherAttendees(_ event: EKEvent) -> Bool {
        (event.attendees ?? []).contains { !$0.isCurrentUser }
    }

    private func save(_ event: EKEvent, span: EKSpan) throws {
        do { try store.save(event, span: span, commit: true) }
        catch { throw WriteError.invalid(error.localizedDescription) }
    }

    /// The occurrence a ref designates. A recurring occurrence is found through a date-range predicate around its
    /// `originalStart` and matched on `eventIdentifier` and `occurrenceDate` (occurrences share an identifier). A
    /// deleted event, or one `refresh()` reports as gone, is `.notFound`.
    private func locate(_ ref: EventRef) throws -> EKEvent {
        var found: EKEvent?
        if ref.seriesID != nil {
            guard let original = ref.originalStart else { throw WriteError.invalid("a recurring occurrence needs its original start") }
            let predicate = store.predicateForEvents(withStart: original.addingTimeInterval(-86_400), end: original.addingTimeInterval(2 * 86_400), calendars: nil)
            found = store.events(matching: predicate).first {
                $0.eventIdentifier == ref.eventID && abs($0.occurrenceDate.timeIntervalSince(original)) < 1
            }
        } else {
            found = store.event(withIdentifier: ref.eventID)
        }
        guard let event = found, event.refresh() else { throw WriteError.notFound }
        return event
    }

    /// The event to change and the span to save it with. `.allInSeries` starts from the series' first occurrence and
    /// saves with `.futureEvents`, which edits every occurrence.
    private func locateTarget(_ ref: EventRef, scope: RecurrenceScope) throws -> (event: EKEvent, span: EKSpan) {
        guard ref.seriesID != nil else { return (try locate(ref), .thisEvent) }
        if scope == .allInSeries {
            guard let first = store.event(withIdentifier: ref.eventID), first.refresh() else { throw WriteError.notFound }
            return (first, .futureEvents)
        }
        return (try locate(ref), EventKitWriteMapping.span(for: scope))
    }

    private func applyTiming(_ timing: EventTiming, to event: EKEvent) {
        if timing.isAllDay, let floating = EventKitWriteMapping.floatingAllDay(timing, calendar: Calendar.current) {
            event.isAllDay = true
            event.startDate = floating.start
            event.endDate = floating.end
        } else {
            event.isAllDay = false
            event.startDate = timing.start
            event.endDate = timing.end
            event.timeZone = timing.timeZone
        }
    }

    private func apply(_ patch: EventPatch, to event: EKEvent) {
        if let title = patch.title { event.title = title }
        switch patch.notes { case .keep: break; case .set(let value): event.notes = value; case .clear: event.notes = nil }
        switch patch.location { case .keep: break; case .set(let value): event.location = value; case .clear: event.location = nil }
        if let timing = patch.timing { applyTiming(timing, to: event) }
        if let availability = patch.availability { event.availability = availability == .free ? .free : .busy }
        switch patch.reminders {
        case .keep: break
        case .set(let list): event.alarms = EventKitWriteMapping.alarms(list)
        case .clear: event.alarms = nil
        }
        switch patch.recurrence {
        case .keep: break
        case .clear: event.recurrenceRules?.forEach(event.removeRecurrenceRule)
        case .set(let rule):
            event.recurrenceRules?.forEach(event.removeRecurrenceRule)
            event.addRecurrenceRule(EventKitWriteMapping.recurrenceRule(rule))
        }
    }
}
```

- [ ] **Step 4: Run to verify pass, then the live suite on the user's Mac**

Run: `swift test --package-path Packages/EventKitSource`
Expected: PASS (unit tests; live tests are skipped without the variable).

Then, with the user: `TIMETUG_LIVE_EVENTKIT=1 swift test --package-path Packages/EventKitSource --filter Live 2>&1 | tail -40`
Expected: all four live write tests pass. If a scope test fails, record the observed behaviour in the spec's Risks, then either fix the mapping or remove that scope from `recurrenceScopes` (and update the capabilities test and the spec table).

- [ ] **Step 5: Commit**

```bash
git add Packages/EventKitSource
git commit -m "feat(eventkit): create, update and delete through EventKit, with strict field checks" -m "Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 14: Google live smoke test (user-run)

The events scope cannot create calendars, so the smoke test works in the primary calendar with clearly named events, no attendees, and always deletes what it created.

**Files:**
- Modify: `Packages/CalendarApple/Package.swift` (test target gains `GoogleCalendar`)
- Create: `Packages/CalendarApple/Tests/CalendarAppleTests/GoogleLiveWriteSmokeTests.swift`

**Interfaces:**
- Consumes: `GoogleConnectorKind`, `LoopbackAuthorizationInteraction`, `CryptoKitSHA256`, `InMemoryCredentialStore`, `InMemorySyncStateStore`, `WritableCalendarSource`.

- [ ] **Step 1: Add the test dependency.** In `Packages/CalendarApple/Package.swift`, add `.product(name: "GoogleCalendar", package: "CalendarConnectors")` to the `CalendarAppleTests` target's dependencies (next to `CalendarOAuth`).

- [ ] **Step 2: Write the smoke test**

```swift
import CalendarCore
import CalendarOAuth
import Foundation
import GoogleCalendar
import Testing
@testable import CalendarApple

private let liveGoogle = ProcessInfo.processInfo.environment["TIMETUG_LIVE_GOOGLE"] == "1"

/// Opt-in, interactive: `TIMETUG_LIVE_GOOGLE=1 GOOGLE_OAUTH_CLIENT_ID=... GOOGLE_OAUTH_CLIENT_SECRET=... swift test
/// --package-path Packages/CalendarApple --filter googleWriteSmoke`. Signs in through the browser, creates events named
/// "TimeTug write smoke" in the primary calendar (no attendees, nothing is emailed), and deletes them at the end.
@Test(.enabled(if: liveGoogle), .timeLimit(.minutes(10))) func googleWriteSmoke() async throws {
    let env = ProcessInfo.processInfo.environment
    let clientID = try #require(env["GOOGLE_OAUTH_CLIENT_ID"])
    let clientSecret = try #require(env["GOOGLE_OAUTH_CLIENT_SECRET"])
    let kind = GoogleConnectorKind(config: GoogleOAuthConfig(clientID: clientID, clientSecret: clientSecret), hasher: CryptoKitSHA256())
    let interaction = LoopbackAuthorizationInteraction(openURL: { url in
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/open")
        process.arguments = [url.absoluteString]
        do { try process.run(); return true } catch { return false }
    })
    let credentials = InMemoryCredentialStore()
    let connection = try await kind.authorize(using: interaction, credentials: credentials)
    let source = try kind.makeSource(for: connection, credentials: credentials, syncState: InMemorySyncStateStore())
    let writable = try #require(source as? WritableCalendarSource)
    let calendars = try await source.calendars()
    let primary = try #require(calendars.first(where: \.isPrimary))
    print("LIVE signed in as \(connection.displayName); writing to the primary calendar")

    let utc = TimeZone(identifier: "UTC")!
    let tomorrow = Calendar(identifier: .gregorian).date(byAdding: .day, value: 1, to: Date())!
    let start = Calendar(identifier: .gregorian).date(bySettingHour: 3, minute: 0, second: 0, of: tomorrow)!
    func timing(_ offsetDays: Int) -> EventTiming {
        let s = start.addingTimeInterval(Double(offsetDays) * 86_400)
        return EventTiming(start: s, end: s.addingTimeInterval(1800), timeZone: utc, isAllDay: false)
    }
    let window = DateInterval(start: start.addingTimeInterval(-3600), duration: 86_400 * 40)
    var cleanup: [EventRef] = []
    func mine() async throws -> [CalendarEvent] {
        try await source.events(in: window).filter { $0.title.hasPrefix("TimeTug write smoke") && $0.calendarID == primary.id }.sorted { $0.start < $1.start }
    }

    do {
        // 1. A single event: create, edit through EventEdit, then a stale edit merges and a conflicting one fails.
        let created = try await writable.create(EventDraft(title: "TimeTug write smoke", timing: timing(0), location: "Room 1"), in: primary.id, notify: .none)
        cleanup.append(EventRef(created))
        #expect(created.title == "TimeTug write smoke" && created.sourceID == source.id)
        var edit = EventEdit(created)
        edit.event.title = "TimeTug write smoke renamed"
        let renamed = try await writable.update(EventRef(created), edit.patch, scope: .thisInstance, notify: .none)
        #expect(renamed.title == "TimeTug write smoke renamed" && renamed.location == "Room 1" && renamed.version != created.version)
        // `created` is now stale. A patch to notes only merges; a patch to the title conflicts.
        var notesEdit = EventEdit(created)
        notesEdit.event.notes = "merged"
        let merged = try await writable.update(EventRef(created), notesEdit.patch, scope: .thisInstance, notify: .none)
        #expect(merged.notes == "merged" && merged.title == "TimeTug write smoke renamed")
        var titleEdit = EventEdit(created)
        titleEdit.event.title = "TimeTug write smoke mine"
        do {
            _ = try await writable.update(EventRef(created), titleEdit.patch, scope: .thisInstance, notify: .none)
            Issue.record("expected a conflict")
        } catch let error as WriteError { #expect(error == .conflict(fields: [.title])) }
        try await writable.delete(EventRef(merged), scope: .thisInstance, notify: .none)
        cleanup.removeAll()

        // 2. A weekly series of four: instance, this-and-following and whole-series edits, then delete.
        let series = try await writable.create(
            EventDraft(title: "TimeTug write smoke series", timing: timing(2), recurrence: RecurrenceRule(frequency: .weekly, end: .count(4))),
            in: primary.id, notify: .none)
        cleanup.append(EventRef(series))
        try await Task.sleep(for: .seconds(2))
        var list = try await mine()
        #expect(list.count == 4 && list.allSatisfy { $0.seriesID != nil && $0.originalStart != nil })
        _ = try await writable.update(EventRef(list[1]), EventPatch(title: "TimeTug write smoke series (second)"), scope: .thisInstance, notify: .none)
        _ = try await writable.update(EventRef(list[2]), EventPatch(location: .set("Lab")), scope: .thisAndFollowing, notify: .none)
        try await Task.sleep(for: .seconds(2))
        list = try await mine()
        print("LIVE after split: \(list.map { ($0.title, $0.location ?? "-") })")
        #expect(list.count == 4 && list[0].location == nil && list[1].title.hasSuffix("(second)") && list[2].location == "Lab" && list[3].location == "Lab")
        _ = try await writable.update(EventRef(list[3]), EventPatch(notes: .set("whole series")), scope: .allInSeries, notify: .none)
        try await Task.sleep(for: .seconds(2))
        list = try await mine()
        // Instances after the split belong to the new series, so only the series the ref points at is guaranteed to change.
        print("LIVE notes after allInSeries: \(list.map { $0.notes ?? "-" })")
        // Delete each series once: the split left the old and the new series both present.
        for seriesID in Set(list.compactMap(\.seriesID)) {
            if let one = list.first(where: { $0.seriesID == seriesID }) {
                try await writable.delete(EventRef(one), scope: .allInSeries, notify: .none)
            }
        }
        cleanup.removeAll()
    } catch {
        print("LIVE failure: \(error)")
        for ref in cleanup { try? await writable.delete(ref, scope: .allInSeries, notify: .none) }
        var seen = Set<String>()
        for leftover in (try? await mine()) ?? [] where seen.insert(leftover.seriesID ?? leftover.eventID).inserted {
            try? await writable.delete(EventRef(leftover), scope: .allInSeries, notify: .none)
        }
        throw error
    }
    let leftovers = try await mine()
    #expect(leftovers.isEmpty, "smoke events were left behind: \(leftovers.map(\.title))")
}
```

- [ ] **Step 3: Run it with the user**

Run (the client id and secret come from the user's git-ignored `~/.config/timetug/google-oauth.xcconfig`; the user exports them in their shell; never print them):
`TIMETUG_LIVE_GOOGLE=1 GOOGLE_OAUTH_CLIENT_ID=... GOOGLE_OAUTH_CLIENT_SECRET=... swift test --package-path Packages/CalendarApple --filter googleWriteSmoke 2>&1 | grep -E "LIVE|passed|failed|Issue"`
Expected: a browser sign-in opens; the output shows the `LIVE` lines, the test passes, and no "TimeTug write smoke" events remain in the calendar. Note what it shows for Risk 2 in the spec (etag `If-Match` behaviour on PATCH, the recurring insert needing a zone, the truncation and split results).

- [ ] **Step 4: Record the findings** in the spec's Risks section (replace items 2 and 3 with what was observed). If `.thisAndFollowing` misbehaves, fix `GoogleWriteMapper.newSeriesBody` or `splitSeries` (with a failing unit test first), or remove `.thisAndFollowing` from Google's `recurrenceScopes` and update the tests and the spec table.

- [ ] **Step 5: Commit**

```bash
git add Packages/CalendarApple docs/superpowers/specs/2026-09-23-calendar-connectors-phase3-design.md
git commit -m "test: opt-in live Google write smoke test" -m "Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 15: Docs and spec reconciliation

**Files:**
- Modify: `docs/calendar-connectors-api.md`, `docs/superpowers/specs/2026-09-23-calendar-connectors-phase3-design.md`, `docs/decisions/0012-calendar-connector-library.md`, `AGENTS.md`, `docs/manual-tests/macos-checklist.md`

- [ ] **Step 1: Reconcile the spec with what was built.** Edit the spec so it matches the code:
  - Capabilities table and Google scopes row: all three scopes for Google; EventKit per Task 1's findings.
  - "An empty patch": replace "makes no request (a series is never split without a change to carry)" with "performs no write: it returns the patch's `base`, or fetches and returns the current event when the patch has none (a series is never split without a change to carry)".
  - `respond` with `.thisAndFollowing` on Google throws `.unsupported(fields: [.attendees])` (splitting a series only to change one response is not offered).
  - Testing section: conformance runs against `FakeWritableSource` and, live, against the real EventKit source; Google is covered by request-shape tests with the fake transport (a stateful fake backend was judged too heavy). The Google live smoke test uses the primary calendar with clearly named, attendee-free events because the events scope cannot create calendars.
  - `EventKitSource.init(store:)` exists so live tests share a store.

- [ ] **Step 2: Update the API contract** (`docs/calendar-connectors-api.md`): in the status line say parts 1 to 10 describe `master`; retitle part 10 from "Proposed: write capabilities (Phase 3)" to "Write capabilities (Phase 3)" and delete the "Status: Proposed" paragraph; make sure part 10 matches the final signatures (`writableFields`, `originalStart`, `WriteError.partial`, `EventPatch.base`, `PatchMerge`); in part 3.2 replace "A source is read-only unless it also conforms to `WritableCalendarSource` (Phase 3, part 10)" with "(part 10)"; in part 3.3 update the capabilities struct and the "Current values" line (Google now writes everything; EventKit per its capabilities); in parts 5 and 7 add a short "Writes" bullet each; remove the known gap about EventKit ids if Task 1 resolved it and the `seriesID`/`originalStart`/`version` gap (now filled).

- [ ] **Step 3: ADR 0012 addendum.** Append a `## Phase 3: write capabilities` section: opt-in by `WritableCalendarSource`, capabilities (`writableFields`, `controlsNotifications`, `recurrenceScopes`), patch-based updates with `base` and field-level conflicts, explicit `NotifyPolicy`, Google `.thisAndFollowing` as truncate plus new series (caveats), EventKit restrictions, links kept local (provider metadata deferred).

- [ ] **Step 4: `AGENTS.md`.** In the `Packages/CalendarConnectors` bullet, change "`GoogleCalendar` (read-only Google connector)" to "`GoogleCalendar` (Google connector with optional writes)" and add "and the optional write API (`WritableCalendarSource`, `EventDraft`, `EventPatch`, `RecurrenceRule`, `PatchMerge`)" to the `CalendarCore` list; in Gotchas add: "Writes are opt-in: `capabilities.canWrite == (source is WritableCalendarSource)`, unsupported fields throw `WriteError.unsupported`, updates send only changed fields and a stale version is judged per field against `EventPatch.base`. Live write tests are opt-in (`TIMETUG_LIVE_EVENTKIT=1`, `TIMETUG_LIVE_GOOGLE=1`) and never run in CI."

- [ ] **Step 5: Manual test checklist.** Append to `docs/manual-tests/macos-checklist.md`:

```markdown
- [ ] Calendar write smoke (before releasing a change to the write API): `TIMETUG_LIVE_EVENTKIT=1 swift test --package-path Packages/EventKitSource --filter Live` passes, and the Google smoke test (see `AGENTS.md`, Gotchas) passes and leaves no "TimeTug write smoke" events behind.
```

- [ ] **Step 6: Commit**

```bash
git add docs AGENTS.md
git commit -m "docs: Phase 3 write capabilities (contract, ADR 0012, AGENTS.md, checklist)" -m "Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 16: Full verification, review and PR

**Files:** none new.

- [ ] **Step 1: Run every package suite**

```bash
swift test --package-path Packages/CalendarConnectors
swift test --package-path Packages/TimeTugCore
swift test --package-path Packages/CalendarBridge
swift test --package-path Packages/CalendarApple
swift test --package-path Packages/EventKitSource
swift test --package-path Packages/AppleIntelligenceInference
```
Expected: all PASS. Tests that compare events built by a source with hand-built events may now see `sourceID`; fix them by stamping the expected event or comparing fields (only if a failure appears).

- [ ] **Step 2: Check the library stays portable.** Run `grep -rEn "import (EventKit|CoreGraphics|Security|AppKit|AuthenticationServices|Network)" Packages/CalendarConnectors/Sources` and expect no output. If Docker is available, also run the Linux job's command locally (`docker run --rm -v "$PWD":/w -w /w swift:6.0 swift test --package-path Packages/CalendarConnectors`); otherwise rely on CI's `core-linux` job and say so.

- [ ] **Step 3: App tests.** `xcodegen generate --spec Apps/macOS/project.yml && xcodebuild -project Apps/macOS/TimeTug.xcodeproj -scheme TimeTug -destination 'platform=macOS' test`, then `git checkout -- Apps/macOS/Sources/Info.plist Apps/macOS/Widgets/Info.plist`. Expected: PASS. Never run `pkill -x TimeTug`.

- [ ] **Step 4: DeepSeek review of the diff.** Use the `deepseek-review` skill with `--files` set to the new and changed source files (`git diff --name-only origin/master...HEAD -- 'Packages/*/Sources'`), `--context` the spec, the API contract and the new tests, and questions targeting: the `PatchMerge` loop, `GoogleCalendarSource+Write` attendee flow and `splitSeries` rollback, `EventPatch(from:to:)` data loss, `EventKitSource+Write` occurrence lookup, capability invariants. Scan-flagged sync/page-token variable names may be allowed line by line after checking each. Verify every Critical or Important finding against the repo, fix the real ones with a failing test first, and tell the user how many were real.

- [ ] **Step 5: Push and open the PR** (branch `claude/phase-3-definition-f7c388`, base `master`). Title `Calendar connectors Phase 3: optional write API, Google and EventKit writes`. Body: what was built, the opt-in model, the verification commands and results, the live-test results (or that they still need the user's run), the review calibration, and the known caveats (Google `.thisAndFollowing` two-call split; EventKit lastModifiedDate granularity). End the body with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`. Do not merge; tell the user the PR is ready and wait for their word.

---

## Self-review (run after writing; results recorded here)

**Spec coverage:** goals 1 to 5 map to Tasks 2, 7, 10, 13 (opt-in, capabilities, strictness); patch safety to Tasks 5, 6, 9, 10; Google to Tasks 8 to 11; EventKit to Tasks 12, 13; recurrence subset to Task 3; copying to Task 4; conflicts to Task 6; `sourceID` to Tasks 2, 10, 12; docs to Task 15; risks 1, 2, 3, 5 to Tasks 1, 14; process (DeepSeek, PR) to Task 16. Spec deviations recorded in Task 15: empty-patch wording, Google `respond` for `.thisAndFollowing`, conformance only against the fake and live EventKit, Google smoke in the primary calendar.

**Type consistency:** `EventRef(calendarID:eventID:version:seriesID:originalStart:)`, `EventPatch.base`, `WriteError` cases (`unsupported/conflict/notFound/forbidden/invalid/partial`), `PatchMerge.apply(patch:version:maxAttempts:fetchCurrent:write:)`, `SourceCapabilities.writableFields/controlsNotifications/recurrenceScopes`, `GoogleAPIClient.send(method:path:query:body:headers:mode:)` and `GoogleWriteMapper.Body` are used with the same names and labels in every task.
