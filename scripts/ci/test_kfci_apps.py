"""Stage migration falsifier: same fake APPRES input differs in rows, verdict or exit.

The checked-in oracle is apps_gate.sh from 50640c605. Timestamps and elapsed wall
seconds are the only normalized fields for ordinary cases. Hostile display controls
allow the documented sanitization and compare authoritative payload grades.
"""

from pathlib import Path
import re
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

import test_vast_payload_apps as existing

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
from kfci.apps_gate import AppsGate


class Equivalence(existing.AppsGateScript):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.legacy = root / "scripts/bench/box/apps_gate.sh"
        self.legacy.parent.mkdir(parents=True)
        shutil.copyfile(
            SCRIPTS / "ci/fixtures/kfci/apps_gate_legacy.sh.txt", self.legacy
        )
        (root / "scripts/apps").mkdir(parents=True)
        shutil.copyfile(SCRIPTS / "apps/run_apps.sh", root / "scripts/apps/run_apps.sh")

    def pair(self, *args, **kw):
        current = self.run_gate(*args, **kw)
        with patch.object(existing, "GATE", self.legacy):
            legacy = self.run_gate(*args, **kw)

        def stable(out):
            # Owner 2026-11-09: missing app media is MEDIA. Historical Bash used NOT_BUILT.
            # Both remain RETRY/rc3; only this documented nonpassing spelling is normalized.
            out = out.replace("APPS_STAGE_RETRY class=NOT_BUILT", "APPS_STAGE_RETRY class=MEDIA")
            lines = []
            for line in out.splitlines():
                if (
                    line.startswith(
                        (
                            "APP_ROW ",
                            "APP_TRY ",
                            "APPS_STAGE_LIST ",
                            "APPS_STAGE_RETRY ",
                            "APPS_STAGE_EXIT ",
                        )
                    )
                    or line.endswith(" --- FAILED")
                    or line in ("APPS_STAGE_PASS", "APPS_STAGE_FAIL")
                ):
                    lines.append(line)
                elif line.startswith(("APPS_STAGE_MATRIX_END ", "APPS_STAGE_COUNTS ")):
                    lines.append(re.sub(r"secs=[0-9]+", "secs=WALL", line))
            return lines

        self.assertEqual(
            (current[0], stable(current[1])), (legacy[0], stable(legacy[1]))
        )
        return current

    def test_equivalence_for_all_result_kinds(self):
        for verdict in (
            "PASS",
            "FAIL",
            "TIMEOUT",
            "NOTRUN",
            "BOOT_FAIL",
            "PASS_STAR",
            "",
        ):
            with self.subTest(verdict):
                self.pair([self.res("vectorAdd", verdict), self.res("deviceQuery")])

    def test_equivalence_isolated_last_row_wins(self):
        self.pair(
            [self.res("vectorAdd", "FAIL"), self.res("deviceQuery")],
            isolated=[self.res("vectorAdd", "FAIL"), self.res("vectorAdd", "PASS")],
        )

    def test_equivalence_isolated_failure_beats_batched_pass(self):
        self.pair(
            [self.res("vectorAdd"), self.res("deviceQuery")],
            isolated=[self.res("vectorAdd", "FAIL")],
        )

    def test_equivalence_missing_and_duplicate_rows(self):
        for rows in (
            [],
            [self.res("vectorAdd")],
            [
                self.res("vectorAdd", "FAIL"),
                self.res("vectorAdd"),
                self.res("deviceQuery"),
            ],
        ):
            with self.subTest(rows):
                self.pair(rows)

    def test_equivalence_time_budget(self):
        self.pair([], budget="0.1", matrix_body="#!/bin/bash\nsleep 10\n")

    def test_equivalence_nonzero_matrix_with_complete_rows(self):
        self.pair(
            [],
            matrix_body='#!/bin/bash\nR=${KF_APPS_ROOT}/results/$2; mkdir -p "$R"\n'
            + "\n".join(
                "echo '%s' >> \"$R/guest.res\"" % self.res(a)
                for a in ("vectorAdd", "deviceQuery")
            )
            + "\nexit 9\n",
        )

    def test_equivalence_preflight(self):
        for text in (
            "",
            "# only\n",
            "unknown_app\n",
            "a b\n",
            "vectorAdd \n",
            "vectorAdd\r\n",
            "# comment\n\nvectorAdd\ndeviceQuery\n",
        ):
            with self.subTest(text):
                self.pair([], list_text=text)
        self.pair([], provisioned=False)
        self.pair([], tag="bad tag;")

    def test_equivalence_invalid_gnu_timeout_duration(self):
        self.pair([], budget="invalid")

    def test_equivalence_notes_and_missing_fields(self):
        rows = [
            self.res("vectorAdd").replace("note=-", "note=" + "x" * 150),
            "APPRES side=guest app=deviceQuery verdict=PASS rc=0 secs=- quiet=0 note=has spaces boot=t",
        ]
        self.pair(rows)

    def test_equivalence_failure_outranks_missing_rows_and_time(self):
        self.pair([self.res("vectorAdd", "FAIL")])

    def assert_payload_failure(self, rows):
        from test_kfci_progress import matrix_bytes

        module = existing.payload()
        for legacy in (False, True):
            gate = self.legacy if legacy else existing.GATE
            with patch.object(existing, "GATE", gate):
                rc, out = self.run_gate(
                    [], matrix_body=matrix_bytes(("\n".join(rows) + "\n").encode())
                )
            self.assertEqual(rc, 1, out[-1000:])
            with self.assertRaises(module.EvidenceError) as caught:
                module.apps_rows(out, rc, ["vectorAdd", "deviceQuery"])
            self.assertEqual(caught.exception.name, "apps")
            self.assertEqual(caught.exception.rows, [module.row("apps_passed", 1)])

    def test_payload_failure_priority_with_cr_progress_matches_old(self):
        self.assert_payload_failure(
            [
                self.res("vectorAdd", "FAIL"),
                self.res("deviceQuery").replace("note=-", "note=bad\rnote"),
            ]
        )

    def test_payload_failure_priority_with_oversized_progress_matches_old(self):
        self.assert_payload_failure(
            [
                self.res("vectorAdd", "FAIL"),
                self.res("deviceQuery"),
                "APPRES side=guest app=unused note=" + "x" * 66000,
            ]
        )


class StageMetadata(unittest.TestCase):
    def test_apps_is_a_step_with_declared_budget_and_artifacts(self):
        step = AppsGate()
        self.assertEqual(step.id, "apps_gate")
        self.assertIn("linux_apps_ready", step.needs)
        self.assertIn("linux_apps_checked", step.produces)
        self.assertGreater(step.timeout_seconds, 0)
        self.assertGreater(step.estimated_minutes, 0)


if __name__ == "__main__":
    unittest.main()
