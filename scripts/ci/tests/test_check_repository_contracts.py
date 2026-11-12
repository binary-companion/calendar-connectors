import subprocess, sys, tempfile, unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / 'check-repository-contracts.py'
GOOD_RUST_KIND = '''/// Read-only (ADR 0017). Not "Calendars.ReadWrite".
pub const SCOPES: [&'static str; 5] = [
    "offline_access", "User.Read", "MailboxSettings.Read", "Calendars.Read", "Calendars.Read.Shared",
];
'''
GOOD_RUST_BROKERED = '''pub fn scopes() -> Vec<String> {
    MicrosoftConnectorKind::SCOPES.iter().filter(|s| **s != "offline_access").map(|s| s.to_string()).collect()
}
'''
RUST_KIND = 'crates/calendar-microsoft/src/connector_kind.rs'
RUST_BROKERED = 'crates/calendar-microsoft/src/brokered_connector_kind.rs'
CI = '''name: CI
on: [pull_request]
jobs:
  a:
    runs-on: ubuntu-latest
    steps:
      - run: echo "${{ secrets.GITHUB_TOKEN }}${{ secrets.DOCKERHUB_TOKEN }}"
  macos:
    runs-on: ubuntu-latest
    steps:
      - run: echo macos
  linux:
    runs-on: ubuntu-latest
    steps:
      - run: echo linux
  windows-x64:
    runs-on: ubuntu-latest
    steps:
      - run: echo windows-x64
  windows-arm64:
    runs-on: ubuntu-latest
    steps:
      - run: echo windows-arm64
  architecture:
    runs-on: ubuntu-latest
    steps:
      - run: echo architecture
  compat:
    runs-on: ubuntu-latest
    steps:
      - run: echo compat
  rust-lint:
    runs-on: ubuntu-latest
    steps:
      - run: echo rust-lint
  rust-targets:
    runs-on: ubuntu-latest
    steps:
      - run: echo rust-targets
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

def make(root: Path, **overrides):
    files = {
        RUST_KIND: GOOD_RUST_KIND,
        RUST_BROKERED: GOOD_RUST_BROKERED,
        '.github/workflows/ci.yml': CI,
        '.github/workflows/release.yml': RELEASE,
        'fixtures/persisted-formats/1.0.7/connections.json': '[]\n',
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

    def test_missing_persisted_formats_fail(self):
        self.check(False, 'fixtures/persisted-formats', **{'fixtures/persisted-formats/1.0.7/connections.json': None})

    def test_the_old_persisted_formats_path_does_not_count(self):
        self.check(False, 'fixtures/persisted-formats', **{
            'fixtures/persisted-formats/1.0.7/connections.json': None,
            'Tests/PersistedFormats/1.0.7/connections.json': '[]\n'})

    def test_a_renamed_required_ci_job_fails(self):
        for job in ('macos', 'linux', 'windows-x64', 'windows-arm64', 'architecture', 'compat', 'rust-lint',
                    'rust-targets'):
            with self.subTest(job=job):
                self.check(False, f'missing documented job {job}', **{
                    '.github/workflows/ci.yml': CI.replace(f'  {job}:\n', f'  renamed-{job}:\n')})

    def test_a_missing_compat_job_fails(self):
        self.check(False, 'missing documented job compat', **{
            '.github/workflows/ci.yml': CI.replace('  compat:\n', '  renamed-compat:\n')})

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

    # --- the scope checks read code, not comments ---

    # --- portability: every spelling of an import, in every target and every nested directory ---

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

    def test_the_word_secrets_in_a_step_name_is_not_a_violation(self):
        # "secrets. Mask" must not read as secrets.Mask.
        self.check(True, **{
            '.github/workflows/ci.yml': CI + '      - name: Never echo secrets. Mask them\n        run: echo ok\n'})

    def test_secrets_dot_name_in_prose_outside_an_expression_is_not_a_violation(self):
        self.check(True, **{
            '.github/workflows/ci.yml': CI + '      - name: Check\n        run: echo "the secrets.SIGNING_KEY value is never printed"\n'})

    def test_secret_in_an_if_without_expression_braces_outside_an_environment_fails(self):
        self.check(False, 'SIGNING_KEY', **{
            '.github/workflows/ci.yml': CI.replace('    runs-on: ubuntu-latest',
                                                   "    if: SECRETS.SIGNING_KEY != ''\n    runs-on: ubuntu-latest")})

    def test_secret_in_a_step_if_outside_an_environment_fails(self):
        self.check(False, 'SIGNING_KEY', **{
            '.github/workflows/ci.yml': CI + "      - if: secrets['SIGNING_KEY'] != ''\n        run: echo ok\n"})

    def test_secret_in_an_expression_of_a_step_name_still_fails(self):
        self.check(False, 'SIGNING_KEY', **{
            '.github/workflows/ci.yml': CI + '      - name: Deploy ${{ secrets.SIGNING_KEY }}\n        run: echo ok\n'})

    # --- a "}}" inside a quoted string does not end the expression (fail closed) ---

    PROBES = {
        'format_literal_then_secret': "format('}}{0}', secrets.BOT)",
        'quoted_close_then_secret': "'}}' && secrets.BOT",
        'quoted_close_then_whole_context': "'}}' && toJSON(secrets)",
        'escaped_quote_then_close_then_secret': "'it''s }}' && secrets.BOT",
        'bracket_form_after_quoted_close': "'}}' && secrets['BOT']",
        'unterminated_expression_then_secret': "secrets.BOT",
    }

    def probe_workflow(self, expression, environment=False, terminated=True):
        text = CI.replace('${{ secrets.DOCKERHUB_TOKEN }}', '${{ ' + expression + (' }}' if terminated else ''))
        if environment:
            text = text.replace('    runs-on: ubuntu-latest', '    environment: publish\n    runs-on: ubuntu-latest')
        return {'.github/workflows/ci.yml': text}

    def test_secret_after_a_quoted_close_braces_fails_outside_an_environment(self):
        for name, expression in self.PROBES.items():
            if name.startswith('unterminated'):
                continue
            with self.subTest(probe=name):
                needle = 'whole secrets context' if 'toJSON' in expression else 'BOT'
                self.check(False, needle, **self.probe_workflow(expression))

    def test_secret_after_a_quoted_close_braces_passes_inside_an_environment(self):
        for name, expression in self.PROBES.items():
            if name.startswith('unterminated') or 'toJSON' in expression:
                continue
            with self.subTest(probe=name):
                self.check(True, **self.probe_workflow(expression, environment=True))

    def test_whole_context_after_a_quoted_close_braces_fails_even_inside_an_environment(self):
        self.check(False, 'whole secrets context', **self.probe_workflow("'}}' && toJSON(secrets)", environment=True))

    def test_unterminated_expression_is_read_to_the_end_of_the_string(self):
        self.check(False, 'BOT', **self.probe_workflow('secrets.BOT', terminated=False))
        self.check(False, 'BOT', **self.probe_workflow("'}}' && secrets.BOT", terminated=False))

    def test_legal_quoted_close_braces_without_a_secret_passes(self):
        self.check(True, **self.probe_workflow("format('}}{0}', github.sha)"))

    def test_allowed_secret_after_a_quoted_close_braces_passes(self):
        self.check(True, **self.probe_workflow("format('}}{0}', secrets.GITHUB_TOKEN)"))

    def test_two_expressions_after_a_quoted_close_braces_are_both_read(self):
        self.check(False, 'BOT', **self.probe_workflow("'}}' }} ${{ secrets.BOT"))

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

    # --- the Rust port of the Microsoft connector requests the pinned scopes ---

    def test_rust_scopes_in_the_pinned_order_pass(self):
        self.check(True)

    def test_a_missing_rust_kind_file_fails(self):
        self.check(False, 'missing documented path: ' + RUST_KIND, **{RUST_KIND: None})

    def test_a_missing_rust_brokered_file_fails(self):
        self.check(False, 'missing documented path: ' + RUST_BROKERED, **{RUST_BROKERED: None})

    def test_an_extra_rust_scope_fails(self):
        self.check(False, 'Rust Microsoft scopes differ from the pinned list', **{
            RUST_KIND: GOOD_RUST_KIND.replace('"Calendars.Read.Shared",', '"Calendars.Read.Shared", "Mail.Read",')})

    def test_a_missing_rust_scope_fails(self):
        self.check(False, 'Rust Microsoft scopes differ from the pinned list', **{
            RUST_KIND: GOOD_RUST_KIND.replace('"MailboxSettings.Read", ', '')})

    def test_a_reordered_rust_scope_list_fails(self):
        self.check(False, 'Rust Microsoft scopes differ from the pinned list', **{
            RUST_KIND: GOOD_RUST_KIND.replace('"User.Read", "MailboxSettings.Read"', '"MailboxSettings.Read", "User.Read"')})

    def test_a_rust_write_scope_fails(self):
        self.check(False, 'Rust Microsoft scopes differ from the pinned list', **{
            RUST_KIND: GOOD_RUST_KIND.replace('"Calendars.Read",', '"Calendars.ReadWrite",')})

    def test_a_rust_write_scope_outside_the_list_fails(self):
        self.check(False, 'must stay read-only in the Rust crate', **{
            RUST_KIND: GOOD_RUST_KIND + 'const LATER: &str = "Calendars.ReadWrite.Shared";\n'})

    def test_a_rust_scope_only_in_a_comment_is_not_requested(self):
        self.check(False, 'Rust Microsoft scopes differ from the pinned list', **{
            RUST_KIND: GOOD_RUST_KIND.replace('"Calendars.Read.Shared",', '// "Calendars.Read.Shared",')})

    def test_a_forbidden_rust_scope_in_comments_passes(self):
        self.check(True, **{
            RUST_KIND: GOOD_RUST_KIND + '/* "Calendars.ReadWrite" */ // "Calendars.ReadWrite.Shared"\n'})

    def test_a_comment_marker_inside_a_rust_string_starts_no_comment(self):
        self.check(False, 'must stay read-only in the Rust crate', **{
            RUST_KIND: GOOD_RUST_KIND + 'const URL: &str = "https://x"; const W: &str = "Calendars.ReadWrite";\n'})

    def test_a_rust_scope_after_a_raw_string_with_a_comment_marker_is_still_read(self):
        self.check(False, 'Rust Microsoft scopes differ from the pinned list', **{
            RUST_KIND: GOOD_RUST_KIND.replace('"Calendars.Read.Shared",', 'r#"//"#, "Calendars.Read.Shared",')})

    def test_a_missing_rust_scopes_constant_fails(self):
        self.check(False, 'SCOPES constant not found', **{RUST_KIND: '// no constant\n'})

    def test_a_rust_brokered_kind_that_spells_its_own_scopes_fails(self):
        self.check(False, 'must not spell scopes of its own', **{
            RUST_BROKERED: GOOD_RUST_BROKERED + 'const OWN: [&str; 1] = ["Calendars.Read"];\n'})

    def test_a_rust_brokered_kind_that_ignores_the_browser_kinds_scopes_fails(self):
        self.check(False, 'derive its scopes from MicrosoftConnectorKind::SCOPES', **{
            RUST_BROKERED: 'pub fn scopes() -> Vec<String> { Vec::new() }\n'})

if __name__ == '__main__':
    unittest.main()
