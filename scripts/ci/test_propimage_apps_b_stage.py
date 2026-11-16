#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of scripts/ci/propimage_apps_b_stage.py (2026-11-16): pins and manifest, the boot plan and command, row collection and
grading (PASS_LIMITED, unproven passes, an unasserted GPU row, gate versus discovery lanes), a whole fake run (boots, offline proof, cleanup,
evidence copy), the record's validation in vast_report.py (gate rows decide, discovery rows are the breakages table, strict and bounded) and
the payload wiring of the manual-only stage `propimage_apps_b`."""
import json
import os
import shutil
import sys
import tempfile
import unittest
import unittest.mock
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import box_downloader  # noqa: E402
import propimage_apps as pa  # noqa: E402
import profile_resources  # noqa: E402
import propimage_apps_b_stage as sb  # noqa: E402
import test_propimage_apps as tpa  # noqa: E402
import vast_payload  # noqa: E402
import vast_report  # noqa: E402

MOD = sb.apps_module(pa.ROOT)
ORDER = tuple(MOD.ENABLED)
H64 = "c" * 64
SHA = "a" * 40


def pins_doc(unset=()):
    doc = json.loads(sb.PINS_PATH.read_text())
    for it in doc["items"]:
        if it["role"] not in unset:
            it["sha256"], it["size"] = it["sha256"] or H64, it["size"] or 1234567
    return doc


class Pins(unittest.TestCase):
    def test_committed_file_is_valid_and_fully_pinned(self):
        doc = pa.load_pins(sb.PINS_PATH)
        roles = {i["role"] for i in doc["items"]}
        self.assertEqual(roles, {"image", "gop", "data", "libtpms", "swtpm"})
        by = {i["role"]: i for i in doc["items"]}
        self.assertEqual(by["image"]["sha256"], "fe75afa169e65770f974fbcc3a2cb018ff14d58bd39b298539128cc6edcc154a")
        self.assertEqual(by["image"]["size"], 62506351001)
        self.assertEqual(by["image"]["name"], "propimage-kfimg-20261116.zip")
        self.assertEqual(sb.build_manifest(doc, True)["stages"], ["tools", "image"])

    def test_data_pin_is_required_and_manifest_has_it(self):
        with self.assertRaises(pa.Refused) as c:
            sb.build_manifest(pins_doc(unset=("data",)) if False else _no(pins_doc(), "data"), False)
        self.assertEqual(c.exception.reason, "PIN_UNSET:data")
        m = sb.build_manifest(pins_doc(), False)
        names = [i["name"] for i in m["items"]]
        self.assertIn("test-assets-20261116b.iso", names)
        parsed = box_downloader.parse_manifest(json.dumps(m).encode())
        by = {i.name: i for i in parsed.items}
        self.assertEqual([s.kind for s in by["propimage-kfimg-20261116.zip"].sources], ["signed"])
        self.assertEqual([s.kind for s in by["test-assets-20261116b.iso"].sources], ["mirror"])

    def test_no_vendor_or_licence_wording_in_the_pins(self):
        text = sb.PINS_PATH.read_text().lower()
        for bad in ("adobe", "licen", "serial", "account"):
            if bad == "licen":
                continue        # the open-asset licence file is a public bundle member, not image wording
            self.assertNotIn(bad, text)


def _no(doc, role):
    for it in doc["items"]:
        if it["role"] == role:
            it["sha256"] = it["size"] = None
    return doc


class Plan(unittest.TestCase):
    def test_plan_covers_every_row_once_and_respects_the_budget(self):
        timeouts = {a: MOD.CHECKS[a]["timeout"] for a in ORDER}
        boots = sb.plan_boots(timeouts, ORDER)
        self.assertEqual([a for b in boots for a in b], list(ORDER))
        for b in boots:
            self.assertLessEqual(sum(timeouts[a] for a in b), sb.BOOT_BUDGET_S)

    def test_boot_command(self):
        a = {"dir": "/i", "work": "/w", "out": "/o", "qemu": "/q", "flags": "/f", "broker": "/b", "gop": "/g", "data": "/d.iso", "apps": ["photoshop"]}
        c = sb.boot_command(Path("/c.py"), a)
        self.assertEqual(c[c.index("--app-set") + 1], "b")
        self.assertEqual(c[c.index("--data-iso") + 1], "/d.iso")
        self.assertNotIn("--gpu-minor", c)


class Collect(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="kf-pab-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def row(self, app, verdict="PASS", limited=False, gpu=None, ok=True, evidence="FILE", **extra):
        d = self.tmp / app / os.urandom(16).hex()
        d.mkdir(parents=True)
        doc = {"app": app, "ok": ok, "evidence": evidence, "limited": limited, "secs": 42, "signal": "x"}
        if gpu is not None:
            doc["gpu"] = gpu
        doc.update(extra)
        (d / "verdict.json").write_text(json.dumps(doc))
        return {"step": "app:" + app, "verdict": verdict, "evidence": evidence, "class": "app", "detail": "d"}

    GPU = {"required": ["gpu"], "verdict": "PASS", "peak": {"gpu": 55, "mem_mb": 2000}}

    def test_pass_with_asserted_gpu(self):
        r = sb.collect_app(self.tmp, "photoshop", [self.row("photoshop", gpu=self.GPU)], [], ("gpu",))
        self.assertEqual(r["verdict"], "PASS")
        self.assertEqual(r["gpu"]["peak"]["gpu"], 55)

    def test_pass_without_the_gpu_record_is_unproven(self):
        r = sb.collect_app(self.tmp, "photoshop", [self.row("photoshop")], [], ("gpu",))
        self.assertEqual(r["verdict"], "FAIL")
        self.assertIn("UNPROVEN_PASS", r["detail"])
        r = sb.collect_app(self.tmp, "photoshop", [self.row("photoshop", gpu=dict(self.GPU, verdict="FAIL"))], [], ("gpu",))
        self.assertEqual(r["verdict"], "FAIL")
        r = sb.collect_app(self.tmp, "photoshop", [self.row("photoshop", gpu=dict(self.GPU, required=[]))], [], ("gpu",))
        self.assertEqual(r["verdict"], "FAIL")

    def test_limited_row_needs_the_limited_flag(self):
        r = sb.collect_app(self.tmp, "premiere", [self.row("premiere", "PASS_LIMITED", True, evidence="UIA")], [], ())
        self.assertEqual((r["verdict"], r["limited"]), ("PASS_LIMITED", True))
        r = sb.collect_app(self.tmp, "premiere", [self.row("premiere", "PASS_LIMITED", False, evidence="UIA")], [], ())
        self.assertEqual(r["verdict"], "FAIL")
        r = sb.collect_app(self.tmp, "premiere", [self.row("premiere", "PASS", True, evidence="UIA")], [], ())
        self.assertEqual(r["verdict"], "FAIL")

    def test_missing_and_failed(self):
        self.assertEqual(sb.collect_app(self.tmp, "acrobat", [], [], ())["verdict"], "MISSING")
        r = sb.collect_app(self.tmp, "acrobat", [self.row("acrobat", "FAIL", ok=False)], [], ())
        self.assertEqual(r["verdict"], "FAIL")

    def test_grade(self):
        apps = [{"app": a, "verdict": "PASS", "lane": MOD.CHECKS[a]["lane"]} for a in ORDER]
        gate = [a for a in ORDER if MOD.CHECKS[a]["lane"] == "gate"]
        disc = [a for a in ORDER if MOD.CHECKS[a]["lane"] == "discovery"]
        self.assertEqual(sb.grade(apps, ORDER)[0], "PASS")
        apps[3]["verdict"] = "PASS_LIMITED"
        self.assertEqual(sb.grade(apps, ORDER)[0], "PASS")
        by = {a["app"]: a for a in apps}
        by[gate[2]]["verdict"] = "FAIL"
        self.assertEqual(sb.grade(apps, ORDER), ("FAIL", "APP_FAIL:" + gate[2], [gate[2]]))
        by[gate[2]]["verdict"] = "HARNESS"
        self.assertEqual(sb.grade(apps, ORDER)[0], "RETRY")
        by[gate[2]]["verdict"] = "PASS"
        for v in ("FAIL", "HARNESS", "MISSING", "NOT_RUN", "INCONCLUSIVE"):     # a discovery row never decides
            by[disc[0]]["verdict"] = v
            self.assertEqual(sb.grade(apps, ORDER), ("PASS", "", []), v)

    def test_error_class(self):
        item = lambda **kw: dict({"verdict": "FAIL", "detail": "", "host_xid": {"count": 0, "lines": []}, "census": {}}, **kw)  # noqa: E731
        self.assertEqual(sb.error_class(item()), "OUTPUT_MISMATCH")
        self.assertEqual(sb.error_class(item(host_xid={"count": 2, "lines": []})), "XID")
        self.assertEqual(sb.error_class(item(census={"counts": {"xid": 1}})), "XID")
        self.assertEqual(sb.error_class(item(census={"counts": {"tdr": 1}})), "TDR")
        self.assertEqual(sb.error_class(item(detail="Assertion failed")), "ASSERT")
        self.assertEqual(sb.error_class(item(verdict="MISSING")), "CRASH")
        self.assertEqual(sb.error_class(item(verdict="MISSING"), 124), "HANG")
        self.assertEqual(sb.error_class(item(), 1), "CRASH")


class RunBase:
    """A whole fake run of the stage (boots, offline proof, cleanup); the test classes below build records with it."""

    def make(self, **scenario):
        env = tpa.Env(self, pins=self.pins())
        env.ctx.pins_path = self.pins_path
        env.ctx.boot_runner = lambda ctx, argv, log, timeout, mark: self.boot(env, scenario, ctx, argv, mark)
        return env

    def pins(self):
        tmp = Path(tempfile.mkdtemp(prefix="kf-pab-pins-"))
        self.addCleanup(shutil.rmtree, tmp, True)
        self.pins_path = tmp / "pins.json"
        self.pins_path.write_text(json.dumps(pins_doc()))
        return self.pins_path

    def boot(self, env, scenario, ctx, argv, mark):
        arg = lambda k: argv[argv.index(k) + 1]  # noqa: E731
        out, work, apps = Path(arg("--out")), Path(arg("--work")), arg("--apps").split(",")
        self.argv = argv
        (work / "run").mkdir(parents=True)
        (work / "run/last_cmd.txt").write_text("qemu -netdev user,id=n0,restrict=on -drive file=%s/run/clone.qcow2\n" % work)
        rows = [{"step": "verify", "verdict": "PASS", "evidence": "NONE", "class": "harness", "detail": "ok"}]
        mark.line("verify PASS NONE ok\n")
        for app in apps:
            limited = app in MOD.LAUNCH_ONLY
            gpu = MOD.CHECKS[app]["gpu"]
            d = out / app / os.urandom(16).hex()
            d.mkdir(parents=True)
            verdict = scenario.get(app, "PASS_LIMITED" if limited else "PASS")
            ok = verdict in ("PASS", "PASS_LIMITED")
            doc = {"app": app, "ok": ok, "evidence": "UIA" if limited else "FILE", "limited": limited, "secs": 5, "signal": "s",
                   "lane": scenario.get("_lane", MOD.CHECKS[app]["lane"]),
                   "gpu": {"required": list(gpu), "verdict": "PASS", "peak": {"gpu": 60}}}
            (d / "verdict.json").write_text(json.dumps(doc))
            (d / "result.json").write_text("{}")
            (d / "out.png").write_bytes(b"\x89PNG not evidence")
            (d / (app + "_gpu.json")).write_text("{}")
            rows.append({"step": "app:" + app, "verdict": verdict, "evidence": doc["evidence"], "class": "app", "detail": "d"})
            mark.line("app:%s %s %s d\n" % (app, verdict, doc["evidence"]))
        (out / "result.json").write_text(json.dumps(rows))
        return 0


class Run(RunBase, unittest.TestCase):
    def test_whole_run_passes_and_copies_text_evidence_only(self):
        env = self.make()
        ev = env.tmp / "evidence"
        rec = sb.run(env.ctx, str(ev))
        self.assertEqual((rec["verdict"], rec["reason"]), ("PASS", ""), rec)
        self.assertEqual([a["app"] for a in rec["apps"]], list(ORDER))
        self.assertEqual(sorted(rec["limited"]), sorted(a for a in MOD.LAUNCH_ONLY if a in ORDER))
        self.assertIn("animate", rec["not_run"])
        self.assertNotIn("animate", [a["app"] for a in rec["apps"]])
        self.assertTrue(all(b["offline"] for b in rec["boots"]))
        self.assertIn("--app-set", self.argv)
        copied = {p.name for p in ev.rglob("*") if p.is_file()}
        self.assertIn("verdict.json", copied)
        self.assertIn("photoshop_gpu.json", copied)
        self.assertNotIn("out.png", copied)
        self.assertFalse((env.ctx.prefix / "unpacked").exists())

    def test_one_failing_gate_row_fails_the_stage_and_names_it(self):
        rec = sb.run(self.make(illustrator="FAIL").ctx)
        self.assertEqual((rec["verdict"], rec["reason"], rec["failed"]), ("FAIL", "APP_FAIL:illustrator", ["illustrator"]))
        self.assertEqual(rec["breakages"], [])

    def test_a_failing_discovery_row_does_not_fail_the_stage_and_is_listed(self):
        rec = sb.run(self.make(aftereffects="FAIL", photoshop="HARNESS").ctx)
        self.assertEqual((rec["verdict"], rec["reason"], rec["failed"]), ("PASS", "", []), rec)
        self.assertEqual([(b["row"], b["verdict"], b["class"]) for b in rec["breakages"]],
                         [("photoshop", "HARNESS", "CRASH"), ("aftereffects", "FAIL", "OUTPUT_MISMATCH")])
        self.assertEqual({a["app"]: a["lane"] for a in rec["apps"]}["aftereffects"], "discovery")

    def test_the_lane_comes_from_the_table_not_from_the_guest_verdict(self):
        rec = sb.run(self.make(illustrator="FAIL", _lane="discovery").ctx)
        self.assertEqual((rec["verdict"], rec["reason"]), ("FAIL", "APP_FAIL:illustrator"))
        self.assertEqual({a["app"]: a["lane"] for a in rec["apps"]}["illustrator"], "gate")

    def test_breakage_lines_are_printed_with_the_result(self):
        import contextlib
        import io
        rec = sb.run(self.make(aftereffects="FAIL").ctx)
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            self.assertEqual(sb.finish(rec, None), 0)
        lines = buf.getvalue().splitlines()
        self.assertEqual(lines[0], "KF_PROPIMAGE_APPS_B_BREAKAGE row=aftereffects verdict=FAIL class=OUTPUT_MISMATCH")
        self.assertTrue(lines[-1].startswith("KF_PROPIMAGE_APPS_B_RESULT {"))

    def test_unset_data_pin_refuses_before_anything_is_fetched(self):
        env = self.make()
        doc = json.loads(self.pins_path.read_text())
        for it in doc["items"]:
            if it["role"] == "data":
                it["sha256"] = it["size"] = None
        self.pins_path.write_text(json.dumps(doc))
        rec = sb.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"]), ("REFUSED", "PIN_UNSET:data"))


class Report(RunBase, unittest.TestCase):
    """vast_report.validate_propimage_apps_b: strict and bounded; only gate rows decide, discovery rows are a known-breakages table."""

    def record(self, **scenario):
        rec = sb.run(self.make(**scenario).ctx)
        return json.loads(json.dumps(rec))

    def refused(self, rec, label=""):
        with self.subTest(label), self.assertRaises(ValueError):
            vast_report.validate_propimage_apps_b(rec)

    def test_the_row_table_equals_the_application_table(self):
        self.assertEqual({n: (c["lane"], not c.get("disabled")) for n, c in MOD.CHECKS.items()}, vast_report.PROPIMAGE_B_ROWS)
        self.assertEqual(list(MOD.CHECKS), list(vast_report.PROPIMAGE_B_ROWS))

    def test_a_clean_run_validates_and_lists_the_disabled_rows_as_not_run(self):
        rec = self.record()
        vast_report.validate_propimage_apps_b(rec)
        self.assertEqual(sorted(rec["not_run"]), ["animate", "animateprojects", "animatesample", "lightroom"])
        self.assertEqual(rec["breakages"], [])

    def test_discovery_failures_pass_the_stage_and_are_the_breakages_table(self):
        rec = self.record(aftereffects="FAIL", photoshop="HARNESS", mediaencoder="MISSING")
        self.assertEqual(rec["verdict"], "PASS")
        vast_report.validate_propimage_apps_b(rec)
        self.assertEqual([b["row"] for b in rec["breakages"]], ["photoshop", "aftereffects", "mediaencoder"])
        for b in rec["breakages"]:
            self.assertEqual(set(b), {"row", "verdict", "class", "census"})

    def test_a_gate_failure_fails_the_stage(self):
        rec = self.record(illustrator="FAIL")
        self.assertEqual(rec["verdict"], "FAIL")
        vast_report.validate_propimage_apps_b(rec)           # a red record is a valid record
        rec["verdict"], rec["reason"], rec["failed"] = "PASS", "", []
        self.refused(rec, "PASS over a failed gate row")
        rec = self.record(illustrator="FAIL")
        rec["verdict"], rec["reason"] = "PASS", ""
        self.refused(rec, "PASS over a failed gate row, failed list kept")
        rec = self.record()
        next(a for a in rec["apps"] if a["app"] == "acrobat").update(verdict="HARNESS")
        self.refused(rec, "a gate row that is not PASS or PASS_LIMITED under PASS")

    def test_a_pass_needs_every_enabled_gate_row(self):
        rec = self.record()
        rec["apps"] = [a for a in rec["apps"] if a["app"] != "uxp"]
        self.refused(rec, "gate row missing")
        rec = self.record()
        rec["apps"] = [a for a in rec["apps"] if a["app"] != "aeparallel"]
        vast_report.validate_propimage_apps_b(rec)           # a discovery row may be absent
        self.refused(dict(self.record(), not_run={}), "disabled rows not listed")
        rec = self.record()
        rec["not_run"]["photoshop"] = "enabled rows are never not_run"
        self.refused(rec, "an enabled row listed as not_run")

    def test_the_breakages_table_must_match_the_discovery_rows_exactly(self):
        rec = self.record(aftereffects="FAIL")
        for label, fn in (("hidden", lambda r: r["breakages"].clear()),
                          ("extra", lambda r: r["breakages"].append(dict(r["breakages"][0], row="photoshop"))),
                          ("a gate row listed", lambda r: r["breakages"][0].update(row="illustrator")),
                          ("class", lambda r: r["breakages"][0].update({"class": "BANANA"})),
                          ("verdict disagrees", lambda r: r["breakages"][0].update(verdict="PASS")),
                          ("key", lambda r: r["breakages"][0].update(extra=1)),
                          ("census key", lambda r: r["breakages"][0]["census"]["counts"].update(bogus=1)),
                          ("census count type", lambda r: r["breakages"][0]["census"]["counts"].update(xid="1")),
                          ("not a dict", lambda r: r["breakages"].__setitem__(0, "x"))):
            r = json.loads(json.dumps(rec))
            fn(r)
            self.refused(r, label)

    def test_lane_comes_from_the_table_and_a_forged_lane_is_refused(self):
        rec = self.record(illustrator="FAIL")
        next(a for a in rec["apps"] if a["app"] == "illustrator")["lane"] = "discovery"
        rec.update(verdict="PASS", reason="", failed=[])
        self.refused(rec)

    def test_unknown_keys_and_oversized_strings_are_refused(self):
        rec = self.record(aftereffects="FAIL")
        row = lambda r, app: next(a for a in r["apps"] if a["app"] == app)  # noqa: E731
        for label, fn in (("top key", lambda r: r.update(extra=1)),
                          ("app key", lambda r: row(r, "photoshop").update(extra=1)),
                          ("unknown app", lambda r: r["apps"].append(dict(r["apps"][0], app="notepad"))),
                          ("schema", lambda r: r.update(schema=2)), ("image_set", lambda r: r.update(image_set="a")),
                          ("detail length", lambda r: row(r, "photoshop").update(detail="x" * 201)),
                          ("detail non-ascii", lambda r: row(r, "photoshop").update(detail="caf\u00e9")),
                          ("error length", lambda r: r.update(error="x" * 201)),
                          ("not_run reason length", lambda r: r["not_run"].update(animate="x" * 161)),
                          ("not_run reason empty", lambda r: r["not_run"].update(animate="")),
                          ("reason token", lambda r: r.update(reason="free text with spaces")),
                          ("xid lines", lambda r: row(r, "photoshop")["host_xid"].update(lines=["NVRM: Xid"] * 9)),
                          ("xid line length", lambda r: row(r, "photoshop")["host_xid"].update(lines=["x" * 201])),
                          ("evidence count", lambda r: row(r, "photoshop").update(evidence=[{"file": "a%d" % i, "bytes": 1, "sha256": "b" * 64} for i in range(25)])),
                          ("evidence hash", lambda r: row(r, "photoshop").update(evidence=[{"file": "a", "bytes": 1, "sha256": "nothex"}])),
                          ("evidence file name", lambda r: row(r, "photoshop").update(evidence=[{"file": "../a", "bytes": 1, "sha256": "b" * 64}])),
                          ("census ids", lambda r: row(r, "photoshop").update(census={"lines": 1, "counts": {}, "xid": 0, "ids": {"xid": ["0x1"] * 9}})),
                          ("census id length", lambda r: row(r, "photoshop").update(census={"lines": 1, "counts": {}, "xid": 0, "ids": {"xid": ["x" * 25]}})),
                          ("census keyword", lambda r: row(r, "photoshop").update(census={"lines": 1, "counts": {"bogus": 1}, "xid": 0, "ids": {}})),
                          ("census extra key", lambda r: row(r, "photoshop").update(census={"lines": 1, "counts": {}, "xid": 0, "ids": {}, "raw": "x"})),
                          ("gpu key", lambda r: row(r, "photoshop")["gpu"].update(extra=1)),
                          ("gpu peak size", lambda r: row(r, "photoshop")["gpu"].update(peak={"k%d" % i: 1 for i in range(13)})),
                          ("gpu peak type", lambda r: row(r, "photoshop")["gpu"].update(peak={"gpu": "55"})),
                          ("verdict value", lambda r: row(r, "photoshop").update(verdict="OK")),
                          ("limited flag", lambda r: row(r, "photoshop").update(limited=True)),
                          ("order", lambda r: r["apps"].reverse()),
                          ("boots rows", lambda r: r["boots"][0].update(apps=["notepad"])),
                          ("preflight", lambda r: r.update(preflight=[{"name": "x", "status": "MATCH", "detail": "y" * 201}])),
                          ("sha", lambda r: r.update(sha="g" * 40))):
            r = json.loads(json.dumps(rec))
            fn(r)
            self.refused(r, label)

    def test_a_pass_row_needs_a_real_evidence_kind_and_an_offline_proof(self):
        rec = self.record()
        rec["boots"][0]["offline"] = False
        self.refused(rec, "online boot under PASS")
        rec = self.record()
        next(a for a in rec["apps"] if a["app"] == "illustrator")["kind"] = "NONE"
        self.refused(rec, "PASS without evidence kind")

    def test_a_red_or_refused_record_needs_a_reason(self):
        rec = self.record(illustrator="FAIL")
        rec["reason"] = ""
        self.refused(rec, "FAIL without reason")
        refused = {"schema": 1, "image_set": "b", "verdict": "REFUSED", "sha": "a" * 40, "reason": "PIN_UNSET:data", "family": None, "cpu_vendor": None,
                   "preflight": [], "image": {"name": None, "sha256": None, "size": None}, "swtpm": {"version": None, "built": False}, "boots": [],
                   "apps": [], "failed": [], "limited": [], "breakages": [], "not_run": {}}
        vast_report.validate_propimage_apps_b(refused)
        refused["reason"] = ""
        self.refused(refused, "REFUSED without reason")

    def test_the_worst_case_record_is_shrunk_by_the_stage_to_fit_and_still_validates(self):
        rec = self.record(aftereffects="FAIL")
        for a in rec["apps"]:
            a["detail"] = "d" * 200
            a["evidence"] = [{"file": "f%02d_%s.txt" % (i, "x" * 50), "bytes": 9, "sha256": "e" * 64} for i in range(24)]
            a["host_xid"] = {"count": 8, "lines": ["NVRM: Xid " + "9" * 180] * 8}
            a["census"] = {"lines": 9, "counts": {k: 9 for k in vast_report.CENSUS_KEYWORDS}, "xid": 9,
                           "ids": {k: ["0x%022x" % n for n in range(8)] for k in vast_report.CENSUS_KEYWORDS}}
        self.assertGreater(len(json.dumps(rec, separators=(",", ":"))), 49152)
        self.refused(rec, "oversized record")
        tmp = Path(tempfile.mkdtemp(prefix="kf-pab-rec-"))
        self.addCleanup(shutil.rmtree, tmp, True)
        data = sb.write_record(tmp / "r.json", rec)
        self.assertLessEqual(len(data.encode()), sb.MAX_RECORD_BYTES)
        vast_report.validate_propimage_apps_b(json.loads(data))

    def file_with(self, doc):
        tmp = Path(tempfile.mkdtemp(prefix="kf-pab-res-"))
        self.addCleanup(shutil.rmtree, tmp, True)
        (tmp / "r.json").write_text(json.dumps(doc))
        return tmp / "r.json"

    def doc(self, rec, verdict="PASS"):
        return {"verdict": verdict, "suites": [{"name": "propimage_apps_b", "pass": 1, "fail": 0, "skip": 0}], "propimage_apps_b": rec}

    def test_result_file_carries_the_record_and_refuses_a_pass_over_a_red_one(self):
        rec = self.record(aftereffects="FAIL")
        self.assertEqual(vast_report.read_result_file(self.file_with(self.doc(rec)))["propimage_apps_b"]["verdict"], "PASS")
        red = self.record(illustrator="FAIL")
        with self.assertRaises(vast_report.ResultError):
            vast_report.read_result_file(self.file_with(self.doc(red)))
        self.assertEqual(vast_report.read_result_file(self.file_with(self.doc(red, "FAIL")))["verdict"], "FAIL")
        bad = json.loads(json.dumps(rec))
        bad["extra"] = 1
        with self.assertRaises(vast_report.ResultError):
            vast_report.read_result_file(self.file_with(self.doc(bad, "FAIL")))


class PayloadWiring(unittest.TestCase):
    def instruction(self, tier="large"):
        import payload_instruction as pi
        return pi.Instruction(tier=tier, windows=True, windows_runs=1, stages=(), spec=None, winapps=None, linux_apps="middle", rehearsal=False,
                              source="default")

    def test_manual_only_and_planned_only_with_only_in_large(self):
        self.assertFalse(vast_payload.PROPIMAGE_APPS_B_IN_LARGE)
        self.assertEqual(vast_payload.MANUAL_ONLY_STAGES["propimage_apps_b"], "PROPIMAGE_APPS_B_IN_LARGE")
        self.assertNotIn("propimage_apps_b", vast_payload.plan_stage_names(self.instruction()))
        self.assertNotIn("propimage_apps_b", vast_payload.plan_stage_names(self.instruction(), ["propimage_apps"]))
        self.assertIn("propimage_apps_b", vast_payload.plan_stage_names(self.instruction(), ["propimage_apps_b"]))
        self.assertNotIn("propimage_apps", vast_payload.plan_stage_names(self.instruction(), ["propimage_apps_b"]))
        for tier in ("small", "middle"):
            self.assertNotIn("propimage_apps_b", vast_payload.plan_stage_names(self.instruction(tier), ["propimage_apps_b"]))

    def test_runs_after_the_first_image_and_before_the_spec_lane(self):
        import payload_instruction as pi
        inst = pi.Instruction(tier="large", windows=True, windows_runs=2, stages=("specviewperf",), spec="both", winapps="small",
                              linux_apps="middle", rehearsal=False, source="default")
        plan = vast_payload.plan_stage_names(inst, ["propimage_apps", "propimage_apps_b"])
        self.assertEqual(plan[-4:], ["windows_gate_2", "propimage_apps", "propimage_apps_b", "specviewperf"])

    def test_the_budget_guard_covers_the_stage(self):
        data = json.loads((profile_resources.ROOT / "ci/vast-broker/defaults.json").read_text())
        for stage in data["payloadStageResources"].values():
            stage["source"] = "scripts/ci/vast_payload.py"
        source = (HERE / "vast_payload.py").read_text()
        self.assertEqual(profile_resources.validate(data, source), [])
        flipped = source.replace("PROPIMAGE_APPS_B_IN_LARGE = False", "PROPIMAGE_APPS_B_IN_LARGE = True")
        self.assertIn("manual-only stage is in a default plan: propimage_apps_b", profile_resources.validate(data, flipped))
        data["payloadStageResources"]["propimage_apps_b"] = dict(next(iter(data["payloadStageResources"].values())))
        self.assertIn("manual-only stage has a budget row: propimage_apps_b", profile_resources.validate(data, source))

    def line(self, doc):
        return "noise\nKF_PROPIMAGE_APPS_B_RESULT " + json.dumps(doc) + "\n"

    def test_rows_pass_only_on_a_pass_record_with_rc_zero_and_keep_a_red_record(self):
        result = {}
        ok = {"verdict": "PASS", "sha": SHA, "reason": ""}
        self.assertEqual(vast_payload.propimage_apps_b_rows(self.line(ok), 0, SHA, result), [vast_payload.row("propimage_apps_b", 1)])
        self.assertEqual(result["propimage_apps_b"], ok)
        for doc, rc in ((dict(ok, verdict="FAIL"), 1), (ok, 1), (dict(ok, verdict="REFUSED"), 2), (dict(ok, verdict="RETRY"), 3),
                        (dict(ok, sha="c" * 40), 0), (dict(ok, verdict="NOT_RUN"), 0)):
            red = {}
            with self.subTest(verdict=doc["verdict"], rc=rc), self.assertRaises(vast_payload.EvidenceError):
                vast_payload.propimage_apps_b_rows(self.line(doc), rc, SHA, red)
            self.assertEqual(red["propimage_apps_b"], doc)       # the red record stays in the result file for the report
        for text in ("", self.line(ok) * 2, "KF_PROPIMAGE_APPS_B_RESULT {not json}\n", "KF_PROPIMAGE_APPS_RESULT " + json.dumps(ok) + "\n"):
            with self.subTest(text[:30]), self.assertRaises(vast_payload.EvidenceError):
                vast_payload.propimage_apps_b_rows(text, 0, SHA, {})


if __name__ == "__main__":
    unittest.main()
