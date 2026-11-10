#!/usr/bin/env python3
"""Fails when the dependency tree enables a feature of tokio outside the allowed set.
Usage: check-tokio-features.py [--tree FILE]

Design section 6.2: tokio is used with only rt, time, sync and macros (a current-thread runtime), never net, fs,
process or rt-multi-thread. Reads the output of `cargo tree -e features,normal,build --prefix none -i tokio` (run
here, or FILE) and exits 1, naming every tokio feature that is not in ALLOWED_FEATURES. Dev-dependencies are not
read: tokio's `test-util` is allowed there. A tree without tokio passes."""
import argparse
import re
import subprocess
import sys

# The four features of the design, plus the internal feature the pinned tokio (1.54.1) enables for them: in its
# Cargo.toml, macros = ["tokio-macros"] (the optional dependency, which `cargo tree` lists as a feature node); rt,
# time and sync enable no other feature of tokio. Re-read tokio's Cargo.toml when the pin is raised.
ALLOWED_FEATURES = {'rt', 'time', 'sync', 'macros', 'tokio-macros'}
FEATURE_LINE = re.compile(r'(?:^|[\s─-╿])tokio feature "([^"]+)"')


def features_in(tree):
    return {match.group(1) for line in tree.splitlines() for match in [FEATURE_LINE.search(line)] if match}


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--tree', help='a saved `cargo tree` output')
    args = parser.parse_args()
    try:
        if args.tree:
            with open(args.tree, encoding='utf-8') as handle:
                tree = handle.read()
        else:
            result = subprocess.run(['cargo', 'tree', '-e', 'features,normal,build', '--prefix', 'none', '-i', 'tokio', '--locked'],
                                    capture_output=True, text=True)
            if result.returncode != 0 and 'did not match any packages' in result.stderr:
                tree = ''
            elif result.returncode != 0:
                print(f'check-tokio-features: cargo tree failed: {result.stderr.strip()}', file=sys.stderr)
                return 1
            else:
                tree = result.stdout
    except OSError as error:
        print(f'check-tokio-features: cannot read the dependency tree: {error}', file=sys.stderr)
        return 1
    extra = sorted(features_in(tree) - ALLOWED_FEATURES)
    for feature in extra:
        print(f'check-tokio-features: tokio feature "{feature}" is enabled; allowed: {", ".join(sorted(ALLOWED_FEATURES))}', file=sys.stderr)
    return 1 if extra else 0


if __name__ == '__main__':
    sys.exit(main())
