#!/usr/bin/env python3
"""Library invariants that the compiler does not enforce. Usage: check-repository-contracts.py [root]"""
import re
import sys
from pathlib import Path

import yaml

root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
errors = []

def require(condition, message):
    if not condition:
        errors.append(message)

def read(path):
    file = root / path
    require(file.is_file(), f'missing documented path: {path}')
    return file.read_text() if file.is_file() else ''

manifest = read('Package.swift')
for line in (
    '.target(name: "CalendarCore")',
    '.target(name: "CalendarOAuth", dependencies: ["CalendarCore"])',
    '.target(name: "GoogleCalendar", dependencies: ["CalendarCore", "CalendarOAuth"])',
    '.target(name: "MicrosoftCalendar", dependencies: ["CalendarCore", "CalendarOAuth"])',
):
    require(line in manifest, f'manifest dependency changed: {line}')

# The whole library must stay portable: no Apple-only or UI framework in any target under Sources.
FORBIDDEN = {'AppKit', 'SwiftUI', 'EventKit', 'WidgetKit', 'FoundationModels',
             'UIKit', 'Security', 'CryptoKit', 'Network', 'Combine'}
# "import X", "@testable import X", "public import X", "@_exported public import X", "import struct X.Y".
IMPORT = re.compile(
    r'^[ \t]*(?:(?:@\w+(?:\([^)]*\))?|public|package|internal|fileprivate|private)[ \t]+)*'
    r'import[ \t]+(?:(?:struct|class|enum|protocol|typealias|let|var|func)[ \t]+)?(\w+)', re.M)
for file in sorted((root / 'Sources').rglob('*.swift')):
    relative = file.relative_to(root / 'Sources')
    text = file.read_text()
    for match in IMPORT.finditer(text):
        require(match.group(1) not in FORBIDDEN,
                f'Apple/UI import in portable source ({relative.parts[0]}): '
                f'Sources/{relative}:{text.count(chr(10), 0, match.start()) + 1}: import {match.group(1)}')

def strip_swift_comments(text, trailing):
    """The Swift text without its comments, for the scope checks.

    Always removed: a "//" comment that starts its line (only whitespace before it) and "/* */"
    blocks (nested, as in Swift). With trailing=True a "//" comment after code on the same line is
    removed too; with trailing=False it is kept verbatim, so what sits in it still counts (the
    absence check fails closed). String literals are scanned whole, including "\\( ... )"
    interpolations that contain strings of their own, multi-line and raw strings, so a comment
    marker inside a string starts no comment. The rest of a kept trailing comment is copied without
    being scanned, so a "/*" in it opens nothing. Newlines survive."""
    n = len(text)

    def skip_string(i):
        """Index after the string literal that starts at text[i] (a '"' or a '#' run before one)."""
        hashes = 0
        while text.startswith('#', i + hashes):
            hashes += 1
        quote = i + hashes
        multiline = text.startswith('"""', quote)
        opener = quote + (3 if multiline else 1)
        closer = ('"""' if multiline else '"') + '#' * hashes
        j = opener
        while j < n:
            if text.startswith('\\' + '#' * hashes + '(', j):
                j = skip_parens(j + 1 + hashes)
            elif hashes == 0 and text[j] == '\\':
                j += 2
            elif text.startswith(closer, j):
                return j + len(closer)
            elif not multiline and text[j] == '\n':
                return j
            else:
                j += 1
        return n

    def skip_parens(i):
        """Index after the ")" that closes the "(" at text[i], scanning nested strings and parens."""
        depth, j = 0, i
        while j < n:
            if text[j] == '"':
                j = skip_string(j)
                continue
            if text[j] == '(':
                depth += 1
            elif text[j] == ')':
                depth -= 1
                if depth == 0:
                    return j + 1
            j += 1
        return n

    out, i, line_start = [], 0, True
    while i < n:
        two = text[i:i + 2]
        char = text[i]
        if two == '/*':
            depth, i = 1, i + 2
            while i < n and depth:
                if text.startswith('/*', i):
                    depth += 1; i += 2
                elif text.startswith('*/', i):
                    depth -= 1; i += 2
                else:
                    if text[i] == '\n':
                        out.append('\n'); line_start = True
                    i += 1
        elif two == '//':
            end = text.find('\n', i)
            end = n if end < 0 else end
            if line_start or trailing:
                pass  # a comment: drop it (the newline stays)
            else:
                out.append(text[i:end]); line_start = False
            i = end
        elif char == '"' or (char == '#' and re.match(r'#+"', text[i:i + 64])):
            end = skip_string(i)
            out.append(text[i:end]); i = end; line_start = False
        else:
            out.append(char)
            line_start = char == '\n' or (line_start and char.isspace())
            i += 1
    return ''.join(out)

# A scope that appears only in a comment is neither requested nor forbidden. The presence check
# reads code with every comment removed (a mis-strip can only make it fail). The absence check
# reads the text with only full-line comments and blocks removed, so it never trusts a trailing
# comment to hide a forbidden scope.
scope_text = read('Sources/MicrosoftCalendar/MicrosoftConnectorKind.swift')
scopes = strip_swift_comments(scope_text, trailing=True)
scopes_uncommented = strip_swift_comments(scope_text, trailing=False)
for scope in ('Calendars.Read', 'Calendars.Read.Shared', 'MailboxSettings.Read'):
    require(f'"{scope}"' in scopes, f'intended Microsoft scope absent: {scope}')
for scope in ('Calendars.ReadWrite', 'Calendars.ReadWrite.Shared'):
    require(f'"{scope}"' not in scopes_uncommented, f'Microsoft scope must stay read-only (ADR 0017 in timetug): {scope}')
source = read('Sources/MicrosoftCalendar/MicrosoftCalendarSource.swift')
require('canWrite: true' in source and 'canEditAttendees: true' in source, 'Microsoft write capabilities changed')
require('extension MicrosoftCalendarSource: WritableCalendarSource' in
        read('Sources/MicrosoftCalendar/MicrosoftCalendarSource+Write.swift'),
        'Microsoft write protocol implementation absent')
require('canWrite: true' in read('Sources/GoogleCalendar/GoogleCalendarSource.swift'), 'Google write capability changed')
require('extension GoogleCalendarSource: WritableCalendarSource' in
        read('Sources/GoogleCalendar/GoogleCalendarSource+Write.swift'), 'Google write protocol implementation absent')

# Workflows: untrusted code never meets secrets (program design, section 8).
ALLOWED_ANYWHERE = {'GITHUB_TOKEN', 'DOCKERHUB_USERNAME', 'DOCKERHUB_TOKEN'}
def expressions_in(text):
    """The bodies of the ${{ ... }} expressions in text. The scan is quote-aware: a "}}" inside a
    single-quoted literal (with '' as its escape) does not end the expression. It fails closed: an
    expression with no closing "}}" outside quotes runs to the end of the string."""
    bodies, i = [], text.find('${{')
    while i >= 0:
        start = j = i + 3
        quoted = False
        end = len(text)
        while j < len(text):
            if text[j] == "'":
                if quoted and text.startswith("''", j):
                    j += 2
                    continue
                quoted = not quoted
            elif not quoted and text.startswith('}}', j):
                end = j
                break
            j += 1
        bodies.append(text[start:end])
        i = text.find('${{', min(end + 2, len(text)))
    return bodies
# Context and secret names are case-insensitive in GitHub expressions: SECRETS.X and Secrets.x are the same.
SECRET_NAME = re.compile(r"(?<![\w.])secrets\s*(?:\.\s*([A-Za-z0-9_]+)|\[\s*['\"]([^'\"]+)['\"]\s*\])", re.I)
# The secrets context used whole (toJSON(secrets), secrets[expr], format('{0}', secrets)): every secret at once.
WHOLE_CONTEXT = re.compile(
    r"(?<![\w.'\"])secrets\b(?!\s*\.\s*\w|\s*\[\s*['\"][^'\"]+['\"]\s*\])", re.I)

def strings(node, key=None):
    """Every string in a parsed YAML document, paired with the mapping key it sits under."""
    if isinstance(node, str):
        yield key, node
    elif isinstance(node, dict):
        for k, v in node.items():
            yield from strings(k)
            yield from strings(v, k)
    elif isinstance(node, list):
        for item in node:
            yield from strings(item, key)

def secret_use(node):
    """(names of secrets referenced, whether the whole secrets context is used) anywhere in node."""
    names, whole = set(), False
    for key, text in strings(node):
        # Only expression code can read the secrets context: the bodies of ${{ ... }} and an `if:`
        # value (an expression with or without the braces). Prose such as "Never echo secrets. Mask
        # them" in a name or a script is not a reference.
        expressions = expressions_in(text) + ([text] if key == 'if' else [])
        for expression in expressions:
            for match in SECRET_NAME.finditer(expression):
                names.add((match.group(1) or match.group(2)).upper())
        whole = whole or any(WHOLE_CONTEXT.search(expression) for expression in expressions)
    return names, whole

def named_environment(job):
    environment = job.get('environment')
    if isinstance(environment, dict):
        environment = environment.get('name')
    return isinstance(environment, str) and bool(environment.strip())

def load(path):
    try:
        data = yaml.safe_load(path.read_text())
    except yaml.YAMLError as problem:
        require(False, f'{path.relative_to(root)}: not valid YAML: {problem}')
        return {}
    return data if isinstance(data, dict) else {}

# The documented workflows must exist: a deleted release or CI workflow must not pass unnoticed.
require((root / '.github' / 'workflows').is_dir(), 'missing documented path: .github/workflows')
if (root / '.github' / 'workflows').is_dir():
    for documented in ('ci.yml', 'release.yml'):
        require((root / '.github' / 'workflows' / documented).is_file(),
                f'missing documented path: .github/workflows/{documented}')

for workflow in sorted((root / '.github' / 'workflows').glob('*.y*ml')):
    data = load(workflow)
    name = workflow.name
    triggers = data.get(True, data.get('on')) or {}
    names = triggers if isinstance(triggers, (list, dict)) else [triggers]
    require('pull_request_target' not in names, f'{name}: pull_request_target is banned')
    jobs = data.get('jobs') if isinstance(data.get('jobs'), dict) else {}
    # Workflow-level keys (env, defaults, ...) apply to every job, so a secret there reaches all of them.
    used, whole = secret_use({key: value for key, value in data.items() if key != 'jobs'})
    for secret in sorted(used - ALLOWED_ANYWHERE):
        require(False, f'{name}: workflow-level use of secret {secret} (only {sorted(ALLOWED_ANYWHERE)} may be)')
    require(not whole, f'{name}: workflow-level use of the whole secrets context')
    for job_id, job in jobs.items():
        job = job if isinstance(job, dict) else {}
        require(job.get('secrets') != 'inherit', f'{name}: job {job_id} passes secrets: inherit')
        used, whole = secret_use(job)
        require(not whole, f'{name}: job {job_id} uses the whole secrets context (secrets[...], toJSON(secrets))')
        for secret in sorted(used - ALLOWED_ANYWHERE):
            require(named_environment(job),
                    f'{name}: job {job_id} uses secret {secret} outside an environment')

# Composite actions cannot read the secrets context directly: any use there is a mistake or a leak.
for action in sorted((root / '.github' / 'actions').glob('**/action.y*ml')):
    used, whole = secret_use(load(action))
    for secret in sorted(used):
        require(False, f'{action.relative_to(root)}: composite action uses secret {secret}')
    require(not whole, f'{action.relative_to(root)}: composite action uses the whole secrets context')

if errors:
    for error in errors:
        print(f'FAIL: {error}', file=sys.stderr)
    sys.exit(1)
print('PASS: repository contracts')
