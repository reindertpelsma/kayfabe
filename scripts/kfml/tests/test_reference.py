"""Per-die-class references and the cross-die comparison rule (owner 2026-11-16)."""
import json
import os
import tempfile
import unittest

from tests.helpers import fake_env
from kfml import reference, runner
from kfml.spec import Spec


def spec(name="t"):
    return Spec(name, "fake.mod", "d", "mid", 100)


class Fake:
    def __init__(self, result):
        self.result, self.calls = result, []

    def __call__(self, ctx, budget, env, log, python=None):
        self.calls.append(ctx)
        return dict(self.result, log=log)


class Reference(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.d = os.path.join(self.tmp.name, "ref")
        self.root = os.path.join(self.tmp.name, "root")

    def tearDown(self):
        self.tmp.cleanup()

    def rec(self, gpu, cc, data, wall=10.0):
        reference.write(self.d, "t", fake_env(gpu_name=gpu, cc=cc, driver="D"), data, wall_s=wall, run_id="r1")

    def test_one_directory_per_die(self):
        self.rec("NVIDIA GeForce RTX 4070", (8, 9), {"a": 1})
        self.rec("NVIDIA A10", (8, 6), {"a": 2})
        self.assertEqual(sorted(os.listdir(self.d)), ["nvidia-a10", "nvidia-geforce-rtx-4070"])

    def test_prefers_same_die_then_same_cc_then_any(self):
        self.rec("Die A", (8, 9), {"a": "A"})
        self.rec("Die B", (7, 5), {"a": "B"})
        doc, how = reference.find(self.d, "t", fake_env(gpu_name="Die B", cc=(7, 5)))
        self.assertEqual((how, doc["data"]), ("same_die", {"a": "B"}))
        doc, how = reference.find(self.d, "t", fake_env(gpu_name="Die C", cc=(8, 9)))
        self.assertEqual((how, doc["data"]), ("same_cc", {"a": "A"}))
        doc, how = reference.find(self.d, "t", fake_env(gpu_name="Die D", cc=(12, 0)))
        self.assertEqual(how, "other_die")  # a card never seen still gets the algorithmic reference

    def test_flat_legacy_file_is_read(self):
        os.makedirs(self.d)
        with open(os.path.join(self.d, "t.json"), "w") as f:
            json.dump({"data": {"old": 1}}, f)
        self.assertEqual(reference.load(self.d, "t", fake_env()), {"old": 1})

    def test_wall_ratio_only_against_the_same_die(self):
        self.rec("Die A", (8, 9), {"a": 1}, wall=10.0)
        doc, how = reference.find(self.d, "t", fake_env(gpu_name="Die A", cc=(8, 9)))
        self.assertEqual(reference.wall_ratio(doc, how, 12.0), 1.2)
        doc, how = reference.find(self.d, "t", fake_env(gpu_name="Die Z", cc=(8, 9)))
        self.assertIsNone(reference.wall_ratio(doc, how, 12.0))

    def test_runner_uses_other_die_data_but_reports_no_ratio_and_never_fails_on_wall(self):
        self.rec("Die A", (8, 9), {"x": 1}, wall=1.0)
        ex = Fake({"status": "PASS", "metrics": {}})
        rows = runner.run_all([spec()], fake_env(gpu_name="Die Z", cc=(8, 6)), environ={}, root=self.root,
                              reference_dir=self.d, exec_fn=ex, resolve_fn=lambda u, t: None)
        self.assertEqual(ex.calls[0]["reference"], {"x": 1})
        m = rows[0]["metrics"]
        self.assertEqual(m["reference_from"]["how"], "other_die")
        self.assertEqual(m["reference_from"]["gpu_name"], "Die A")
        self.assertIsNone(m["wall_ratio"])
        self.assertEqual(rows[0]["status"], "PASS")

    def test_every_pass_row_is_recorded_even_without_workload_data(self):
        ex = Fake({"status": "PASS", "metrics": {"x": 1.5, "s": "t", "nested": {"a": 1}}})
        runner.run_all([spec()], fake_env(gpu_name="Die A"), environ={}, root=self.root, record_dir=self.d,
                       exec_fn=ex, resolve_fn=lambda u, t: None)
        doc = json.load(open(os.path.join(self.d, "die-a", "t.json")))
        self.assertEqual(doc["data"], {})
        self.assertEqual(doc["metrics"], {"x": 1.5, "s": "t", "nested": {"a": 1}})  # the row's metrics as measured, minus the log path
        self.assertGreaterEqual(doc["wall_s"], 0)
        self.assertEqual(doc["measured_on"]["driver"], "999.1")

    def test_failed_and_not_run_rows_are_not_recorded(self):
        for st in ("FAIL", "NOT_RUN"):
            ex = Fake({"status": st, "metrics": {}, "reason": "x"})
            runner.run_all([spec()], fake_env(gpu_name="Die B"), environ={}, root=self.root, record_dir=self.d,
                           exec_fn=ex, resolve_fn=lambda u, t: None)
        self.assertFalse(os.path.exists(os.path.join(self.d, "die-b")))

    def test_no_reference_means_no_reference_fields(self):
        ex = Fake({"status": "PASS", "metrics": {}})
        rows = runner.run_all([spec()], fake_env(), environ={}, root=self.root, reference_dir=self.d,
                              exec_fn=ex, resolve_fn=lambda u, t: None)
        self.assertNotIn("reference_from", rows[0]["metrics"])
        self.assertIsNone(ex.calls[0]["reference"])


if __name__ == "__main__":
    unittest.main()


class CommittedReference(unittest.TestCase):
    """reference/<die>/<test>.json is labelled with the die and driver it was measured on, covers every spec, and loads."""

    def test_every_spec_has_a_labelled_baseline(self):
        from pathlib import Path
        from kfml import registry
        base = Path(__file__).resolve().parent.parent / "reference"
        dies = sorted(p for p in base.iterdir() if p.is_dir())
        self.assertTrue(dies)
        for die in dies:
            for s in registry.all_specs():
                doc = json.loads((die / (s.name + ".json")).read_text())
                m = doc["measured_on"]
                for k in ("gpu_name", "compute_cap", "driver", "driver_cuda", "cuda", "torch", "flavor", "run_id"):
                    self.assertTrue(m.get(k), (die.name, s.name, k))
                self.assertEqual(reference.die_slug(m["gpu_name"]), die.name)
                self.assertGreater(doc["wall_s"], 0)
                self.assertIn("data", doc)

    def test_a_box_of_another_die_still_finds_them(self):
        from pathlib import Path
        base = Path(__file__).resolve().parent.parent / "reference"
        doc, how = reference.find(str(base), "nanogpt_compile", fake_env(gpu_name="Never Seen Card", cc=(12, 0)))
        self.assertEqual(how, "other_die")
        self.assertTrue(doc["data"])  # the loss windows
        self.assertIsNone(reference.wall_ratio(doc, how, 99.0))
