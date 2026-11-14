import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / 'check-tokio-features.py'

# The shape of `cargo tree -e features,normal,build --prefix none -i tokio` (copied from the real output at the
# pinned versions, shortened): the package line, then each enabled feature of tokio followed by what enabled it.
CLEAN = '''tokio v1.54.1
tokio feature "macros"
calendar-core v0.0.0 (/work/crates/calendar-core)
calendar-core feature "default" (command-line)
tokio feature "rt"
calendar-core v0.0.0 (/work/crates/calendar-core) (*)
tokio feature "sync"
calendar-core v0.0.0 (/work/crates/calendar-core) (*)
tokio feature "time"
calendar-core v0.0.0 (/work/crates/calendar-core) (*)
tokio feature "tokio-macros"
tokio feature "macros" (*)
'''
WITH_NET = CLEAN + 'tokio feature "net"\ncalendar-core v0.0.0 (/work/crates/calendar-core) (*)\n'
WITH_MULTI_THREAD = CLEAN + 'tokio feature "rt-multi-thread"\ncalendar-core v0.0.0 (/work/crates/calendar-core) (*)\n'
WITH_DEFAULT = CLEAN + 'tokio feature "default"\ncalendar-core v0.0.0 (/work/crates/calendar-core) (*)\n'


def run(text):
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / 'tree.txt'
        path.write_text(text)
        result = subprocess.run([sys.executable, str(SCRIPT), '--tree', str(path)], capture_output=True, text=True)
    return result.returncode, result.stdout + result.stderr


class TokioFeatureTests(unittest.TestCase):
    def test_the_four_allowed_features_pass(self):
        code, output = run(CLEAN)
        self.assertEqual(code, 0, output)

    def test_net_fails_naming_the_feature(self):
        code, output = run(WITH_NET)
        self.assertEqual(code, 1, output)
        self.assertIn('net', output)

    def test_rt_multi_thread_fails(self):
        code, output = run(WITH_MULTI_THREAD)
        self.assertEqual(code, 1, output)
        self.assertIn('rt-multi-thread', output)

    def test_default_feature_fails(self):
        code, output = run(WITH_DEFAULT)
        self.assertEqual(code, 1, output)

    def test_tree_without_tokio_passes(self):
        code, output = run('')
        self.assertEqual(code, 0, output)


if __name__ == '__main__':
    unittest.main()
