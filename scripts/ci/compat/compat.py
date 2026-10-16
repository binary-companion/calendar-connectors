#!/usr/bin/env python3
"""Backward-compatibility checks of the library against the latest release (AGENTS.md, "Backward compatibility").
Within a major version a release must drop into an existing front end with no change there, so a pull request may add
but not remove, retype or change; that needs the label semver:major.

A copy of scripts/ci/compat/compat.py in binary-companion/timetug-shared, without its C header, export and NuGet
commands (this library has none of those). The repositories do not depend on each other: change both on purpose.

Usage (from the repository root; the CI job `compat` runs these):
  compat.py gate [--base-ref REF]            decide whether the checks run: writes run=true|false and baseline=TAG
                                             to $GITHUB_OUTPUT (pull requests only; skipped with a notice for
                                             semver:major, other events and when no X.Y.Z tag exists)
  compat.py baseline [--ref REF]             print the highest X.Y.Z tag reachable from REF (default origin/main)
  compat.py frozen BASELINE [--strict PATH...] [--additive PATH...] [--appendable PATH...]
                                             fail if a file under PATH that exists in BASELINE was deleted or
                                             changed in a way its kind does not allow:
                                               --strict      no change at all (persisted-data fixtures)
                                               --additive    the JSON may only gain object members; arrays keep
                                                             their length
                                               --appendable  as --additive, and arrays may also gain entries at
                                                             the end
                                             New files are always allowed.

The pull request's labels come from the event payload at $GITHUB_EVENT_PATH; the event name from $GITHUB_EVENT_NAME.
Exit status: 0 pass or skip, 1 a compatibility break, 2 a usage or environment error."""
import json
import os
import re
import subprocess
import sys

MAJOR_LABEL = 'semver:major'
RELEASE_TAG = re.compile(r'^(\d+)\.(\d+)\.(\d+)$')


def notice(message):
    print(f'::notice::{message}')


def error(message):
    print(f'::error::{message}')


def git(*args):
    return subprocess.run(['git', *args], check=True, capture_output=True, text=True).stdout


def highest_release(names):
    """The highest strict X.Y.Z among names, or None."""
    releases = [tuple(int(part) for part in match.groups()) for match in map(RELEASE_TAG.match, names) if match]
    return '.'.join(map(str, max(releases))) if releases else None


def baseline_tag(ref):
    return highest_release(git('tag', '--merged', ref, '--sort=-v:refname').split())


def labels(event):
    pull = event.get('pull_request') if isinstance(event, dict) else None
    found = pull.get('labels') if isinstance(pull, dict) else None
    return [label.get('name') for label in found or [] if isinstance(label, dict)]


def decide(event_name, event, baseline):
    """(run, reason): whether the compatibility checks apply to this run."""
    if event_name != 'pull_request':
        return False, f'compatibility checks run on pull requests only (event: {event_name or "none"})'
    if MAJOR_LABEL in labels(event):
        return False, f'the pull request is labelled {MAJOR_LABEL}: breaking changes are allowed, checks skipped'
    if not baseline:
        return False, 'no X.Y.Z release tag is reachable from the base branch (first release): nothing to compare with'
    return True, f'comparing with release {baseline}'


def load_event(path):
    if not path:
        return {}
    try:
        with open(path, encoding='utf-8') as handle:
            return json.load(handle)
    except (OSError, ValueError) as problem:
        raise SystemExit(f'compat: cannot read the event payload {path}: {problem}')


def write_outputs(**values):
    target = os.environ.get('GITHUB_OUTPUT')
    lines = ''.join(f'{key}={value}\n' for key, value in values.items())
    if target:
        with open(target, 'a', encoding='utf-8') as handle:
            handle.write(lines)
    else:
        sys.stdout.write(lines)


def changed_baseline_files(name_status):
    """The lines of `git diff --name-status --no-renames` that are not additions."""
    return [line for line in name_status.splitlines() if line.strip() and not line.startswith('A\t')]


def covers(current, baseline, append=False):
    """True when every member of the baseline JSON value is in current with an equal value, recursively: an object
    may gain members; an array keeps its length (or, with append, may gain entries at the end) and covers the
    baseline's element by element; any other value is equal and of the same JSON type."""
    if isinstance(baseline, dict):
        return isinstance(current, dict) and all(
            key in current and covers(current[key], value, append) for key, value in baseline.items())
    if isinstance(baseline, list):
        if not isinstance(current, list) or len(current) < len(baseline) or (not append and len(current) != len(baseline)):
            return False
        return all(covers(mine, theirs, append) for mine, theirs in zip(current, baseline))
    if isinstance(baseline, bool) or isinstance(current, bool):
        return isinstance(baseline, bool) and isinstance(current, bool) and current == baseline
    if isinstance(baseline, (int, float)):
        return isinstance(current, (int, float)) and current == baseline
    return type(current) is type(baseline) and current == baseline


def additive_change(baseline_text, current_text, append):
    """None when current is an allowed additive change of baseline, else why not."""
    try:
        old, new = json.loads(baseline_text), json.loads(current_text)
    except ValueError:
        return 'modified (not JSON, so no change is allowed)'
    if covers(new, old, append):
        return None
    return 'modified: a field or entry of the baseline was removed or changed (only additions are allowed)'


def frozen_problems(baseline, paths, mode):
    """[(path, reason)] for the files under paths that exist in baseline and changed in a way mode does not allow."""
    if not paths:
        return []
    problems = []
    for line in changed_baseline_files(git('diff', '--name-status', '--no-renames', baseline, 'HEAD', '--', *paths)):
        status, _, path = line.partition('\t')
        if status == 'D':
            problems.append((path, 'deleted'))
        elif mode == 'strict':
            problems.append((path, 'modified (files here model data on users\' disks: existing ones never change)'))
        else:
            why = additive_change(git('show', f'{baseline}:{path}'), git('show', f'HEAD:{path}'), mode == 'appendable')
            if why:
                problems.append((path, why))
            else:
                print(f'{path}: additive change since {baseline} (allowed)')
    return problems


def parse_frozen(args):
    """(baseline, {mode: [paths]}) from `BASELINE [--strict P...] [--additive P...] [--appendable P...]`."""
    if not args or args[0].startswith('--'):
        return None, {}
    modes, mode = {'strict': [], 'additive': [], 'appendable': []}, 'strict'
    for arg in args[1:]:
        if arg.startswith('--'):
            mode = arg[2:]
            if mode not in modes:
                return None, {}
        else:
            modes[mode].append(arg)
    return args[0], modes


def main(argv):
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    command, args = argv[0], argv[1:]
    if command == 'gate':
        ref = args[args.index('--base-ref') + 1] if '--base-ref' in args else 'origin/main'
        event_name = os.environ.get('GITHUB_EVENT_NAME', '')
        event = load_event(os.environ.get('GITHUB_EVENT_PATH'))
        baseline = baseline_tag(ref) if event_name == 'pull_request' else None
        run, reason = decide(event_name, event, baseline)
        if run:
            print(reason)
        else:
            notice(reason)
        write_outputs(run='true' if run else 'false', baseline=baseline or '')
        return 0
    if command == 'baseline':
        ref = args[args.index('--ref') + 1] if '--ref' in args else 'origin/main'
        print(baseline_tag(ref) or '')
        return 0
    if command == 'frozen':
        baseline, modes = parse_frozen(args)
        if not baseline or not any(modes.values()):
            print(__doc__, file=sys.stderr)
            return 2
        problems = [problem for mode, paths in modes.items() for problem in frozen_problems(baseline, paths, mode)]
        for path, why in problems:
            error(f'{path}: {why} since {baseline}; label the pull request {MAJOR_LABEL} for a breaking change')
        if not problems:
            print(f'baseline files intact since {baseline}: {" ".join(p for paths in modes.values() for p in paths)}')
        return 1 if problems else 0
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
