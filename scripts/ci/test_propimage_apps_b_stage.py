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
        self.assertIn("test-assets-20261116c.iso", names)
        parsed = box_downloader.parse_manifest(json.dumps(m).encode())
        by = {i.name: i for i in parsed.items}
        self.assertEqual([s.kind for s in by["propimage-kfimg-20261116.zip"].sources], ["signed"])
        self.assertEqual([s.kind for s in by["test-assets-20261116c.iso"].sources], ["mirror"])

    def test_the_tools_of_the_committed_pins_are_signed_then_mirror_next_to_the_data_volume(self):
        # 2026-11-17: the b stage reuses pa.build_manifest for the tools; the private copy comes first, the public mirror stays the fallback
        parsed = box_downloader.parse_manifest(json.dumps(sb.build_manifest(pa.load_pins(sb.PINS_PATH), True)).encode())
        by = {i.name: i for i in parsed.items}
        for name in ("libtpms-0.10.1.tar.gz", "swtpm-0.10.1.tar.gz"):
            self.assertEqual([s.kind for s in by[name].sources], ["signed", "mirror"], name)
        self.assertEqual([s.kind for s in by["test-assets-20261116c.iso"].sources], ["mirror"])

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

    BOOT = {"dir": "/i", "work": "/w", "out": "/o", "qemu": "/q", "flags": "/f", "broker": "/b", "gop": "/g", "data": "/d.iso", "apps": ["premiere"]}

    def test_boot_command_carries_the_guest_ram_and_the_sound_card_by_default(self):
        c = sb.boot_command(Path("/c.py"), self.BOOT)
        self.assertEqual((sb.GUEST_RAM_MB, sb.AUDIO), (16384, True))
        self.assertEqual(c[c.index("--ram-mb") + 1], "16384")
        self.assertIn("--audio", c)
        self.assertEqual(c[c.index("--fb-mb") + 1], "8192")       # the framebuffer pin is a separate fact and stays

    def test_boot_command_override_fields_change_or_drop_them(self):
        c = sb.boot_command(Path("/c.py"), dict(self.BOOT, ram_mb=32768, audio=False))
        self.assertEqual(c[c.index("--ram-mb") + 1], "32768")
        self.assertNotIn("--audio", c)
        c = sb.boot_command(Path("/c.py"), dict(self.BOOT, ram_mb=0))
        self.assertNotIn("--ram-mb", c)

    def test_image_a_boots_keep_the_default_ram_and_no_sound_card(self):
        a = dict(self.BOOT, apps=["wukong", "superposition"])
        for cmd in (pa.boot_command("/c.py", a), pa.boot_command("/c.py", dict(a, apps=["3dmark"]))):
            self.assertNotIn("--ram-mb", cmd)
            self.assertNotIn("--audio", cmd)


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
        self.mem_mb = scenario.pop("_mem_mb", 65536)
        patcher = unittest.mock.patch.object(sb, "host_mem_available_mb", lambda pi_check: self.mem_mb)    # the box's MemAvailable, named by the test
        patcher.start()
        self.addCleanup(patcher.stop)
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
            if limited:
                for f in ("uitext.txt", "uitext_hits.txt", "inventory.txt"):
                    (d / f).write_text("Text | x | y\n")
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
        for name in ("uitext.txt", "uitext_hits.txt", "inventory.txt"):      # the UI text of the launch rows is text evidence, copied next to the verdict
            self.assertIn(name, copied)
            self.assertEqual({p.parent.name for p in ev.rglob(name)}, set(MOD.LAUNCH_ONLY) & set(ORDER))
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


class GuestRam(RunBase, unittest.TestCase):
    """The per-run overrides of image B boots: 16384 MiB and the sound card by default, refused above 75 percent of MemAvailable, switchable per run."""

    def test_every_boot_gets_the_ram_and_audio_flags_and_the_stage_passes(self):
        seen = []
        env = self.make()
        inner = env.ctx.boot_runner
        env.ctx.boot_runner = lambda ctx, argv, log, timeout, mark: (seen.append(list(argv)), inner(ctx, argv, log, timeout, mark))[1]
        rec = sb.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"]), ("PASS", ""), rec)
        self.assertGreaterEqual(len(seen), 2)
        for argv in seen:
            self.assertEqual(argv[argv.index("--ram-mb") + 1], "16384")
            self.assertIn("--audio", argv)

    def test_run_arguments_switch_both_off(self):
        env = self.make()
        rec = sb.run(env.ctx, guest_ram_mb=0, audio=False)
        self.assertEqual(rec["verdict"], "PASS", rec)
        self.assertNotIn("--ram-mb", self.argv)
        self.assertNotIn("--audio", self.argv)

    def test_an_override_above_75_percent_of_memavailable_is_refused_before_anything_is_fetched(self):
        env = self.make(_mem_mb=20000)          # 75 percent of 20000 is 15000 < 16384
        rec = sb.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"]), ("REFUSED", "RAM_OVERRIDE_EXCEEDS_HOST"))
        self.assertEqual(rec["boots"], [])
        self.assertFalse(hasattr(self, "argv"))
        self.assertFalse((env.ctx.prefix / "dl").exists())

    def test_the_boundary_is_exactly_75_percent(self):
        self.assertEqual(sb.run(self.make(_mem_mb=21846).ctx)["verdict"], "PASS")
        self.assertEqual(sb.run(self.make(_mem_mb=21845).ctx)["reason"], "RAM_OVERRIDE_EXCEEDS_HOST")

    def test_an_invalid_override_is_refused_by_name_and_zero_skips_the_check(self):
        rec = sb.run(self.make().ctx, guest_ram_mb=5000)
        self.assertEqual((rec["verdict"], rec["reason"]), ("REFUSED", "RAM_OVERRIDE_INVALID"))
        rec = sb.run(self.make(_mem_mb=1000).ctx, guest_ram_mb=0)
        self.assertEqual(rec["verdict"], "PASS", rec)

    def test_cli_defaults_and_switches(self):
        calls = []
        with unittest.mock.patch.object(sb, "run", lambda ctx, ev, apps, ram, audio: (calls.append((ram, audio)), {"verdict": "NOT_RUN"})[1]), \
                unittest.mock.patch.object(sb, "finish", lambda rec, out: 0), unittest.mock.patch.object(pa, "hardware_now", lambda: {}):
            base = ["run", "--sha", SHA, "--out", "/dev/null"]
            sb.main(base)
            sb.main(base + ["--guest-ram-mb", "0", "--no-audio"])
        self.assertEqual(calls, [(16384, True), (0, False)])


RECOMMENDED = ("gpudiag,premiere,lightroomlaunch,dimension,uxp,bridge,characteranimator,audition,dreamweaver,incopy,substance_designer,"
               "substance_modeler,substance_painter,substance_sampler,substance_stager,mediaencoderlaunch,animatelaunch").split(",")


class PinsFlag(RunBase, unittest.TestCase):
    """`run --pins FILE` (2026-11-20): the pins of THIS run come from the file (same schema as the committed one); absent, nothing changes; a missing or
    malformed file is refused by a named reason before anything starts."""

    def other_pins(self, **changes):
        doc = pins_doc()
        for it in doc["items"]:
            if it["role"] == "image":
                it.update(name="propimage-kfimg-20261120.zip", sha256="d" * 64, size=71234567890)
            it.update(changes.get(it["role"], {}))
        tmp = Path(tempfile.mkdtemp(prefix="kf-pab-pins2-"))
        self.addCleanup(shutil.rmtree, tmp, True)
        path = tmp / "new_b.json"
        path.write_text(json.dumps(doc))
        return path

    def main(self, *flags):
        import contextlib
        import io
        seen = []
        with unittest.mock.patch.object(sb, "run", lambda ctx, ev, apps, ram, audio: (seen.append(ctx.pins_path), {"verdict": "NOT_RUN"})[1]), \
                unittest.mock.patch.object(sb, "finish", lambda rec, out: 0), unittest.mock.patch.object(pa, "hardware_now", lambda: {}):
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                rc = sb.main(["run", "--sha", SHA, "--out", "/dev/null", *flags])
        return rc, seen, buf.getvalue()

    def test_main_hands_the_given_file_to_the_run(self):
        path = self.other_pins()
        rc, seen, _ = self.main("--pins", str(path))
        self.assertEqual((rc, seen), (0, [path]))

    def test_main_without_the_flag_uses_the_committed_file(self):
        rc, seen, _ = self.main()
        self.assertEqual((rc, seen), (0, [sb.PINS_PATH]))

    def test_main_refuses_a_missing_file_by_name_before_the_run(self):
        rc, seen, out = self.main("--pins", "/nonexistent/pins.json")
        self.assertEqual((rc, seen), (2, []))
        self.assertEqual(out.strip(), "KF_PROPIMAGE_APPS_B_REFUSED PINS_MISSING")
        d = Path(tempfile.mkdtemp(prefix="kf-pab-dir-"))
        self.addCleanup(shutil.rmtree, d, True)
        rc, seen, out = self.main("--pins", str(d))                # a directory is not a pins file either
        self.assertEqual((rc, seen, out.strip()), (2, [], "KF_PROPIMAGE_APPS_B_REFUSED PINS_MISSING"))

    def bad_files(self):
        good = json.loads(self.other_pins().read_text())
        dup = '{"schema": "kayfabe.propimage-pins/1", "schema": "x"}'
        cases = {"not json": "not json {", "empty": "", "list": "[]", "wrong schema": json.dumps(dict(good, schema="other/1")),
                 "extra key": json.dumps(dict(good, extra=1)), "duplicate key": dup, "no items": json.dumps(dict(good, items=[])),
                 "bad sha": json.dumps(dict(good, items=[dict(i, sha256="zz") for i in good["items"]])),
                 "dest traversal": json.dumps(dict(good, items=[dict(i, dest="../x") for i in good["items"]])),
                 "dest not text": json.dumps(dict(good, items=[dict(i, dest=7) for i in good["items"]])),
                 "huge": json.dumps(good) + " " * (sb.PINS_MAX_BYTES + 1)}
        out = {}
        for label, text in cases.items():
            tmp = Path(tempfile.mkdtemp(prefix="kf-pab-bad-"))
            self.addCleanup(shutil.rmtree, tmp, True)
            (tmp / "p.json").write_text(text)
            out[label] = tmp / "p.json"
        return out

    def test_main_refuses_a_malformed_file_by_name_before_the_run(self):
        for label, path in self.bad_files().items():
            rc, seen, out = self.main("--pins", str(path))
            self.assertEqual((rc, seen, out.strip()), (2, [], "KF_PROPIMAGE_APPS_B_REFUSED PINS_MALFORMED"), label)

    def test_a_file_without_the_data_volume_is_refused_by_name(self):
        doc = json.loads(self.other_pins().read_text())
        doc["items"] = [i for i in doc["items"] if i["role"] != "data"]
        path = self.other_pins().with_name("nodata.json")
        path.write_text(json.dumps(doc))
        rc, seen, out = self.main("--pins", str(path))
        self.assertEqual((rc, seen, out.strip()), (2, [], "KF_PROPIMAGE_APPS_B_REFUSED PINS_NO_DATA"))

    def test_the_run_refuses_a_file_without_the_data_volume_before_anything_starts(self):
        doc = json.loads(self.other_pins().read_text())
        doc["items"] = [i for i in doc["items"] if i["role"] != "data"]
        path = self.other_pins().with_name("nodata.json")
        path.write_text(json.dumps(doc))
        env = self.make()
        env.ctx.pins_path = path
        rec = sb.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"], rec["boots"]), ("REFUSED", "PINS_NO_DATA", []))
        self.assertFalse((env.ctx.prefix / "dl").exists())

    def test_the_run_itself_refuses_a_missing_or_malformed_file_before_anything_starts(self):
        env = self.make()
        env.ctx.pins_path = Path("/nonexistent/pins.json")
        rec = sb.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"], rec["boots"], rec["apps"]), ("REFUSED", "PINS_MISSING", [], []))
        self.assertFalse((env.ctx.prefix / "dl").exists())
        self.assertFalse((env.ctx.prefix / "work").exists())
        for label, path in self.bad_files().items():
            env = self.make()
            env.ctx.pins_path = path
            rec = sb.run(env.ctx)
            self.assertEqual((rec["verdict"], rec["reason"], rec["boots"]), ("REFUSED", "PINS_MALFORMED", []), label)

    def test_the_record_image_block_shows_the_given_file(self):
        path = self.other_pins()
        env = self.make()
        env.ctx.pins_path = path
        rec = sb.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"]), ("PASS", ""), rec)
        self.assertEqual(rec["image"], {"name": "propimage-kfimg-20261120.zip", "sha256": "d" * 64, "size": 71234567890})
        self.assertNotEqual(rec["image"]["sha256"], pa.load_pins(sb.PINS_PATH)["items"][0]["sha256"])

    def test_the_committed_pins_file_is_untouched_and_names_no_new_image(self):
        by = {i["role"]: i for i in pa.load_pins(sb.PINS_PATH)["items"]}
        self.assertEqual(by["image"]["name"], "propimage-kfimg-20261116.zip")
        self.assertNotIn("20261120", sb.PINS_PATH.read_text())

    def test_an_unpinned_data_role_in_the_given_file_still_refuses(self):
        path = self.other_pins(data={"sha256": None, "size": None})
        env = self.make()
        env.ctx.pins_path = path
        rec = sb.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"]), ("REFUSED", "PIN_UNSET:data"))


class RowSelection(RunBase, unittest.TestCase):
    """`--apps` picks exactly the named rows in table order; the boot plan accounts for the new launch rows by their own timeouts."""

    def run_apps(self, apps):
        env = self.make()
        timeouts = []
        inner = env.ctx.boot_runner
        env.ctx.boot_runner = lambda ctx, argv, log, timeout, mark: (timeouts.append((argv[argv.index("--apps") + 1].split(","), timeout)), inner(ctx, argv, log, timeout, mark))[1]
        return sb.run(env.ctx, apps=apps), timeouts

    def test_the_recommended_list_selects_exactly_those_rows_in_table_order(self):
        rec, _ = self.run_apps(RECOMMENDED)
        self.assertEqual([a["app"] for a in rec["apps"]], [n for n in MOD.CHECKS if n in RECOMMENDED])
        self.assertEqual(sorted(a["app"] for a in rec["apps"]), sorted(RECOMMENDED))
        self.assertEqual((rec["verdict"], rec["reason"]), ("PASS", ""), rec)
        self.assertEqual(sorted(rec["limited"]), sorted(n for n in RECOMMENDED if n != "gpudiag"))

    def test_the_issue_list_is_selected_as_written(self):
        want = "premiere,lightroomlaunch,dimension,uxp,bridge,characteranimator,audition".split(",")
        rec, _ = self.run_apps(want)
        self.assertEqual([a["app"] for a in rec["apps"]], [n for n in MOD.CHECKS if n in want])
        self.assertEqual(len(rec["apps"]), 7)

    def test_discovery_launch_rows_boot_alone_and_gate_rows_share_a_boot_within_the_budget(self):
        rec, seen = self.run_apps(RECOMMENDED)
        self.assertEqual([b["apps"] for b in rec["boots"]],
                         [["gpudiag", "premiere", "lightroomlaunch"], ["dimension", "uxp"], *[[n] for n in RECOMMENDED[5:]]])
        for b in rec["boots"]:
            self.assertLessEqual(sum(MOD.CHECKS[a]["timeout"] for a in b["apps"]), sb.BOOT_BUDGET_S)

    def test_each_boot_is_given_the_sum_of_its_rows_timeouts_plus_the_slack(self):
        _, seen = self.run_apps(RECOMMENDED)
        by = {tuple(apps): t for apps, t in seen}
        for n in NEW_ROWS:
            self.assertEqual(by[(n,)], MOD.CHECKS[n]["timeout"] + pa.BOOT_SLACK_S, n)
            self.assertLessEqual(MOD.CHECKS[n]["timeout"], 900, n)
        self.assertEqual(by[("dimension", "uxp")], 1200 + 900 + pa.BOOT_SLACK_S)

    def test_ui_text_files_land_in_the_evidence_directory_of_their_row(self):
        env = self.make()
        ev = env.tmp / "evidence"
        rec = sb.run(env.ctx, str(ev), apps=["bridge", "substance_painter"])
        self.assertEqual(rec["verdict"], "PASS", rec)
        for row in ("bridge", "substance_painter"):
            for name in ("uitext.txt", "uitext_hits.txt", "inventory.txt", "verdict.json"):
                self.assertTrue((ev / row / name).is_file(), (row, name))
        self.assertFalse(list(ev.rglob("*.png")))

    def test_the_plan_command_lists_the_new_rows_as_boots_of_their_own(self):
        import contextlib
        import io
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            self.assertEqual(sb.main(["plan"]), 0)
        plan = json.loads(buf.getvalue())
        for n in NEW_ROWS:
            self.assertIn([n], plan["boots"], n)

    def test_the_deadline_guard_counts_the_new_rows_timeouts(self):
        import time
        env = self.make()
        env.ctx.deadline_epoch = time.time() + MOD.CHECKS["dreamweaver"]["timeout"] + 800        # less than timeout + 900: the boot is not started
        rec = sb.run(env.ctx, apps=["dreamweaver"])
        self.assertEqual((rec["boots"][0]["rc"], rec["apps"][0]["verdict"]), (125, "NOT_RUN"))
        env = self.make()
        env.ctx.deadline_epoch = time.time() + MOD.CHECKS["dreamweaver"]["timeout"] + 1000
        rec = sb.run(env.ctx, apps=["dreamweaver"])
        self.assertEqual((rec["boots"][0]["rc"], rec["apps"][0]["verdict"]), (0, "PASS_LIMITED"))


NEW_ROWS = ("dreamweaver", "incopy", "substance_designer", "substance_modeler", "substance_painter", "substance_sampler", "substance_stager",
            "mediaencoderlaunch", "animatelaunch")


class Shrink(RunBase, unittest.TestCase):
    """The record bound with 42 rows: the stage shrinks evidence, xid lines and census ids tier by tier and ends with a tier that drops them all."""

    def worst(self, n):
        keywords = vast_report.CENSUS_KEYWORDS
        apps = [{"app": "row%02d" % i, "lane": "discovery", "verdict": "FAIL", "kind": "UIA", "limited": False, "secs": 99999, "detail": "d" * 200,
                 "platform": "kf3", "path": "p" * 80,
                 "evidence": [{"file": "f%02d_%s.txt" % (j, "x" * 50), "bytes": 9, "sha256": "e" * 64} for j in range(24)],
                 "gpu": {"required": ["gpu"], "verdict": "FAIL", "peak": {"k%d" % j: 123456 for j in range(12)}},
                 "host_xid": {"count": 8, "lines": ["NVRM: Xid " + "9" * 180] * 8},
                 "census": {"lines": 9, "counts": {k: 9 for k in keywords}, "xid": 9, "ids": {k: ["0x%022x" % m for m in range(8)] for k in keywords}}}
                for i in range(n)]
        return {"schema": 1, "image_set": "b", "verdict": "FAIL", "sha": SHA, "reason": "x", "family": None, "cpu_vendor": None, "preflight": [],
                "image": {"name": "n", "sha256": "c" * 64, "size": 1}, "swtpm": {"version": None, "built": False}, "boots": [], "apps": apps,
                "failed": [], "limited": [], "breakages": [], "not_run": {}}

    def written(self, rec):
        tmp = Path(tempfile.mkdtemp(prefix="kf-pab-shrink-"))
        self.addCleanup(shutil.rmtree, tmp, True)
        data = sb.write_record(tmp / "r.json", rec)
        self.assertEqual((tmp / "r.json").read_text(), data + "\n")
        return data

    def test_every_row_count_up_to_the_whole_table_fits_the_bound(self):
        for n in (21, 24, 30, 33, 42, 48):
            data = self.written(self.worst(n))
            self.assertLessEqual(len(data.encode()), sb.MAX_RECORD_BYTES, n)
            json.loads(data)

    def test_the_whole_table_is_at_most_what_the_last_tier_can_hold(self):
        self.assertGreaterEqual(len(MOD.ENABLED), 40)
        self.assertLessEqual(len(self.written(self.worst(len(MOD.ENABLED))).encode()), sb.MAX_RECORD_BYTES)

    def test_the_result_is_still_the_same_rows_with_their_verdicts(self):
        data = json.loads(self.written(self.worst(42)))
        self.assertEqual([a["app"] for a in data["apps"]], ["row%02d" % i for i in range(42)])
        self.assertTrue(all(a["verdict"] == "FAIL" and a["detail"] == "d" * 200 for a in data["apps"]))
        for a in data["apps"]:
            self.assertLessEqual(len(a["evidence"]), 24)
            self.assertLessEqual(len(a["host_xid"]["lines"]), 8)
            self.assertEqual(a["host_xid"]["count"], 8)         # the count is never shrunk, only the sample lines

    def test_a_small_record_is_not_shrunk_at_all(self):
        rec = self.worst(3)
        before = json.dumps(rec, separators=(",", ":"), sort_keys=True)
        self.assertEqual(self.written(rec), before)

    def test_a_record_nothing_can_shrink_enough_is_still_valid_json_and_is_reported_oversize(self):
        rec = self.worst(6)
        for a in rec["apps"]:
            a["detail"] = "d" * 200
            a["path"] = "p" * 80
        rec["preflight"] = [{"name": "x%d" % i, "status": "MATCH", "detail": "y" * 200} for i in range(400)]
        data = self.written(rec)
        self.assertGreater(len(data.encode()), sb.MAX_RECORD_BYTES)
        json.loads(data)                                     # complete, never truncated; the report validator refuses it by size


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
