#!/usr/bin/env python3
"""Checks the Swift-to-Rust test parity table (Rust port conventions section 8).
Usage: check-test-parity.py [--require-complete TARGET]... [--root DIR]

Reads docs/rust-port/swift-tests-baseline.txt (one `<Target>/<File>.swift::<funcName>` line per Swift test) and
docs/rust-port/test-parity.md (one row per baseline line: `| Swift test | Rust test | Status |`). Exits 1 when

  * a baseline line has no row, a row names a test that is not in the baseline, or a test has two rows;
  * a status is not `pending`, `ported` or `n/a: <reason>` (the reason after the colon must not be empty);
  * a `ported` row's Rust test is not `<path>::<fn_name>` with the file present under the root and containing
    `fn <fn_name>(`;
  * with --require-complete TARGET, any row of that target (Swift test starting `TARGET/`) is still `pending`,
    or the target has no rows at all."""
import argparse
import re
import sys
from pathlib import Path

HEADER = ('Swift test', 'Rust test', 'Status')


def read_baseline(path):
    return [line.strip() for line in path.read_text(encoding='utf-8').splitlines() if line.strip()]


def read_rows(path):
    """Returns (rows, problems); a row is (line number, swift, rust, status)."""
    rows, problems = [], []
    for number, line in enumerate(path.read_text(encoding='utf-8').splitlines(), start=1):
        text = line.strip()
        if not text.startswith('|'):
            continue
        cells = [cell.strip() for cell in text.strip('|').split('|', 2)]
        if len(cells) != 3:
            problems.append(f'{path.name}:{number}: a row needs three cells: {line!r}')
            continue
        if tuple(cells) == HEADER or all(re.fullmatch(r':?-+:?', cell) for cell in cells):
            continue
        rows.append((number, *cells))
    return rows, problems


def status_problem(status):
    if status in ('pending', 'ported'):
        return None
    if status.startswith('n/a:'):
        return None if status[4:].strip() else 'n/a needs a reason after the colon'
    return f'status {status!r} is not pending, ported or "n/a: <reason>"'


def rust_problem(root, rust):
    path, separator, name = rust.rpartition('::')
    if not separator or not path or not name:
        return f'Rust test {rust!r} is not <path>::<fn_name>'
    file = root / path
    if not file.is_file():
        return f'Rust file {path} does not exist'
    if f'fn {name}(' not in file.read_text(encoding='utf-8'):
        return f'{path} has no `fn {name}(`'
    return None


def check(root, require_complete):
    baseline_path = root / 'docs/rust-port/swift-tests-baseline.txt'
    parity_path = root / 'docs/rust-port/test-parity.md'
    problems = []
    for path in (baseline_path, parity_path):
        if not path.is_file():
            problems.append(f'{path} does not exist')
    if problems:
        return problems
    baseline = read_baseline(baseline_path)
    rows, problems = read_rows(parity_path)
    known = set(baseline)
    seen = {}
    for number, swift, rust, status in rows:
        where = f'test-parity.md:{number}: {swift}'
        if swift not in known:
            problems.append(f'{where}: not in the baseline')
        if swift in seen:
            problems.append(f'{where}: a second row (first at line {seen[swift]})')
        seen.setdefault(swift, number)
        problem = status_problem(status)
        if problem:
            problems.append(f'{where}: {problem}')
        elif status == 'ported':
            problem = rust_problem(root, rust)
            if problem:
                problems.append(f'{where}: {problem}')
    for line in baseline:
        if line not in seen:
            problems.append(f'{line}: in the baseline but has no row in test-parity.md')
    for target in require_complete:
        target_rows = [row for row in rows if row[1].startswith(target + '/')]
        if not target_rows:
            problems.append(f'--require-complete {target}: no row for that target')
        for number, swift, _rust, status in target_rows:
            if status == 'pending':
                problems.append(f'test-parity.md:{number}: {swift} is still pending ({target} must be complete)')
    return problems


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--require-complete', action='append', default=[], metavar='TARGET')
    parser.add_argument('--root', default=str(Path(__file__).resolve().parents[2]))
    args = parser.parse_args()
    problems = check(Path(args.root), args.require_complete)
    for problem in problems:
        print(f'error: {problem}', file=sys.stderr)
    if not problems:
        print('test parity: ok')
    return 1 if problems else 0


if __name__ == '__main__':
    sys.exit(main())
