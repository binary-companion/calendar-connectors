import subprocess, sys, tempfile, unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / 'check-repository-contracts.py'
GOOD_SCOPES = '''let scopes = ["Calendars.Read", "Calendars.Read.Shared", "MailboxSettings.Read"]\n'''
GOOD_SOURCE = 'canWrite: true, canEditAttendees: true\n'
CI = '''name: CI
on: [pull_request]
jobs:
  a:
    runs-on: ubuntu-latest
    steps:
      - run: echo "${{ secrets.GITHUB_TOKEN }}${{ secrets.DOCKERHUB_TOKEN }}"
'''

RELEASE = '''name: Release
on:
  push:
    branches: [main]
jobs:
  publish:
    runs-on: ubuntu-latest
    steps:
      - run: echo release
'''

ACTION = '''name: setup
runs:
  using: composite
  steps:
    - shell: bash
      run: echo hi
'''

def make_manifest():
    return ('.target(name: "CalendarCore")\n.target(name: "CalendarOAuth", dependencies: ["CalendarCore"])\n'
            '.target(name: "GoogleCalendar", dependencies: ["CalendarCore", "CalendarOAuth"])\n'
            '.target(name: "MicrosoftCalendar", dependencies: ["CalendarCore", "CalendarOAuth"])\n')

def make(root: Path, **overrides):
    files = {
        'Package.swift': make_manifest(),
        'Sources/CalendarCore/A.swift': 'import Foundation\n',
        'Sources/MicrosoftCalendar/MicrosoftConnectorKind.swift': GOOD_SCOPES,
        'Sources/MicrosoftCalendar/MicrosoftCalendarSource.swift': GOOD_SOURCE,
        'Sources/MicrosoftCalendar/MicrosoftCalendarSource+Write.swift': 'extension MicrosoftCalendarSource: WritableCalendarSource {}\n',
        'Sources/GoogleCalendar/GoogleCalendarSource.swift': 'canWrite: true\n',
        'Sources/GoogleCalendar/GoogleCalendarSource+Write.swift': 'extension GoogleCalendarSource: WritableCalendarSource {}\n',
        '.github/workflows/ci.yml': CI,
        '.github/workflows/release.yml': RELEASE,
    }
    files.update(overrides)
    for name, text in files.items():
        if text is None:  # an override of None leaves the file out
            continue
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

def run(root):
    return subprocess.run([sys.executable, str(SCRIPT), str(root)], capture_output=True, text=True)

class Contracts(unittest.TestCase):
    def check(self, expect_ok, needle=None, **overrides):
        with tempfile.TemporaryDirectory() as tmp:
            make(Path(tmp), **overrides)
            result = run(tmp)
        self.assertEqual(result.returncode == 0, expect_ok, result.stderr + result.stdout)
        if needle:
            self.assertIn(needle, result.stderr)

    def test_clean_fixture_passes(self):
        self.check(True)

    def test_apple_import_in_portable_source_fails(self):
        self.check(False, 'Apple/UI import', **{'Sources/CalendarCore/B.swift': 'import AppKit\n'})

    def test_microsoft_write_scope_fails(self):
        self.check(False, 'Calendars.ReadWrite', **{
            'Sources/MicrosoftCalendar/MicrosoftConnectorKind.swift': GOOD_SCOPES + '"Calendars.ReadWrite"\n'})

    def test_pull_request_target_fails(self):
        self.check(False, 'pull_request_target', **{
            '.github/workflows/ci.yml': CI.replace('[pull_request]', '[pull_request_target]')})

    def test_secret_outside_an_environment_fails(self):
        self.check(False, 'SIGNING_KEY', **{
            '.github/workflows/ci.yml': CI.replace('DOCKERHUB_TOKEN', 'SIGNING_KEY')})

    def test_secret_inside_an_environment_passes(self):
        self.check(True, **{
            '.github/workflows/ci.yml': CI.replace('    runs-on: ubuntu-latest', '    environment: publish\n    runs-on: ubuntu-latest')
                                            .replace('DOCKERHUB_TOKEN', 'SIGNING_KEY')})

    # --- GitHub context names are case-insensitive ---

    def test_uppercase_secrets_context_outside_an_environment_fails(self):
        self.check(False, 'SIGNING_KEY', **{
            '.github/workflows/ci.yml': CI.replace('secrets.DOCKERHUB_TOKEN', 'SECRETS.SIGNING_KEY')})

    def test_mixed_case_secrets_context_inside_an_environment_passes(self):
        self.check(True, **{
            '.github/workflows/ci.yml': CI.replace('    runs-on: ubuntu-latest', '    environment: publish\n    runs-on: ubuntu-latest')
                                            .replace('secrets.DOCKERHUB_TOKEN', 'Secrets.SIGNING_KEY')})

    def test_uppercase_bracket_syntax_secret_fails(self):
        self.check(False, 'SIGNING_KEY', **{
            '.github/workflows/ci.yml': CI.replace('secrets.DOCKERHUB_TOKEN', "SECRETS['SIGNING_KEY']")})

    def test_uppercase_whole_secrets_context_fails(self):
        self.check(False, 'whole secrets context', **{
            '.github/workflows/ci.yml': CI.replace('secrets.DOCKERHUB_TOKEN', 'toJSON(Secrets)')})

    def test_uppercase_whole_secrets_context_at_workflow_level_fails(self):
        self.check(False, 'workflow-level use of the whole secrets context', **{
            '.github/workflows/ci.yml': CI.replace('jobs:', 'env:\n  ALL: ${{ toJSON(SECRETS) }}\njobs:')})

    def test_uppercase_whole_secrets_context_in_an_if_fails(self):
        self.check(False, 'whole secrets context', **{
            '.github/workflows/ci.yml': CI.replace('    runs-on: ubuntu-latest',
                                                   "    if: contains(toJSON(SECRETS), 'x')\n    runs-on: ubuntu-latest")})

    def test_lowercase_spelling_of_an_allowed_secret_passes(self):
        self.check(True, **{
            '.github/workflows/ci.yml': CI.replace('secrets.DOCKERHUB_TOKEN', 'secrets.dockerhub_token')})

    def test_uppercase_secret_in_a_composite_action_fails(self):
        self.check(False, 'composite action uses secret SIGNING_KEY', **{
            '.github/actions/setup/action.yml': ACTION.replace('echo hi', 'echo ${{ SECRETS.SIGNING_KEY }}')})

    # --- the documented workflows must exist ---

    def test_missing_workflows_directory_fails(self):
        self.check(False, 'missing documented path: .github/workflows', **{
            '.github/workflows/ci.yml': None, '.github/workflows/release.yml': None})

    def test_missing_release_workflow_fails(self):
        self.check(False, 'missing documented path: .github/workflows/release.yml', **{
            '.github/workflows/release.yml': None})

    def test_missing_ci_workflow_fails(self):
        self.check(False, 'missing documented path: .github/workflows/ci.yml', **{
            '.github/workflows/ci.yml': None})

    # --- every remaining rule fails when its violation is introduced ---

    def test_changed_manifest_dependency_fails(self):
        self.check(False, 'manifest dependency changed: .target(name: "GoogleCalendar"', **{
            'Package.swift': make_manifest().replace(
                '.target(name: "GoogleCalendar", dependencies: ["CalendarCore", "CalendarOAuth"])',
                '.target(name: "GoogleCalendar", dependencies: ["CalendarCore"])')})

    def test_missing_manifest_fails(self):
        self.check(False, 'missing documented path: Package.swift', **{'Package.swift': None})

    def test_absent_intended_microsoft_scope_fails(self):
        self.check(False, 'intended Microsoft scope absent: Calendars.Read.Shared', **{
            'Sources/MicrosoftCalendar/MicrosoftConnectorKind.swift':
                GOOD_SCOPES.replace('"Calendars.Read.Shared", ', '')})

    def test_microsoft_shared_write_scope_fails(self):
        self.check(False, ': Calendars.ReadWrite.Shared', **{
            'Sources/MicrosoftCalendar/MicrosoftConnectorKind.swift': GOOD_SCOPES + '"Calendars.ReadWrite.Shared"\n'})

    def test_microsoft_without_can_write_fails(self):
        self.check(False, 'Microsoft write capabilities changed', **{
            'Sources/MicrosoftCalendar/MicrosoftCalendarSource.swift': 'canEditAttendees: true\n'})

    def test_microsoft_without_can_edit_attendees_fails(self):
        self.check(False, 'Microsoft write capabilities changed', **{
            'Sources/MicrosoftCalendar/MicrosoftCalendarSource.swift': 'canWrite: true\n'})

    def test_microsoft_write_extension_absent_fails(self):
        self.check(False, 'Microsoft write protocol implementation absent', **{
            'Sources/MicrosoftCalendar/MicrosoftCalendarSource+Write.swift': '// nothing\n'})

    def test_google_without_can_write_fails(self):
        self.check(False, 'Google write capability changed', **{
            'Sources/GoogleCalendar/GoogleCalendarSource.swift': 'canWrite: false\n'})

    def test_google_write_extension_absent_fails(self):
        self.check(False, 'Google write protocol implementation absent', **{
            'Sources/GoogleCalendar/GoogleCalendarSource+Write.swift': '// nothing\n'})

    def test_missing_documented_file_fails(self):
        self.check(False, 'missing documented path: Sources/GoogleCalendar/GoogleCalendarSource+Write.swift', **{
            'Sources/GoogleCalendar/GoogleCalendarSource+Write.swift': None})

    # --- portability: every spelling of an import, in every target and every nested directory ---

    def test_apple_import_in_a_nested_directory_fails(self):
        self.check(False, 'Sources/CalendarCore/Write/Probe.swift:2: import AppKit', **{
            'Sources/CalendarCore/Write/Probe.swift': 'import Foundation\nimport AppKit\n'})

    def test_apple_import_in_a_deeply_nested_directory_fails(self):
        self.check(False, 'Apple/UI import', **{'Sources/GoogleCalendar/A/B/C/Probe.swift': 'import SwiftUI\n'})

    def test_apple_import_in_a_target_outside_the_core_set_fails(self):
        self.check(False, 'Apple/UI import', **{'Sources/ICalendar/Probe.swift': 'import AppKit\n'})

    def test_every_forbidden_framework_fails(self):
        for framework in ('AppKit', 'SwiftUI', 'EventKit', 'WidgetKit', 'FoundationModels',
                          'UIKit', 'Security', 'CryptoKit', 'Network', 'Combine'):
            with self.subTest(framework=framework):
                self.check(False, f'import {framework}', **{
                    'Sources/CalDAVCalendar/Probe.swift': f'import {framework}\n'})

    def test_attribute_and_access_level_import_forms_fail(self):
        for statement in ('@preconcurrency import AppKit', '@testable import AppKit', 'public import AppKit',
                          'internal import AppKit', 'package import AppKit', 'fileprivate import AppKit',
                          'private import AppKit', '@_exported public import AppKit',
                          '@preconcurrency public import AppKit', '@available(macOS 14, *) import AppKit',
                          'import struct AppKit.NSColor', 'public import class AppKit.NSView',
                          '    import AppKit', '\timport AppKit'):
            with self.subTest(statement=statement):
                self.check(False, 'Apple/UI import', **{
                    'Sources/CalendarOAuth/Probe.swift': statement + '\n'})

    def test_allowed_import_forms_pass(self):
        self.check(True, **{'Sources/CalendarOAuth/Probe.swift':
                            '@_exported public import Foundation\nimport struct Foundation.Date\n'
                            '#if canImport(FoundationNetworking)\nimport FoundationNetworking\n#endif\n'})

    # --- workflow secrets ---

    def test_workflow_level_secret_fails(self):
        self.check(False, 'workflow-level use of secret SIGNING_KEY', **{
            '.github/workflows/ci.yml': CI.replace(
                'jobs:', 'env:\n  KEY: ${{ secrets.SIGNING_KEY }}\njobs:')})

    def test_workflow_level_whole_secrets_context_fails(self):
        self.check(False, 'workflow-level use of the whole secrets context', **{
            '.github/workflows/ci.yml': CI.replace(
                'jobs:', 'env:\n  ALL: ${{ toJSON(secrets) }}\njobs:')})

    def test_workflow_level_allowed_secret_passes(self):
        self.check(True, **{
            '.github/workflows/ci.yml': CI.replace(
                'jobs:', 'env:\n  TOKEN: ${{ secrets.GITHUB_TOKEN }}\njobs:')})

    def test_workflow_level_secret_is_not_rescued_by_a_job_environment(self):
        self.check(False, 'workflow-level use of secret SIGNING_KEY', **{
            '.github/workflows/ci.yml': CI.replace('jobs:', 'env:\n  KEY: ${{ secrets.SIGNING_KEY }}\njobs:')
                .replace('    runs-on: ubuntu-latest', '    environment: publish\n    runs-on: ubuntu-latest')})

    def test_inherit_secrets_fails(self):
        self.check(False, 'secrets: inherit', **{
            '.github/workflows/ci.yml': CI + '  b:\n    uses: ./.github/workflows/reusable.yml\n    secrets: inherit\n'})

    def test_inherit_secrets_fails_even_inside_an_environment(self):
        self.check(False, 'secrets: inherit', **{
            '.github/workflows/ci.yml': CI + '  b:\n    environment: publish\n'
                                             '    uses: ./.github/workflows/reusable.yml\n    secrets: inherit\n'})

    def test_bracket_syntax_secret_fails(self):
        self.check(False, 'SIGNING_KEY', **{
            '.github/workflows/ci.yml': CI.replace("secrets.DOCKERHUB_TOKEN", "secrets['SIGNING_KEY']")})

    def test_bracket_syntax_with_double_quotes_fails(self):
        self.check(False, 'SIGNING_KEY', **{
            '.github/workflows/ci.yml': CI.replace("secrets.DOCKERHUB_TOKEN", 'secrets["SIGNING_KEY"]')})

    def test_bracket_syntax_allowed_secret_passes(self):
        self.check(True, **{
            '.github/workflows/ci.yml': CI.replace("secrets.DOCKERHUB_TOKEN", "secrets['DOCKERHUB_TOKEN']")})

    def test_whole_secrets_context_fails(self):
        self.check(False, 'whole secrets context', **{
            '.github/workflows/ci.yml': CI.replace("secrets.DOCKERHUB_TOKEN", 'toJSON(secrets)')})

    def test_dynamic_secret_lookup_fails(self):
        self.check(False, 'whole secrets context', **{
            '.github/workflows/ci.yml': CI.replace("secrets.DOCKERHUB_TOKEN", 'secrets[format(\'{0}\', github.ref)]')})

    def test_whole_secrets_context_in_an_if_fails(self):
        self.check(False, 'whole secrets context', **{
            '.github/workflows/ci.yml': CI.replace('    runs-on: ubuntu-latest',
                                                   "    if: contains(toJSON(secrets), 'x')\n    runs-on: ubuntu-latest")})

    def test_the_word_secrets_in_a_shell_script_is_not_a_violation(self):
        self.check(True, **{
            '.github/workflows/ci.yml': CI + '      - run: echo "no secrets are used here"\n'})

    def test_empty_environment_fails(self):
        self.check(False, 'SIGNING_KEY', **{
            '.github/workflows/ci.yml': CI.replace('    runs-on: ubuntu-latest', "    environment: ''\n    runs-on: ubuntu-latest")
                                            .replace('DOCKERHUB_TOKEN', 'SIGNING_KEY')})

    def test_null_environment_fails(self):
        self.check(False, 'SIGNING_KEY', **{
            '.github/workflows/ci.yml': CI.replace('    runs-on: ubuntu-latest', '    environment:\n    runs-on: ubuntu-latest')
                                            .replace('DOCKERHUB_TOKEN', 'SIGNING_KEY')})

    def test_environment_mapping_without_a_name_fails(self):
        self.check(False, 'SIGNING_KEY', **{
            '.github/workflows/ci.yml': CI.replace('    runs-on: ubuntu-latest',
                                                   '    environment:\n      url: https://example.com\n    runs-on: ubuntu-latest')
                                            .replace('DOCKERHUB_TOKEN', 'SIGNING_KEY')})

    def test_environment_mapping_with_a_name_passes(self):
        self.check(True, **{
            '.github/workflows/ci.yml': CI.replace('    runs-on: ubuntu-latest',
                                                   '    environment:\n      name: publish\n    runs-on: ubuntu-latest')
                                            .replace('DOCKERHUB_TOKEN', 'SIGNING_KEY')})

    def test_composite_action_using_a_secret_fails(self):
        self.check(False, 'composite action uses secret GITHUB_TOKEN', **{
            '.github/actions/setup/action.yml': ACTION.replace('echo hi', 'echo ${{ secrets.GITHUB_TOKEN }}')})

    def test_composite_action_using_the_whole_context_fails(self):
        self.check(False, 'composite action uses the whole secrets context', **{
            '.github/actions/setup/action.yml': ACTION.replace('echo hi', 'echo ${{ toJSON(secrets) }}')})

    def test_clean_composite_action_passes(self):
        self.check(True, **{'.github/actions/setup/action.yml': ACTION})

    def test_secret_in_a_nested_composite_action_fails(self):
        self.check(False, 'composite action uses secret SIGNING_KEY', **{
            '.github/actions/a/b/action.yaml': ACTION.replace('echo hi', "echo ${{ secrets['SIGNING_KEY'] }}")})

if __name__ == '__main__':
    unittest.main()
