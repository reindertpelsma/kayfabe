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
LADDER = ROOT / "scripts/ci/ignore_ladder.py"


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

    def test_the_strip_runs_in_the_checkout_after_the_reset_and_before_every_stage(self):
        text = code(MERGE_CHECK)
        strip = text.index("ignore_ladder.py strip --in-place")
        self.assertLess(text.index("reset -q --hard"), strip)
        self.assertLess(text.index('cd "$CHECKOUT"'), strip)
        for later in ("crates/kf-*/Cargo.toml", "kvm_tests.sh", "v3_gates.sh", "build_kf3.sh"):
            self.assertLess(strip, text.index(later), later)
        self.assertIn("exit 2", text[strip : strip + 200])

    def test_the_stripped_checkout_is_verified_right_after_the_strip_and_before_every_stage(self):
        text = code(MERGE_CHECK)
        strip = text.index("ignore_ladder.py strip --in-place")
        verify = text.index("ignore_ladder.py strip-verify")
        self.assertLess(strip, verify)
        for later in ("crates/kf-*/Cargo.toml", "kvm_tests.sh", "v3_gates.sh", "build_kf3.sh"):
            self.assertLess(verify, text.index(later), later)
        self.assertIn("exit 2", text[verify : verify + 250])

    def test_a_failed_strip_verify_stops_the_run_with_a_named_message(self):
        text = code(MERGE_CHECK)
        verify = text.index("ignore_ladder.py strip-verify")
        line = text[verify : text.index("\n", verify)]
        self.assertIn("STRIP_VERIFY_FAILED", line)
        self.assertIn("exit 2", line)

    def test_oracles_are_required_on_the_box_unless_a_caller_says_otherwise(self):
        text = code(MERGE_CHECK)
        self.assertIn("export KF_REQUIRE_ORACLES=${KF_REQUIRE_ORACLES:-1}", text)
        self.assertLess(
            text.index("KF_REQUIRE_ORACLES"), text.index("kvm_tests.sh"), "before the first stage"
        )

    def test_the_header_names_what_the_bar_is(self):
        head = "\n".join(MERGE_CHECK.read_text().splitlines()[:12])
        self.assertNotIn("every kf-* crate test", head)
        self.assertIn("CI", head)


class MergeCheckRun(unittest.TestCase):
    """Run the real merge_check.sh in a one-crate git fixture: it must stop at the census."""

    WORKERIGNORE = "/.git\n/.workerignore\n/docs\n**/*.md\n"

    def run_bar(self, workerignore=WORKERIGNORE, ladder=True, args=(), crates=1, stubs=False, tree=False,
                break_origin=False, **extra):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d) / "repo"
            (root / "scripts/bench/box").mkdir(parents=True)
            (root / "scripts/ci").mkdir(parents=True)
            for i in range(crates):
                (root / f"crates/kf-only{i or ''}").mkdir(parents=True)
                (root / f"crates/kf-only{i or ''}/Cargo.toml").write_text("[package]\nname='kf-only'\n")
            if stubs:  # record which stages the script reaches; every stage passes
                for rel, line in (
                    ("scripts/bench/v3_gates.sh", 'echo "V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS" > "$1"'),
                    ("scripts/bench/build_kf3.sh", "echo kf3-built"),
                    ("scripts/fastguest/bare_metal_suite.sh", 'echo "BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30"'),
                    ("scripts/fastguest/build_fast_guest.sh", "echo fg"),
                    ("scripts/fastguest/fast_suite.sh", 'echo "FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30" > "$BENCH_DIR/$1_suite.out"'),
                    ("scripts/bench/box/birth_census.sh", 'echo BIRTH_CENSUS_SUITE; echo BIRTH_CENSUS_GATES; echo BIRTH_CENSUS_OK'),
                ):
                    f = root / rel
                    f.parent.mkdir(parents=True, exist_ok=True)
                    f.write_text(f'#!/bin/bash\necho "STAGE {Path(rel).name}" >> "$STAGE_LOG"\n{line}\n')
            (root / "docs").mkdir()
            (root / "docs/ignored.txt").write_text("doc\n")
            if workerignore is not None:
                (root / ".workerignore").write_text(workerignore)
            if ladder:
                (root / "scripts/ci/ignore_ladder.py").write_text(LADDER.read_text())
            for name in ("merge_check.sh", "kvm_tests.sh"):
                source = MERGE_CHECK if name == "merge_check.sh" else KVM_TESTS
                (root / "scripts/bench/box" / name).write_text(source.read_text())
            git = ["git", "-C", str(root), "-c", "user.name=t", "-c", "user.email=t@t"]
            subprocess.run(["git", "init", "-q", "-b", "main", str(root)], check=True)
            subprocess.run([*git, "add", "-A"], check=True)
            subprocess.run([*git, "commit", "-q", "-m", "x"], check=True)
            subprocess.run([*git, "remote", "add", "origin", str(Path(d) / "nowhere") if break_origin else str(root)], check=True)
            fakebin = Path(d) / "fakebin"
            fakebin.mkdir()
            (fakebin / "cargo").write_text("#!/bin/bash\necho STAGE cargo >> \"$STAGE_LOG\"\n")
            (fakebin / "cargo").chmod(0o755)
            env = dict(
                os.environ,
                PATH=f"{fakebin}:{os.environ['PATH']}",
                STAGE_LOG=str(Path(d) / "stages"),
                BENCH_DIR=str(Path(d) / "bench"),
                KF_PROV_DIR=str(Path(d) / "prov"),
                CARGO_TARGET_DIR=str(Path(d) / "target"),
                **extra,
            )
            p = subprocess.run(
                ["bash", str(root / "scripts/bench/box/merge_check.sh"), *args, *(["--tree", str(root)] if tree else []), "main", "tag1"],
                env=env,
                capture_output=True,
                text=True,
                timeout=120,
            )
            self.stages = (Path(d) / "stages").read_text().split("\n") if (Path(d) / "stages").exists() else []
            return p.returncode, (Path(d) / "prov/tag1.log").read_text() if (Path(d) / "prov/tag1.log").exists() else p.stderr

    def run_or_skip(self, **extra):
        rc, log = self.run_bar(**extra)
        if "another merge check is running" in log:
            self.skipTest("a real merge check holds /tmp/kayfabe-merge-check.lock")
        return rc, log

    # --- S9 (2026-11-15): --no-fetch, --tree and per-section flags --------------------------------
    def stage_names(self):
        return [l.split()[1] for l in self.stages if l.startswith("STAGE ")]

    def test_the_default_runs_every_section_in_bar_order(self):
        rc, log = self.run_or_skip(crates=36, stubs=True)
        self.assertEqual(rc, 0, log)
        self.assertEqual(self.stage_names(), ["v3_gates.sh", "build_kf3.sh", "cargo", "bare_metal_suite.sh",
                                              "build_fast_guest.sh", "fast_suite.sh", "birth_census.sh", "birth_census.sh"])

    def test_one_section_flag_runs_only_that_section(self):
        for flag, want in (("--gates", ["v3_gates.sh"]), ("--kf3", ["build_kf3.sh"]),
                           ("--host", ["cargo", "bare_metal_suite.sh"]),
                           ("--fast", ["cargo", "build_fast_guest.sh", "fast_suite.sh"]),
                           ("--census", ["birth_census.sh", "birth_census.sh"])):
            with self.subTest(flag):
                rc, log = self.run_or_skip(crates=36, stubs=True, args=(flag,))
                if flag == "--census":
                    self.assertEqual(self.stage_names(), want, log)  # the stub census passes without logs
                    continue
                self.assertEqual(rc, 0, log)
                self.assertEqual(self.stage_names(), want, log)
                self.assertIn("SECTIONS= " + flag[2:] + " ", log)

    def test_two_flags_run_in_bar_order_not_flag_order(self):
        rc, log = self.run_or_skip(crates=36, stubs=True, args=("--fast", "--gates"))
        self.assertEqual(self.stage_names(), ["v3_gates.sh", "cargo", "build_fast_guest.sh", "fast_suite.sh"], log)

    def test_an_unknown_option_or_a_wrong_positional_count_is_refused_before_any_log(self):
        for args in (("--nope",),):
            rc, log = self.run_bar(args=args)
            self.assertEqual(rc, 2)
            self.assertIn("unknown option", log)

    def test_no_fetch_never_asks_origin(self):
        rc, log = self.run_or_skip(break_origin=True)
        self.assertEqual(rc, 128, log)  # the broken origin makes the default fetch fail (git's rc under set -e)
        rc, log = self.run_or_skip(break_origin=True, args=("--no-fetch",))
        self.assertEqual(rc, 2, log)
        self.assertIn("NOFETCH=1", log)
        self.assertIn("v3 crate census truncated", log)  # got past the checkout, the strip and verify

    def test_tree_uses_the_checkout_as_is_with_no_worktree_and_no_strip(self):
        rc, log = self.run_or_skip(tree=True)
        self.assertEqual(rc, 2, log)
        self.assertIn("v3 crate census truncated", log)
        self.assertNotIn("IGNORE_STRIP", log)
        self.assertRegex(log, r"TREE=/\S+repo ")
        self.assertNotIn("verify-worktrees", log)

    def test_tree_must_be_a_git_checkout(self):
        with tempfile.TemporaryDirectory() as d:
            p = subprocess.run(["bash", str(MERGE_CHECK), "--tree", d, "-", "tag1"], capture_output=True, text=True,
                               timeout=30, env=dict(os.environ, KF_PROV_DIR=d))
            self.assertEqual(p.returncode, 2)
            self.assertIn("not a git checkout", p.stderr)

    def test_a_truncated_checkout_stops_at_the_census_with_rc_2(self):
        rc, log = self.run_or_skip()
        self.assertEqual(rc, 2, log)
        self.assertIn("v3 crate census truncated", log)
        self.assertRegex(log, r"EXIT rc=2 ")
        self.assertNotIn("GATES_RC", log)

    def test_the_checkout_is_stripped_before_the_census_and_the_result_is_logged(self):
        rc, log = self.run_or_skip()
        self.assertRegex(log, r"IGNORE_STRIP kept=\d+ removed=\d+ rev=HEAD")
        self.assertRegex(log, r"IGNORE_STRIP_KEY [0-9a-f]{64}")
        self.assertLess(log.index("IGNORE_STRIP kept="), log.index("v3 crate census truncated"))

    def test_the_log_says_oracles_are_required_by_default(self):
        rc, log = self.run_or_skip()
        self.assertIn("KF_REQUIRE_ORACLES=1\n", log)

    def test_a_caller_can_opt_out_of_required_oracles_and_the_log_says_so(self):
        rc, log = self.run_or_skip(KF_REQUIRE_ORACLES="0")
        self.assertIn("KF_REQUIRE_ORACLES=0\n", log)
        self.assertNotIn("KF_REQUIRE_ORACLES=1\n", log)

    def test_an_unreadable_ignore_file_stops_the_run_before_any_stage_with_rc_2(self):
        rc, log = self.run_or_skip(workerignore="!/crates\n")
        self.assertEqual(rc, 2, log)
        self.assertIn("STRIP_FAILED", log)
        self.assertIn("IGNORE_LADDER_FAIL_CLOSED", log)
        self.assertNotIn("census", log)
        self.assertNotIn("GATES_RC", log)

    def test_a_commit_without_an_ignore_file_stops_the_run_with_rc_2(self):
        rc, log = self.run_or_skip(workerignore=None)
        self.assertEqual(rc, 2, log)
        self.assertIn("STRIP_FAILED", log)
        self.assertNotIn("census", log)

    def test_an_ignore_set_that_removes_crate_code_stops_the_run_with_rc_2(self):
        rc, log = self.run_or_skip(workerignore=self.WORKERIGNORE + "/crates/kf-only\n")
        self.assertEqual(rc, 2, log)
        self.assertIn("STRIP_FAILED", log)
        self.assertNotIn("census", log)

    def test_a_checkout_that_lacks_the_strip_tool_stops_the_run_with_rc_2(self):
        rc, log = self.run_or_skip(ladder=False)
        self.assertEqual(rc, 2, log)
        self.assertIn("STRIP_FAILED", log)
        self.assertNotIn("census", log)

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
