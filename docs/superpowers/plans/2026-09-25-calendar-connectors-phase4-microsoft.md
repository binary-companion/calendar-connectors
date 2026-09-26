# Calendar Connectors Phase 4 (Microsoft / Outlook) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let TimeTug users add a Microsoft (Outlook / Microsoft 365) account in Settings > Accounts, with the same read, change-detection, write and series support as Google.

**Architecture:** A new `MicrosoftCalendar` module in the portable `Packages/CalendarConnectors` library talks to Microsoft Graph (see the spec). It is already written and tested (Part A below). The macOS app registers `MicrosoftConnectorKind` the way it registers Google: the Entra client id is injected into Info.plist at build time from a git-ignored xcconfig, and CI injects it from a repository secret. A user-run live smoke test settles the behaviours a fake transport cannot.

**Tech Stack:** Swift 6 (library) / Swift 5 mode (app, CalendarApple), Swift Testing (library) and XCTest (app), XcodeGen, bash CI helpers, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-09-25-calendar-connectors-phase4-microsoft-design.md`

## Global Constraints

- The library (`Packages/CalendarConnectors`) stays dependency-free and portable: no Apple frameworks, no secret. `MicrosoftCalendar` depends only on `CalendarCore` and `CalendarOAuth`.
- Microsoft is a public client: PKCE, **no client secret**. Redirect URI registered in Entra: `http://localhost` (the kind rewrites the loopback listener's `127.0.0.1` host to `localhost`).
- Authority `common` (work/school and personal accounts). Scopes: `offline_access`, `User.Read`, `MailboxSettings.Read`, `Calendars.ReadWrite`, `Calendars.ReadWrite.Shared`.
- The client id is never printed, logged or committed. `~/.config/timetug/microsoft-oauth.xcconfig` (defines `MICROSOFT_OAUTH_CLIENT_ID`) and `Apps/macOS/Config/MicrosoftOAuth.xcconfig` are git-ignored.
- Without a client id Microsoft is shown as "Unavailable" in Settings > Accounts and everything else works (same as Google).
- `xcodegen generate` rewrites `Apps/macOS/Sources/Info.plist` and `Apps/macOS/Widgets/Info.plist`: restore them with `git checkout` before committing.
- Do not merge the PR; the user merges (squash) when ready. Do not set repository secrets; the user does.
- App tests: `xcodebuild -project Apps/macOS/TimeTug.xcodeproj -scheme TimeTug test` after `xcodegen generate` (run from `Apps/macOS`). Library tests: `swift test --package-path Packages/CalendarConnectors`.

---

## File map

Already done (Part A, commits `3cf9bbd` and `57ce7d0` on this branch):

| File | Responsibility |
|---|---|
| `Packages/CalendarConnectors/Sources/MicrosoftCalendar/WindowsTimeZones.swift` | Windows <-> IANA zone names |
| `.../GraphTime.swift` | Graph date-time text <-> `Date` / `CalendarDate` |
| `.../GraphAPIClient.swift` | HTTP client: token refresh, retries, paging, error mapping |
| `.../GraphDTOs.swift` | Codable Graph shapes |
| `.../GraphEventMapper.swift`, `GraphRecurrenceMapper.swift` | Graph event / recurrence <-> `CalendarEvent` / `RecurrenceRule` |
| `.../GraphWriteMapper.swift` | `EventDraft` / `EventPatch` -> Graph JSON |
| `.../MicrosoftCalendarSource.swift` (+ `+Sync`, `+Series`, `+Write`, `+Split`) | The source: reads, delta change detection, `SeriesSource`, `WritableCalendarSource`, `.thisAndFollowing` split |
| `.../MicrosoftConnectorKind.swift` | `MicrosoftOAuthConfig`, `MicrosoftConnectorKind` (sign-in) |
| `Packages/CalendarConnectors/Tests/MicrosoftCalendarTests/*` | 116 tests incl. a stateful `FakeGraph` running `WritableSourceConformance` |
| `CalendarCore/Model.swift`, `SourceTypes.swift`, `CalendarOAuth/OAuth.swift` | `CalendarService.microsoft`; `controlsNotifications` doc; `interaction_required` -> `authExpired` |

To do (Parts B-D):

| File | Responsibility |
|---|---|
| Create `Apps/macOS/Sources/MicrosoftOAuthSettings.swift` | Reads the client id from Info.plist |
| Modify `Apps/macOS/Sources/AppConnectors.swift` | Registers `MicrosoftConnectorKind` |
| Modify `Apps/macOS/Sources/AppCoordinator.swift` | Loads the settings, reports Microsoft as unconfigured when absent |
| Modify `Apps/macOS/Sources/AccountsPane.swift` | Removes the "Soon" Microsoft placeholder tile |
| Modify `Apps/macOS/Sources/ProviderIcon.swift`, `SettingsSearch.swift` | Microsoft mark; search keywords |
| Modify `Apps/macOS/project.yml`, `Config/Signing.xcconfig`, `scripts/dev/link-signing.sh`, `.gitignore` | Build-time injection |
| Create `Apps/macOS/Tests/MicrosoftOAuthSettingsTests.swift`; modify `AppConnectorsTests.swift`, `ProviderIconTests.swift` | App tests |
| Create `scripts/ci/microsoft-oauth-config.sh`, `scripts/ci/tests/test-microsoft-oauth-config.sh` | CI helper |
| Modify `scripts/ci/build-release.sh`, `.github/workflows/beta.yml`, `release.yml`, `scripts/ci/tests/test-workflows.sh`, `docs/release.md` | CI wiring |
| Modify `Packages/CalendarApple/Package.swift`; create `Tests/CalendarAppleTests/MicrosoftLiveWriteSmokeTests.swift` | Opt-in live smoke test |
| Modify `docs/calendar-connectors-api.md`, `AGENTS.md`, the spec | Docs |

---

## Part B: App wiring

### Task 1: Register Microsoft in the app

**Files:**
- Create: `Apps/macOS/Sources/MicrosoftOAuthSettings.swift`
- Create: `Apps/macOS/Tests/MicrosoftOAuthSettingsTests.swift`
- Modify: `Apps/macOS/Sources/AppConnectors.swift`, `Apps/macOS/Tests/AppConnectorsTests.swift`
- Modify: `Apps/macOS/Sources/AppCoordinator.swift:51-54`
- Modify: `Apps/macOS/project.yml` (dependencies + Info.plist key)

**Interfaces:**
- Consumes: `MicrosoftOAuthConfig(clientID: String, redirectHost: String? = "localhost")`, `MicrosoftConnectorKind(config:hasher:)` from the `MicrosoftCalendar` product; `CryptoKitSHA256()` from `CalendarApple`.
- Produces: `MicrosoftOAuthSettings.config(from: [String: Any]?) -> MicrosoftOAuthConfig?`, `MicrosoftOAuthSettings.config(bundle:) -> MicrosoftOAuthConfig?`, `AppConnectors.makeRegistry(google: GoogleOAuthConfig?, microsoft: MicrosoftOAuthConfig?, eventKit: EventKitSource) -> ConnectorRegistry`.

- [ ] **Step 1: Write the failing tests**

`Apps/macOS/Tests/MicrosoftOAuthSettingsTests.swift`:

```swift
import XCTest
@testable import TimeTug

final class MicrosoftOAuthSettingsTests: XCTestCase {
    func testReadsTheClientID() {
        let config = MicrosoftOAuthSettings.config(from: ["TimeTugMicrosoftClientID": " 11111111-2222-3333-4444-555555555555 "])
        XCTAssertEqual(config?.clientID, "11111111-2222-3333-4444-555555555555")
    }

    func testMissingEmptyOrUnexpandedValuesDisableMicrosoft() {
        XCTAssertNil(MicrosoftOAuthSettings.config(from: nil))
        XCTAssertNil(MicrosoftOAuthSettings.config(from: [:]))
        XCTAssertNil(MicrosoftOAuthSettings.config(from: ["TimeTugMicrosoftClientID": ""]))
        XCTAssertNil(MicrosoftOAuthSettings.config(from: ["TimeTugMicrosoftClientID": "$(MICROSOFT_OAUTH_CLIENT_ID)"]))
    }
}
```

Replace `Apps/macOS/Tests/AppConnectorsTests.swift` with:

```swift
import CalendarCore
import EventKitSource
import GoogleCalendar
import MicrosoftCalendar
import XCTest
@testable import TimeTug

final class AppConnectorsTests: XCTestCase {
    func testGoogleIsRegisteredOnlyWhenConfigured() {
        let none = AppConnectors.makeRegistry(google: nil, microsoft: nil, eventKit: EventKitSource())
        XCTAssertNil(none.kind(id: "google"))
        XCTAssertNotNil(none.kind(id: "eventkit"))
        let some = AppConnectors.makeRegistry(
            google: GoogleOAuthConfig(clientID: "i", clientSecret: "s"), microsoft: nil, eventKit: EventKitSource())
        XCTAssertNotNil(some.kind(id: "google"))
    }

    func testMicrosoftIsRegisteredOnlyWhenConfigured() {
        let none = AppConnectors.makeRegistry(google: nil, microsoft: nil, eventKit: EventKitSource())
        XCTAssertNil(none.kind(id: "microsoft"))
        let some = AppConnectors.makeRegistry(google: nil, microsoft: MicrosoftOAuthConfig(clientID: "i"), eventKit: EventKitSource())
        XCTAssertNotNil(some.kind(id: "microsoft"))
    }
}
```

- [ ] **Step 2: Add the dependency and the Info.plist key in `Apps/macOS/project.yml`**

In the `TimeTug` target's `dependencies`, after the `GoogleCalendar` entry add:

```yaml
      - package: CalendarConnectors
        product: MicrosoftCalendar
```

In the `TimeTugTests` target's `dependencies`, after its `GoogleCalendar` entry add the same two lines. In the `TimeTug` target's `info.properties`, after `TimeTugGoogleClientSecret: $(GOOGLE_OAUTH_CLIENT_SECRET)` add:

```yaml
        TimeTugMicrosoftClientID: $(MICROSOFT_OAUTH_CLIENT_ID)
```

- [ ] **Step 3: Write `MicrosoftOAuthSettings.swift`**

```swift
import Foundation
import MicrosoftCalendar

/// The Microsoft Entra app registration's client id (a public client, so no secret), injected at build time from the
/// git-ignored MicrosoftOAuth.xcconfig into Info.plist. Without it (CI PR builds, fresh checkouts) Microsoft is not
/// offered and everything else works.
enum MicrosoftOAuthSettings {
    static func config(from info: [String: Any]?) -> MicrosoftOAuthConfig? {
        guard let raw = (info?["TimeTugMicrosoftClientID"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines),
              !raw.isEmpty, !raw.hasPrefix("$(") else { return nil }
        return MicrosoftOAuthConfig(clientID: raw)
    }

    static func config(bundle: Bundle = .main) -> MicrosoftOAuthConfig? { config(from: bundle.infoDictionary) }
}
```

- [ ] **Step 4: Register the kind** — replace `Apps/macOS/Sources/AppConnectors.swift`:

```swift
import CalendarApple
import CalendarCore
import EventKitSource
import GoogleCalendar
import MicrosoftCalendar

enum AppConnectors {
    static func makeRegistry(google: GoogleOAuthConfig?, microsoft: MicrosoftOAuthConfig?, eventKit: EventKitSource) -> ConnectorRegistry {
        var registry = ConnectorRegistry()
        registry.register(EventKitConnectorKind(source: eventKit))
        if let google { registry.register(GoogleConnectorKind(config: google, hasher: CryptoKitSHA256())) }
        if let microsoft { registry.register(MicrosoftConnectorKind(config: microsoft, hasher: CryptoKitSHA256())) }
        return registry
    }
}
```

- [ ] **Step 5: Update `AppCoordinator.swift`** — replace the three lines at 51-54:

```swift
    private lazy var googleOAuthConfig = GoogleOAuthSettings.config()
    private lazy var microsoftOAuthConfig = MicrosoftOAuthSettings.config()
    private lazy var registry = AppConnectors.makeRegistry(google: googleOAuthConfig, microsoft: microsoftOAuthConfig, eventKit: eventKit)
    /// Kinds TimeTug supports in code but this build couldn't register — see `AccountsController.unconfiguredKindIDs`.
    private lazy var unconfiguredKindIDs: [String] =
        (googleOAuthConfig == nil ? ["google"] : []) + (microsoftOAuthConfig == nil ? ["microsoft"] : [])
```

- [ ] **Step 6: Regenerate, build and run the app tests**

Run: `cd Apps/macOS && xcodegen generate && cd ../.. && git checkout Apps/macOS/Sources/Info.plist Apps/macOS/Widgets/Info.plist && xcodebuild -project Apps/macOS/TimeTug.xcodeproj -scheme TimeTug test -only-testing:TimeTugTests/MicrosoftOAuthSettingsTests -only-testing:TimeTugTests/AppConnectorsTests 2>&1 | tail -15`
Expected: `** TEST SUCCEEDED **`. (If Xcode cannot see the new library files, delete only this worktree's `~/Library/Developer/Xcode/DerivedData/TimeTug-<hash>`.)

- [ ] **Step 7: Commit**

```bash
git add Apps/macOS/Sources/MicrosoftOAuthSettings.swift Apps/macOS/Sources/AppConnectors.swift Apps/macOS/Sources/AppCoordinator.swift Apps/macOS/Tests/MicrosoftOAuthSettingsTests.swift Apps/macOS/Tests/AppConnectorsTests.swift Apps/macOS/project.yml Apps/macOS/TimeTug.xcodeproj
git commit -m "App: register the Microsoft connector when a client id is configured"
```

### Task 2: Build-time client id and the Accounts UI

**Files:**
- Modify: `Apps/macOS/Config/Signing.xcconfig`, `scripts/dev/link-signing.sh`, `.gitignore`
- Modify: `Apps/macOS/Sources/AccountsPane.swift:17`, `Apps/macOS/Sources/ProviderIcon.swift`, `Apps/macOS/Sources/SettingsSearch.swift:40`
- Modify: `Apps/macOS/Tests/ProviderIconTests.swift`

**Interfaces:**
- Consumes: `MICROSOFT_OAUTH_CLIENT_ID` from `~/.config/timetug/microsoft-oauth.xcconfig`.
- Produces: `ProviderIcon.Style.microsoft`; the env override `TIMETUG_MICROSOFT_XCCONFIG` for `link-signing.sh` (used by CI in Task 4).

- [ ] **Step 1: Write the failing test** — in `Apps/macOS/Tests/ProviderIconTests.swift` add inside the class:

```swift
    func testMicrosoftAccountsGetTheMicrosoftMark() {
        XCTAssertEqual(ProviderIcon.Style.forKind("microsoft"), .microsoft)
    }
```

- [ ] **Step 2: Xcconfig, link script and gitignore**

Append to `Apps/macOS/Config/Signing.xcconfig`:

```
// Microsoft Entra app registration (git-ignored). Defines MICROSOFT_OAUTH_CLIENT_ID, which project.yml injects into
// Info.plist. scripts/dev/link-signing.sh links it from ~/.config/timetug/microsoft-oauth.xcconfig.
// A missing file leaves it empty and Microsoft is not offered.
#include? "MicrosoftOAuth.xcconfig"
```

In `scripts/dev/link-signing.sh` change the header comment's second sentence to mention both files, and append one line after the Google `link_one` line:

```bash
link_one "${TIMETUG_MICROSOFT_XCCONFIG:-${HOME:-/nonexistent}/.config/timetug/microsoft-oauth.xcconfig}" "$CONFIG_DIR/MicrosoftOAuth.xcconfig"
```

Update the comment lines above it: "...and ~/.config/timetug/microsoft-oauth.xcconfig to Apps/macOS/Config/MicrosoftOAuth.xcconfig (all git-ignored)... Environment: TIMETUG_SIGNING_XCCONFIG, TIMETUG_GOOGLE_XCCONFIG and TIMETUG_MICROSOFT_XCCONFIG override the source paths."

Append to `.gitignore` after the `GoogleOAuth.xcconfig` line:

```
Apps/macOS/Config/MicrosoftOAuth.xcconfig
```

- [ ] **Step 3: Accounts pane** — in `AccountsPane.swift` delete the placeholder line `.init(name: "Microsoft", systemImage: "envelope"),` (the real kind now shows as an available or "Unavailable" tile; keeping the placeholder would show two).

- [ ] **Step 4: Icon** — in `ProviderIcon.swift` replace the `Style` enum and the `body`'s background/switch:

```swift
    enum Style: Equatable {
        case google, microsoft, generic

        static func forKind(_ kindID: String) -> Style {
            switch kindID {
            case "google": .google
            case "microsoft": .microsoft
            default: .generic
            }
        }
    }
```

```swift
            switch Style.forKind(kindID) {
            case .google: GoogleMark().padding(size * 0.2)
            case .microsoft: MicrosoftMark().padding(size * 0.22)
            case .generic:
                Image(systemName: "person.crop.circle")
                    .resizable().scaledToFit().foregroundStyle(.secondary).padding(size * 0.1)
            }
        }
        .frame(width: size, height: size)
        .background(Style.forKind(kindID) == .generic ? AnyShapeStyle(.clear) : AnyShapeStyle(.white),
                    in: RoundedRectangle(cornerRadius: size * 0.22))
```

and add below `GoogleMark`:

```swift
/// Four coloured squares in a two by two grid.
private struct MicrosoftMark: View {
    var body: some View {
        GeometryReader { geo in
            let side = min(geo.size.width, geo.size.height)
            let gap = side * 0.07
            let cell = (side - gap) / 2
            ZStack(alignment: .topLeading) {
                square(Color(red: 0.95, green: 0.31, blue: 0.13), cell).offset(x: 0, y: 0)
                square(Color(red: 0.50, green: 0.73, blue: 0.00), cell).offset(x: cell + gap, y: 0)
                square(Color(red: 0.00, green: 0.64, blue: 0.94), cell).offset(x: 0, y: cell + gap)
                square(Color(red: 1.00, green: 0.73, blue: 0.00), cell).offset(x: cell + gap, y: cell + gap)
            }
            .frame(width: side, height: side)
            .position(x: geo.size.width / 2, y: geo.size.height / 2)
        }
    }

    private func square(_ color: Color, _ side: CGFloat) -> some View {
        Rectangle().fill(color).frame(width: side, height: side)
    }
}
```

- [ ] **Step 5: Search keywords** — in `SettingsSearch.swift` add `"microsoft", "outlook", "office 365", "exchange"` to the accounts item's keywords array (after `"google"`).

- [ ] **Step 6: Regenerate and run the whole app test suite**

Run: `cd Apps/macOS && xcodegen generate && cd ../.. && git checkout Apps/macOS/Sources/Info.plist Apps/macOS/Widgets/Info.plist && xcodebuild -project Apps/macOS/TimeTug.xcodeproj -scheme TimeTug test 2>&1 | tail -15`
Expected: `** TEST SUCCEEDED **` (a `SettingsSearchTests` case may assert the keyword list; update it if it fails).

- [ ] **Step 7: Verify sign-in appears locally** (uses the real `~/.config/timetug/microsoft-oauth.xcconfig`; never print it)

Run: `scripts/dev/link-signing.sh && ls -l Apps/macOS/Config/MicrosoftOAuth.xcconfig | awk '{print $NF}'`
Expected: prints the path `/Users/.../.config/timetug/microsoft-oauth.xcconfig`. Build and launch the app, open Settings > Accounts, and confirm a "Microsoft" tile without an "Unavailable" badge. (The user completes an actual sign-in in Task 5.)

- [ ] **Step 8: Commit**

```bash
git add Apps/macOS scripts/dev/link-signing.sh .gitignore
git commit -m "App: inject the Microsoft client id at build time; Microsoft tile, mark and search keywords"
```

---

## Part C: CI

### Task 3: The CI helper

**Files:**
- Create: `scripts/ci/microsoft-oauth-config.sh`
- Create: `scripts/ci/tests/test-microsoft-oauth-config.sh`

**Interfaces:**
- Produces: `prepare_microsoft_oauth_xcconfig [DIR]`: when `MICROSOFT_OAUTH_CLIENT_ID` is non-empty, writes `MICROSOFT_OAUTH_CLIENT_ID = <id>` to a mode-600 temp file, exports `TIMETUG_MICROSOFT_XCCONFIG`, removes the file on exit (keeping an existing EXIT trap); does nothing when unset or empty; never prints the value.

- [ ] **Step 1: Write the failing test** — `scripts/ci/tests/test-microsoft-oauth-config.sh` (make it executable):

```bash
#!/usr/bin/env bash
# Tests scripts/ci/microsoft-oauth-config.sh (prepare_microsoft_oauth_xcconfig) with fake values only.
set -euo pipefail
cd "$(dirname "$0")/../../.."
H="$PWD/scripts/ci/microsoft-oauth-config.sh"
fail() { echo "FAIL: $*" >&2; exit 1; }
[ -f "$H" ] || fail "helper $H is missing"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT

# Set: file written 0600 with the line, env exported, file removed on exit, prior EXIT trap kept.
out="$(env -i PATH="$PATH" MICROSOFT_OAUTH_CLIENT_ID=test-client-id \
  RUNNER_TEMP="$T" HELPER="$H" MARK="$T/prev-trap-ran" bash -c '
  set -euo pipefail
  trap "touch \"\$MARK\"" EXIT
  source "$HELPER"
  prepare_microsoft_oauth_xcconfig "$RUNNER_TEMP"
  f="$TIMETUG_MICROSOFT_XCCONFIG"
  [ -n "$f" ] || exit 11
  printf "%s\n" "$f"
  stat -f "%Lp" "$f" 2>/dev/null || stat -c "%a" "$f"
  cat "$f"
  bash -c "[ -n \"\${TIMETUG_MICROSOFT_XCCONFIG:-}\" ]" || exit 12   # exported to children
')" || fail "set: helper failed"
f="$(printf '%s\n' "$out" | sed -n 1p)"
[ "$(printf '%s\n' "$out" | sed -n 2p)" = 600 ] || fail "set: mode is not 600"
[ "$(printf '%s\n' "$out" | sed -n 3p)" = "MICROSOFT_OAUTH_CLIENT_ID = test-client-id" ] || fail "set: id line"
case "$f" in "$T"/*) ;; *) fail "set: file not under RUNNER_TEMP";; esac
[ ! -e "$f" ] || fail "set: temp file not removed on exit"
[ -e "$T/prev-trap-ran" ] || fail "set: existing EXIT trap was clobbered"

# Unset or empty: nothing written, nothing exported.
for setup in "" "MICROSOFT_OAUTH_CLIENT_ID="; do
  mkdir -p "$T/n"; rm -rf "$T/n"/*
  # shellcheck disable=SC2086
  env -i PATH="$PATH" $setup RUNNER_TEMP="$T/n" HELPER="$H" bash -c '
    set -euo pipefail
    source "$HELPER"
    prepare_microsoft_oauth_xcconfig "$RUNNER_TEMP"
    [ -z "${TIMETUG_MICROSOFT_XCCONFIG:-}" ] || exit 21
  ' || fail "unset: helper failed or exported a path"
  [ -z "$(ls -A "$T/n")" ] || fail "unset: a file was written"
done
echo "PASS"
```

- [ ] **Step 2: Run it to verify it fails**

Run: `chmod +x scripts/ci/tests/test-microsoft-oauth-config.sh && scripts/ci/tests/test-microsoft-oauth-config.sh`
Expected: `FAIL: helper .../microsoft-oauth-config.sh is missing`

- [ ] **Step 3: Write the helper** — `scripts/ci/microsoft-oauth-config.sh`:

```bash
#!/usr/bin/env bash
# Sourceable helper: hand the Microsoft Entra client id to the build without writing it into the repo.
#
# prepare_microsoft_oauth_xcconfig [DIR]
#   When MICROSOFT_OAUTH_CLIENT_ID is non-empty, writes it as an xcconfig line to a mode-600 temp file under DIR
#   (default: ${RUNNER_TEMP:-$TMPDIR}), exports TIMETUG_MICROSOFT_XCCONFIG pointing at it
#   (scripts/dev/link-signing.sh links it in during `xcodegen generate`) and removes the file on exit, keeping any
#   EXIT trap that already exists. When it is unset or empty it does nothing (local and PR builds stay Microsoft-less).
# A public client id is not confidential (it ships inside the app and appears in the sign-in URL), but it stays out of
# git and logs.

_tt_microsoft_cleanup() { [ -z "${_TT_MICROSOFT_TMP:-}" ] || rm -f "$_TT_MICROSOFT_TMP"; }

prepare_microsoft_oauth_xcconfig() {
  local dir="${1:-${RUNNER_TEMP:-${TMPDIR:-/tmp}}}"
  local id="${MICROSOFT_OAUTH_CLIENT_ID:-}"
  [ -n "$id" ] || return 0
  mkdir -p "$dir"
  local umask_old; umask_old="$(umask)"
  umask 077
  _TT_MICROSOFT_TMP="$(mktemp "$dir/microsoft-oauth.XXXXXX")" || { umask "$umask_old"; return 1; }
  umask "$umask_old"
  chmod 600 "$_TT_MICROSOFT_TMP"
  printf 'MICROSOFT_OAUTH_CLIENT_ID = %s\n' "$id" > "$_TT_MICROSOFT_TMP"
  # Chain onto an existing EXIT trap instead of replacing it.
  local prev=""
  eval "set -- $(trap -p EXIT)"
  [ "${1:-}" = "trap" ] && prev="${3:-}"
  # shellcheck disable=SC2064
  trap "${prev:+$prev; }_tt_microsoft_cleanup" EXIT
  export TIMETUG_MICROSOFT_XCCONFIG="$_TT_MICROSOFT_TMP"
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `scripts/ci/tests/test-microsoft-oauth-config.sh`
Expected: `PASS`

- [ ] **Step 5: Commit**

```bash
git add scripts/ci/microsoft-oauth-config.sh scripts/ci/tests/test-microsoft-oauth-config.sh
git commit -m "CI: helper that hands the Microsoft client id to the build"
```

### Task 4: Wire the helper into the release build and workflows

**Files:**
- Modify: `scripts/ci/build-release.sh:14-15,45-47,93-103`
- Modify: `.github/workflows/beta.yml:88-90`, `.github/workflows/release.yml:128-130`
- Modify: `scripts/ci/tests/test-workflows.sh:40-46`
- Modify: `docs/release.md:147`

**Interfaces:**
- Consumes: `prepare_microsoft_oauth_xcconfig`, `TIMETUG_MICROSOFT_XCCONFIG` (Task 3); repository secret `MICROSOFT_OAUTH_CLIENT_ID` (set by the user).

- [ ] **Step 1: Extend the workflow test first** — in `scripts/ci/tests/test-workflows.sh` replace the block from the "The Google OAuth secrets belong..." comment to the `ci.yml` check with:

```bash
# The OAuth secrets belong to the privileged build workflows only, never to PR CI.
for w in beta release; do
  for s in GOOGLE_OAUTH_CLIENT_ID GOOGLE_OAUTH_CLIENT_SECRET MICROSOFT_OAUTH_CLIENT_ID; do
    grep -q "secrets\.$s" ".github/workflows/$w.yml" || { echo "FAIL: $w.yml does not reference secrets.$s" >&2; exit 1; }
  done
done
if grep -q "GOOGLE_OAUTH\|MICROSOFT_OAUTH" .github/workflows/ci.yml; then echo "FAIL: ci.yml must not reference the OAuth secrets" >&2; exit 1; fi
```

Run: `scripts/ci/tests/test-workflows.sh`
Expected: `FAIL: beta.yml does not reference secrets.MICROSOFT_OAUTH_CLIENT_ID`

- [ ] **Step 2: Workflows** — in both `beta.yml` and `release.yml`, in the `Build Release app` step's `env:` add after the Google secret line:

```yaml
          MICROSOFT_OAUTH_CLIENT_ID: ${{ secrets.MICROSOFT_OAUTH_CLIENT_ID }}
```

- [ ] **Step 3: build-release.sh** — add to the environment comment after the Google entry:

```
#   MICROSOFT_OAUTH_CLIENT_ID  Microsoft Entra client id baked into the app (optional; without it Microsoft is not
#              offered). Never printed; see scripts/ci/microsoft-oauth-config.sh.
```

After the two Google lines (`source ... prepare_google_oauth_xcconfig ...`) add:

```bash
# shellcheck source=scripts/ci/microsoft-oauth-config.sh
source "$ROOT/scripts/ci/microsoft-oauth-config.sh"
prepare_microsoft_oauth_xcconfig "${RUNNER_TEMP:-$BUILD_DIR}"
```

After the Google verification block (ends with `echo "Google OAuth client: absent"` / `fi`) add:

```bash
# Verify the Microsoft client id landed in the app without printing it.
microsoft_id="$(/usr/libexec/PlistBuddy -c 'Print :TimeTugMicrosoftClientID' "$PLIST" 2>/dev/null || true)"
if [ -n "${TIMETUG_MICROSOFT_XCCONFIG:-}" ]; then
  if [ -z "$microsoft_id" ] || [[ "$microsoft_id" == *'$('* ]]; then
    echo "error: Microsoft client id was provided but is missing from the built Info.plist" >&2
    exit 1
  fi
  echo "Microsoft client id: configured"
else
  echo "Microsoft client id: absent"
fi
```

- [ ] **Step 4: Docs** — in `docs/release.md` after the Google paragraph (line 147) add:

```markdown
One more optional secret, `MICROSOFT_OAUTH_CLIENT_ID`, holds the client (application) id of the Microsoft Entra app registration that `beta.yml` and `release.yml` bake into the app the same way (only the `Build Release app` step reads it). It is a public client id, not a secret in the cryptographic sense (it appears in the sign-in URL), but keep it out of git and logs. Without it the build still succeeds but Microsoft shows as unavailable in Settings > Accounts. Set it with `gh secret set MICROSOFT_OAUTH_CLIENT_ID` (paste the value at the prompt). The registration must list `http://localhost` as a "Mobile and desktop applications" redirect URI and allow personal and work accounts; work accounts outside the registering tenant need an admin's approval until the app's publisher is verified (see the Phase 4 spec).
```

- [ ] **Step 5: Run every CI test**

Run: `for t in scripts/ci/tests/test-*.sh; do echo "== $t"; "$t"; done`
Expected: each prints `PASS`.

- [ ] **Step 6: Commit**

```bash
git add scripts/ci .github/workflows docs/release.md
git commit -m "CI: bake the Microsoft client id into beta and release builds"
```

- [ ] **Step 7 (user): set the secret** — give the user this command (run by the user, not by the agent; the agent never sets or prints the value):

```bash
gh secret set MICROSOFT_OAUTH_CLIENT_ID
```

---

## Part D: Live smoke test, docs, review

### Task 5: The opt-in live smoke test

The unit tests use a fake Graph. This test runs against a real mailbox and prints what only Microsoft can answer. It is opt-in (`TIMETUG_LIVE_MICROSOFT=1`), interactive (browser sign-in), creates only events titled "TimeTug write smoke" (no attendees; nothing is emailed) and deletes them at the end.

**Files:**
- Modify: `Packages/CalendarApple/Package.swift` (add the `MicrosoftCalendar` product to the test target)
- Create: `Packages/CalendarApple/Tests/CalendarAppleTests/MicrosoftLiveWriteSmokeTests.swift`

**Interfaces:**
- Consumes: `MicrosoftConnectorKind`, `MicrosoftOAuthConfig`, `LoopbackAuthorizationInteraction`, `CryptoKitSHA256`, `WritableCalendarSource`, `SeriesSource`.

- [ ] **Step 1: Package.swift** — in the test target's dependencies add `.product(name: "MicrosoftCalendar", package: "CalendarConnectors"),` after the GoogleCalendar line.

- [ ] **Step 2: Write the test**

```swift
import CalendarApple
import CalendarCore
import CalendarOAuth
import Foundation
import MicrosoftCalendar
import Testing

private let liveMicrosoft = ProcessInfo.processInfo.environment["TIMETUG_LIVE_MICROSOFT"] == "1"

/// Every event this test creates starts with this prefix, and nothing without it is ever deleted.
private let smokePrefix = "TimeTug write smoke"

/// Deletes every smoke event in `window` (series once, by master). Failures are printed, not thrown.
private func cleanUp(_ source: any CalendarSource, _ writable: any WritableCalendarSource, calendarID: String, window: DateInterval) async {
    await Task.detached {   // detached so it still runs when the test task was cancelled
        do {
            let mine = try await source.events(in: window).filter { $0.title.hasPrefix(smokePrefix) && $0.calendarID == calendarID }
            var seen = Set<String>()
            for event in mine {
                let key = event.seriesID ?? event.eventID
                guard seen.insert(key).inserted else { continue }
                do { try await writable.delete(EventRef(event), scope: .allInSeries, notify: .none) }
                catch { print("LIVE cleanup could not delete \"\(event.title)\": \(error)") }
            }
        } catch { print("LIVE cleanup could not list events: \(error)") }
    }.value
}

/// Opt-in, interactive: `TIMETUG_LIVE_MICROSOFT=1 MICROSOFT_OAUTH_CLIENT_ID=<id> swift test
/// --package-path Packages/CalendarApple --filter microsoftWriteSmoke` (read the id from your xcconfig; do not paste
/// it into shared logs). Prints `LIVE ...` lines; record the answers in the Phase 4 spec:
/// - the sign-in with the `http://localhost` redirect and the account's identity;
/// - the calendars and their permissions, and the account's zone (`LIVE calendar ...`);
/// - a created single event's version, uid scope and stored zone, and whether an edit changes the version;
/// - a Teams meeting on `.generate` (a personal account may refuse it: `LIVE teams: ...`);
/// - a weekly numbered series: the occurrences read back, and `.thisAndFollowing` on the third occurrence, printed as
///   the instance count per series after the split (expected `[2, 2]`), including the numbered-count arithmetic;
/// - `originalStart` on each occurrence (`LIVE occurrence ...`), which the split depends on.
@Test(.enabled(if: liveMicrosoft), .timeLimit(.minutes(10))) func microsoftWriteSmoke() async throws {
    let clientID = try #require(ProcessInfo.processInfo.environment["MICROSOFT_OAUTH_CLIENT_ID"])
    let kind = MicrosoftConnectorKind(config: MicrosoftOAuthConfig(clientID: clientID), hasher: CryptoKitSHA256())
    let interaction = LoopbackAuthorizationInteraction(openURL: { url in
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/open")
        process.arguments = [url.absoluteString]
        do { try process.run(); return true } catch { return false }
    })
    let credentials = InMemoryCredentialStore()
    let connection = try await kind.authorize(using: interaction, credentials: credentials)
    print("LIVE signed in (\(connection.displayName.contains("@") ? "identity read" : "no identity"))")
    let source = try kind.makeSource(for: connection, credentials: credentials, syncState: InMemorySyncStateStore())
    let writable = try #require(source as? WritableCalendarSource)
    let calendars = try await source.calendars()
    for calendar in calendars {
        print("LIVE calendar default=\(calendar.isDefault == true) canEdit=\(calendar.permissions.canEdit) zone=\(calendar.timeZone.identifier)")
    }
    let primary = try #require(calendars.first { $0.isDefault == true })

    let zone = primary.timeZone
    var local = Calendar(identifier: .gregorian)
    local.timeZone = zone
    let tomorrow = try #require(local.date(byAdding: .day, value: 1, to: Date()))
    let start = try #require(local.date(bySettingHour: 3, minute: 0, second: 0, of: tomorrow))
    func timing(_ offsetDays: Int) -> EventTiming {
        let s = start.addingTimeInterval(Double(offsetDays) * 86_400)
        return EventTiming(start: s, end: s.addingTimeInterval(1800), timeZone: zone, isAllDay: false)
    }
    // The series below ends about day 23 (weekly, four occurrences from day 2).
    let window = DateInterval(start: start.addingTimeInterval(-3600), duration: 86_400 * 30)

    await cleanUp(source, writable, calendarID: primary.id, window: window)   // leftovers from an earlier run
    do {
        // 1. A single event: create, edit, and see whether the version changes.
        let single = try await writable.create(EventDraft(title: smokePrefix, timing: timing(0), location: "Room 1"), in: primary.id, notify: .none)
        print("LIVE single uidScope=\(String(describing: single.uidScope)) zone=\(single.timeZone.identifier) version=\(single.version != nil)")
        let renamed = try await writable.update(EventRef(single), EventPatch(title: "\(smokePrefix) renamed"), scope: .thisInstance, notify: .none)
        #expect(renamed.title == "\(smokePrefix) renamed" && renamed.location == "Room 1")
        print("LIVE edit changed the version: \(renamed.version != single.version)")
        try await writable.delete(EventRef(renamed), scope: .thisInstance, notify: .none)

        // 2. Teams on generate. A personal account may refuse it; either answer is recorded.
        do {
            let meeting = try await writable.create(
                EventDraft(title: "\(smokePrefix) teams", timing: timing(1), conference: .generate), in: primary.id, notify: .none)
            print("LIVE teams: created, conferences=\(meeting.conferences.map { "\($0.provider)" })")
            try await writable.delete(EventRef(meeting), scope: .thisInstance, notify: .none)
        } catch { print("LIVE teams: \(error)") }

        // 3. A weekly numbered series, read back, then split at the third occurrence.
        let rule = RecurrenceRule(frequency: .weekly, end: .count(4))
        let master = try await writable.create(
            EventDraft(title: "\(smokePrefix) series", timing: timing(2), recurrence: rule), in: primary.id, notify: .none)
        var occurrences: [CalendarEvent] = []
        for _ in 0..<15 {
            occurrences = try await source.events(in: window).filter { $0.title == "\(smokePrefix) series" }.sorted { $0.start < $1.start }
            if occurrences.count == 4 { break }
            try await Task.sleep(for: .seconds(1))
        }
        for occurrence in occurrences {
            print("LIVE occurrence start=\(occurrence.start) originalStart=\(String(describing: occurrence.originalStart)) seriesID=\(occurrence.seriesID != nil)")
        }
        let third = try #require(occurrences.count == 4 ? occurrences[2] : nil)
        _ = try await writable.update(EventRef(third), EventPatch(title: "\(smokePrefix) series 2"), scope: .thisAndFollowing, notify: .none)
        var counts: [Int] = []
        for _ in 0..<15 {
            let now = try await source.events(in: window)
            counts = [now.filter { $0.title == "\(smokePrefix) series" }.count, now.filter { $0.title == "\(smokePrefix) series 2" }.count]
            if counts == [2, 2] { break }
            try await Task.sleep(for: .seconds(1))
        }
        print("LIVE split instance counts [old, new] = \(counts)")
        #expect(counts == [2, 2])
        _ = master
    } catch {
        print("LIVE smoke failed: \(error)")
        await cleanUp(source, writable, calendarID: primary.id, window: window)
        throw error
    }
    await cleanUp(source, writable, calendarID: primary.id, window: window)
}
```

- [ ] **Step 3: Make sure it compiles and is skipped by default**

Run: `swift test --package-path Packages/CalendarApple --filter microsoftWriteSmoke 2>&1 | tail -6`
Expected: the test is reported as skipped (or the run passes with 0 executed); no compile errors.

- [ ] **Step 4: Commit**

```bash
git add Packages/CalendarApple
git commit -m "CalendarApple: opt-in live smoke test for the Microsoft connector"
```

- [ ] **Step 5 (user): run it once**, then paste the `LIVE` lines back so the spec can record them:

```bash
set -a; source ~/.config/timetug/microsoft-oauth.xcconfig 2>/dev/null; set +a
TIMETUG_LIVE_MICROSOFT=1 MICROSOFT_OAUTH_CLIENT_ID="${MICROSOFT_OAUTH_CLIENT_ID// /}" swift test --package-path Packages/CalendarApple --filter microsoftWriteSmoke 2>&1 | grep -E "LIVE|✘|✔"
```

(The xcconfig line reads `MICROSOFT_OAUTH_CLIENT_ID = <guid>`; if `source` cannot parse the spaces, export the value by hand.) Open questions this run settles, and what to do with each answer:
1. Sign-in works with the `http://localhost` redirect. If Microsoft rejects the redirect, set `MicrosoftOAuthConfig(redirectHost: nil)` in `AppConnectors` and re-register `http://127.0.0.1`.
2. `originalStart` is present on occurrences. If it is nil, `.thisAndFollowing` cannot split; the connector must then read `originalStart` from the instance's `start` and the mapper needs a fix.
3. The split leaves `[2, 2]`. If the old series shows 3, `truncated`'s `endDate` is inclusive of the split day in the account's zone; adjust `GraphRecurrenceMapper.truncated`.
4. Teams generation on a personal account; the delta window lifetime and a shared calendar are checked by hand in the app (Task 6, Step 3).

### Task 6: Docs, spec reconciliation, review and PR

**Files:**
- Modify: `docs/calendar-connectors-api.md` (add a Microsoft part), `AGENTS.md:48` region, the spec

- [ ] **Step 1: API contract** — append a section "Part 11: MicrosoftCalendar" to `docs/calendar-connectors-api.md` covering: the kind (`MicrosoftConnectorKind(config: MicrosoftOAuthConfig(clientID:), hasher:)`, kind id `microsoft`, public client, `common` authority, scopes, `http://localhost` redirect), capabilities (fully writable, `syncKind == .token`, `controlsNotifications == false`, `respond` alone honors `NotifyPolicy` via `sendResponse`), the account time zone rule (mailbox zone, UTC fallback), delta change detection (`calendarView/delta` over -30d/+365d, re-baselined after 14 days), the write limits (one display reminder, reminders can't be cleared, recurrence can be set but not removed, Teams on generate, no `If-Match` so optimistic locking is a read-before-write), `.thisAndFollowing` as truncate-and-insert with `transactionId` and `WriteError.partial`, and `SeriesSource` (excluded/extra dates are nil). Mirror the layout of the Google part.

- [ ] **Step 2: AGENTS.md** — after the Google OAuth bullet add:

```markdown
- Microsoft accounts need an Entra app registration's client id (a public client, no secret): a git-ignored `Apps/macOS/Config/MicrosoftOAuth.xcconfig` (or `~/.config/timetug/microsoft-oauth.xcconfig`, linked in by `scripts/dev/link-signing.sh`) defining `MICROSOFT_OAUTH_CLIENT_ID`. Without it Microsoft shows as unavailable in Settings > Accounts. Beta and release builds get it from the optional repository secret `MICROSOFT_OAUTH_CLIENT_ID` through `scripts/ci/microsoft-oauth-config.sh`, the same way as Google (never `ci.yml`).
```

- [ ] **Step 3: Manual checklist (user)** in the app, with the real account: add the account and see it in the list with the right name; events from the Outlook calendar appear and tug on time; a Teams meeting shows its Join link; a shared or delegated calendar (if the account has one) appears read-only or writable to match its permissions; remove and re-add the account; after 14+ days a poll re-baselines silently.

- [ ] **Step 4: Reconcile the spec** — edit the Phase 4 spec so it matches the code: the redirect host defaults to `localhost`; there is no color-enum fallback; occurrences are never looked up by `originalStart` (the ref's `originalStart` is only the split point); series `excludedDates`/`extraDates` are nil; writes only ever send Monday as `firstDayOfWeek`; reads send `Prefer: outlook.body-content-type="text"`; `iCalUId` is read-only, so `EventDraft.uid` is used only for the duplicate check and is not stored; a numbered split counts prior occurrences through the `instances` endpoint. Add the live-smoke results once the user has run Task 5.

- [ ] **Step 5: Whole-repo verification**

Run: `swift test --package-path Packages/CalendarConnectors 2>&1 | grep -E "Test run with|✘"; swift test --package-path Packages/CalendarBridge 2>&1 | tail -3; for t in scripts/ci/tests/test-*.sh; do "$t" | tail -1; done`
Expected: every run passes (Microsoft 116, Core 198, OAuth 26, Google 183).

- [ ] **Step 6: Commit docs**

```bash
git add docs AGENTS.md
git commit -m "Docs: Microsoft connector API, setup and spec reconciliation"
```

- [ ] **Step 7: DeepSeek review of the branch diff** (skill `deepseek-review`, brief = the Phase 4 spec; exclude OAuth Swift files the secret scanner flags on variable names, describing their behavior in the background instead). Verify every Critical or Important finding against the code before acting; fix or dismiss, and re-run the tests.

- [ ] **Step 8: Push and open the PR to `master`** (never merge locally; squash-merge only when the user says so). Bind the PR with the ccd_pr tools and offer Auto-fix for CI.
