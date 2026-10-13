#!/usr/bin/env bash
# Tests scripts/release/next-version.sh.
set -euo pipefail
cd "$(dirname "$0")/../../.."
S=scripts/release/next-version.sh
fail() { echo "FAIL: $*" >&2; exit 1; }
# expect <exit code> <stdout> <args...>
expect() {
  local want_code="$1" want_out="$2" out code=0; shift 2
  out="$("$S" "$@" 2>/dev/null)" || code=$?
  [ "$code" = "$want_code" ] || fail "$*: exit $code, wanted $want_code"
  [ "$out" = "$want_out" ] || fail "$*: printed '$out', wanted '$want_out'"
}
expect 0 1.0.1 --latest 1.0.0 --labels ""
expect 0 1.0.1 --latest 1.0.0 --labels "bug,docs"
expect 0 1.1.0 --latest 1.0.7 --labels "semver:minor"
expect 0 2.0.0 --latest 1.4.9 --labels "semver:major"
expect 0 2.0.0 --latest 1.4.9 --labels "semver:patch,semver:minor,semver:major"
expect 0 1.5.0 --latest 1.4.9 --labels "semver:patch,semver:minor"
expect 0 1.0.1 --latest 1.0.0 --labels "semver:Major,semver-minor,semver:"
expect 0 1.4.10 --latest 1.4.9 --labels ""
expect 0 3.0.0 --latest 1.0.0 --labels "semver:major" --override 3.0.0
expect 0 1.0.0 --latest "" --labels "" --override 1.0.0
expect 3 "" --latest "" --labels "semver:major"
expect 2 "" --latest 01.2.3 --labels ""
expect 2 "" --latest 1.2 --labels ""
expect 2 "" --latest 1.0.0 --labels "" --override v1.0.1
expect 2 "" --latest 1.0.0 --labels "" --bogus x
echo PASS
