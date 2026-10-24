"""BW5 round B1p run kit: the combined fix revision 007ba1eb0, pinned and checked (tests first).

Controller 11:50: ONE combined revision, the merge of the relay-route fix (b85d02bae) and the
second-life fix (38fd948fd), both on cb80cbd29. The B1p plan reuses the B1n kit (capture,
collection boots, fatal collection, guard) unchanged and changes only (a) the binary every arm
runs: the pinned merge, built on the box with its GPU-free tests, checked to have exactly the two
pinned parents and both fixes' markers, and to carry the route log line, and (b) the arm plan:
on_fix (default flags, 10-minute hold), on_fix_pnp (the existing single DIAGNOSTIC
disable/enable cycle), lx_reload (the existing Linux reload arm, regression). The
plan-independent B1n suites (Present, CapturePack, CapturePull, FatalCollect, ScriptShapes) run
on this kit too.
"""

import copy
import datetime as dt
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
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

FIX = "007ba1eb03db9e21d351e465e95b46cbc3134a95"
PARENTS = ["b85d02bae3ed085a10cd1c8376c1a842e86c571b", "38fd948fd67d89ebf42827f56a3435355df592ae"]
RED = PARENTS[0]
MARKER = "fn the_next_lifes_own_mailbox_write_ends_the_halted_lifes_sentinel"
BASE = "cb80cbd29b7a05f904acd822f172485fd49938a9"
LOG_LINE = "USER_RING_ROUTE_DECLINED family="
PLAN = ["on_fix", "on_fix_pnp", "lx_reload"]
WINDOWS = ["on_fix", "on_fix_pnp"]
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
        for run in ("on_fix",):
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

    def test_only_lx_reload_is_linux_and_no_arm_carries_a_flag(self):
        for arm in A.ARMS:
            want = "linux" if arm["run"] == "lx_reload" else "windows"
            self.assertEqual(arm["kind"], want, arm["run"])
            self.assertFalse(arm["env"], arm["run"])
        self.assertEqual(A.windows_arms(), WINDOWS)

    def bad(self, **change):
        arms = copy.deepcopy(list(A.ARMS))
        index = change.pop("index", 0)
        arms[index].update(change)
        with self.assertRaises(ValueError):
            A.validate(arms)

    def test_shadow_flag_is_not_admitted(self):
        # CODE READ at b85d02bae: shadow acts only on the private ring, which GB20x no longer takes.
        arms = copy.deepcopy(list(A.ARMS))
        arms[0].update(label="DIAGNOSTIC", env={"KF3_WIN_USER_RING_SHADOW": "1"})
        with self.assertRaises(ValueError):
            A.validate(arms)

    def test_flag_table_is_the_unchanged_b1n_table(self):
        self.assertEqual(set(A.ENV_VALUES), {"KF3_USER_RING_TRACE", "KF3_WIN_USER_RING_RELAY"})

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
            self.assertEqual(A.prepare("lx_reload", prov=tmp), "- 0 - DIAGNOSTIC")
            with self.assertRaises(ValueError):
                A.prepare("on_urt", prov=tmp)


class B1pAuthority(unittest.TestCase):
    def build(self, die="GB205"):
        return AUTH.build(lease_id=57500002, die=die, start=START, end=END, table=O.DIE_PAIRS)

    def test_authority_names_the_b1p_arms_and_the_cycle(self):
        a = self.build()
        self.assertEqual(a["arms"], WINDOWS)
        self.assertEqual(a["diagnostic_enable_arms"], {"on_fix_pnp": 2})
        self.assertEqual(a["exact_channels"], ["0x14", "0x1000000b"])
        self.assertEqual(self.build("GB202")["exact_channels"], ["0x14", "0x0e00000b"])

    def test_every_b1p_permit_is_accepted_by_the_unchanged_guard(self):
        a = self.build()
        with tempfile.TemporaryDirectory() as tmp:
            probed = Path(tmp) / "probed_die.json"
            probed.write_text(json.dumps({"die": "GB205"}))
            for run in WINDOWS:
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

    def test_pin_names_the_merge_its_two_parents_and_its_base(self):
        pin = self.pin()
        self.assertEqual(pin["source"], FIX)
        self.assertEqual(pin["parents"], PARENTS)
        self.assertEqual(pin["source_markers"], [["crates/kf-gsp/src/boot.rs", MARKER]])
        self.assertEqual(pin["base"], BASE)
        self.assertEqual(pin["bin_dir"], FIX[:8])
        self.assertEqual(pin["log_line"], LOG_LINE)
        self.assertEqual(pin["log_source"], "crates/kf-qemu/src/userrelay.rs")
        self.assertEqual(FB.load_pin(K / "fix_pin.json"), pin)

    def test_pin_matches_the_repository_objects(self):
        git = ["git", "-C", str(REPO)]
        revs = [(FIX + "^1", PARENTS[0]), (FIX + "^2", PARENTS[1])]
        revs += [(FIX + "^1~2", BASE), (FIX + "^2~2", BASE)]
        for rev, want in revs:
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
        boot = subprocess.check_output(
            [*git, "show", FIX + ":crates/kf-gsp/src/boot.rs"], text=True
        )
        self.assertIn(MARKER, boot)
        base = subprocess.check_output(
            [*git, "show", BASE + ":crates/kf-gsp/src/boot.rs"], text=True
        )
        self.assertNotIn(MARKER, base)

    def test_load_pin_refuses_a_bin_dir_or_hash_mismatch(self):
        with tempfile.TemporaryDirectory() as tmp:
            for change in (
                {"bin_dir": "cb80cbd2"},
                {"source": "007ba1eb"},
                {"log_line": ""},
                {"parents": PARENTS[:1]},
                {"source_markers": []},
            ):
                path = Path(tmp) / "pin.json"
                path.write_text(json.dumps({**self.pin(), **change}))
                with self.subTest(change=change), self.assertRaises(ValueError):
                    FB.load_pin(path)


def chain(tmp):
    """A real merge of two fix branches (base -> red -> fix each), like the pinned revision."""
    repo = Path(tmp) / "repo"
    for d in ("crates/kf-qemu/src", "crates/kf-gsp/src"):
        (repo / d).mkdir(parents=True)
    git = ["git", "-C", str(repo), "-c", "user.name=t", "-c", "user.email=t@t"]
    subprocess.run(["git", "init", "-q", "-b", "main", str(repo)], check=True)
    relay, boot = repo / "crates/kf-qemu/src/userrelay.rs", repo / "crates/kf-gsp/src/boot.rs"

    def commit(path, body):
        path.write_text(body)
        subprocess.run([*git, "add", "-A"], check=True)
        subprocess.run([*git, "commit", "-qm", body[:8]], check=True)
        return subprocess.check_output([*git, "rev-parse", "HEAD"], text=True).strip()

    boot.write_text("base\n")
    base = commit(relay, "base\n")
    commit(relay, "red\n")
    fix1 = commit(relay, 'eprintln!("kf3: ' + LOG_LINE + '{x}");\n')
    subprocess.run([*git, "checkout", "-q", "-b", "life", base], check=True)
    commit(boot, "red\n")
    fix2 = commit(boot, MARKER + "() {}\n")
    subprocess.run([*git, "checkout", "-q", "main"], check=True)
    subprocess.run([*git, "merge", "-q", "--no-ff", "--no-edit", "life"], check=True)
    pin = {
        "source": subprocess.check_output([*git, "rev-parse", "HEAD"], text=True).strip(),
        "parents": [fix1, fix2],
        "base": base,
        "log_line": LOG_LINE,
        "log_source": "crates/kf-qemu/src/userrelay.rs",
        "source_markers": [["crates/kf-gsp/src/boot.rs", MARKER]],
    }
    pin["bin_dir"] = pin["source"][:8]
    return repo, pin, git


OK_LOG = (
    "test result: ok. 410 passed; 0 failed; 2 ignored; 0 measured\n"
    "test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured\n"
)


def cli(*args):
    return subprocess.run(
        [sys.executable, str(K / "fix_build.py"), *map(str, args)], capture_output=True, text=True
    )


class FixBuild(unittest.TestCase):
    def test_source_check_accepts_the_exact_clean_chain(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo, pin, _ = chain(tmp)
            self.assertEqual(FB.check_source(repo, pin), pin["source"])

    def test_source_check_refuses_another_head_a_wrong_chain_or_a_dirty_tree(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo, pin, git = chain(tmp)
            for change in (
                {"source": pin["parents"][0]},
                {"parents": pin["parents"][::-1]},
                {"parents": [pin["parents"][0], pin["base"]]},
                {"base": pin["parents"][0]},
                {"source_markers": [["crates/kf-gsp/src/boot.rs", "NOT_A_MARKER"]]},
            ):
                with self.subTest(change=change), self.assertRaises(ValueError):
                    FB.check_source(repo, {**pin, **change})
            with self.assertRaises(ValueError):
                FB.check_source(repo, {**pin, "log_line": "NOT_IN_SOURCE"})
            (repo / "crates/kf-qemu/src/userrelay.rs").write_text("edited\n")
            with self.assertRaises(ValueError):
                FB.check_source(repo, pin)

    def test_receipt_refuses_each_foreign_revision_and_a_dirty_tree(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo, pin, bench, tests, build, _ = self.receipt_env(tmp, LOG_LINE.encode())
            for change in (
                {"source": pin["parents"][0]},
                {"parents": pin["parents"][::-1]},
                {"parents": [pin["parents"][0], pin["base"]]},
                {"base": pin["parents"][0]},
                {"source_markers": [["crates/kf-gsp/src/boot.rs", "NOT_A_MARKER"]]},
            ):
                with self.subTest(change=change), self.assertRaises(ValueError):
                    FB.receipt({**pin, **change}, repo, bench, tests, build)
            (repo / "crates/kf-qemu/src/userrelay.rs").write_text("edited\n")
            with self.assertRaises(ValueError):
                FB.receipt(pin, repo, bench, tests, build)

    def test_cli_receipt_writes_nothing_for_a_binary_without_the_line_or_missing(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo, pin, bench, tests, build, binary = self.receipt_env(tmp, b"old binary")
            pin_path = Path(tmp) / "pin.json"
            pin_path.write_text(json.dumps(pin))
            out = Path(tmp) / "observed_fix_binary.json"
            r = cli("receipt", pin_path, repo, bench, tests, build, out)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("does not carry the route log line", r.stderr)
            binary.unlink()
            r = cli("receipt", pin_path, repo, bench, tests, build, out)
            self.assertIn("pinned kf3 binary missing", r.stderr)
            self.assertFalse(out.exists())
            binary.write_bytes(b"new " + LOG_LINE.encode())
            r = cli("receipt", pin_path, repo, bench, tests, build, out)
            self.assertEqual(r.returncode, 0, r.stderr[-300:])
            self.assertEqual(json.loads(out.read_text())["source"], pin["source"])

    def test_cli_source_refuses_a_pin_with_a_foreign_bin_dir(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo, pin, _ = chain(tmp)
            pin_path = Path(tmp) / "pin.json"
            pin_path.write_text(json.dumps({**pin, "bin_dir": "cb80cbd2"}))
            r = cli("source", pin_path, repo)
            self.assertIn("bin_dir", r.stderr)
            pin_path.write_text(json.dumps({**pin, "parents": pin["parents"][:1]}))
            r = cli("source", pin_path, repo)
            self.assertIn("exactly two parents", r.stderr)
            pin_path.write_text(json.dumps(pin))
            self.assertEqual(cli("source", pin_path, repo).returncode, 0)

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
        code = code.replace("/root/prov", str(root / "prov")).replace(
            "/root/sweep2", str(root / "s2")
        )
        code = code.replace("/workspace/bench", str(root / "bench"))
        (root / "meta.py").write_text(code)
        return subprocess.run(
            [sys.executable, str(root / "meta.py")], capture_output=True, text=True
        )

    def test_fix_meta_heredoc_prints_the_pinned_binary_and_source(self):
        with tempfile.TemporaryDirectory() as tmp:
            r = self.fix_meta(tmp)
            self.assertEqual(r.returncode, 0, r.stderr[-400:])
            lines = r.stdout.splitlines()
            self.assertEqual(len(lines), 2)
            self.assertTrue(lines[0].endswith("/kf3-bins/007ba1eb/qemu-system-x86_64"))
            self.assertEqual(lines[1], FIX)

    def test_fix_meta_heredoc_refuses_other_tampering(self):
        for kwargs in ({"sha": "f" * 64}, {"source": PARENTS[0]}, {"line": None}):
            with tempfile.TemporaryDirectory() as tmp, self.subTest(kwargs=kwargs):
                r = self.fix_meta(tmp, **kwargs)
                self.assertNotEqual(r.returncode, 0)
                self.assertEqual(r.stdout, "")

    def test_controller_plan_count_is_the_plan_length(self):
        found = re.findall(r'\[ "\$\{#plan\[@\]\}" = (\d+) \]', text("controller.sh"))
        self.assertEqual([int(n) for n in found], [len(A.ARMS)])

    def test_candidates_call_only_scripts_of_this_kit(self):
        for name in re.findall(
            r"bash /root/sweep2/([A-Za-z0-9_.-]+\.sh)", text("build_candidates.sh")
        ):
            self.assertTrue((K / name).is_file(), name)

    def test_final_audit_reads_the_worktree_build_fix_creates(self):
        work = re.search(r"^work=(\S+)$", text("build_fix.sh"), re.M)[1]
        audit = re.search(r"\('observed_fix','([^']+)'\)", text("final_remote_audit.py"))[1]
        self.assertEqual(audit, work)

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
