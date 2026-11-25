"""A finished stage frees its private prefix before the Windows gate (STATUS: LIVE, 2026-11-25).

Measured: gate run 45580105382 of 2026-11-25 on revision 058c81986 (one RTX 4090 box, 117 GB disk, 108 GB free at preflight): every Linux
stage passed, then the Windows gate refused to start with `disk-44G-free-need-45G`, because the kfml stage's environment and datasets were
still on the disk. The payload now removes the kfml prefix when the stage has ended and a Windows stage follows.
"""

import contextlib
import importlib.util
import io
import tempfile
import unittest
from collections import namedtuple
from pathlib import Path

PAYLOAD = Path(__file__).with_name("vast_payload.py")


def load():
    spec = importlib.util.spec_from_file_location("payload_reclaim", PAYLOAD)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ReclaimTests(unittest.TestCase):
    def setUp(self):
        self.m = load()

    def run_stages(self, specs, soft=10**13):
        """specs: (name, rc, min_remaining_ms or None). Returns the order of events and the payload's output."""
        events = []
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            stages = []
            for name, rc, need in specs:
                stage = dict(
                    name=name,
                    command=["true"],
                    output=root / (name + ".run"),
                    progress=[root / (name + ".progress")],
                    grade=lambda code, name=name: (events.append("grade " + name), [self.m.row(name, 1)])[1],
                    after=lambda name=name: events.append("after " + name),
                    code=rc,
                )
                if need is not None:
                    stage.update(min_remaining_ms=need, short_row=name + "_retry_time")
                stages.append(stage)
            codes = {str(s["output"]): s.pop("code") for s in stages}

            def runner(command, soft, output, progress):
                events.append("run " + Path(output).name.rsplit(".", 1)[0])
                Path(output).write_text("")
                progress[0].write_text("")
                return codes[str(output)]

            buffer = io.StringIO()
            with contextlib.redirect_stdout(buffer):
                self.m.execute_stages(stages, soft, {"verdict": "FAIL", "suites": [], "hardware": {}}, root / "r.json", runner=runner)
        return events, buffer.getvalue()

    def test_the_after_action_runs_once_the_stage_ended_and_before_the_next_stage_starts(self):
        events, _ = self.run_stages([("kfml", 0, None), ("windows_gate", 0, None)])
        self.assertEqual(events, ["run kfml", "after kfml", "grade kfml", "run windows_gate", "after windows_gate", "grade windows_gate"])

    def test_the_after_action_runs_when_the_stage_failed(self):
        events, _ = self.run_stages([("kfml", 1, None)])
        self.assertEqual(events[:2], ["run kfml", "after kfml"])

    def test_a_stage_that_did_not_run_has_nothing_to_free(self):
        events, out = self.run_stages([("kfml", 0, 10**9)], soft=0)
        self.assertNotIn("after kfml", events)
        self.assertNotIn("run kfml", events)
        self.assertIn("kfml_retry_time", out)

    def test_a_stage_without_an_after_action_is_unchanged(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            stage = dict(name="apps", command=["true"], output=root / "apps.run", progress=[root / "apps.progress"],
                         grade=lambda rc: [self.m.row("apps", 1)])

            def runner(command, soft, output, progress):
                Path(output).write_text("")
                progress[0].write_text("")
                return 0

            with contextlib.redirect_stdout(io.StringIO()):
                rc = self.m.execute_stages([stage], 10**13, {"verdict": "FAIL", "suites": [], "hardware": {}}, root / "r.json", runner=runner)
            self.assertEqual(rc, 0)

    def test_reclaim_removes_the_kfml_prefix_and_prints_the_free_space(self):
        removed = []
        usage = namedtuple("usage", "total used free")
        buffer = io.StringIO()
        with contextlib.redirect_stdout(buffer):
            self.m.reclaim_stage_prefix(self.m.KFML_PREFIX, rmtree=lambda p, ignore_errors: removed.append((p, ignore_errors)),
                                        usage=lambda p: usage(100, 40, 60))
        self.assertEqual(removed, [("/workspace/kf-kfml", True)])
        self.assertEqual(buffer.getvalue(), "KF_PAYLOAD_RECLAIM path=/workspace/kf-kfml free_bytes=60\n")

    def test_reclaim_refuses_every_path_that_is_not_a_named_stage_prefix(self):
        removed = []
        for path in ("/", "/workspace", "/workspace/kf-kfml/..", "/workspace/kf-kfml/", "", "/root/prov", "/workspace/bench"):
            with self.assertRaises(ValueError, msg=path):
                self.m.reclaim_stage_prefix(path, rmtree=lambda p, ignore_errors: removed.append(p))
        self.assertEqual(removed, [])
        self.assertEqual(self.m.RECLAIMABLE_PREFIXES, ("/workspace/kf-kfml",))

    def test_the_kfml_stage_frees_its_prefix_only_when_a_windows_stage_follows(self):
        code = " ".join(" ".join(line for line in PAYLOAD.read_text().splitlines() if not line.strip().startswith("#")).split())
        self.assertIn("if windows: stage.update(after=lambda: reclaim_stage_prefix(KFML_PREFIX))", code)
        self.assertEqual(code.count("reclaim_stage_prefix(KFML_PREFIX)"), 1)

    def test_the_windows_gate_still_needs_45_gb(self):
        gate = (PAYLOAD.parents[1] / "bench/box/windows_gate.sh").read_text()
        self.assertIn('if [ -e "$VM/installed" ]; then need_gb=20; else need_gb=45; fi', gate)


if __name__ == "__main__":
    unittest.main()
