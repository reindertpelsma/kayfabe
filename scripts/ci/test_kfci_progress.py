"""Falsifier: hostile progress erases an attributed FAIL, or invalid evidence passes.

All stage executions use synthetic files and a fake matrix, never a VM or network.
Display text may be sanitized; authoritative app fields, verdict and rc must agree.
"""

import base64
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]


def matrix_bytes(data):
    encoded = base64.b64encode(data).decode("ascii")
    return (
        '#!/bin/bash\nR="$KF_APPS_ROOT/results/$2"; mkdir -p "$R"\n'
        "python3 - \"$R\" <<'DATA'\nimport base64,sys\nfrom pathlib import Path\n"
        "Path(sys.argv[1], 'guest.res').write_bytes(base64.b64decode(%r))\nDATA\n"
        % encoded
    )


def row(app, verdict="PASS", note="-"):
    return (
        "APPRES side=guest app=%s verdict=%s rc=0 secs=2 quiet=0 note=%s "
        "boot=fake guest_xid=0 kf3_lines=1 kf3_refusals=0" % (app, verdict, note)
    )


class Progress(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.legacy = self.root / "old/scripts/bench/box/apps_gate.sh"
        self.legacy.parent.mkdir(parents=True)
        shutil.copyfile(
            SCRIPTS / "ci/fixtures/kfci/apps_gate_legacy.sh.txt", self.legacy
        )
        apps = self.root / "old/scripts/apps"
        apps.mkdir(parents=True)
        shutil.copyfile(SCRIPTS / "apps/run_apps.sh", apps / "run_apps.sh")

    def execute(self, rows, legacy=False, listing="vectorAdd\ndeviceQuery\n"):
        directory = self.root / ("legacy" if legacy else "current")
        directory.mkdir()
        apps = directory / "apps"
        apps.mkdir()
        (apps / "bundle.tgz").write_text("fake")
        (apps / "PROVISIONED").write_text("fake")
        (directory / "list").write_text(listing)
        (directory / "matrix").write_text(
            matrix_bytes(("\n".join(rows) + "\n").encode())
        )
        env = dict(
            PATH=os.environ["PATH"],
            KF_APPS_ROOT=str(apps),
            KF_APPS_LIST=str(directory / "list"),
            KF_APPS_MATRIX=str(directory / "matrix"),
            KF_APPS_BUDGET_S="5",
            KF_APPS_POLL_S="0.01",
        )
        command = self.legacy if legacy else SCRIPTS / "bench/box/apps_gate.sh"
        return subprocess.run(
            ["bash", str(command), "fake", str(directory / "prov")],
            env=env,
            capture_output=True,
            timeout=15,
        )

    def assert_failure_pair(self, rows):
        for legacy in (True, False):
            ran = self.execute(rows, legacy=legacy)
            self.assertEqual(ran.returncode, 1, ran.stdout[-2000:])
            self.assertTrue(
                ran.stdout.endswith(b"APPS_STAGE_FAIL\nAPPS_STAGE_EXIT rc=1\n")
            )
            self.assertIn(b"APP_ROW name=vectorAdd os=linux result=FAIL ", ran.stdout)
            self.assertNotIn(b"APPS_STAGE_RETRY", ran.stdout)
            if not legacy:
                self.assertNotIn(b"\r", ran.stdout)
                for line in ran.stdout.split(b"\n"):
                    if line.startswith(b"APP_TRY "):
                        self.assertLessEqual(len(line), 300)

    def test_failure_survives_cr_in_its_note(self):
        self.assert_failure_pair(
            [row("vectorAdd", "FAIL", "bad\rnote"), row("deviceQuery")]
        )

    def test_failure_survives_cr_in_passing_note(self):
        self.assert_failure_pair(
            [row("vectorAdd", "FAIL"), row("deviceQuery", note="bad\rnote")]
        )

    def test_failure_survives_cr_in_unrelated_progress(self):
        self.assert_failure_pair(
            [
                row("vectorAdd", "FAIL"),
                row("deviceQuery"),
                row("unused", note="bad\rnote"),
            ]
        )

    def test_failure_survives_oversized_unrelated_progress_after(self):
        self.assert_failure_pair(
            [
                row("vectorAdd", "FAIL"),
                row("deviceQuery"),
                row("unused", note="x" * 66000),
            ]
        )

    def test_failure_survives_oversized_unrelated_progress_before(self):
        self.assert_failure_pair(
            [
                row("unused", note="x" * 66000),
                row("vectorAdd", "FAIL"),
                row("deviceQuery"),
            ]
        )

    def test_failure_beats_missing_after_hostile_progress(self):
        self.assert_failure_pair(
            [row("unused", note="bad\rnote"), row("vectorAdd", "FAIL")]
        )

    def test_oversized_required_pass_is_not_evidence_of_pass(self):
        ran = self.execute([row("vectorAdd", note="x" * 66000), row("deviceQuery")])
        self.assertNotEqual(ran.returncode, 0)
        self.assertNotIn(b"APPS_STAGE_PASS", ran.stdout)

    def test_oversized_last_required_row_cannot_reuse_earlier_pass(self):
        ran = self.execute(
            [row("vectorAdd"), row("vectorAdd", note="x" * 66000), row("deviceQuery")]
        )
        self.assertNotEqual(ran.returncode, 0)
        self.assertNotIn(b"APPS_STAGE_PASS", ran.stdout)

    def test_nul_in_pass_verdict_is_not_stripped_into_a_pass(self):
        rows = [row("vectorAdd", "PA\0SS"), row("deviceQuery")]
        self.assertEqual(self.execute(rows, legacy=True).returncode, 0)
        ran = self.execute(rows)
        self.assertEqual(ran.returncode, 1)

    def test_non_ascii_comment_prefix_nbsp_matches_legacy_refusal(self):
        self.assert_bad_list("\u00a0")

    def test_non_ascii_comment_prefix_em_space_matches_legacy_refusal(self):
        self.assert_bad_list("\u2003")

    def test_non_ascii_comment_prefix_nel_matches_legacy_refusal(self):
        self.assert_bad_list("\u0085")

    def assert_bad_list(self, prefix):
        for legacy in (True, False):
            ran = self.execute(
                [row("vectorAdd"), row("deviceQuery")],
                legacy=legacy,
                listing=prefix + "# comment\nvectorAdd\ndeviceQuery\n",
            )
            self.assertEqual(ran.returncode, 3)
            self.assertIn(b"APPS_STAGE_RETRY class=BAD_LIST", ran.stdout)

    def test_unicode_notes_have_a_complete_utf8_byte_bound(self):
        ran = self.execute([row("vectorAdd", note="é" * 150), row("deviceQuery")])
        self.assertEqual(ran.returncode, 0, ran.stdout)
        line = next(
            line
            for line in ran.stdout.split(b"\n")
            if line.startswith(b"APP_ROW name=vectorAdd ")
        )
        note = line.split(b"note=", 1)[1]
        note.decode("utf-8", errors="strict")
        self.assertLessEqual(len(note), 120)

    def test_unicode_progress_has_a_complete_utf8_byte_bound(self):
        ran = self.execute([row("vectorAdd", note="é" * 200), row("deviceQuery")])
        self.assertEqual(ran.returncode, 0)
        for line in ran.stdout.split(b"\n"):
            if line.startswith(b"APP_TRY "):
                line.decode("utf-8", errors="strict")
                self.assertLessEqual(len(line), 300)

    def test_unrelated_invalid_progress_cannot_supply_missing_app_evidence(self):
        ran = self.execute([row("unused", note="x" * 66000)])
        self.assertEqual(ran.returncode, 3)
        self.assertIn(b"APPS_STAGE_RETRY class=BOX", ran.stdout)
        self.assertNotIn(b"APPS_STAGE_PASS", ran.stdout)

    def test_cr_inside_a_comment_does_not_create_a_new_list_entry(self):
        self.assert_legacy_comment("# comment\rnot_an_app\n")

    def test_cr_inside_an_indented_comment_keeps_legacy_line_grammar(self):
        self.assert_legacy_comment(" \t# comment\rnot_an_app\n")

    def assert_legacy_comment(self, comment):
        for legacy in (True, False):
            ran = self.execute(
                [row("vectorAdd"), row("deviceQuery")],
                legacy=legacy,
                listing=comment + "vectorAdd\ndeviceQuery\n",
            )
            self.assertEqual(ran.returncode, 0, ran.stdout)
