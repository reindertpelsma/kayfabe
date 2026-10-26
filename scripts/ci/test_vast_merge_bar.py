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

    def test_the_census_precedes_every_stage_and_exits_2(self):
        text = code(MERGE_CHECK)
        census = text.index("v3 crate census truncated")
        self.assertLess(census, text.index("v3_gates.sh"))
        self.assertLess(census, text.index("kvm_tests.sh"))
        self.assertRegex(text[census : census + 80], r"exit 2")

    def test_the_kvm_stage_runs_only_for_the_flag_value_1(self):
        text = code(MERGE_CHECK)
        block = re.search(
            r'if \[ "\$\{KF_MERGE_KVM_TESTS:-0\}" = 1 \]; then\n(.*?)\nfi\n', text, re.S
        )
        self.assertIsNotNone(block)
        script = (
            'PROV=p; T=t\nif [ "${KF_MERGE_KVM_TESTS:-0}" = 1 ]; then\n'
            + block[1].replace("bash scripts/bench/box/kvm_tests.sh", "echo CALLED")
            + "\nfi\n"
        )
        for value, called in (
            ("1", True),
            ("0", False),
            ("yes", False),
            ("", False),
            (None, False),
        ):
            with self.subTest(repr(value)):
                env = {k: v for k, v in os.environ.items() if k != "KF_MERGE_KVM_TESTS"}
                if value is not None:
                    env["KF_MERGE_KVM_TESTS"] = value
                out = subprocess.run(
                    ["bash", "-c", script], env=env, capture_output=True, text=True, timeout=30
                ).stdout
                self.assertEqual("CALLED" in out, called, out)

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


class MergeCheckRun(unittest.TestCase):
    """Run the real merge_check.sh in a one-crate git fixture: it must stop at the census."""

    def run_bar(self, **extra):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d) / "repo"
            (root / "scripts/bench/box").mkdir(parents=True)
            (root / "crates/kf-only").mkdir(parents=True)
            (root / "crates/kf-only/Cargo.toml").write_text("[package]\nname='kf-only'\n")
            for name in ("merge_check.sh", "kvm_tests.sh"):
                source = MERGE_CHECK if name == "merge_check.sh" else KVM_TESTS
                (root / "scripts/bench/box" / name).write_text(source.read_text())
            git = ["git", "-C", str(root), "-c", "user.name=t", "-c", "user.email=t@t"]
            subprocess.run(["git", "init", "-q", "-b", "main", str(root)], check=True)
            subprocess.run([*git, "add", "-A"], check=True)
            subprocess.run([*git, "commit", "-q", "-m", "x"], check=True)
            subprocess.run([*git, "remote", "add", "origin", str(root)], check=True)
            env = dict(
                os.environ,
                BENCH_DIR=str(Path(d) / "bench"),
                KF_PROV_DIR=str(Path(d) / "prov"),
                CARGO_TARGET_DIR=str(Path(d) / "target"),
                **extra,
            )
            p = subprocess.run(
                ["bash", str(root / "scripts/bench/box/merge_check.sh"), "main", "tag1"],
                env=env,
                capture_output=True,
                text=True,
                timeout=120,
            )
            return p.returncode, (Path(d) / "prov/tag1.log").read_text()

    def run_or_skip(self, **extra):
        rc, log = self.run_bar(**extra)
        if "another merge check is running" in log:
            self.skipTest("a real merge check holds /tmp/kayfabe-merge-check.lock")
        return rc, log

    def test_a_truncated_checkout_stops_at_the_census_with_rc_2(self):
        rc, log = self.run_or_skip()
        self.assertEqual(rc, 2, log)
        self.assertIn("v3 crate census truncated", log)
        self.assertRegex(log, r"EXIT rc=2 ")
        self.assertNotIn("GATES_RC", log)

    def test_the_kvm_flag_does_not_run_tests_on_a_truncated_checkout(self):
        rc, log = self.run_or_skip(KF_MERGE_KVM_TESTS="1")
        self.assertEqual(rc, 2, log)
        self.assertNotIn("KVM_TESTS_REQUESTED", log)

    def test_no_box_script_prints_the_legacy_totals_line(self):
        for script in (MERGE_CHECK.parent).glob("*.sh"):
            with self.subTest(script.name):
                self.assertNotIn("TESTS passed", code(script).replace("KVM_TESTS passed", ""))


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

    def test_the_cargo_command_line_is_exactly_the_two_kvm_crates(self):
        _, _, args, _ = self.run_script(self.GOOD, 0)
        self.assertEqual(args.split(), "test -q --no-fail-fast -p kf-linux-raw -p kf-chan".split())

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

    def test_several_skipped_gates_are_red(self):
        log = self.GOOD + "".join("KVM-GATE: SKIPPED t%d \u2014 x\n" % i for i in range(3))
        rc, out, _, _ = self.run_script(log, 0)
        self.assertNotEqual(rc, 0)
        self.assertIn("kvm_skipped=3", out)

    def test_cargo_ok_with_no_gate_markers_at_all_is_red(self):
        log = "test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"
        rc, out, _, _ = self.run_script(log, 0)
        self.assertNotEqual(rc, 0)
        self.assertIn("kvm_ran=0 kvm_skipped=0", out)

    def test_a_build_error_reports_its_rc_and_is_red(self):
        rc, out, _, _ = self.run_script("error[E0432]: unresolved import `x`\n", 2)
        self.assertNotEqual(rc, 0)
        self.assertIn("KVM_TESTS_RC=2", out.splitlines())

    def test_the_log_of_a_failing_run_is_kept(self):
        _, _, _, kept = self.run_script("t::a --- FAILED\n", 101)
        self.assertEqual(kept, "t::a --- FAILED\n")

    def test_a_run_that_reached_no_kvm_test_is_red(self):
        log = "test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"
        rc, out, _, _ = self.run_script(log, 0)
        self.assertNotEqual(rc, 0)
        self.assertIn("kvm_ran=0", out)

    def test_a_filtered_out_run_with_zero_tests_is_red(self):
        log = "KVM-GATE: RAN a_test\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured\n"
        rc, out, _, _ = self.run_script(log, 0)
        self.assertNotEqual(rc, 0)
        self.assertIn("KVM_TESTS passed 0 failed 0", out)

    def test_a_non_zero_cargo_rc_is_red_even_when_the_totals_look_green(self):
        for code_ in (101, 2):
            with self.subTest(code_):
                rc, out, _, _ = self.run_script(self.GOOD, code_)
                self.assertNotEqual(rc, 0)
                self.assertIn("KVM_TESTS_RC=%d" % code_, out.splitlines())

    def test_a_crashed_test_binary_is_red_not_a_pass(self):
        rc, out, _, _ = self.run_script(
            self.GOOD + "error: test failed, to rerun pass `-p kf-chan`\n", 101
        )
        self.assertNotEqual(rc, 0)
        self.assertIn("KVM_TESTS passed 12 failed 0 kvm_ran=2 kvm_skipped=0", out.splitlines())

    def test_the_request_line_is_printed_even_when_cargo_fails(self):
        _, out, _, _ = self.run_script("boom\n", 101)
        self.assertEqual(out.splitlines()[0], "KVM_TESTS_REQUESTED=1")

    def test_zero_tests_is_red(self):
        rc, out, _, _ = self.run_script("KVM-GATE: RAN a_test\n", 0)
        self.assertNotEqual(rc, 0)
        self.assertIn("KVM_TESTS passed 0 failed 0", out)


if __name__ == "__main__":
    unittest.main()
