# calendar-connectors

A portable library that reads and writes calendars through Google, Microsoft Graph, CalDAV (including iCloud) and iCalendar links, with one event model. On the `v2` branch it is written in Rust: eight crates in a Cargo workspace, with no Swift left. The 1.x line is a Swift package on `main`. The Rust crates build and pass their tests on macOS, Linux, Windows x64 and Windows ARM64 (all four run in CI).

## Crates

In `crates/`: `calendar-core` (model, connector and source traits, stores, recurrence), `calendar-oauth` (PKCE and refresh), `calendar-ical`, `calendar-google`, `calendar-microsoft`, `calendar-caldav`, `calendar-ical-subscription`, `calendar-test-support`. The public Rust API reference, with runnable examples, is [docs/api.md](docs/api.md); the host supplies HTTP through the `HttpTransport` trait ([docs/host-http-contract.md](docs/host-http-contract.md)). Every test of the Swift package has a Rust counterpart ([docs/rust-port/test-parity.md](docs/rust-port/test-parity.md)); the files that 1.x wrote on users' disks ([fixtures/persisted-formats](fixtures/persisted-formats)) and the golden vectors from the Swift code ([vectors](vectors/README.md)) are replayed by the Rust tests.

## Use (Rust, 2.x on `v2`)

The crates are not published to a registry. Hosts take them from a git tag: `2.0.0-<prerelease>` prereleases are cut from `v2` by hand (see Releases); the latest is `2.0.0-alpha.1`. Install lines and runnable examples are in [docs/api.md](docs/api.md). Build and test with `cargo test --workspace` (the toolchain is pinned in `rust-toolchain.toml`). A host implements `HttpTransport`, `CredentialStore`, `SyncStateStore` and the OAuth interaction, then builds a connector kind such as `GoogleConnectorKind::new(...)`.

## Use (Swift package, 1.x on `main`)

```swift
.package(url: "https://github.com/binary-companion/calendar-connectors", from: "1.0.0")
```

[TimeTug](https://github.com/binary-companion/timetug) is the reference host of the 1.x package: its `CalendarApple` package implements the seams with Keychain, CryptoKit and a loopback listener. The 1.x behaviour contract is [docs/behaviour-contract-1x.md](docs/behaviour-contract-1x.md).

## Releases

`main` (Swift, 1.x): every merge publishes a version tag (`X.Y.Z`) and a GitHub release. The `semver:major`, `semver:minor` or `semver:patch` labels of the pull requests merged since the latest tag choose the bump (the highest wins); patch is the default. Published tags are immutable. After each release the `notify` job dispatches `library-released` to [TimeTug](https://github.com/binary-companion/timetug) and to `timetug-shared`, whose `Bump dependencies` workflows open a "Bump calendar-connectors to X.Y.Z" pull request (opened by `timetug-bot`, never merged by it).

`v2` (Rust): merging releases nothing. The owner cuts a prerelease by dispatching `Release` on the `v2` ref with the `prerelease` input: `gh workflow run Release --repo binary-companion/calendar-connectors --ref v2 -f prerelease=alpha.1` tags the `v2` commit `2.0.0-alpha.1` and publishes a GitHub prerelease (no package, no dependents notified).

## License

MIT. See [LICENSE](LICENSE).
