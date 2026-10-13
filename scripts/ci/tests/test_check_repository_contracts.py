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

def make(root: Path, **overrides):
    files = {
        'Package.swift': '.target(name: "CalendarCore")\n.target(name: "CalendarOAuth", dependencies: ["CalendarCore"])\n'
                         '.target(name: "GoogleCalendar", dependencies: ["CalendarCore", "CalendarOAuth"])\n'
                         '.target(name: "MicrosoftCalendar", dependencies: ["CalendarCore", "CalendarOAuth"])\n',
        'Sources/CalendarCore/A.swift': 'import Foundation\n',
        'Sources/MicrosoftCalendar/MicrosoftConnectorKind.swift': GOOD_SCOPES,
        'Sources/MicrosoftCalendar/MicrosoftCalendarSource.swift': GOOD_SOURCE,
        'Sources/MicrosoftCalendar/MicrosoftCalendarSource+Write.swift': 'extension MicrosoftCalendarSource: WritableCalendarSource {}\n',
        'Sources/GoogleCalendar/GoogleCalendarSource.swift': 'canWrite: true\n',
        'Sources/GoogleCalendar/GoogleCalendarSource+Write.swift': 'extension GoogleCalendarSource: WritableCalendarSource {}\n',
        '.github/workflows/ci.yml': CI,
    }
    files.update(overrides)
    for name, text in files.items():
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

if __name__ == '__main__':
    unittest.main()
