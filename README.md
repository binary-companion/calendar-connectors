# calendar-connectors

A portable library that reads and writes calendars through Google, Microsoft Graph, CalDAV (including iCloud) and iCalendar links, with one event model. On the `v2` branch it is written in Rust (eight crates in a Cargo workspace); on `main` it is a Swift package with no third-party dependencies. Both build and passes its tests on macOS, Linux, Windows x64 and Windows ARM64 (all four run in CI).

## Products

On `v2` (Rust, crates in `crates/`): `calendar-core`, `calendar-oauth`, `calendar-ical`, `calendar-google`, `calendar-microsoft`, `calendar-caldav`, `calendar-ical-subscription`, `calendar-test-support`. The public API is in [docs/api.md](docs/api.md) part 16; the host supplies HTTP through the `HttpTransport` trait ([docs/host-http-contract.md](docs/host-http-contract.md)), and every Swift test has a Rust counterpart ([docs/rust-port/test-parity.md](docs/rust-port/test-parity.md)).

On `main` (Swift package): `CalendarCore` (model, connector and source protocols, stores, recurrence), `CalendarOAuth` (PKCE and refresh), `GoogleCalendar`, `MicrosoftCalendar`, `ICalendar`, `CalDAVCalendar`, `ICalSubscription`, `CalendarTestSupport`.

## Use (Swift package, 1.x)

```swift
.package(url: "https://github.com/binary-companion/calendar-connectors", from: "1.0.0")
```

The host supplies what is platform specific by implementing the library's seams: `CredentialStore` (secure storage), `SHA256Hashing`, and the OAuth redirect interaction (browser or sign-in sheet). The contract is [docs/api.md](docs/api.md). [TimeTug](https://github.com/binary-companion/timetug) is the reference host: its `CalendarApple` package implements the seams with Keychain, CryptoKit and a loopback listener.

## Use (Rust, 2.x on `v2`)

The crates are not published yet and `v2` has no release; the first `2.0.0-alpha` comes from the engine phase of the Rust engine program. Build and test with `cargo test --workspace` (the toolchain is pinned in `rust-toolchain.toml`). A host implements `HttpTransport`, `CredentialStore`, `SyncStateStore` and the OAuth interaction, then builds a connector kind such as `GoogleConnectorKind::new(...)`.

## Releases (Swift, `main`)

Every merge to `main` publishes a version tag (`X.Y.Z`) and a GitHub release. The `semver:major`, `semver:minor` or `semver:patch` labels of the pull requests merged since the latest tag choose the bump (the highest wins); patch is the default. Published tags are immutable. After each release the `notify` job dispatches `library-released` to [TimeTug](https://github.com/binary-companion/timetug) and to `timetug-shared`, whose `Bump dependencies` workflows open a "Bump calendar-connectors to X.Y.Z" pull request (opened by `timetug-bot`, never merged by it). The loop is proven only after the first bump pull request has run.

## License

MIT. See [LICENSE](LICENSE).
