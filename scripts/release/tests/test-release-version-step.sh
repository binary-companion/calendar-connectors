#!/usr/bin/env bash
# Tests the "Compute the version" step of .github/workflows/release.yml and pins the shape of the
# workflow's notify job (the dispatch to the dependents).
#
# The step's run: script is extracted from the workflow and executed in throwaway git repositories
# with a stub `gh` on PATH, so the real script is what is tested, not a copy of it.
# PYTHON selects the interpreter that has PyYAML (CI points it at build/venv/bin/python).
# WORKFLOW selects the workflow file (default .github/workflows/release.yml).
# STEP_SHELL selects the shell that runs the step (default bash).
set -euo pipefail
cd "$(dirname "$0")/../../.."
ROOT="$PWD"
PYTHON="${PYTHON:-python3}"
STEP_SHELL="${STEP_SHELL:-bash}"
WORKFLOW="${WORKFLOW:-.github/workflows/release.yml}" # overridable so a mutated copy can be checked
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

# --- the notify job: pinned shape, then its dispatch step executed against a stub gh ---
"$PYTHON" - "$WORKFLOW" "$T/notify.sh" <<'PY'
import re, sys, yaml
workflow, script = sys.argv[1:]
jobs = yaml.safe_load(open(workflow))['jobs']
def check(condition, message):
    if not condition:
        sys.exit(f'FAIL: {workflow}: notify: {message}')
check('notify' in jobs, "job notify must exist")
job = jobs['notify']
check(job.get('needs') in ('publish', ['publish']), "job notify must need exactly publish")
check(job.get('if') == "needs.publish.outputs.version != ''", "job notify must run only when publish released a version")
check(job.get('environment') == 'bump', "job notify must use the bump environment")
check(job.get('runs-on') == 'ubuntu-latest', "job notify must run on ubuntu-latest")
check('continue-on-error' not in job, "job notify must not set continue-on-error (a failed dispatch must fail the run)")
check(job.get('permissions') == {'contents': 'read'}, "job notify must have only contents: read")
steps = job['steps']
check(len(steps) == 2, "job notify must have exactly two steps (token, dispatch)")
token, dispatch = steps
for step in steps:
    check('continue-on-error' not in step, f"step {step.get('id') or step.get('name')!r} must not set continue-on-error")
check(token.get('id') == 'app', "the first step must be the token step with id: app")
check(token.get('uses') == 'actions/create-github-app-token@64ab729a6294000f5aa4a4bc0b73d1f32bc45510',
      "the token step must use actions/create-github-app-token pinned to the reviewed SHA")
check(token.get('with') == {'app-id': '${{ vars.BUMP_APP_ID }}',
                            'private-key': '${{ secrets.BUMP_APP_PRIVATE_KEY }}',
                            'owner': '${{ github.repository_owner }}',
                            'repositories': 'timetug',
                            'permission-contents': 'write'},
      "the token step must be scoped to repositories: timetug with only permission-contents: write (repository_dispatch) and read app-id/private-key from the bump environment")
check(dispatch.get('env') == {'GH_TOKEN': '${{ steps.app.outputs.token }}',
                              'VERSION': '${{ needs.publish.outputs.version }}'},
      "the dispatch step env must be exactly GH_TOKEN (app token) and VERSION (publish output)")
run = dispatch.get('run', '')
check('${{' not in run, "the dispatch script must not contain ${{ }} (the version only reaches it through env)")
check(re.search(r'for dependent in timetug;', run) is not None, "the dispatch must loop over exactly: timetug")
check('gh api "repos/binary-companion/$dependent/dispatches"' in run, "the dispatch must call repos/binary-companion/$dependent/dispatches")
fields = re.findall(r'-f\s+"?([^\s"]+)"?', run)
check(sorted(fields) == sorted(['event_type=library-released',
                                'client_payload[library]=calendar-connectors',
                                'client_payload[version]=$VERSION']),
      f"the dispatch fields must be exactly event_type, client_payload[library], client_payload[version] (got {fields})")
open(script, 'w').write(run)
PY
mkdir -p "$T/notify-bin"
cat > "$T/notify-bin/gh" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$NOTIFY_CALLS"
STUB
chmod +x "$T/notify-bin/gh"
NOTIFY_CALLS="$T/notify-calls"; export NOTIFY_CALLS; : > "$NOTIFY_CALLS"
PATH="$T/notify-bin:$PATH" VERSION=1.2.3 GH_TOKEN=unused "$STEP_SHELL" "$T/notify.sh" ||
  fail "the notify dispatch step failed against the stub gh"
[ "$(cat "$NOTIFY_CALLS")" = 'api repos/binary-companion/timetug/dispatches -f event_type=library-released -f client_payload[library]=calendar-connectors -f client_payload[version]=1.2.3' ] ||
  fail "the notify dispatch made the wrong call: $(cat "$NOTIFY_CALLS")"

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

# 7. Only tags reachable from the pushed SHA count: a tag on unrelated history is no base, so nothing
# is released and the API is not called.
new_repo; tag 1.0.0; git -C "$R" checkout -q --orphan other; commit island; run_step ""
check "unrelated history" 0 "version=" 0

# 8. A higher tag that is NOT an ancestor of the pushed SHA (a hand-made tag on a side branch) is
# ignored: the version comes from the highest REACHABLE tag, and only its commits are asked about.
new_repo; tag 1.0.0
git -C "$R" checkout -q -b side; commit sidework; tag 2.0.0; git -C "$R" checkout -q main
commit one; run_step ""
check "side-branch tag ignored" 0 "version=1.0.1" 1
grep -q "commits/$(git -C "$R" rev-parse HEAD)/pulls" "$STUB/calls" || fail "side-branch tag: the API was not asked about HEAD"
grep -q "commits/$(git -C "$R" rev-parse side)/" "$STUB/calls" && fail "side-branch tag: the side commit was asked about"
# ...and the labels still raise the bump from the reachable tag.
new_repo; tag 1.4.0
git -C "$R" checkout -q -b side; commit sidework; tag 3.0.0; git -C "$R" checkout -q main
commit one; label HEAD "semver:minor"; run_step ""
check "side-branch tag, minor label" 0 "version=1.5.0" 1
# A side-branch tag that is LOWER than the reachable one changes nothing either.
new_repo; commit a; tag 1.2.0
git -C "$R" checkout -q -b side HEAD~1; commit sidework; tag 1.1.0; git -C "$R" checkout -q main
commit one; run_step ""
check "lower side-branch tag" 0 "version=1.2.1" 1
# A reachable tag that is not strict X.Y.Z (a pre-release) is not the base.
new_repo; tag 1.0.0; commit one; tag 1.1.0-rc1; commit two; run_step ""
check "reachable pre-release tag" 0 "version=1.0.1" 2

echo PASS
