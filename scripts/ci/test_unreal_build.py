#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of scripts/bench/unreal: cache key, manifest, disk and RAM budgets, the build steps with a fake runner,
resume, the build-or-fetch decision, pins (strict only), upload plan, rows. No source tree, no network, no process of the real
build is started: the runner is a fake and the source tree is a few empty files in a temporary directory.

STATUS: DESIGN-ONLY tooling, 2026-11-17. Run: python3 -B -I scripts/ci/test_unreal_build.py
"""
import copy
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
UE = ROOT / "scripts/bench/unreal"
sys.path.insert(0, str(UE))
sys.path.insert(0, str(ROOT / "scripts/ci"))
sys.path.insert(0, str(ROOT / "scripts/bench/windows"))
import build_installed as B  # noqa: E402
import resolve_unreal as RS  # noqa: E402
import ue_lib as L  # noqa: E402

COMMIT = "a" * 40
COMMIT2 = "b" * 40
GB = 10**9


def make_tree(root, version=(5, 1, 1), sdk="v99_clang-1.0.0-test", gitdeps=True):
    root = Path(root)
    for rel in ("Setup.sh", "GenerateProjectFiles.sh", "Engine/Build/InstalledEngineBuild.xml"):
        (root / rel).parent.mkdir(parents=True, exist_ok=True)
        (root / rel).write_text("#fake\n")
    (root / "Engine/Build/Build.version").write_text(json.dumps({"MajorVersion": version[0], "MinorVersion": version[1], "PatchVersion": version[2], "Changelist": 0, "CompatibleChangelist": 7, "IsPromotedBuild": 0, "BranchName": "T"}))
    (root / "Engine/Config/Linux").mkdir(parents=True, exist_ok=True)
    (root / "Engine/Config/Linux/Linux_SDK.json").write_text(json.dumps({"MainVersion": sdk}))
    if gitdeps:
        (root / "Engine/Build/Commit.gitdeps.xml").write_text("<deps/>")
    return root


class FakeRunner:
    """Scripted: git answers from `git`, every build step is recorded and its exit code comes from `rcs[phase-first-argv-token]`."""

    fake = True

    def __init__(self, commit=COMMIT, dirty=False, rcs=None, tree_files=None, diff=b"diff-bytes"):
        self.commit, self.dirty, self.rcs, self.diff = commit, dirty, rcs or {}, diff
        self.calls, self.captures = [], []
        self.tree_files = tree_files
        self.envs = []

    def capture(self, argv, cwd=None, env=None, timeout=300):
        self.captures.append(argv)
        if argv[0] == "git" and "rev-parse" in argv:
            return (0, (self.commit + "\n").encode()) if self.commit else (128, b"")
        if argv[0] == "git" and "status" in argv:
            return 0, (b" M Engine/x\0" if self.dirty else b"")
        if argv[0] == "git" and "diff" in argv:
            return 0, self.diff
        return 0, b""

    def run(self, argv, cwd, env, log_path, timeout):
        key = os.path.basename(argv[0])
        self.calls.append({"argv": argv, "cwd": cwd, "env": env, "log": log_path})
        self.envs.append(env)
        rc = self.rcs.get(key, 0)
        if rc == 0 and key == "RunUAT.sh" and self.tree_files is not None:
            installed = [a for a in argv if a.startswith("-set:BuiltDirectory=")][0].split("=", 1)[1]
            for rel in self.tree_files:
                p = Path(installed) / "Linux" / rel
                p.parent.mkdir(parents=True, exist_ok=True)
                p.write_text("x")
                p.chmod(0o755)
        return rc

    def pipeline(self, argv1, argv2, log_path, timeout, env):
        self.calls.append({"argv": argv1, "pipe": argv2, "env": env})
        part = argv2[argv2.index("-o") + 1]
        Path(part).write_bytes(b"zstd-bytes" * 10)
        return self.rcs.get("tar", 0)


REQUIRED = list(L.RECIPE["required_in_tree"]) + list(L.RECIPE["expected_in_tree"])


def builder(tmp, runner=None, free=10**13, ram=32.0, cpus=24, dev_of=None, say=None, environ=None):
    src = make_tree(Path(tmp) / "src")
    r = runner or FakeRunner(tree_files=REQUIRED)
    out = []
    b = B.Builder(r, str(src), str(Path(tmp) / "work"), environ=environ if environ is not None else {"PATH": "/usr/bin", "HOME": "/h", "GH_TOKEN": "secret-token", "AWS_SECRET_ACCESS_KEY": "s"},
                  free_fn=(free if callable(free) else (lambda p: free)), sysinfo_fn=lambda: (cpus, ram), dev_of=dev_of, say=say or out.append)
    return b, r, out


class T(unittest.TestCase):
    def setUp(self):
        self.td = tempfile.TemporaryDirectory(prefix="ue-test-")
        self.tmp = self.td.name
        self.addCleanup(self.td.cleanup)

    # ------------------------------------------------------------------------------------------------ the cache key
    def comp(self, **kw):
        self.n = getattr(self, "n", 0) + 1
        src = make_tree(Path(self.tmp) / ("s%d" % self.n), **{k: v for k, v in kw.items() if k in ("version", "sdk", "gitdeps")})
        return L.compute_key(FakeRunner(commit=kw.get("commit", COMMIT), dirty=kw.get("dirty", False)), str(src), allow_dirty=True)

    def test_key_is_deterministic(self):
        self.assertEqual(self.comp()["key"], self.comp()["key"])
        self.assertRegex(self.comp()["key"], r"^[0-9a-f]{64}$")

    def test_key_changes_with_the_source_commit(self):
        self.assertNotEqual(self.comp()["key"], self.comp(commit=COMMIT2)["key"])

    def test_key_changes_with_the_engine_version(self):
        self.assertNotEqual(self.comp()["key"], self.comp(version=(5, 1, 2))["key"])

    def test_key_changes_with_the_toolchain_version(self):
        self.assertNotEqual(self.comp()["key"], self.comp(sdk="v100_clang-2.0.0-test")["key"])

    def test_key_changes_with_the_dependency_manifest(self):
        self.assertNotEqual(self.comp()["key"], self.comp(gitdeps=False)["key"])

    def test_key_changes_with_the_recipe_flags(self):
        ident = {"kind": "git", "id": COMMIT, "dirty_digest": None}
        v = {"major": 5, "minor": 1, "patch": 1, "branch": "T", "compatible_changelist": 7, "is_promoted": 0, "version": "5.1.1"}
        base = L.cache_key(L.key_inputs(ident, v, {"sdk_version": "x"}, None))
        r2 = copy.deepcopy(L.RECIPE)
        r2["set"]["GameConfigurations"] = "Development;Shipping"
        self.assertNotEqual(base, L.cache_key(L.key_inputs(ident, v, {"sdk_version": "x"}, None, r2)))
        r3 = copy.deepcopy(L.RECIPE)
        r3["target"]["arch"] = "arm64"
        self.assertNotEqual(base, L.cache_key(L.key_inputs(ident, v, {"sdk_version": "x"}, None, r3)))

    def test_key_changes_with_a_recipe_field_that_is_not_a_flag(self):
        ident = {"kind": "git", "id": COMMIT, "dirty_digest": None}
        v = {"major": 5, "minor": 1, "patch": 1, "branch": "T", "compatible_changelist": 7, "is_promoted": 0, "version": "5.1.1"}
        r2 = copy.deepcopy(L.RECIPE)
        r2["archive"]["level"] = 9
        self.assertNotEqual(L.cache_key(L.key_inputs(ident, v, {"sdk_version": "x"}, None)), L.cache_key(L.key_inputs(ident, v, {"sdk_version": "x"}, None, r2)))

    def test_bumping_the_recipe_version_changes_the_key(self):
        ident = {"kind": "git", "id": COMMIT, "dirty_digest": None}
        v = {"major": 5, "minor": 1, "patch": 1, "branch": "T", "compatible_changelist": 7, "is_promoted": 0, "version": "5.1.1"}
        before = L.cache_key(L.key_inputs(ident, v, {"sdk_version": "x"}, None))
        old = L.RECIPE_VERSION
        L.RECIPE_VERSION = old + 1
        try:
            after = L.cache_key(L.key_inputs(ident, v, {"sdk_version": "x"}, None))
        finally:
            L.RECIPE_VERSION = old
        self.assertNotEqual(before, after)

    def test_recipe_rev_follows_the_recipe_contents(self):
        r2 = copy.deepcopy(L.RECIPE)
        r2["set"]["WithDDC"] = "false"
        self.assertNotEqual(L.recipe_rev(), L.recipe_rev(r2))
        self.assertRegex(L.recipe_rev(), r"^r\d+-[0-9a-f]{12}$")

    def test_key_inputs_name_the_commit_the_engine_version_and_the_recipe(self):
        i = self.comp()["inputs"]
        self.assertEqual(i["source"]["id"], COMMIT)
        self.assertEqual((i["engine"]["major"], i["engine"]["minor"], i["engine"]["patch"]), (5, 1, 1))
        self.assertEqual(i["recipe_rev"], L.recipe_rev())
        self.assertEqual(i["toolchain"], {"sdk_version": "v99_clang-1.0.0-test"})
        self.assertIsNotNone(i["deps_manifest_sha256"])
        self.assertEqual(i["target"], {"platform": "Linux", "arch": "x86_64"})
        self.assertEqual(i["flags"]["set"], L.RECIPE["set"])

    def test_the_dirty_digest_depends_on_the_difference(self):
        src = make_tree(Path(self.tmp) / "dd")
        a = L.compute_key(FakeRunner(dirty=True, diff=b"one"), str(src), allow_dirty=True)
        b = L.compute_key(FakeRunner(dirty=True, diff=b"two"), str(src), allow_dirty=True)
        self.assertNotEqual(a["key"], b["key"])
        self.assertRegex(a["inputs"]["source"]["dirty_digest"], r"^[0-9a-f]{64}$")

    def test_key_names_the_target_platform(self):
        self.assertEqual(self.comp()["inputs"]["target"], {"platform": "Linux", "arch": "x86_64"})

    def test_key_inputs_hold_the_flags_but_never_the_machine_or_the_time(self):
        s = L.canonical(self.comp()["inputs"])
        for word in ("cpus", "ram", "built_utc", "hostname", "clang_observed"):
            self.assertNotIn(word, s)
        self.assertIn("HostPlatformOnly", s)

    def test_dirty_tree_is_refused_unless_allowed(self):
        src = make_tree(Path(self.tmp) / "d")
        with self.assertRaises(L.UnrealError) as c:
            L.compute_key(FakeRunner(dirty=True), str(src))
        self.assertEqual(c.exception.cls, L.FAIL_SOURCE_DIRTY)
        a = L.compute_key(FakeRunner(dirty=True), str(src), allow_dirty=True)
        clean = L.compute_key(FakeRunner(), str(src))
        self.assertIsNotNone(a["inputs"]["source"]["dirty_digest"])
        self.assertNotEqual(a["key"], clean["key"])

    def test_a_tree_without_history_needs_a_declared_id(self):
        src = make_tree(Path(self.tmp) / "n")
        with self.assertRaises(L.UnrealError) as c:
            L.compute_key(FakeRunner(commit=""), str(src))
        self.assertEqual(c.exception.cls, L.FAIL_SOURCE_ID)
        ok = L.compute_key(FakeRunner(commit=""), str(src), source_id="unpacked-20261117")
        self.assertEqual(ok["inputs"]["source"]["kind"], "declared")

    def test_a_declared_id_must_be_plain(self):
        src = make_tree(Path(self.tmp) / "n2")
        for bad in ("short", "has space in it", "a/b/../../c" * 2, "x" * 200):
            with self.assertRaises(L.UnrealError):
                L.compute_key(FakeRunner(commit=""), str(src), source_id=bad)

    def test_layout_files_are_required(self):
        for rel in ("Setup.sh", "GenerateProjectFiles.sh", "Engine/Build/InstalledEngineBuild.xml", "Engine/Build/Build.version", "Engine/Config/Linux/Linux_SDK.json"):
            d = Path(self.tmp) / ("l_" + rel.replace("/", "_"))
            src = make_tree(d)
            (src / rel).unlink()
            with self.assertRaises(L.UnrealError) as c:
                L.compute_key(FakeRunner(), str(src))
            self.assertEqual(c.exception.cls, L.FAIL_SOURCE_LAYOUT, rel)

    def test_a_tree_without_setup_sh_is_not_a_source_root(self):
        src = make_tree(Path(self.tmp) / "nosetup")
        (src / "Setup.sh").unlink()
        with self.assertRaises(L.UnrealError) as c:
            L.check_layout(str(src))
        self.assertIn("Setup.sh", c.exception.detail)

    def test_version_and_toolchain_files_must_parse(self):
        src = make_tree(Path(self.tmp) / "p")
        (src / "Engine/Build/Build.version").write_text("{}")
        with self.assertRaises(L.UnrealError):
            L.read_build_version(str(src))
        (src / "Engine/Config/Linux/Linux_SDK.json").write_text(json.dumps({"MainVersion": "bad value!"}))
        with self.assertRaises(L.UnrealError):
            L.read_toolchain(str(src))

    def test_object_name_is_derived_from_the_key(self):
        k = "ab" * 32
        self.assertEqual(L.object_name("5.1.1", k), "linux-proprietary/unreal-5.1.1-abababababab/artifact.tar.zst")
        self.assertNotEqual(L.object_name("5.1.1", k), L.object_name("5.1.1", "cd" * 32))
        self.assertEqual(L.object_name("5.1.1", k, "mine/unreal"), "mine/unreal/unreal-5.1.1-abababababab/artifact.tar.zst")

    def test_object_name_refuses_a_bad_prefix_version_or_key(self):
        for args in (("5.1.1", "ab" * 32, "../x"), ("5.1.1", "ab" * 32, "a b"), ("5.1", "ab" * 32), ("5.1.1", "zz")):
            with self.assertRaises(L.UnrealError):
                L.object_name(*args)

    def test_build_id_is_guid_shaped_and_key_derived(self):
        a, b = L.build_id_for("ab" * 32), L.build_id_for("cd" * 32)
        self.assertRegex(a, r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
        self.assertNotEqual(a, b)

    # ------------------------------------------------------------------------------------------------ disk and memory
    LOC = {"source": "/a", "installed": "/b", "out": "/c"}

    def test_disk_refuses_below_the_budget_and_says_the_numbers(self):
        with self.assertRaises(L.UnrealError) as c:
            L.check_disk("buildgraph", self.LOC, free_fn=lambda p: 100 * GB, dev_of=lambda p: p)
        self.assertEqual(c.exception.cls, L.FAIL_DISK)
        self.assertIn("buildgraph", c.exception.detail)
        self.assertIn("100 GB is free", c.exception.detail)

    def test_disk_boundary_is_exact(self):
        need = (sum(L.PHASE_BUDGET_GB["package"].values()) + L.DEFAULT_RESERVE_GB) * GB
        L.check_disk("package", self.LOC, free_fn=lambda p: need, dev_of=lambda p: p)
        with self.assertRaises(L.UnrealError):
            L.check_disk("package", self.LOC, free_fn=lambda p: need - 1, dev_of=lambda p: p)

    def test_two_locations_on_one_filesystem_add_up(self):
        per = (L.PHASE_BUDGET_GB["buildgraph"]["source"] + L.DEFAULT_RESERVE_GB) * GB
        both = (L.PHASE_BUDGET_GB["buildgraph"]["source"] + L.PHASE_BUDGET_GB["buildgraph"]["installed"] + L.DEFAULT_RESERVE_GB) * GB
        L.check_disk("buildgraph", self.LOC, free_fn=lambda p: per + 45 * GB, dev_of=lambda p: 1)
        with self.assertRaises(L.UnrealError):
            L.check_disk("buildgraph", self.LOC, free_fn=lambda p: both - 1, dev_of=lambda p: 1)
        # on two filesystems each is checked on its own
        L.check_disk("buildgraph", self.LOC, free_fn=lambda p: per, dev_of=lambda p: p)

    def test_the_reserve_is_part_of_the_budget(self):
        need = sum(L.PHASE_BUDGET_GB["setup"].values()) * GB
        with self.assertRaises(L.UnrealError):
            L.check_disk("setup", self.LOC, free_fn=lambda p: need, dev_of=lambda p: p)

    def test_there_is_no_way_to_turn_the_disk_check_off(self):
        text = (UE / "build_installed.py").read_text().lower()
        for word in ("--no-disk", "--disk-override", "--skip-disk", "--force-disk"):
            self.assertNotIn(word, text)

    def test_ram_below_the_minimum_is_refused(self):
        L.check_ram(L.MIN_RAM_GB)
        with self.assertRaises(L.UnrealError) as c:
            L.check_ram(L.MIN_RAM_GB - 0.1)
        self.assertEqual(c.exception.cls, L.FAIL_RAM)

    def test_jobs_are_bounded_by_cpus_and_memory_and_a_request_only_lowers(self):
        self.assertEqual(L.compute_jobs(24, 29.0), 16)
        self.assertEqual(L.compute_jobs(8, 64.0), 8)
        self.assertEqual(L.compute_jobs(24, 29.0, requested=4), 4)
        self.assertEqual(L.compute_jobs(24, 29.0, requested=64), 16)
        self.assertEqual(L.compute_jobs(1, 1.0), 1)

    def test_the_plan_lists_every_phase_and_marks_the_numbers_as_estimates(self):
        t = L.plan_text(L.phases_for(True, True))
        for p in ("clone", "setup", "genproj", "buildgraph", "package", "total"):
            self.assertIn(p, t)
        self.assertIn("ESTIMATE", t)
        self.assertEqual(L.phases_for(False, False), ["setup", "genproj", "buildgraph"])

    # ------------------------------------------------------------------------------------------------ the build steps
    def run_build(self, b, **kw):
        kw.setdefault("allow_modify_source", True)
        return b.build(**kw)

    def test_the_build_needs_the_flag_that_allows_modifying_the_source(self):
        b, r, _ = builder(self.tmp)
        with self.assertRaises(L.UnrealError) as c:
            b.build(allow_modify_source=False)
        self.assertEqual(c.exception.cls, L.FAIL_MODIFY)
        self.assertEqual(r.calls, [])

    def test_steps_run_in_order_with_the_recipe_arguments(self):
        b, r, _ = builder(self.tmp)
        res = self.run_build(b)
        argvs = [c["argv"] for c in r.calls]
        self.assertEqual([os.path.basename(a[0]) for a in argvs], ["Setup.sh", "GenerateProjectFiles.sh", "RunUAT.sh"])
        self.assertEqual(argvs[0][1:], ["--force"])
        self.assertTrue(all(c["cwd"] == b.source for c in r.calls))
        g = argvs[2]
        self.assertEqual(g[1], "BuildGraph")
        self.assertIn("-target=Make Installed Build Linux", g)
        self.assertIn("-script=Engine/Build/InstalledEngineBuild.xml", g)
        for k, v in L.RECIPE["set"].items():
            self.assertIn("-set:%s=%s" % (k, v), g)
        self.assertNotIn("-clean", g)
        self.assertTrue(any(a == "-set:ExtraCompileArgs=-MaxParallelActions=18" for a in g))
        self.assertTrue(any(a.startswith("-set:BuildIdOverride=") for a in g))
        self.assertTrue(res["tree"].endswith("installed/Linux"))

    def test_a_jobs_request_lowers_the_parallel_actions_in_the_build_command(self):
        b, r, _ = builder(self.tmp)
        self.run_build(b, jobs=2)
        self.assertIn("-set:ExtraCompileArgs=-MaxParallelActions=2", r.calls[2]["argv"])

    def test_the_build_command_never_cleans_and_switches_off_arm64(self):
        argv = L.buildgraph_argv("/s", "/i", "ab" * 32, 8)
        self.assertNotIn("-clean", argv)
        self.assertIn("-set:WithLinuxArm64=false", argv)
        self.assertIn("-set:BuiltDirectory=/i", argv)
        self.assertEqual(argv, L.buildgraph_argv("/s", "/i", "ab" * 32, 8))

    def test_buildgraph_sums_the_source_and_the_output_on_one_filesystem(self):
        b, r, _ = builder(self.tmp, free=130 * GB, dev_of=lambda p: 1)
        with self.assertRaises(L.UnrealError) as c:
            self.run_build(b)
        self.assertEqual(c.exception.cls, L.FAIL_DISK)
        self.assertNotIn("RunUAT.sh", [os.path.basename(x["argv"][0]) for x in r.calls])

    def test_the_linux_arm64_target_is_switched_off_explicitly(self):
        self.assertEqual(L.RECIPE["set"]["WithLinuxArm64"], "false")
        self.assertEqual(L.RECIPE["set"]["HostPlatformOnly"], "true")

    def test_nothing_runs_when_the_disk_is_short(self):
        b, r, _ = builder(self.tmp, free=1 * GB)
        with self.assertRaises(L.UnrealError) as c:
            self.run_build(b)
        self.assertEqual(c.exception.cls, L.FAIL_DISK)
        self.assertEqual(r.calls, [])

    def test_the_disk_is_checked_again_before_each_phase(self):
        seen = []

        def free(p):
            seen.append(1)
            return 10**13 if len(seen) < 3 else 1 * GB

        b, r, _ = builder(self.tmp, free=free)
        with self.assertRaises(L.UnrealError) as c:
            self.run_build(b)
        self.assertEqual(c.exception.cls, L.FAIL_DISK)
        self.assertEqual([os.path.basename(x["argv"][0]) for x in r.calls], ["Setup.sh", "GenerateProjectFiles.sh"])

    def test_nothing_runs_when_the_memory_is_short(self):
        b, r, _ = builder(self.tmp, ram=16.0)
        with self.assertRaises(L.UnrealError) as c:
            self.run_build(b)
        self.assertEqual(c.exception.cls, L.FAIL_RAM)
        self.assertEqual(r.calls, [])

    def test_a_failed_phase_stops_the_build_and_is_recorded(self):
        b, r, _ = builder(self.tmp, runner=FakeRunner(rcs={"RunUAT.sh": 1}, tree_files=REQUIRED))
        with self.assertRaises(L.UnrealError) as c:
            self.run_build(b)
        self.assertEqual(c.exception.cls, L.FAIL_BUILD)
        self.assertIn("buildgraph.log", c.exception.detail)
        st = json.load(open(os.path.join(b.work, "state.json")))
        self.assertTrue(st["phases"]["setup"]["done"])
        self.assertFalse(st["phases"]["buildgraph"]["done"])

    def test_a_second_run_resumes_at_the_failed_phase(self):
        r = FakeRunner(rcs={"RunUAT.sh": 1}, tree_files=REQUIRED)
        b, _, _ = builder(self.tmp, runner=r)
        with self.assertRaises(L.UnrealError):
            self.run_build(b)
        r.rcs = {}
        r.calls.clear()
        self.run_build(b)
        self.assertEqual([os.path.basename(c["argv"][0]) for c in r.calls], ["RunUAT.sh"])

    def test_a_finished_build_runs_nothing_again(self):
        b, r, _ = builder(self.tmp)
        self.run_build(b)
        r.calls.clear()
        self.run_build(b)
        self.assertEqual(r.calls, [])

    def test_state_of_another_key_refuses_and_restart_clears_it(self):
        b, r, _ = builder(self.tmp)
        self.run_build(b)
        b2, r2, _ = builder(self.tmp, runner=FakeRunner(commit=COMMIT2, tree_files=REQUIRED))
        with self.assertRaises(L.UnrealError) as c:
            self.run_build(b2)
        self.assertEqual(c.exception.cls, L.FAIL_STATE)
        self.assertEqual(r2.calls, [])
        self.run_build(b2, restart=True)
        self.assertEqual(len(r2.calls), 3)

    def test_a_state_belongs_to_one_key(self):
        p = os.path.join(self.tmp, "st.json")
        s1 = L.State(p, "a" * 64)
        s1.mark("setup", 0)
        self.assertTrue(L.State(p, "a" * 64).done("setup"))
        with self.assertRaises(L.UnrealError) as c:
            L.State(p, "b" * 64)
        self.assertEqual(c.exception.cls, L.FAIL_STATE)

    def test_a_state_whose_phases_are_not_a_mapping_refuses(self):
        p = os.path.join(self.tmp, "st4.json")
        Path(p).write_text(json.dumps({"schema": L.STATE_SCHEMA, "key": "a" * 64, "phases": []}))
        with self.assertRaises(L.UnrealError):
            L.State(p, "a" * 64)

    def test_a_state_file_of_another_schema_refuses(self):
        p = os.path.join(self.tmp, "st2.json")
        Path(p).write_text(json.dumps({"schema": "other", "key": "a" * 64, "phases": {}}))
        with self.assertRaises(L.UnrealError):
            L.State(p, "a" * 64)

    def test_a_failed_phase_is_not_marked_done(self):
        s1 = L.State(os.path.join(self.tmp, "st3.json"), "a" * 64)
        s1.mark("setup", 1)
        self.assertFalse(s1.done("setup"))
        self.assertFalse(L.State(os.path.join(self.tmp, "st3.json"), "a" * 64).done("setup"))

    def test_a_corrupt_state_file_refuses(self):
        b, r, _ = builder(self.tmp)
        os.makedirs(b.work)
        Path(b.work, "state.json").write_text("{nope")
        with self.assertRaises(L.UnrealError) as c:
            self.run_build(b)
        self.assertEqual(c.exception.cls, L.FAIL_STATE)

    def test_no_credential_reaches_a_build_step(self):
        b, r, _ = builder(self.tmp)
        self.run_build(b)
        for env in r.envs:
            self.assertNotIn("GH_TOKEN", env)
            self.assertNotIn("AWS_SECRET_ACCESS_KEY", env)
            self.assertEqual(env["GIT_TERMINAL_PROMPT"], "0")
            self.assertEqual(set(env) - set(L.ENV_ALLOW) - set(L.ENV_FORCED), set())

    def test_only_the_clone_step_may_carry_a_token(self):
        b, r, _ = builder(self.tmp)
        b.source = str(Path(self.tmp) / "cloned")
        os.makedirs(b.source)
        # the clone writes the tree: the fake does it when asked to run `gh`
        def fake_run(argv, cwd, env, log, timeout, _orig=r.run):
            if argv[0] == "gh":
                make_tree(b.source)
                r.calls.append({"argv": argv, "env": env})
                r.envs.append(env)
                return 0
            return _orig(argv, cwd, env, log, timeout)

        r.run = fake_run
        self.run_build(b, clone="Owner/Repo", ref="rel")
        gh = [c for c in r.calls if c["argv"][0] == "gh"][0]
        self.assertEqual(gh["argv"][:4], ["gh", "repo", "clone", "Owner/Repo"])
        self.assertIn("--depth", gh["argv"])
        self.assertEqual(gh["env"].get("GH_TOKEN"), "secret-token")
        self.assertEqual(gh["env"]["GH_PROMPT_DISABLED"], "1")
        self.assertEqual(gh["env"]["GIT_TERMINAL_PROMPT"], "0")
        for c in r.calls:
            if c["argv"][0] != "gh":
                self.assertNotIn("GH_TOKEN", c["env"])

    def test_an_incomplete_installed_tree_is_a_failure_and_a_missing_expected_file_a_warning(self):
        b, r, out = builder(self.tmp, runner=FakeRunner(tree_files=list(L.RECIPE["required_in_tree"])))
        self.run_build(b)
        self.assertTrue(any("KF_UNREAL_WARN missing_expected" in str(x) for x in out))
        b2, r2, _ = builder(Path(self.tmp) / "two", runner=FakeRunner(tree_files=L.RECIPE["required_in_tree"][:1]))
        with self.assertRaises(L.UnrealError) as c:
            self.run_build(b2)
        self.assertEqual(c.exception.cls, L.FAIL_TREE)

    def test_package_writes_a_manifest_that_describes_the_archive(self):
        b, r, out = builder(self.tmp)
        cache = str(Path(self.tmp) / "cache")
        res = self.run_build(b, cache_out=cache)
        man = L.load_manifest(res["manifest"])
        self.assertEqual(man["key"], res["key"])
        self.assertEqual(man["artifact"]["sha256"], L.sha256_file(res["artifact"])[0])
        self.assertEqual(man["artifact"]["size"], os.path.getsize(res["artifact"]))
        self.assertFalse(os.path.exists(res["artifact"] + ".partial"))
        self.assertTrue(man["loads_software"])
        self.assertIn("UNVERIFIED", man["reproducibility"])
        self.assertTrue(any("KF_UNREAL_ARTIFACT" in str(x) for x in out))
        pipe = [c for c in r.calls if "pipe" in c][0]
        self.assertIn("--sort=name", pipe["argv"])
        self.assertIn("--numeric-owner", pipe["argv"])

    def test_the_archive_command_is_sorted_with_a_fixed_owner(self):
        tar, zstd = L.tar_zstd_argv("/t", 3)
        for a in ("--sort=name", "--owner=0", "--group=0", "--numeric-owner"):
            self.assertIn(a, tar)
        self.assertIn("-3", zstd)

    def test_a_failed_archive_leaves_no_artifact(self):
        b, r, _ = builder(self.tmp, runner=FakeRunner(rcs={"tar": 1}, tree_files=REQUIRED))
        cache = str(Path(self.tmp) / "cache")
        with self.assertRaises(L.UnrealError):
            self.run_build(b, cache_out=cache)
        self.assertFalse(os.path.exists(os.path.join(cache, "artifact.tar.zst")))
        self.assertFalse(os.path.exists(os.path.join(cache, "manifest.json")))

    def test_a_short_disk_at_package_time_leaves_no_artifact_and_no_manifest(self):
        b, r, _ = builder(self.tmp, free=lambda p: 10**13 if "cache" not in p else 1 * GB, dev_of=lambda p: p)
        cache = Path(self.tmp) / "cache"
        with self.assertRaises(L.UnrealError):
            self.run_build(b, cache_out=str(cache))
        self.assertFalse((cache / "artifact.tar.zst").exists())
        self.assertFalse((cache / "manifest.json").exists())

    def test_the_package_phase_checks_the_disk_of_the_output_directory(self):
        calls = []

        def free(p):
            calls.append(p)
            return 10**13 if "cache" not in p else 1 * GB

        b, r, _ = builder(self.tmp, free=free, dev_of=lambda p: p)
        with self.assertRaises(L.UnrealError) as c:
            self.run_build(b, cache_out=str(Path(self.tmp) / "cache"))
        self.assertEqual(c.exception.cls, L.FAIL_DISK)
        self.assertEqual(len([x for x in r.calls if "pipe" in x]), 0)

    def test_reclaim_removes_only_the_two_generated_directories(self):
        b, r, _ = builder(self.tmp)
        for rel in B.RECLAIM + ("Engine/Source/keep",):
            p = Path(b.source, rel)
            p.mkdir(parents=True)
            (p / "f").write_text("x")
        self.run_build(b, reclaim=True)
        for rel in B.RECLAIM:
            self.assertFalse(Path(b.source, rel).exists(), rel)
        self.assertTrue(Path(b.source, "Engine/Source/keep/f").exists())
        self.assertTrue(Path(b.source, "Setup.sh").exists())

    def test_the_reclaim_list_is_exactly_the_generated_directories(self):
        self.assertEqual(B.RECLAIM, ("Engine/Intermediate/Build", "LocalBuilds/InstalledDDC"))

    def test_safe_child_refuses_a_path_that_leaves_the_root(self):
        os.makedirs(os.path.join(self.tmp, "root"))
        for rel in ("../x", "a/../../x"):
            with self.assertRaises(L.UnrealError):
                B.safe_child(os.path.join(self.tmp, "root"), rel)
        self.assertTrue(B.safe_child(os.path.join(self.tmp, "root"), "a/b").endswith("root/a/b"))

    def test_reclaim_refuses_a_symlinked_directory(self):
        b, r, _ = builder(self.tmp)
        outside = Path(self.tmp) / "precious"
        outside.mkdir()
        (outside / "f").write_text("x")
        (Path(b.source) / "Engine/Intermediate").mkdir(parents=True)
        os.symlink(outside, Path(b.source) / "Engine/Intermediate/Build")
        with self.assertRaises(L.UnrealError) as c:
            self.run_build(b, reclaim=True)
        self.assertEqual(c.exception.cls, L.FAIL_PATH)
        self.assertTrue((outside / "f").exists())

    def test_ldd_output_is_reduced_to_the_missing_sonames(self):
        t = "\tlibfoo.so.1 => not found\n\tlibc.so.6 => /lib/libc.so.6 (0x1)\n\tlibfoo.so.1 => not found\n\tlibbar.so.2 => not found\n"
        self.assertEqual(L.parse_ldd_missing(t), ["libbar.so.2", "libfoo.so.1"])

    def test_the_audit_lands_in_the_manifest(self):
        r = FakeRunner(tree_files=REQUIRED)
        r.ldd_text = True
        orig = r.capture

        def cap(argv, cwd=None, env=None, timeout=300):
            if argv[0] == "ldd":
                return 0, b"\tlibxyz.so.9 => not found\n"
            return orig(argv, cwd, env, timeout)

        r.capture = cap
        b, _, _ = builder(self.tmp, runner=r)
        res = self.run_build(b, cache_out=str(Path(self.tmp) / "cache"))
        self.assertEqual(L.load_manifest(res["manifest"])["host_libs"], ["libxyz.so.9"])

    def test_the_real_runner_writes_a_start_marker_and_an_exit_line(self):
        log = os.path.join(self.tmp, "x.log")
        rc = L.Runner().run(["sh", "-c", "echo hi; exit 3"], self.tmp, {"PATH": "/usr/bin:/bin"}, log, 30)
        self.assertEqual(rc, 3)
        text = open(log).read()
        self.assertIn("KF_UNREAL_START", text)
        self.assertIn("hi", text)
        self.assertIn("KF_UNREAL_EXIT rc=3", text)

    def test_the_real_runner_times_out(self):
        log = os.path.join(self.tmp, "t.log")
        self.assertEqual(L.Runner().run(["sleep", "30"], self.tmp, {"PATH": "/usr/bin:/bin"}, log, 1), 124)

    @unittest.skipUnless(shutil.which("git"), "git not installed")
    def test_real_git_identity_and_dirty_detection(self):
        src = make_tree(Path(self.tmp) / "g")
        env = {"PATH": os.environ["PATH"], "HOME": self.tmp, "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t"}
        for a in (["init", "-q"], ["add", "-A"], ["commit", "-q", "-m", "x"]):
            subprocess.run(["git", "-C", str(src)] + a, check=True, env=env, capture_output=True)
        ident = L.source_identity(L.Runner(), str(src))
        self.assertEqual(ident["kind"], "git")
        self.assertRegex(ident["id"], r"^[0-9a-f]{40}$")
        (src / "Setup.sh").write_text("changed\n")
        with self.assertRaises(L.UnrealError) as c:
            L.source_identity(L.Runner(), str(src))
        self.assertEqual(c.exception.cls, L.FAIL_SOURCE_DIRTY)
        self.assertIsNotNone(L.source_identity(L.Runner(), str(src), allow_dirty=True)["dirty_digest"])

    # ------------------------------------------------------------------------------------------------ the manifest
    def manifest(self, **kw):
        comp = self.comp()
        art = {"name": "artifact.tar.zst", "sha256": "1" * 64, "size": 123}
        art.update(kw)
        return L.make_manifest(comp, art, {"bytes": 1000, "files": 5}, "clang-20", ["libx.so.1"], {"cpus": 4}, "rev123", built_utc="2026-11-17T10:00:00Z")

    def test_a_manifest_validates_and_records_the_key_inputs(self):
        m = self.manifest()
        L.validate_manifest(m)
        self.assertEqual(m["key"], L.cache_key(m["key_inputs"]))
        self.assertEqual(m["created_utc"], "2026-11-17T10:00:00Z")

    def test_a_manifest_with_a_wrong_key_is_refused(self):
        m = self.manifest()
        m["key"] = "f" * 64
        with self.assertRaises(L.UnrealError) as c:
            L.validate_manifest(m)
        self.assertEqual(c.exception.cls, L.FAIL_MANIFEST)

    def test_a_manifest_with_edited_key_inputs_is_refused(self):
        m = self.manifest()
        m["key_inputs"]["source"]["id"] = COMMIT2
        with self.assertRaises(L.UnrealError):
            L.validate_manifest(m)

    def test_loads_software_must_be_true(self):
        m = self.manifest()
        m["loads_software"] = False
        with self.assertRaises(L.UnrealError) as c:
            L.validate_manifest(m)
        self.assertIn("loads_software", c.exception.detail)

    def test_manifest_strictness(self):
        for edit in (lambda m: m.update(extra=1), lambda m: m["artifact"].update(sha256="xyz"), lambda m: m["artifact"].update(size=0),
                     lambda m: m["artifact"].update(size=True), lambda m: m.update(loads_software=False), lambda m: m.update(created_utc="yesterday"),
                     lambda m: m["artifact"].update(name="../x"), lambda m: m.update(schema="other"), lambda m: m.update(host_libs="x")):
            m = self.manifest()
            edit(m)
            with self.assertRaises(L.UnrealError):
                L.validate_manifest(m)

    def test_member_check_passes_the_file_its_manifest_describes(self):
        p = Path(self.tmp) / "a.bin"
        p.write_bytes(b"hello")
        sha, size = L.sha256_file(str(p))
        self.assertEqual(L.check_member(str(p), self.manifest(sha256=sha, size=size)), (sha, size))

    def test_member_check_catches_a_changed_hash_of_the_same_size(self):
        p = Path(self.tmp) / "a.bin"
        p.write_bytes(b"hello")
        sha, size = L.sha256_file(str(p))
        m = self.manifest(sha256=sha, size=size)
        p.write_bytes(b"hellO")
        with self.assertRaises(L.UnrealError) as c:
            L.check_member(str(p), m)
        self.assertEqual(c.exception.cls, L.FAIL_INTEGRITY)

    def test_member_check_catches_a_changed_size(self):
        p = Path(self.tmp) / "a.bin"
        p.write_bytes(b"hello")
        sha, size = L.sha256_file(str(p))
        with self.assertRaises(L.UnrealError):
            L.check_member(str(p), self.manifest(sha256=sha, size=size + 1))

    def test_the_pin_entry_is_read_by_the_clock_rule_as_loaded_software(self):
        import clockrule

        m = self.manifest()
        pin = L.pin_entry(m)
        got = clockrule.pin_created({"objects": [pin]}, pin["ref"])
        self.assertEqual(clockrule.fmt_utc(got), "2026-11-17T10:00:00Z")
        self.assertTrue(pin["loads_software"])
        self.assertEqual(pin["key"], m["key"])
        self.assertTrue(pin["object"].startswith("linux-proprietary/unreal-5.1.1-"))

    def test_the_pin_object_name_and_ref_carry_the_key_prefix(self):
        m = self.manifest()
        pin = L.pin_entry(m)
        self.assertIn(m["key"][:12], pin["object"])
        self.assertIn(m["key"][:12], pin["ref"])

    def test_the_pin_entry_has_the_fields_of_the_deployment_object_list(self):
        pin = L.pin_entry(self.manifest())
        cfg = json.load(open(ROOT / "ci/vast-broker/propimages.json"))
        for k in cfg["objects"][0]:
            self.assertIn(k, pin)
        self.assertEqual(pin["role"], "artifact")

    def test_the_download_manifest_is_accepted_by_the_box_downloader(self):
        import box_downloader as D

        pin = L.pin_entry(self.manifest())
        man = D.parse_manifest(json.dumps(L.download_manifest(pin)).encode())
        self.assertEqual(man.items[0].size, 123)

    def test_the_upload_plan_is_plain_s3_and_carries_no_credential(self):
        plan = L.s3_upload_plan("https://s3.example.org", "my-bucket", [("/x/a.zst", "p/a.zst"), ("/x/m.json", "p/m.json")])
        text = " ".join(" ".join(p) for p in plan).lower()
        for word in ("r2", "wrangler", "cloudflare", "secret", "access_key", "token"):
            self.assertNotIn(word, text)
        self.assertEqual(plan[0][:3], ["aws", "s3", "cp"])
        self.assertIn("--endpoint-url", plan[0])
        self.assertIn("s3://my-bucket/p/a.zst", plan[0])

    def test_the_upload_plan_refuses_a_bad_endpoint(self):
        for ep in ("ftp://x", "https://x y", ""):
            with self.assertRaises(L.UnrealError):
                L.s3_upload_plan(ep, "my-bucket", [("/a", "b")])

    def test_the_upload_plan_refuses_a_bad_bucket(self):
        for bk in ("A_B", "", "x"):
            with self.assertRaises(L.UnrealError):
                L.s3_upload_plan("https://x", bk, [("/a", "b")])

    # ------------------------------------------------------------------------------------------------ build-or-fetch
    class Facts(RS.Facts):
        def __init__(self, test, key=None, gh=(True, ""), man=None, member=("s" * 64, 10), dirs=(), files=()):
            self.test, self.key, self.gh, self.man, self.member = test, key or ("c" * 64), gh, man, member
            self.dirs, self.files, self.keyed = set(dirs), set(files), []

        def compute_key(self, source, source_id, allow_dirty):
            self.keyed.append(source)
            return {"key": self.key, "inputs": {}, "version": "5.1.1", "identity": {}}

        def load_manifest(self, path):
            return self.man

        def member_check(self, path, manifest):
            if self.member == "bad":
                raise L.UnrealError(L.FAIL_INTEGRITY, "tampered")
            return self.member

        def gh_access(self, repo):
            return self.gh

        def isdir(self, p):
            return p in self.dirs

        def isfile(self, p):
            return p in self.files or p.endswith("manifest.json") and "nomanifest" not in p and p in self.files

    def fman(self, key="c" * 64, sha="s" * 64, size=10):
        return {"key": key, "created_utc": "2026-11-17T10:00:00Z", "artifact": {"sha256": sha, "size": size, "name": "artifact.tar.zst"}}

    def decide(self, facts=None, **kw):
        return RS.decide(RS.Opts(**kw), facts or self.Facts(self))

    ART = "/x/artifact.tar.zst"
    MAN = "/x/manifest.json"

    def art_facts(self, **kw):
        kw.setdefault("man", self.fman())
        kw.setdefault("files", (self.ART, self.MAN))
        return self.Facts(self, **kw)

    def test_an_explicit_artifact_is_used_in_standalone(self):
        d = self.decide(self.art_facts(), artifact=self.ART)
        self.assertEqual((d.action, d.exit_code), ("use_artifact", 0))
        self.assertEqual(d.extra["sha256"], "s" * 64)

    def test_an_explicit_artifact_needs_its_manifest(self):
        d = self.decide(self.Facts(self, files=(self.ART,)), artifact=self.ART)
        self.assertEqual((d.action, d.cls), ("fail", L.FAIL_MANIFEST))

    def test_integrity_against_its_own_manifest_is_enforced_in_every_mode(self):
        for mode in L.MODES:
            d = self.decide(self.art_facts(member="bad"), artifact=self.ART, mode=mode)
            self.assertEqual(d.cls, L.FAIL_INTEGRITY, mode)

    def test_artifact_and_source_with_different_keys_are_refused(self):
        d = self.decide(self.art_facts(key="e" * 64), artifact=self.ART, source="/src")
        self.assertEqual((d.action, d.cls), ("fail", L.FAIL_KEY_MISMATCH))

    def test_the_key_mismatch_names_both_keys(self):
        d = self.decide(self.art_facts(key="e" * 64), artifact=self.ART, source="/src")
        self.assertIn("eeeeeeeeeeee", d.detail)
        self.assertIn("cccccccccccc", d.detail)

    def test_artifact_and_source_with_the_same_key_are_verified(self):
        d = self.decide(self.art_facts(), artifact=self.ART, source="/src")
        self.assertEqual(d.action, "use_artifact")
        self.assertTrue(d.extra["key_verified"])

    def test_standalone_enforces_no_pin_and_records_the_computed_sha256(self):
        pin = {"key": "c" * 64, "sha256": "9" * 64, "size": 999}
        d = self.decide(self.art_facts(), artifact=self.ART, cache_listing=pin)
        self.assertEqual(d.action, "use_artifact")
        self.assertFalse(d.extra["pin_enforced"])
        self.assertTrue(d.extra["record_sha256"])
        self.assertEqual(d.extra["sha256"], "s" * 64)

    def test_strict_and_worker_enforce_the_pin(self):
        for mode in ("strict", "worker"):
            bad_sha = self.decide(self.art_facts(), artifact=self.ART, mode=mode, cache_listing={"key": "c" * 64, "sha256": "9" * 64, "size": 10})
            self.assertEqual(bad_sha.cls, L.FAIL_PIN, mode)
            bad_size = self.decide(self.art_facts(), artifact=self.ART, mode=mode, cache_listing={"key": "c" * 64, "sha256": "s" * 64, "size": 11})
            self.assertEqual(bad_size.cls, L.FAIL_PIN, mode)
            ok = self.decide(self.art_facts(), artifact=self.ART, mode=mode, cache_listing={"key": "c" * 64, "sha256": "s" * 64, "size": 10})
            self.assertEqual(ok.action, "use_artifact", mode)
            nopin = self.decide(self.art_facts(), artifact=self.ART, mode=mode)
            self.assertEqual(nopin.cls, L.FAIL_PIN, mode)

    def test_a_pin_for_another_key_is_a_key_mismatch(self):
        d = self.decide(self.art_facts(), artifact=self.ART, mode="strict", cache_listing={"key": "q" * 64, "sha256": "s" * 64, "size": 10})
        self.assertEqual(d.cls, L.FAIL_KEY_MISMATCH)

    def test_expect_sha256_mismatch_is_refused_and_names_both(self):
        bad = self.decide(self.art_facts(), artifact=self.ART, expect_sha256="a" * 64)
        self.assertEqual(bad.cls, L.FAIL_EXPECT)
        self.assertIn("aaaaaaaaaaaa", bad.detail)

    def test_expect_sha256_match_is_accepted(self):
        f = self.art_facts(member=("a" * 64, 10))
        self.assertEqual(self.decide(f, artifact=self.ART, expect_sha256="a" * 64).action, "use_artifact")

    def test_expect_sha256_must_be_hex(self):
        self.assertEqual(self.decide(self.art_facts(), artifact=self.ART, expect_sha256="NOPE").cls, L.FAIL_USAGE)

    def test_without_expect_sha256_nothing_is_compared(self):
        self.assertEqual(self.decide(self.art_facts(member=("b" * 64, 10)), artifact=self.ART).action, "use_artifact")

    def test_a_directory_is_used_in_standalone_without_a_key_and_refused_in_strict(self):
        import tempfile as tf

        d_ = Path(self.tmp) / "tree"
        for rel in L.RECIPE["required_in_tree"]:
            (d_ / rel).parent.mkdir(parents=True, exist_ok=True)
            (d_ / rel).write_text("x")
        f = self.Facts(self, dirs=(str(d_),))
        d = self.decide(f, artifact=str(d_))
        self.assertEqual((d.action, d.extra["key_verified"]), ("use_tree", False))
        f2 = self.Facts(self, dirs=(self.tmp,))
        self.assertEqual(self.decide(f2, artifact=self.tmp).cls, L.FAIL_TREE)

    def test_strict_refuses_a_directory_because_it_has_no_key(self):
        d_ = Path(self.tmp) / "tree2"
        for rel in L.RECIPE["required_in_tree"]:
            (d_ / rel).parent.mkdir(parents=True, exist_ok=True)
            (d_ / rel).write_text("x")
        f = self.Facts(self, dirs=(str(d_),))
        self.assertEqual(self.decide(f, artifact=str(d_), mode="strict").cls, L.FAIL_MANIFEST)
        self.assertEqual(self.decide(f, artifact=str(d_), mode="worker").cls, L.FAIL_MANIFEST)

    def test_a_cache_listing_is_fetched_and_a_standalone_run_does_not_enforce_its_hash(self):
        listing = {"key": "c" * 64, "sha256": "9" * 64, "size": 5}
        d = self.decide(cache_listing=listing)
        self.assertEqual(d.action, "fetch")
        self.assertIsNone(d.extra["enforce_pin"])
        self.assertTrue(d.extra["record_sha256"])
        self.assertNotIn("pin", d.extra["verify_after"])
        w = self.decide(cache_listing=listing, mode="worker")
        self.assertEqual(w.extra["enforce_pin"], listing)
        self.assertIn("pin", w.extra["verify_after"])
        self.assertIn("key", w.extra["verify_after"])

    def test_a_listing_without_a_key_is_refused(self):
        self.assertEqual(self.decide(cache_listing={"sha256": "9" * 64}).cls, L.FAIL_MANIFEST)

    def test_a_stale_listing_with_a_source_rebuilds_in_standalone(self):
        d = self.decide(cache_listing={"key": "d" * 64, "sha256": "9" * 64, "size": 5}, source="/src")
        self.assertEqual(d.action, "build")
        self.assertIn("UNREAL_CACHE_KEY_MISMATCH", d.extra["note"])

    def test_a_stale_listing_with_a_source_fails_in_strict(self):
        s = self.decide(cache_listing={"key": "d" * 64}, source="/src", mode="strict")
        self.assertEqual((s.action, s.cls), ("fail", L.FAIL_KEY_MISMATCH))
        self.assertIn("dddddddddddd", s.detail)

    def test_matching_listing_with_a_source_fetches_instead_of_building(self):
        d = self.decide(cache_listing={"key": "c" * 64}, source="/src")
        self.assertEqual(d.action, "fetch")

    def test_a_source_alone_is_built_and_the_output_says_hours_and_disk(self):
        d = self.decide(source="/src", cache_out="/cache")
        self.assertEqual(d.action, "build")
        self.assertIn("package", d.extra["phases"])
        self.assertIn("GB", d.extra["plan"])
        self.assertIn("h", d.extra["plan"])
        self.assertIn("hours", d.detail)
        self.assertNotIn("package", self.decide(source="/src").extra["phases"])

    def test_a_worker_never_builds_from_a_source_path(self):
        d = self.decide(mode="worker", source="/src")
        self.assertEqual((d.action, d.cls), ("fail", L.FAIL_WORKER_BUILD))

    def test_a_worker_never_clones_a_source(self):
        d = self.decide(mode="worker", source_clone="O/R")
        self.assertEqual((d.action, d.cls), ("fail", L.FAIL_WORKER_BUILD))

    def test_a_worker_with_a_cache_and_a_source_is_still_refused(self):
        d = self.decide(mode="worker", source="/src", cache_listing={"key": "c" * 64})
        self.assertEqual(d.cls, L.FAIL_WORKER_BUILD)

    def test_nothing_given_skips_in_standalone_fails_in_strict_and_misses_in_worker(self):
        s = self.decide()
        self.assertEqual((s.action, s.cls, s.exit_code), ("skip", L.SKIP_NO_SOURCE_OR_CACHE, 0))
        st = self.decide(mode="strict")
        self.assertEqual((st.action, st.cls, st.exit_code), ("fail", L.SKIP_NO_SOURCE_OR_CACHE, 3))
        w = self.decide(mode="worker")
        self.assertEqual((w.action, w.cls, w.exit_code), ("fail", L.FAIL_CACHE_MISS, 3))
        self.assertIn("rebuild", w.detail)

    def test_the_order_is_artifact_then_listing_then_source(self):
        f = self.art_facts()
        self.assertEqual(self.decide(f, artifact=self.ART, cache_listing={"key": "c" * 64}, source="/src").action, "use_artifact")
        self.assertEqual(self.decide(cache_listing={"key": "c" * 64}, source="/src").action, "fetch")
        self.assertEqual(self.decide(source="/src").action, "build")

    def test_clone_needs_a_login_and_repository_access(self):
        for why in ("gh_not_logged_in", "no_repo_access", "gh_not_installed"):
            f = self.Facts(self, gh=(False, why))
            s = self.decide(f, source_clone="O/R")
            self.assertEqual((s.action, s.cls, s.exit_code), ("skip", L.SKIP_NO_SOURCE_OR_CACHE, 0))
            self.assertIn(why, s.detail)
        d = self.decide(self.Facts(self, gh=(True, "")), source_clone="O/R", source_ref="rel")
        self.assertEqual(d.action, "build")
        self.assertTrue(d.extra["clone"] == "O/R" and "clone" in d.extra["phases"])
        self.assertIsNone(d.key)

    def test_strict_fails_when_the_clone_is_not_possible(self):
        f = self.Facts(self, gh=(False, "gh_not_logged_in"))
        d = self.decide(f, source_clone="O/R", mode="strict")
        self.assertEqual((d.action, d.cls, d.exit_code), ("fail", L.FAIL_CLONE, 3))

    def test_the_manifest_path_defaults_to_the_directory_of_the_artifact(self):
        self.assertEqual(RS._manifest_path_for("/a/b/x.tar.zst", None), "/a/b/manifest.json")
        self.assertEqual(RS._manifest_path_for("/a/b/x.tar.zst", "/m/y.json"), "/m/y.json")

    def test_source_and_clone_together_are_refused(self):
        self.assertEqual(self.decide(source="/s", source_clone="O/R").cls, L.FAIL_USAGE)

    def test_a_bad_mode_is_refused(self):
        self.assertEqual(self.decide(mode="loose").cls, L.FAIL_USAGE)

    def test_the_real_gh_check_never_prompts_and_names_its_reasons(self):
        src = (UE / "build_installed.py").read_text()
        self.assertIn('"auth", "status"', src)
        self.assertEqual(L.ENV_FORCED["GH_PROMPT_DISABLED"], "1")
        self.assertEqual(B.RealFacts().gh_access("bad repo name"), (False, "bad_repository_name"))

    # ------------------------------------------------------------------------------------------------ the command line
    def cli(self, *argv):
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            rc = B.main(list(argv))
        return rc, out.getvalue(), err.getvalue()

    def test_cli_resolve_with_nothing_is_a_skip_and_exit_zero(self):
        rc, out, _ = self.cli("resolve")
        self.assertEqual(rc, 0)
        self.assertIn("UNREAL_NO_SOURCE_OR_CACHE", out)

    def real_artifact(self, name="real"):
        d = Path(self.tmp) / name
        d.mkdir()
        art = d / "artifact.tar.zst"
        art.write_bytes(b"payload")
        sha, size = L.sha256_file(str(art))
        (d / "manifest.json").write_text(json.dumps(self.manifest(sha256=sha, size=size)))
        return art, sha

    def test_cli_verify_checks_an_expected_hash_when_one_is_given(self):
        art, sha = self.real_artifact()
        rc, out, _ = self.cli("verify", str(art), "--expect-sha256", "a" * 64)
        self.assertEqual(rc, 3)
        self.assertIn("UNREAL_EXPECT_SHA256_MISMATCH", out)
        rc, out, _ = self.cli("verify", str(art), "--expect-sha256", sha)
        self.assertEqual(rc, 0)
        rc, out, _ = self.cli("verify", str(art))
        self.assertEqual(rc, 0)
        self.assertIn(sha, out)

    def test_cli_resolve_refuses_a_directory_in_strict_mode(self):
        d_ = Path(self.tmp) / "tree3"
        for rel in L.RECIPE["required_in_tree"]:
            (d_ / rel).parent.mkdir(parents=True, exist_ok=True)
            (d_ / rel).write_text("x")
        rc, out, _ = self.cli("resolve", "--unreal-artifact", str(d_))
        self.assertEqual(rc, 0)
        rc, out, _ = self.cli("resolve", "--mode", "strict", "--unreal-artifact", str(d_))
        self.assertEqual(rc, 3)
        self.assertIn("UNREAL_MANIFEST_INVALID", out)

    def test_cli_resolve_skips_when_a_clone_is_not_possible(self):
        rc, out, _ = self.cli("resolve", "--unreal-source-clone", "bad repo name")
        self.assertEqual(rc, 0)
        self.assertIn("bad_repository_name", out)
        self.assertIn("UNREAL_NO_SOURCE_OR_CACHE", out)

    def test_cli_resolve_in_strict_mode_without_anything_fails(self):
        rc, out, _ = self.cli("resolve", "--mode", "strict")
        self.assertEqual(rc, 3)
        self.assertIn("UNREAL_NO_SOURCE_OR_CACHE", out)

    def test_cli_build_without_the_modify_flag_is_refused_before_anything_runs(self):
        src = make_tree(Path(self.tmp) / "cb")
        rc, _, err = self.cli("build", "--source", str(src), "--work", str(Path(self.tmp) / "w"), "--source-id", "unpacked-20261117")
        self.assertEqual(rc, 3)
        self.assertIn("UNREAL_MODIFY_SOURCE_NOT_ALLOWED", err)
        self.assertFalse((Path(self.tmp) / "w").exists())

    def test_cli_resolve_in_worker_mode_without_a_cache_is_a_failure(self):
        rc, out, _ = self.cli("resolve", "--mode", "worker")
        self.assertEqual(rc, 3)
        self.assertIn("UNREAL_CACHE_MISS", out)

    def test_cli_key_prints_the_inputs(self):
        src = make_tree(Path(self.tmp) / "k")
        rc, out, _ = self.cli("key", "--source", str(src), "--source-id", "unpacked-20261117")
        self.assertEqual(rc, 0)
        doc = json.loads(out)
        self.assertEqual(doc["key"], L.cache_key(doc["inputs"]))

    def test_cli_refusal_prints_a_named_class(self):
        rc, _, err = self.cli("key", "--source", str(Path(self.tmp) / "nope"), "--source-id", "unpacked-20261117")
        self.assertEqual(rc, 3)
        self.assertIn("KF_UNREAL_REFUSED class=UNREAL_SOURCE_LAYOUT", err)

    def test_cli_plan_runs_nothing_and_prints_estimates(self):
        rc, out, _ = self.cli("plan", "--package")
        self.assertEqual(rc, 0)
        self.assertIn("ESTIMATE", out)

    def test_upload_prints_commands_and_does_not_run_them_without_execute(self):
        art = Path(self.tmp) / "a.zst"
        art.write_bytes(b"data")
        sha, size = L.sha256_file(str(art))
        m = self.manifest(sha256=sha, size=size)
        mp = Path(self.tmp) / "manifest.json"
        mp.write_text(json.dumps(m))
        orig = L.Runner.capture
        L.Runner.capture = lambda *a, **k: self.fail("ran a command")
        try:
            rc, out, _ = self.cli("upload", "--manifest", str(mp), "--artifact", str(art), "--endpoint", "https://s3.example.org", "--bucket", "my-bucket")
        finally:
            L.Runner.capture = orig
        self.assertEqual(rc, 0)
        self.assertIn("aws s3 cp", out)
        self.assertIn("manifest.json", out)
        self.assertNotIn("KF_UNREAL_UPLOADED", out)

    def test_a_manifest_with_the_wrong_size_refuses_an_upload(self):
        art = Path(self.tmp) / "a.zst"
        art.write_bytes(b"data")
        m = self.manifest(sha256=L.sha256_file(str(art))[0], size=5)
        mp = Path(self.tmp) / "manifest.json"
        mp.write_text(json.dumps(m))
        rc, out, err = self.cli("upload", "--manifest", str(mp), "--artifact", str(art), "--endpoint", "https://s3.example.org", "--bucket", "my-bucket")
        self.assertEqual(rc, 3)
        self.assertIn("UNREAL_ARTIFACT_INTEGRITY", err)

    def test_upload_refuses_a_bad_endpoint_before_printing_anything(self):
        art = Path(self.tmp) / "a.zst"
        art.write_bytes(b"data")
        m = self.manifest(sha256=L.sha256_file(str(art))[0], size=4)
        mp = Path(self.tmp) / "manifest.json"
        mp.write_text(json.dumps(m))
        rc, out, err = self.cli("upload", "--manifest", str(mp), "--artifact", str(art), "--endpoint", "ftp://nope", "--bucket", "my-bucket")
        self.assertEqual((rc, out), (3, ""))
        self.assertIn("UNREAL_USAGE", err)

    def test_upload_without_execute_says_it_only_printed(self):
        art = Path(self.tmp) / "a.zst"
        art.write_bytes(b"data")
        m = self.manifest(sha256=L.sha256_file(str(art))[0], size=4)
        mp = Path(self.tmp) / "manifest.json"
        mp.write_text(json.dumps(m))
        orig = L.Runner.capture
        L.Runner.capture = lambda *a, **k: self.fail("ran a command")
        try:
            rc, out, _ = self.cli("upload", "--manifest", str(mp), "--artifact", str(art), "--endpoint", "https://s3.example.org", "--bucket", "my-bucket")
        finally:
            L.Runner.capture = orig
        self.assertIn("printed only", out)

    def test_upload_with_execute_runs_the_commands_with_only_the_aws_variables(self):
        art = Path(self.tmp) / "a.zst"
        art.write_bytes(b"data")
        m = self.manifest(sha256=L.sha256_file(str(art))[0], size=4)
        mp = Path(self.tmp) / "manifest.json"
        mp.write_text(json.dumps(m))
        seen = []
        orig = L.Runner.capture
        L.Runner.capture = lambda self_, argv, cwd=None, env=None, timeout=300: (seen.append((argv, env)), (0, b""))[1]
        old = dict(os.environ)
        os.environ["AWS_ACCESS_KEY_ID"], os.environ["GH_TOKEN"] = "id", "ghsecret"
        try:
            rc, out, _ = self.cli("upload", "--manifest", str(mp), "--artifact", str(art), "--endpoint", "https://s3.example.org", "--bucket", "my-bucket", "--execute")
        finally:
            L.Runner.capture = orig
            os.environ.clear()
            os.environ.update(old)
        self.assertEqual(rc, 0)
        self.assertEqual(len(seen), 2)
        self.assertNotIn("GH_TOKEN", seen[0][1])
        self.assertEqual(seen[0][1]["AWS_ACCESS_KEY_ID"], "id")
        self.assertNotIn("id", seen[0][0])

    def test_upload_refuses_an_artifact_that_does_not_match_its_manifest(self):
        art = Path(self.tmp) / "a.zst"
        art.write_bytes(b"data")
        m = self.manifest(sha256="1" * 64, size=4)
        mp = Path(self.tmp) / "manifest.json"
        mp.write_text(json.dumps(m))
        rc, out, err = self.cli("upload", "--manifest", str(mp), "--artifact", str(art), "--endpoint", "https://s3.example.org", "--bucket", "my-bucket")
        self.assertEqual(rc, 3)
        self.assertIn("UNREAL_ARTIFACT_INTEGRITY", err)
        self.assertEqual(out, "")

    def test_pin_command_prints_a_pin_a_manifest_object_and_a_download_manifest(self):
        art = Path(self.tmp) / "a.zst"
        art.write_bytes(b"data")
        m = self.manifest(sha256=L.sha256_file(str(art))[0], size=4)
        mp = Path(self.tmp) / "manifest.json"
        mp.write_text(json.dumps(m))
        rc, out, _ = self.cli("pin", "--manifest", str(mp), "--prefix", "my/prefix")
        doc = json.loads(out)
        self.assertEqual(rc, 0)
        self.assertEqual([o["role"] for o in doc["objects"]], ["artifact", "manifest"])
        self.assertTrue(doc["objects"][0]["loads_software"])
        self.assertTrue(doc["objects"][0]["object"].startswith("my/prefix/unreal-5.1.1-"))
        self.assertIn("standalone runs only record", doc["note"])

    # ------------------------------------------------------------------------------------------------ rows
    def rows(self):
        return json.load(open(UE / "rows.json"))

    def test_the_committed_rows_are_valid_and_all_start_in_discovery(self):
        d = self.rows()
        self.assertEqual(L.validate_rows(d), [])
        self.assertTrue(all(r["lane"] == "discovery" for r in d["rows"]))
        self.assertGreaterEqual(len(d["rows"]), 8)

    def test_a_row_in_the_gate_lane_is_refused(self):
        d = self.rows()
        d["rows"][0]["lane"] = "gate"
        self.assertTrue(L.validate_rows(d))

    def test_a_row_without_host_verification_is_refused(self):
        d = self.rows()
        d["rows"][1]["verify"] = []
        self.assertTrue(L.validate_rows(d))
        d["rows"][1]["verify"] = ["exit_zero_only_is_never_enough"]
        self.assertTrue(L.validate_rows(d))

    def test_a_row_without_a_gpu_assertion_must_say_why(self):
        d = self.rows()
        r = [x for x in d["rows"] if x["gpu_assert"] == "none"][0]
        r["notes"] = ""
        self.assertTrue(L.validate_rows(d))

    def test_every_row_that_renders_is_checked_by_an_image_and_a_gpu_assertion(self):
        for r in self.rows()["rows"]:
            if r["id"].startswith("ue_scene"):
                self.assertIn("image_nonflat", r["verify"])
                self.assertIn("image_parity", r["verify"])
                self.assertIn(r["gpu_assert"], ("util", "mem"))

    def test_the_automation_rows_use_a_report_and_the_names_found_in_source(self):
        by = {r["id"]: r for r in self.rows()["rows"]}
        self.assertIn("Rendering.RHI", " ".join(by["ue_rhi_automation"]["cmd"]))
        self.assertIn("System.Renderer.RayTracing.BasicRayTracing", " ".join(by["ue_rt_automation"]["cmd"]))
        self.assertEqual(by["ue_rt_automation"]["names"]["System.Renderer.RayTracing.BasicRayTracing"], "VERIFIED_IN_SOURCE")

    def test_the_public_files_name_no_private_source_or_version(self):
        for p in list(UE.glob("*.py")) + [UE / "rows.json"]:
            t = p.read_text()
            for bad in ("reindertpelsma/Unreal", "UnrealEngine/contents", "UnrealEngine/git"):
                self.assertNotIn(bad, t, p.name)


if __name__ == "__main__":
    unittest.main(verbosity=1)
