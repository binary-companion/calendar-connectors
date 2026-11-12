# Golden vectors

Reference answers of the Swift library at baseline tag `1.2.0`, replayed by the Rust tests of the port. The Swift exporter that wrote them was removed from `v2` with the Swift package; it is at the last dual-language commit named in `docs/rust-port/README.md` (the `Sources/VectorExporter/` paths below refer to it). Format and rules:
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

## Frozen

Nothing regenerates the vectors on `v2`: `scripts/dev/export-vectors.sh` and the `vectors-current` job went with the Swift package, and a vector is never edited by hand. They are what the shipped Swift app answered, so a Rust answer that differs is a Rust bug (or, rarely, a documented difference that gets its own Rust test and an entry in `docs/rust-port/`), never a reason to change a file here. To regenerate for a reason that the owner has approved, check out the last dual-language commit, run `scripts/dev/export-vectors.sh` there, and bring the result over in a reviewed pull request.

The zone-dependent cases stop at 2026-11-01 (`zoneHorizon` in `VectorFile.swift`): the zone database's rules for later dates are a forecast and differ between operating system releases (the first run of the job found that the runner's macOS 26 and the macOS 27 that wrote the files disagree on America/Havana's 2027 transitions). Africa/Casablanca stops earlier still (`horizon(of:)`, 2026-06-01): the runner's database lacks the change of Morocco's rules on 2026-09-20 that the newer one has. The recurrence cases reach years ahead by design but draw only on zones whose rules are stable.

The Rust crates read the system zone database on macOS and Linux and a bundled one on Windows (`jiff`, see `Cargo.toml`), so `zones/known.json`, `zones/offsets.json`, `zones/wall-clock.json` and `ical/timezones.json` depend on the runner's database. If a later runner image changes a zone's rules or adds or removes an identifier, the Rust tests fail on that platform. The fix is to pin the cases to past dates in the Rust test (never to edit the vector), because the Rust port must reproduce the answers the shipped app gave.

## When to re-check the vectors

`.github/workflows/vectors-drift.yml` replays them every Monday on macOS and Linux (and on `workflow_dispatch`) and opens an issue when a run fails; the schedule only fires from the default branch, so until `v2` is the default branch, dispatch it by hand from `v2`. Also re-check by hand when any of these changes:

- [ ] The `jiff` version or its zone database features (its zone database must still give the answers in the vectors).
- [ ] The runner images of CI (a failing zone case on one platform only points at its database).

Investigate every failure in `zones/*`, `ical/timezones`; never accept one blindly.
