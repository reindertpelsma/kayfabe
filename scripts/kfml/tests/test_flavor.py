"""Automatic wheel flavour selection (owner, 2026-11-16): derived from the box, no die or driver pinned.
The fixtures below are made-up facts (a driver-CUDA version and a compute capability), not a list of supported boards."""
import io
import json
import unittest
from pathlib import Path

from tests.helpers import fake_env
from kfml import cli, datasets, flavor, gating, rows, runner, setup_venv
from kfml import env as envmod
from kfml.spec import Spec

PINS = datasets.load_pins()


def smi(driver_cuda, cc, driver="999.9"):
    return {"name": "Fake", "driver": driver, "driver_cuda": driver_cuda, "cc": cc, "total": 24.0, "free": 23.0}


class Choose(unittest.TestCase):
    def pick(self, driver_cuda, cc):
        return flavor.choose(PINS, driver_cuda, cc)

    def test_newest_driver_gets_the_newest_flavour(self):
        c = self.pick("13.2", (8, 9))
        self.assertEqual(c.flavor, "cu128")
        self.assertIn("chose cu128", c.reason)

    def test_older_driver_gets_the_older_flavour_and_says_why(self):
        c = self.pick("12.6", (8, 9))
        self.assertEqual(c.flavor, "cu126")
        self.assertIn("rejected cu128", c.reason)
        self.assertIn("driver supports CUDA 12.6 < 12.8", c.reason)

    def test_blackwell_needs_the_new_flavour(self):
        self.assertEqual(self.pick("12.8", (12, 0)).flavor, "cu128")
        c = self.pick("12.6", (12, 0))  # old driver on a Blackwell card: nothing fits
        self.assertIsNone(c.flavor)
        self.assertIn("no pinned flavour can run", c.reason)
        self.assertIn("cu126: its wheels cover", c.reason)  # the old flavour has no sm_120 kernels

    def test_driver_too_old_for_any_flavour(self):
        c = self.pick("11.8", (8, 6))
        self.assertIsNone(c.flavor)
        self.assertEqual([x[1] for x in c.considered], [False, False])

    def test_unknown_facts_are_not_a_blocked_box(self):
        e = fake_env(driver_cuda=None, driver=None)
        envmod.choose_flavor(e, PINS)
        self.assertIsNone(e.flavor)
        self.assertFalse(e.flavor_blocked)  # unknown facts are env-missing, not a quiet NOT_RUN
        e = fake_env(driver_cuda="12.6", cc=(12, 0))
        envmod.choose_flavor(e, PINS)
        self.assertTrue(e.flavor_blocked)

    def test_driver_cuda_parsers(self):
        self.assertEqual(flavor.driver_cuda_from_smi_text("| NVIDIA-SMI 560.35.03  Driver Version: 560.35.03  CUDA Version: 12.6 |"), "12.6")
        self.assertIsNone(flavor.driver_cuda_from_smi_text("nothing"))
        self.assertEqual(flavor.driver_cuda_from_driver("575.51.03", envmod.MIN_DRIVER_FOR_CUDA), "12.9")
        self.assertEqual(flavor.driver_cuda_from_driver("570.26", envmod.MIN_DRIVER_FOR_CUDA), "12.8")
        self.assertIsNone(flavor.driver_cuda_from_driver("500.0", envmod.MIN_DRIVER_FOR_CUDA))

    def test_no_die_or_driver_literal_in_the_selection_code(self):
        """The rule is a table lookup: no GPU name, driver number or sm_ literal in flavor.py/gating.py."""
        import re
        for name in ("flavor.py", "gating.py"):
            text = (Path(flavor.__file__).parent / name).read_text()
            self.assertIsNone(re.search(r"RTX|GeForce|Tesla|\b[0-9]{3}\.[0-9]{2}\b|sm_[0-9]", text), name)


class PinsTable(unittest.TestCase):
    def test_every_flavour_has_a_lock_with_its_index_and_a_sha(self):
        import hashlib
        self.assertEqual({k: v["lock"] for k, v in PINS["flavors"].items()}, setup_venv.FLAVORS)
        for name, meta in PINS["flavors"].items():
            lock = setup_venv.ROOT / meta["lock"]
            text = lock.read_text()
            self.assertIn("download.pytorch.org/whl/" + name, text)
            self.assertIn("torch==2.7.1+" + name, text)
            self.assertEqual(meta["lock_sha256"], hashlib.sha256(lock.read_bytes()).hexdigest(),
                             "%s changed: update pins.json flavors.%s.lock_sha256" % (meta["lock"], name))
            self.assertIn(meta["cuda"], PINS["cuda_redist"][name]["manifest"])

    def test_every_flavour_has_redist_pins(self):
        for name in PINS["flavors"]:
            comps = PINS["cuda_redist"][name]["components"]
            self.assertEqual(sorted(comps), ["cuda_cccl", "cuda_cudart", "cuda_nvcc"])
            for c in comps.values():
                self.assertRegex(c["sha256"], r"^[0-9a-f]{64}$")


class AutoSetup(unittest.TestCase):
    def test_auto_resolves_from_smi_and_records_the_reason(self):
        f, why = setup_venv.resolve_flavor("auto", PINS, smi("12.6", (8, 9)))
        self.assertEqual(f, "cu126")
        self.assertIn("rejected cu128", why)
        self.assertEqual(setup_venv.resolve_flavor("auto", PINS, smi("13.0", (12, 0)))[0], "cu128")

    def test_explicit_flavour_is_not_second_guessed(self):
        self.assertEqual(setup_venv.resolve_flavor("cu128", PINS, smi("12.6", (8, 9)))[0], "cu128")

    def test_no_flavour_is_NoFlavor_not_a_generic_error(self):
        with self.assertRaises(setup_venv.NoFlavor):
            setup_venv.resolve_flavor("auto", PINS, smi("12.6", (12, 0)))

    def test_no_gpu_is_a_plain_setup_error(self):
        with self.assertRaises(setup_venv.SetupError) as cm:
            setup_venv.resolve_flavor("auto", PINS, {})
        self.assertNotIsInstance(cm.exception, setup_venv.NoFlavor)  # no GPU is an environment problem, not NOT_RUN

    def test_cli_setup_exit_3_and_json_not_run(self):
        real = envmod.nvidia_smi
        envmod.nvidia_smi = lambda: smi("12.6", (12, 0))
        try:
            out = io.StringIO()
            rc = cli.main(["setup", "--venv", "/nonexistent/never-created"], out)
        finally:
            envmod.nvidia_smi = real
        self.assertEqual(rc, 3)
        doc = json.loads(out.getvalue())
        self.assertEqual(doc["status"], "NOT_RUN")
        self.assertIn("no pinned flavour", doc["reason"])
        self.assertFalse(Path("/nonexistent/never-created").exists())

    def test_nvcc_if_needed_skips_the_download_when_system_nvcc_matches(self):
        import tempfile
        from kfml import env as e2
        real = e2.find_nvcc
        e2.find_nvcc = lambda environ=None, root=None: ("12.6", "/usr/local/cuda/bin/nvcc")
        try:
            self.assertTrue(setup_venv.nvcc_matches_flavor("cu126", PINS))
            self.assertFalse(setup_venv.nvcc_matches_flavor("cu128", PINS))
        finally:
            e2.find_nvcc = real


class NotRunNeverFail(unittest.TestCase):
    def test_blocked_box_row_is_not_run_with_named_reason(self):
        e = fake_env(torch=None, cuda=None, cuda_available=False, driver_cuda="12.6", cc=(12, 0))
        envmod.choose_flavor(e, PINS)
        v = gating.evaluate(Spec("t", "m", "d", "mid", 10), e, {})
        self.assertEqual(v.kind, "not_run")
        self.assertIn("no pinned flavour can run", v.reason)

    def test_runner_emits_NOT_RUN_rows_not_FAIL(self):
        e = fake_env(torch=None, cuda=None, cuda_available=False, driver_cuda="11.8", cc=(8, 6))
        envmod.choose_flavor(e, PINS)
        seen = []
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            rs = runner.run_all([Spec("t", "m", "d", "mid", 10)], e, environ={"KF_ML_HOME": d}, on_row=seen.append)
        self.assertEqual([r["status"] for r in rs], ["NOT_RUN"])
        self.assertEqual(rs[0]["flavor"], None)
        self.assertEqual(rs[0]["driver_cuda"], "11.8")

    def test_missing_torch_with_a_fitting_flavour_stays_env_missing(self):
        e = fake_env(torch=None, cuda=None, cuda_available=False, driver_cuda="12.6", cc=(8, 9))
        envmod.choose_flavor(e, PINS)
        self.assertEqual(gating.evaluate(Spec("t", "m", "d", "mid", 10), e, {}).kind, "env_missing")

    def test_rows_record_die_cc_driver_cuda_and_flavour(self):
        e = fake_env(driver_cuda="12.6", flavor="cu126")
        r = rows.make_row("x", "PASS", "", 1.0, e)
        for k in ("gpu_name", "compute_cap", "driver", "driver_cuda", "cuda", "flavor", "torch"):
            self.assertIn(k, r)
        self.assertEqual((r["driver_cuda"], r["flavor"], r["compute_cap"]), ("12.6", "cu126", "8.9"))


class InstalledFlavour(unittest.TestCase):
    def test_row_says_the_installed_flavour_not_the_derived_one(self):
        e = fake_env(driver_cuda="12.6", flavor="cu126", flavor_installed="cu128", torch="2.7.1+cu128")
        self.assertEqual(rows.make_row("x", "PASS", "", 1.0, e)["flavor"], "cu128")
        self.assertEqual(rows.make_row("x", "PASS", "", 1.0, fake_env(flavor="cu126"))["flavor"], "cu126")

    def test_installed_flavour_is_read_from_the_setup_record(self):
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            self.assertIsNone(envmod.read_installed_flavor(Path(d)))
            (Path(d) / "kfml-setup.json").write_text(json.dumps({"flavor": "cu128"}))
            self.assertEqual(envmod.read_installed_flavor(Path(d)), "cu128")
            self.assertIsNone(envmod.read_installed_flavor(None))


class CheckSays(unittest.TestCase):
    def test_check_prints_choice_and_reason(self):
        e = fake_env(torch=None, cuda_available=False, nvcc=None)
        e.driver_cuda = "12.6"
        envmod.choose_flavor(e, PINS)
        out = io.StringIO()
        cli._check(None, out, probe=lambda: e)
        text = out.getvalue()
        self.assertIn("flavor: cu126", text)
        self.assertIn("rejected cu128", text)

    def test_check_names_a_blocked_box(self):
        e = fake_env(torch=None, cuda=None, cuda_available=False, nvcc=None, driver_cuda="12.6", cc=(12, 0))
        envmod.choose_flavor(e, PINS)
        out = io.StringIO()
        cli._check(None, out, probe=lambda: e)
        text = out.getvalue()
        self.assertIn("flavor: NONE", text)
        self.assertIn("NOT_RUN", text)
        self.assertIn("test nanogpt_compile    NOT_RUN: no pinned flavour can run", text)


if __name__ == "__main__":
    unittest.main()
