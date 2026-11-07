#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Static policy fixtures: never execute a driver installer or patch application."""

import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import no_patch_check as check

HERE = Path(__file__).resolve().parent


class Policy(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="kf-patch-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.write(
            "docs/howto/installing.md",
            (HERE / "fixtures/release_checks/patch-policy.md.txt").read_text(),
        )
        self.write("scripts/install.sh", "#!/bin/sh\nsh NVIDIA.run --silent\n")
        self.write("docs/kernel_patches/a.patch", "diff --git a/a b/a\n")
        self.write(
            "docs/opt-in.md",
            "Explicit opt-in diagnostic: run tools/diagnostic.sh manually.\n",
        )
        self.write(
            "tools/diagnostic.sh",
            "#!/bin/sh\npatch -p1 < docs/kernel_patches/a.patch\n",
        )
        self.inventory = {
            "version": 1,
            "entrypoints": ["scripts/install.sh"],
            "policy_doc": "docs/howto/installing.md",
            "patches": {
                "docs/kernel_patches/a.patch": {
                    "kind": "opt-in",
                    "opt_in": "docs/opt-in.md",
                    "reason": "manual diagnostic",
                }
            },
            "applicators": {"tools/diagnostic.sh": self.digest("tools/diagnostic.sh")},
        }

    def write(self, path, text):
        p = self.root / path
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text)

    def digest(self, path):
        return hashlib.sha256((self.root / path).read_bytes()).hexdigest()

    def errors(self):
        return "\n".join(check.validate(self.root, self.inventory))

    def cli(self):
        inventory = self.root / "inventory.json"
        inventory.write_text(json.dumps(self.inventory))
        return subprocess.run(
            [
                sys.executable,
                str(HERE / "no_patch_check.py"),
                "--root",
                str(self.root),
                "--inventory",
                str(inventory),
            ],
            capture_output=True,
            text=True,
            timeout=10,
        )

    def test_cli_reports_positive_fixture_and_table(self):
        r = self.cli()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("docs/kernel_patches/a.patch", r.stdout)

    def test_cli_new_patch_failure_is_not_hidden(self):
        self.write("scripts/unexpected.patch", "diff\n")
        r = self.cli()
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("unregistered patch", r.stderr)

    def test_cli_stock_path_failure_is_not_hidden(self):
        self.write("scripts/install.sh", "bash tools/diagnostic.sh\n")
        r = self.cli()
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("stock path reaches opt-in", r.stderr)

    def test_stock_installer_and_separate_opt_in_pass(self):
        self.assertEqual(self.errors(), "")

    def test_inventory_table_names_patch_and_opt_in(self):
        table = check.table(self.inventory)
        self.assertIn("docs/kernel_patches/a.patch", table)
        self.assertIn("docs/opt-in.md", table)

    def test_unregistered_patch_fails(self):
        self.write("tools/new.patch", "patch\n")
        self.assertIn("unregistered patch", self.errors())

    def test_unregistered_diff_fails(self):
        self.write("scripts/new.diff", "diff\n")
        self.assertIn("unregistered patch", self.errors())

    def test_deleted_patch_cannot_stay_registered(self):
        (self.root / "docs/kernel_patches/a.patch").unlink()
        self.assertIn("missing patch", self.errors())

    def test_deleted_opt_in_document_fails(self):
        (self.root / "docs/opt-in.md").unlink()
        self.assertIn("opt-in pointer", self.errors())

    def test_missing_opt_in_pointer_fails(self):
        self.inventory["patches"]["docs/kernel_patches/a.patch"][
            "opt_in"
        ] = "docs/absent.md"
        self.assertIn("opt-in pointer", self.errors())

    def test_new_patch_command_in_stock_path_fails(self):
        self.write(
            "scripts/install.sh", "#!/bin/sh\npatch -p1 < docs/kernel_patches/a.patch\n"
        )
        self.assertIn("unreviewed patch applicator", self.errors())

    def test_new_git_apply_in_helper_fails(self):
        self.write("scripts/helper.sh", "#!/bin/sh\ngit -C ogkm apply fix.patch\n")
        self.assertIn("unreviewed patch applicator", self.errors())

    def test_python_git_apply_is_detected(self):
        self.write(
            "scripts/helper.py",
            "import subprocess\nsubprocess.run(['git', 'apply', 'change'])\n",
        )
        self.assertIn("unreviewed patch applicator", self.errors())

    def test_patch_without_flags_is_detected(self):
        self.write("scripts/helper.sh", "patch < changes\n")
        self.assertIn("unreviewed patch applicator", self.errors())

    def test_plain_patch_command_is_detected(self):
        self.write("scripts/helper.sh", "patch old changes\n")
        self.assertIn("unreviewed patch applicator", self.errors())

    def test_sudo_plain_patch_command_is_detected(self):
        self.write("scripts/helper.sh", "sudo patch old changes\n")
        self.assertIn("unreviewed patch applicator", self.errors())

    def test_checker_name_does_not_exempt_unrelated_tool(self):
        self.write("tools/no_patch_check.py", "subprocess.run(['git', 'apply', 'p'])\n")
        self.assertIn("unreviewed patch applicator", self.errors())

    def test_checker_name_does_not_exempt_bench_script(self):
        self.write(
            "scripts/bench/no_patch_check.py", "subprocess.run(['git', 'apply', 'p'])\n"
        )
        self.assertIn("unreviewed patch applicator", self.errors())

    def test_patch_level_regex_is_not_a_patch_application(self):
        self.write(
            "scripts/version.py",
            "import re\nVERSION = re.compile(r'^(?: (patch level not recorded))?$')\n",
        )
        self.assertEqual(self.errors(), "")

    def test_python_patch_is_detected(self):
        self.write(
            "scripts/helper.py", "subprocess.run(['patch', '-p1'], stdin=changes)\n"
        )
        self.assertIn("unreviewed patch applicator", self.errors())

    def test_reviewed_applicator_must_not_be_called_by_stock_path(self):
        self.write("scripts/install.sh", "#!/bin/sh\nbash tools/diagnostic.sh\n")
        self.assertIn("stock path reaches opt-in", self.errors())

    def test_transitive_stock_call_is_rejected(self):
        self.write("scripts/install.sh", "#!/bin/sh\nbash scripts/helper.sh\n")
        self.write("scripts/helper.sh", 'bash "$ROOT/tools/diagnostic.sh"\n')
        self.assertIn("stock path reaches opt-in", self.errors())

    def test_basename_call_is_rejected(self):
        self.write("scripts/install.sh", 'bash "$HERE/diagnostic.sh"\n')
        self.assertIn("stock path reaches opt-in", self.errors())

    def test_python_import_reaches_opt_in(self):
        self.write("scripts/install.sh", "python3 scripts/install.py\n")
        self.write("scripts/install.py", "import diagnostic\n")
        self.write(
            "tools/diagnostic.py",
            "import subprocess\nsubprocess.run(['git', 'apply', 'p'])\n",
        )
        self.inventory["applicators"]["tools/diagnostic.py"] = self.digest(
            "tools/diagnostic.py"
        )
        self.assertIn("stock path reaches opt-in", self.errors())

    def test_ordinary_apply_word_is_not_a_python_import(self):
        self.write("tools/apply.py", "print('apply')\n")
        self.inventory["applicators"]["tools/apply.py"] = self.digest("tools/apply.py")
        self.write("scripts/install.sh", "echo 'apply vendor installer'\n")
        self.assertEqual(self.errors(), "")

    def test_review_hash_changes_fail(self):
        p = self.root / "tools/diagnostic.sh"
        p.write_text(p.read_text() + "# changed\n")
        self.assertIn("review hash changed", self.errors())

    def test_command_change_fails_review(self):
        self.write("tools/diagnostic.sh", "git apply dangerous.patch\n")
        self.assertIn("review hash changed", self.errors())

    def test_default_wording_must_be_qualified(self):
        self.write("docs/howto/installing.md", "Nothing is patched.\n")
        self.assertIn("by default", self.errors())

    def test_opt_in_wording_must_be_present(self):
        self.write(
            "docs/howto/installing.md",
            "Stock host and guest NVIDIA drivers are unmodified by default.\n",
        )
        self.assertIn("opt-in", self.errors())

    def test_unqualified_stock_claim_fails(self):
        self.write(
            "docs/howto/installing.md",
            "Stock drivers are unmodified. Separate opt-in patches exist.\n",
        )
        self.assertIn("by default", self.errors())

    def test_exceptions_cannot_be_omitted(self):
        self.write(
            "docs/howto/installing.md",
            "Stock host and guest NVIDIA drivers are unmodified by default; there are no exceptions.\n",
        )
        self.assertIn("opt-in", self.errors())

    def test_explicit_default_patching_is_rejected(self):
        self.write(
            "docs/howto/installing.md",
            "Stock host and guest NVIDIA drivers are patched by default. Other patches are opt-in.\n",
        )
        self.assertIn("by default", self.errors())

    def test_opt_in_word_does_not_excuse_required_host_patch(self):
        self.write(
            "docs/howto/installing.md",
            "Host drivers require a patch by default; the guest patch is opt-in.\n",
        )
        self.assertIn("by default", self.errors())

    def test_missing_policy_doc_fails(self):
        (self.root / "docs/howto/installing.md").unlink()
        self.assertIn("policy document", self.errors())

    def test_missing_stock_entrypoint_fails(self):
        (self.root / "scripts/install.sh").unlink()
        self.assertIn("stock entrypoint", self.errors())

    def test_another_missing_stock_entrypoint_fails(self):
        self.inventory["entrypoints"].append("scripts/missing-build.sh")
        self.assertIn("stock entrypoint", self.errors())

    def test_record_cannot_have_unknown_classification(self):
        self.write("traces/proposal.patch", "diff\n")
        self.inventory["patches"]["traces/proposal.patch"] = {
            "kind": "exempt",
            "opt_in": "traces/proposal.patch",
            "reason": "record",
        }
        self.assertIn("classification", self.errors())

    def test_unknown_classification_fails(self):
        self.inventory["patches"]["docs/kernel_patches/a.patch"]["kind"] = "safe"
        self.assertIn("classification", self.errors())


if __name__ == "__main__":
    unittest.main()
