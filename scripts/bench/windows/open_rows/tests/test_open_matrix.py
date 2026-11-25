# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The open rows inside the Windows app matrix: apps.json rows, the online_source class, verdict.py (KFNOTRUN, proof_all) and the guest push list.
GPU-free. The rows' success regexes are checked against the text the harness really prints."""
import contextlib
import io
import json
import os
import re
import tempfile
import unittest

import helpers as H
import cts_run as U
import dxs_harness as D
import gate_profiles as G
import gl_soak as S
import kfopen_pins as P
import verdict as V
import winapps
from kfopen_common import Result

PINS = P.load_pins()
APPS = {a["id"]: a for a in json.load(open(os.path.join(H.AM, "apps.json")))["apps"]}
PROF = json.load(open(os.path.join(H.AM, "gate_profiles.json")))
# rows that are wired into apps.json; a row flagged matrix=deferred runs through kfopen.py only (hwmon_lhm_nvapi: apps.json is hash-covered by bundle.pin)
OPEN_IDS = sorted(r for r, row in PINS["rows"].items() if row.get("matrix", {}).get("state") != "deferred")


class Rows(unittest.TestCase):
    def test_every_open_row_is_an_apps_json_row_and_the_other_way_round(self):
        mine = {i for i in APPS if i.startswith(("vkglcts_", "dxsamples_"))}
        self.assertEqual(set(OPEN_IDS), mine)

    def test_rows_are_in_the_online_source_class_and_in_no_gate_profile(self):
        self.assertEqual(set(OPEN_IDS), G.online_source(PROF))
        for profile in ("small", "mid", "full"):
            self.assertFalse(set(OPEN_IDS) & set(G.resolve(profile)), profile)
        self.assertEqual(OPEN_IDS, sorted(G.resolve("open_source")))

    def test_open_rows_not_in_full(self):
        opn = set(G.open_apps(json.load(open(os.path.join(H.AM, "apps.json"))), json.load(open(os.path.join(H.AM, "manifest.json"))), PROF))
        self.assertFalse(set(OPEN_IDS) & opn)

    def test_class_is_a_discovery_lane_with_a_named_reason_and_budget(self):
        block = PROF["online_source"]
        self.assertEqual("discovery", block["lane"])
        self.assertGreater(block["budget_s"], 3600)
        self.assertIn("dxsamples_multiadapter", block["not_run"])
        self.assertEqual([], G.check())

    def test_profile_budget_and_unknown_profile(self):
        self.assertEqual(PROF["online_source"]["budget_s"], G.budget("open_source", PROF))
        with self.assertRaises(SystemExit):
            G.resolve("open_sources")

    def test_rows_need_only_public_or_official_packages_and_no_private_object(self):
        for rid in OPEN_IDS:
            self.assertEqual(["python_embed", "vc_redist"], APPS[rid]["pkgs"], rid)
            self.assertEqual({"official_fetch": ["vc_redist"]}, APPS[rid]["requires"], rid)
            self.assertNotRegex(APPS[rid]["ps"], r"(?i)r2|presign|token|secret|\.s3\.", rid)

    def test_ps_bodies_are_ascii_and_call_the_one_entry_point(self):
        for rid in OPEN_IDS:
            ps = APPS[rid]["ps"]
            ps.encode("ascii")
            self.assertIn(r"C:\kf\open_rows\kfopen.py", ps)
            self.assertIn("'run', '%s'" % rid, ps.replace("@@ID@@", rid))
            self.assertIn("exit 3", ps, "a row without its files is NOTFOUND, never a pass")

    def test_control_file_kind_per_row(self):
        for rid in ("vkglcts_gl46", "vkglcts_vk"):
            self.assertIn(r"control\@@ID@@.tsv", APPS[rid]["ps"])
        for rid in ("dxsamples_nbody_async", "dxsamples_work_graphs"):
            self.assertIn(r"control\@@ID@@.json", APPS[rid]["ps"])

    def test_sessions_match_what_each_row_needs(self):
        for rid in ("vkglcts_gl46", "vkglcts_gl_dual_soak", "dxsamples_nbody_async", "dxsamples_execute_indirect", "dxsamples_reserved_resources", "dxsamples_hdr"):
            self.assertEqual("interactive", APPS[rid]["session"], rid)
        self.assertEqual("service", APPS["dxsamples_work_graphs"]["session"])

    def test_timeouts_cover_the_expected_run(self):
        for rid in OPEN_IDS:
            self.assertGreaterEqual(APPS[rid]["timeout_s"], APPS[rid]["expect_s"], rid)
        self.assertGreaterEqual(APPS["vkglcts_gl46"]["timeout_s"], 7200)

    def test_no_full_desktop_screenshot_is_requested(self):
        for rid in OPEN_IDS:
            self.assertIsNone(APPS[rid]["screenshot_s"], rid)

    def test_async_rows_require_all_engines(self):
        for rid in ("dxsamples_nbody_async", "dxsamples_execute_indirect"):
            self.assertTrue(APPS[rid].get("proof_all"), rid)
            self.assertEqual(["pdh:3D", "pdh:Compute"], APPS[rid]["proof"], rid)
        for rid in OPEN_IDS:
            if rid not in ("dxsamples_nbody_async", "dxsamples_execute_indirect"):
                self.assertNotIn("proof_all", APPS[rid], rid)

    def test_proof_all_in_apps_json_matches_open_rows_json(self):
        for rid, row in PINS["rows"].items():
            if row.get("engines_all"):
                self.assertTrue(APPS[rid].get("proof_all"))
                self.assertEqual(["pdh:" + e for e in row["engines_all"]], APPS[rid]["proof"])
            elif row.get("engines_any"):
                self.assertEqual(["pdh:" + "|".join(row["engines_any"])], APPS[rid]["proof"])

    def test_multiadapter_row_is_notrun(self):
        log = "KFNOTRUN dxsamples_multiadapter reason=needs-two-gpus (...)\n"
        d = V.decide(APPS["dxsamples_multiadapter"], {"rc": 3, "secs": 1}, log)
        self.assertEqual("NOTRUN", d["verdict"])

    def test_every_row_has_a_cmd_and_a_reasoned_why(self):
        for rid in OPEN_IDS:
            self.assertIn("kfopen.py run " + rid, APPS[rid]["cmd"])
            self.assertTrue(APPS[rid]["why"].startswith(("Unverified", "Conformance", "One process", "NOT_RUN")), rid)
            self.assertEqual("high", APPS[rid]["difficulty"])


class Verdicts(unittest.TestCase):
    def facts(self, engines, rc=0):
        return {"rc": rc, "secs": 100, "pdh": {"nv": dict(engines), "other": {}}, "smi": {}, "gpu_devices": []}

    def test_kfnotrun_is_notrun_verdict(self):
        app = APPS["vkglcts_gl46"]
        log = "KFOPEN START row=vkglcts_gl46\nKFNOTRUN vkglcts_gl46 reason=no-native-control (discovery run)\n"
        self.assertEqual("NOTRUN", V.decide(app, self.facts({"3d": 50}, rc=3), log)["verdict"])

    def test_kfnotrun_never_hides_a_pass(self):
        app = APPS["dxsamples_work_graphs"]
        log = "KFNOTRUN other row\nKFOPEN VERDICT PASS row=dxsamples_work_graphs uav=8\n"
        self.assertEqual("PASS", V.decide(app, self.facts({"3d": 40}), log)["verdict"])

    def test_proof_all_needs_both_engines(self):
        app = APPS["dxsamples_nbody_async"]
        log = "KFOPEN VERDICT PASS row=dxsamples_nbody_async keys=0\n"
        self.assertEqual("FAIL", V.decide(app, self.facts({"engtype_3d": 60}), log)["verdict"])
        self.assertEqual("FAIL", V.decide(app, self.facts({"engtype_compute_0": 60}), log)["verdict"])
        self.assertEqual("PASS", V.decide(app, self.facts({"engtype_3d": 60, "engtype_compute_0": 30}), log)["verdict"])

    def test_any_proof_rows_need_one_engine(self):
        app = APPS["dxsamples_reserved_resources"]
        log = "KFOPEN VERDICT PASS row=dxsamples_reserved_resources keys=24\n"
        self.assertEqual("PASS", V.decide(app, self.facts({"engtype_3d": 60}), log)["verdict"])
        self.assertEqual("FAIL", V.decide(app, self.facts({}), log)["verdict"])

    def test_eval_proof_default_is_any(self):
        ok, held, missed = V.eval_proof(["pdh:3D", "pdh:Compute"], "", self.facts({"3d": 50}))
        self.assertTrue(ok)
        ok, held, missed = V.eval_proof(["pdh:3D", "pdh:Compute"], "", self.facts({"3d": 50}), require_all=True)
        self.assertFalse(ok)

    def test_a_check_fail_line_fails_the_row_even_with_a_pass_line(self):
        app = APPS["vkglcts_gl_dual_soak"]
        log = "CHECK no_nvoglv_crash FAIL 1000 Faulting module nvoglv64.dll\nKFOPEN VERDICT PASS row=vkglcts_gl_dual_soak seconds=600 share=0\n"
        self.assertEqual("FAIL", V.decide(app, self.facts({"3d": 50}), log)["verdict"])


class RxMatchesWhatTheHarnessPrints(unittest.TestCase):
    """The success regex of every row must match the line the code prints (a row that can never pass is the worst bug of this kind)."""

    def match(self, rid, text):
        self.assertIsNotNone(re.search(APPS[rid]["rx"], text, re.M), "%s: rx %r vs %r" % (rid, APPS[rid]["rx"], text[-200:]))

    def test_cts_rows(self):
        for rid in ("vkglcts_gl46", "vkglcts_vk"):
            buf = io.StringIO()
            native = {"a.b.c": "Pass", "a.b.d": "Fail"}
            with contextlib.redirect_stdout(buf), tempfile.TemporaryDirectory() as t:
                rc = U.emit_verdict(rid, {k: (v, "") for k, v in native.items()}, native, {}, t)
            self.assertEqual(0, rc)
            self.match(rid, buf.getvalue())

    def test_soak_row(self):
        ok = "KFGL renderer NVIDIA X\nKFGL RESULT OK iters=1/1\n"
        r = H.FakeRunner([(lambda a: True, lambda a, c: Result(0, "[]") if a[0] == "powershell.exe" else Result(0, ok, 605.0))])
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            S.run_soak("vkglcts_gl_dual_soak", PINS["rows"]["vkglcts_gl_dual_soak"], r, "kf_gldual.exe")
        self.match("vkglcts_gl_dual_soak", buf.getvalue())

    def test_window_rows(self):
        for rid, row in PINS["rows"].items():
            if row["kind"] != "dx-window":
                continue
            facts = {"captures": [], "titles": [], "keys": D.expand_keys(row["keys"])}
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                D.report(rid, row, [("x", True, "")], facts)
            self.match(rid, buf.getvalue())

    def test_work_graphs_row(self):
        import kfopen as K
        import argparse
        import kfopen_build as B
        from unittest import mock
        UAV = "".join("    UAV[%d] = 0x%x\n" % (i, 4 + i) for i in range(8)) + "Execution complete\n"
        with tempfile.TemporaryDirectory() as t:
            ctx = B.Ctx(PINS, root=t, runner=H.FakeRunner(default=Result(0, UAV)), fetcher=H.FakeFetcher(), log=lambda *a: None)
            a = argparse.Namespace(out=t, control=None, role="guest", allow_different_gpu=False, smoke=False, share=False)
            buf = io.StringIO()
            with mock.patch.object(K, "preflight", lambda *x, **k: None), mock.patch.object(K, "prepare", lambda c, r: {"exe": "x", "cwd": t}), contextlib.redirect_stdout(buf):
                K.run_row(ctx, "dxsamples_work_graphs", a)
        self.match("dxsamples_work_graphs", buf.getvalue())

    def test_no_row_can_pass_by_printing_the_notrun_line(self):
        for rid in OPEN_IDS:
            self.assertIsNone(re.search(APPS[rid]["rx"], "KFNOTRUN %s reason=x\n" % rid, re.M), rid)


class GuestPush(unittest.TestCase):
    def test_open_rows_files_pushed_include_runtime_modules(self):
        files = set(winapps.open_rows_files())
        for need in ("kfopen.py", "kfopen_common.py", "kfopen_pins.py", "kfopen_build.py", "cts_run.py", "cts_results.py", "cts_caselist.py", "dxs_harness.py", "gl_soak.py",
                     "open_rows.json", "caselist_spec.json", "caselists/gl46_core.txt", "caselists/vk_breadth.txt", "kf_gldual.c", "kf_gldual_logic.h",
                     "open_events.ps1", "open_signature.ps1", "soak/CMakeLists.txt"):
            self.assertIn(need, files)

    def test_tests_and_mutants_not_pushed(self):
        files = winapps.open_rows_files()
        self.assertFalse([f for f in files if f.startswith("tests/") or f.endswith("_mutants.py") or "__pycache__" in f])

    def test_fixtures_and_qpa_logs_are_not_pushed(self):
        self.assertFalse([f for f in winapps.open_rows_files() if "fixtures" in f or f.endswith(".qpa")])

    def test_mutant_harness_is_not_pushed(self):
        self.assertNotIn("open_rows_mutants.py", winapps.open_rows_files())

    def test_every_pushed_file_exists_and_is_small_text(self):
        for rel in winapps.open_rows_files():
            p = os.path.join(winapps.OPEN_ROWS_DIR, *rel.split("/"))
            self.assertTrue(os.path.isfile(p), rel)
            self.assertLess(os.path.getsize(p), 200000, rel)

    def test_every_module_the_entry_point_imports_is_pushed(self):
        files = set(winapps.open_rows_files())
        src = open(os.path.join(H.OR, "kfopen.py")).read()
        for mod in re.findall(r"^import (\w+)", src, re.M) + re.findall(r"^import (\w+) as", src, re.M):
            if os.path.exists(os.path.join(H.OR, mod + ".py")):
                self.assertIn(mod + ".py", files, mod)

    def test_the_guest_directory_layout_is_created_before_the_push(self):
        text = open(os.path.join(H.AM, "winapps.py")).read()
        self.assertIn(r"C:\\kf\\open_rows\\caselists", text)
        self.assertIn(r"C:\\kf\\open_rows\\soak", text)


if __name__ == "__main__":
    unittest.main()
