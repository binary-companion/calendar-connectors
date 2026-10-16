# calendar-connectors agent guide

## What this is

The portable calendar connector library used by TimeTug (Mac now, Windows next). It imports nothing from TimeTug and nothing Apple-only, has no package dependencies, and builds and passes its tests on Linux and Windows (x64 and ARM64). Platform adapters (Keychain, Network.framework loopback, EventKit, Windows Credential Locker) live in the apps, never here.

## Repositories

- `calendar-connectors` (this repository): the library. Released as `X.Y.Z` tags.
- `timetug-shared` (phase 2): TimeTugCore, CalendarBridge, the engine. Depends on this library by version range.
- `timetug`: the Mac app and the product website. Pins this library exactly in `Apps/macOS/project.yml`.
- `timetug-windows` (later): the WinUI app.

Dependency direction: `calendar-connectors` <- `timetug-shared` <- the apps; `timetug` also depends on this library directly.

## Boundaries

- Allowed imports: Foundation, FoundationNetworking, FoundationXML, Testing and this package's own modules. `scripts/ci/check-architecture.sh` enforces it.
- A new product needs a line in `Package.swift`, tests, and an entry in `docs/api.md`.
- Microsoft sign-in requests read-only scopes (`scripts/ci/check-repository-contracts.py` pins them); widening them is a deliberate, reviewed change.

## Backward compatibility

Owner rule (2026-10-16): within a major version, a release of this library must drop into an existing front end with no change to that front end; front ends are not released when the library is, and their releases stay owner-initiated. Additions are allowed. Removing, renaming or retyping anything, changing a value or a meaning, or modifying a persisted-format fixture needs the pull request label `semver:major` (which also makes the release a major version).

On every pull request the `compat` job of `ci.yml` compares with the baseline, the highest `X.Y.Z` tag reachable from the base branch (`scripts/ci/compat/compat.py`, an adapted copy of the one in `timetug-shared`; the repositories do not depend on each other, so change both on purpose; its tests are `scripts/ci/tests/test_compat.py`):

1. Swift public API: `swift package diagnose-api-breaking-changes <baseline>` for every library product in `Package.swift` (all are portable, so the job runs in the `swift:6.0` Linux container). A new product must be added to the job's list; `test_compat.py` fails until it is.
2. Persisted data: `Tests/PersistedFormats/<version>/` holds files that release's own code wrote, the way hosts keep them on users' disks: `connections.json` (`FileConnectionStore`, a connection of every kind with the `config` keys its kind writes) and `sync-state.json` (`FileSyncStateStore`, with the Google and Microsoft calendar-set keys and tokens, Microsoft's stored delta positions, and the CalDAV list signature and markers). They are strictly add-only: an existing file never changes, even by whitespace, and is never deleted; a new version directory is fine. The `PersistedFormatTests` of the test targets load them with the current code and check the meaning (each connection still opens through its kind, tokens and delta positions still parse, the same CalDAV calendars still give the stored signature). When a release changes what it writes, add `Tests/PersistedFormats/<that version>/` generated with that release's code (a scratch executable target in a throwaway checkout of the tag, built with `-Xswiftc -enable-testing` for the internal formats) and load it in the tests too.

Not covered: changed behaviour behind an unchanged API, and the provider responses the library parses (they live inline in the Swift tests, which run against the current code only). Running a release's own test suite against newer code is not done: those tests use `@testable` internals, so every internal refactor would break them.

With the `semver:major` label each check is skipped with a notice; the checks are also skipped on pushes and before the first release. Adding or removing a label re-runs CI.

## Verify

`swift test` (macOS), the Linux container job and the `windows-x64` and `windows-arm64` jobs in CI (Windows installs Swift with `scripts/ci/install-swift-windows.ps1`; its test step fails when `scripts/ci/count-swift-tests.py` finds too few tests, so raise its floor with the suite), `scripts/ci/check-architecture.sh`, `scripts/ci/check-repository-contracts.py`, the shell tests `scripts/ci/tests/test-check-architecture.sh`, `scripts/release/tests/test-next-version.sh` and `scripts/release/tests/test-release-version-step.sh` (set `PYTHON` to an interpreter that has PyYAML; CI installs `scripts/ci/requirements-test.txt` into `build/venv`), and the Python unit tests: `python3 -m unittest discover -s scripts/ci/tests -p 'test_*.py'` (they include `test_compat.py`). The `compat` job runs on pull requests only (see Backward compatibility).

## Release flow

Do not tag by hand. Merging a pull request to `main` runs `release.yml`: the highest `semver:*` label among the pull requests merged since the latest tag picks the bump (patch by default), the workflow tags and publishes. After a version is published, the `notify` job of `release.yml` dispatches `library-released` to `timetug` and to `timetug-shared` (one dispatch each; a failure to reach one does not stop the other, and fails the run), whose `Bump dependencies` workflows open a "Bump calendar-connectors to X.Y.Z" pull request (opened by `timetug-bot`, never merged by it). The loop is proven only once the first bump pull request has run; until then the owner checks that it opened, and moves the pin by hand if it did not. Breaking changes to `docs/api.md` need `semver:major`. The agent never merges or publishes; the owner does.

## Finding code

- If your agent has a semantic code-index skill (in Claude Code and Codex, the `index` skill), use it in this repo without asking first; this is standing permission. At the start of a task check its status. If this checkout is not indexed, or files changed a lot since the last index, run the (incremental) index. A new checkout or worktree is seeded from the index that the `Code index` workflow publishes from `main` (artifact `qindex`, fetched with `gh`), so the first run only embeds what differs from `main` and takes seconds. Without a compatible published index a full build takes many minutes: start it in the background and keep working with grep until it finishes. Then search the index before broad grepping or reading many files, and read the hit ranges rather than trusting snippets. Use grep for exact identifiers and strings. `.qindexignore` lists paths the index leaves out.
- Without such a skill, start from `Sources/` (one directory per product) and grep.
- Exact references ("who uses or calls this?", definitions, implementations) come from Serena (symbol tools over the sourcekit-lsp language server) when your agent has it; use it before grep. Swift; run `swift build --build-tests` first so sourcekit-lsp has an index.

## CI workflows

- `.github/workflows/index.yml` (`Code index`): on pushes to `main` builds the semantic code index incrementally from the previous `qindex` artifact against a throwaway Qdrant and publishes it as the `qindex` artifact (90 days) for agents' index skill. `scripts/dev/qindex.py` is a copy of the index skill's script (`~/.agents/skills/index/scripts/qindex.py`): keep the two identical (`diff` them) and keep the library versions in the workflow equal to the skill's. The export records the model and chunker version, and the skill refuses an index that does not match and builds locally instead.
