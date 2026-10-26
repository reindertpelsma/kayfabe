#!/usr/bin/env python3
"""GPU-free falsifiers for what the box merge bar runs (scripts/bench/box/merge_check.sh).

Measured 2026-10-26 (live gate run 40263112941, RTX 3060): merge_bar failed with rc=101 after 31
minutes in `cargo test` over every kf-* crate. CI's `stable` job runs the same crates on every
commit (GPU-free, KAYFABE_NO_KVM=1) and no kf-* test opens /dev/nvidia*, a kf3 binary or QEMU, so
the box bar no longer repeats them. The only tests that behave differently on a box are the ones
gated on /dev/kvm; they are an OPT-IN stage (KF_MERGE_KVM_TESTS=1, scripts/bench/box/kvm_tests.sh)
whose markers the payload grades.

    KF_MERGE_CHECK=<path>  KF_KVM_TESTS=<path>   aim these tests at a mutated copy
"""

import os
import re
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MERGE_CHECK = Path(os.environ.get("KF_MERGE_CHECK") or ROOT / "scripts/bench/box/merge_check.sh")
KVM_TESTS = Path(os.environ.get("KF_KVM_TESTS") or ROOT / "scripts/bench/box/kvm_tests.sh")


def code(path):
    return "\n".join(
        s for s in Path(path).read_text().splitlines() if not s.lstrip().startswith("#")
    )


class MergeCheckComposition(unittest.TestCase):
    def test_no_cargo_test_runs_in_the_default_bar(self):
        self.assertNotRegex(code(MERGE_CHECK), r"cargo\s+test")

    def test_the_whole_crate_list_is_not_handed_to_a_test_run(self):
        text = code(MERGE_CHECK)
        self.assertNotIn("--no-fail-fast", text)
        self.assertNotRegex(text, r"cargo[^\n]*\"\$\{pk\[@\]\}\"")

    def test_the_legacy_totals_line_is_gone(self):
        self.assertNotIn("TESTS passed", MERGE_CHECK.read_text())

    def test_the_crate_census_stays_as_a_sparse_checkout_guard(self):
        text = code(MERGE_CHECK)
        self.assertIn("v3 crate census truncated", text)
        self.assertRegex(text, r"crates/kf-\*/Cargo.toml")

    def test_kvm_stage_is_opt_in(self):
        text = code(MERGE_CHECK)
        gate = text.index("KF_MERGE_KVM_TESTS")
        call = text.index("kvm_tests.sh")
        self.assertLess(gate, call)
        self.assertRegex(
            text, r'if \[ "\$\{KF_MERGE_KVM_TESTS:-0\}" = 1 \]; then\n[^\n]*kvm_tests\.sh'
        )

    def test_stages_after_the_census_are_unchanged(self):
        text = code(MERGE_CHECK)
        order = [
            "v3_gates.sh",
            "build_kf3.sh",
            "bare_metal_suite.sh",
            "fast_suite.sh",
            "birth_census.sh",
        ]
        positions = [text.index(s) for s in order]
        self.assertEqual(positions, sorted(positions))

    def test_the_header_names_what_the_bar_is(self):
        head = "\n".join(MERGE_CHECK.read_text().splitlines()[:12])
        self.assertNotIn("every kf-* crate test", head)
        self.assertIn("CI", head)


class KvmTestsBehaviour(unittest.TestCase):
    """Run kvm_tests.sh against a fake `cargo` that prints a chosen libtest log."""

    def run_script(self, log, rc):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            bin_dir = root / "bin"
            bin_dir.mkdir()
            (root / "fake.log").write_text(log)
            cargo = bin_dir / "cargo"
            cargo.write_text(
                '#!/bin/bash\necho "$@" > "$FAKE_ARGS"\ncat "$FAKE_LOG"\nexit "$FAKE_RC"\n'
            )
            cargo.chmod(cargo.stat().st_mode | stat.S_IEXEC)
            prov = root / "prov"
            prov.mkdir()
            env = dict(
                os.environ,
                PATH=str(bin_dir) + ":" + os.environ["PATH"],
                FAKE_LOG=str(root / "fake.log"),
                FAKE_RC=str(rc),
                FAKE_ARGS=str(root / "args"),
            )
            p = subprocess.run(
                ["bash", str(KVM_TESTS), str(prov), "tagx"],
                env=env,
                cwd=root,
                capture_output=True,
                text=True,
                timeout=60,
            )
            args = (root / "args").read_text() if (root / "args").exists() else ""
            kept = (
                (prov / "tagx_tests.log").read_text() if (prov / "tagx_tests.log").exists() else ""
            )
            return p.returncode, p.stdout, args, kept

    GOOD = (
        "KVM-GATE: RAN a_test\nKVM-GATE: RAN b_test\n"
        "test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"
        "test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"
    )

    def test_it_tests_only_the_kvm_crates(self):
        _, _, args, _ = self.run_script(self.GOOD, 0)
        self.assertIn("-p kf-linux-raw", args)
        self.assertIn("-p kf-chan", args)
        self.assertNotIn("kf-rm", args)
        self.assertIn("--no-fail-fast", args)

    def test_a_clean_run_prints_the_request_and_the_totals(self):
        rc, out, _, _ = self.run_script(self.GOOD, 0)
        self.assertEqual(rc, 0, out)
        self.assertEqual(out.splitlines()[0], "KVM_TESTS_REQUESTED=1")
        self.assertIn("KVM_TESTS passed 12 failed 0 kvm_ran=2 kvm_skipped=0", out.splitlines())
        self.assertNotIn("KVM_TESTS_RC", out)

    def test_its_log_is_kept_where_the_payload_reads_it(self):
        _, _, _, kept = self.run_script(self.GOOD, 0)
        self.assertEqual(kept, self.GOOD)

    def test_a_failing_cargo_is_red_with_its_rc(self):
        bad = self.GOOD.replace(
            "0 failed; 0 ignored; 0 measured; 0 filtered out\ntest result: ok. 5",
            "1 failed; 0 ignored; 0 measured; 0 filtered out\ntest result: FAILED. 5",
        )
        rc, out, _, _ = self.run_script(bad, 101)
        self.assertNotEqual(rc, 0)
        self.assertIn("KVM_TESTS_RC=101", out.splitlines())

    def test_a_skipped_gate_is_red_even_when_cargo_passes(self):
        log = self.GOOD + "KVM-GATE: SKIPPED c_test — /dev/kvm is absent or not permitted\n"
        rc, out, _, _ = self.run_script(log, 0)
        self.assertNotEqual(rc, 0)
        self.assertIn("kvm_skipped=1", out)

    def test_a_run_that_reached_no_kvm_test_is_red(self):
        log = "test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"
        rc, out, _, _ = self.run_script(log, 0)
        self.assertNotEqual(rc, 0)
        self.assertIn("kvm_ran=0", out)

    def test_zero_tests_is_red(self):
        rc, out, _, _ = self.run_script("KVM-GATE: RAN a_test\n", 0)
        self.assertNotEqual(rc, 0)
        self.assertIn("KVM_TESTS passed 0 failed 0", out)


if __name__ == "__main__":
    unittest.main()
