"""Black-box regressions for the in-use source line-length ratchet (no GPU)."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CHECKER = Path(__file__).with_name("linelen.py")
BASELINE = "scripts/ci/linelen-baseline.json"
SOURCE = "crates/kf-example/src/lib.rs"
LONG = "x" * 101 + "\n"


class GitFixture(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ll1-test-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.git("init", "-q")
        self.git("config", "user.name", "Reindert Pelsma")
        self.git("config", "user.email", "229738317+reindertpelsma@users.noreply.github.com")
        self.write(BASELINE, "{}\n")
        self.commit()
        self.base = self.git("rev-parse", "HEAD").strip()

    def git(self, *args):
        result = subprocess.run(
            ["git", *args], cwd=self.root, capture_output=True, text=True, check=True
        )
        return result.stdout

    def write(self, name, content):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        self.git("add", "--", name)

    def baseline(self, entries):
        self.write(BASELINE, json.dumps(entries, sort_keys=True, indent=2) + "\n")

    def commit(self):
        self.git("add", "-A")
        self.git(
            "commit",
            "-qm",
            "LL1 test fixture\n\nCo-Authored-By: Codex <noreply@openai.com>\n"
            "Lane: overnight-20261018-LL1",
        )

    def check(self, *args, code=0):
        result = subprocess.run(
            [sys.executable, str(CHECKER), "--root", str(self.root), *args],
            capture_output=True,
            text=True,
            timeout=20,
        )
        self.assertEqual(result.returncode, code, result.stdout + result.stderr)
        return result.stdout + result.stderr


class LineLengthTests(GitFixture):
    def test_100_columns_pass(self):
        self.write(SOURCE, "x" * 100 + "\n")
        self.check()

    def test_101_columns_fail_in_a_new_file(self):
        self.write(SOURCE, LONG)
        self.assertIn(SOURCE, self.check(code=1))

    def test_an_existing_file_cannot_increase_its_count(self):
        self.write(SOURCE, LONG * 2)
        self.baseline({SOURCE: 1})
        self.check(code=1)

    def test_removing_debt_elsewhere_cannot_buy_a_new_file(self):
        self.write(SOURCE, "short\n")
        self.baseline({SOURCE: 2})
        self.write("broker/new.c", LONG)
        self.check(code=1)

    def test_unchanged_legacy_lines_pass(self):
        self.write(SOURCE, LONG)
        self.baseline({SOURCE: 1})
        self.commit()
        self.check("--base", self.git("rev-parse", "HEAD").strip())

    def test_added_line_fails_even_when_file_count_stays_the_same(self):
        self.write(SOURCE, LONG)
        self.baseline({SOURCE: 1})
        self.commit()
        base = self.git("rev-parse", "HEAD").strip()
        self.write(SOURCE, "y" * 101 + "\n")
        self.commit()
        self.assertIn(f"{SOURCE}:1", self.check("--base", base, code=1))

    def test_added_line_fails_even_when_file_count_decreases(self):
        self.write(SOURCE, LONG * 2)
        self.baseline({SOURCE: 2})
        self.commit()
        base = self.git("rev-parse", "HEAD").strip()
        self.write(SOURCE, "z" * 101 + "\n")
        self.commit()
        self.check("--base", base, code=1)

    def test_changed_short_lines_do_not_reject_legacy_context(self):
        self.write(SOURCE, "old\n" + LONG)
        self.baseline({SOURCE: 1})
        self.commit()
        base = self.git("rev-parse", "HEAD").strip()
        self.write(SOURCE, "new\n" + LONG)
        self.commit()
        self.check("--base", base)

    def test_diff_uses_merge_base_and_ignores_changes_on_base_branch(self):
        self.write(SOURCE, "short\n")
        self.commit()
        fork = self.git("rev-parse", "HEAD").strip()
        self.write(SOURCE, "other\n")
        self.commit()
        head = self.git("rev-parse", "HEAD").strip()
        self.git("checkout", "-q", "--detach", fork)
        self.write("broker/base_only.c", LONG)
        self.commit()
        base = self.git("rev-parse", "HEAD").strip()
        self.git("checkout", "-q", "--detach", head)
        self.check("--base", base)

    def test_diff_reads_head_content_despite_uncommitted_exemption(self):
        self.write(SOURCE, "short\n")
        self.baseline({SOURCE: 1})
        self.commit()
        base = self.git("rev-parse", "HEAD").strip()
        self.write(SOURCE, LONG)
        self.commit()
        self.write(SOURCE, "// @generated; do not edit\n" + LONG)
        self.check("--base", base, code=1)

    def test_update_lowers_and_removes_resolved_entries(self):
        self.write(SOURCE, LONG)
        self.baseline({SOURCE: 2, "broker/gone.c": 3})
        self.commit()
        self.check("--update")
        self.assertEqual(json.loads((self.root / BASELINE).read_text()), {SOURCE: 1})

    def test_update_refuses_to_raise_and_does_not_write(self):
        self.write(SOURCE, LONG * 2)
        self.baseline({SOURCE: 1})
        self.commit()
        before = (self.root / BASELINE).read_bytes()
        self.check("--update", code=1)
        self.assertEqual((self.root / BASELINE).read_bytes(), before)

    def test_update_refuses_to_add_new_debt(self):
        self.write("broker/new.c", LONG)
        self.check("--update", code=1)
        self.assertEqual(json.loads((self.root / BASELINE).read_text()), {})

    def test_update_checks_added_lines_before_writing(self):
        self.write(SOURCE, LONG * 2)
        self.baseline({SOURCE: 2})
        self.commit()
        base = self.git("rev-parse", "HEAD").strip()
        self.write(SOURCE, "new" * 34 + "\n")
        self.commit()
        before = (self.root / BASELINE).read_bytes()
        self.check("--base", base, "--update", code=1)
        self.assertEqual((self.root / BASELINE).read_bytes(), before)

    def test_pr_cannot_raise_committed_baseline(self):
        self.write(SOURCE, LONG)
        self.baseline({SOURCE: 1})
        self.commit()
        base = self.git("rev-parse", "HEAD").strip()
        self.baseline({SOURCE: 2})
        self.commit()
        self.check("--base", base, code=1)

    def test_pr_cannot_add_a_baseline_entry(self):
        self.baseline({SOURCE: 1})
        self.commit()
        self.check("--base", self.base, code=1)

    def test_update_cannot_accept_a_locally_inflated_baseline(self):
        self.write(SOURCE, LONG)
        self.baseline({SOURCE: 1})
        self.commit()
        self.baseline({SOURCE: 2})
        self.check("--update", code=1)

    def test_missing_baseline_fails_closed_including_update(self):
        self.git("rm", "-q", BASELINE)
        self.check(code=2)
        self.check("--update", code=2)

    def test_malformed_baselines_fail_closed(self):
        for value in ("[]", "null", '{"broker/a.c": true}', '{"broker/a.c": -1}'):
            with self.subTest(value=value):
                self.write(BASELINE, value)
                self.check(code=2)

    def test_unknown_base_is_an_infrastructure_error(self):
        self.check("--base", "missing-ref", code=2)

    def test_untracked_files_are_not_baselined(self):
        (self.root / "broker").mkdir()
        (self.root / "broker/untracked.c").write_text(LONG)
        self.check()

    def test_tracked_deleted_file_is_resolved(self):
        self.write(SOURCE, LONG)
        self.baseline({SOURCE: 1})
        self.commit()
        (self.root / SOURCE).unlink()
        self.check("--update")
        self.assertEqual(json.loads((self.root / BASELINE).read_text()), {})

    def test_tabs_use_eight_column_stops(self):
        self.write("broker/tab.c", "\t" + "x" * 93 + "\n")
        self.check(code=1)

    def test_unicode_is_counted_as_characters_not_utf8_bytes(self):
        self.write(SOURCE, "é" * 100 + "\n")
        self.check()
        self.write(SOURCE, "é" * 101 + "\n")
        self.check(code=1)

    def test_crlf_does_not_add_a_column(self):
        self.write("broker/crlf.c", "x" * 100 + "\r\n")
        self.check()

    def test_url_and_hash_only_lines_are_exempt(self):
        self.write("broker/urls.c", "https://example.org/" + "a" * 150 + "\n" + "a" * 128)
        self.check()

    def test_code_containing_a_url_is_not_exempt(self):
        self.write("broker/urls.c", 'url = "https://example.org/' + "a" * 150 + '";\n')
        self.check(code=1)

    def test_long_line_ok_requires_a_reason(self):
        self.write("broker/reason.c", LONG.rstrip() + " // long-line-ok: wire fixture\n")
        self.check()
        self.write("broker/reason.c", LONG.rstrip() + " // long-line-ok: \n")
        self.check(code=1)

    def test_generated_header_and_path_exemptions(self):
        for path, header in (
            ("crates/kf-abi/src/generated/facts.rs", ""),
            ("broker/protocol.c", "// @generated by protocol-gen; do not edit.\n"),
            ("tools/constants.py", "# Generated by derive.py; do not edit.\n"),
        ):
            self.write(path, header + LONG)
        self.check()

    def test_generator_source_is_not_mistaken_for_its_output(self):
        self.write("tools/generate.py", 'print("// @generated; DO NOT EDIT")\n' + LONG)
        self.check(code=1)

    def test_archive_vendor_frozen_and_oneoff_trees_are_exempt(self):
        for path in (
            "archive/code.c",
            "tools/archive/old.py",
            "tools/archive-2020/old.py",
            "third_party/code.c",
            "broker/vendor/code.c",
            "traces/code.py",
            "crates/kayfabe-core/src/lib.rs",
            "scripts/bench/windows/host_tools/old.py",
            "scripts/bench/windows/owner-20261018.sh",
            "scripts/bench/w392_llm.sh",
        ):
            self.write(path, LONG)
        self.check()

    def test_in_use_extensions_and_tests_are_enforced(self):
        for path in (
            "crates/kf-core/tests/a.rs",
            "qemu/hw/misc/kf3/a.h",
            "cuda/a.cu",
            "broker/test/a.c",
            "ci/vast-broker/a.py",
            "tools/a.js",
            "tools/a.ts",
            "scripts/ci/a.sh",
            "scripts/fastguest/a.sh",
            "scripts/drivermatrix/a.py",
            "scripts/apps/a.sh",
            "scripts/bench/v3_gates.sh",
            "scripts/bench/build_kf3.sh",
            "scripts/bench/cuda_ladder.sh",
            "scripts/bench/box/a.sh",
            "tests/a.py",
            ".github/workflows/ci.yml",
            "pyproject.toml",
        ):
            with self.subTest(path=path):
                self.write(path, LONG)
                self.check(code=1)
                self.git("rm", "-qf", path)

    def test_markdown_json_tsv_and_lockfiles_are_exempt(self):
        for path in ("tools/a.md", "tools/a.json", "tools/a.tsv", "ci/package-lock.json"):
            self.write(path, LONG)
        self.check()

    def test_symlink_does_not_read_a_target_outside_the_checkout(self):
        external = self.root.parent / (self.root.name + "-external")
        external.write_text(LONG)
        self.addCleanup(external.unlink)
        (self.root / "broker").mkdir()
        os.symlink(external, self.root / "broker/link.c")
        self.git("add", "broker/link.c")
        self.check()

    def test_diff_handles_spaces_quotes_and_newlines_in_paths(self):
        for name in ("broker/a space.c", 'broker/a"quote.c', "broker/a\nnewline.c"):
            with self.subTest(name=name):
                self.write(name, LONG)
                self.baseline({name: 1})
                self.commit()
                self.check("--base", self.base, code=1)
                self.git("rm", "-q", name)

    def test_diff_exemptions_apply_to_head_and_added_lines(self):
        self.write("broker/allowed.c", "x" * 101 + " // long-line-ok: protocol fixture\n")
        self.write("tools/archive/code.c", LONG)
        self.write("broker/generated.c", "// @generated; do not edit\n" + LONG)
        self.commit()
        self.check("--base", self.base)

    def test_rename_to_a_new_path_does_not_inherit_debt(self):
        self.write(SOURCE, LONG)
        self.baseline({SOURCE: 1})
        self.commit()
        base = self.git("rev-parse", "HEAD").strip()
        self.git("mv", SOURCE, "crates/kf-example/src/renamed.rs")
        self.commit()
        self.check("--base", base, code=1)

    def test_report_is_read_only_and_separates_exempt_code(self):
        self.write(SOURCE, LONG)
        self.baseline({SOURCE: 1})
        self.write("crates/kayfabe-core/src/lib.rs", LONG * 2)
        before = (self.root / BASELINE).read_bytes()
        output = self.check("--report")
        self.assertIn("Rust", output)
        self.assertIn("EXEMPT", output)
        self.assertEqual((self.root / BASELINE).read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
