#!/usr/bin/env python3
"""wtier, 2026-11-05: the release tier's payload extras, GPU-free.

FALSIFIER (written before the code): the SPECviewperf stage wrapper prints anything but one verdict
line and its exit (a score could reach the streamed log), keeps the lane's output readable by
others, passes without its lane, or the grader accepts two verdicts, a PASS with a non-zero exit, or
a verdict class that carries digits.
"""

import importlib.util
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
WRAPPER = HERE / "vast_stage_specviewperf.sh"


def payload():
    spec = importlib.util.spec_from_file_location("payload_tiers", HERE / "vast_payload.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class SpecviewperfWrapper(unittest.TestCase):
    def run_wrapper(self, lane_text=None, args=("ci0123abcd", "/root/prov")):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            env = {
                "PATH": os.environ["PATH"],
                "KF_CI_PRIVATE_DIR": str(root / "private"),
                "KF_SPECVIEWPERF_LANE": str(root / "lane.sh"),
            }
            if lane_text is not None:
                (root / "lane.sh").write_text(lane_text)
            p = subprocess.run(
                ["bash", str(WRAPPER), *args], env=env, capture_output=True, text=True, timeout=30
            )
            private = sorted((root / "private").glob("*")) if (root / "private").exists() else []
            modes = [stat.S_IMODE(x.stat().st_mode) for x in private]
            texts = [x.read_text() for x in private]
            dmode = (
                stat.S_IMODE((root / "private").stat().st_mode)
                if (root / "private").exists()
                else None
            )
            return p, texts, modes, dmode

    def test_missing_lane_is_a_retry_not_built(self):
        p, texts, _, _ = self.run_wrapper()
        self.assertEqual(p.returncode, 3)
        self.assertEqual(
            p.stdout, "SPECVIEWPERF_STAGE_RETRY class=NOT_BUILT\nSPECVIEWPERF_STAGE_EXIT rc=3\n"
        )
        self.assertEqual(texts, [])

    def test_scores_stay_in_a_private_file(self):
        lane = (
            "echo 'composite catia-07 score=123.45'\n"
            "echo 'SPECVIEWPERF_STAGE_PASS'\n"
            "echo 'SPECVIEWPERF_STAGE_PASS score=99' \n"
            "exit 0\n"
        )
        p, texts, modes, dmode = self.run_wrapper(lane)
        self.assertEqual(p.returncode, 0)
        self.assertEqual(
            p.stdout,
            "SPEC_STAGE_START lane=none\nSPECVIEWPERF_STAGE_PASS\nSPECVIEWPERF_STAGE_EXIT rc=0\n",
        )
        self.assertNotIn("123", p.stdout + p.stderr)
        self.assertEqual(len(texts), 1)
        self.assertIn("123.45", texts[0])
        self.assertEqual(modes, [0o600])
        self.assertEqual(dmode, 0o700)

    def test_no_verdict_and_digit_classes_are_not_passed_through(self):
        p, _, _, _ = self.run_wrapper("echo 'SPECVIEWPERF_STAGE_RETRY class=SCORE_123'\nexit 1\n")
        self.assertEqual(
            p.stdout,
            "SPEC_STAGE_START lane=none\nSPECVIEWPERF_STAGE_RETRY class=NO_VERDICT\nSPECVIEWPERF_STAGE_EXIT rc=1\n",
        )

    def test_bad_arguments_are_refused(self):
        for args in ((), ("a",), ("../x", "/root/prov")):
            p, _, _, _ = self.run_wrapper("exit 0\n", args)
            self.assertEqual(p.returncode, 2, args)
            self.assertTrue(p.stdout.startswith("SPECVIEWPERF_STAGE_RETRY class=USAGE"))


class Grading(unittest.TestCase):
    def setUp(self):
        self.m = payload()

    def test_specviewperf_rows(self):
        # 2026-11-07: the grader needs the lane and its table; the full PASS contract is in test_vast_payload_spec.py
        m = self.m
        table = m.spec_table(HERE.parents[1])
        lane = "lanea"
        names = m.spec_names(table, lane)
        rows = "".join(
            "SPEC_ROW name=%s result=PASS subtests=1/1 reason=- failed=-\n" % n for n in names
        )
        ok = "SPEC_STAGE_LIST lane=lanea n=4\n" + rows + "SPECVIEWPERF_STAGE_PASS\nSPECVIEWPERF_STAGE_EXIT rc=0\n"
        self.assertEqual(
            m.specviewperf_rows(ok, 0, lane, table),
            [m.row("specviewperf", 4), m.row("specviewperf_lanea", 4)],
        )
        for text, rc, name in (
            ("SPECVIEWPERF_STAGE_PASS\n", 1, "specviewperf_retry_operator"),
            ("SPECVIEWPERF_STAGE_PASS\nSPECVIEWPERF_STAGE_EXIT rc=0\n", 0, "specviewperf_retry_operator"),
            ("SPECVIEWPERF_STAGE_FAIL\n", 1, "specviewperf"),
            ("SPECVIEWPERF_STAGE_RETRY class=NOT_BUILT\n", 3, "specviewperf_retry_not_built"),
            ("SPECVIEWPERF_STAGE_RETRY\n", 3, "specviewperf_retry_operator"),
            (
                "SPECVIEWPERF_STAGE_PASS\nSPECVIEWPERF_STAGE_PASS\n",
                0,
                "specviewperf_retry_operator",
            ),
            ("", 0, "specviewperf_retry_operator"),
            ("SPECVIEWPERF_STAGE_RETRY class=X1\n", 3, "specviewperf_retry_operator"),
        ):
            with self.subTest(text=text, rc=rc):
                with self.assertRaises(m.EvidenceError) as caught:
                    m.specviewperf_rows(text, rc, lane, table)
                self.assertEqual(caught.exception.name, name)

    def test_tier_requested_pops_and_validates(self):
        m = self.m
        env = {"KF_TIER": "large", "KF_WINDOWS_RUNS": "2", "KF_STAGES": "specviewperf"}
        self.assertEqual(m.tier_requested(env, True), ("large", 2, ["specviewperf"]))
        self.assertEqual(env, {})
        self.assertEqual(m.tier_requested({}, True), ("unknown", 1, []))
        self.assertEqual(m.tier_requested({"KF_TIER": "middle"}, True), ("middle", 1, []))
        for bad, windows in (
            ({"KF_WINDOWS_RUNS": "2"}, False),
            ({"KF_TIER": "large", "KF_STAGES": "trio"}, True),
            ({"KF_TIER": "large", "KF_STAGES": "two_guests"}, True),
            ({"KF_TIER": "small", "KF_STAGES": "specviewperf"}, True),
            ({"KF_TIER": "x"}, True),
            ({"KF_WINDOWS_RUNS": "0"}, True),
        ):
            with self.subTest(bad=bad):
                with self.assertRaises(m.EvidenceError):
                    m.tier_requested(dict(bad), windows)

    def test_slots_match_the_worker(self):
        import json

        defaults = json.loads((HERE.parents[1] / "ci/vast-broker/defaults.json").read_text())
        self.assertEqual(
            {k: v["state"] for k, v in defaults["stageSlots"].items()}, self.m.STAGE_SLOTS
        )


if __name__ == "__main__":
    unittest.main()
