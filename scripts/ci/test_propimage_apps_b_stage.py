#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of scripts/ci/propimage_apps_b_stage.py (2026-11-16): pins and manifest, the boot plan and command, row collection and
grading (PASS_LIMITED, unproven passes, an unasserted GPU row), and a whole fake run (boots, offline proof, cleanup, evidence copy)."""
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
import propimage_apps_b_stage as sb  # noqa: E402
import test_propimage_apps as tpa  # noqa: E402

MOD = sb.apps_module(pa.ROOT)
ORDER = tuple(MOD.CHECKS)
H64 = "c" * 64


def pins_doc(unset=()):
    doc = json.loads(sb.PINS_PATH.read_text())
    for it in doc["items"]:
        if it["role"] not in unset:
            it["sha256"], it["size"] = it["sha256"] or H64, it["size"] or 1234567
    return doc


class Pins(unittest.TestCase):
    def test_committed_file_is_valid_and_gop_is_the_named_gap(self):
        doc = pa.load_pins(sb.PINS_PATH)
        roles = {i["role"] for i in doc["items"]}
        self.assertEqual(roles, {"image", "gop", "data", "libtpms", "swtpm"})
        by = {i["role"]: i for i in doc["items"]}
        self.assertEqual(by["image"]["sha256"], "fe75afa169e65770f974fbcc3a2cb018ff14d58bd39b298539128cc6edcc154a")
        self.assertEqual(by["image"]["size"], 62506351001)
        with self.assertRaises(pa.Refused) as c:
            sb.build_manifest(doc, False)
        self.assertEqual(c.exception.reason, "PIN_UNSET:gop")

    def test_data_pin_is_required_and_manifest_has_it(self):
        with self.assertRaises(pa.Refused) as c:
            sb.build_manifest(pins_doc(unset=("data",)) if False else _no(pins_doc(), "data"), False)
        self.assertEqual(c.exception.reason, "PIN_UNSET:data")
        m = sb.build_manifest(pins_doc(), False)
        names = [i["name"] for i in m["items"]]
        self.assertIn("test-assets-20261116.iso", names)
        parsed = box_downloader.parse_manifest(json.dumps(m).encode())
        by = {i.name: i for i in parsed.items}
        self.assertEqual([s.kind for s in by["propimage-kfimg-20261116.zip"].sources], ["signed"])
        self.assertEqual([s.kind for s in by["test-assets-20261116.iso"].sources], ["mirror"])

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
        c = sb.boot_command(Path("/c.py"), dict(a, gpu_minor=1))
        self.assertEqual(c[c.index("--gpu-minor") + 1], "1")


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
        apps = [{"app": a, "verdict": "PASS"} for a in ORDER]
        self.assertEqual(sb.grade(apps, ORDER)[0], "PASS")
        apps[3]["verdict"] = "PASS_LIMITED"
        self.assertEqual(sb.grade(apps, ORDER)[0], "PASS")
        apps[2]["verdict"] = "FAIL"
        self.assertEqual(sb.grade(apps, ORDER)[:2], ("FAIL", "APP_FAIL:" + ORDER[2]))
        apps[2]["verdict"] = "HARNESS"
        self.assertEqual(sb.grade(apps, ORDER)[0], "RETRY")


class Run(unittest.TestCase):
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
                   "gpu": {"required": list(gpu), "verdict": "PASS", "peak": {"gpu": 60}}}
            (d / "verdict.json").write_text(json.dumps(doc))
            (d / "result.json").write_text("{}")
            (d / "out.png").write_bytes(b"\x89PNG not evidence")
            (d / (app + "_gpu.json")).write_text("{}")
            rows.append({"step": "app:" + app, "verdict": verdict, "evidence": doc["evidence"], "class": "app", "detail": "d"})
            mark.line("app:%s %s %s d\n" % (app, verdict, doc["evidence"]))
        (out / "result.json").write_text(json.dumps(rows))
        return 0

    def test_whole_run_passes_and_copies_text_evidence_only(self):
        env = self.make()
        ev = env.tmp / "evidence"
        rec = sb.run(env.ctx, str(ev))
        self.assertEqual((rec["verdict"], rec["reason"]), ("PASS", ""), rec)
        self.assertEqual([a["app"] for a in rec["apps"]], list(ORDER))
        self.assertEqual(sorted(rec["limited"]), sorted(MOD.LAUNCH_ONLY))
        self.assertTrue(all(b["offline"] for b in rec["boots"]))
        self.assertIn("--app-set", self.argv)
        copied = {p.name for p in ev.rglob("*") if p.is_file()}
        self.assertIn("verdict.json", copied)
        self.assertIn("photoshop_gpu.json", copied)
        self.assertNotIn("out.png", copied)
        self.assertFalse((env.ctx.prefix / "unpacked").exists())

    def test_one_failing_row_fails_the_stage_and_names_it(self):
        rec = sb.run(self.make(aftereffects="FAIL").ctx)
        self.assertEqual((rec["verdict"], rec["reason"]), ("FAIL", "APP_FAIL:aftereffects"))

    def test_unset_gop_pin_refuses_before_anything_is_fetched(self):
        env = tpa.Env(self)           # the committed pins: gop unset
        env.ctx.pins_path = sb.PINS_PATH
        rec = sb.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"]), ("REFUSED", "PIN_UNSET:gop"))


if __name__ == "__main__":
    unittest.main()
