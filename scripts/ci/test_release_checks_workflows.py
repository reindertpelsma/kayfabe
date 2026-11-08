#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Run the actual advisory job command on positive/negative ledger fixtures.

The existing validator suite supplies the row/link/status fixtures and mutants.
Here the workflow command itself must preserve their failure statuses.
"""

import csv
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import release_claims as rc

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/release-claims.yml"


class Advisory(unittest.TestCase):
    def setUp(self):
        self.text = WORKFLOW.read_text()
        commands = re.findall(
            r"^        run: (python3 scripts/ci/release_claims\.py[^\n]+)$",
            self.text,
            re.M,
        )
        self.assertEqual(len(commands), 1, "one explicit validator command must exist")
        self.command = commands[0]
        self.temp = tempfile.TemporaryDirectory(prefix="kf-advisory-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "scripts/ci").mkdir(parents=True)
        (self.root / "docs").mkdir()
        shutil.copyfile(
            ROOT / "scripts/ci/release_claims.py",
            self.root / "scripts/ci/release_claims.py",
        )
        (self.root / "scripts/check.sh").write_text("# fixture\n")
        (self.root / "README.md").write_text("# Fixture\n\nA stock driver claim.\n")
        row = dict.fromkeys(rc.COLUMNS, "-")
        row.update(
            claim_id="fixture-stock",
            source="README.md:3",
            src_hash=rc.line_hash("A stock driver claim."),
            claim="A stock driver claim.",
            kind="auto",
            verification="static fixture",
            check_ref="scripts/check.sh",
            where="CI GPU-free",
            status="never-verified",
            action="keep",
        )
        with (self.root / "docs/RELEASE_CLAIMS.tsv").open("w") as stream:
            writer = csv.DictWriter(
                stream, rc.COLUMNS, delimiter="\t", lineterminator="\n"
            )
            writer.writeheader()
            writer.writerow(row)
        (self.root / "docs/RELEASE_CLAIMS.md").write_text("# Fixture ledger\n")

    def run_command(self):
        return subprocess.run(
            ["bash", "-eo", "pipefail", "-c", self.command],
            cwd=self.root,
            capture_output=True,
            text=True,
            timeout=10,
        )

    def test_valid_fixture_passes_actual_workflow_command(self):
        r = self.run_command()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("0 errors", r.stdout)

    def test_readme_drift_is_visible_from_job_command(self):
        (self.root / "README.md").write_text("# Fixture\n\nChanged claim.\n")
        r = self.run_command()
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("not covered", r.stdout)

    def test_missing_pointer_is_visible_from_job_command(self):
        (self.root / "scripts/check.sh").unlink()
        r = self.run_command()
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("does not exist", r.stdout)

    def test_advisory_job_does_not_join_required_gate(self):
        self.assertIn("continue-on-error: true", self.text)
        gate = (ROOT / ".github/workflows/ci.yml").read_text().split("  gate:", 1)[1]
        self.assertNotIn("release-claims", gate)

    def test_readme_and_ledger_changes_trigger_pr_job(self):
        for trigger in ("pull_request:", "'README.md'", "'docs/RELEASE_CLAIMS.*'"):
            self.assertIn(trigger, self.text)
        self.assertNotRegex(self.text, r"(?m)^  (?:push|release):")


if __name__ == "__main__":
    unittest.main()
