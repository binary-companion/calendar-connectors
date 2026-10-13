# calendar-connectors

A portable Swift library that reads and writes calendars through Google, Microsoft Graph, CalDAV (including iCloud) and iCalendar links, with one event model and no third-party dependencies. It builds on macOS and Linux and has been built on Windows ARM64.

## Products

`CalendarCore` (model, connector and source protocols, stores, recurrence), `CalendarOAuth` (PKCE and refresh), `GoogleCalendar`, `MicrosoftCalendar`, `ICalendar`, `CalDAVCalendar`, `ICalSubscription`, `CalendarTestSupport`.

## Use

```swift
.package(url: "https://github.com/binary-companion/calendar-connectors", from: "1.0.0")
```

The host supplies what is platform specific by implementing the library's seams: `CredentialStore` (secure storage), `SHA256Hashing`, and the OAuth redirect interaction (browser or sign-in sheet). The contract is [docs/api.md](docs/api.md). [TimeTug](https://github.com/binary-companion/timetug) is the reference host: its `CalendarApple` package implements the seams with Keychain, CryptoKit and a loopback listener.

## Releases

Every merge to `main` publishes a version tag (`X.Y.Z`) and a GitHub release. The `semver:major`, `semver:minor` or `semver:patch` labels of the pull requests merged since the latest tag choose the bump (the highest wins); patch is the default. Published tags are immutable. Dependents are not notified automatically yet: they move their pin to a new version by hand.

## License

MIT. See [LICENSE](LICENSE).
