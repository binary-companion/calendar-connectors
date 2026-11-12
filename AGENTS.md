# calendar-connectors agent guide

## What this is

The portable calendar connector library used by TimeTug (Mac now, Windows next). On `v2` it is a Rust Cargo workspace of eight crates; `v2` has no Swift left. The 1.x Swift package lives on `main` (and in the tags `1.0.7` to `1.2.1`) and `main` keeps releasing it until the Rust engine program moves the default branch.

The library imports nothing from TimeTug and nothing platform specific. Platform adapters (Keychain, Network.framework loopback, EventKit, Windows Credential Locker, the HTTP stack) live in the hosts and are reached through traits, never here (`docs/host-http-contract.md`). It builds and passes its tests on macOS, Linux, Windows x64 and Windows ARM64.

## The Rust workspace (`v2`)

- Layout: a Cargo workspace at the root, one crate per former Swift product in `crates/` (`calendar-core`, `calendar-oauth`, `calendar-ical`, `calendar-google`, `calendar-microsoft`, `calendar-caldav`, `calendar-ical-subscription`, `calendar-test-support`). The dependency table is `ALLOWED` in `scripts/ci/check-crate-architecture.py`. `Cargo.lock` is committed. A new crate needs a line in that table, tests and an entry in `docs/api.md`.
- Public surface: `docs/api.md` part 16; what a host owes for HTTP: `docs/host-http-contract.md`. The Swift API text in the earlier parts of `docs/api.md` is the history of the 1.x design.
- Toolchain: pinned in `rust-toolchain.toml` (1.99.1, with rustfmt, clippy and the targets of design section 6.4); raise it only in a reviewed pull request, together with `timetug-shared`. External crates are pinned exactly in `[workspace.dependencies]`; a crate not on the list of conventions section 3 needs a decision record, and `deny.toml` (the `allow` list), `EXTERNAL` in the architecture script and the pin must change together. `tokio` has only `rt`, `time`, `sync`, `macros` (`scripts/ci/check-tokio-features.py`). `cargo-deny` is pinned at 0.20.2 in the `rust-lint` job of `ci.yml`.
- Rules for the code: the conventions file `docs/superpowers/plans/2026-11-10-rust-port-conventions.md` in `timetug` (design section 6.3 holds the language rules). Open pull requests against `v2`; never push to `v2` or `main`.
- Microsoft sign-in requests read-only scopes (`scripts/ci/check-repository-contracts.py` pins the list in `crates/calendar-microsoft`); widening them is a deliberate, reviewed change.
- The program's design, conventions and tracker are in the `timetug` repository under `docs/superpowers/`; this repository's records are in `docs/rust-port/` (`README.md` names the last commit that still had the Swift package).

### Frozen data

- `fixtures/persisted-formats/<version>/`: files that release's own code wrote, the way hosts keep them on users' disks (see Backward compatibility). Never edited; moved here from `Tests/PersistedFormats/` with the Swift package's removal, byte for byte.
- `vectors/`: reference answers the Swift 1.2.0 code produced, replayed by the Rust tests (`vectors/README.md`). Nothing regenerates them any more (the exporter went with the Swift package); never edit a vector by hand to make Rust pass. A case the vectors do not cover and Rust gets wrong becomes a Rust test, not a vector edit.
- `docs/rust-port/swift-tests-baseline.txt` and `test-parity.md`: the Swift tests that existed at the baseline and the Rust test that replaced each. `check-test-parity.py --require-complete` (in `rust-lint`) fails when a ported Rust test is renamed or deleted.

## Repositories

- `calendar-connectors` (this repository): the library. Released as `X.Y.Z` tags from `main`; prereleases `2.0.0-<name>` from `v2`.
- `timetug-shared`: the engine (Rust on its `v2`), the C interface and the .NET package. Depends on this library by git tag.
- `timetug`: the Mac app and the product website.
- `timetug-windows` (later): the WinUI app.

Dependency direction: `calendar-connectors` <- `timetug-shared` <- the apps; `timetug` also depends on this library directly.

## Backward compatibility

Owner rule (2026-10-16): within a major version, a release of this library must drop into an existing front end with no change to that front end; front ends are not released when the library is, and their releases stay owner-initiated. Additions are allowed. Removing, renaming or retyping anything, changing a value or a meaning, or modifying a persisted-format fixture needs the pull request label `semver:major` (which also makes the release a major version).

On every pull request the `compat` job of `ci.yml` compares with the baseline, the highest `X.Y.Z` tag reachable from the base branch (`scripts/ci/compat/compat.py`, an adapted copy of the one in `timetug-shared`; the repositories do not depend on each other, so change both on purpose; its tests are `scripts/ci/tests/test_compat.py`). On `v2` that baseline is the last 1.x tag, a Swift release, so only the contracts that carry over from Swift to Rust are checked:

1. Persisted data: `fixtures/persisted-formats/<version>/` holds files that release's own code wrote: `connections.json` (a connection of every kind with the `config` keys its kind writes) and `sync-state.json` (the Google and Microsoft calendar-set keys and tokens, Microsoft's stored delta positions, and the CalDAV list signature and markers). They are strictly add-only: an existing file never changes, even by whitespace, and is never deleted; a new version directory is fine. The baseline tag keeps them under `Tests/PersistedFormats/`, so the job runs `compat.py frozen "$BASELINE" --strict fixtures/persisted-formats --moved Tests/PersistedFormats=fixtures/persisted-formats`: a baseline file must exist byte-identical at its old path or at the same place under the new one. The `persisted_format_tests.rs` files of the crates load them with the current code and check the meaning (each connection still opens through its kind, tokens and delta positions still parse, the same CalDAV calendars still give the stored signature). When a release changes what it writes, add `fixtures/persisted-formats/<that version>/` generated with that release's code and load it in the tests too.

The Swift public API diff (`swift package diagnose-api-breaking-changes`) is gone: there is no Swift API on `v2` to compare, and the Rust API has no earlier release. Once a `2.x` tag exists, a Rust API check (for example `cargo semver-checks`) belongs here; add it in the pull request that publishes the first one.

Not covered: changed behaviour behind an unchanged API, and the provider responses the library parses (they live inline in the tests, which run against the current code only).

With the `semver:major` label each check is skipped with a notice; the checks are also skipped on pushes and before the first release. Adding or removing a label re-runs CI.

## Verify

`cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check && cargo deny check && python3 scripts/ci/check-crate-architecture.py && python3 scripts/ci/check-tokio-features.py && python3 scripts/ci/check-test-parity.py && python3 scripts/ci/check-repository-contracts.py`, the shell tests `scripts/release/tests/test-next-version.sh` and `scripts/release/tests/test-release-version-step.sh`, and the Python unit tests `python3 -m unittest discover -s scripts/ci/tests -p 'test_*.py'` (they include `test_compat.py`). Set `PYTHON` to an interpreter that has PyYAML for the contracts check and the release step test; CI installs `scripts/ci/requirements-test.txt` into `build/venv`. Put the pinned toolchain first on `PATH` if `rustup` is not managing it.

In CI the jobs are:

- `macos`, `linux`, `windows-x64`, `windows-arm64`: `cargo test --workspace --locked` and a release build on that platform.
- `architecture`: crate architecture, tokio features, repository contracts, the release-script tests and the Python unit tests.
- `rust-lint`: rustfmt, clippy `-D warnings`, `cargo deny check`, the test parity table.
- `rust-targets`: release builds for the other Apple targets (macOS x86_64, iOS device and simulator).
- `compat`: pull requests only (see Backward compatibility).
- `rust-macos`, `rust-linux`, `rust-windows-x64`, `rust-windows-arm64`: aliases that pass when the platform job of the same platform passed. The ruleset "Protect v2" still lists these names as required checks (they were the Rust-only jobs while Swift and Rust ran side by side). Delete the aliases from `ci.yml` and from `REQUIRED_CI_JOBS` in `check-repository-contracts.py` once the ruleset lists only the plain names.

The job ids are the status-check names the rulesets match: renaming a job makes its required check unreported and the pull request unmergeable, and `check-repository-contracts.py` fails when one goes missing. The weekly `vectors-drift.yml` replays the vectors on the current macOS and Linux runner images (see its header for when it actually runs).

## Release flow

Do not tag by hand. Merging a pull request to `main` runs `release.yml`: the highest `semver:*` label among the pull requests merged since the latest tag picks the bump (patch by default), the workflow tags and publishes. After a version is published, the `notify` job of `release.yml` dispatches `library-released` to `timetug` and to `timetug-shared` (one dispatch each; a failure to reach one does not stop the other, and fails the run), whose `Bump dependencies` workflows open a "Bump calendar-connectors to X.Y.Z" pull request (opened by `timetug-bot`, never merged by it). Merging to `v2` releases nothing.

Prereleases of the Rust line come from `v2` by hand: dispatch `Release` on the `v2` ref with the `prerelease` input, for example `gh workflow run Release --repo binary-companion/calendar-connectors --ref v2 -f prerelease=alpha.1`. That tags the dispatched `v2` commit `2.0.0-alpha.1` and publishes a GitHub prerelease. There is no package and no `library-released` dispatch; the dependents move to it by hand (`timetug-shared`'s `bump-cargo-dependency.py 2.0.0-alpha.1`). The `prerelease` job runs on the `v2` ref only and fails on any other; `scripts/release/next-version.sh --prerelease <name>` builds and validates the version, and refuses a tag name that already points at another commit. Prerelease tags never count as the compatibility baseline or as the latest release (only strict `X.Y.Z` tags do).

Breaking changes to the public API need `semver:major`. The agent never merges or publishes; the owner does.

## Finding code

- If your agent has a semantic code-index skill (in Claude Code and Codex, the `index` skill), use it in this repo without asking first; this is standing permission. At the start of a task check its status. If this checkout is not indexed, or files changed a lot since the last index, run the (incremental) index. A new checkout or worktree is seeded from the index that the `Code index` workflow publishes from `main` (artifact `qindex`, fetched with `gh`), so the first run only embeds what differs from `main` and takes seconds. Without a compatible published index a full build takes many minutes: start it in the background and keep working with grep until it finishes. Then search the index before broad grepping or reading many files, and read the hit ranges rather than trusting snippets. Use grep for exact identifiers and strings. `.qindexignore` lists paths the index leaves out.
- Without such a skill, start from `crates/` (one directory per crate) and grep.
- Exact references ("who uses or calls this?", definitions, implementations) come from Serena (symbol tools over rust-analyzer) when your agent has it; use it before grep. Run `cargo check --workspace --all-targets` first so the language server has an index.

## CI workflows

- `.github/workflows/index.yml` (`Code index`): on pushes to `main` builds the semantic code index incrementally from the previous `qindex` artifact against a throwaway Qdrant and publishes it as the `qindex` artifact (90 days) for agents' index skill. `scripts/dev/qindex.py` is a copy of the index skill's script (`~/.agents/skills/index/scripts/qindex.py`): keep the two identical (`diff` them) and keep the library versions in the workflow equal to the skill's. The export records the model and chunker version, and the skill refuses an index that does not match and builds locally instead.
