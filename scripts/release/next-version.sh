#!/usr/bin/env bash
# Prints the version a push to main should release.
#
#   next-version.sh --latest <X.Y.Z or ""> --labels <comma-separated PR labels> [--override <X.Y.Z>]
#
# Exit codes: 0 printed a version; 2 bad usage or malformed input; 3 no tag exists and no override
# was given (the extraction push: nothing is released until the first version is dispatched by hand).
set -euo pipefail
set -f
latest="" labels="" override=""
while [ $# -gt 0 ]; do
  case "$1" in
    --latest|--labels|--override)
      [ $# -ge 2 ] || { echo "$1 needs a value" >&2; exit 2; }
      case "$1" in
        --latest) latest="$2" ;;
        --labels) labels="$2" ;;
        --override) override="$2" ;;
      esac
      shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
semver='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'
if [ -n "$override" ]; then
  [[ "$override" =~ $semver ]] || { echo "override is not X.Y.Z: $override" >&2; exit 2; }
  echo "$override"
  exit 0
fi
if [ -z "$latest" ]; then
  echo "no version tag exists; dispatch the workflow with a version" >&2
  exit 3
fi
[[ "$latest" =~ $semver ]] || { echo "latest tag is not X.Y.Z: $latest" >&2; exit 2; }
major="${BASH_REMATCH[1]}" minor="${BASH_REMATCH[2]}" patch="${BASH_REMATCH[3]}"
bump=patch
# Labels are separated by commas only: whitespace around a label is trimmed, whitespace inside one
# is kept, so "needs semver:major review" is a single label and is not a bump.
IFS=',' read -r -a parts <<< "$labels" || true
for label in ${parts[@]+"${parts[@]}"}; do
  label="${label#"${label%%[![:space:]]*}"}"
  label="${label%"${label##*[![:space:]]}"}"
  case "$label" in
    semver:major) bump=major ;;
    semver:minor) [ "$bump" = major ] || bump=minor ;;
  esac
done
case "$bump" in
  major) echo "$((major + 1)).0.0" ;;
  minor) echo "$major.$((minor + 1)).0" ;;
  patch) echo "$major.$minor.$((patch + 1))" ;;
esac
