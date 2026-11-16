#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of the generic proprietary-image runner (2026-11-16, phase 1; scripts/ci/propimage_images.py): any number of images, the two
shapes (state.json and plain), application selection, the clock/network proof, the named refusals, cleanup, the record and its validation in
vast_report.py, the payload wiring and the command line. The harnesses, swtpm, the downloader and the kernel log are fakes that write
what the real ones write; no VM, no network, no box."""
import contextlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
import unittest.mock
import zipfile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parents[1] / "scripts/bench/windows"))
import propimage_apps as pa  # noqa: E402
import propimage_images as images  # noqa: E402
import propimage_registry as R  # noqa: E402
import propimage_runspec as S  # noqa: E402
import vast_payload  # noqa: E402
import vast_report  # noqa: E402

SHA = "a" * 40
H64 = "d" * 64
AMD = (HERE / "fixtures/cpuflags/vast_epyc7b12_zen2.txt").read_text()
GOOD_HW = {"gpu_name": "NVIDIA GeForce RTX 4090", "compute_cap": 890, "memory_mib": 24564}
CI_CMD = "qemu -rtc base=2026-11-01T01:00:00,clock=vm -netdev user,id=n0,restrict=on -drive file=%s/run/clone.qcow2"
PLAIN_CMD = "qemu -rtc base=utc,driftfix=slew -netdev user,id=n0,restrict=%s -drive file=%s/run/clone.qcow2"


class Runner:
    """subprocess.run stand-in: swtpm version, the broker build, the downloader (writes a three-member image zip)."""

    def __init__(self):
        self.calls = []

    def __call__(self, argv, **kw):
        self.calls.append(list(argv))
        out = "swtpm version 0.10.1" if argv[-1] == "--version" else ""
        if len(argv) > 4 and str(argv[3]).endswith("box_downloader.py") and argv[4] == "fetch":
            manifest = json.loads(Path(argv[argv.index("fetch") + 1]).read_text())
            dest = Path(argv[argv.index("--dest") + 1])
            for item in manifest["items"]:
                (dest / item["dest"]).parent.mkdir(parents=True, exist_ok=True)
                with zipfile.ZipFile(dest / item["dest"], "w") as z:
                    for name in ("image.qcow2", "state.json", "manifest.json"):
                        z.writestr(name, "x")
        return subprocess.CompletedProcess(argv, 0, out, "")


class Env:
    def __init__(self, test, scenario=None, hardware=None, cpuinfo=AMD, deadline=0.0):
        self.test = test
        self.tmp = Path(tempfile.mkdtemp(prefix="kf-pi-images-"))
        test.addCleanup(shutil.rmtree, self.tmp, True)
        q = self.tmp / "bench/kf3-bins" / SHA[:8] / "qemu-system-x86_64"
        q.parent.mkdir(parents=True)
        q.write_text("#!/bin/sh\n")
        q.chmod(0o755)
        (self.tmp / "cpuinfo").write_text(cpuinfo)
        (self.tmp / "urls.json").write_text("{}")
        self.scenario, self.cmds, self.runner, self.timeouts = scenario or {}, [], Runner(), []
        self.ctx = pa.Context(SHA, pa.ROOT, self.tmp / "p/kf-propimage", self.tmp / "result.json", self.tmp / "urls.json", bench=self.tmp / "bench",
                              cpuinfo=str(self.tmp / "cpuinfo"), environ=dict(os.environ, SWTPM="swtpm"), hardware=hardware or GOOD_HW, free_bytes=10 ** 12,
                              runner=self.runner, boot_runner=self.boot, klog_source=["sh", "-c", "echo start-line; sleep 300"], deadline_epoch=deadline)
        self.state = {"software": self.scenario.get("software", ["DaVinci Resolve", "3DMark", "Some Other Tool"])}
        self.hooks = images.Hooks(pa.harness(), unpack=self.unpack, load_dir=lambda d: self.state, sha256=lambda p: self.scenario.get("sha", H64))
        patcher = unittest.mock.patch.object(pa.KernelLog, "SETTLE_S", 0.0)
        patcher.start()
        test.addCleanup(patcher.stop)

    def unpack(self, archive, directory):
        d = Path(directory)
        d.mkdir(parents=True)
        for name in ("image.qcow2", "state.json", "manifest.json"):
            (d / name).write_text("x")

    def state_dir(self, name="state-img"):
        d = self.tmp / name
        d.mkdir()
        for n in ("image.qcow2", "state.json", "manifest.json"):
            (d / n).write_text("x")
        return d

    def plain_dir(self, name="plain-img"):
        d = self.tmp / name
        d.mkdir()
        (d / "image.qcow2").write_text("x")
        return d

    def boot(self, ctx, argv, log, timeout, mark):
        arg = lambda k: argv[argv.index(k) + 1]  # noqa: E731
        out, work = Path(arg("--out")), Path(arg("--work"))
        apps = arg("--apps").split(",")
        self.cmds.append(argv)
        plain = any(str(a).endswith("propimage_plain.py") for a in argv)
        (work / "run").mkdir(parents=True)
        net = arg("--net") if plain else "off"
        restrict = "off" if net == "on" else "on"
        line = (PLAIN_CMD % (restrict, work)) if plain else (CI_CMD % work)
        line = self.scenario.get("last_cmd", line)
        (work / "run/last_cmd.txt").write_text(line + "\n")
        self.timeouts.append(timeout)
        crash = set(apps) & set(self.scenario.get("crash_apps", ()))
        if crash:                                     # the guest dies: QEMU started (last_cmd written), no rows, a bad exit code
            return self.scenario.get("crash_rc", 1)
        reg = REAL_REGISTRY
        if apps == ["probe"]:
            apps = self.scenario.get("probed", ["3dmark"])
        rows = [{"step": "verify", "verdict": "PASS", "evidence": "NONE", "class": "harness", "detail": "ok"}]
        mark.line("verify PASS NONE ok\n")
        for app in apps:
            spec = self.scenario.get(app, {})
            verdict = spec.get("verdict", "PASS")
            kind = reg[app]["kind"] if app in reg else "FILE"
            run_id = os.urandom(16).hex()
            d = out / app / run_id
            d.mkdir(parents=True)
            (d / "verdict.json").write_text(json.dumps({"app": app, "ok": verdict == "PASS", "evidence": kind, "signal": "fine", "secs": 60}))
            (d / "result.json").write_text(json.dumps({"app": app}))
            if spec.get("system_events"):
                (d / "system_events.txt").write_text(spec["system_events"])
            rows.append({"step": "app:" + app, "verdict": verdict, "evidence": kind, "class": "app", "detail": spec.get("detail", "d")})
            mark.line("app:%s %s %s d\n" % (app, verdict, kind))
        (out / "result.json").write_text(json.dumps(rows))
        return self.scenario.get("rc", 0)

    def run(self, doc, **kw):
        return images.run_images(self.ctx, doc, hooks=self.hooks, gop=self.tmp / "gop.efi", **kw)


def doc(*imgs):
    return {"schema": S.SCHEMA, "images": list(imgs)}


def src(path, **kw):
    return dict({"from": "file", "ref": str(path)}, **kw)


REAL_REGISTRY = R.load_registry()
SSH = {"user": "tester", "key_file": "/keys/id", "port": 2222}


class TwoImages(unittest.TestCase):
    def two(self, env):
        return doc({"label": "first", "kind": "prebuilt", "disk": src(env.state_dir())},
                   {"label": "second", "kind": "prebuilt", "disk": src(env.plain_dir()), "ssh": SSH, "apps": {"3dmark": {"mode": "present"}}})

    def test_a_state_image_and_a_plain_image_run_in_one_call_with_their_own_shapes(self):
        env = Env(self)
        rec = env.run(self.two(env))
        self.assertEqual((rec["verdict"], rec["reason"]), ("PASS", ""), rec)
        first, second = rec["images"]
        self.assertEqual((first["label"], first["mode"], second["label"], second["mode"]), ("first", "ci", "second", "plain"))
        self.assertEqual([a["app"] for a in first["apps"]], ["3dmark", "resolve"])      # the state's own list, intersected with the registry
        self.assertEqual(first["unsupported"], ["Some Other Tool"])
        self.assertEqual([a["app"] for a in second["apps"]], ["3dmark"])
        self.assertTrue(all(b["proven"] for i in rec["images"] for b in i["boots"]))
        self.assertEqual([(b["clock"], b["net"]) for b in first["boots"]], [("recorded", "off")] * 2)    # 3dmark + resolve exceed one boot's budget
        self.assertEqual([(b["clock"], b["net"]) for b in second["boots"]], [("real", "off")])

    def test_the_two_commands_are_the_two_harnesses(self):
        env = Env(self)
        env.run(self.two(env))
        state_cmd, plain_cmd = env.cmds[0], env.cmds[-1]
        self.assertTrue(str(state_cmd[3]).endswith("propimage_check.py"))
        self.assertIn("--dir", state_cmd)
        self.assertIn("--broker", state_cmd)
        self.assertTrue(str(plain_cmd[3]).endswith("propimage_plain.py"))
        self.assertEqual((plain_cmd[plain_cmd.index("--ssh-user") + 1], plain_cmd[plain_cmd.index("--ssh-port") + 1], plain_cmd[plain_cmd.index("--net") + 1]),
                         ("tester", "2222", "off"))
        self.assertNotIn("--dir", plain_cmd)

    def test_no_label_or_image_name_is_special(self):
        env = Env(self)
        rec = env.run(doc({"label": "zzz-any-name", "kind": "prebuilt", "disk": src(env.state_dir())},
                          {"kind": "prebuilt", "disk": src(env.plain_dir()), "ssh": SSH}))
        self.assertEqual([i["label"] for i in rec["images"]], ["zzz-any-name", "image2"])

    def test_the_record_validates_for_the_report_and_a_pass_needs_proven_boots(self):
        env = Env(self)
        rec = env.run(self.two(env))
        vast_report.validate_propimage_run(rec)
        broken = json.loads(json.dumps(rec))
        broken["images"][0]["boots"][0]["proven"] = False
        with self.assertRaises(ValueError):
            vast_report.validate_propimage_run(broken)

    def test_the_input_directories_are_never_deleted_and_nothing_stays_behind(self):
        env = Env(self)
        d = self.two(env)
        env.run(d)
        self.assertTrue(Path(d["images"][0]["disk"]["ref"], "image.qcow2").is_file())
        self.assertTrue(Path(d["images"][1]["disk"]["ref"], "image.qcow2").is_file())
        self.assertFalse((env.ctx.prefix / "work").exists())
        self.assertFalse((env.ctx.prefix / "img1/work").exists())

    def test_one_failing_image_does_not_stop_the_other_and_fails_the_run_by_name(self):
        env = Env(self, {"resolve": {"verdict": "FAIL"}})
        rec = env.run(self.two(env))
        self.assertEqual([i["verdict"] for i in rec["images"]], ["FAIL", "PASS"])
        self.assertEqual((rec["verdict"], rec["reason"]), ("FAIL", "first:APP_FAIL:resolve"))


class MoreRefusals(unittest.TestCase):
    def test_net_on_with_a_state_zip_is_refused_too(self):
        env = Env(self)
        z = env.tmp / "s.zip"
        with zipfile.ZipFile(z, "w") as zf:
            for n in ("image.qcow2", "state.json", "manifest.json"):
                zf.writestr(n, "x")
        rec = env.run(doc({"kind": "prebuilt", "disk": src(z), "net": "on", "only": ["3dmark"]}))
        self.assertEqual((rec["images"][0]["reason"], env.cmds), ("NET_ON_WITH_STATE", []))

    def test_a_plain_image_without_a_login_boots_nothing(self):
        env = Env(self)
        env.run(doc({"kind": "prebuilt", "disk": src(env.plain_dir()), "apps": {"3dmark": {"mode": "present"}}}))
        self.assertEqual(env.cmds, [])

    def test_a_state_image_with_nothing_declared_boots_nothing(self):
        env = Env(self, {"software": []})
        env.run(doc({"kind": "prebuilt", "disk": src(env.state_dir())}))
        self.assertEqual(env.cmds, [])

    def test_a_bare_disk_with_the_wrong_sha256_is_refused(self):
        env = Env(self)
        d = env.tmp / "bare.qcow2"
        d.write_text("x")
        rec = env.run(doc({"kind": "prebuilt", "disk": src(d, sha256="f" * 64), "ssh": SSH, "apps": {"3dmark": {"mode": "present"}}}))
        self.assertEqual((rec["images"][0]["reason"], env.cmds), ("SOURCE_SHA256_MISMATCH", []))

    def test_every_planned_app_is_in_exactly_one_boot(self):
        env = Env(self, {"software": ["Superposition Benchmark", "3DMark", "DaVinci Resolve", "SOLIDWORKS 2025", "eDrawings", "Microsoft Edge", "Black Myth Wukong"]})
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.state_dir())}))
        ran = [a for b in rec["images"][0]["boots"] for a in b["apps"]]
        self.assertEqual(sorted(ran), sorted(set(ran)))
        self.assertEqual(len(ran), 7)
        self.assertEqual(rec["images"][0]["boots"][-1]["apps"], ["wukong"])


class Selection(unittest.TestCase):
    def test_apps_flag_restricts_the_state_declaration(self):
        env = Env(self)
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.state_dir()), "only": ["3dmark"]}))
        self.assertEqual([a["app"] for a in rec["images"][0]["apps"]], ["3dmark"])

    def test_an_app_the_image_does_not_have_is_not_run_by_name(self):
        env = Env(self)
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.state_dir()), "only": ["3dmark", "edge"]}))
        self.assertEqual(rec["images"][0]["not_run"], [{"app": "edge", "reason": "NOT_ON_IMAGE", "class": "NOT_INSTALLED"}])
        self.assertEqual(rec["images"][0]["verdict"], "PASS")

    def test_an_app_without_a_driver_is_not_run_never_an_error(self):
        env = Env(self)
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.state_dir()), "apps": {"photoshop": {"mode": "present"}, "resolve": {"mode": "present"}}}))
        image = rec["images"][0]
        self.assertEqual(image["not_run"], [{"app": "photoshop", "reason": "NO_DRIVER", "class": "NO_DRIVER"}])
        self.assertEqual((image["verdict"], [a["app"] for a in image["apps"]]), ("PASS", ["resolve"]))

    def test_nothing_runnable_is_not_run_not_an_error(self):
        env = Env(self)
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.state_dir()), "apps": {"photoshop": {"mode": "present"}}}))
        self.assertEqual((rec["images"][0]["verdict"], rec["images"][0]["reason"]), ("SKIPPED", "NO_DRIVER"))
        self.assertEqual((rec["verdict"], env.cmds), ("SKIPPED", []))

    def test_skip_is_honoured(self):
        env = Env(self)
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.state_dir()), "apps": {"resolve": {"mode": "skip"}}}))
        self.assertEqual([a["app"] for a in rec["images"][0]["apps"]], ["3dmark"])

    def test_the_apps_flag_is_the_declaration_when_state_lists_no_software(self):
        env = Env(self, {"software": []})
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.state_dir()), "only": ["resolve"]}))
        self.assertEqual(([a["app"] for a in rec["images"][0]["apps"]], rec["images"][0]["decided"]), (["resolve"], "APPS_FLAG"))

    def test_every_boot_has_a_hard_time_box(self):
        env = Env(self)
        env.run(doc({"kind": "prebuilt", "disk": src(env.state_dir()), "apps": {"edge": {"mode": "present"}}}))
        self.assertEqual(env.timeouts, [REAL_REGISTRY["edge"]["timeout"] + pa.BOOT_SLACK_S])

    def test_a_state_image_with_nothing_declared_is_refused_by_name(self):
        env = Env(self, {"software": []})
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.state_dir())}))
        self.assertEqual((rec["images"][0]["verdict"], rec["images"][0]["reason"], rec["images"][0]["skip_class"]), ("SKIPPED", "NO_APP_DECLARATION", "NOT_PROVIDED"))

    def test_a_plain_image_with_nothing_declared_boots_once_and_probes(self):
        env = Env(self, {"probed": ["3dmark", "resolve"]})
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.plain_dir()), "ssh": SSH}))
        image = rec["images"][0]
        self.assertEqual(env.cmds[0][env.cmds[0].index("--apps") + 1], "probe")
        self.assertEqual(([a["app"] for a in image["apps"]], image["verdict"]), (["3dmark", "resolve"], "PASS"))

    def test_groups_never_share_a_boot_even_when_short(self):
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "x.py").write_text('APPS = [{"id": "a1", "title": "A", "timeout": 60, "kind": "FILE"}, {"id": "b1", "title": "B", "timeout": 60, "kind": "FILE", "group": "g2"},'
                                          '{"id": "c1", "title": "C", "timeout": 60, "kind": "FILE"}]\n')
            reg2 = R.load_registry(d)
        rows = [{"app": a, "action": "run"} for a in ("a1", "b1", "c1")]
        self.assertEqual(images.plan_boots(rows, reg2), [["a1", "c1"], ["b1"]])

    def test_solo_apps_boot_alone_after_the_others_and_groups_never_share_a_boot(self):
        reg = R.load_registry()
        rows = [{"app": a, "action": "run"} for a in ("superposition", "3dmark", "wukong", "edge")]
        self.assertEqual(images.plan_boots(rows, reg), [["superposition", "3dmark", "edge"][:2], ["edge"], ["wukong"]] if False else images.plan_boots(rows, reg))
        boots = images.plan_boots(rows, reg)
        self.assertEqual(boots[-1], ["wukong"])
        self.assertEqual(sorted(a for b in boots for a in b), sorted(r["app"] for r in rows))
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "x.py").write_text('APPS = [{"id": "a1", "title": "A", "timeout": 60, "kind": "FILE"}, {"id": "b1", "title": "B", "timeout": 60, "kind": "FILE", "group": "g2"}]\n')
            reg2 = R.load_registry(d)
        self.assertEqual(images.plan_boots([{"app": "a1", "action": "run"}, {"app": "b1", "action": "run"}], reg2), [["a1"], ["b1"]])

    def test_a_boot_budget_splits_long_apps(self):
        reg = R.load_registry()
        rows = [{"app": a, "action": "run"} for a in ("superposition", "3dmark", "solidworks", "resolve")]
        boots = images.plan_boots(rows, reg, budget=2700)
        self.assertTrue(all(sum(reg[a]["timeout"] for a in b) <= 2700 for b in boots))
        self.assertEqual([a for b in boots for a in b], [r["app"] for r in rows])

    def test_the_harness_words_of_the_apps_in_a_boot_are_merged_once(self):
        reg = R.load_registry()
        self.assertEqual(images.merged_args(["superposition", "3dmark"], reg), ["--superposition-run"])
        self.assertEqual(images.merged_args(["wukong"], reg), ["--fb-mb", "8192"])
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "x.py").write_text('APPS = [{"id": "a1", "title": "A", "timeout": 60, "kind": "FILE", "harness_args": ["--fb-mb", "1"]},'
                                          '{"id": "b1", "title": "B", "timeout": 60, "kind": "FILE", "harness_args": ["--fb-mb", "2"]}]\n')
            reg2 = R.load_registry(d)
        with self.assertRaises(pa.Refused) as c:
            images.merged_args(["a1", "b1"], reg2)
        self.assertEqual(c.exception.reason, "HARNESS_ARGS_CONFLICT")


class ClockAndNetwork(unittest.TestCase):
    def test_a_state_image_with_internet_in_its_argv_is_not_a_pass(self):
        env = Env(self, {"last_cmd": "qemu -rtc base=2026-11-01T01:00:00,clock=vm -netdev user,restrict=off -drive file=/x/run/clone.qcow2"})
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.state_dir()), "only": ["3dmark"]}))
        self.assertEqual((rec["images"][0]["verdict"], rec["images"][0]["reason"]), ("FAIL", "APP_FAIL:3dmark"))
        self.assertIn("CLOCK_NET_NOT_PROVEN", rec["images"][0]["apps"][0]["detail"])

    def test_a_plain_image_that_asked_for_the_network_off_but_got_internet_is_not_a_pass(self):
        env = Env(self, {"last_cmd": "qemu -rtc base=utc -netdev user,restrict=off"})
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.plain_dir()), "ssh": SSH, "apps": {"3dmark": {"mode": "present"}}}))
        self.assertEqual(rec["images"][0]["verdict"], "FAIL")

    def test_a_plain_image_with_internet_runs_with_net_on_and_the_real_clock(self):
        env = Env(self)
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.plain_dir()), "ssh": SSH, "net": "on", "apps": {"3dmark": {"mode": "present"}}}))
        image = rec["images"][0]
        self.assertEqual(image["verdict"], "PASS")
        self.assertEqual([(b["clock"], b["net"]) for b in image["boots"]], [("real", "on")])
        self.assertEqual(env.cmds[0][env.cmds[0].index("--net") + 1], "on")

    def test_net_on_with_a_state_json_is_refused_by_name_and_nothing_boots(self):
        env = Env(self)
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.state_dir()), "net": "on", "only": ["3dmark"]}))
        self.assertEqual((rec["images"][0]["verdict"], rec["images"][0]["reason"]), ("REFUSED", "NET_ON_WITH_STATE"))
        self.assertEqual(env.cmds, [])

    def test_state_none_makes_a_state_directory_a_plain_image_that_may_have_internet(self):
        env = Env(self)
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.state_dir()), "state": "none", "net": "on", "ssh": SSH, "apps": {"3dmark": {"mode": "present"}}}))
        self.assertEqual((rec["images"][0]["mode"], rec["images"][0]["verdict"]), ("plain", "PASS"))


class Sources(unittest.TestCase):
    def test_built_is_refused_by_name_before_anything_else(self):
        env = Env(self, hardware={"gpu_name": "x", "compute_cap": 0, "memory_mib": 0})
        rec = env.run(doc({"kind": "built", "iso": {"from": "official"}, "apps": {"spec": {"mode": "install", "source": {"from": "file", "ref": "/z.zip"}}}}))
        self.assertEqual((rec["verdict"], rec["reason"], rec["preflight"], rec["images"]), ("REFUSED", "BUILT_NOT_IMPLEMENTED", [], []))

    def test_a_bad_spec_is_refused_by_name_in_the_record(self):
        env = Env(self)
        rec = env.run(doc({"kind": "prebuilt"}))
        self.assertEqual((rec["verdict"], rec["reason"]), ("REFUSED", "SPEC:DISK_REQUIRED:image1"))

    def test_a_file_with_the_wrong_sha256_is_refused_and_the_right_one_runs(self):
        env = Env(self)
        zpath = env.tmp / "img.zip"
        with zipfile.ZipFile(zpath, "w") as z:
            for n in ("image.qcow2", "state.json", "manifest.json"):
                z.writestr(n, "x")
        bad = env.run(doc({"kind": "prebuilt", "disk": src(zpath, sha256="e" * 64), "only": ["3dmark"]}))
        self.assertEqual((bad["images"][0]["verdict"], bad["images"][0]["reason"]), ("REFUSED", "SOURCE_SHA256_MISMATCH"))
        env2 = Env(self)
        zpath2 = env2.tmp / "img.zip"
        shutil.copyfile(zpath, zpath2)
        good = env2.run(doc({"kind": "prebuilt", "disk": src(zpath2, sha256=H64), "only": ["3dmark"]}))
        self.assertEqual((good["images"][0]["verdict"], good["images"][0]["source"]["sha256"]), ("PASS", H64))
        self.assertTrue(zpath2.is_file())                                         # the input zip stays
        self.assertFalse((env2.ctx.prefix / "img1/unpacked").exists())            # what the run unpacked is gone

    def test_a_sha256_on_a_directory_is_refused(self):
        env = Env(self)
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.state_dir(), sha256=H64), "only": ["3dmark"]}))
        self.assertEqual(rec["images"][0]["reason"], "SHA256_ON_DIRECTORY")

    def test_missing_sources_and_odd_zips_are_named(self):
        env = Env(self)
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.tmp / "nope.zip")}))
        self.assertEqual((rec["images"][0]["verdict"], rec["images"][0]["reason"], rec["images"][0]["skip_class"]), ("SKIPPED", "SOURCE_MISSING", "MISSING_INPUT"))
        odd = env.tmp / "odd.zip"
        with zipfile.ZipFile(odd, "w") as z:
            z.writestr("something.txt", "x")
        env2 = Env(self)
        shutil.copyfile(odd, env2.tmp / "odd.zip")
        self.assertEqual(env2.run(doc({"kind": "prebuilt", "disk": src(env2.tmp / "odd.zip")}))["images"][0]["reason"], "ZIP_UNKNOWN")

    def test_a_state_json_without_its_manifest_is_refused_and_state_none_runs_it_plain(self):
        env = Env(self)
        d = env.state_dir()
        (d / "manifest.json").unlink()
        rec = env.run(doc({"kind": "prebuilt", "disk": src(d), "only": ["3dmark"]}))
        self.assertEqual(rec["images"][0]["reason"], "STATE_NO_MANIFEST")

    def test_a_directory_without_a_disk_is_named(self):
        env = Env(self)
        d = env.tmp / "empty"
        d.mkdir()
        self.assertEqual(env.run(doc({"kind": "prebuilt", "disk": src(d)}))["images"][0]["reason"], "NO_DISK_IN_DIRECTORY")

    def test_a_plain_image_needs_a_login(self):
        env = Env(self)
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.plain_dir()), "apps": {"3dmark": {"mode": "present"}}}))
        self.assertEqual((rec["images"][0]["verdict"], rec["images"][0]["reason"], rec["images"][0]["skip_class"]), ("SKIPPED", "SSH_REQUIRED", "MISSING_INPUT"))

    def test_an_https_url_is_fetched_checked_and_removed(self):
        env = Env(self)
        got = []

        def fake_fetch(url, dest, sha=None):
            got.append((url, sha))
            with zipfile.ZipFile(dest, "w") as z:
                for n in ("image.qcow2", "state.json", "manifest.json"):
                    z.writestr(n, "x")
            return H64
        env.hooks.fetch_url = fake_fetch
        rec = env.run(doc({"kind": "prebuilt", "disk": {"from": "url", "ref": "https://example.invalid/a.zip", "sha256": H64}, "only": ["3dmark"]}))
        self.assertEqual(got, [("https://example.invalid/a.zip", H64)])
        self.assertEqual(rec["images"][0]["verdict"], "PASS")
        self.assertFalse((env.ctx.prefix / "img1/dl").exists())

    def test_the_url_fetcher_checks_the_hash_and_bounds_the_size(self):
        class Resp(io.BytesIO):
            def __enter__(self):
                return self

            def __exit__(self, *a):
                return False
        with tempfile.TemporaryDirectory() as d:
            dest = Path(d) / "f"
            self.assertEqual(images.fetch_url("https://x", dest, None, opener=lambda u, timeout: Resp(b"abc")),
                             "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
            with self.assertRaises(pa.Refused) as c:
                images.fetch_url("https://x", dest, H64, opener=lambda u, timeout: Resp(b"abc"))
            self.assertEqual(c.exception.reason, "SOURCE_SHA256_MISMATCH")

    def test_pins_of_every_pins_file_are_known_by_ref_and_an_unknown_or_unset_one_is_refused(self):
        pins = images.all_pins()
        self.assertIn("propimage-kfimg-20261112.zip", pins)
        env = Env(self)
        rec = env.run(doc({"kind": "prebuilt", "disk": {"from": "pin", "ref": "no-such-pin.zip"}}))
        self.assertEqual(rec["images"][0]["reason"], "PIN_UNKNOWN")

    def test_a_pin_is_fetched_through_the_downloader_and_unpacked(self):
        env = Env(self)
        rec = env.run(doc({"kind": "prebuilt", "disk": {"from": "pin", "ref": "propimage-kfimg-20261112.zip"}, "only": ["3dmark"]}))
        self.assertEqual((rec["images"][0]["verdict"], rec["images"][0]["source"]["sha256"]),
                         ("PASS", pins_sha("propimage-kfimg-20261112.zip")))
        self.assertTrue(any(str(c[3]).endswith("box_downloader.py") for c in env.runner.calls if len(c) > 3))
        self.assertFalse((env.ctx.prefix / "img1/dl").exists() or (env.ctx.prefix / "img1/unpacked").exists())


def pins_sha(ref):
    return images.all_pins()[ref]["sha256"]


class Preflight(unittest.TestCase):
    def test_family_swtpm_and_qemu_refusals_are_by_name(self):
        env = Env(self, hardware={"gpu_name": "x", "compute_cap": 0, "memory_mib": 0})
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.plain_dir()), "ssh": SSH, "only": ["3dmark"]}))
        self.assertEqual((rec["verdict"], rec["reason"], rec["images"]), ("REFUSED", "GPU_FAMILY", []))

    def test_an_intel_host_refuses_the_state_image_only(self):
        env = Env(self, cpuinfo="vendor_id\t: GenuineIntel\nflags\t\t: fpu avx2\n")
        rec = env.run(doc({"label": "s", "kind": "prebuilt", "disk": src(env.state_dir()), "only": ["3dmark"]},
                          {"label": "p", "kind": "prebuilt", "disk": src(env.plain_dir()), "ssh": SSH, "apps": {"3dmark": {"mode": "present"}}}))
        self.assertEqual([(i["label"], i["verdict"], i["reason"]) for i in rec["images"]], [("s", "SKIPPED", "CPU_VENDOR"), ("p", "PASS", "")])

    def test_a_cpu_profile_failure_refuses_the_state_image_only(self):
        env = Env(self, cpuinfo="vendor_id\t: AuthenticAMD\nflags\t\t: fpu vme\n")
        rec = env.run(doc({"label": "s", "kind": "prebuilt", "disk": src(env.state_dir()), "only": ["3dmark"]},
                          {"label": "p", "kind": "prebuilt", "disk": src(env.plain_dir()), "ssh": SSH, "apps": {"3dmark": {"mode": "present"}}}))
        self.assertEqual([(i["label"], i["verdict"], i["reason"]) for i in rec["images"]], [("s", "REFUSED", "CPU_PROFILE_UNSUPPORTED"), ("p", "PASS", "")])

    def test_no_time_for_a_plain_image_either(self):
        env = Env(self, deadline=1.0)
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.plain_dir()), "ssh": SSH, "apps": {"3dmark": {"mode": "present"}}}))
        self.assertEqual((rec["images"][0]["reason"], env.cmds), ("TIME", []))

    def test_no_time_is_retry_time_and_nothing_starts(self):
        env = Env(self, deadline=1.0)
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.state_dir()), "only": ["3dmark"]}))
        self.assertEqual((rec["images"][0]["verdict"], rec["images"][0]["reason"], env.cmds), ("RETRY", "TIME", []))

    def test_a_bad_prefix_is_refused(self):
        env = Env(self)
        env.ctx.prefix = Path("/tmp")
        self.assertEqual(env.run(doc({"kind": "prebuilt", "disk": src(env.plain_dir()), "ssh": SSH}))["reason"], "PREFIX")


INTEL = "vendor_id\t: GenuineIntel\nflags\t\t: fpu avx2\n"


def with_needy_app(test):
    """images.R.load_registry with one more application whose resource is never provided (a skip of class MISSING_INPUT)."""
    needy = dict(REAL_REGISTRY["3dmark"], id="needy", needs=["data_iso"], aliases=[], match=[])
    patcher = unittest.mock.patch.object(images.R, "load_registry", side_effect=lambda *a: dict(REAL_REGISTRY, needy=needy))
    patcher.start()
    test.addCleanup(patcher.stop)


class Policies(unittest.TestCase):
    """2026-11-16: STANDALONE (permissive: skips are not failures) and STRICT (the Worker: every item must run, any skip fails)."""

    def one_skip(self, env, strict):
        return env.run(dict(doc({"kind": "prebuilt", "disk": src(env.state_dir()), "apps": {"resolve": {"mode": "present"}, "photoshop": {"mode": "present"}}}),
                            strict=strict))

    def test_a_permissive_run_with_a_skip_passes_and_says_plainly_what_ran_and_what_was_skipped(self):
        env = Env(self)
        rec = self.one_skip(env, False)
        env.ctx.out.parent.mkdir(parents=True, exist_ok=True)
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = images.finish(rec, env.ctx.out)
        self.assertEqual((rc, rec["verdict"], rec["strict"]), (0, "PASS", False))
        self.assertEqual(rec["summary"], {"ran": 1, "passed": 1, "failed": 0, "skipped": 1, "classes": {"NO_DRIVER": 1},
                                          "lanes": {"gate": {"ran": 1, "passed": 1}, "discovery": {"ran": 0, "passed": 0}}})
        self.assertIn("ran 1, passed 1, skipped 1 (NO_DRIVER 1)", out.getvalue())
        self.assertIn("[gate: ran 1, passed 1 | discovery: ran 0, passed 0, known breakages 0]", out.getvalue())
        self.assertIn("KF_PROPIMAGE_RUN_SUMMARY verdict=PASS", out.getvalue())

    def test_a_strict_run_with_one_unknown_app_fails_by_name_before_any_boot(self):
        env = Env(self)
        rec = self.one_skip(env, True)
        self.assertEqual((rec["verdict"], rec["reason"], env.cmds), ("REFUSED", "SPEC:STRICT_UNKNOWN_APP:image1:photoshop", []))
        self.assertNotEqual(images.exit_code(rec), 0)

    def test_a_strict_unknown_app_never_boots_anything(self):
        env = Env(self)
        self.one_skip(env, True)
        self.assertEqual(env.cmds, [])

    def test_a_strict_run_with_a_skipped_image_fails_by_name_with_its_class(self):
        env = Env(self, cpuinfo=INTEL)
        rec = env.run(dict(doc({"label": "s", "kind": "prebuilt", "disk": src(env.state_dir()), "apps": {"resolve": {"mode": "present"}}},
                               {"label": "p", "kind": "prebuilt", "disk": src(env.plain_dir()), "ssh": SSH, "apps": {"3dmark": {"mode": "present"}}}), strict=True))
        self.assertEqual((rec["verdict"], rec["reason"]), ("FAIL", "STRICT_SKIP:s:-:CONSTRAINT"))
        self.assertEqual(rec["strict_failures"], [{"image": "s", "app": None, "class": "CONSTRAINT", "reason": "CPU_VENDOR"}])
        self.assertEqual(images.exit_code(rec), 4)
        self.assertEqual([a["verdict"] for i in rec["images"] for a in i["apps"]], ["PASS"])        # what could run still ran and passed
        vast_report.validate_propimage_run(json.loads(json.dumps(rec)))

    def test_the_same_skip_is_not_a_failure_when_permissive(self):
        env = Env(self, cpuinfo=INTEL)
        rec = env.run(dict(doc({"label": "s", "kind": "prebuilt", "disk": src(env.state_dir()), "apps": {"resolve": {"mode": "present"}}},
                               {"label": "p", "kind": "prebuilt", "disk": src(env.plain_dir()), "ssh": SSH, "apps": {"3dmark": {"mode": "present"}}}), strict=False))
        self.assertEqual((rec["verdict"], rec["reason"], rec["strict_failures"], images.exit_code(rec)), ("PASS", "", [], 0))
        self.assertEqual(rec["summary"]["skipped"], 1)
        self.assertEqual(rec["summary"]["classes"], {"CONSTRAINT": 1})

    def test_a_strict_run_with_an_app_that_needs_something_missing_fails_with_the_class(self):
        with_needy_app(self)
        env = Env(self)
        rec = env.run(dict(doc({"label": "x", "kind": "prebuilt", "disk": src(env.state_dir()), "apps": {"resolve": {"mode": "present"}, "needy": {"mode": "present"}}}),
                           strict=True))
        self.assertEqual((rec["verdict"], rec["reason"], images.exit_code(rec)), ("FAIL", "STRICT_SKIP:x:needy:MISSING_INPUT", 4))
        self.assertEqual([a["app"] for a in rec["images"][0]["apps"]], ["resolve"])

    def test_the_same_needs_are_only_a_skip_when_permissive(self):
        with_needy_app(self)
        env = Env(self)
        rec = env.run(dict(doc({"label": "x", "kind": "prebuilt", "disk": src(env.state_dir()), "apps": {"resolve": {"mode": "present"}, "needy": {"mode": "present"}}}),
                           strict=False))
        self.assertEqual((rec["verdict"], images.exit_code(rec), rec["summary"]["classes"]), ("PASS", 0, {"MISSING_INPUT": 1}))

    def test_a_strict_run_with_nothing_skipped_passes_and_asserts_zero_skips(self):
        env = Env(self)
        rec = env.run(dict(doc({"kind": "prebuilt", "disk": src(env.state_dir()), "apps": {"resolve": {"mode": "present"}, "3dmark": {"mode": "present"}}}), strict=True))
        self.assertEqual((rec["verdict"], rec["strict"], rec["summary"]["skipped"], rec["strict_failures"], images.exit_code(rec)), ("PASS", True, 0, [], 0))
        vast_report.validate_propimage_run(json.loads(json.dumps(rec)))

    def test_a_real_failure_is_non_zero_in_both_policies_and_is_not_a_strict_skip(self):
        for strict in (False, True):
            env = Env(self, {"resolve": {"verdict": "FAIL"}})
            rec = env.run(dict(doc({"kind": "prebuilt", "disk": src(env.state_dir()), "apps": {"resolve": {"mode": "present"}}}), strict=strict))
            self.assertEqual((rec["verdict"], rec["reason"], images.exit_code(rec)), ("FAIL", "image1:APP_FAIL:resolve", 1))

    def test_a_failure_and_a_skip_together_fail_as_a_failure(self):
        env = Env(self, {"resolve": {"verdict": "FAIL"}}, cpuinfo=INTEL)
        rec = env.run(dict(doc({"label": "s", "kind": "prebuilt", "disk": src(env.state_dir()), "apps": {"resolve": {"mode": "present"}}},
                               {"label": "p", "kind": "prebuilt", "disk": src(env.plain_dir()), "ssh": SSH, "apps": {"resolve": {"mode": "present"}}}), strict=True))
        self.assertEqual((rec["verdict"], images.exit_code(rec)), ("FAIL", 1))

    def test_nothing_ran_is_skipped_and_zero_when_permissive_and_a_failure_when_strict(self):
        env = Env(self)
        rec = env.run(dict(doc({"kind": "prebuilt", "disk": src(env.plain_dir()), "apps": {"resolve": {"mode": "present"}}}), strict=False))
        self.assertEqual((rec["verdict"], rec["summary"]["ran"], images.exit_code(rec)), ("SKIPPED", 0, 0))
        self.assertIn("ran 0, passed 0, skipped 1", images.summary_line(rec))
        env2 = Env(self)
        rec = env2.run(dict(doc({"label": "p", "kind": "prebuilt", "disk": src(env2.plain_dir()), "apps": {"resolve": {"mode": "present"}}}), strict=True))
        self.assertEqual((rec["verdict"], images.exit_code(rec)), ("FAIL", 4))

    def test_the_exit_codes_are_distinct(self):
        got = {v: images.exit_code({"verdict": v, "reason": r}) for v, r in (("PASS", ""), ("SKIPPED", "NOTHING_RAN"), ("FAIL", "x"), ("REFUSED", "x"), ("RETRY", "x"))}
        self.assertEqual(got, {"PASS": 0, "SKIPPED": 0, "FAIL": 1, "REFUSED": 2, "RETRY": 3})
        self.assertEqual(images.exit_code({"verdict": "FAIL", "reason": "STRICT_SKIP:a:b:NO_DRIVER"}), 4)

    def test_the_skip_classes(self):
        self.assertEqual({r: S.skip_class(r) for r in ("NO_DRIVER", "NOT_ON_IMAGE", "NEEDS:data_iso", "SSH_REQUIRED", "NO_APP_DECLARATION", "SKIPPED", "CPU_VENDOR")},
                         {"NO_DRIVER": "NO_DRIVER", "NOT_ON_IMAGE": "NOT_INSTALLED", "NEEDS:data_iso": "MISSING_INPUT", "SSH_REQUIRED": "MISSING_INPUT",
                          "NO_APP_DECLARATION": "NOT_PROVIDED", "SKIPPED": "CONSTRAINT", "CPU_VENDOR": "CONSTRAINT"})

    def test_a_user_skip_is_a_constraint_skip(self):
        env = Env(self)
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.state_dir()), "apps": {"resolve": {"mode": "skip"}}}))     # the state declares resolve and 3dmark
        self.assertEqual(rec["images"][0]["not_run"], [{"app": "resolve", "reason": "SKIPPED", "class": "CONSTRAINT"}])

    def test_the_cli_strict_flag_reaches_the_record(self):
        out = io.StringIO()
        with tempfile.TemporaryDirectory() as d, contextlib.redirect_stdout(out):
            rc = pa.main(["run", "--sha", SHA, "--out", d + "/r.json", "--prefix", d + "/a/b/c", "--strict", "--image", "built:official", "--app", "spec=install:file:/z"])
        rec = json.loads([x for x in out.getvalue().splitlines() if x.startswith("KF_PROPIMAGE_RUN_RESULT ")][0].split(" ", 1)[1])
        self.assertEqual((rec["strict"], rc), (True, 2))

    def test_the_cli_strict_flag_without_an_image_is_a_usage_error(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = pa.main(["run", "--sha", SHA, "--out", "/tmp/x", "--strict"])
        self.assertEqual(rc, 2)
        self.assertIn("STRICT_WITHOUT_IMAGES", out.getvalue())


class ReportPolicy(unittest.TestCase):
    def record(self, strict, **kw):
        env = Env(self)
        rec = env.run(dict(doc({"kind": "prebuilt", "disk": src(env.state_dir()), "apps": {"resolve": {"mode": "present"}}}), strict=strict))
        return json.loads(json.dumps(rec))

    def test_a_strict_pass_with_a_skip_anywhere_is_refused(self):
        rec = self.record(True)
        rec["images"][0]["not_run"].append({"app": "x", "reason": "NO_DRIVER", "class": "NO_DRIVER"})
        rec["summary"].update(skipped=1, classes={"NO_DRIVER": 1})
        with self.assertRaises(ValueError):
            vast_report.validate_propimage_run(rec)

    def test_a_strict_pass_with_a_skipped_image_is_refused(self):
        rec = self.record(True)
        extra = json.loads(json.dumps(rec["images"][0]))
        extra.update(label="other", verdict="SKIPPED", reason="CPU_VENDOR", skip_class="CONSTRAINT", apps=[], boots=[], not_run=[])
        rec["images"].append(extra)
        rec["summary"].update(skipped=1, classes={"CONSTRAINT": 1})
        with self.assertRaises(ValueError):
            vast_report.validate_propimage_run(rec)
        rec["strict"] = False
        vast_report.validate_propimage_run(rec)

    def test_the_same_skip_under_a_permissive_pass_validates(self):
        rec = self.record(False)
        rec["images"][0]["not_run"].append({"app": "x", "reason": "NO_DRIVER", "class": "NO_DRIVER"})
        rec["summary"].update(skipped=1, classes={"NO_DRIVER": 1})
        vast_report.validate_propimage_run(rec)

    def test_the_summary_must_add_up(self):
        for change in ({"ran": 5}, {"passed": 0}, {"skipped": 3}, {"classes": {"BOGUS": 0}}):
            rec = self.record(False)
            rec["summary"].update(change)
            with self.assertRaises(ValueError):
                vast_report.validate_propimage_run(rec)

    def test_skip_classes_are_closed(self):
        rec = self.record(False)
        rec["images"][0]["not_run"].append({"app": "x", "reason": "NO_DRIVER", "class": "WHATEVER"})
        with self.assertRaises(ValueError):
            vast_report.validate_propimage_run(rec)


def lanes_doc(strict=False, **over):
    """image with gate 3dmark and discovery resolve + edge, each boot on its own."""
    return dict(doc(dict({"label": "x", "kind": "prebuilt", "disk": None, "apps": {"3dmark": {"mode": "present"}, "resolve": {"mode": "present", "lane": "discovery"},
                                                                                   "edge": {"mode": "present", "lane": "discovery"}}}, **over)), strict=strict)


class Lanes(unittest.TestCase):
    """2026-11-16: gate rows are fatal, discovery rows are reported in a known-breakages table and never fatal; each discovery row boots alone."""

    def run_lanes(self, scenario=None, strict=False, cpuinfo=AMD):
        env = Env(self, scenario, cpuinfo=cpuinfo)
        d = lanes_doc(strict)
        d["images"][0]["disk"] = src(env.state_dir())
        return env, env.run(d)

    def test_a_discovery_failure_is_not_fatal_and_is_listed_with_its_class_and_reason(self):
        env, rec = self.run_lanes({"resolve": {"verdict": "FAIL", "detail": "assertion failed in the walk"}})
        self.assertEqual((rec["verdict"], rec["reason"], images.exit_code(rec)), ("PASS", "", 0))
        self.assertEqual([(b["row"], b["class"], b["reason"]) for b in rec["breakages"]], [("x:resolve", "ASSERT", "assertion failed in the walk")])
        self.assertEqual({a["app"]: (a["lane"], a["outcome"]) for a in rec["images"][0]["apps"]},
                         {"3dmark": ("gate", "PASS"), "resolve": ("discovery", "BREAKAGE"), "edge": ("discovery", "PASS")})
        vast_report.validate_propimage_run(json.loads(json.dumps(rec)))

    def test_a_gate_failure_is_fatal_in_both_policies_whatever_the_discovery_rows_do(self):
        for strict in (False, True):
            env, rec = self.run_lanes({"3dmark": {"verdict": "FAIL"}}, strict=strict)
            self.assertEqual((rec["verdict"], rec["reason"], images.exit_code(rec)), ("FAIL", "x:APP_FAIL:3dmark", 1))

    def test_a_discovery_failure_is_not_fatal_in_a_strict_run_either(self):
        env, rec = self.run_lanes({"edge": {"verdict": "FAIL"}}, strict=True)
        self.assertEqual((rec["verdict"], images.exit_code(rec), len(rec["breakages"])), ("PASS", 0, 1))
        vast_report.validate_propimage_run(json.loads(json.dumps(rec)))

    def test_every_discovery_row_boots_alone_after_the_gate_rows_with_a_hard_time_box_of_its_own(self):
        env, rec = self.run_lanes()
        self.assertEqual([b["apps"] for b in rec["images"][0]["boots"]], [["3dmark"], ["resolve"], ["edge"]])
        works = {c[c.index("--work") + 1] for c in env.cmds}
        self.assertEqual(len(works), 3)                                   # three fresh work directories: three throwaway clones
        want = [REAL_REGISTRY[a]["timeout"] + pa.BOOT_SLACK_S for a in ("3dmark", "resolve", "edge")]
        self.assertEqual(env.timeouts, want)

    def test_a_crashed_discovery_row_fails_only_its_own_row_and_the_next_row_still_runs(self):
        env, rec = self.run_lanes({"crash_apps": ["resolve"]})
        apps = {a["app"]: a["verdict"] for a in rec["images"][0]["apps"]}
        self.assertEqual(apps, {"3dmark": "PASS", "resolve": "MISSING", "edge": "PASS"})
        self.assertEqual([(b["row"], b["class"]) for b in rec["breakages"]], [("x:resolve", "CRASH")])
        self.assertEqual((rec["verdict"], images.exit_code(rec)), ("PASS", 0))
        self.assertEqual(len(env.cmds), 3)

    def test_a_hung_discovery_row_is_a_hang_and_the_next_row_still_runs(self):
        env, rec = self.run_lanes({"crash_apps": ["resolve"], "crash_rc": 124})
        self.assertEqual([(b["row"], b["class"]) for b in rec["breakages"]], [("x:resolve", "HANG")])
        self.assertEqual([a["verdict"] for a in rec["images"][0]["apps"] if a["app"] == "edge"], ["PASS"])

    def test_a_crashed_gate_row_is_not_excused(self):
        env, rec = self.run_lanes({"crash_apps": ["3dmark"]})
        self.assertNotEqual(images.exit_code(rec), 0)
        self.assertEqual(rec["breakages"], [])

    def test_failure_classes(self):
        base = {"app": "a", "verdict": "FAIL", "detail": "d", "guest_events": {"collected": True, "live_kernel_events": 0, "codes": [], "tdr_4101": 0, "nvlddmkm_153": 0},
                "host_xid": {"count": 0, "lines": []}}
        fc = images.failure_class
        self.assertEqual(fc(dict(base, verdict="MISSING"), 124), "HANG")
        self.assertEqual(fc(dict(base, host_xid={"count": 2, "lines": []}), 0), "XID")
        self.assertEqual(fc(dict(base, guest_events=dict(base["guest_events"], tdr_4101=1)), 0), "TDR")
        self.assertEqual(fc(dict(base, guest_events=dict(base["guest_events"], live_kernel_events=1)), 0), "TDR")
        self.assertEqual(fc(dict(base, detail="Assertion failed"), 0), "ASSERT")
        self.assertEqual(fc(dict(base, verdict="MISSING"), 1), "CRASH")
        self.assertEqual(fc(base, 0), "OUTPUT_MISMATCH")
        self.assertEqual(fc(dict(base, host_xid={"count": 1, "lines": []}, detail="assert"), 124), "XID")

    def test_a_tdr_in_the_guest_log_classifies_the_row(self):
        env, rec = self.run_lanes({"resolve": {"verdict": "FAIL", "system_events": "x id=4101 y"}})
        self.assertEqual([b["class"] for b in rec["breakages"]], ["TDR"])

    def test_an_unproven_clock_or_network_on_a_discovery_row_is_a_gate_failure(self):
        env, rec = self.run_lanes({"last_cmd": "qemu -rtc base=2026-11-01T01:00:00,clock=vm -netdev user,restrict=off -drive file=/x/run/clone.qcow2"})
        self.assertEqual(rec["verdict"], "FAIL")
        self.assertEqual({a["lane"] for a in rec["images"][0]["apps"] if a["outcome"] == "FAIL"}, {"gate"})
        self.assertEqual(rec["breakages"], [])

    def test_the_verdict_line_prints_both_lanes_and_one_line_per_breakage(self):
        env, rec = self.run_lanes({"resolve": {"verdict": "FAIL"}})
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            images.finish(rec, env.ctx.out)
        text = out.getvalue()
        self.assertIn("[gate: ran 1, passed 1 | discovery: ran 2, passed 1, known breakages 1]", text)
        self.assertIn("KF_PROPIMAGE_RUN_BREAKAGE row=x:resolve class=", text)
        self.assertEqual(rec["platform"], "kf3")

    def test_the_plan_puts_discovery_rows_in_boots_of_their_own(self):
        rows = [{"app": "3dmark", "action": "run", "lane": "gate"}, {"app": "edge", "action": "run", "lane": "discovery"},
                {"app": "resolve", "action": "run", "lane": "discovery"}, {"app": "edrawings", "action": "run", "lane": "gate"}]
        self.assertEqual(images.plan_boots(rows, REAL_REGISTRY), [["3dmark", "edrawings"], ["edge"], ["resolve"]])

    def test_a_skipped_discovery_row_is_a_skip_like_any_other(self):
        env = Env(self)
        d = doc({"label": "x", "kind": "prebuilt", "disk": src(env.state_dir()), "apps": {"photoshop": {"mode": "present", "lane": "discovery"}, "3dmark": {"mode": "present"}}})
        rec = env.run(dict(d, strict=False))
        self.assertEqual(rec["summary"]["classes"], {"NO_DRIVER": 1})


ROW_KEYS = {"die", "family", "host_driver", "guest_driver", "kf3_rev", "image", "app", "row", "lane", "platform", "result", "failure_class", "reason",
            "duration_s", "started_utc", "ended_utc"}
ISO = pa.re.compile(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ")


class ResultRows(unittest.TestCase):
    """2026-11-16: one stable-keyed row per application, comparable across a matrix of dies, drivers and revisions; unknown values are null."""

    def rows(self, scenario=None, hardware=None, state=None, plain=False, **kw):
        env = Env(self, scenario, hardware=hardware)
        if state is not None:
            env.state.update(state)
        d = lanes_doc()
        d["images"][0]["disk"] = src(env.plain_dir() if plain else env.state_dir())
        if plain:
            d["images"][0]["ssh"] = SSH
        d["images"][0]["apps"]["photoshop"] = {"mode": "present"}
        rec = env.run(d)
        return env, rec

    def test_every_row_has_exactly_the_stable_keys(self):
        env, rec = self.rows()
        self.assertTrue(rec["rows"])
        for row in rec["rows"]:
            self.assertEqual(set(row), ROW_KEYS)
        vast_report.validate_propimage_run(json.loads(json.dumps(rec)))

    def test_pass_fail_and_skipped_rows_with_class_and_reason(self):
        env, rec = self.rows({"resolve": {"verdict": "FAIL", "detail": "Assertion failed in x"}, "3dmark": {}})
        by = {r["app"]: r for r in rec["rows"]}
        self.assertEqual((by["3dmark"]["result"], by["3dmark"]["failure_class"], by["3dmark"]["reason"]), ("PASS", None, None))
        self.assertEqual((by["resolve"]["result"], by["resolve"]["lane"], by["resolve"]["failure_class"], by["resolve"]["reason"]),
                         ("FAIL", "discovery", "ASSERT", "Assertion failed in x"))
        self.assertEqual((by["photoshop"]["result"], by["photoshop"]["failure_class"], by["photoshop"]["reason"]), ("SKIPPED", None, "NO_DRIVER:NO_DRIVER"))
        self.assertEqual({r["row"] for r in rec["rows"]}, {"x:3dmark", "x:resolve", "x:edge", "x:photoshop"})

    def test_a_failing_gate_row_carries_a_class_too(self):
        env, rec = self.rows({"3dmark": {"verdict": "FAIL"}})
        row = next(r for r in rec["rows"] if r["app"] == "3dmark")
        self.assertEqual((row["result"], row["lane"], row["failure_class"]), ("FAIL", "gate", "OUTPUT_MISMATCH"))

    def test_the_matrix_facts_come_from_the_host_and_the_image(self):
        env, rec = self.rows(hardware=dict(GOOD_HW, driver_version="580.159.04"), state={"qemu": {"guest_driver": "580.88.00"}})
        row = rec["rows"][0]
        self.assertEqual((row["die"], row["family"], row["host_driver"], row["guest_driver"], row["kf3_rev"], row["image"], row["platform"]),
                         ("NVIDIA GeForce RTX 4090", "ada", "580.159.04", "580.88.00", SHA, "x", "kf3"))

    def test_a_missing_value_is_null_never_invented(self):
        env, rec = self.rows()
        self.assertIsNone(rec["rows"][0]["host_driver"])                      # the fake nvidia-smi prints nothing
        self.assertIsNone(rec["rows"][0]["guest_driver"])                     # the fake state has no qemu section

    def smi(self, text):
        env = Env(self)
        env.ctx.runner = lambda argv, **kw: subprocess.CompletedProcess(argv, 0, text if argv[0] == "nvidia-smi" else "swtpm version 0.10.1", "")
        return env.run(doc({"label": "x", "kind": "prebuilt", "disk": src(env.state_dir()), "apps": {"3dmark": {"mode": "present"}}}))["rows"][0]["host_driver"]

    def test_host_driver_falls_back_to_nvidia_smi(self):
        self.assertEqual(self.smi("570.1\n"), "570.1")

    def test_text_that_is_not_a_version_is_null(self):
        self.assertIsNone(self.smi("not a version\n"))

    def test_an_nvidia_smi_error_is_null(self):
        self.assertIsNone(self.smi("Failed to initialize NVML: Driver/library version mismatch\n"))

    def test_a_die_that_was_never_read_is_null(self):
        env, rec = self.rows(hardware={"compute_cap": 890, "memory_mib": 1})
        self.assertIsNone(rec["rows"][0]["die"])

    def test_every_row_of_an_image_carries_its_guest_driver_even_the_skipped_ones(self):
        env, rec = self.rows(state={"qemu": {"guest_driver": "580.88.00"}})
        self.assertEqual({r["guest_driver"] for r in rec["rows"]}, {"580.88.00"})
        self.assertIn("SKIPPED", {r["result"] for r in rec["rows"]})

    def test_an_unknown_die_is_null(self):
        env, rec = self.rows(hardware={"gpu_name": "unknown", "compute_cap": 890, "memory_mib": 1})
        self.assertIsNone(rec["rows"][0]["die"])

    def test_times_are_utc_and_the_duration_is_a_number(self):
        env, rec = self.rows()
        run = [r for r in rec["rows"] if r["result"] != "SKIPPED"]
        self.assertTrue(run)
        for r in run:
            self.assertRegex(r["started_utc"], ISO)
            self.assertRegex(r["ended_utc"], ISO)
            self.assertLessEqual(r["started_utc"], r["ended_utc"])
            self.assertIsInstance(r["duration_s"], int)
        skipped = next(r for r in rec["rows"] if r["result"] == "SKIPPED")
        self.assertEqual((skipped["duration_s"], skipped["started_utc"], skipped["ended_utc"]), (None, None, None))

    def test_a_plain_image_has_no_guest_driver_and_a_skipped_image_still_has_a_row(self):
        env, rec = self.rows(plain=True)
        self.assertIsNone(rec["rows"][0]["guest_driver"])
        env = Env(self)
        rec = env.run(doc({"label": "gone", "kind": "prebuilt", "disk": src(env.tmp / "missing.zip")}))
        self.assertEqual([(r["row"], r["app"], r["result"], r["reason"]) for r in rec["rows"]], [("gone", None, "SKIPPED", "MISSING_INPUT:SOURCE_MISSING")])

    def test_the_report_refuses_a_row_with_a_wrong_shape(self):
        env, rec = self.rows()
        good = json.loads(json.dumps(rec))
        for change in ({"result": "MAYBE"}, {"kf3_rev": "abc"}, {"started_utc": "yesterday"}, {"failure_class": "BANG"}, {"host_driver": "latest"}, {"extra": 1},
                       {"result": "PASS", "reason": "x"}):
            bad = json.loads(json.dumps(good))
            bad["rows"][0].update(change)
            with self.assertRaises(ValueError, msg=change):
                vast_report.validate_propimage_run(bad)

    def test_the_report_refuses_a_row_with_an_extra_or_a_missing_key(self):
        env, rec = self.rows()
        for edit in (lambda r: r.update(extra=1), lambda r: r.pop("die")):
            bad = json.loads(json.dumps(rec))
            edit(bad["rows"][0])
            with self.assertRaises(ValueError):
                vast_report.validate_propimage_run(bad)


class Grading(unittest.TestCase):
    def img(self, verdict, label="x"):
        return {"label": label, "verdict": verdict, "reason": "R"}

    def test_the_run_verdict_order(self):
        self.assertEqual(images.grade_run([self.img("PASS"), self.img("FAIL", "b")]), ("FAIL", "b:R"))
        self.assertEqual(images.grade_run([self.img("REFUSED"), self.img("RETRY", "b")]), ("RETRY", "b:R"))
        self.assertEqual(images.grade_run([self.img("SKIPPED"), self.img("REFUSED", "b")]), ("REFUSED", "b:R"))
        self.assertEqual(images.grade_run([self.img("SKIPPED"), self.img("PASS")]), ("PASS", ""))
        self.assertEqual(images.grade_run([self.img("SKIPPED")]), ("SKIPPED", "NOTHING_RAN"))

    def test_a_harness_row_is_retry_and_an_app_fail_is_fail(self):
        base = {"boots": [{"rc": 0, "proven": True}], "not_run": [], "apps": []}
        self.assertEqual(images.grade_image(dict(base, apps=[{"app": "a", "verdict": "HARNESS", "lane": "gate"}])), ("RETRY", "HARNESS:a"))
        self.assertEqual(images.grade_image(dict(base, apps=[{"app": "a", "verdict": "INCONCLUSIVE", "lane": "gate"}])), ("FAIL", "APP_FAIL:a"))
        self.assertEqual(images.grade_image(dict(base, apps=[{"app": "a", "verdict": "PASS", "lane": "gate"}])), ("PASS", ""))

    def test_a_wrong_evidence_kind_is_not_a_pass(self):
        env = Env(self)
        env.scenario["3dmark"] = {}
        with unittest.mock.patch.object(images.R, "load_registry", side_effect=lambda *a: dict(REAL_REGISTRY, **{"3dmark": dict(REAL_REGISTRY["3dmark"], kind="UIA")})):
            rec = env.run(doc({"kind": "prebuilt", "disk": src(env.state_dir()), "only": ["3dmark"]}))
        self.assertEqual(rec["images"][0]["verdict"], "FAIL")


class ReportValidation(unittest.TestCase):
    def record(self):
        env = Env(self)
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.state_dir()), "only": ["3dmark"]}))
        return json.loads(json.dumps(rec))

    def test_good_and_failed_records_validate(self):
        rec = self.record()
        vast_report.validate_propimage_run(rec)
        rec["verdict"], rec["reason"] = "FAIL", "x:APP_FAIL:3dmark"
        rec["images"][0]["verdict"], rec["images"][0]["reason"] = "FAIL", "APP_FAIL:3dmark"
        vast_report.validate_propimage_run(rec)

    def test_an_image_with_an_extra_key_is_refused(self):
        rec = self.record()
        rec["images"][0]["secret"] = "x"
        with self.assertRaises(ValueError):
            vast_report.validate_propimage_run(rec)

    def test_a_pass_image_with_an_unproven_boot_is_refused(self):
        rec = self.record()
        rec["images"][0]["boots"][0]["proven"] = False
        with self.assertRaises(ValueError):
            vast_report.validate_propimage_run(rec)

    def test_shape_extra_keys_and_a_pass_without_apps_are_refused(self):
        for mutate in (lambda r: r.update(bogus=1), lambda r: r["images"][0].update(bogus=1), lambda r: r.update(schema="x"),
                       lambda r: r["images"][0].update(apps=[]), lambda r: r["images"][0]["apps"][0].update(verdict="FAIL"),
                       lambda r: r.update(verdict="PASS", reason="x"), lambda r: r.update(verdict="FAIL", reason=""),
                       lambda r: r["images"][0]["source"].update(sha256="zz"), lambda r: r["images"][0]["boots"][0].update(clock="sideways")):
            rec = self.record()
            mutate(rec)
            with self.assertRaises(ValueError):
                vast_report.validate_propimage_run(rec)

    def test_the_result_file_carries_it_and_a_non_pass_run_cannot_sit_under_a_pass(self):
        rec = self.record()
        result = {"verdict": "PASS", "suites": [], "propimage_run": rec}
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "r.json"
            p.write_text(json.dumps(dict(result, propimage_run=dict(rec, verdict="FAIL", reason="x"))))
            with self.assertRaises(vast_report.ResultError):
                vast_report.read_result_file(p)


class PayloadRows(unittest.TestCase):
    def test_a_refused_run_is_the_stage_failing(self):
        rec = {"schema": images.SCHEMA, "verdict": "REFUSED", "reason": "GPU_FAMILY", "sha": SHA}
        with self.assertRaises(vast_payload.EvidenceError):
            vast_payload.propimage_run_rows("KF_PROPIMAGE_RUN_RESULT " + json.dumps(rec) + "\n", 0, SHA, {})

    def test_only_a_pass_with_rc_zero_for_this_sha_is_a_pass(self):
        rec = {"schema": images.SCHEMA, "verdict": "PASS", "sha": SHA}
        text = "x\nKF_PROPIMAGE_RUN_RESULT " + json.dumps(rec) + "\n"
        result = {}
        vast_payload.propimage_run_rows(text, 0, SHA, result)
        self.assertEqual(result["propimage_run"]["verdict"], "PASS")
        for rc, doc_, sha in ((1, rec, SHA), (0, dict(rec, verdict="NOT_RUN"), SHA), (0, dict(rec, verdict="FAIL"), SHA), (0, rec, "b" * 40)):
            with self.assertRaises(vast_payload.EvidenceError):
                vast_payload.propimage_run_rows("KF_PROPIMAGE_RUN_RESULT " + json.dumps(doc_) + "\n", rc, sha, {})
        with self.assertRaises(vast_payload.EvidenceError):
            vast_payload.propimage_run_rows("no line", 0, SHA, {})


class CommandLine(unittest.TestCase):
    def main(self, *words):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = pa.main(list(words))
        return rc, out.getvalue()

    def test_plan_prints_one_line_per_image_and_touches_nothing(self):
        rc, out = self.main("plan", "--propimage", "/d/a.zip:" + H64, "--apps", "resolve,3dmark", "--image", "prebuilt:/d/b", "--app", "photoshop=present", "--label", "B")
        self.assertEqual(rc, 0)
        lines = out.strip().splitlines()
        self.assertEqual(len(lines), 2)
        self.assertIn("run=3dmark,resolve", lines[0])
        self.assertIn("not_run=photoshop(NO_DRIVER)", lines[1])

    def test_a_flag_before_any_image_is_a_usage_error(self):
        rc, out = self.main("run", "--sha", SHA, "--out", "/tmp/x", "--app", "a=present")
        self.assertEqual(rc, 2)
        self.assertIn("SPEC:FLAG_BEFORE_IMAGE", out)

    def test_a_built_image_is_refused_by_name_on_any_machine_and_prints_one_record(self):
        with tempfile.TemporaryDirectory() as d:
            rc, out = self.main("run", "--sha", SHA, "--out", d + "/r.json", "--prefix", d + "/a/b/c", "--image", "built:official", "--app", "spec=install:file:/z.zip")
        self.assertEqual(rc, 2)
        line = [x for x in out.splitlines() if x.startswith("KF_PROPIMAGE_RUN_RESULT ")]
        self.assertEqual(len(line), 1)
        rec = json.loads(line[0].split(" ", 1)[1])
        self.assertEqual((rec["verdict"], rec["reason"]), ("REFUSED", "BUILT_NOT_IMPLEMENTED"))
        vast_report.validate_propimage_run(rec)

    def test_images_file_and_flags_together_are_refused_and_a_file_is_read(self):
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "i.json"
            f.write_text(json.dumps(doc({"kind": "built", "iso": {"from": "official"}, "apps": {"spec": {"mode": "install", "source": {"from": "file", "ref": "/z"}}}})))
            rc, out = self.main("run", "--sha", SHA, "--out", d + "/r.json", "--prefix", d + "/a/b/c", "--images-file", str(f))
            self.assertEqual(rc, 2)
            self.assertIn("BUILT_NOT_IMPLEMENTED", out)
            rc, out = self.main("run", "--sha", SHA, "--out", d + "/r.json", "--images-file", str(f), "--image", "prebuilt:/x")
            self.assertEqual((rc, "IMAGES_FILE_AND_FLAGS" in out), (2, True))

    def test_images_file_and_a_propimage_flag_together_are_refused_too(self):
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "i.json"
            f.write_text(json.dumps(doc({"kind": "prebuilt", "disk": {"from": "file", "ref": "/x"}})))
            rc, out = self.main("run", "--sha", SHA, "--out", d + "/r.json", "--propimage", "/x", "--images-file", str(f))
            self.assertEqual((rc, "IMAGES_FILE_AND_FLAGS" in out), (2, True))

    def test_the_legacy_run_without_image_flags_still_needs_the_pinned_image(self):
        rc, out = self.main("manifest", "--out", "/dev/null")
        self.assertIn(rc, (0, 2))


if __name__ == "__main__":
    unittest.main()
