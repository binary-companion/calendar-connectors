"""Tests of scripts/ci/compat/compat.py (the backward-compatibility checks) and of the CI job that runs it.

This file and compat.py are adapted copies of the ones in binary-companion/timetug-shared (without the C header,
export and NuGet checks). The git-based checks run against throwaway repositories with a baseline tag, so the real
commands are what is tested: a persisted-format file of the baseline changed or deleted fails, an added one passes,
and the semver:major label from the event payload skips every check."""
import json
import os
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[3]
COMPAT = ROOT / 'scripts' / 'ci' / 'compat' / 'compat.py'
sys.path.insert(0, str(COMPAT.parent))
import compat  # noqa: E402

GIT_ENV = {'GIT_AUTHOR_NAME': 't', 'GIT_AUTHOR_EMAIL': 't@example.com', 'GIT_COMMITTER_NAME': 't',
           'GIT_COMMITTER_EMAIL': 't@example.com', 'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_NOSYSTEM': '1'}
CONNECTIONS = 'Tests/PersistedFormats/1.0.0/connections.json'
SYNC_STATE = 'Tests/PersistedFormats/1.0.0/sync-state.json'
FIXTURE = 'Tests/Fixtures/feed.json'


class Repo:
    """A throwaway repository with persisted-format files and one other JSON fixture, tagged 1.0.0."""

    def __init__(self, directory):
        self.root = Path(directory)
        self.git('init', '-q', '-b', 'main')
        self.write(CONNECTIONS, '[\n  {\n    "config" : {\n      "email" : "a@example.com"\n    },\n    "kindID" : "google"\n  }\n]')
        self.write(SYNC_STATE, '{"c1":{"_calendars":"a@example.com"}}')
        self.write(FIXTURE, '{"steps":[1,2],"name":"feed"}\n')
        self.commit('baseline')
        self.git('tag', '1.0.0')

    def git(self, *args):
        return subprocess.run(['git', *args], cwd=self.root, check=True, capture_output=True, text=True,
                              env={**os.environ, **GIT_ENV}).stdout

    def write(self, name, text):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def commit(self, message):
        self.git('add', '-A')
        self.git('commit', '-q', '--allow-empty', '-m', message)

    def compat(self, *args, env=None):
        return subprocess.run([sys.executable, str(COMPAT), *args], cwd=self.root, capture_output=True, text=True,
                              env={**os.environ, **GIT_ENV, **(env or {})})


FROZEN = ['--strict', 'Tests/PersistedFormats']


class Baseline(unittest.TestCase):
    def test_highest_strict_release(self):
        self.assertEqual(compat.highest_release(['1.0.7', '1.0.10', '1.0.9', 'v9.0.0', '2.0.0-rc.1', 'x']), '1.0.10')
        self.assertIsNone(compat.highest_release(['v1.0.0', 'latest']))

    def test_tag_reachable_from_the_base_branch(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Repo(directory)
            repo.commit('second')
            repo.git('tag', '1.1.0')
            repo.git('tag', 'v7.0.0')
            repo.git('checkout', '-q', '-b', 'side')
            repo.commit('side')
            repo.git('tag', '9.0.0')  # on a side branch: not reachable from main
            repo.git('checkout', '-q', 'main')
            self.assertEqual(repo.compat('baseline', '--ref', 'main').stdout.strip(), '1.1.0')

    def test_no_tag_means_no_baseline(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Repo(directory)
            repo.git('tag', '-d', '1.0.0')
            self.assertEqual(repo.compat('baseline', '--ref', 'main').stdout.strip(), '')


class Gate(unittest.TestCase):
    def pull_request(self, *names):
        return {'pull_request': {'labels': [{'name': name} for name in names]}}

    def test_a_pull_request_with_a_baseline_runs(self):
        self.assertEqual(compat.decide('pull_request', self.pull_request('semver:minor'), '1.0.7'),
                         (True, 'comparing with release 1.0.7'))

    def test_the_major_label_skips(self):
        run, reason = compat.decide('pull_request', self.pull_request('semver:minor', 'semver:major'), '1.0.7')
        self.assertFalse(run)
        self.assertIn('semver:major', reason)

    def test_other_events_and_the_first_release_skip(self):
        self.assertFalse(compat.decide('push', {}, '1.0.7')[0])
        self.assertFalse(compat.decide('pull_request', self.pull_request(), None)[0])

    def test_a_label_that_only_resembles_major_does_not_skip(self):
        self.assertTrue(compat.decide('pull_request', self.pull_request('semver:major?', 'major'), '1.0.0')[0])

    def run_gate(self, repo, event, event_name='pull_request'):
        event_file = repo.root / 'event.json'
        event_file.write_text(json.dumps(event))
        output = repo.root / 'output'
        output.write_text('')
        result = repo.compat('gate', '--base-ref', 'main', env={
            'GITHUB_EVENT_PATH': str(event_file), 'GITHUB_EVENT_NAME': event_name, 'GITHUB_OUTPUT': str(output)})
        self.assertEqual(result.returncode, 0, result.stderr)
        return result, dict(line.split('=', 1) for line in output.read_text().splitlines())

    def test_the_gate_reads_the_event_payload(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Repo(directory)
            _, outputs = self.run_gate(repo, self.pull_request('semver:minor'))
            self.assertEqual(outputs, {'run': 'true', 'baseline': '1.0.0'})
            result, outputs = self.run_gate(repo, self.pull_request('semver:major'))
            self.assertEqual(outputs['run'], 'false')
            self.assertIn('::notice::', result.stdout)
            _, outputs = self.run_gate(repo, {}, event_name='push')
            self.assertEqual(outputs, {'run': 'false', 'baseline': ''})

    def test_a_major_labelled_breaking_change_passes_the_job(self):
        # A persisted format broken and deleted; with semver:major the gate turns every check step off.
        with tempfile.TemporaryDirectory() as directory:
            repo = Repo(directory)
            repo.write(CONNECTIONS, '[]')
            (repo.root / SYNC_STATE).unlink()
            repo.commit('break everything')
            self.assertEqual(repo.compat('frozen', '1.0.0', *FROZEN).returncode, 1)
            _, outputs = self.run_gate(repo, self.pull_request('semver:major'))
            self.assertEqual(outputs['run'], 'false')
            checks = [step for step in workflow()['jobs']['compat']['steps']
                      if step.get('id') != 'gate' and 'uses' not in step]
            self.assertTrue(checks)
            for step in checks:
                self.assertTrue(skipped_when_gate_is_off(step), f'{step.get("name")} runs for semver:major')


class Frozen(unittest.TestCase):
    def frozen(self, change, *modes):
        with tempfile.TemporaryDirectory() as directory:
            repo = Repo(directory)
            change(repo)
            repo.commit('change')
            return repo.compat('frozen', '1.0.0', *(modes or FROZEN))

    def assert_passes(self, change, *modes):
        result = self.frozen(change, *modes)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def assert_breaks(self, change, needle, *modes):
        result = self.frozen(change, *modes)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn(needle, result.stdout)
        self.assertIn('semver:major', result.stdout)

    def test_unchanged_and_added_files_pass(self):
        def change(repo):
            repo.write('Tests/PersistedFormats/1.1.0/connections.json', '[]')
            repo.write('Tests/PersistedFormats/1.0.0/new-store.json', '{}')
            repo.write('Sources/CalendarCore/Unrelated.swift', '// changed elsewhere\n')
            repo.write(FIXTURE, '{"changed": true}\n')  # not under a checked path
        result = self.assert_passes(change)
        self.assertIn('baseline files intact since 1.0.0: Tests/PersistedFormats', result.stdout)

    def test_a_field_added_to_a_persisted_file_fails(self):
        self.assert_breaks(lambda repo: repo.write(SYNC_STATE, '{"c1":{"_calendars":"a@example.com","x":"y"}}'),
                           f'{SYNC_STATE}: modified')

    def test_a_changed_persisted_file_fails(self):
        self.assert_breaks(lambda repo: repo.write(CONNECTIONS, '[]'), f'{CONNECTIONS}: modified')

    def test_reformatting_a_persisted_file_fails(self):
        # The bytes are the format the baseline wrote: even a whitespace change is a change.
        self.assert_breaks(lambda repo: repo.write(SYNC_STATE, '{"c1": {"_calendars": "a@example.com"}}'),
                           f'{SYNC_STATE}: modified')

    def test_a_deleted_or_renamed_persisted_file_fails(self):
        self.assert_breaks(lambda repo: (repo.root / SYNC_STATE).unlink(), f'{SYNC_STATE}: deleted')
        self.assert_breaks(lambda repo: repo.git('mv', CONNECTIONS, 'Tests/PersistedFormats/1.0.0/list.json'),
                           f'{CONNECTIONS}: deleted')

    # --- the other modes of the shared script, kept so the copies stay alike ---

    def test_an_additive_change_passes_only_where_allowed(self):
        added = lambda repo: repo.write(FIXTURE, '{"steps":[1,2],"name":"feed","new":{}}\n')  # noqa: E731
        self.assert_passes(added, '--additive', 'Tests/Fixtures')
        appended = lambda repo: repo.write(FIXTURE, '{"steps":[1,2,3],"name":"feed"}\n')  # noqa: E731
        self.assert_passes(appended, '--appendable', 'Tests/Fixtures')
        self.assert_breaks(appended, f'{FIXTURE}: modified', '--additive', 'Tests/Fixtures')
        self.assert_breaks(lambda repo: repo.write(FIXTURE, '{"steps":[1,2],"name":"other"}\n'),
                           f'{FIXTURE}: modified', '--appendable', 'Tests/Fixtures')
        self.assert_breaks(lambda repo: repo.write(FIXTURE, 'not json\n'), 'not JSON', '--additive', 'Tests/Fixtures')

    def test_usage_errors(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Repo(directory)
            self.assertEqual(repo.compat('frozen', '1.0.0').returncode, 2)
            self.assertEqual(repo.compat('frozen', '1.0.0', '--loose', 'x').returncode, 2)
            self.assertEqual(repo.compat('header', '1.0.0', 'x.h').returncode, 2)

    def test_only_additions_are_listed_as_unchanged(self):
        self.assertEqual(compat.changed_baseline_files('A\ta.json\nM\tb.json\nD\tc.json\n'), ['M\tb.json', 'D\tc.json'])


class Covers(unittest.TestCase):
    BASE = {'a': 1, 'b': 'x', 'c': True, 'd': None, 'list': [{'e': 1.5}], 'o': {'f': [1, 2]}}

    def test_added_fields_pass(self):
        current = {**self.BASE, 'new': 1, 'list': [{'e': 1.5, 'g': 'y'}], 'o': {'f': [1, 2], 'h': {}}}
        self.assertTrue(compat.covers(current, self.BASE))

    def test_removed_retyped_or_changed_fields_fail(self):
        for key, value in (('a', '1'), ('a', True), ('a', 2), ('b', 'y'), ('c', 1), ('d', 0), ('o', {'f': [1]})):
            with self.subTest(key=key, value=value):
                self.assertFalse(compat.covers({**self.BASE, key: value}, self.BASE))


def workflow():
    return yaml.safe_load((ROOT / '.github' / 'workflows' / 'ci.yml').read_text())


def skipped_when_gate_is_off(step):
    return "steps.gate.outputs.run == 'true'" in str(step.get('if', ''))


class Workflow(unittest.TestCase):
    def test_label_changes_rerun_ci(self):
        data = workflow()
        triggers = data.get('on', data.get(True))
        self.assertTrue({'labeled', 'unlabeled', 'opened', 'synchronize', 'reopened'}
                        <= set(triggers['pull_request']['types']))

    def test_the_compat_job_checks_out_the_history_and_gates_first(self):
        steps = workflow()['jobs']['compat']['steps']
        self.assertEqual(steps[0]['with']['fetch-depth'], 0)
        self.assertEqual(steps[1]['id'], 'gate')
        self.assertIn('compat.py gate', steps[1]['run'])
        for step in steps[2:]:
            if 'uses' not in step:
                self.assertTrue(skipped_when_gate_is_off(step), step.get('name'))

    def test_the_compat_job_runs_every_check(self):
        runs = '\n'.join(step.get('run', '') for step in workflow()['jobs']['compat']['steps'])
        self.assertIn('compat.py frozen "$BASELINE" --strict Tests/PersistedFormats', runs)
        self.assertIn('diagnose-api-breaking-changes "$BASELINE"', runs)

    def test_every_library_product_is_compared(self):
        package = (ROOT / 'Package.swift').read_text()
        runs = '\n'.join(step.get('run', '') for step in workflow()['jobs']['compat']['steps'])
        products = re.findall(r'\.library\(name: "(\w+)"', package)
        self.assertGreaterEqual(len(products), 8)
        compared = runs.split('diagnose-api-breaking-changes', 1)[1]
        for product in products:
            self.assertRegex(compared, rf'\s{product}\b', product)

    def test_persisted_formats_exist_and_are_loaded_by_tests(self):
        versions = sorted(path.parent.name for path in (ROOT / 'Tests' / 'PersistedFormats').glob('*/connections.json'))
        self.assertIn('1.0.7', versions)
        tests = (ROOT / 'Tests' / 'CalendarCoreTests' / 'PersistedFormatTests.swift').read_text()
        for version in versions:
            self.assertIn(f'persisted("{version}", "connections.json")', tests, version)
            self.assertIn(f'persisted("{version}", "sync-state.json")', tests, version)

    def test_the_rule_is_documented(self):
        agents = (ROOT / 'AGENTS.md').read_text()
        self.assertIn('## Backward compatibility', agents)
        self.assertIn('semver:major', agents)
        self.assertIn('Tests/PersistedFormats', agents)


if __name__ == '__main__':
    unittest.main()
