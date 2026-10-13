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

scopes = read('Sources/MicrosoftCalendar/MicrosoftConnectorKind.swift')
for scope in ('Calendars.Read', 'Calendars.Read.Shared', 'MailboxSettings.Read'):
    require(f'"{scope}"' in scopes, f'intended Microsoft scope absent: {scope}')
for scope in ('Calendars.ReadWrite', 'Calendars.ReadWrite.Shared'):
    require(f'"{scope}"' not in scopes, f'Microsoft scope must stay read-only (ADR 0017 in timetug): {scope}')
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
EXPRESSION = re.compile(r'\$\{\{(.*?)\}\}', re.S)
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
        for match in SECRET_NAME.finditer(text):
            names.add((match.group(1) or match.group(2)).upper())
        expressions = EXPRESSION.findall(text) + ([text] if key == 'if' else [])
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
