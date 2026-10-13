#!/usr/bin/env bash
# The library imports nothing from TimeTug and nothing Apple-only, and has no package dependencies,
# so it builds on Linux and Windows. Usage: scripts/ci/check-architecture.sh [repo-root]
# Prints one "path:line: message" per violation and exits 1 if there are any.
set -euo pipefail
cd "${1:-$(dirname "$0")/../..}"
bad=0
report() { echo "$1: $2"; bad=$((bad + 1)); }
has() { case " $1 " in *" $2 "*) return 0 ;; esac; return 1; }

MODULES="$(ls Sources 2>/dev/null | tr '\n' ' ')"
ALLOWED="Foundation FoundationNetworking FoundationXML Testing $MODULES"
while read -r loc mod; do
  [ -n "${mod:-}" ] || continue
  has "$ALLOWED" "$mod" || report "$loc" "the library may not import $mod (allowed: $ALLOWED)"
done < <(grep -rnE --include='*.swift' '^[[:space:]]*(@[A-Za-z_]+(\([^)]*\))?[[:space:]]+)*import[[:space:]]' Sources Tests 2>/dev/null |
  sed -E 's/^([^:]+:[0-9]+):[[:space:]]*(@[A-Za-z_]+(\([^)]*\))?[[:space:]]+)*import[[:space:]]+((typealias|struct|class|enum|protocol|let|var|func)[[:space:]]+)?([A-Za-z_][A-Za-z0-9_]*).*/\1 \6/' || true)

if line="$(grep -n '\.package(' Package.swift | head -1)" && [ -n "$line" ]; then
  report "Package.swift:${line%%:*}" "the library has package dependencies (it must have none)"
fi

[ "$bad" = 0 ] || { echo "$bad architecture violation(s)" >&2; exit 1; }
