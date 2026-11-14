import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / 'check-test-parity.py'

BASELINE = ['CoreTests/AllDayTests.swift::dateOfInstant', 'CoreTests/AllDayTests.swift::startOfDay', 'OAuthTests/SHA256Tests.swift::emptyInput']
RUST_FILE = 'crates/calendar-core/src/tests/all_day_tests.rs'
RUST_SOURCE = 'fn date_of_instant() {}\n'
HEADER = '<!-- generated -->\n| Swift test | Rust test | Status |\n|---|---|---|\n'
OK_ROWS = [
    f'| {BASELINE[0]} | {RUST_FILE}::date_of_instant | ported |',
    f'| {BASELINE[1]} | - | pending |',
    f'| {BASELINE[2]} | - | n/a: sha2 is tested upstream |',
]


def run(rows, baseline=BASELINE, rust_source=RUST_SOURCE, with_file=True, args=()):
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        (root / 'docs/rust-port').mkdir(parents=True)
        (root / 'docs/rust-port/swift-tests-baseline.txt').write_text('\n'.join(baseline) + '\n')
        (root / 'docs/rust-port/test-parity.md').write_text(HEADER + '\n'.join(rows) + '\n')
        if with_file:
            (root / RUST_FILE).parent.mkdir(parents=True)
            (root / RUST_FILE).write_text(rust_source)
        result = subprocess.run([sys.executable, str(SCRIPT), '--root', directory, *args], capture_output=True, text=True)
    return result.returncode, result.stdout + result.stderr


class ParityTests(unittest.TestCase):
    def test_a_complete_table_passes(self):
        code, output = run(OK_ROWS)
        self.assertEqual(code, 0, output)

    def test_a_baseline_line_without_a_row_fails(self):
        code, output = run(OK_ROWS[:2])
        self.assertEqual(code, 1, output)
        self.assertIn(BASELINE[2], output)

    def test_a_row_for_a_test_not_in_the_baseline_fails(self):
        code, output = run(OK_ROWS + ['| CoreTests/Other.swift::ghost | - | pending |'])
        self.assertEqual(code, 1, output)
        self.assertIn('ghost', output)

    def test_a_ported_row_naming_a_missing_file_fails(self):
        code, output = run(OK_ROWS, with_file=False)
        self.assertEqual(code, 1, output)
        self.assertIn('does not exist', output)

    def test_a_ported_row_naming_a_missing_fn_fails(self):
        code, output = run(OK_ROWS, rust_source='fn something_else() {}\n')
        self.assertEqual(code, 1, output)
        self.assertIn('fn date_of_instant(', output)

    def test_na_without_a_reason_fails(self):
        for status in ('n/a', 'n/a:', 'n/a:   '):
            rows = OK_ROWS[:2] + [f'| {BASELINE[2]} | - | {status} |']
            code, output = run(rows)
            self.assertEqual(code, 1, f'{status!r}: {output}')

    def test_an_unknown_status_fails(self):
        code, output = run(OK_ROWS[:2] + [f'| {BASELINE[2]} | - | done |'])
        self.assertEqual(code, 1, output)

    def test_require_complete_fails_on_a_pending_row_of_that_target(self):
        code, output = run(OK_ROWS, args=['--require-complete', 'CoreTests'])
        self.assertEqual(code, 1, output)
        self.assertIn('startOfDay', output)

    def test_require_complete_ignores_pending_rows_of_other_targets(self):
        code, output = run(OK_ROWS, args=['--require-complete', 'OAuthTests'])
        self.assertEqual(code, 0, output)

    def test_require_complete_passes_when_all_rows_are_ported_or_na(self):
        rows = [OK_ROWS[0], f'| {BASELINE[1]} | - | n/a: removed in Rust |', OK_ROWS[2]]
        code, output = run(rows, args=['--require-complete', 'CoreTests', '--require-complete', 'OAuthTests'])
        self.assertEqual(code, 0, output)

    def test_require_complete_fails_for_a_target_without_rows(self):
        code, output = run(OK_ROWS, args=['--require-complete', 'NoSuchTests'])
        self.assertEqual(code, 1, output)


if __name__ == '__main__':
    unittest.main()
