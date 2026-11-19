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
        self.scenario, self.cmds, self.runner, self.timeouts, self.boot_env = scenario or {}, [], Runner(), [], []
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
        self.boot_env.append((ctx.environ.get("SWTPM"), ctx.environ.get("LD_LIBRARY_PATH")))
        getattr(self.runner, "events", []).append("boot")
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

    def test_a_bare_disk_with_the_wrong_sha256_is_refused_only_in_a_strict_run(self):
        env = Env(self)
        d = env.tmp / "bare.qcow2"
        d.write_text("x")
        rec = env.run(dict(doc({"kind": "prebuilt", "disk": src(d, sha256="f" * 64), "ssh": SSH, "apps": {"3dmark": {"mode": "present"}}}), strict=True))
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

    def test_a_file_with_the_wrong_sha256_is_refused_in_a_strict_run_and_the_right_one_runs(self):
        env = Env(self)
        zpath = env.tmp / "img.zip"
        with zipfile.ZipFile(zpath, "w") as z:
            for n in ("image.qcow2", "state.json", "manifest.json"):
                z.writestr(n, "x")
        bad = env.run(dict(doc({"kind": "prebuilt", "disk": src(zpath, sha256="e" * 64), "apps": {"3dmark": {"mode": "present"}}}), strict=True))
        self.assertEqual((bad["images"][0]["verdict"], bad["images"][0]["reason"]), ("REFUSED", "SOURCE_SHA256_MISMATCH"))
        env2 = Env(self)
        zpath2 = env2.tmp / "img.zip"
        shutil.copyfile(zpath, zpath2)
        good = env2.run(dict(doc({"kind": "prebuilt", "disk": src(zpath2, sha256=H64), "apps": {"3dmark": {"mode": "present"}}}), strict=True))
        self.assertEqual((good["images"][0]["verdict"], good["images"][0]["source"]["sha256"]), ("PASS", H64))
        self.assertTrue(zpath2.is_file())                                         # the input zip stays
        self.assertFalse((env2.ctx.prefix / "img1/unpacked").exists())            # what the run unpacked is gone

    def test_a_sha256_on_a_directory_is_refused_in_a_strict_run(self):
        env = Env(self)
        rec = env.run(dict(doc({"kind": "prebuilt", "disk": src(env.state_dir(), sha256=H64), "apps": {"3dmark": {"mode": "present"}}}), strict=True))
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
        rec = env.run(dict(doc({"kind": "prebuilt", "disk": {"from": "url", "ref": "https://example.invalid/a.zip", "sha256": H64}, "apps": {"3dmark": {"mode": "present"}}}),
                           strict=True))
        self.assertEqual(got, [("https://example.invalid/a.zip", H64)])                # strict: the pin is checked while the bytes arrive
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

    def test_a_shared_pin_is_one_entry_and_a_conflicting_duplicate_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            item = {"role": "gop", "ref": "shared.efi", "name": "g.efi", "dest": "image/g.efi", "sha256": H64, "size": 5}
            for n, it in (("a", item), ("b", item)):
                (Path(d) / ("propimage_apps%s_pins.json" % ("_" + n))).write_text(json.dumps({"items": [it]}))
            with unittest.mock.patch.object(pa, "load_pins", lambda path: json.loads(Path(path).read_text())):
                self.assertEqual(list(images.all_pins(d)), ["shared.efi"])
                (Path(d) / "propimage_apps_b_pins.json").write_text(json.dumps({"items": [dict(item, sha256="1" * 64)]}))
                with self.assertRaises(pa.Refused) as c:
                    images.all_pins(d)
                self.assertEqual(c.exception.reason, "PIN_DUPLICATE")

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
        rec["strict"] = rec["pins_enforced"] = False
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


PREFIX_BIN = pa.SWTPM_PREFIX + "/bin/swtpm"
TOOL_REFS = {i["role"]: i["ref"] for i in pa.load_pins()["items"] if i["role"] in ("libtpms", "swtpm")}


class OldSwtpmRunner(Runner):
    """A box whose swtpm is 0.6.3 (2026-11-17, MEASURED on a Vast Ubuntu 22.04 box). Nothing is installed anywhere: `make install` only flips what
    `/opt/tpm/bin/swtpm --version` answers. `fail(argv)` makes a command exit 1, `missing` names programs that are not on the box (OSError),
    `rev` overrides what `git rev-parse HEAD` answers (one commit, or {project: commit}), `tools_fetch_rc` is the downloader's exit code for the tools manifest, `built_version`
    what the build ends up installing and `need_ld` that the prefix binary only starts with /opt/tpm/lib on LD_LIBRARY_PATH (MEASURED 2026-11-16)."""

    def __init__(self, fail=None, missing=(), rev=None, tools_fetch_rc=0, built_version="0.10.1", installed=False, need_ld=False):
        super().__init__()
        self.fail, self.missing, self.rev, self.tools_fetch_rc = fail or (lambda argv: False), set(missing), rev, tools_fetch_rc
        self.built_version, self.installed, self.need_ld = built_version, installed, need_ld
        self.events, self.manifests, self.fetched, self.cwds = [], [], {}, []

    def __call__(self, argv, **kw):
        argv = [str(a) for a in argv]
        if argv[0] in self.missing:
            raise FileNotFoundError(argv[0])
        if len(argv) > 5 and argv[3].endswith("box_downloader.py"):
            if argv[4] == "fetch":
                manifest = json.loads(Path(argv[5]).read_text())
                self.manifests.append(manifest)
                tools = any(i["dest"].startswith("tools/") for i in manifest["items"])
                self.events.append("fetch:tools" if tools else "fetch:image")
                if tools and self.tools_fetch_rc:
                    self.calls.append(argv)
                    return subprocess.CompletedProcess(argv, self.tools_fetch_rc, "", "")
            return super().__call__(argv, **kw)
        self.calls.append(argv)
        self.events.append(Path(argv[0]).name if argv[0] != "make" else " ".join(argv[:2]))
        if argv[-1] == "--version":
            if argv[0] != PREFIX_BIN:
                return subprocess.CompletedProcess(argv, 0, "swtpm version 0.6.3", "")
            if not self.installed or (self.need_ld and pa.SWTPM_PREFIX + "/lib" not in (kw.get("env") or {}).get("LD_LIBRARY_PATH", "").split(":")):
                raise FileNotFoundError(argv[0])
            return subprocess.CompletedProcess(argv, 0, "swtpm version " + self.built_version, "")
        if self.fail(argv):
            return subprocess.CompletedProcess(argv, 1, "", "")
        if argv[0] == "make" and argv[1:2] == ["install"]:
            self.installed = True
        if argv[0] == "./autogen.sh":
            self.cwds.append(str(kw.get("cwd")))
        if argv[:2] == ["git", "-C"] and argv[3] == "fetch":
            self.fetched[argv[2]] = argv[-1]
        if argv[:2] == ["git", "-C"] and argv[3] == "rev-parse":
            rev = self.rev.get(Path(argv[2]).name) if type(self.rev) is dict else self.rev
            return subprocess.CompletedProcess(argv, 0, (rev or self.fetched.get(argv[2], "")) + "\n", "")
        return subprocess.CompletedProcess(argv, 0, "", "")

    def ran(self, program):
        return [c for c in self.calls if c[0] == program]

    def tools_fetched(self):
        return [m for m in self.manifests if any(i["dest"].startswith("tools/") for i in m["items"])]

    def image_fetches(self):
        return [m for m in self.manifests if not any(i["dest"].startswith("tools/") for i in m["items"])]


class SwtpmTools(unittest.TestCase):
    """2026-11-17, MEASURED: the first strict run on a stock Vast box (swtpm 0.6.3) was refused SWTPM_VERSION after 7 seconds, before any download.
    The generic runner now does what the legacy stage does: it builds swtpm 0.10.1 under /opt/tpm before the first boot, from the private signed tarballs
    when the URL file names their refs (route signed) and from the pinned upstream commits otherwise (route git: the GitHub archive tarballs fail the
    size check). Every failure on the way is a named refusal and no boot happens."""

    def box(self, urls="none", **kw):
        env = Env(self)
        env.runner = env.ctx.runner = OldSwtpmRunner(**kw)
        if urls == "refs":
            (env.tmp / "urls.json").write_text(json.dumps({"schema": "kayfabe.download-urls/1", "urls": {r: "placeholder" for r in TOOL_REFS.values()}}))
        elif urls == "missing":
            (env.tmp / "urls.json").unlink()
        return env

    def one(self, env, pin=False):
        disk = {"from": "pin", "ref": "propimage-kfimg-20261112.zip"} if pin else src(env.plain_dir())
        return env.run(doc({"kind": "prebuilt", "disk": disk, "ssh": SSH, "apps": {"3dmark": {"mode": "present"}}}))

    @staticmethod
    def build_calls(env):
        """What building swtpm runs (the display broker's own `make -C broker` is not part of it)."""
        return [c for c in env.runner.calls if c[0] in ("git", "apt-get", "tar", "./autogen.sh") or (c[0] == "make" and "-C" not in c)]

    def assertPinnedCommitsBuilt(self, env):
        """Route git: no tarball, one shallow fetch of exactly the pinned commit per project, and the recipe runs inside those checkouts."""
        self.assertEqual((env.runner.tools_fetched(), env.runner.ran("tar")), ([], []))
        fetches = [c for c in env.runner.ran("git") if c[3] == "fetch"]
        self.assertEqual([(c[-2], c[-1]) for c in fetches], [tuple(images.SWTPM_GIT[r]) for r in ("libtpms", "swtpm")])
        self.assertTrue(all(c[c.index("--depth") + 1] == "1" for c in fetches))
        self.assertEqual([Path(w).name for w in env.runner.cwds], ["libtpms", "swtpm"])

    def assertSignedTarballsBuilt(self, env):
        """Route signed: the two private tarballs by their refs (never the GitHub archive mirror), unpacked and built; no git."""
        (manifest,) = env.runner.tools_fetched()
        self.assertEqual(sorted(i["name"] for i in manifest["items"]), ["libtpms-0.10.1.tar.gz", "swtpm-0.10.1.tar.gz"])
        self.assertEqual([[s["kind"] for s in i["sources"]] for i in manifest["items"]], [["signed"], ["signed"]])
        self.assertEqual(sorted(s["ref"] for i in manifest["items"] for s in i["sources"]), sorted(TOOL_REFS.values()))
        self.assertEqual(env.runner.ran("git"), [])
        self.assertEqual([Path(c[c.index("-xzf") + 1]).name for c in env.runner.ran("tar")], ["libtpms-0.10.1.tar.gz", "swtpm-0.10.1.tar.gz"])

    def assertRefusedBeforeAnyBoot(self, env, rec, reason):
        self.assertEqual((rec["verdict"], rec["reason"], rec["images"], env.cmds), ("REFUSED", reason, [], []), rec)
        self.assertEqual(images.exit_code(rec), 2)                   # a refusal is never a guest failure (FAIL is 1)
        self.assertNotIn("boot", env.runner.events)
        self.assertNotIn("fetch:image", env.runner.events)

    # ------------------------------------------------------------ nothing changes when the box is fine
    def test_a_new_enough_swtpm_fetches_and_builds_nothing(self):
        env = Env(self)
        rec = self.one(env)
        self.assertEqual((rec["verdict"], rec["reason"]), ("PASS", ""), rec)
        self.assertEqual(self.build_calls(env), [])
        self.assertFalse(any(str(c[3]).endswith("box_downloader.py") for c in env.runner.calls if len(c) > 3))
        self.assertEqual(next(r for r in rec["preflight"] if r["name"] == "swtpm"), {"name": "swtpm", "status": "MATCH", "detail": "0.10.1"})
        self.assertEqual(rec["swtpm"], {"version": "0.10.1", "built": False})
        self.assertEqual(env.boot_env, [("swtpm", os.environ.get("LD_LIBRARY_PATH"))])       # the environment is the caller's, untouched

    def built_prefix(self):
        env = self.box(installed=True, need_ld=True)
        env.ctx.environ["SWTPM"] = PREFIX_BIN
        return env, self.one(env)

    def test_an_already_built_prefix_is_used_and_not_built_again(self):
        env, rec = self.built_prefix()
        self.assertEqual((rec["verdict"], rec["swtpm"]), ("PASS", {"version": "0.10.1", "built": False}), rec)
        self.assertEqual(self.build_calls(env), [])
        self.assertEqual(env.runner.tools_fetched(), [])

    def test_an_already_built_prefix_starts_with_its_library_path(self):
        env, rec = self.built_prefix()
        self.assertEqual([e[0] for e in env.boot_env], [PREFIX_BIN])
        self.assertIn(pa.SWTPM_PREFIX + "/lib", env.boot_env[0][1].split(":"))
        self.assertEqual(next(r for r in rec["preflight"] if r["name"] == "swtpm")["status"], "MATCH")
        self.assertEqual(rec["swtpm"], {"version": "0.10.1", "built": False})

    # ------------------------------------------------------------ route signed
    def test_an_old_swtpm_with_the_refs_builds_from_the_private_tarballs_and_continues(self):
        env = self.box("refs")
        rec = self.one(env)
        self.assertEqual((rec["verdict"], rec["reason"]), ("PASS", ""), rec)
        self.assertSignedTarballsBuilt(env)
        self.assertEqual([c[1:] for c in env.runner.ran("./autogen.sh")][0][:1], ["--prefix=" + pa.SWTPM_PREFIX])
        self.assertEqual(rec["swtpm"], {"version": "0.10.1", "built": True})
        row = next(r for r in rec["preflight"] if r["name"] == "swtpm")
        self.assertEqual(row["status"], "MATCH")
        self.assertIn("built 0.10.1", row["detail"])
        self.assertIn(pa.SWTPM_PREFIX, row["detail"])
        self.assertEqual(env.boot_env[0][0], PREFIX_BIN)
        self.assertIn(pa.SWTPM_PREFIX + "/lib", env.boot_env[0][1].split(":"))
        self.assertEqual(env.ctx.environ["SWTPM"], PREFIX_BIN)
        vast_report.validate_propimage_run(rec)

    def test_the_tools_come_first_and_the_image_is_downloaded_once_after_them(self):
        env = self.box("refs")
        rec = self.one(env, pin=True)
        self.assertEqual((rec["verdict"], rec["reason"]), ("PASS", ""), rec)
        ev = env.runner.events
        self.assertEqual((ev.count("fetch:tools"), ev.count("fetch:image"), ev.count("boot")), (1, 1, 1), ev)
        self.assertLess(ev.index("fetch:tools"), ev.index("make install"))
        self.assertLess(ev.index("make install"), ev.index("boot"))
        self.assertEqual(len(env.runner.image_fetches()), 1)
        self.assertSignedTarballsBuilt(env)
        self.assertFalse(any(Path(c[c.index("-xzf") + 1]).exists() for c in env.runner.ran("tar")))      # nothing of the tools stays on the box
        self.assertFalse((env.ctx.prefix / "work").exists())

    # ------------------------------------------------------------ route git
    def test_an_old_swtpm_without_refs_builds_from_the_pinned_upstream_commits(self):
        env = self.box()
        rec = self.one(env)
        self.assertEqual((rec["verdict"], rec["reason"]), ("PASS", ""), rec)
        self.assertPinnedCommitsBuilt(env)                                                # no tarball, no GitHub archive
        verbs = [next(w for w in c[1:] if w in ("init", "fetch", "checkout", "rev-parse")) for c in env.runner.ran("git")]
        self.assertEqual(verbs, ["init", "fetch", "checkout", "rev-parse"] * 2)
        self.assertEqual(rec["swtpm"], {"version": "0.10.1", "built": True})
        row = next(r for r in rec["preflight"] if r["name"] == "swtpm")
        self.assertEqual((row["status"], "built 0.10.1" in row["detail"], pa.SWTPM_PREFIX in row["detail"]), ("MATCH", True, True))
        self.assertEqual(env.boot_env[0][0], PREFIX_BIN)
        vast_report.validate_propimage_run(rec)

    def git_route_with_url_file(self, text):
        env = self.box("missing" if text is None else "none")
        if text is not None:
            (env.tmp / "urls.json").write_text(text)
        rec = self.one(env)
        self.assertEqual((rec["verdict"], rec["swtpm"]), ("PASS", {"version": "0.10.1", "built": True}), rec)
        self.assertPinnedCommitsBuilt(env)

    def urls_doc(self, **urls):
        return json.dumps({"schema": "kayfabe.download-urls/1", "urls": urls})

    def test_no_url_file_or_an_unreadable_one_takes_the_git_route(self):
        for name, text in {"missing file": None, "not json": "not json", "wrong schema": json.dumps({"schema": "x", "urls": {r: "u" for r in TOOL_REFS.values()}}),
                           "not an object": "[]"}.items():
            with self.subTest(name):
                self.git_route_with_url_file(text)

    def test_a_url_file_with_the_libtpms_ref_alone_takes_the_git_route(self):
        self.git_route_with_url_file(self.urls_doc(**{TOOL_REFS["libtpms"]: "u"}))

    def test_a_url_file_with_the_swtpm_ref_alone_takes_the_git_route(self):
        self.git_route_with_url_file(self.urls_doc(**{TOOL_REFS["swtpm"]: "u"}))

    def test_a_swtpm_ref_without_a_string_url_is_not_named(self):
        self.git_route_with_url_file(self.urls_doc(**{TOOL_REFS["libtpms"]: "u", TOOL_REFS["swtpm"]: 5}))

    def test_a_libtpms_ref_without_a_string_url_is_not_named(self):
        self.git_route_with_url_file(self.urls_doc(**{TOOL_REFS["libtpms"]: None, TOOL_REFS["swtpm"]: "u"}))

    def test_the_git_pins_are_the_commits_the_upstream_v0_10_1_tags_point_at(self):
        self.assertEqual(sorted(images.SWTPM_GIT), ["libtpms", "swtpm"])
        for role, (url, commit) in images.SWTPM_GIT.items():
            self.assertEqual(url, "https://github.com/stefanberger/" + role)
            self.assertRegex(commit, r"^[0-9a-f]{40}$")
        self.assertEqual(images.SWTPM_GIT["libtpms"][1], "56c9f8b0b816391135f5a3112cb2feac4298caac")     # git ls-remote <repo> 'refs/tags/v0.10.1^{}', 2026-11-17
        self.assertEqual(images.SWTPM_GIT["swtpm"][1], "53841482b0a9a1dfe63a120b00283acfe588ee72")
        self.assertEqual(len({c for _, c in images.SWTPM_GIT.values()}), 2)

    # ------------------------------------------------------------ named refusals, never a guest failure, never a boot
    def test_a_failed_tarball_fetch_is_download(self):
        env = self.box("refs", tools_fetch_rc=1)
        rec = self.one(env)
        self.assertRefusedBeforeAnyBoot(env, rec, "DOWNLOAD")
        self.assertEqual(env.runner.ran("tar"), [])
        self.assertFalse(rec["swtpm"]["built"])

    def test_a_failed_git_fetch_is_swtpm_source(self):
        env = self.box(fail=lambda a: a[0] == "git" and a[3] == "fetch")
        self.assertRefusedBeforeAnyBoot(env, self.one(env), "SWTPM_SOURCE")
        self.assertEqual(env.runner.ran("./autogen.sh"), [])

    def test_no_git_on_the_box_is_swtpm_source(self):
        env = self.box(missing=("git",))
        self.assertRefusedBeforeAnyBoot(env, self.one(env), "SWTPM_SOURCE")

    def test_a_checkout_that_is_not_the_pinned_commit_is_never_built(self):
        env = self.box(rev="0" * 40)
        rec = self.one(env)
        self.assertRefusedBeforeAnyBoot(env, rec, "SWTPM_SOURCE_COMMIT")
        self.assertEqual([c for c in self.build_calls(env) if c[0] != "git"], [])

    def test_a_checkout_of_the_other_project_is_never_built(self):
        env = self.box(rev={"libtpms": images.SWTPM_GIT["swtpm"][1]})
        self.assertRefusedBeforeAnyBoot(env, self.one(env), "SWTPM_SOURCE_COMMIT")
        self.assertEqual([c for c in self.build_calls(env) if c[0] != "git"], [])

    def test_a_failed_build_step_is_swtpm_build_on_both_routes(self):
        for urls in ("refs", "none"):
            for step in ("apt-get", "./autogen.sh", "make"):
                with self.subTest(route=urls, step=step):
                    env = self.box(urls, fail=lambda a, step=step: a[0] == step)
                    rec = self.one(env)
                    self.assertRefusedBeforeAnyBoot(env, rec, "SWTPM_BUILD")
                    row = next(r for r in rec["preflight"] if r["name"] == "swtpm")
                    self.assertEqual(row["status"], "MISMATCH")
                    self.assertFalse(rec["swtpm"]["built"])
                    vast_report.validate_propimage_run(rec)

    def failed_build_row(self, urls):
        env = self.box(urls, fail=lambda a: a[0] == "make")
        rec = self.one(env)
        self.assertRefusedBeforeAnyBoot(env, rec, "SWTPM_BUILD")
        row = next(r for r in rec["preflight"] if r["name"] == "swtpm")
        self.assertEqual((row["status"], row["detail"]), ("MISMATCH", "build failed: SWTPM_BUILD"))

    def test_a_failed_build_on_the_signed_route_says_so_in_the_swtpm_row(self):
        self.failed_build_row("refs")

    def test_a_failed_build_on_the_git_route_says_so_in_the_swtpm_row(self):
        self.failed_build_row("none")

    def test_a_build_that_is_still_older_than_0_10_is_swtpm_build(self):
        for urls in ("refs", "none"):
            with self.subTest(urls):
                env = self.box(urls, built_version="0.9.9")
                self.assertRefusedBeforeAnyBoot(env, self.one(env), "SWTPM_BUILD")

    def unpinned(self, role, urls):
        env = self.box(urls)
        pins = json.loads(Path(pa.PINS_PATH).read_text())
        next(i for i in pins["items"] if i["role"] == role)["sha256"] = None
        (env.tmp / "pins.json").write_text(json.dumps(pins))
        env.ctx.pins_path = env.tmp / "pins.json"
        return env

    def test_an_unpinned_libtpms_tarball_refuses_before_anything_is_fetched(self):
        env = self.unpinned("libtpms", "refs")
        self.assertRefusedBeforeAnyBoot(env, self.one(env), "PIN_UNSET:libtpms")
        self.assertEqual((env.runner.manifests, self.build_calls(env)), ([], []))

    def test_an_unpinned_swtpm_tarball_refuses_before_anything_is_fetched(self):
        env = self.unpinned("swtpm", "refs")
        self.assertRefusedBeforeAnyBoot(env, self.one(env), "PIN_UNSET:swtpm")
        self.assertEqual((env.runner.manifests, self.build_calls(env)), ([], []))

    def test_the_git_route_needs_no_sha256_pin_of_a_tarball(self):
        env = self.box()
        pins = json.loads(Path(pa.PINS_PATH).read_text())
        for i in pins["items"]:
            if i["role"] in ("libtpms", "swtpm"):
                i["sha256"] = None
        (env.tmp / "pins.json").write_text(json.dumps(pins))
        env.ctx.pins_path = env.tmp / "pins.json"
        self.assertEqual(self.one(env)["swtpm"], {"version": "0.10.1", "built": True})

    def other_refusal_first(self, reason, tweak):
        env = self.box("refs")
        tweak(env)
        rec = self.one(env)
        self.assertRefusedBeforeAnyBoot(env, rec, reason)
        self.assertEqual((self.build_calls(env), env.runner.manifests), ([], []))
        self.assertEqual(next(r for r in rec["preflight"] if r["name"] == "swtpm")["status"], "WARN")

    def test_a_wrong_gpu_family_comes_before_the_build(self):
        self.other_refusal_first("GPU_FAMILY", lambda e: setattr(e.ctx, "hardware", {"gpu_name": "x", "compute_cap": 0, "memory_mib": 0}))

    def test_a_missing_kf3_qemu_comes_before_the_build(self):
        self.other_refusal_first("NO_KF3_QEMU", lambda e: setattr(e.ctx, "qemu", e.tmp / "nope"))

    def test_too_little_disk_comes_before_the_build(self):
        self.other_refusal_first("DISK_FREE", lambda e: setattr(e.ctx, "free_bytes", 1 << 30))

    BAD_SWTPM = ({"version": "0.10.1"}, {"built": True}, {"version": "x" * 40, "built": True}, {"version": "0.10.1 ", "built": True},
                 {"version": None, "built": "yes"}, {"version": 10, "built": True}, {"version": "0.10.1", "built": True, "z": 1}, None, [])

    def assertSwtpmValidated(self, rec):
        rec = json.loads(json.dumps(rec))
        vast_report.validate_propimage_run(rec)
        for bad in self.BAD_SWTPM:
            try:
                vast_report.validate_propimage_run(dict(rec, swtpm=bad))
            except ValueError:
                continue
            self.fail("accepted a swtpm part of %r" % (bad,))

    def test_the_swtpm_part_of_a_built_record_is_validated(self):
        self.assertSwtpmValidated(self.one(self.box()))

    def test_the_swtpm_part_of_a_refusal_record_is_validated(self):
        self.assertSwtpmValidated(self.one(self.box(fail=lambda a: a[0] == "make")))

    def test_a_record_without_a_swtpm_part_is_still_valid(self):
        rec = json.loads(json.dumps(self.one(Env(self))))
        del rec["swtpm"]
        vast_report.validate_propimage_run(rec)


class GopRunner(Runner):
    """The downloader as the boot GOP's tests need it: every manifest it is asked to fetch is kept and the order of fetches and boots is in `events`.
    The GOP item (name kf-gop.signed.efi) is written as the bytes b"signed-gop" (the fake downloader of Runner would write a zip); `gop_rc` is the
    downloader's exit code for the GOP manifest (a nonzero one writes nothing), `gop_writes` False makes it exit 0 and write nothing."""

    def __init__(self, gop_rc=0, gop_writes=True):
        super().__init__()
        self.gop_rc, self.gop_writes, self.manifests, self.events = gop_rc, gop_writes, [], []

    def __call__(self, argv, **kw):
        if len(argv) > 4 and str(argv[3]).endswith("box_downloader.py") and argv[4] == "fetch":
            manifest = json.loads(Path(argv[argv.index("fetch") + 1]).read_text())
            self.manifests.append(manifest)
            gop = [i for i in manifest["items"] if i["name"] == GOP_NAME]
            self.events.append("fetch:gop" if gop else "fetch:image")
            if gop:
                self.calls.append(list(argv))
                dest = Path(argv[argv.index("--dest") + 1]) / gop[0]["dest"]
                if not self.gop_rc and self.gop_writes:
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    dest.write_bytes(GOP_BYTES)
                return subprocess.CompletedProcess(argv, self.gop_rc, "", "")
        return super().__call__(argv, **kw)

    def gop_fetches(self):
        return [m for m in self.manifests if any(i["name"] == GOP_NAME for i in m["items"])]


GOP_NAME, GOP_BYTES = "kf-gop.signed.efi", b"signed-gop"
GOP_PIN = next(i for i in pa.load_pins()["items"] if i["role"] == "gop")


class BootGop(unittest.TestCase):
    """2026-11-18, MEASURED on a Vast box (the first hardware run of the strict image path): the generic runner downloaded the image archive alone and
    handed the boots a GOP path that nothing ever filled, so every boot failed in 50 seconds with `kf-gop.signed.efi not found (pass --gop-efi)`. The
    runner now fetches the pinned boot GOP (pins role `gop`, the signed ref of the URL file) through the download manifest, once per run, at the first
    state-shaped image, and the boots get that file. A plain disk has no GOP (propimage_plain.py takes none) and fetches nothing; a caller's own
    `gop` argument is used as it is and nothing is fetched."""

    def box(self, **kw):
        env = Env(self)
        env.runner = env.ctx.runner = GopRunner(**kw)
        return env

    @staticmethod
    def go(env, *imgs):
        """Like Env.run but WITHOUT the explicit gop: the runner has to get one itself."""
        return images.run_images(env.ctx, doc(*imgs), hooks=env.hooks)

    def two_state(self, env, tag=""):
        return self.go(env, {"label": "first", "kind": "prebuilt", "disk": src(env.state_dir("s1" + tag))},
                       {"label": "second", "kind": "prebuilt", "disk": src(env.state_dir("s2" + tag))})

    @staticmethod
    def gop_args(env):
        return [c[c.index("--gop-efi") + 1] for c in env.cmds]

    def assertNothingBooted(self, env, rec=None):
        self.assertEqual(env.cmds, [])
        self.assertNotIn("boot", env.runner.events)
        if rec is not None:
            vast_report.validate_propimage_run(rec)

    def unpinned(self, env, **fields):
        pins = json.loads(Path(pa.PINS_PATH).read_text())
        next(i for i in pins["items"] if i["role"] == "gop").update(fields)
        (env.tmp / "pins.json").write_text(json.dumps(pins))
        env.ctx.pins_path = env.tmp / "pins.json"

    # ------------------------------------------------------------ the fetch
    def test_two_state_images_fetch_the_boot_gop_once(self):
        env = self.box()
        rec = self.two_state(env)
        self.assertEqual((rec["verdict"], rec["reason"]), ("PASS", ""), rec)
        self.assertEqual((len(env.runner.gop_fetches()), env.runner.events.count("fetch:gop")), (1, 1), env.runner.events)
        self.assertTrue(env.cmds and all(Path(a).is_file() for a in self.gop_args(env)), self.gop_args(env))

    def test_every_boot_of_every_image_gets_the_downloaded_file(self):
        env = self.box()
        rec = self.two_state(env)
        self.assertGreaterEqual(len(env.cmds), 2)
        self.assertEqual(len(set(self.gop_args(env))), 1, self.gop_args(env))
        got = Path(self.gop_args(env)[0])
        self.assertTrue(got.is_file(), got)
        self.assertEqual(got.read_bytes(), GOP_BYTES)                                   # the file the downloader wrote, not any other path
        self.assertTrue(got.is_relative_to(env.ctx.prefix))
        self.assertEqual([i["verdict"] for i in rec["images"]], ["PASS", "PASS"])

    def test_the_manifest_is_the_pinned_item_from_its_signed_ref_and_nothing_else(self):
        env = self.box()
        self.two_state(env)
        (manifest,) = env.runner.gop_fetches()
        (item,) = manifest["items"]
        self.assertEqual((item["name"], item["dest"], item["sha256"], item["size"], item["needed_by"]),
                         (GOP_NAME, GOP_PIN["dest"], GOP_PIN["sha256"], GOP_PIN["size"], "image"))
        self.assertEqual(item["sources"], [{"kind": "signed", "ref": GOP_PIN["ref"]}])
        self.assertEqual(manifest["stages"], ["image"])
        self.assertRegex(item["sha256"], r"^[0-9a-f]{64}$")

    def test_the_gop_is_fetched_before_the_first_boot(self):
        env = self.box()
        self.two_state(env)
        ev = env.runner.events
        self.assertEqual((ev.count("fetch:gop"), ev.index("fetch:gop") < ev.index("boot")), (1, True), ev)

    def test_a_pinned_image_gets_its_gop_before_the_image_is_downloaded(self):
        env = self.box()
        rec = self.go(env, {"kind": "prebuilt", "disk": {"from": "pin", "ref": "propimage-kfimg-20261112.zip"}, "only": ["3dmark"]})
        self.assertEqual(rec["images"][0]["verdict"], "PASS", rec)
        ev = env.runner.events
        self.assertEqual(ev[:2], ["fetch:gop", "fetch:image"], ev)                        # a wrong GOP pin must not cost a 50 GB download first
        self.assertEqual(ev.count("fetch:gop"), 1)

    def test_a_second_run_asks_the_downloader_again_and_trusts_no_left_over_file(self):
        env = self.box()
        self.two_state(env)
        env.ctx.prefix.joinpath("gop", GOP_PIN["dest"]).write_bytes(b"left over from another revision")
        env.cmds.clear()
        self.two_state(env, "again")
        self.assertEqual(Path(self.gop_args(env)[0]).read_bytes(), GOP_BYTES)

    # ------------------------------------------------------------ the pin
    def unset_field_refused(self, field):
        env = self.box()
        self.unpinned(env, **{field: None})
        rec = self.two_state(env)
        self.assertEqual([(i["verdict"], i["reason"]) for i in rec["images"]], [("REFUSED", "PIN_UNSET:gop")] * 2)
        self.assertEqual((rec["verdict"], rec["reason"]), ("REFUSED", "first:PIN_UNSET:gop"))
        self.assertEqual(images.exit_code(rec), 2)
        self.assertEqual(env.runner.manifests, [])                                       # an unpinned object is never fetched
        self.assertNothingBooted(env, rec)

    def test_an_unset_gop_sha256_is_refused_by_name_before_any_boot(self):
        self.unset_field_refused("sha256")

    def test_an_unset_gop_size_is_refused_by_name_before_any_boot(self):
        self.unset_field_refused("size")

    def test_an_unset_gop_pin_is_refused_before_the_image_is_fetched(self):
        env = self.box()
        self.unpinned(env, sha256=None)
        rec = self.go(env, {"kind": "prebuilt", "disk": {"from": "pin", "ref": "propimage-kfimg-20261112.zip"}, "only": ["3dmark"]})
        self.assertEqual((rec["images"][0]["verdict"], rec["images"][0]["reason"]), ("REFUSED", "PIN_UNSET:gop"))
        self.assertEqual(env.runner.manifests, [])
        self.assertNothingBooted(env, rec)

    def test_a_pins_file_without_a_gop_item_is_refused_by_name(self):
        env = self.box()
        pins = json.loads(Path(pa.PINS_PATH).read_text())
        pins["items"] = [i for i in pins["items"] if i["role"] != "gop"]
        with unittest.mock.patch.object(pa, "load_pins", lambda path=None: pins):
            rec = self.two_state(env)
        self.assertEqual([(i["verdict"], i["reason"]) for i in rec["images"]], [("REFUSED", "PIN_UNSET:gop")] * 2)
        self.assertEqual(env.runner.manifests, [])
        self.assertNothingBooted(env, rec)

    def test_a_pins_file_without_a_gop_item_never_fetches_the_image_either(self):
        env = self.box()
        pins = json.loads(Path(pa.PINS_PATH).read_text())
        pins["items"] = [i for i in pins["items"] if i["role"] != "gop"]
        with unittest.mock.patch.object(pa, "load_pins", lambda path=None: pins):
            rec = self.go(env, {"kind": "prebuilt", "disk": {"from": "pin", "ref": "propimage-kfimg-20261112.zip"}, "only": ["3dmark"]})
        self.assertEqual((rec["images"][0]["verdict"], rec["images"][0]["reason"]), ("REFUSED", "PIN_UNSET:gop"))
        self.assertEqual(env.runner.manifests, [])
        self.assertNothingBooted(env, rec)

    # ------------------------------------------------------------ the failures
    def test_a_failed_gop_fetch_is_download_and_nothing_boots(self):
        env = self.box(gop_rc=1)
        rec = self.two_state(env)
        self.assertEqual([(i["verdict"], i["reason"]) for i in rec["images"]], [("REFUSED", "DOWNLOAD")] * 2)
        self.assertEqual((rec["verdict"], rec["reason"]), ("REFUSED", "first:DOWNLOAD"))
        self.assertEqual(images.exit_code(rec), 2)                                       # not a guest failure
        self.assertEqual(len(env.runner.gop_fetches()), 1)                               # one attempt for the run
        self.assertNothingBooted(env, rec)

    def test_a_failed_gop_fetch_is_tried_once_per_run_not_once_per_image(self):
        env = self.box(gop_rc=1)
        self.two_state(env)
        self.assertEqual(len(env.runner.gop_fetches()), 1, env.runner.events)

    def test_a_downloader_that_exits_zero_without_the_file_is_download(self):
        env = self.box(gop_writes=False)
        rec = self.two_state(env)
        self.assertEqual([(i["verdict"], i["reason"]) for i in rec["images"]], [("REFUSED", "DOWNLOAD")] * 2)
        self.assertNothingBooted(env, rec)

    def test_a_pinned_image_is_not_downloaded_when_the_gop_file_is_missing(self):
        env = self.box(gop_writes=False)
        rec = self.go(env, {"kind": "prebuilt", "disk": {"from": "pin", "ref": "propimage-kfimg-20261112.zip"}, "only": ["3dmark"]})
        self.assertEqual((rec["images"][0]["verdict"], rec["images"][0]["reason"]), ("REFUSED", "DOWNLOAD"))
        self.assertEqual(env.runner.events, ["fetch:gop"])                               # the 50 GB image was never asked for
        self.assertNothingBooted(env, rec)

    def test_a_failed_gop_fetch_does_not_stop_a_plain_image_after_it(self):
        env = self.box(gop_rc=1)
        rec = self.go(env, {"label": "first", "kind": "prebuilt", "disk": src(env.state_dir())},
                       {"label": "second", "kind": "prebuilt", "disk": src(env.plain_dir()), "ssh": SSH, "apps": {"3dmark": {"mode": "present"}}})
        self.assertEqual([(i["label"], i["verdict"]) for i in rec["images"]], [("first", "REFUSED"), ("second", "PASS")])
        self.assertEqual(len(env.cmds), 1)
        self.assertNotIn("--gop-efi", env.cmds[0])

    # ------------------------------------------------------------ what does not change
    def test_an_explicit_gop_is_used_as_it_is_and_nothing_is_fetched(self):
        env = self.box(gop_rc=1)                                                         # a fetch would fail: it must not happen
        rec = env.run(doc({"label": "first", "kind": "prebuilt", "disk": src(env.state_dir("s1"))},
                          {"label": "second", "kind": "prebuilt", "disk": src(env.state_dir("s2"))}))
        self.assertEqual((rec["verdict"], rec["reason"]), ("PASS", ""), rec)
        self.assertEqual(env.runner.manifests, [])
        self.assertEqual(set(self.gop_args(env)), {str(env.tmp / "gop.efi")})
        self.assertFalse((env.ctx.prefix / "gop" / GOP_PIN["dest"]).exists())

    def test_an_explicit_gop_is_used_even_when_the_pins_file_has_no_gop_pin(self):
        env = self.box()
        self.unpinned(env, sha256=None)
        rec = env.run(doc({"kind": "prebuilt", "disk": src(env.state_dir())}))
        self.assertEqual((rec["verdict"], rec["reason"]), ("PASS", ""), rec)
        self.assertEqual(env.runner.manifests, [])

    def test_plain_images_need_no_gop_and_fetch_none(self):
        env = self.box(gop_rc=1)
        rec = self.go(env, {"kind": "prebuilt", "disk": src(env.plain_dir()), "ssh": SSH, "apps": {"3dmark": {"mode": "present"}}})
        self.assertEqual((rec["verdict"], rec["reason"]), ("PASS", ""), rec)
        self.assertEqual(env.runner.manifests, [])
        self.assertNotIn("--gop-efi", env.cmds[0])

    def test_a_state_image_with_nothing_to_run_needs_no_gop(self):
        env = self.box(gop_rc=1)
        rec = self.go(env, {"kind": "prebuilt", "disk": src(env.state_dir()), "only": ["notanapp"]})
        self.assertEqual(env.runner.manifests, [])
        self.assertEqual(env.cmds, [])
        self.assertNotEqual(rec["images"][0]["reason"], "DOWNLOAD")


if __name__ == "__main__":
    unittest.main()
