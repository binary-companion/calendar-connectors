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
printf '@preconcurrency import CalendarCore\nimport struct Foundation.Date\n' > "$T/Sources/GoogleCalendar/A.swift"
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
expect Package.swift 'let package = Package(dependencies: [.package(url: "https://github.com/x/y", from: "1.0.0")])\n' \
  "has package dependencies"
echo PASS
