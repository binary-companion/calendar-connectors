import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / 'check-crate-architecture.py'


def package(name, deps=()):
    """A `cargo metadata` package entry. deps are (name, kind) pairs; kind None is a normal dependency."""
    return {
        'name': name,
        'id': f'path+file:///work/crates/{name}#0.0.0',
        'source': None,
        'dependencies': [
            {'name': dep, 'kind': kind, 'source': None if dep.startswith('calendar-') else 'registry+https://github.com/rust-lang/crates.io-index'}
            for dep, kind in deps
        ],
    }


# The workspace of design section 6.1 with every internal edge the table allows, plus external crates on the list.
CLEAN = [
    package('calendar-core', [('serde', None), ('serde_json', None), ('jiff', None), ('thiserror', None)]),
    package('calendar-oauth', [('calendar-core', None), ('sha2', None), ('base64', None), ('serde_json', 'dev')]),
    package('calendar-ical', [('calendar-core', None)]),
    package('calendar-google', [('calendar-core', None), ('calendar-oauth', None), ('calendar-test-support', 'dev')]),
    package('calendar-microsoft', [('calendar-core', None), ('calendar-oauth', None)]),
    package('calendar-caldav', [('calendar-core', None), ('calendar-ical', None), ('quick-xml', None)]),
    package('calendar-ical-subscription', [('calendar-core', None), ('calendar-ical', None)]),
    package('calendar-test-support', [('calendar-core', None), ('async-trait', None)]),
]


def run(packages):
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / 'metadata.json'
        path.write_text(json.dumps({'packages': packages, 'workspace_members': [p['id'] for p in packages]}))
        result = subprocess.run([sys.executable, str(SCRIPT), '--metadata', str(path)], capture_output=True, text=True)
    return result.returncode, result.stdout + result.stderr


def replaced(name, **changes):
    return [dict(p, **changes) if p['name'] == name else p for p in CLEAN]


class CrateArchitectureTests(unittest.TestCase):
    def test_clean_workspace_passes(self):
        code, output = run(CLEAN)
        self.assertEqual(code, 0, output)

    def test_core_depending_on_ical_fails_naming_the_edge(self):
        packages = replaced('calendar-core', dependencies=package('x', [('calendar-ical', None)])['dependencies'])
        code, output = run(packages)
        self.assertEqual(code, 1, output)
        self.assertIn('calendar-core -> calendar-ical', output)

    def test_external_crate_off_the_list_fails(self):
        packages = replaced('calendar-google', dependencies=package('x', [('calendar-core', None), ('reqwest', None)])['dependencies'])
        code, output = run(packages)
        self.assertEqual(code, 1, output)
        self.assertIn('reqwest', output)

    def test_test_support_is_allowed_only_as_a_dev_dependency(self):
        packages = replaced('calendar-ical', dependencies=package('x', [('calendar-core', None), ('calendar-test-support', None)])['dependencies'])
        code, output = run(packages)
        self.assertEqual(code, 1, output)
        self.assertIn('calendar-ical -> calendar-test-support', output)

    def test_workspace_crate_missing_from_the_table_fails(self):
        code, output = run(CLEAN + [package('calendar-extra', [('calendar-core', None)])])
        self.assertEqual(code, 1, output)
        self.assertIn('calendar-extra', output)

    def test_unreadable_metadata_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'metadata.json'
            path.write_text('not json')
            result = subprocess.run([sys.executable, str(SCRIPT), '--metadata', str(path)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)


if __name__ == '__main__':
    unittest.main()
