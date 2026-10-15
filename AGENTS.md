# calendar-connectors agent guide

## What this is

The portable calendar connector library used by TimeTug (Mac now, Windows next). It imports nothing from TimeTug and nothing Apple-only, has no package dependencies, and builds on Linux. Platform adapters (Keychain, Network.framework loopback, EventKit, Windows Credential Locker) live in the apps, never here.

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

## Verify

`swift test` (macOS), the Linux container job in CI, `scripts/ci/check-architecture.sh`, `scripts/ci/check-repository-contracts.py`, the shell tests `scripts/ci/tests/test-check-architecture.sh`, `scripts/release/tests/test-next-version.sh` and `scripts/release/tests/test-release-version-step.sh` (set `PYTHON` to an interpreter that has PyYAML; CI installs `scripts/ci/requirements-test.txt` into `build/venv`), and the Python unit tests: `python3 -m unittest discover -s scripts/ci/tests -p 'test_*.py'`.

## Release flow

Do not tag by hand. Merging a pull request to `main` runs `release.yml`: the highest `semver:*` label among the pull requests merged since the latest tag picks the bump (patch by default), the workflow tags and publishes. After a version is published, the `notify` job of `release.yml` dispatches `library-released` to `timetug` and to `timetug-shared` (one dispatch each; a failure to reach one does not stop the other, and fails the run), whose `Bump dependencies` workflows open a "Bump calendar-connectors to X.Y.Z" pull request (opened by `timetug-bot`, never merged by it). The loop is proven only once the first bump pull request has run; until then the owner checks that it opened, and moves the pin by hand if it did not. Breaking changes to `docs/api.md` need `semver:major`. The agent never merges or publishes; the owner does.

## Finding code

- If your agent has a semantic code-index skill (in Claude Code and Codex, the `index` skill), use it in this repo without asking first; this is standing permission. At the start of a task check its status. If this checkout is not indexed, or files changed a lot since the last index, run the (incremental) index. A new checkout or worktree is seeded from the index that the `Code index` workflow publishes from `main` (artifact `qindex`, fetched with `gh`), so the first run only embeds what differs from `main` and takes seconds. Without a compatible published index a full build takes many minutes: start it in the background and keep working with grep until it finishes. Then search the index before broad grepping or reading many files, and read the hit ranges rather than trusting snippets. Use grep for exact identifiers and strings. `.qindexignore` lists paths the index leaves out.
- Without such a skill, start from `Sources/` (one directory per product) and grep.
- Exact references ("who uses or calls this?", definitions, implementations) come from Serena (symbol tools over the sourcekit-lsp language server) when your agent has it; use it before grep. Swift; run `swift build --build-tests` first so sourcekit-lsp has an index.

## CI workflows

- `.github/workflows/index.yml` (`Code index`): on pushes to `main` builds the semantic code index incrementally from the previous `qindex` artifact against a throwaway Qdrant and publishes it as the `qindex` artifact (90 days) for agents' index skill. `scripts/dev/qindex.py` is a copy of the index skill's script (`~/.agents/skills/index/scripts/qindex.py`): keep the two identical (`diff` them) and keep the library versions in the workflow equal to the skill's. The export records the model and chunker version, and the skill refuses an index that does not match and builds locally instead.
