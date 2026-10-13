#!/usr/bin/env bash
# Tests check-architecture.sh: the repository passes, a fixture passes, each rule catches its violation.
set -euo pipefail
cd "$(dirname "$0")/../../.."
S="$PWD/scripts/ci/check-architecture.sh"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
"$S" >/dev/null || { "$S" >&2; fail "the repository itself violates the rules"; }

mkdir -p "$T/Sources/CalendarCore" "$T/Sources/GoogleCalendar" "$T/Tests/CalendarCoreTests"
printf '// swift-tools-version: 6.0\nlet package = Package(name: "x", targets: [])\n' > "$T/Package.swift"
printf '#if canImport(FoundationNetworking)\nimport FoundationNetworking\n#endif\nimport Foundation\n' > "$T/Sources/CalendarCore/A.swift"
printf '@preconcurrency import CalendarCore\nimport struct Foundation.Date\npublic import Foundation\n@_exported public import CalendarCore\n' > "$T/Sources/GoogleCalendar/A.swift"
printf 'import Testing\n@testable import CalendarCore\n' > "$T/Tests/CalendarCoreTests/T.swift"
out="$("$S" "$T")" || fail "clean fixture rejected: $out"

expect() { # expect <file> <content> <message substring>
  local f="$T/$1" had=0 out
  [ -e "$f" ] && { had=1; cp "$f" "$f.orig"; }
  mkdir -p "$(dirname "$f")"; printf '%b' "$2" > "$f"
  if out="$("$S" "$T" 2>/dev/null)"; then fail "accepted $1 containing: $2"; fi
  case "$out" in *"$3"*) ;; *) fail "for $1 expected '$3' in: $out" ;; esac
  if [ "$had" = 1 ]; then mv "$f.orig" "$f"; else rm "$f"; fi
}
expect Sources/CalendarCore/X.swift 'import AppKit\n' "may not import AppKit"
expect Sources/CalendarCore/X.swift 'import Security\n' "may not import Security"
expect Sources/GoogleCalendar/X.swift 'import TimeTugCore\n' "may not import TimeTugCore"
expect Sources/GoogleCalendar/X.swift 'import XMLCoder\n' "may not import XMLCoder"
expect Tests/CalendarCoreTests/X.swift 'import CryptoKit\n' "may not import CryptoKit"
expect Sources/GoogleCalendar/X.swift 'public import AppKit\n' "may not import AppKit"
expect Sources/GoogleCalendar/X.swift 'internal import AppKit\n' "may not import AppKit"
expect Sources/GoogleCalendar/X.swift 'package import AppKit\n' "may not import AppKit"
expect Sources/GoogleCalendar/X.swift 'fileprivate import AppKit\n' "may not import AppKit"
expect Sources/GoogleCalendar/X.swift 'private import AppKit\n' "may not import AppKit"
expect Sources/GoogleCalendar/X.swift '@_exported public import AppKit\n' "may not import AppKit"
expect Sources/GoogleCalendar/X.swift '@preconcurrency public import AppKit\n' "may not import AppKit"
expect Sources/GoogleCalendar/X.swift '@testable import AppKit\n' "may not import AppKit"
expect Sources/GoogleCalendar/X.swift '@available(macOS 14, *) import AppKit\n' "may not import AppKit"
expect Sources/GoogleCalendar/X.swift 'import struct AppKit.NSColor\n' "may not import AppKit"
expect Sources/GoogleCalendar/X.swift 'public import class AppKit.NSView\n' "may not import AppKit"
expect Sources/GoogleCalendar/X.swift '\n\n  import AppKit\n' "Sources/GoogleCalendar/X.swift:3: the library may not import AppKit"
expect Tests/CalendarCoreTests/X.swift '@testable import AppKit\n' "may not import AppKit"
expect Package.swift 'let package = Package(dependencies: [.package(url: "https://github.com/x/y", from: "1.0.0")])\n' \
  "has package dependencies"
# Platform conditions would make a target Apple-only: Package.swift may not use .when(...).
expect Package.swift 'let package = Package(targets: [.target(name: "x", dependencies: [.product(name: "y", package: "z", condition: .when(platforms: [.macOS]))])])\n' \
  "Package.swift:1: platform conditions (.when) are not allowed"
expect Package.swift '// swift-tools-version: 6.0\n\nlet package = Package(\n  targets: [.target(name: "x", swiftSettings: [.define("A", .when(platforms: [.linux]))])]\n)\n' \
  "Package.swift:4: platform conditions (.when) are not allowed"
# ... while the manifest's platforms list stays legal.
printf '// swift-tools-version: 6.0\nlet package = Package(name: "x", platforms: [.macOS(.v14)], targets: [])\n' > "$T/Package.swift"
out="$("$S" "$T")" || fail "platforms: [.macOS(.v14)] rejected: $out"
echo PASS
