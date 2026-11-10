import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / 'swift-test-inventory.py'

# The shapes that make an inventory miss tests (Review Focus 5): a plain test, a display name with async throws,
# arguments, a test inside a suite, a test inside an extension of that suite, and a helper that is not a test.
SHAPES = '''import Testing

@Test func a() {}

@Test("display name")
func b() async throws {}

@Test(arguments: [1, 2])
func c(_ n: Int) {}

@Suite struct S {
    @Test func inSuite() {}
    func helper() {}
}

extension S {
    @Test func inExtension() {}
}

func helperOutsideTheSuite() {}
'''

TRICKY = '''import Testing

@Suite struct Outer {
    @Suite struct Inner {
        @Test func nested() {}
    }
}

@Test(arguments: [
    "a (b", "c) d",
    ("e", "f"),
])
func multiLineArguments(_ value: Any) {}

@Test("it says \\"@Test func fake()\\"") func quotedName() {}
@Test(.tags(.slow)) @MainActor func twoAttributes() async {}
@Test func `backticked name`() {}

// @Test func inLineComment() {}
/* @Test func inBlockComment() {} */
let text = "@Test func inString() {}"
let raw = #"@Test func inRawString() {}"#
let many = """
@Test func inMultiLine() {}
"""
'''


def run(files):
    with tempfile.TemporaryDirectory() as directory:
        for name, text in files.items():
            path = Path(directory) / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        result = subprocess.run([sys.executable, str(SCRIPT), directory], capture_output=True, text=True)
    return result.returncode, result.stdout.splitlines(), result.stderr


class InventoryTests(unittest.TestCase):
    def test_the_five_shapes_give_exactly_five_sorted_lines(self):
        code, lines, error = run({'CoreTests/Shapes.swift': SHAPES})
        self.assertEqual(code, 0, error)
        self.assertEqual(lines, [
            'CoreTests/Shapes.swift::a',
            'CoreTests/Shapes.swift::b',
            'CoreTests/Shapes.swift::c',
            'CoreTests/Shapes.swift::inExtension',
            'CoreTests/Shapes.swift::inSuite',
        ])

    def test_nested_suites_multi_line_arguments_and_stacked_attributes(self):
        code, lines, error = run({'CoreTests/Tricky.swift': TRICKY})
        self.assertEqual(code, 0, error)
        self.assertEqual([line.split('::')[1] for line in lines],
                         ['backticked name', 'multiLineArguments', 'nested', 'quotedName', 'twoAttributes'])

    def test_comments_and_strings_do_not_count(self):
        code, lines, error = run({'CoreTests/Tricky.swift': TRICKY})
        self.assertEqual(code, 0, error)
        for name in ('inLineComment', 'inBlockComment', 'inString', 'inRawString', 'inMultiLine', 'fake'):
            self.assertFalse(any(name in line for line in lines), name)

    def test_paths_are_relative_to_the_directory_and_lines_are_sorted_across_files(self):
        code, lines, error = run({'BTests/Sub/Z.swift': '@Test func z() {}\n', 'ATests/Y.swift': '@Test func y() {}\n'})
        self.assertEqual(code, 0, error)
        self.assertEqual(lines, ['ATests/Y.swift::y', 'BTests/Sub/Z.swift::z'])

    def test_a_missing_directory_fails(self):
        result = subprocess.run([sys.executable, str(SCRIPT), '/nonexistent-dir-for-test'], capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)


if __name__ == '__main__':
    unittest.main()
