# Calendar connectors, Phase 3: write capabilities

Status: implemented on the Phase 3 branch and reconciled with the code as built (this document describes what the code does; deviations from the original design are folded in). The EventKit verification spike (Risks 1, 4 and 5) has not been run yet, and the Google items marked "unverified" wait for the live smoke test. Phases 1, 2 and 2.5 are merged (#16, #17, #18). This phase adds an optional, provider-neutral write API to the connector library, implements it for Google and EventKit, and leaves TimeTug's UI and Core untouched. The read-side contract is in `docs/calendar-connectors-api.md`; part 10 of that document is the summary of this spec.

## Goals

1. A source can *opt in* to writing by adopting `WritableCalendarSource`. Read-only connectors (any provider or feed that can be read but not written) implement nothing new and cannot be mistaken for writable.
2. The write API covers create, update, delete and respond-to-invite, with recurrence scopes (this instance, this and following, all in series) and an explicit attendee-notification policy.
3. Writes are safe against data we do not model: updates send only what changed, and a concurrent edit to an unrelated field never causes a spurious failure.
4. Google (REST) and EventKit implement the API. EventKit is the second implementation, so the API is proven not to be Google-shaped, and it is the connector that reports restricted capabilities (no attendee edits, no RSVP, no notification control).
5. Each connector declares exactly what it can write. Anything it cannot write is an error (`WriteError.unsupported`), never a silent partial write.

## Non-goals

- No TimeTug write UI, no Core or bridge changes beyond keeping existing tests green.
- No orchestration of linked events across calendars or accounts. Links, when TimeTug needs them, live in TimeTug's local store; the write API already supports the per-source calls that orchestration would make.
- No provider-side custom metadata (Google `extendedProperties`, iCalendar `X-` properties, Graph extensions). A `metadata` field and capability can be added later without breaking this API.
- No Microsoft or CalDAV connector (Phase 4 and later).
- No reading of a series' recurrence rule (reads still return expanded instances only). No moving an event between calendars. No editing an event's status, kind or provider-owned fields.
- No attendee editing or RSVP on EventKit (EventKit does not allow either).

## Decisions

Made with the user during brainstorming:

- **Opt-in by protocol.** `WritableCalendarSource: CalendarSource` is a separate protocol; callers check `source as? WritableCalendarSource`. Fine-grained limits are `capabilities` flags, and per-calendar limits are `CalendarDescriptor.accessRole`. (Rejected: write methods on `CalendarSource` throwing `.unsupported` by default; a connector-level static declaration on `ConnectorKind`.)
- **Scope: API + Google + EventKit.** No app-level write hook.
- **Recurrence: a structured RFC 5545 subset.** Rules outside the subset are rejected, not mangled.
- **Notifications: an explicit `NotifyPolicy` argument with no default**, so a caller cannot email attendees by accident.
- **Patch is the write primitive.** `update` takes an `EventPatch` of changed fields, never a whole event, because `CalendarEvent` deliberately carries fewer fields than a provider's event and a whole-event replace would delete the rest. A diff initializer and an `EventEdit` wrapper give callers the "mutate a copy" ergonomics.
- **Conflicts are field-level.** A stale version triggers a fetch and a three-way check on the fields the patch touches; only a real overlap is an error.
- **Provider metadata deferred; links stay local.**

## Placement

| Package | New or changed |
|---|---|
| `CalendarCore` (`Write/`) | `WritableCalendarSource`, `EventDraft`, `EventPatch`, `EventEdit`, `EventRef`, `RecurrenceRule`, `RecurrenceScope`, `NotifyPolicy`, `WriteError`, `WriteValidation`, `PatchMerge`, `SourceCapabilities` additions (`writableFields`, `controlsNotifications`, `recurrenceScopes`), `CalendarEvent.sourceID` |
| `CalendarTestSupport` | `WritableSourceConformance`, `FakeWritableSource` |
| `GoogleCalendar` | write client and mapper, `GoogleCalendarSource: WritableCalendarSource`, `GoogleAPIClient.send` |
| `EventKitSource` | `EventKitWriteMapping`, `EventKitSource: WritableCalendarSource` (`EventKitSource+Write.swift`), `init(store:)`, `version`, `sourceID`, `seriesID` and `originalStart` on read events |
| `CalendarBridge`, `TimeTugCore`, app | none (existing tests updated only where `CalendarEvent` equality now sees `sourceID`) |

Everything new in `CalendarCore` is pure Swift (Foundation only) so `core-linux` keeps building it.

## Capabilities

`SourceCapabilities` gains three fields, all defaulting to the read-only value so existing initializers and sources are unchanged:

```swift
var writableFields: Set<EventField> = []   // the fields create/update can write; drives validation and EventDraft(copying:for:)
var controlsNotifications: Bool = false    // honors NotifyPolicy (Google: sendUpdates); false = the server decides
var recurrenceScopes: Set<RecurrenceScope> = []   // scopes accepted for update/delete/respond on a series
```

Existing `canWrite`, `canEditAttendees` and `canRespondToInvite` keep their meaning. Invariants: `capabilities.canWrite == (source is WritableCalendarSource)`; `canEditAttendees == writableFields.contains(.attendees)`; `writableFields` and `recurrenceScopes` are empty unless `canWrite`; `canRespondToInvite` implies `canWrite`. `WritableSourceConformance` checks only two of them (`canWrite` is true on a writable source, and `canEditAttendees == writableFields.contains(.attendees)`); the rest are conventions that the read-only defaults and unit tests uphold. Authoring a `RecurrenceRule` needs `.recurrence` in `writableFields`.

| | `canWrite` | `canEditAttendees` | `canRespondToInvite` | `writableFields` | `controlsNotifications` | `recurrenceScopes` |
|---|---|---|---|---|---|---|
| Google | true | true | true | all | true | all three (an `.allInSeries` time change only from the series' first occurrence, see Scopes) |
| EventKit | true | false | false | `.title`, `.notes`, `.location`, `.timing`, `.availability`, `.reminders`, `.recurrence` | false | all three, as coded (subject to the unrun EventKit spike, see Risks; an `.allInSeries` time change only from the series' first occurrence) |
| Read-only connector | false | false | false | empty | false | empty |

## API

```swift
public protocol WritableCalendarSource: CalendarSource {
    func create(_ draft: EventDraft, in calendarID: String, notify: NotifyPolicy) async throws -> CalendarEvent
    func update(_ ref: EventRef, _ patch: EventPatch, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent
    func delete(_ ref: EventRef, scope: RecurrenceScope, notify: NotifyPolicy) async throws
    func respond(to ref: EventRef, _ response: ResponseStatus, scope: RecurrenceScope, notify: NotifyPolicy) async throws -> CalendarEvent
}

public enum NotifyPolicy: Sendable { case all, externalOnly, none }
public enum RecurrenceScope: Sendable { case thisInstance, thisAndFollowing, allInSeries }
```

`respond` accepts `.accepted`, `.tentative` and `.declined`; `.needsAction` throws `.invalid`. `scope` is ignored when the ref is not part of a series. `respond` with `.thisAndFollowing` on Google throws `.unsupported(fields: [.attendees])`: splitting a series only to change one response is not offered.

**Return values.** Writes return the event in the provider's resulting form, including its new `version`, so calls can chain. `create` of a recurring event and a series-wide Google write return the series' first occurrence or master (Google's master resource has no `seriesID`); callers that need instances re-read. `delete` returns nothing.

### Errors

```swift
public enum EventField: Sendable, Hashable { case title, notes, location, timing, availability, visibility, reminders, attendees, recurrence, conference }

public enum WriteError: Error, Sendable, Equatable {
    case unsupported(fields: Set<EventField>)   // the connector cannot write these (or the operation)
    case conflict(fields: Set<EventField>)      // someone else changed a field this patch touches
    case notFound                               // the event or calendar is gone
    case forbidden(String?)                     // read-only calendar or no permission on this event
    case invalid(String)                        // malformed input (end before start, needsAction RSVP, ...)
    case partial(String)                        // a multi-step write stopped half way (Google .thisAndFollowing: series truncated, new series not created)
}

public enum ConferenceRequest: Sendable { case none, generate }   // on a draft
public enum ConferenceChange: Sendable { case generate, remove }  // on a patch
```

An unsupported *operation* is reported against the closest field: `respond` on a connector that cannot RSVP throws `.unsupported(fields: [.attendees])`. Authentication, network, rate-limit and server failures stay `SourceError`, thrown by the same methods. Validation (`invalid`, `unsupported`) happens before any write request or store mutation (Google's `.allInSeries` timing refusal and the split checks read the master first). `WriteValidation.requireWritable(_:_:)` is the shared check of a set of fields against `writableFields`. On EventKit the same holds for the store: writes validate before `requireAccess()`, so malformed input is reported even when calendar access is missing.

### `EventRef`

```swift
public struct EventRef: Hashable, Sendable {
    var calendarID: String
    var eventID: String
    var version: String?       // the version the caller last saw; used for optimistic locking
    var seriesID: String?      // set for an instance of a series
    var originalStart: Date?   // the occurrence's slot in its series (Google originalStartTime, EventKit occurrenceDate); differs from start for a moved occurrence
    init(_ event: CalendarEvent)
}
```

`originalStart` does two jobs: it identifies the occurrence when `eventID` is shared (EventKit) and it is the split point for `.thisAndFollowing`. A ref with a `seriesID` but no `originalStart` is handled per connector. Google: only `.thisAndFollowing` needs it (it throws `.invalid`, "this and following needs the occurrence's original start"); `.thisInstance`, `.allInSeries` and `respond` never read it, except that an `.allInSeries` update with a timing change and no `originalStart` is `.unsupported(fields: [.timing])`. EventKit: `.thisInstance` and `.thisAndFollowing` throw `.invalid` (a recurring occurrence is located by its original start); `.allInSeries` starts from the series' first occurrence and needs it only for a timing change (`.unsupported(fields: [.timing])` without it).

`CalendarEvent` gains `sourceID: String?` (last initializer parameter, default nil), stamped by the source that produced it (`Connection.sourceID` for network connectors, `"eventkit"` for EventKit). It lets a host route an event to its account without a wrapper. `CalendarEvent.id` is unchanged by it; a host that holds several sources uses `event.sourceID` to find the right one.

### `EventDraft` (create)

```swift
public struct EventDraft: Sendable, Equatable {
    var title: String
    var notes: String?; var location: String?
    var timing: EventTiming
    var availability: Availability = .busy
    var visibility: Visibility = .default
    var reminders: [Reminder]?            // nil = the calendar's defaults; empty = none
    var attendees: [AttendeeDraft] = []
    var conference: ConferenceRequest = .none   // .none | .generate
    var recurrence: RecurrenceRule?
    init(copying event: CalendarEvent, for capabilities: SourceCapabilities)
    func validate() throws                    // timing, recurrence, attendee emails, non-negative reminders
    var usedFields: Set<EventField> { get }   // what the draft sets beyond defaults; checked against `writableFields`
}
public struct EventTiming: Sendable, Equatable {   // start, end, timeZone, isAllDay; canonical all-day form
    init(start: Date, end: Date, timeZone: TimeZone?, isAllDay: Bool)   // memberwise, never throws
    func validate() throws                                              // .invalid if end <= start, a time is not finite, or all-day without a zone or off midnight; every write calls it before any request
}
public struct AttendeeDraft: Sendable, Hashable {   // init(email:name:role:) trims and lowercases the email
    private(set) var email: String; var name: String?; var role: AttendeeRole
}
```

`EventDraft(copying:for:)` is the lenient path for copying an event to another calendar or account: it carries over only the fields in the target's `writableFields` and drops the rest (for example attendees, visibility and conference for EventKit). An empty `reminders` list on the source becomes `nil` (the target calendar's defaults), because reads cannot tell "defaults" from "none" (see Risks). Attachments, extended properties and colors are not modeled and are not copied. Direct writes are strict; only this constructor is best-effort.

### `EventPatch` (update)

```swift
public enum FieldUpdate<Value: Sendable & Equatable>: Sendable, Equatable { case keep, set(Value), clear }

public struct EventPatch: Sendable, Equatable {
    var title: String?                       // nil = keep
    var notes: FieldUpdate<String> = .keep
    var location: FieldUpdate<String> = .keep
    var timing: EventTiming?                 // time is one unit: start, end, zone and all-day together
    var availability: Availability?
    var visibility: Visibility?
    var reminders: FieldUpdate<[Reminder]> = .keep   // .clear = calendar defaults; .set([]) = none
    var attendees: AttendeeChanges?          // a delta, never a replacement list
    var recurrence: FieldUpdate<RecurrenceRule> = .keep
    var conference: ConferenceChange?        // .generate | .remove
    var touchedFields: Set<EventField> { get }
    var isEmpty: Bool { get }
    private(set) var base: CalendarEvent?        // the original the patch was diffed from; nil for a hand-built patch
    init(title:notes:location:timing:availability:visibility:reminders:attendees:recurrence:conference:)   // all defaulted; hand-built, no `base`
    init(from original: CalendarEvent, to edited: CalendarEvent)   // non-throwing; sets `base`
    func withoutBase() -> EventPatch
    func applied(to event: CalendarEvent) -> CalendarEvent   // used by the in-memory test source
}
public struct AttendeeChanges: Sendable, Equatable { var add: [AttendeeDraft]; var remove: [String] /* trimmed, lowercased on init */ }
```

`AttendeeChanges.add` upserts by email: an existing attendee keeps their response and takes the new role, and the new name when one is given. An empty patch performs no write: it returns the patch's `base`, or fetches and returns the current event when the patch has none (a series is never split without a change to carry).

`base` is what makes field-level conflict handling possible: `update` receives only an `EventRef`, so the values the caller edited *from* travel inside the patch. A hand-built patch has no base and gets the strict behavior described under Conflict handling.

`EventPatch(from:to:)` compares `title`, `notes`, `location`, timing (`start`, `end`, `timeZone`, `isAllDay`), `availability`, `visibility`, `reminders` and `attendees` (by normalized email, so a changed role is an upsert; attendees without an email cannot be targeted and are ignored), plus removal of `conference`. The account owner (the `isSelf` attendee) is never diffed, even when the edited copy lost the flag. A name cleared to `nil` with the role unchanged is not an attendee change, because an `AttendeeDraft` cannot express clearing a name. The diff ignores provider-owned fields (`id`, `eventID`, `uid`, `calendarID`, `sourceID`, `organizer`, `status`, `kind`, `url`, `version`, `seriesID`, `originalStart`, `myResponse`, attendees' `response`, `isSelf`, `isOrganizer`), a changed or added `conference` (only `.generate` and `.remove` are writable) and `recurrence` (reads do not carry one). Callers who need to set a recurrence rule build the patch by hand. Separately, `applied(to:)` (used by the in-memory test source) turns a `.clear` on reminders into `[]`, because that source has no calendar defaults.

### `EventEdit`

```swift
public struct EventEdit: Sendable {
    public let original: CalendarEvent
    public var event: CalendarEvent          // the working copy
    public var patch: EventPatch { EventPatch(from: original, to: event) }
    public var hasChanges: Bool { !patch.isEmpty }
}
```

Only callers that edit hold two copies; reads and widgets stay single-copy, and `CalendarEvent`'s `Hashable` behavior is unchanged apart from `sourceID`.

### `RecurrenceRule`

```swift
public struct RecurrenceRule: Hashable, Sendable {
    enum Frequency { case daily, weekly, monthly, yearly }
    enum Weekday { case monday, tuesday, wednesday, thursday, friday, saturday, sunday }
    struct WeekdayOccurrence: Hashable, Sendable { var weekday: Weekday; var ordinal: Int? }   // ordinal: ±1...±5, monthly/yearly only
    enum End: Hashable, Sendable { case never, count(Int), until(Date) }
    var frequency: Frequency
    var interval: Int = 1                     // >= 1
    var weekdays: [WeekdayOccurrence] = []
    var monthDays: [Int] = []                 // 1...31 or -31...-1
    var months: [Int] = []                    // 1...12
    var end: End = .never
    init(rrule: String, in zone: TimeZone? = nil) throws   // accepts an optional "RRULE:" prefix; a date-only UNTIL is that day in `zone` (default UTC)
    func validate() throws
    func rruleString(allDay: Bool, in zone: TimeZone?) -> String
}
```

`init(rrule:)` supports `FREQ` (daily to yearly), `INTERVAL`, `COUNT`, `UNTIL`, `BYDAY` (with ordinals), `BYMONTHDAY`, `BYMONTH` and `WKST` (accepted, must be the default). Everything else (`BYSETPOS`, `BYHOUR`, `BYYEARDAY`, `BYWEEKNO`, sub-daily frequencies, `COUNT` together with `UNTIL`) throws `WriteError.unsupported([.recurrence])`. `until` is an instant; `rruleString` renders it as a UTC date-time for timed events and as a date in `zone` for all-day events (Google requires `UNTIL` to have the same form as `DTSTART`). Validation (`interval >= 1`, ordinal only on monthly and yearly frequencies, ranges) runs in the throwing parse and in write validation; a daily rule with `BYDAY` is `.unsupported([.recurrence])`.

EXDATE and RDATE are not authorable; removing one occurrence is `delete` with `.thisInstance`.

## Conflict handling (shared, in `CalendarCore`)

`PatchMerge` is a pure helper used by every connector that has a version to compare:

```swift
public enum PatchMerge {
    public enum Attempt<Result> { case done(Result), stale }
    public static func conflicts(patch: EventPatch, current: CalendarEvent) -> Set<EventField>
    public static func apply<Result>(patch: EventPatch, version: String?, maxAttempts: Int = 3,
        fetchCurrent: () async throws -> CalendarEvent,
        write: (String?) async throws -> Attempt<Result>) async throws -> Result
}
```

1. The connector sends the write with the ref's `version` (Google: `If-Match`; EventKit: compares `lastModifiedDate` before saving); `write` reports `.stale` when the provider rejects it.
2. On `.stale` it fetches the current event (`current`).
3. `PatchMerge.conflicts(patch:current:)` returns the touched fields whose value in `current` differs from `patch.base`. Timing is compared as a unit (start, end, zone identifier, all-day). Notes and location treat `nil` and `""` as equal. Reminders are compared by their minutes, ignoring order (providers do not promise to keep it). Attendees are compared only for the emails the patch adds or removes, by role: an email conflicts when its role changed between `base` and `current` and `current` does not already hold what the patch wants (a wanted role, or the attendee already gone), so an attendee someone else already added or removed is not a conflict. A patch that touches `recurrence` always conflicts on a stale version (reads carry no rule to compare). A patch with no `base` cannot be compared, so a stale version is `.conflict(fields: touchedFields)` immediately.
4. Non-empty result: throws `.conflict(fields: conflicting)`.
5. Empty result: the patch is re-applied on `current`'s version. If that write is stale again, go back to step 2 with the newer `current`, so a second concurrent edit is judged the same way. `apply` makes at most `maxAttempts` writes (clamped to at least 1) and checks for cancellation before each; if the event is still changing, it throws `.conflict(fields: touchedFields)` (documented as "being edited concurrently").

The retry fails closed: when a retry is due, the original write was versioned (`version` non-nil) and the fetched event has no version (`nil`), `apply` throws `.conflict(fields: touchedFields)`, because retrying without a version would be an unconditional write that could overwrite a change made between the fetch and the write. Only a write that started unversioned (no `ref.version`) retries unconditionally, since it had nothing to lose.

Deletes are last-writer-wins: `delete` ignores `ref.version` on both connectors (Google sends no `If-Match`; EventKit does no modification-date check), so it removes the event even if someone edited it after the caller read it. This is a deliberate carve-out from the rule that a stale version is judged per field: a patch has fields to compare, a delete has none.

Attendee changes go through the same check: Google fetches the current event, and if its etag differs from `ref.version` reports `.stale` so `PatchMerge` runs before the delta is applied. `respond` on Google does not use `PatchMerge`: only the caller's own response changes, so a 412 simply restarts from a fresh fetch (at most three times, then `.conflict(fields: [.attendees])`).

Series-wide Google writes (`.allInSeries`) have no usable version: an instance's etag does not cover the master. They send no `If-Match` and overwrite only the touched fields. EventKit does the same for `.allInSeries` (an occurrence's modification date does not describe the series). This is documented behavior, not a bug.

## Google implementation

`GoogleAPIClient` gains `send(method:path:query:body:headers:)`, refactored out of `get` so the 401-refresh, rate-limit backoff and 5xx handling are shared (`get` becomes a thin wrapper). The client keeps its existing provider-level outcomes, so reads behave exactly as before: 410 stays `GoogleAPIError.gone` (the sync-token reset in `GoogleCalendarSource.poll` depends on it), 404 `.notFound`, a 403 `forbidden` reason `.forbidden`, and `insufficientPermissions` stays `SourceError.authExpired`. Two cases are added, in write mode only (`GoogleRequestMode.write`; reads keep the old mapping): 412 becomes `GoogleAPIError.preconditionFailed` and 400 becomes `GoogleAPIError.badRequest(message)`. Write mode also changes 403 handling: a quota reason (`quotaExceeded`, `calendarUsageLimitsExceeded`, `dailyLimitExceeded`) surfaces as `SourceError.rateLimited` (retry later, not a permission problem), and any other reason except `insufficientPermissions` (which stays `SourceError.authExpired`) is `GoogleAPIError.forbidden`. Only the write layer translates these into `WriteError` (`gone` and `notFound` to `.notFound`, `forbidden` to `.forbidden`, `badRequest` to `.invalid`, `preconditionFailed` into the merge loop); nothing new escapes from a read method. `percentEncode` uses an ASCII allow-list (`CharacterSet.alphanumerics` admits non-ASCII letters that trap `URLComponents.percentEncodedPath`).

Write-side fetches (the pre-write GET for attendee edits and RSVP, the master and instance fetches for `.thisAndFollowing`, and the fetch of a patch's current event) use no `fields` mask, so the full resource including `attendees(self)` and unmodeled fields is available. A fetched event with status `cancelled` is `.notFound`, so a deleted event is never patched back to life. Every write first checks that the calendar exists and its `accessRole` is owner or writer (`.notFound` or `.forbidden` otherwise).

| Operation | Request |
|---|---|
| create | `POST /calendars/{id}/events?sendUpdates=…`; adds `conferenceDataVersion=1` and `conferenceData.createRequest` (fresh `requestId`, `hangoutsMeet`) when a Meet link is requested |
| update | `PATCH /calendars/{id}/events/{eventId}` with only the touched fields, `If-Match`, `sendUpdates=…` |
| delete | `DELETE …/events/{eventId}?sendUpdates=…` |
| respond | `GET` the event, set the self attendee's `responseStatus`, `PATCH` the full `attendees` array with the fetched etag; a 412 restarts from a fresh `GET` (three tries, then `.conflict(fields: [.attendees])`) |

Field mapping: `summary`, `description`, `location`; `start` and `end` as `date` (all-day, exclusive end, through `AllDay`) or `dateTime` plus `timeZone`, always sent together, and always with a zone for recurring events; `availability` to `transparency` (`opaque`/`transparent`); `visibility` to `default/public/private/confidential`; `reminders` to `{useDefault: true}` when a patch clears them (a draft's nil list omits the key, so the calendar's defaults apply), else `{useDefault: false, overrides: [{method: popup, minutes}]}` (at most 5 reminders, 0 to 40320 minutes, otherwise `.invalid`); a patch's timing also sends explicit `null`s for the other form's keys so an event can change between all-day and timed; `recurrence` to `["RRULE:…"]`. A `.clear` on notes or location sends JSON `null`.

**Attendees.** Google replaces the whole array on PATCH, so an attendee delta (and `respond`) first fetches the current event, runs the conflict check above if its etag differs from `ref.version`, applies the change and sends the full array with the fetched etag. That etag is safe to use because the delta is applied to the fresh copy. `respond` on an event where the user is not an attendee throws `.invalid`. A guest without permission to modify guests gets a 403, surfaced as `.forbidden`.

**Scopes.**

| Scope | update / respond | delete |
|---|---|---|
| `.thisInstance` | the instance id (each instance has its own id under `singleEvents`); `If-Match` with the instance etag. A recurrence change is `.unsupported(fields: [.recurrence])` before any request; a ref whose `seriesID` equals its `eventID` (a series master, see below) is `.invalid` | delete the instance id (a master ref is `.invalid`) |
| `.allInSeries` | the master id (`recurringEventId`); no `If-Match` for an instance ref (its etag is not the master's), the ref's version for a master ref (whose version is the master's). A patch with a time change is accepted only when `ref` is the series' first occurrence (the master's start equals `ref.originalStart`, within a second); otherwise `.unsupported(fields: [.timing])` before any PATCH | delete the master id |
| `.thisAndFollowing` | see "Splitting a series" below (`respond`: `.unsupported(fields: [.attendees])`) | truncate the master's `RRULE` as described below |

What a write returns for a series: `create` with a recurrence, `.allInSeries` updates, and `.thisAndFollowing` updates return a series master. Write results (only; reads use `singleEvents` and never return a master) map a resource with a non-empty `recurrence` with `seriesID` = its own id and `originalStart` = its start, so `EventRef(returned)` cannot be mistaken for a single event: a `.thisInstance` update, delete or respond on it throws `WriteError.invalid("this is a recurring series; use .allInSeries or read the occurrence first")` before any request (EventKit deletes only the first occurrence in this case, so the two would otherwise diverge). `.allInSeries` and `.thisAndFollowing` on a master ref keep working (a split at the master's start is the whole series). EventKit returns the first occurrence.

Why the `.allInSeries` timing rule: callers read expanded instances, so a `timing` in the patch is the instance's absolute date. Sent to the master it would move the whole series' start to that instance's date. Rebasing the change as a delta from the instance's `originalStart` is a possible follow-up.

**Splitting a series (`.thisAndFollowing`).**

1. Read and validate before the first write. Fetch the master (full resource) and, for update, the instance. The split point is the ref's `originalStart` (the slot in the series, not the possibly moved `start`). A series ref without `originalStart` is `.invalid`. If `originalStart` is the master's start, the split is the same as `.allInSeries` and is done that way. For update, everything that can fail without changing anything is done here: the instance fetch, the staleness check, the instance count for a `COUNT` rule, and building and validating the insert body. When `ref.version` differs from the fetched instance's etag, staleness is judged with `PatchMerge.conflicts(patch:current:)` against the fetched instance (the split rewrites the instance's fields from that copy); an overlap throws `.conflict(fields:)`.
2. Truncate the master's `RRULE`: remove `COUNT` and `UNTIL` and set `UNTIL` to just before `originalStart`, as a date for all-day series and as a UTC date-time (`originalStart` minus one second) for timed series, since Google requires `UNTIL` to have the same form as `DTSTART`. The PATCH carries the master's etag as `If-Match` (a 412 is `.conflict(fields: [.recurrence])`). EXDATE and RDATE values are split by date between the two series: the master keeps those before the split, the new series gets those at or after it (an unreadable value stays with the master). The truncation carries the caller's `sendUpdates` policy on both delete and update, and on update so does the insert, so guests may receive two messages (the cut and the new series); a caller who wants silence passes `.none`. Delete stops here.
3. Update only: insert a new series starting at the occurrence's original slot (`ref.originalStart`) with the master's length, in the master's own form (`dateTime` with its zone name, or all-day `date`s), unless the patch sets `timing`, in which case the patch's time applies. Starting at the slot rather than the instance's current start means a moved occurrence does not move every following occurrence. Its body is built from the full master resource with the patch applied, dropping output-only fields (`id`, `etag`, `iCalUID`, `htmlLink`, `created`, `updated`, `sequence`, `creator`, `organizer`, `recurringEventId`, `originalStartTime`, `conferenceData`, `hangoutLink`, `kind`, `status`) and resetting guest responses; unmodeled fields such as color, attachments and extended properties carry over. `supportsAttachments=true` is sent when the master has attachments. If the master had a Meet link, the new series requests a new one (`createRequest`, with `conferenceDataVersion=1`), so following instances do not lose their conference; an add-on (non-Meet) conference is dropped. Its time zone is the master's own `start.timeZone`, else the calendar's (Google requires one on recurring events; the name goes through the same "UTC" spelling as other zone writes). Its rule is the master's original rule, and for a `COUNT` rule the remaining count is `COUNT` minus the occurrences before the split, counted with `events.instances` (`showDeleted=true`, all pages, filtered client-side by `originalStartTime` because the listing is not bounded by `timeMax`), because an individually deleted occurrence still counts toward `COUNT`. The insert uses a client-chosen event id.
3a. If the truncating PATCH itself fails in a way that may have been applied (`mightHaveApplied`: a 5xx, a broken connection, a cancellation) and a new series was to be inserted (update), the connector restores the master's original `recurrence` (unconditionally, with the caller's `sendUpdates` policy so guests told about the cut hear about its reversal, in a task that ignores cancellation, as in step 4) and rethrows the original error, so the master is whole and a retry is safe (a retry after an unrestored truncation would have lost the EXDATE and RDATE values it moved). If the restore fails too: `.partial` naming both errors. A definite failure (a 412 as `.conflict([.recurrence])`, 400, 403, 404, auth or rate limit) changed nothing and is not restored. Delete keeps the plain error: the truncation is the operation and is idempotent.
4. If the insert fails, the connector first decides whether it may nevertheless have been applied (a 5xx, a broken connection, a cancellation, or a 409, which with our own id can only mean an earlier attempt of the same POST went through). In that case it looks the event up by id: found means success (return it, no rollback); a definite not-found (404 or already deleted) means the insert did not happen; any other lookup failure means the outcome is unknown, so the master is left alone and the call throws `.partial` (naming both errors). Only a definite failure or a definite not-found triggers the rollback: restore the master's original `recurrence` (unconditionally, in a task that ignores the caller's cancellation, so the series is not left cut off). If the rollback also fails, throw `.partial` (the series stays truncated, documented). Once the insert has succeeded it is never undone, even if its reply cannot be read (the copy stored under our id is read instead).

A patch that sets or clears `recurrence` on a split replaces the new series' rule with the patch's (or omits it) instead of listing the master's original rule, and skips the `COUNT` arithmetic. At the first instance, `.thisAndFollowing` is the same as `.allInSeries`. Modified or cancelled instances after the split point stay attached to the old series and are not carried over (documented; see item 3 of the Risks for what that means in practice). Truncation edits the raw `RRULE` string, so it works for rules outside the authorable subset.

`sendUpdates` maps `.all` to `all`, `.externalOnly` to `externalOnly` and `.none` to `none`. Our own writes show up as changes on the next poll and cause one extra refresh; no suppression is added.

Read-side additions: the mapper stamps `sourceID`; `version` (etag), `seriesID` (`recurringEventId`) and `originalStart` (`originalStartTime`) were already mapped.

## EventKit implementation

`EventKitSource` adopts `WritableCalendarSource` (`EventKitSource+Write.swift`) and gains `init(store: EKEventStore = EKEventStore())` so live tests share one store with the source under test. Reads now also fill `version` (`lastModifiedDate` as a fractional-epoch string), `sourceID` (`"eventkit"`), and, for events that recur or are detached occurrences of a series, `seriesID` (the `eventIdentifier`) and `originalStart` (`occurrenceDate`). Without `seriesID` a recurring occurrence would look like a standalone event and scopes would be ignored.

- **Access and validation order.** Writes validate their input (`draft.validate()`, `WriteValidation.requireWritable`, a patch's timing, recurrence and reminders) before `requireAccess()`, because a malformed recurrence rule crashes EventKit with an `NSException`. All writes then require full access (`.needsPermission` otherwise). `create` requires the calendar to exist and to allow modifications; `update` and `delete` refuse an event on a read-only calendar with `.forbidden`. An unknown calendar or event gives `.notFound`.
- **Strict validation.** Attendee changes, `respond`, a generated conference and `visibility` (EventKit has no such field) throw `.unsupported(fields:)` before anything is saved. A `NotifyPolicy` other than `.all` throws `.unsupported(fields: [.attendees])` when the event has other attendees; with no other attendees any policy is accepted and ignored. `respond` always throws `.unsupported(fields: [.attendees])`.
- **Fields.** Title, notes, location, availability (`EKEventAvailability`), timing (all-day through the reverse of `EventKitMapping.canonicalAllDay`: the calendar dates in the draft's zone are rebuilt as device-local floating dates, consistent with the read side), reminders as `EKAlarm` relative offsets (a nil draft list gives no alarms, because EventKit has no calendar-default alarms; this is a documented deviation from the draft's "nil = defaults"; a `.clear` on reminders removes all alarms), recurrence as `EKRecurrenceRule` (`EKRecurrenceDayOfWeek` carries ordinals; `EKRecurrenceEnd` carries count or until). A recurrence change (`.set` or `.clear`) with scope `.thisInstance` on a series occurrence is refused with `.unsupported(fields: [.recurrence])` before the permission check or any store access (a rule belongs to the whole series; Google refuses it too).
- **Locating an occurrence.** A non-recurring event is loaded by `eventIdentifier`. A recurring occurrence is found with a date-range predicate restricted to the ref's calendar, over a window of one year either way of the ref's `originalStart` (an occurrence's actual dates may be far from its slot; EventKit searches at most four years at a time), and matched on `eventIdentifier` and `occurrenceDate`, because occurrences of one series share an identifier. The found event's calendar id must equal the ref's, or it is `.notFound`.
- **Scopes.** `.thisInstance` saves or removes with `.thisEvent`; `.thisAndFollowing` with `.futureEvents`; `.allInSeries` loads the series' first occurrence (by `eventIdentifier`) and saves with `.futureEvents`. A series-wide time change is accepted only when `ref` is the series' first occurrence (the loaded event's `occurrenceDate` equals `ref.originalStart`, within a second); otherwise it throws `.unsupported(fields: [.timing])` before any save, because an occurrence's absolute date would otherwise move the whole series (a possible follow-up is rebasing on the delta).
- **Versions.** The event is reloaded and `refresh()` is called before a write; a `nil` event or a `false` from `refresh()` (expected to mean the event was deleted; to verify) is `.notFound`. A `lastModifiedDate` that differs from `ref.version` runs the shared `PatchMerge` against the reloaded event. `.allInSeries` on a series has no lock (`version` nil). If a save fails the event is discarded with `rollback()` so no pending edit stays on the shared store object (`refresh()` would keep the unsaved edits, since it only unloads properties that were not modified). Save errors surface as `.invalid(localizedDescription)`.
- **Empty patch.** The occurrence is still located (so a missing event is `.notFound`), then nothing is saved and the patch's `base`, or the mapped current event, is returned. This happens before the read-only check, so an empty patch on a read-only calendar returns instead of throwing `.forbidden`.
- Mapping code (`EventKitWriteMapping`) is pure functions over plain values where possible so it is unit-testable; only the save path touches `EKEventStore`.

## Testing

- **`CalendarCore` (pure, Linux CI).** `EventPatch` construction and `EventPatch(from:to:)` (each field, ignored fields, attendee upserts and removals, the self attendee, a cleared name, empty patch); `FieldUpdate` semantics; `EventEdit`; `EventTiming` and `AttendeeDraft` validation; `RecurrenceRule` RRULE round trips for the supported subset and rejection of each unsupported key; `until` rendering for timed and all-day; `PatchMerge` (no overlap, overlap, repeated stale results re-judged each time, the attempt limit and its clamp, cancellation, no-base patches, timing as a unit, attendee compare including a change someone else already made, `nil` and `""` notes, reminder order); `EventDraft(copying:for:)` for each capability combination; capability-invariant checks.
- **`WritableSourceConformance` (in `CalendarTestSupport`).** Checks a source and a scratch calendar: that `canWrite` is true and `canEditAttendees` matches `writableFields`, create then read back through `events(in:)`, update changes only the patched field (title changes, location and timing stay, the version moves), an empty patch changes nothing (including the version), an unwritable field (attendees) throws `.unsupported` and is not silently accepted, delete removes the event, and deleting it again throws `.notFound`. It cleans up the event it creates even when a check fails part way. It runs against `FakeWritableSource` (an in-memory, non-recurring implementation with a write counter and `simulateExternalEdit`) and, live and opt-in, against the real EventKit source. Conformance meta-tests feed it deliberately broken sources to prove each check fires. Stale-version behavior (merge versus conflict), unsupported fields, RSVP and validation are covered by `FakeWritableSource` tests that go through `PatchMerge`.
- **Google (fake transport).** Google is covered by request-shape tests with the fake transport; a stateful fake Google backend was judged too heavy, so `WritableSourceConformance` does not run against Google. For each operation and scope, assert method, path, query (`sendUpdates`, `conferenceDataVersion`, `supportsAttachments`), body JSON (only touched fields, `null` for clears, start and end together, `timeZone` on recurring), and headers (`If-Match`). Cover 412 then merge success, 412 then conflict, series-wide no-`If-Match`, the `.allInSeries` timing refusal, attendee delta and RSVP flows (fetch then patch the full array), `.thisAndFollowing` (a `COUNT` rule with a deleted occurrence before the split, timed and all-day `UNTIL`, EXDATE and RDATE splitting, a moved occurrence, the full-resource insert body, attachments and Meet re-request, staleness against the fetched instance, insert failure with rollback, a lost reply resolved by lookup, an unknown lookup outcome and a failed rollback as `.partial`), reads still handling 410 as a sync-token reset after the `send` refactor, and the 400/403/404/410/412/429 error mappings (including quota 403s as `.rateLimited`).
- **EventKit.** Pure tests for recurrence and timing mapping, capability and strictness rules, validation that runs before any permission check, and version strings. The save paths are covered by the gated live tests.
- **Live tests (manual, opt-in, never in CI).** EventKit: `TIMETUG_LIVE_EVENTKIT=1 swift test --package-path Packages/EventKitSource --filter eventKit` (every live test id starts with `eventKit`; a filter of `Live` matches nothing). It creates a scratch calendar in the local source, runs the spike (`SPIKE` lines), the conformance checks, recurring scopes, stale-version merge and conflict, and the refusal of unsupported fields, then removes the scratch calendar. Google: `TIMETUG_LIVE_GOOGLE=1` with the OAuth client id and secret in the environment, `swift test --package-path Packages/CalendarApple --filter googleWriteSmoke` (interactive sign-in). The events scope cannot create calendars, so it uses the primary calendar with clearly named ("TimeTug write smoke"), attendee-free events (nothing is emailed), deletes only events with that prefix, and cleans up on every path. It prints `LIVE ...` lines to record what the unit tests could only check against the fake, including a split of a series that has a modified and a cancelled occurrence after the split point (`LIVE split with later exceptions ...`) and a UTC-zoned recurring event (`LIVE UTC ...`). Neither touches existing events. They are listed in `docs/manual-tests/macos-checklist.md`.
- **Existing suites.** `CalendarBridgeTests`, `TimeTugCoreTests` and the app tests still pass; tests that compare events built by a source with hand-built events are updated for `sourceID`.

## Migration and compatibility

- All changes to shared types are additive with defaults: new `SourceCapabilities` fields, `CalendarEvent.sourceID` (nil by default). No persisted data changes (`Connection` and stores are untouched; `sourceID` on events is not persisted).
- Read-only behavior of every existing source is unchanged. A source that does not adopt `WritableCalendarSource` needs no code change.
- The OAuth scope `calendar.events` already covers Google writes, so existing accounts need no re-authorization.

## CI and docs

- No CI change: new `CalendarCore` code is covered by the existing `core-linux` job and the macOS jobs. Live tests are opt-in (`TIMETUG_LIVE_EVENTKIT=1`, `TIMETUG_LIVE_GOOGLE=1`) and never run in CI.
- Updated with this phase: `docs/calendar-connectors-api.md` (part 10 from Proposed to implemented, reconciled with the code, plus parts 3.2, 3.3, 5, 7 and 11), `docs/decisions/0012-calendar-connector-library.md` (Phase 3 addendum), `AGENTS.md` (library summary and a Gotcha) and `docs/manual-tests/macos-checklist.md` (write smoke item).

## Risks and items to verify first

The EventKit spike (`EventKitLiveSpikeTests.swift`, printing `SPIKE` lines) and the rest of the live EventKit tests have NOT been run yet, so items 1, 4 and 5 are open and the EventKit write code carries "Unverified" comments where it depends on them. Run:

```
TIMETUG_LIVE_EVENTKIT=1 swift test --package-path Packages/EventKitSource --filter eventKit 2>&1 | grep -E "SPIKE|LIVE|error|Issue"
```

`xctest` may lack calendar permission (the live tests then record "calendar access was denied"); if so, run them from a bundled context that holds the calendars entitlement. Record the results here and adjust `recurrenceScopes` or the code as the findings require.

1. **EventKit identifiers and scopes (OPEN).** Verify that occurrences of a series share `eventIdentifier`, that the predicate lookup finds the right occurrence (including a moved, detached one), and that saving the first occurrence with `.futureEvents` edits the whole series. Also that `.futureEvents` on a later occurrence keeps one series rather than splitting it. If `.allInSeries` cannot be done reliably, EventKit drops it from `recurrenceScopes` rather than approximating.
2. **Google PATCH behavior (UNVERIFIED until the live smoke test runs).** Verify with a live account: `If-Match` returns 412 on a stale etag for PATCH, an instance PATCH etag versus the master's, that `attendees` replacement preserves other guests' responses, and that recurring inserts require the zone on `start` and `end`. Also only checked against the fake transport: `supportsAttachments=true` on the insert when the master has attachments (the smoke test's series has none, so it needs an event with a Drive attachment added by hand), the `conferenceDataVersion=1` Meet re-request on the new series, and the COUNT arithmetic, which assumes `events.instances?showDeleted=true` includes occurrences removed by EXDATE.
3. **`.thisAndFollowing` on Google is two calls.** Rollback can fail, leaving a truncated series (`.partial`), an unknown insert outcome is also `.partial` (the master is left alone), and post-split exceptions are orphaned. Mitigation: documented behavior, the client-chosen id with lookup, and the rollback path; the scope can be removed from Google's `recurrenceScopes` without an API change if the smoke test shows it is unreliable. Known limitations of the split, stated plainly:
   - If the split occurrence was moved and the patch sets no `timing`, it snaps back to its original slot in the new series and its old exception may linger next to it.
   - An occurrence after the split point that was cancelled (deleted through the API) is not carried over: only EXDATE lines are, so it may reappear in the new series. Converting cancelled occurrences to EXDATEs is not done.
   - The carried EXDATE lines keep the old time of day, so if the patch changes the series' time they no longer match the new series' occurrences.
   - A modified exception after the split point stays with the old series. Whether Google still shows it beyond the old series' new `UNTIL` (a possible duplicate next to the new series' own occurrence) is UNVERIFIED. So is whether a cancelled occurrence really reappears. Both are to be checked by the live smoke test, which now modifies one occurrence and cancels another after the split point before splitting and prints what the calendar shows (`LIVE split with later exceptions ...`). The result is not recorded yet.
   - **Decided: notifications on update.** The truncation of the old series carries the caller's `NotifyPolicy`, like the insert of the new series (and like a restore of the master's rule after a failure, so guests told about the cut hear about its reversal). Rationale: external guests who use iTIP (invitations by email, not a Google account) would otherwise keep the old series running and also receive the new series, ending up with both. The cost is that guests may receive two messages, the truncation and the new series; a caller who wants silence passes `.none`.
4. **EventKit `lastModifiedDate` granularity (OPEN).** If two writes in the same second are indistinguishable, conflicts on EventKit are best-effort (documented); the Google path is exact. The spike prints whether the date changes on each save.
5. **EventKit reads (OPEN).** Verify `EKEvent.occurrenceDate` and `hasRecurrenceRules` give a reliable `seriesID` and `originalStart`, including for detached (moved) occurrences, that `EKObject.refresh()` returns false for a deleted event, and that negative `daysOfTheMonth` and `weekNumber` values are accepted by `EKRecurrenceRule`.
6. **Reminder defaults.** "Use the calendar's default reminders" and "no reminders" both read as an empty list, so a diff cannot tell them apart; patches that change reminders always set them explicitly, and copying an empty list means "defaults".
7. **Series-wide time changes.** An `.allInSeries` update that changes timing is refused (`.unsupported([.timing])`) unless `ref` is the series' first occurrence, on Google and EventKit. Rebasing the change as a delta from the instance's `originalStart` is a possible follow-up.
8. **Retry without a version.** Resolved: a versioned write whose retry finds no `version` on the fresh event fails closed with `.conflict` (see Conflict handling). Only an unversioned write retries unconditionally.
9. **Deletes ignore the version.** `delete` is last-writer-wins on both connectors (see Conflict handling).

## Process

Done: the spec was reviewed with the `deepseek-review` skill before the plan was written, the implementation followed the subagent-driven recipe of Phase 2.5, and this document and the API contract were reconciled with the code as built. Remaining (Task 16): the DeepSeek review of the full diff (with the spec and tests), verifying every Critical or Important finding against the repo, then the pull request; the EventKit spike (Risks) and the Google live smoke test are still to be run by the user.
