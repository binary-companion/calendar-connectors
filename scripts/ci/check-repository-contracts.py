#!/usr/bin/env python3
"""Library invariants that the compiler does not enforce. Usage: check-repository-contracts.py [root]"""
import json
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

PORTABLE = ('CalendarCore', 'CalendarOAuth', 'GoogleCalendar', 'MicrosoftCalendar', 'CalendarTestSupport')
FORBIDDEN = {'AppKit', 'SwiftUI', 'EventKit', 'WidgetKit', 'FoundationModels'}
for file in (root / 'Sources').rglob('*.swift'):
    if file.parts[-2] not in PORTABLE:
        continue
    for match in re.finditer(r'^\s*import\s+(\w+)', file.read_text(), re.M):
        require(match.group(1) not in FORBIDDEN,
                f'Apple/UI import in portable source: {file.relative_to(root)}')

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
for workflow in sorted((root / '.github' / 'workflows').glob('*.y*ml')):
    data = yaml.safe_load(workflow.read_text()) or {}
    triggers = data.get(True, data.get('on')) or {}
    names = triggers if isinstance(triggers, (list, dict)) else [triggers]
    require('pull_request_target' not in names, f'{workflow.name}: pull_request_target is banned')
    for job_id, job in (data.get('jobs') or {}).items():
        used = set(re.findall(r'secrets\.([A-Za-z0-9_]+)', json.dumps(job)))
        for secret in sorted(used - ALLOWED_ANYWHERE):
            require('environment' in job,
                    f'{workflow.name}: job {job_id} uses secret {secret} outside an environment')

if errors:
    for error in errors:
        print(f'FAIL: {error}', file=sys.stderr)
    sys.exit(1)
print('PASS: repository contracts')
