#!/usr/bin/env python3
"""Counts the tests that a `swift test` log says were executed and fails when too few ran or any failed.
Usage: count-swift-tests.py LOG --minimum N [--minimum-runs R] [--minimum-per-run P]

The Windows jobs use it because a green job must mean "the tests ran": a skipped target or an
unexpected zero would otherwise pass. It counts

  * Swift Testing: every `Test run with N tests ... passed|failed` summary line (one per test target);
  * XCTest: the `Executed N tests, with M failures` line under `Test Suite 'All tests'`, once per
    bundle (the same number is repeated under the .xctest suite, which is not counted again).

--minimum-runs R also requires R Swift Testing summaries (one per test target), so a target that
vanishes is noticed even when the remaining ones keep the total above the minimum.

--minimum-per-run P (default 1) requires every Swift Testing summary to report at least P tests, so a
test target that still builds but holds no tests (`Test run with 0 tests ... passed`) is red even when
the total stays above the floor. P 0 turns the check off.

It exits 1 when the total is zero or below the minimum, when a summary reports a failure, or when
the log cannot be read."""
import argparse
import re
import sys

SWIFT_TESTING = re.compile(r'Test run with (\d+) tests?\b[^\r\n]*?\b(passed|failed)\b')
XCTEST = re.compile(r"Test Suite 'All tests' (passed|failed) at [^\r\n]*\s+"
                    r"Executed (\d+) tests?, with (\d+) failures?")

parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
parser.add_argument('log')
parser.add_argument('--minimum', type=int, required=True)
parser.add_argument('--minimum-runs', type=int, default=0)
parser.add_argument('--minimum-per-run', type=int, default=1)
args = parser.parse_args()

try:
    with open(args.log, 'rb') as handle:
        text = handle.read().decode('utf-8-sig', errors='replace')
except OSError as problem:
    print(f'cannot read {args.log}: {problem}', file=sys.stderr)
    sys.exit(1)

swift_testing = xctest = runs = 0
problems = []
summaries = list(SWIFT_TESTING.finditer(text))
for number, summary in enumerate(summaries, start=1):
    count, status = summary.groups()
    swift_testing += int(count)
    runs += 1
    if status == 'failed':
        problems.append(f'a Swift Testing run of {count} tests failed')
    if int(count) < args.minimum_per_run:
        problems.append(f'Swift Testing run {number} of {len(summaries)} ran too few tests '
                        f'({int(count)} < {args.minimum_per_run}): "{summary.group(0)}" '
                        '(a test target holds no tests)')
for status, count, failures in XCTEST.findall(text):
    xctest += int(count)
    if status == 'failed' or int(failures) > 0:
        problems.append(f'an XCTest run of {count} tests reported {failures} failures')

total = swift_testing + xctest
print(f'Executed tests: {total} (Swift Testing {swift_testing} in {runs} runs, XCTest {xctest}); '
      f'floor {args.minimum}')
if total == 0:
    problems.append('no test was executed (no summary line found, or all of them say 0)')
elif total < args.minimum:
    problems.append(f'fewer tests ran than the floor: {total} < {args.minimum}')
if runs < args.minimum_runs:
    problems.append(f'only {runs} Swift Testing run(s) reported, expected at least {args.minimum_runs} '
                    '(a test target did not run)')
for problem in problems:
    print(f'error: {problem}', file=sys.stderr)
sys.exit(1 if problems else 0)
