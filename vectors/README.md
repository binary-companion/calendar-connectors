# Golden vectors

Reference answers of the Swift library at baseline tag `1.2.0`, replayed by the Rust tests of the port. Format and rules:
conventions section 9 (`docs/superpowers/plans/2026-11-10-rust-port-conventions.md` in the `timetug` repository).
A vector is never edited by hand and never regenerated to make Rust pass.

| File | Cases | Swift entry points |
|---|---|---|
| `zones/offsets.json` | 2,400 | `TimeZone.secondsFromGMT(for:)` |
| `zones/wall-clock.json` | 600 | `WallClock.date(_:hour:minute:second:in:)` |
| `zones/all-day.json` | 500 | `AllDay.startOfDay`, `date(of:in:)`, `canonical`, `dates`, `CalendarDate.adding(days:)` |
| `zones/known.json` | 483 | `TimeZone(identifier:)` |
| `zones/windows-names.json` | 478 | `WindowsTimeZones.timeZone(for:)`, `windowsName(for:)` |
| `recurrence/rule-instances.json` | 2,000 | `RecurrenceRule.instances(...)` |
| `recurrence/set-occurrences.json` | 300 | `RecurrenceSet.occurrences(...)`, `ruleInstanceCount(...)` |
| `recurrence/rule-parse.json` | 400 | `RecurrenceRule(rrule:in:)`, `rruleString(allDay:in:)` |
| `ical/content-lines.json` | 300 | `ICalParser.unfold`, `parseLine`, `ICalSerializer.contentLine` |
| `ical/timezones.json` | 150 | `TimeZoneResolver.zone(for:)`, `date(_:floating:)`, `displayZone` |
| `microsoft/graph-time.json` | 300 | `GraphTime.*` |
| `core/conference.json` | 400 | `ConferenceDetector.*`, `JoinURLPolicy.isAllowed` |
| `core/url-accept.json` | 300 | `URL(string:)` |
| `oauth/pkce.json` | 50 | `PKCE.challenge(for:)` |

The `input` and `expected` shapes of each file are written as a comment at the top of its exporter, `Sources/VectorExporter/<Area>.swift`.
The cases are synthetic: example hosts and addresses, no real calendar content.

## Regenerating

```
scripts/dev/export-vectors.sh
git diff vectors/
```

The script builds the `VectorExporter` executable target in release mode with `-Xswiftc -enable-testing` (it reads internal entry points
through `@testable import`) and writes every file. The exporter is deterministic: hand-written cases first, then cases from a fixed-seed
linear congruential generator (`LCG.swift`, one constant seed per area). Two runs give identical bytes.

## The `vectors-current` job

CI (`vectors-current` in `.github/workflows/ci.yml`, `macos-26`) runs the script and `git diff --exit-code vectors/`. It fails when a
Swift change alters an answer, which is the point: either the change was meant (then it is a behaviour change the Rust port must receive,
see `docs/rust-port/parity-ledger.md`) or it is a bug.

The zone-dependent cases stop at 2026-11-01 (`zoneHorizon` in `VectorFile.swift`): the zone database's rules for later dates are a
forecast and differ between operating system releases (the first run of the job found that the runner's macOS 26 and the macOS 27 that
wrote the files disagree on America/Havana's 2027 transitions). Each zone and each generated case draws from its own generator, so one
zone's difference changes only that zone's cases. The recurrence cases reach years ahead by design but draw only on zones whose rules are
stable (Havana appears in hand-placed cases that end before the horizon).

The job also depends on the runner's zone database and Foundation. `zones/known.json` lists every identifier the system knows,
`zones/offsets.json` and `zones/wall-clock.json` ask the system for offsets, `ical/timezones.json` matches `VTIMEZONE`s against the system's
zones, and `core/url-accept.json` records Foundation's URL parsing. If a later runner image changes a zone's rules, adds or removes an
identifier, or parses URLs differently, the job fails. The fix is to pin the cases to past dates (or to remove the identifier from the
generated list), never to accept the new output silently, because the Rust port must reproduce the answers the shipped app gave.
