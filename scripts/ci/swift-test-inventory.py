#!/usr/bin/env python3
"""Lists every Swift Testing test under a directory, one sorted line per test.
Usage: swift-test-inventory.py TESTS_DIR

Each line is `<Target>/<relative path>.swift::<funcName>`, the path relative to TESTS_DIR. A test is a `func` that
follows an `@Test` attribute, with or without arguments (which may span lines) or a display name, in any suite,
nested suite or extension. Comments and string literals are skipped, so an `@Test` inside either is not counted.
Rust port conventions section 8 and the baseline in docs/rust-port/swift-tests-baseline.txt use this format."""
import argparse
import re
import sys
from pathlib import Path

IDENT = re.compile(r'[A-Za-z_][A-Za-z0-9_]*')
BACKTICKED = re.compile(r'`([^`\n]+)`')


def skip_string(text, i):
    """`i` is at the opening quote (after any leading `#`s were counted by the caller). Returns the index after the string."""
    hashes = 0
    j = i
    while j > 0 and text[j - 1] == '#':
        hashes += 1
        j -= 1
    multi = text.startswith('"""', i)
    quote = '"""' if multi else '"'
    closing = quote + '#' * hashes
    i += len(quote)
    while i < len(text):
        if text[i] == '\\' and text.startswith('\\' + '#' * hashes, i):
            i += 1 + hashes
            if i < len(text) and text[i] == '(':
                i = skip_parens(text, i)
            else:
                i += 1
        elif text.startswith(closing, i):
            return i + len(closing)
        elif not multi and text[i] == '\n':
            return i
        else:
            i += 1
    return i


def skip_block_comment(text, i):
    depth = 0
    while i < len(text):
        if text.startswith('/*', i):
            depth += 1
            i += 2
        elif text.startswith('*/', i):
            depth -= 1
            i += 2
            if depth == 0:
                return i
        else:
            i += 1
    return i


def skip_parens(text, i):
    """`i` is at `(`. Returns the index after the matching `)`, skipping strings and comments."""
    depth = 0
    while i < len(text):
        c = text[i]
        if c == '(':
            depth += 1
        elif c == ')':
            depth -= 1
            if depth == 0:
                return i + 1
        elif c == '"':
            i = skip_string(text, i)
            continue
        elif text.startswith('//', i):
            i = text.find('\n', i)
            if i < 0:
                return len(text)
            continue
        elif text.startswith('/*', i):
            i = skip_block_comment(text, i)
            continue
        i += 1
    return i


def test_names(text):
    names = []
    pending = False
    i = 0
    while i < len(text):
        c = text[i]
        if text.startswith('//', i):
            i = text.find('\n', i)
            if i < 0:
                break
        elif text.startswith('/*', i):
            i = skip_block_comment(text, i)
        elif c == '#' and text.startswith('#"', i) or c == '#' and text.startswith('##', i):
            j = i
            while j < len(text) and text[j] == '#':
                j += 1
            i = skip_string(text, j) if j < len(text) and text[j] == '"' else j
        elif c == '"':
            i = skip_string(text, i)
        elif c == '@':
            match = IDENT.match(text, i + 1)
            i = match.end() if match else i + 1
            if match and match.group(0) == 'Test':
                pending = True
                j = i
                while j < len(text) and text[j] in ' \t\r\n':
                    j += 1
                if j < len(text) and text[j] == '(':
                    i = skip_parens(text, j)
        elif c == '`':
            match = BACKTICKED.match(text, i)
            word = match.group(1) if match else None
            i = match.end() if match else i + 1
            if pending and word and names and names[-1] is None:
                names[-1] = word
                pending = False
        else:
            match = IDENT.match(text, i)
            if not match:
                i += 1
                continue
            i = match.end()
            if pending and match.group(0) == 'func':
                j = i
                while j < len(text) and text[j] in ' \t\r\n':
                    j += 1
                name = BACKTICKED.match(text, j) or IDENT.match(text, j)
                if name:
                    names.append(name.group(1) if name.re is BACKTICKED else name.group(0))
                    i = name.end()
                pending = False
    return [name for name in names if name]


def inventory(root):
    root = Path(root)
    lines = []
    for path in sorted(root.rglob('*.swift')):
        text = path.read_text(encoding='utf-8', errors='replace')
        relative = path.relative_to(root).as_posix()
        lines.extend(f'{relative}::{name}' for name in test_names(text))
    return sorted(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('tests_dir')
    args = parser.parse_args()
    if not Path(args.tests_dir).is_dir():
        print(f'swift-test-inventory: {args.tests_dir} is not a directory', file=sys.stderr)
        return 1
    for line in inventory(args.tests_dir):
        print(line)
    return 0


if __name__ == '__main__':
    sys.exit(main())
