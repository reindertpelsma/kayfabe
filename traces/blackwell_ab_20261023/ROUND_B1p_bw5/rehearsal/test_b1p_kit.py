"""BW5 round B1p run kit: the fix revision b85d02bae, pinned and checked (tests first).

The B1p plan reuses the B1n kit (capture, collection boots, fatal collection, guard) unchanged
and changes only (a) the binary every arm runs: the pinned fix revision, built on the box with
its GPU-free tests and checked to carry the route log line, and (b) the arm plan, in the task's
priority order: on_fix (default flags, 10-minute hold), on_fix_pnp (the existing single
DIAGNOSTIC disable/enable cycle), on_fix_hold2 (the main arm repeated). The plan-independent
B1n suites (Present, CapturePack, CapturePull, FatalCollect, ScriptShapes) run on this kit too.
"""

import copy
import datetime as dt
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
K = Path(os.environ.get("KF_BW3_K", HERE.parent / "attempt1" / "run_kit"))
os.environ["KF_BW4_K"] = str(K)  # the reused B1n suites read the kit from here
BW = Path(os.environ.get("KF_BW4_BW", HERE.parents[3] / "scripts/bench/blackwell"))
REPO = HERE.parents[3]
B1N_REHEARSAL = HERE.parent.parent / "ROUND_B1n_bw4" / "rehearsal"
sys.path.insert(0, str(BW))
sys.path.insert(0, str(K))
sys.path.insert(0, str(B1N_REHEARSAL))

from test_b1n_kit import CapturePack, CapturePull, FatalCollect, Present, ScriptShapes  # noqa
from test_b1n_snippets import heredoc  # noqa: E402

FIX = "b85d02bae3ed085a10cd1c8376c1a842e86c571b"
RED = "e3bae55ddebe35e95809c06d7195152a4679aa31"
BASE = "cb80cbd29b7a05f904acd822f172485fd49938a9"
LOG_LINE = "USER_RING_ROUTE_DECLINED family="
PLAN = ["on_fix", "on_fix_pnp", "on_fix_hold2"]
NOW = dt.datetime(2026, 10, 24, 12, 0, tzinfo=dt.timezone.utc)
START = NOW - dt.timedelta(minutes=30)
END = START + dt.timedelta(minutes=150)


def load(name, path):
    if not Path(path).exists():
        return None
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


A = load("b1p_arms", K / "b1n_arms.py")
AUTH = load("b1n_authority", K / "b1n_authority.py")
FB = load("fix_build", K / "fix_build.py")
O = load("saved_xid32_observation_b1p", BW / "saved_xid32_observation.py")


def text(name):
    return (K / name).read_text()


class B1pArmPlan(unittest.TestCase):
    def test_priority_order_is_the_task_order(self):
        self.assertEqual([arm["run"] for arm in A.ARMS], PLAN)
        self.assertTrue(A.validate())

    def test_main_arms_run_default_flags_and_one_enable(self):
        arms = {arm["run"]: arm for arm in A.ARMS}
        for run in ("on_fix", "on_fix_hold2"):
            arm = arms[run]
            self.assertEqual(arm["kind"], "windows", run)
            self.assertEqual(arm["env"], {}, run)
            self.assertIsNone(arm["prep"], run)
            self.assertEqual(arm["enable"], "enable580_once.ps1", run)
            self.assertEqual(arm["enable_count"], 1, run)
            self.assertEqual(arm["label"], "MEASUREMENT", run)

    def test_cycle_arm_is_the_existing_single_diagnostic_cycle(self):
        arm = {arm["run"]: arm for arm in A.ARMS}["on_fix_pnp"]
        self.assertEqual(arm["enable"], "enable580_cycle.ps1")
        self.assertEqual(arm["enable_count"], 2)
        self.assertEqual(arm["label"], "DIAGNOSTIC")
        self.assertEqual(arm["env"], {})
        self.assertIsNone(arm["prep"])

    def test_no_arm_is_linux_and_no_arm_carries_a_relay_flag(self):
        for arm in A.ARMS:
            self.assertEqual(arm["kind"], "windows", arm["run"])
            self.assertFalse(arm["env"], arm["run"])

    def bad(self, **change):
        arms = copy.deepcopy(list(A.ARMS))
        index = change.pop("index", 0)
        arms[index].update(change)
        with self.assertRaises(ValueError):
            A.validate(arms)

    def test_shadow_flag_is_not_admitted(self):
        # CODE READ at b85d02bae: shadow acts only on the private ring, which GB20x no longer takes.
        self.bad(env={"KF3_WIN_USER_RING_SHADOW": "1"})

    def test_cycle_count_above_two_refuses(self):
        self.bad(index=1, enable_count=4)

    def test_second_enable_needs_the_cycle_and_diagnostic(self):
        self.bad(index=0, enable_count=2)
        self.bad(index=1, label="MEASUREMENT")
        self.bad(index=1, enable="enable580_once.ps1")

    def test_prepare_writes_an_empty_env_for_the_default_arms(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(A.prepare("on_fix", prov=tmp), "enable580_once.ps1 1 - MEASUREMENT")
            self.assertEqual((Path(tmp) / "arm_env_on_fix.sh").read_text(), "")
            self.assertEqual(
                A.prepare("on_fix_pnp", prov=tmp), "enable580_cycle.ps1 2 - DIAGNOSTIC"
            )
            self.assertEqual(
                A.prepare("on_fix_hold2", prov=tmp), "enable580_once.ps1 1 - MEASUREMENT"
            )
            with self.assertRaises(ValueError):
                A.prepare("on_urt", prov=tmp)


class B1pAuthority(unittest.TestCase):
    def build(self, die="GB205"):
        return AUTH.build(lease_id=57500002, die=die, start=START, end=END, table=O.DIE_PAIRS)

    def test_authority_names_the_b1p_arms_and_the_cycle(self):
        a = self.build()
        self.assertEqual(a["arms"], PLAN)
        self.assertEqual(a["diagnostic_enable_arms"], {"on_fix_pnp": 2})
        self.assertEqual(a["exact_channels"], ["0x14", "0x1000000b"])
        self.assertEqual(self.build("GB202")["exact_channels"], ["0x14", "0x0e00000b"])

    def test_every_b1p_permit_is_accepted_by_the_unchanged_guard(self):
        a = self.build()
        with tempfile.TemporaryDirectory() as tmp:
            probed = Path(tmp) / "probed_die.json"
            probed.write_text(json.dumps({"die": "GB205"}))
            for run in PLAN:
                phase = {"arm": run, "pid": 4242, "starttime": 9, "exe": "/x/q", "pci": "0:0:7"}
                permit = {**phase, "mode": O.MODE, "profile": "on", "lease_id": 57500002}
                permit.update(armed_utc=(NOW - dt.timedelta(minutes=1)).isoformat())
                permit.update(A.permit_fields(run, probed))
                self.assertTrue(O.scope_enabled(phase, permit, a, NOW), run)

    def test_guard_ceiling_is_unchanged(self):
        self.assertEqual(O.MAX_DIAGNOSTIC_ENABLES, 2)
        self.assertEqual(
            O.DIE_PAIRS, {"GB205": ("0x14", "0x1000000b"), "GB202": ("0x14", "0x0e00000b")}
        )


class FixPin(unittest.TestCase):
    def pin(self):
        return json.loads(text("fix_pin.json"))

    def test_pin_names_the_fix_its_red_and_its_base(self):
        pin = self.pin()
        self.assertEqual(pin["source"], FIX)
        self.assertEqual(pin["red"], RED)
        self.assertEqual(pin["base"], BASE)
        self.assertEqual(pin["bin_dir"], FIX[:8])
        self.assertEqual(pin["log_line"], LOG_LINE)
        self.assertEqual(pin["log_source"], "crates/kf-qemu/src/userrelay.rs")
        self.assertEqual(FB.load_pin(K / "fix_pin.json"), pin)

    def test_pin_matches_the_repository_objects(self):
        git = ["git", "-C", str(REPO)]
        for rev, want in ((FIX + "^", RED), (FIX + "^^", BASE)):
            got = subprocess.check_output([*git, "rev-parse", rev], text=True).strip()
            self.assertEqual(got, want, rev)
        source = subprocess.check_output(
            [*git, "show", FIX + ":crates/kf-qemu/src/userrelay.rs"], text=True
        )
        self.assertIn("kf3: " + LOG_LINE, source)
        base = subprocess.check_output(
            [*git, "show", BASE + ":crates/kf-qemu/src/userrelay.rs"], text=True
        )
        self.assertNotIn(LOG_LINE, base)

    def test_load_pin_refuses_a_bin_dir_or_hash_mismatch(self):
        with tempfile.TemporaryDirectory() as tmp:
            for change in ({"bin_dir": "cb80cbd2"}, {"source": "b85d02ba"}, {"log_line": ""}):
                path = Path(tmp) / "pin.json"
                path.write_text(json.dumps({**self.pin(), **change}))
                with self.subTest(change=change), self.assertRaises(ValueError):
                    FB.load_pin(path)


def chain(tmp):
    """A real three-commit chain base -> red -> fix with the log line, like the pinned one."""
    repo = Path(tmp) / "repo"
    (repo / "crates/kf-qemu/src").mkdir(parents=True)
    git = ["git", "-C", str(repo), "-c", "user.name=t", "-c", "user.email=t@t"]
    subprocess.run(["git", "init", "-q", str(repo)], check=True)
    revs = []
    for body in ("base\n", "red\n", 'eprintln!("kf3: ' + LOG_LINE + '{x}");\n'):
        (repo / "crates/kf-qemu/src/userrelay.rs").write_text(body)
        subprocess.run([*git, "add", "-A"], check=True)
        subprocess.run([*git, "commit", "-qm", body[:4]], check=True)
        revs.append(subprocess.check_output([*git, "rev-parse", "HEAD"], text=True).strip())
    pin = {
        "source": revs[2],
        "red": revs[1],
        "base": revs[0],
        "bin_dir": revs[2][:8],
        "log_line": LOG_LINE,
        "log_source": "crates/kf-qemu/src/userrelay.rs",
    }
    return repo, pin, git


OK_LOG = (
    "test result: ok. 410 passed; 0 failed; 2 ignored; 0 measured\n"
    "test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured\n"
)


class FixBuild(unittest.TestCase):
    def test_source_check_accepts_the_exact_clean_chain(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo, pin, _ = chain(tmp)
            self.assertEqual(FB.check_source(repo, pin), pin["source"])

    def test_source_check_refuses_another_head_a_wrong_chain_or_a_dirty_tree(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo, pin, git = chain(tmp)
            for change in ({"source": pin["red"]}, {"red": pin["base"]}, {"base": pin["red"]}):
                with self.subTest(change=change), self.assertRaises(ValueError):
                    FB.check_source(repo, {**pin, **change})
            with self.assertRaises(ValueError):
                FB.check_source(repo, {**pin, "log_line": "NOT_IN_SOURCE"})
            (repo / "crates/kf-qemu/src/userrelay.rs").write_text("edited\n")
            with self.assertRaises(ValueError):
                FB.check_source(repo, pin)

    def test_tests_gate_needs_passes_and_no_failure(self):
        self.assertEqual(FB.tests_ok(OK_LOG), (419, 2))
        for bad in ("", OK_LOG + "test result: FAILED. 1 passed; 1 failed;\n", "error[E0432]\n"):
            with self.subTest(bad=bad[:20]), self.assertRaises(ValueError):
                FB.tests_ok(bad)
        with self.assertRaises(ValueError):
            FB.tests_ok("test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured\n")

    def receipt_env(self, tmp, binary_bytes):
        repo, pin, _ = chain(tmp)
        bench = Path(tmp) / "bench"
        binary = bench / "kf3-bins" / pin["bin_dir"] / "qemu-system-x86_64"
        binary.parent.mkdir(parents=True)
        binary.write_bytes(binary_bytes)
        tests = Path(tmp) / "tests.log"
        tests.write_text(OK_LOG)
        build = Path(tmp) / "build.log"
        build.write_text("built\n")
        return repo, pin, bench, tests, build, binary

    def test_receipt_records_source_sha256_and_the_route_line(self):
        with tempfile.TemporaryDirectory() as tmp:
            body = b"\x7fELF" + b"x" * 64 + LOG_LINE.encode() + b"Blackwell"
            repo, pin, bench, tests, build, binary = self.receipt_env(tmp, body)
            row = FB.receipt(pin, repo, bench, tests, build)
            self.assertEqual(row["source"], pin["source"])
            self.assertEqual(row["base"], pin["base"])
            self.assertEqual(row["binary"], str(binary))
            self.assertEqual(row["binary_sha256"], hashlib.sha256(body).hexdigest())
            self.assertIs(row["tests_passed"], True)
            self.assertEqual((row["test_passes"], row["test_ignored"]), (419, 2))
            self.assertIs(row["route_log_line_in_binary"], True)

    def test_receipt_refuses_a_binary_without_the_route_line(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo, pin, bench, tests, build, _ = self.receipt_env(tmp, b"\x7fELF old binary")
            with self.assertRaises(ValueError):
                FB.receipt(pin, repo, bench, tests, build)

    def test_receipt_refuses_a_missing_binary_or_failed_tests(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo, pin, bench, tests, build, binary = self.receipt_env(tmp, LOG_LINE.encode())
            tests.write_text(OK_LOG + "test result: FAILED. 0 passed; 1 failed;\n")
            with self.assertRaises(ValueError):
                FB.receipt(pin, repo, bench, tests, build)
            tests.write_text(OK_LOG)
            binary.unlink()
            with self.assertRaises(ValueError):
                FB.receipt(pin, repo, bench, tests, build)


class B1pScripts(unittest.TestCase):
    def test_build_fix_runs_tests_before_the_build_and_writes_the_receipt(self):
        source = text("build_fix.sh")
        r = subprocess.run(["bash", "-n", str(K / "build_fix.sh")], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)
        tests = source.index("cargo test -j8 --release -p kf-chan -p kf-qemu -p kf-rm")
        gate = source.index("fix_build.py tests")
        build = source.index("scripts/bench/build_kf3.sh")
        self.assertLess(source.index("fix_build.py source"), tests)
        self.assertLess(tests, gate)
        self.assertLess(gate, build)
        self.assertIn("fix_build.py receipt", source[build:])
        self.assertIn("/root/prov/observed_fix_binary.json", source)
        self.assertIn("/root/sweep2/fix_pin.json", source)
        self.assertNotIn(BASE, source)

    def test_candidates_build_the_fix_and_not_the_b1_binary(self):
        source = text("build_candidates.sh")
        self.assertIn("bash /root/sweep2/build_fix.sh", source)
        self.assertNotIn("build_b1.sh", source)
        self.assertFalse((K / "build_b1.sh").exists())
        self.assertFalse((K / "b1_commit.txt").exists())

    def test_controller_runs_every_arm_on_the_pinned_fix_binary(self):
        source = text("controller.sh")
        self.assertIn("observed_fix_binary.json", source)
        self.assertNotIn("observed_b1_binary.json", source)
        self.assertNotIn(BASE, source)
        self.assertIn('[ "${#plan[@]}" = 3 ]', source)
        self.assertIn('windows) run_cell "$run" "${fix[0]}" "${fix[1]}" ;;', source)
        self.assertIn("CELL_UNRUN_FIX_BUILD", source)

    def fix_meta(self, tmp, *, sha=None, source=FIX, line=True):
        code = heredoc("controller.sh", "FIX_META")
        root = Path(tmp)
        for name in ("prov", "s2", "bench/kf3-bins/" + FIX[:8]):
            (root / name).mkdir(parents=True, exist_ok=True)
        binary = root / "bench/kf3-bins" / FIX[:8] / "qemu-system-x86_64"
        binary.write_bytes(b"bin" + LOG_LINE.encode())
        (root / "s2/fix_pin.json").write_text(text("fix_pin.json"))
        row = {
            "source": source,
            "binary": str(binary),
            "binary_sha256": sha or hashlib.sha256(binary.read_bytes()).hexdigest(),
            "tests_passed": True,
            "route_log_line_in_binary": line,
        }
        (root / "prov/observed_fix_binary.json").write_text(json.dumps(row))
        code = code.replace("/root/prov", str(root / "prov")).replace("/root/sweep2", str(root / "s2"))
        code = code.replace("/workspace/bench", str(root / "bench"))
        (root / "meta.py").write_text(code)
        return subprocess.run([sys.executable, str(root / "meta.py")], capture_output=True, text=True)

    def test_fix_meta_heredoc_prints_the_pinned_binary_and_source(self):
        with tempfile.TemporaryDirectory() as tmp:
            r = self.fix_meta(tmp)
            self.assertEqual(r.returncode, 0, r.stderr[-400:])
            lines = r.stdout.splitlines()
            self.assertEqual(len(lines), 2)
            self.assertTrue(lines[0].endswith("/kf3-bins/b85d02ba/qemu-system-x86_64"))
            self.assertEqual(lines[1], FIX)

    def test_fix_meta_heredoc_refuses_a_tampered_or_foreign_binary(self):
        for kwargs in ({"sha": "0" * 64}, {"source": BASE}, {"line": False}):
            with tempfile.TemporaryDirectory() as tmp, self.subTest(kwargs=kwargs):
                r = self.fix_meta(tmp, **kwargs)
                self.assertNotEqual(r.returncode, 0)
                self.assertEqual(r.stdout, "")

    def test_box_final_audit_pins_the_fix_worktree(self):
        source = text("final_remote_audit.py")
        self.assertIn("/workspace/bw-fix", source)
        self.assertIn(FIX, source)
        self.assertNotIn(BASE, source)


__all__ = [
    "Present",
    "CapturePack",
    "CapturePull",
    "FatalCollect",
    "ScriptShapes",
    "B1pArmPlan",
    "B1pAuthority",
    "FixPin",
    "FixBuild",
    "B1pScripts",
]

if __name__ == "__main__":
    unittest.main()
