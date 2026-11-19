#!/usr/bin/env python3
"""The host-driver preflight of the Vast payload (2026-11-18), GPU-free.

MEASURED 2026-11-18 (Vast box, RTX 5070 Ti = GB203, leg windows_gate): provision_full.sh defaults the host driver to 580.159.04 for every die; on a
Blackwell die the kf3 device then refuses to realize ("the SDR caps page is NOT derived ... display.precomp.0xca73.0x0780: unmeasured on GB203
branch 580 (measured on: GB202/595, GB203/595, GB206/595)"), QEMU exits 10 s into boot 1 and the whole leg is burnt, 25 minutes of Windows install
first. The same box with host 595.91.07 passed (traces/blackwell_win_20261103/README.md).

FALSIFIER (written before the code): any of these lets that run start, or turns a good run into a refusal:
  - a Blackwell box (compute capability 12.x) whose host driver would be on a branch with no measured PRECOMP row reaches a stage;
  - a Blackwell box with a measured-branch driver (595.x) is refused;
  - a die that is not Blackwell sees a changed line, or the table is even read for it;
  - a box with no GPU, an unreadable nvidia-smi or an unknown driver is refused by THIS rule (other preflights own those);
  - the driver the run would install is not the one provision_host_driver.sh installs (HOST_DRIVER, default 580.159.04), or, when no
    provision stage is selected, not the driver already loaded on the box;
  - the refusal text carries a URL or a secret, or the table of measured branches is a second hand-kept copy that drifts from the Rust loader.
"""

import contextlib
import io
import os
import re
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import test_vast_payload_v7 as v7

ROOT = Path(__file__).resolve().parents[2]
BLACKWELL = {"gpu_name": "NVIDIA GeForce RTX 5070 Ti", "compute_cap": 1200, "driver_version": "580.159.04"}
BLACKWELL_595 = dict(BLACKWELL, driver_version="595.91.07")
ADA = {"gpu_name": "NVIDIA GeForce RTX 4090", "compute_cap": 890, "driver_version": "580.159.04"}
AMPERE = {"gpu_name": "NVIDIA GeForce RTX 3060", "compute_cap": 860, "driver_version": "580.159.04"}
TURING = {"gpu_name": "NVIDIA GeForce RTX 2080", "compute_cap": 750, "driver_version": "580.159.04"}
NO_GPU = {"gpu_name": "unknown", "compute_cap": 0, "driver_version": "unknown"}
FULL_PLAN = ["provision", "cuda_host", "merge_bar", "cuda_guest", "windows_gate"]
NO_ROOT = Path("/nonexistent-kayfabe-root")  # a die that is not Blackwell must never read the table
BLACKWELL_TOKEN_580 = "HOST_DRIVER_UNMEASURED:blackwell:580"


class HostDriverRuleTests(unittest.TestCase):
    setUp = v7.V7Tests.setUp

    def refusal(self, env, hw, stages=FULL_PLAN, root=ROOT):
        return self.m.host_driver_refusal(env, hw, stages, root)

    def test_blackwell_with_the_default_driver_is_refused_by_name(self):
        got = self.refusal({}, BLACKWELL)
        self.assertIsNotNone(got)
        self.assertEqual(got[0], BLACKWELL_TOKEN_580)

    def test_the_refusal_names_the_selector_the_measured_branch_and_the_run_url_route(self):
        token, hint = self.refusal({}, BLACKWELL)
        self.assertIn("KF_HOST_DRIVER", hint)
        self.assertIn("HOST_DRIVER", hint)
        self.assertIn("595", hint)
        self.assertIn("595.91.07", hint)
        # 595.91.07 is not pinned in scripts/ci/trust_manifest.json: the hint says a RUN_URL is a user input, and adds no pin
        self.assertIn("RUN_URL", hint)
        self.assertIn("user input", hint)

    def test_the_refusal_text_has_no_url_and_no_secret(self):
        token, hint = self.refusal({}, BLACKWELL)
        for text in (token, hint):
            self.assertNotRegex(text, r"(?i)https?:|://|www\.|\.com|\.dev|\.run\b")
            self.assertNotRegex(text, r"(?i)token=|secret|password|apikey|api_key|bearer|signature|x-amz")
            self.assertNotIn("\n", text)
            self.assertTrue(text.isascii())
        self.assertLess(len(hint), 500)

    def test_a_595_driver_named_by_HOST_DRIVER_passes(self):
        self.assertIsNone(self.refusal({"HOST_DRIVER": "595.91.07"}, BLACKWELL))

    def test_a_595_driver_named_by_KF_HOST_DRIVER_passes(self):
        # the matrix mapping (release_driver_environment) copies KF_HOST_DRIVER into HOST_DRIVER before any stage
        self.assertIsNone(self.refusal({"KF_HOST_DRIVER": "595.91.07"}, BLACKWELL))

    def test_both_selectors_naming_595_pass(self):
        # windows_lane.sh: `export HOST_DRIVER KF_HOST_DRIVER=$HOST_DRIVER`
        self.assertIsNone(self.refusal({"HOST_DRIVER": "595.91.07", "KF_HOST_DRIVER": "595.91.07"}, BLACKWELL))

    def test_the_whole_595_branch_is_the_measured_branch(self):
        self.assertIsNone(self.refusal({"HOST_DRIVER": "595.84"}, BLACKWELL))

    def test_an_explicit_580_or_590_driver_is_refused_with_its_own_branch(self):
        self.assertEqual(self.refusal({"HOST_DRIVER": "580.65.06"}, BLACKWELL)[0], BLACKWELL_TOKEN_580)
        self.assertEqual(self.refusal({"KF_HOST_DRIVER": "590.48.01"}, BLACKWELL)[0], "HOST_DRIVER_UNMEASURED:blackwell:590")

    def test_the_installer_selector_wins_over_the_readiness_selector(self):
        # provision_host_driver.sh reads only HOST_DRIVER; a disagreeing KF_HOST_DRIVER is the READY check's business
        self.assertEqual(
            self.refusal({"HOST_DRIVER": "580.159.04", "KF_HOST_DRIVER": "595.91.07"}, BLACKWELL)[0], BLACKWELL_TOKEN_580
        )
        self.assertIsNone(self.refusal({"HOST_DRIVER": "595.91.07", "KF_HOST_DRIVER": "580.159.04"}, BLACKWELL))

    def test_a_non_blackwell_die_is_never_refused_and_the_table_is_never_read(self):
        for name, hw in (("ada", ADA), ("ampere", AMPERE), ("turing", TURING)):
            with self.subTest(name):
                self.assertIsNone(self.refusal({}, hw, root=NO_ROOT))
                self.assertIsNone(self.refusal({"HOST_DRIVER": "570.148.08"}, hw, root=NO_ROOT))

    def test_ada_with_the_default_driver_is_unchanged(self):
        self.assertIsNone(self.refusal({}, ADA))

    def test_ampere_with_the_default_driver_is_unchanged(self):
        self.assertIsNone(self.refusal({}, AMPERE))

    def test_a_box_without_a_gpu_is_not_this_rules_refusal(self):
        self.assertIsNone(self.refusal({}, NO_GPU, root=NO_ROOT))
        self.assertIsNone(self.refusal({}, {}, root=NO_ROOT))
        self.assertIsNone(self.refusal({}, None, root=NO_ROOT))
        self.assertIsNone(self.refusal({}, {"compute_cap": "12.0"}, root=NO_ROOT))

    def test_nvidia_smi_absent_is_not_this_rules_refusal(self):
        with patch.object(self.m.subprocess, "check_output", side_effect=FileNotFoundError("nvidia-smi")):
            hw = self.m.hardware()
        self.assertEqual(hw["compute_cap"], 0)
        self.assertIsNone(self.refusal({}, hw, root=NO_ROOT))

    def test_only_compute_capability_12_x_is_blackwell(self):
        self.assertEqual(self.refusal({}, dict(BLACKWELL, compute_cap=1210))[0], BLACKWELL_TOKEN_580)
        self.assertIsNone(self.refusal({}, dict(BLACKWELL, compute_cap=1000), root=NO_ROOT), "datacentre Blackwell (10.x) is not this rule")
        self.assertIsNone(self.refusal({}, dict(BLACKWELL, compute_cap=1199), root=NO_ROOT))
        self.assertIsNone(self.refusal({}, dict(BLACKWELL, compute_cap=1300), root=NO_ROOT))

    def test_the_driver_already_loaded_decides_when_no_provision_stage_runs(self):
        # `--only windows_gate` on a box provisioned by hand with 595.91.07: nothing installs a driver
        self.assertIsNone(self.refusal({}, BLACKWELL_595, ["windows_gate"]))

    def test_a_loaded_580_driver_is_refused_when_no_provision_stage_runs(self):
        self.assertEqual(self.refusal({}, BLACKWELL, ["windows_gate"])[0], BLACKWELL_TOKEN_580)

    def test_the_environment_does_not_install_anything_without_a_provision_stage(self):
        # HOST_DRIVER names what provision_host_driver.sh would install; no provision stage, so the loaded 580 is what boots
        self.assertEqual(
            self.refusal({"HOST_DRIVER": "595.91.07"}, BLACKWELL, ["merge_bar", "windows_gate"])[0], BLACKWELL_TOKEN_580
        )

    def test_an_unknown_loaded_driver_is_not_this_rules_refusal(self):
        self.assertIsNone(self.refusal({}, dict(BLACKWELL, driver_version="unknown"), ["windows_gate"]))

    def test_a_selection_that_needs_no_kf3_device_is_not_refused(self):
        # the bare-metal CUDA ladder runs no kf3 device and no PRECOMP row is served to it
        self.assertIsNone(self.refusal({}, BLACKWELL, ["cuda_host"]))

    def test_a_bare_metal_ladder_alone_is_not_refused_whatever_the_selector(self):
        self.assertIsNone(self.refusal({"HOST_DRIVER": "580.159.04"}, BLACKWELL, ["cuda_host"]))
        self.assertIsNone(self.refusal({}, dict(BLACKWELL, driver_version="580.65.06"), ["cuda_host"]))

    def test_a_malformed_selector_is_not_this_rules_refusal(self):
        # the installer refuses a version it cannot fetch; this rule only judges a branch it can read
        self.assertIsNone(self.refusal({"HOST_DRIVER": "latest"}, BLACKWELL))


class MeasuredBranchTableTests(unittest.TestCase):
    setUp = v7.V7Tests.setUp

    HEAD = "key\tdie\tbranch\tsize\tinputs\twords\tsource\tevidence\n"

    def table(self, text):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        path = Path(d.name) / "crates/kf-rm/data/precomp_measured.tsv"
        path.parent.mkdir(parents=True)
        path.write_text(text)
        return Path(d.name)

    def row(self, die, branch, key="display.precomp.0xca73.0x0780"):
        return "%s\t%s\t%s\t4\t-\t0x01df21d0\tmeasured:%s,%s.1,2026-11-03\ttraces/x/y.txt\n" % (key, die, branch, die, branch)

    def test_the_committed_table_measures_branch_595_and_no_other(self):
        self.assertEqual(self.m.precomp_measured(ROOT), {595: ["595.91.07"]})

    def test_the_committed_table_names_the_three_measured_dies(self):
        dies = {
            line.split("\t")[1]
            for line in (ROOT / self.m.PRECOMP_TABLE).read_text().splitlines()
            if line and not line.startswith("#") and not line.startswith("key\t")
        }
        self.assertEqual(dies, {"GB202", "GB203", "GB206"})

    def test_the_payload_reads_the_file_the_rust_loader_includes(self):
        source = (ROOT / "crates/kf-rm/src/consttab.rs").read_text()
        self.assertIn('include_str!("../data/precomp_measured.tsv")', source)
        self.assertEqual(self.m.PRECOMP_TABLE, "crates/kf-rm/data/precomp_measured.tsv")
        # the loader's refusal text is what the box prints; a rename of the key family would leave this rule judging the wrong rows
        self.assertIn("unmeasured on {die} branch {branch}", source)
        self.assertEqual(self.m.PRECOMP_KEY_PREFIX, "display.precomp.")
        self.assertIn('const PREFIX: &str = "display.precomp.";', (ROOT / "crates/kf-rm/src/precomp.rs").read_text())

    def test_the_default_driver_is_the_one_the_provisioning_scripts_install(self):
        host = (ROOT / "scripts/bench/provision_host_driver.sh").read_text()
        full = (ROOT / "scripts/bench/box/provision_full.sh").read_text()
        self.assertIn("DRIVER_VERSION=${HOST_DRIVER:-%s}" % self.m.HOST_DRIVER_DEFAULT, host)
        self.assertIn("${KF_HOST_DRIVER:-%s}" % self.m.HOST_DRIVER_DEFAULT, full)

    def test_a_row_measured_on_branch_580_makes_580_a_measured_branch(self):
        root = self.table(self.HEAD + self.row("GB203", 595) + self.row("GB203", 580))
        self.assertEqual(set(self.m.precomp_measured(root)), {580, 595})
        self.assertIsNone(self.m.host_driver_refusal({"HOST_DRIVER": "580.159.04"}, BLACKWELL, FULL_PLAN, root))

    def test_comments_the_header_and_other_key_families_are_not_branches(self):
        text = "# 2026-11-03 a comment\n" + self.HEAD + self.row("GB202", 595) + self.row("GB202", 580, key="display.other.0x1")
        self.assertEqual(set(self.m.precomp_measured(self.table(text))), {595})

    def test_a_row_of_a_die_that_is_not_blackwell_is_not_a_blackwell_branch(self):
        text = self.HEAD + self.row("GB202", 595) + self.row("AD102", 580)
        self.assertEqual(set(self.m.precomp_measured(self.table(text))), {595})

    def test_an_ada_row_does_not_make_branch_580_a_blackwell_branch(self):
        root = self.table(self.HEAD + self.row("GB202", 595) + self.row("AD102", 580))
        got = self.m.host_driver_refusal({"HOST_DRIVER": "580.159.04"}, BLACKWELL, FULL_PLAN, root)
        self.assertEqual(got[0], BLACKWELL_TOKEN_580)

    def test_a_table_with_no_blackwell_row_is_a_named_refusal_not_a_pass(self):
        got = self.m.host_driver_refusal({"HOST_DRIVER": "595.91.07"}, BLACKWELL, FULL_PLAN, self.table(self.HEAD))
        self.assertEqual(got[0], "PRECOMP_TABLE_UNREADABLE")

    def test_a_missing_table_is_a_named_refusal_for_blackwell_only(self):
        got = self.m.host_driver_refusal({"HOST_DRIVER": "595.91.07"}, BLACKWELL, FULL_PLAN, NO_ROOT)
        self.assertEqual(got[0], "PRECOMP_TABLE_UNREADABLE")
        self.assertIsNone(self.m.host_driver_refusal({}, ADA, FULL_PLAN, NO_ROOT))

    def test_a_malformed_or_empty_table_is_a_named_refusal_not_a_pass(self):
        for name, text in (
            ("empty", ""),
            ("header-only", self.HEAD),
            ("short-row", self.HEAD + "display.precomp.0x1\tGB202\n"),
            ("branch-not-a-number", self.HEAD + self.row("GB202", "x95")),
        ):
            with self.subTest(name):
                got = self.m.host_driver_refusal({"HOST_DRIVER": "595.91.07"}, BLACKWELL, FULL_PLAN, self.table(text))
                self.assertEqual(got[0], "PRECOMP_TABLE_UNREADABLE")


class HostDriverPayloadTests(unittest.TestCase):
    """The rule inside main(): refused before any stage, and nothing else changes."""

    setUp = v7.V7Tests.setUp
    _main_fixture = v7.V7Tests._main_fixture

    def main_run(self, hw, env=None, measured=None, **kw):
        out = io.StringIO()
        with patch.object(self.m, "precomp_measured", return_value=measured or {595: ["595.91.07"]}), contextlib.redirect_stdout(out):
            rc, result, calls = self._main_fixture(initial_hardware=hw, extra_env=env, **kw)
        return rc, result, calls, out.getvalue()

    # --- main(): the fixture's second hardware() answer (after provisioning) is the fixture's own RTX 3060 and is not what is judged here

    def test_main_refuses_a_blackwell_box_with_the_default_driver_before_any_stage(self):
        rc, result, calls, out = self.main_run(BLACKWELL)
        self.assertEqual(rc, 1)
        self.assertEqual(calls, [])
        self.assertIn("KF_PAYLOAD_HOST_DRIVER_REFUSED " + BLACKWELL_TOKEN_580 + "\n", out)
        self.assertIn("KF_PAYLOAD_FAIL suite=preflight\n", out)

    def test_main_records_the_refusal_as_a_failing_preflight_row(self):
        rc, result, calls, out = self.main_run(BLACKWELL)
        self.assertEqual(result["verdict"], "FAIL")
        self.assertIn({"name": "preflight", "pass": 0, "fail": 1, "skip": 0}, result["suites"])
        self.assertEqual(result["hardware"]["compute_cap"], 1200)

    def test_main_prints_the_hint_line_with_the_selector_and_no_url(self):
        rc, result, calls, out = self.main_run(BLACKWELL)
        hint = [line for line in out.splitlines() if line.startswith("KF_PAYLOAD_HOST_DRIVER_HINT ")]
        self.assertEqual(len(hint), 1)
        self.assertIn("KF_HOST_DRIVER", hint[0])
        self.assertIn("595", hint[0])
        self.assertIn("RUN_URL", hint[0])
        self.assertNotRegex(out, r"(?i)https?://")

    def test_main_runs_a_blackwell_box_whose_host_driver_is_on_branch_595(self):
        rc, result, calls, out = self.main_run(BLACKWELL, env={"HOST_DRIVER": "595.91.07"})
        self.assertEqual(rc, 0, result)
        self.assertEqual(calls, ["provision", "cuda_host", "merge_bar", "cuda_guest"])
        self.assertNotIn("KF_PAYLOAD_HOST_DRIVER", out)

    def test_main_does_not_judge_ada_or_ampere_with_the_default_driver(self):
        for name, hw in (("ada", ADA), ("ampere", AMPERE)):
            with self.subTest(name):
                rc, result, calls, out = self.main_run(hw)
                self.assertEqual(rc, 0, result)
                self.assertEqual(calls, ["provision", "cuda_host", "merge_bar", "cuda_guest"])
                self.assertNotIn("KF_PAYLOAD_HOST_DRIVER", out)

    def test_main_does_not_refuse_a_box_without_a_gpu_by_this_rule(self):
        rc, result, calls, out = self.main_run(NO_GPU)
        self.assertNotIn("KF_PAYLOAD_HOST_DRIVER", out)
        self.assertNotEqual(calls, [])

    def test_main_fails_closed_when_the_table_is_missing_on_a_blackwell_box(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc, result, calls = self._main_fixture(initial_hardware=BLACKWELL, extra_env={"HOST_DRIVER": "595.91.07"})
        self.assertEqual(rc, 1)
        self.assertEqual(calls, [])
        self.assertIn("KF_PAYLOAD_HOST_DRIVER_REFUSED PRECOMP_TABLE_UNREADABLE\n", out.getvalue())

    # --- the dry run reads no box state by contract (test_payload_instruction.DryRun: "dry run touched the box"), so it cannot judge a GPU, and it
    # --- is byte for byte what it was whatever the box holds

    def dry_run(self, argv=("--dry-run",), env=None):
        out = io.StringIO()
        args = self.m.parse_args(list(argv))

        def boom(*_a, **_k):
            raise AssertionError("the dry run read the GPU")

        with (
            patch.object(self.m, "hardware", side_effect=boom),
            patch.object(self.m.subprocess, "check_output", side_effect=boom),
            patch.dict(os.environ, env or {}, clear=True),
            contextlib.redirect_stdout(out),
        ):
            rc = self.m.plan_only(args, ROOT)
        return rc, out.getvalue()

    def test_the_dry_run_never_reads_the_gpu_and_prints_the_plain_plan(self):
        rc, out = self.dry_run()
        self.assertEqual(rc, 0)
        self.assertNotIn("KF_PAYLOAD_HOST_DRIVER", out)
        self.assertIn("KF_PAYLOAD_PLAN stages=provision:cuda_host:merge_bar:cuda_guest:windows_gate", out)

    def test_the_dry_run_of_a_selection_never_reads_the_gpu_either(self):
        rc, out = self.dry_run(argv=("--dry-run", "--only", "windows_gate"), env={"HOST_DRIVER": "580.159.04"})
        self.assertEqual(rc, 0)
        self.assertNotIn("KF_PAYLOAD_HOST_DRIVER", out)


if __name__ == "__main__":
    unittest.main()
