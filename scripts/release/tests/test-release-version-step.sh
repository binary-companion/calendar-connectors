#!/usr/bin/env bash
# Tests the "Compute the version" step of .github/workflows/release.yml.
#
# The step's run: script is extracted from the workflow and executed in throwaway git repositories
# with a stub `gh` on PATH, so the real script is what is tested, not a copy of it.
# PYTHON selects the interpreter that has PyYAML (CI points it at build/venv/bin/python).
# STEP_SHELL selects the shell that runs the step (default bash).
set -euo pipefail
cd "$(dirname "$0")/../../.."
ROOT="$PWD"
PYTHON="${PYTHON:-python3}"
STEP_SHELL="${STEP_SHELL:-bash}"
WORKFLOW=.github/workflows/release.yml
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

# --- extract the step and check the shape of the job it belongs to ---
"$PYTHON" - "$WORKFLOW" "$T/step.sh" "$T/step-env" <<'PY'
import re, sys, yaml
workflow, script, envfile = sys.argv[1:]
job = yaml.safe_load(open(workflow))['jobs']['publish']
def check(condition, message):
    if not condition:
        sys.exit(f'FAIL: {workflow}: {message}')
check(job.get('if') == "github.ref == 'refs/heads/main'", "job publish must run only for refs/heads/main")
check(job.get('environment') == 'publish', "job publish must use the publish environment")
checkout = [s for s in job['steps'] if str(s.get('uses', '')).startswith('actions/checkout@')]
check(len(checkout) == 1, "job publish must have exactly one checkout step")
check(re.search(r'@[0-9a-f]{40}$', checkout[0]['uses'].split()[0]), "checkout must be pinned to a commit SHA")
check(checkout[0].get('with', {}).get('persist-credentials') is False, "checkout must set persist-credentials: false")
check(checkout[0]['with'].get('fetch-depth') == 0, "checkout must fetch the full history and the tags")
steps = [s for s in job['steps'] if s.get('id') == 'version']
check(len(steps) == 1, "job publish must have exactly one step with id: version")
open(script, 'w').write(steps[0]['run'])
open(envfile, 'w').write('\n'.join(sorted(steps[0].get('env', {}))) + '\n')
PY
[ "$(cat "$T/step-env")" = "$(printf 'GH_TOKEN\nOVERRIDE\nREPO\nSHA')" ] ||
  fail "the step's env changed ($(tr '\n' ' ' < "$T/step-env")); update this harness"

# --- a stub gh: logs every call, answers /commits/<sha>/pulls from $STUB/labels/<sha> ---
mkdir -p "$T/bin"
cat > "$T/bin/gh" <<'STUB'
#!/usr/bin/env bash
echo "$*" >> "$STUB/calls"
[ ! -e "$STUB/fail" ] || { echo "gh: HTTP 502" >&2; exit 1; }
case "$*" in
  "api repos/o/r/commits/"*"/pulls --jq [.[].labels[].name] | join(\",\")") ;;
  *) echo "stub gh: unexpected call: $*" >&2; exit 99 ;;
esac
sha="${2#repos/o/r/commits/}"; sha="${sha%/pulls}"
if [ -f "$STUB/labels/$sha" ]; then cat "$STUB/labels/$sha"; fi
echo
STUB
chmod +x "$T/bin/gh"

# --- scenario helpers ---
n=0
new_repo() {
  n=$((n + 1)); R="$T/repo$n"; STUB="$T/stub$n"; export STUB
  mkdir -p "$R/scripts/release" "$STUB/labels"
  git -C "$R" init -q -b main
  for kv in user.name=t user.email=t@example.com commit.gpgsign=false tag.gpgsign=false; do
    git -C "$R" config "${kv%%=*}" "${kv#*=}"
  done
  cp "$ROOT/scripts/release/next-version.sh" "$R/scripts/release/"
  commit base
}
commit() { echo "$1" >> "$R/file"; git -C "$R" add -A; git -C "$R" commit -q -m "$1"; }
tag() { git -C "$R" tag "$@"; }
label() { printf '%s' "$2" > "$STUB/labels/$(git -C "$R" rev-parse "$1")"; } # label <rev> <labels>

# run_step <override>: sets STATUS, OUT (the GITHUB_OUTPUT file) and CALLS (gh invocations).
run_step() {
  STATUS=0; OUT="$STUB/output"; : > "$OUT"
  (cd "$R" && PATH="$T/bin:$PATH" GITHUB_OUTPUT="$OUT" GH_TOKEN=unused REPO=o/r \
     SHA="$(git rev-parse HEAD)" OVERRIDE="${1-}" "$STEP_SHELL" "$T/step.sh") >"$STUB/stdout" 2>"$STUB/stderr" || STATUS=$?
  CALLS=0; [ ! -f "$STUB/calls" ] || CALLS="$(wc -l < "$STUB/calls" | tr -d ' ')"
}
# check <case> <exit: N or "nonzero"> <output: "version=X" | "none"> <gh calls>
check() {
  local name="$1" want_status="$2" want_out="$3" want_calls="$4" got
  if [ "$want_status" = nonzero ]; then
    [ "$STATUS" != 0 ] || fail "$name: exit 0, wanted a failure"
  else
    [ "$STATUS" = "$want_status" ] || { cat "$STUB/stderr" >&2; fail "$name: exit $STATUS, wanted $want_status"; }
  fi
  got="$(grep '^version=' "$OUT" || true)"
  if [ "$want_out" = none ]; then
    [ -z "$got" ] || fail "$name: wrote '$got', wanted no version output"
  else
    [ "$got" = "$want_out" ] || fail "$name: wrote '$got', wanted '$want_out'"
  fi
  [ "$CALLS" = "$want_calls" ] || fail "$name: $CALLS gh call(s), wanted $want_calls"
}

# 1. No X.Y.Z tag and no override: the exit-3 path. Nothing is released and the API is not called.
new_repo; commit one; run_step ""
check "no tag" 0 "version=" 0
grep -q '^::notice::' "$STUB/stdout" || fail "no tag: no ::notice:: explaining the skip"
# ...even when there are other tags that are not versions.
new_repo; tag v1.0.0; tag 1.0.0-rc1; tag 1.0; commit one; run_step ""
check "only non-version tags" 0 "version=" 0

# 2. The pushed SHA already carries a version tag and no override: skip (a re-run, or a blank dispatch).
new_repo; commit one; tag 1.0.0; run_step ""
check "tag at HEAD" 0 "version=" 0
grep -q '^::notice::' "$STUB/stdout" || fail "tag at HEAD: no ::notice:: explaining the skip"
new_repo; commit one; tag 1.0.0; tag v9.9.9; run_step ""
check "tag at HEAD plus a non-version tag" 0 "version=" 0
# A non-version tag at HEAD is not a release.
new_repo; tag 1.0.0; commit one; tag v2.0.0; run_step ""
check "non-version tag at HEAD" 0 "version=1.0.1" 1

# 3. A tag behind HEAD: the labels of EVERY commit in latest..SHA are united and the highest wins.
new_repo; tag 1.0.0; commit one; commit two; run_step ""
check "two unlabelled commits" 0 "version=1.0.1" 2
new_repo; tag 1.0.0; commit one; commit two
label HEAD~1 "bug"; label HEAD "semver:minor"; run_step ""
check "minor on the newest" 0 "version=1.1.0" 2
new_repo; tag 1.0.0; commit one; commit two
label HEAD~1 "semver:major"; label HEAD "semver:minor,docs"; run_step ""
check "major on a replaced run" 0 "version=2.0.0" 2
new_repo; tag 1.0.0; commit one; commit two
label HEAD "semver:major"; label HEAD~1 "semver:minor"; run_step ""
check "major on the newest" 0 "version=2.0.0" 2
new_repo; tag 1.0.0; commit one; run_step ""
check "single commit" 0 "version=1.0.1" 1
grep -q "commits/$(git -C "$R" rev-parse HEAD)/pulls" "$STUB/calls" || fail "single commit: the API was not asked about HEAD"
# Only commits after the tag are asked about, never the tagged commit or its ancestors.
new_repo; commit before; tag 1.0.0; commit one; run_step ""
grep -q "commits/$(git -C "$R" rev-parse HEAD~1)/" "$STUB/calls" && fail "the tagged commit was asked about"
# The newest of several version tags is the base (version sort, not text sort).
new_repo; tag 1.9.0; commit one; tag 1.10.0; commit two; label HEAD "semver:minor"; run_step ""
check "version sort" 0 "version=1.11.0" 1

# 4. An override wins, without any API call, and is allowed on a tagged commit (recovery).
new_repo; tag 1.0.0; commit one; commit two; run_step "3.0.0"
check "override" 0 "version=3.0.0" 0
new_repo; commit one; run_step "1.0.0"
check "first release by override" 0 "version=1.0.0" 0
new_repo; commit one; tag 1.0.0; run_step "1.0.1"
check "override on a tagged commit" 0 "version=1.0.1" 0

# 5. A malformed override fails the step and writes no version.
new_repo; tag 1.0.0; commit one; run_step "v1.0.1"
check "malformed override" 2 none 0
new_repo; commit one; run_step "1.0"
check "malformed override, no tag" 2 none 0

# 6. An API failure fails the step; it never falls back to a patch release.
new_repo; tag 1.0.0; commit one; commit two; : > "$STUB/fail"; run_step ""
check "API failure" nonzero none 1

# 7. A SHA the tag is not an ancestor of is still handled (every unreachable commit is asked about).
new_repo; tag 1.0.0; git -C "$R" checkout -q --orphan other; commit island; run_step ""
check "unrelated history" 0 "version=1.0.1" 1

echo PASS
