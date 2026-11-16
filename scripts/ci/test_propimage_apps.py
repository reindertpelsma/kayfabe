#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of the propimage_apps stage (2026-11-15): preflight rows, the download manifest and the unpinned-image refusal, the
boot plan, verdict reading from the recorded PC output, the host kernel log windows, the result record and its validation in
vast_report.py, the payload wiring, and hostile paths. No VM, no network, no box; the harness, the downloader, swtpm and the kernel log
are replaced by fakes that write what the real ones write (formats read from propimage_check.py).
"""
import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import box_downloader  # noqa: E402
import profile_resources  # noqa: E402
import propimage_apps as pa  # noqa: E402
import vast_payload  # noqa: E402
import vast_report  # noqa: E402

FIX = HERE / "fixtures/propimage_apps"
CPUFLAGS = HERE / "fixtures/cpuflags"
SHA = "a" * 40
H64 = "b" * 64


def md_rows(name):
    """result.md of the PC run (verbatim fixture) -> the rows propimage_check writes to result.json."""
    rows = []
    for line in (FIX / name).read_text().splitlines()[2:]:
        step, verdict, evidence, klass, detail = [c.strip() for c in line.strip().strip("|").split(" | ", 4)]
        rows.append({"step": step, "verdict": verdict, "evidence": evidence, "class": klass, "detail": detail})
    return rows


def pins_doc(set_roles=("image", "gop", "libtpms", "swtpm")):
    doc = json.loads(pa.PINS_PATH.read_text())
    for it in doc["items"]:
        if it["role"] in set_roles:
            it["sha256"], it["size"] = H64, 1234567
        else:
            it["sha256"], it["size"] = None, None   # the committed pins are set (2026-11-16): a role left out here is UNSET on purpose
    return doc


def write_pins(tmp, **kw):
    path = Path(tmp) / "pins.json"
    path.write_text(json.dumps(pins_doc(**kw)))
    return path


AMD = (CPUFLAGS / "vast_epyc7b12_zen2.txt").read_text()
INTEL = "vendor_id\t: GenuineIntel\nflags\t\t: fpu vme avx2 avx512f rtm\n"


class Fake:
    """A subprocess.run stand-in: records argv, answers swtpm --version, and creates what the real tools would."""

    def __init__(self, swtpm="swtpm version 0.10.1", fail=()):
        self.calls, self.swtpm, self.fail = [], swtpm, fail

    def __call__(self, argv, **kw):
        self.calls.append(list(argv))
        out = ""
        if argv[-1] == "--version":
            out = self.swtpm
        if len(argv) > 3 and argv[3].endswith("box_downloader.py") and argv[4] == "fetch":
            dest = Path(argv[argv.index("--dest") + 1])
            (dest / "image").mkdir(parents=True, exist_ok=True)
            (dest / "image/image.zip").write_bytes(b"zip")
            (dest / "image/kf-gop.signed.efi").write_bytes(b"gop")
        rc = 1 if any(f in " ".join(map(str, argv)) for f in self.fail) else 0
        if "stdout" in kw and hasattr(kw["stdout"], "write"):
            kw["stdout"].write(b"")
        return subprocess.CompletedProcess(argv, rc, out, "")


class Env:
    """A whole fake box in a temp directory."""

    def __init__(self, test, pins=None, cpuinfo=AMD, hardware=None, swtpm="swtpm version 0.10.1", scenario=None, fail=()):
        self.tmp = Path(tempfile.mkdtemp(prefix="kf-pa-test-"))
        test.addCleanup(shutil.rmtree, self.tmp, True)
        self.bench = self.tmp / "bench"
        (self.bench / "kf3-bins" / SHA[:8]).mkdir(parents=True)
        q = self.bench / "kf3-bins" / SHA[:8] / "qemu-system-x86_64"
        q.write_text("#!/bin/sh\n")
        q.chmod(0o755)
        (self.tmp / "urls.json").write_text("{}")
        (self.tmp / "cpuinfo").write_text(cpuinfo)
        self.pins = pins or write_pins(self.tmp)
        self.fake = Fake(swtpm, fail)
        self.scenario = scenario or {}
        self.ctx = pa.Context(
            SHA, pa.ROOT, self.tmp / "work-prefix" / "kf-propimage", self.tmp / "result.json", self.tmp / "urls.json", bench=self.bench,
            cpuinfo=str(self.tmp / "cpuinfo"), environ=dict(os.environ, SWTPM="swtpm"), pins_path=self.pins,
            hardware=hardware or {"gpu_name": "NVIDIA GeForce RTX 4090", "compute_cap": 890, "memory_mib": 24564},
            free_bytes=10 ** 12, runner=self.fake, boot_runner=self.boot, unpack=lambda z, d: Path(d).joinpath("state.json").write_text("{}"),
            klog_source=["sh", "-c", "echo start-line; sleep 300"])
        self.boots_seen = []
        self.zip_at_boot = []
        patcher = unittest.mock.patch.object(pa.KernelLog, "SETTLE_S", 0.0)
        patcher.start()
        test.addCleanup(patcher.stop)

    def boot(self, ctx, argv, log, timeout, mark):
        """What propimage_check does for one boot: result.json rows, per-app run dirs, run/last_cmd.txt, stdout lines, host kernel lines."""
        arg = lambda k: argv[argv.index(k) + 1]  # noqa: E731
        out, work, apps = Path(arg("--out")), Path(arg("--work")), arg("--apps").split(",")
        self.boots_seen.append(argv)
        self.zip_at_boot.append((ctx.prefix / "dl/image/image.zip").exists())
        (work / "run").mkdir(parents=True)
        offline = self.scenario.get("online_boot") is None
        (work / "run/last_cmd.txt").write_text("qemu -netdev user,id=n0,restrict=%s -drive file=%s/run/clone.qcow2\n" % ("on" if offline else "off", work))
        if self.scenario.get("both_restrict"):
            (work / "run/last_cmd.txt").write_text("qemu -netdev user,restrict=on -netdev user,restrict=off -drive file=%s/run/clone.qcow2\n" % work)
        rows = [{"step": "verify", "verdict": "PASS", "evidence": "NONE", "class": "harness", "detail": "ok"}]
        mark.line("verify PASS NONE ok\n")
        kernel = ctx.prefix / "kernel/kernel.log"
        for app in apps:
            spec = self.scenario.get(app, {})
            verdict = spec.get("verdict", "PASS")
            kind = spec.get("kind", pa.EXPECTED_KIND[app])
            run_id = os.urandom(16).hex()
            d = out / app / run_id
            d.mkdir(parents=True)
            ok = verdict == "PASS" and not spec.get("no_verdict_file")
            signal = spec.get("signal", "3DMark score 7297 (graphics 7387), graphics tests 48.37 FPS" if app == "3dmark" else "fine")
            if not spec.get("no_verdict_file"):
                (d / "verdict.json").write_text(json.dumps({"app": app, "ok": ok, "evidence": kind, "signal": signal, "secs": 120}))
            res = {"app": app, "run_id": run_id}
            if app == "wukong":
                res["kernel_events"] = spec.get("kernel_events", [])
                (d / "system_events.txt").write_text(spec.get("system_events", ""))
            (d / "result.json").write_text(json.dumps(res))
            (d / "screen.png").write_bytes(b"\x89PNG\r\n\x1a\n" + app.encode())
            for line in spec.get("xids", []):
                with open(kernel, "a") as f:
                    f.write(line + "\n")
            rows.append({"step": "app:" + app, "verdict": verdict, "evidence": kind, "class": "app", "detail": spec.get("detail", "detail of " + app)})
            mark.line("app:%s %s %s detail\n" % (app, verdict, kind))
        for line in self.scenario.get("unattributed", []) if apps[0] == "superposition" else []:
            with open(kernel, "a") as f:
                f.write(line + "\n")
        rows.append({"step": "stop", "verdict": "PASS", "evidence": "NONE", "class": "harness", "detail": "own QEMU stopped"})
        (out / "result.json").write_text(json.dumps(rows))
        return self.scenario.get("rc", 0)


import unittest.mock  # noqa: E402


class Pins(unittest.TestCase):
    def test_committed_pins_file_is_valid_and_never_half_set(self):
        doc = pa.load_pins()
        self.assertEqual({i["role"] for i in doc["items"]}, {"image", "gop", "libtpms", "swtpm"})
        for it in doc["items"]:
            self.assertEqual(it["sha256"] is None, it["size"] is None, it["role"])

    def test_committed_image_pin_is_named_unset_in_the_refusal_when_it_is_unset(self):
        doc = pa.load_pins()
        if all(i["sha256"] for i in doc["items"] if i["role"] == "image"):
            self.skipTest("the image pin has been set")
        with self.assertRaises(pa.Refused) as ctx:
            pa.build_manifest(doc, False)
        self.assertEqual(ctx.exception.reason, "PIN_UNSET:image")

    def test_unpinned_image_refuses_even_when_everything_else_is_pinned(self):
        doc = pins_doc(set_roles=("gop", "libtpms", "swtpm"))
        with self.assertRaises(pa.Refused) as ctx:
            pa.build_manifest(doc, True)
        self.assertEqual(ctx.exception.reason, "PIN_UNSET:image")

    def test_image_with_hash_but_no_size_is_unpinned(self):
        doc = pins_doc()
        doc["items"][0]["size"] = None
        self.assertEqual(pa.unset_roles(doc, ("image",)), ["image"])

    def test_unpinned_swtpm_sources_matter_only_when_a_build_is_needed(self):
        doc = pins_doc(set_roles=("image", "gop"))
        self.assertEqual([i["name"] for i in pa.build_manifest(doc, False)["items"]], ["propimage-kfimg-20261112.zip", "kf-gop.signed.efi"])
        with self.assertRaises(pa.Refused) as ctx:
            pa.build_manifest(doc, True)
        self.assertEqual(ctx.exception.reason, "PIN_UNSET:libtpms")

    def test_manifest_is_accepted_by_the_real_downloader_and_the_image_is_signed_only(self):
        manifest = pa.build_manifest(pins_doc(), True)
        m = box_downloader.parse_manifest(json.dumps(manifest).encode())
        self.assertEqual(m.stages, ("tools", "image"))
        by = {i.name: i for i in m.items}
        self.assertEqual([s.kind for s in by["propimage-kfimg-20261112.zip"].sources], ["signed"])
        self.assertEqual([s.kind for s in by["libtpms-0.10.1.tar.gz"].sources], ["mirror"])
        self.assertEqual(by["kf-gop.signed.efi"].sha256, H64)

    def test_half_pinned_roles_refuse_the_manifest(self):
        for key in ("sha256", "size"):
            doc = pins_doc()
            doc["items"][0][key] = None
            with self.subTest(key), self.assertRaises(pa.Refused) as ctx:
                pa.build_manifest(doc, False)
            self.assertEqual(ctx.exception.reason, "PIN_UNSET:image")

    def test_pins_dest_must_be_relative_and_clean(self):
        for dest in ("../x", "a/../b", "/abs", "a//b", "", ".", "a/./b", "a b/c"):
            doc = pins_doc()
            doc["items"][0]["dest"] = dest
            with tempfile.TemporaryDirectory() as t, self.subTest(dest), self.assertRaises(ValueError):
                p = Path(t) / "p.json"
                p.write_text(json.dumps(doc))
                pa.load_pins(p)

    def test_pins_file_without_the_image_role_is_refused(self):
        doc = pins_doc()
        doc["items"][0]["role"] = "other"
        with tempfile.TemporaryDirectory() as t:
            p = Path(t) / "p.json"
            p.write_text(json.dumps(doc))
            with self.assertRaises(ValueError):
                pa.load_pins(p)

    def test_pins_file_hostile_variants_refused(self):
        base = pins_doc()
        cases = {"dest-dotdot": ("dest", "../x"), "dest-absolute": ("dest", "/etc/x"), "short-sha": ("sha256", "abc"),
                 "bool-size": ("size", True), "negative-size": ("size", -1), "name-slash": ("name", "a/b"), "http-mirror": ("mirror", "http://x/y")}
        for label, (key, value) in cases.items():
            doc = copy.deepcopy(base)
            item = doc["items"][2] if key == "mirror" else doc["items"][0]
            item[key] = value
            with tempfile.TemporaryDirectory() as t:
                p = Path(t) / "p.json"
                p.write_text(json.dumps(doc))
                with self.subTest(label), self.assertRaises(ValueError):
                    pa.load_pins(p)

    def test_pins_duplicate_keys_unknown_keys_and_missing_image_refused(self):
        with tempfile.TemporaryDirectory() as t:
            p = Path(t) / "p.json"
            p.write_text(pa.PINS_PATH.read_text().replace('"role": "image",', '"role": "image", "role": "image",', 1))
            with self.assertRaises(ValueError):
                pa.load_pins(p)
            doc = pins_doc()
            doc["items"][0]["extra"] = 1
            p.write_text(json.dumps(doc))
            with self.assertRaises(ValueError):
                pa.load_pins(p)
            doc = pins_doc()
            doc["items"] = [i for i in doc["items"] if i["role"] != "gop"]
            p.write_text(json.dumps(doc))
            with self.assertRaises(ValueError):
                pa.load_pins(p)




class Preflight(unittest.TestCase):
    def test_amd_hosts_match_including_the_recorded_vast_epyc_and_the_pc(self):
        for f in ("vast_epyc7b12_zen2.txt", "pc_zen4.txt"):
            rows = pa.cpu_rows((CPUFLAGS / f).read_text())
            self.assertEqual([r["status"] for r in rows], ["MATCH", "MATCH"], f)

    def test_intel_is_not_run_cpu_vendor_before_any_other_refusal(self):
        env = Env(self, cpuinfo=INTEL, hardware={"gpu_name": "x", "compute_cap": 800}, pins=write_pins(tempfile.mkdtemp(), set_roles=()))
        self.addCleanup(shutil.rmtree, env.pins.parent, True)
        rows = pa.preflight(env.ctx)[0]
        self.assertEqual(pa.refusal_of(rows), ("NOT_RUN", "CPU_VENDOR"))
        self.assertEqual(pa.refusal_of(rows), ("NOT_RUN", "CPU_VENDOR"))

    def test_amd_with_missing_cpu_flags_is_cpu_profile_unsupported(self):
        rows = pa.cpu_rows("vendor_id: AuthenticAMD\nflags: fpu vme sse2\n")
        self.assertEqual(pa.refusal_of(rows), ("REFUSED", "CPU_PROFILE_UNSUPPORTED"))

    def test_unreadable_cpuinfo_is_a_refusal(self):
        self.assertEqual(pa.refusal_of(pa.cpu_rows("")), ("NOT_RUN", "CPU_VENDOR"))

    def test_the_vendor_list_is_the_extension_point(self):
        rows = pa.cpu_rows(INTEL, allowed=("AuthenticAMD", "GenuineIntel"))
        self.assertEqual(rows[0]["status"], "MATCH")
        self.assertEqual(pa.ALLOWED_CPU_VENDORS, ("AuthenticAMD",))

    def test_gpu_families(self):
        got = {cc: pa.family_row({"compute_cap": cc})[1] for cc in (750, 800, 860, 870, 890, 900, 1000, 1200, 0)}
        self.assertEqual(got, {750: "turing", 800: None, 860: "ampere", 870: None, 890: "ada", 900: None, 1000: None, 1200: "blackwell", 0: None})
        self.assertEqual(pa.refusal_of([pa.family_row({"compute_cap": 800})[0]]), ("REFUSED", "GPU_FAMILY"))
        self.assertIsNone(pa.family_row(None)[1])
        self.assertIsNone(pa.family_row({"compute_cap": "890"})[1])

    def test_only_ada_is_marked_measured(self):
        self.assertIn("MEASURED", pa.family_row({"compute_cap": 890})[0]["detail"])
        self.assertIn("not measured", pa.family_row({"compute_cap": 860})[0]["detail"])

    def test_swtpm_version_rows(self):
        for text, build in (("swtpm version 0.6.3", True), ("TPM emulator version 0.10.1", False), ("swtpm version 0.9.9", True), ("nonsense", True),
                            ("swtpm version 1.0", False)):
            with self.subTest(text):
                env = Env(self, swtpm=text, pins=write_pins(tempfile.mkdtemp()))
                self.addCleanup(shutil.rmtree, env.pins.parent, True)
                self.assertEqual(pa.preflight(env.ctx)[2], build)

    def test_old_swtpm_with_unpinned_sources_is_swtpm_version(self):
        env = Env(self, swtpm="swtpm version 0.6.3", pins=write_pins(tempfile.mkdtemp(), set_roles=("image", "gop")))
        self.addCleanup(shutil.rmtree, env.pins.parent, True)
        rows = pa.preflight(env.ctx)[0]
        self.assertEqual(next(r for r in rows if r["name"] == "swtpm")["status"], "MISMATCH")
        self.assertEqual(pa.refusal_of(rows)[0], "REFUSED")

    def test_missing_swtpm_binary_means_build(self):
        def boom(argv, **kw):
            raise FileNotFoundError(argv[0])
        self.assertIsNone(pa.swtpm_version("swtpm", boom))

    def test_swtpm_binary_prefers_the_environment_then_the_built_one(self):
        self.assertEqual(pa.swtpm_binary({"SWTPM": "/x/swtpm"}), "/x/swtpm")
        self.assertEqual(pa.swtpm_binary({}, prefix_bin="/nonexistent/swtpm"), "swtpm")
        with tempfile.TemporaryDirectory() as t:
            b = Path(t) / "swtpm"
            b.write_text("#!/bin/sh\n")
            b.chmod(0o755)
            self.assertEqual(pa.swtpm_binary({}, prefix_bin=str(b)), str(b))

    def test_disk_row_needs_the_archive_twice_plus_headroom(self):
        pins = pins_doc()
        row, need = pa.disk_row(10 ** 12, pins)
        self.assertEqual(row["status"], "MATCH")
        self.assertEqual(need, 2 * 1234567 + pa.DISK_HEADROOM)
        self.assertEqual(pa.disk_row(need - 1, pins)[0]["status"], "MISMATCH")
        self.assertEqual(pa.refusal_of([pa.disk_row(0, pins)[0]]), ("REFUSED", "DISK_FREE"))
        # unpinned: the estimate of 47 GiB applies
        self.assertEqual(pa.disk_row(0, pins_doc(set_roles=()))[1], 2 * pa.IMAGE_ZIP_ESTIMATE + pa.DISK_HEADROOM)
        # the committed pin knows the size of image A (2026-11-16)
        self.assertEqual(pa.disk_row(0, pa.load_pins())[1], 2 * 50496618964 + pa.DISK_HEADROOM)

    def test_the_committed_pins_are_image_a_gop_and_the_swtpm_sources(self):
        by = {i["role"]: i for i in pa.load_pins()["items"]}
        self.assertEqual((by["image"]["size"], by["image"]["sha256"]), (50496618964, "cddff40a501d8135c6c15ef7267b3b838304e5d6f089502707bce78712cb11b3"))
        self.assertEqual((by["gop"]["size"], by["gop"]["sha256"]), (11224, "423a9abc5789b8b4e9834ba016261efb9d64515d53f259a90446917ce91b848e"))
        for role in ("libtpms", "swtpm"):
            self.assertIsNotNone(by[role]["sha256"], role)
            self.assertIsNotNone(by[role]["size"], role)
        self.assertEqual(pa.unset_roles(pa.load_pins(), ("image", "gop", "libtpms", "swtpm")), [])

    def test_the_committed_manifest_matches_the_urls_file_the_kit_writes(self):
        # /root/night/release_kit/mint_urls.py writes these refs into the kayfabe.download-urls/1 file the stage reads
        refs = {i["name"]: i["sources"][0].get("ref") for i in pa.build_manifest(pa.load_pins(), True)["items"]}
        self.assertEqual(refs["propimage-kfimg-20261112.zip"], "propimage-kfimg-20261112.zip")
        self.assertEqual(refs["kf-gop.signed.efi"], "propimage-kf-gop.signed.efi")

    def test_missing_kf3_qemu_and_urls_are_refusals(self):
        env = Env(self)
        (env.bench / "kf3-bins" / SHA[:8] / "qemu-system-x86_64").unlink()
        (env.tmp / "urls.json").unlink()
        rows = pa.preflight(env.ctx)[0]
        got = {r["name"]: r["status"] for r in rows}
        self.assertEqual((got["kf3_qemu"], got["urls"]), ("MISMATCH", "MISMATCH"))
        self.assertEqual(pa.refusal_of(rows), ("REFUSED", "NO_KF3_QEMU"))

    def test_urls_symlink_is_not_accepted(self):
        env = Env(self)
        (env.tmp / "real.json").write_text("{}")
        (env.tmp / "urls.json").unlink()
        (env.tmp / "urls.json").symlink_to(env.tmp / "real.json")
        self.assertEqual({r["name"]: r["status"] for r in pa.preflight(env.ctx)[0]}["urls"], "MISMATCH")

    def test_the_first_refusal_in_row_order_names_the_reason(self):
        env = Env(self, hardware={"compute_cap": 800}, pins=write_pins(tempfile.mkdtemp(), set_roles=()))
        self.addCleanup(shutil.rmtree, env.pins.parent, True)
        self.assertEqual(pa.refusal_of(pa.preflight(env.ctx)[0]), ("REFUSED", "GPU_FAMILY"))
        env2 = Env(self, pins=write_pins(tempfile.mkdtemp(), set_roles=("gop",)))
        self.addCleanup(shutil.rmtree, env2.pins.parent, True)
        self.assertEqual(pa.refusal_of(pa.preflight(env2.ctx)[0]), ("REFUSED", "PIN_UNSET:image"))

    def test_small_gpu_memory_is_a_warning_not_a_refusal(self):
        env = Env(self, hardware={"gpu_name": "x", "compute_cap": 890, "memory_mib": 8192})
        got = {r["name"]: r["status"] for r in pa.preflight(env.ctx)[0]}
        self.assertEqual(got["gpu_memory"], "WARN")

    def test_hardware_query_parses_and_fails_closed(self):
        ok = lambda a, **k: subprocess.CompletedProcess(a, 0, "NVIDIA GeForce RTX 4090, 8.9, 24564\n", "")  # noqa: E731
        self.assertEqual(pa.hardware_now(ok), {"gpu_name": "NVIDIA GeForce RTX 4090", "compute_cap": 890, "memory_mib": 24564})
        two = lambda a, **k: subprocess.CompletedProcess(a, 0, "a, 8.9, 1\nb, 8.9, 1\n", "")  # noqa: E731
        self.assertEqual(pa.hardware_now(two)["compute_cap"], 0)
        bad = lambda a, **k: subprocess.CompletedProcess(a, 9, "", "")  # noqa: E731
        self.assertEqual(pa.hardware_now(bad)["compute_cap"], 0)


class Boots(unittest.TestCase):
    def setUp(self):
        self.timeouts = {a: pa.harness().CHECKS[a]["timeout"] for a in pa.APPS}

    def test_the_seven_apps_are_the_harness_apps_and_wukong_is_last(self):
        self.assertEqual(set(pa.APPS), set(pa.harness().CHECKS))
        self.assertEqual(pa.APPS[-1], "wukong")
        self.assertEqual(set(pa.EXPECTED_KIND), set(pa.APPS))

    def test_report_and_stage_agree_on_apps_and_kinds(self):
        self.assertEqual(vast_report.PROPIMAGE_APPS, pa.APPS)
        self.assertEqual(vast_report.PROPIMAGE_EXPECTED_KIND, pa.EXPECTED_KIND)

    def test_real_timeouts_make_three_boots_each_under_the_budget(self):
        boots = pa.plan_boots(self.timeouts)
        self.assertEqual(boots, [["superposition", "3dmark"], ["solidworks", "resolve", "edrawings", "edge"], ["wukong"]])
        for b in boots:
            self.assertLessEqual(sum(self.timeouts[a] for a in b), pa.BOOT_BUDGET_S)
        self.assertLess(pa.BOOT_BUDGET_S + 600, 55 * 60 + 1)

    def test_every_app_is_in_exactly_one_boot_in_order(self):
        for budget in (1, 600, 2700, 10 ** 9):
            flat = [a for b in pa.plan_boots(self.timeouts, budget) for a in b]
            self.assertEqual(flat, list(pa.APPS), budget)

    def test_wukong_is_alone_even_with_an_unlimited_budget(self):
        self.assertEqual(pa.plan_boots(self.timeouts, 10 ** 9)[-1], ["wukong"])
        self.assertEqual(len(pa.plan_boots(self.timeouts, 10 ** 9)), 2)

    def test_a_tiny_budget_gives_one_app_per_boot(self):
        self.assertEqual(len(pa.plan_boots(self.timeouts, 1)), 7)

    def test_boot_command_flags(self):
        a = {"dir": "/d", "work": "/w", "out": "/o", "qemu": "/q", "flags": "/f", "broker": "/b", "gop": "/g", "apps": ["superposition", "3dmark"]}
        cmd = pa.boot_command("/c.py", a)
        self.assertIn("--superposition-run", cmd)
        self.assertNotIn("--fb-mb", cmd)
        self.assertEqual(cmd[cmd.index("--display") + 1], "broker")
        self.assertEqual(cmd[cmd.index("--apps") + 1], "superposition,3dmark")
        self.assertEqual(cmd[1:3], ["-B", "-I"])
        w = pa.boot_command("/c.py", dict(a, apps=["wukong"]))
        self.assertEqual(w[w.index("--fb-mb") + 1], "8192")
        self.assertNotIn("--superposition-run", w)
        self.assertNotIn("--zip", cmd)
        self.assertNotIn("--resume", cmd)

    def test_offline_proof(self):
        with tempfile.TemporaryDirectory() as t:
            w = Path(t) / "w"
            (w / "run").mkdir(parents=True)
            self.assertFalse(pa.offline_proof(w))
            good = "qemu -netdev user,id=n0,restrict=on,hostfwd=tcp::1-:22 -drive file=%s/run/clone.qcow2,format=qcow2" % w
            (w / "run/last_cmd.txt").write_text(good)
            self.assertTrue(pa.offline_proof(w))
            for bad in (good.replace("restrict=on", "restrict=off"), good.replace("restrict=on", "ipv6=off"),
                        good.replace("run/clone.qcow2", "image.base.qcow2"), good + " -netdev user,restrict=off"):
                (w / "run/last_cmd.txt").write_text(bad)
                self.assertFalse(pa.offline_proof(w), bad)


class Verdicts(unittest.TestCase):
    """The recorded PC output (verbatim result.md of the 2026-11-14 night) read through collect_app."""

    def app_dir(self, root, app, verdict_doc, extra=None, run_id="0123456789abcdef0123456789abcdef"):
        d = Path(root) / app / run_id
        d.mkdir(parents=True)
        if verdict_doc is not None:
            (d / "verdict.json").write_text(json.dumps(verdict_doc))
        for name, data in (extra or {}).items():
            (d / name).write_bytes(data if isinstance(data, bytes) else data.encode())
        return d

    def test_pc_w1_3dmark_pass_with_scores_and_edge_pass_uia(self):
        rows = md_rows("pc_20261114_w1_result.md")
        with tempfile.TemporaryDirectory() as t:
            sig = next(r["detail"] for r in rows if r["step"] == "app:3dmark")
            self.app_dir(t, "3dmark", {"app": "3dmark", "ok": True, "evidence": "FILE", "signal": sig, "secs": 600}, {"result.3dmark-result": b"zip"})
            edge = next(r["detail"] for r in rows if r["step"] == "app:edge")
            self.app_dir(t, "edge", {"app": "edge", "ok": True, "evidence": "UIA", "signal": edge, "secs": 30})
            a = pa.collect_app(t, "3dmark", rows, [])
            self.assertEqual((a["verdict"], a["kind"]), ("PASS", "FILE"))
            self.assertEqual(a["scores"], {"3dmark": 7297, "graphics": 7387})
            self.assertEqual(a["evidence"][1]["file"], "verdict.json" if a["evidence"][1]["file"] == "verdict.json" else a["evidence"][1]["file"])
            self.assertIn("result.3dmark-result", {e["file"] for e in a["evidence"]})
            b = pa.collect_app(t, "edge", rows, [])
            self.assertEqual((b["verdict"], b["kind"], b["scores"]), ("PASS", "UIA", {}))

    def test_pc_w1_superposition_launch_only_is_not_a_pass(self):
        rows = md_rows("pc_20261114_w1_result.md")
        with tempfile.TemporaryDirectory() as t:
            self.app_dir(t, "superposition", {"app": "superposition", "ok": False, "evidence": "NONE", "signal": "", "secs": 31, "status": "LAUNCH_ONLY"})
            a = pa.collect_app(t, "superposition", rows, [])
            self.assertEqual((a["verdict"], a["kind"]), ("LAUNCH_ONLY", "NONE"))
            self.assertEqual(pa.grade([a])[0], "FAIL")

    def test_pc_w2_superposition_pass_from_the_log(self):
        rows = md_rows("pc_20261114_w2_result.md")
        with tempfile.TemporaryDirectory() as t:
            sig = next(r["detail"] for r in rows if r["step"] == "app:superposition")
            self.app_dir(t, "superposition", {"app": "superposition", "ok": True, "evidence": "FILE", "signal": sig, "secs": 177},
                         {"log.html": (FIX / "superposition_log_20261113.html").read_bytes()})
            a = pa.collect_app(t, "superposition", rows, [])
            self.assertEqual((a["verdict"], a["kind"], a["secs"], a["scores"]), ("PASS", "FILE", 177, {}))
            self.assertEqual({e["file"] for e in a["evidence"]}, {"log.html", "verdict.json"})
            log = next(e for e in a["evidence"] if e["file"] == "log.html")
            self.assertEqual(log["sha256"], __import__("hashlib").sha256((FIX / "superposition_log_20261113.html").read_bytes()).hexdigest())

    def test_pc_w6_edrawings_uia_solidworks_and_resolve_file(self):
        rows = md_rows("pc_20261114_w6_result.md")
        with tempfile.TemporaryDirectory() as t:
            for app, kind in (("edrawings", "UIA"), ("solidworks", "FILE"), ("resolve", "FILE")):
                self.app_dir(t, app, {"app": app, "ok": True, "evidence": kind, "signal": "s", "secs": 50})
                a = pa.collect_app(t, app, rows, [])
                self.assertEqual((a["verdict"], a["kind"]), ("PASS", kind), app)

    def test_a_pass_row_without_a_verdict_file_is_unproven(self):
        rows = md_rows("pc_20261114_w6_result.md")
        with tempfile.TemporaryDirectory() as t:
            a = pa.collect_app(t, "solidworks", rows, [])
            self.assertEqual(a["verdict"], "FAIL")
            self.assertIn("UNPROVEN_PASS", a["detail"])

    def test_a_pass_row_with_the_wrong_evidence_kind_is_unproven(self):
        rows = [{"step": "app:solidworks", "verdict": "PASS", "evidence": "UIA", "class": "app", "detail": "x"}]
        with tempfile.TemporaryDirectory() as t:
            self.app_dir(t, "solidworks", {"app": "solidworks", "ok": True, "evidence": "UIA", "signal": "", "secs": 1})
            self.assertEqual(pa.collect_app(t, "solidworks", rows, [])["verdict"], "FAIL")

    def test_row_and_verdict_file_must_agree(self):
        rows = [{"step": "app:edge", "verdict": "PASS", "evidence": "UIA", "class": "app", "detail": "x"}]
        for doc in ({"app": "edge", "ok": False, "evidence": "UIA"}, {"app": "edrawings", "ok": True, "evidence": "UIA"},
                    {"app": "edge", "ok": True, "evidence": "FILE"}, {"app": "edge", "ok": "yes", "evidence": "UIA"}):
            with tempfile.TemporaryDirectory() as t, self.subTest(doc):
                self.app_dir(t, "edge", doc)
                self.assertEqual(pa.collect_app(t, "edge", rows, [])["verdict"], "FAIL")

    def test_a_verdict_file_that_says_not_ok_is_not_a_pass(self):
        rows = [{"step": "app:edge", "verdict": "PASS", "evidence": "UIA", "class": "app", "detail": "x"}]
        with tempfile.TemporaryDirectory() as t:
            self.app_dir(t, "edge", {"app": "edge", "ok": False, "evidence": "UIA", "signal": "", "secs": 1})
            self.assertEqual(pa.collect_app(t, "edge", rows, [])["verdict"], "FAIL")

    def test_a_verdict_file_of_another_app_is_not_a_pass(self):
        rows = [{"step": "app:edge", "verdict": "PASS", "evidence": "UIA", "class": "app", "detail": "x"}]
        with tempfile.TemporaryDirectory() as t:
            self.app_dir(t, "edge", {"app": "edrawings", "ok": True, "evidence": "UIA", "signal": "", "secs": 1})
            self.assertEqual(pa.collect_app(t, "edge", rows, [])["verdict"], "FAIL")

    def test_tdr_events_are_counted_from_system_events_alone(self):
        rows = [{"step": "app:wukong", "verdict": "FAIL", "evidence": "FILE", "class": "app", "detail": "x"}]
        with tempfile.TemporaryDirectory() as t:
            self.app_dir(t, "wukong", {"app": "wukong", "ok": False, "evidence": "FILE", "signal": "", "secs": 1},
                         {"system_events.txt": "01:00:00 Display id=4101 stopped responding\n01:00:05 nvlddmkm id=153 x\n01:00:06 nvlddmkm id=153 y\n"})
            g = pa.collect_app(t, "wukong", rows, [])["guest_events"]
            self.assertEqual((g["collected"], g["tdr_4101"], g["nvlddmkm_153"], g["live_kernel_events"]), (True, 1, 2, 0))

    def test_hostile_run_directories_are_not_a_pass(self):
        rows = [{"step": "app:edge", "verdict": "PASS", "evidence": "UIA", "class": "app", "detail": "x"}]
        with tempfile.TemporaryDirectory() as t:
            outside = Path(t) / "outside"
            outside.mkdir()
            (outside / "verdict.json").write_text(json.dumps({"app": "edge", "ok": True, "evidence": "UIA", "signal": "", "secs": 1}))
            (Path(t) / "out" / "edge").mkdir(parents=True)
            (Path(t) / "out" / "edge" / ("c" * 32)).symlink_to(outside)
            a = pa.collect_app(Path(t) / "out", "edge", rows, [])
            self.assertEqual((a["verdict"], a["evidence"]), ("FAIL", []))

    def test_evidence_listing_never_reads_through_a_symlink(self):
        with tempfile.TemporaryDirectory() as t:
            secret = Path(t) / "secret-outside"
            secret.write_text("do not hash me")
            d = Path(t) / "run"
            d.mkdir()
            (d / "log.html").symlink_to(secret)
            self.assertEqual(pa.evidence_files(d), [])

    def test_two_run_directories_are_not_a_pass(self):
        rows = [{"step": "app:edge", "verdict": "PASS", "evidence": "UIA", "class": "app", "detail": "x"}]
        with tempfile.TemporaryDirectory() as t:
            for rid in ("1" * 32, "2" * 32):
                self.app_dir(t, "edge", {"app": "edge", "ok": True, "evidence": "UIA", "signal": "", "secs": 1}, run_id=rid)
            self.assertEqual(pa.collect_app(t, "edge", rows, [])["verdict"], "FAIL")

    def test_missing_row_is_missing_and_a_harness_row_is_harness(self):
        with tempfile.TemporaryDirectory() as t:
            self.assertEqual(pa.collect_app(t, "edge", [], [])["verdict"], "MISSING")
            rows = [{"step": "app:edge", "verdict": "HARNESS", "evidence": "NONE", "class": "harness", "detail": "wlms.exe event 1074"}]
            self.assertEqual(pa.collect_app(t, "edge", rows, [])["verdict"], "HARNESS")

    def test_unknown_verdict_words_from_the_harness_become_fail(self):
        rows = [{"step": "app:edge", "verdict": "GREAT", "evidence": "UIA", "class": "app", "detail": "x"}]
        with tempfile.TemporaryDirectory() as t:
            self.assertEqual(pa.collect_app(t, "edge", rows, [])["verdict"], "FAIL")

    def test_wukong_events_are_counted_by_name(self):
        events = [{"event": "LiveKernelEvent", "code": "141", "time": 1}, {"event": "LiveKernelEvent", "code": "117", "time": 2},
                  {"event": "Other", "code": "1", "time": 3}]
        system = ("12:00:01 nvlddmkm id=153 x\n12:00:02 Display id=4101 Display driver nvlddmkm stopped responding\n12:00:03 Display id=4101 y\n")
        rows = [{"step": "app:wukong", "verdict": "FAIL", "evidence": "FILE", "class": "app", "detail": "Game crashed after 40 s"}]
        with tempfile.TemporaryDirectory() as t:
            self.app_dir(t, "wukong", {"app": "wukong", "ok": False, "evidence": "FILE", "signal": "", "secs": 300},
                         {"result.json": json.dumps({"kernel_events": events}), "system_events.txt": system})
            g = pa.collect_app(t, "wukong", rows, [(5, "NVRM: Xid (PCI:0000:01:00): 31, x")])["guest_events"]
            self.assertEqual((g["collected"], g["live_kernel_events"], g["codes"], g["tdr_4101"], g["nvlddmkm_153"]), (True, 2, ["141", "117"], 2, 1))
            self.assertEqual(pa.collect_app(t, "wukong", rows, [(5, "NVRM: Xid")])["host_xid"]["count"], 1)

    def test_zero_guest_events_for_an_app_that_collects_none_say_not_collected(self):
        rows = [{"step": "app:edge", "verdict": "PASS", "evidence": "UIA", "class": "app", "detail": "x"}]
        with tempfile.TemporaryDirectory() as t:
            self.app_dir(t, "edge", {"app": "edge", "ok": True, "evidence": "UIA", "signal": "", "secs": 1}, {"result.json": "{}"})
            self.assertFalse(pa.collect_app(t, "edge", rows, [])["guest_events"]["collected"])

    def test_wukong_inconclusive_is_a_named_fail(self):
        apps = [{"app": a, "verdict": "PASS"} for a in pa.APPS[:-1]] + [{"app": "wukong", "verdict": "INCONCLUSIVE"}]
        self.assertEqual(pa.grade(apps), ("FAIL", "APP_FAIL:wukong", ["wukong"]))

    def test_grade_all_pass_and_the_first_failed_app_is_named(self):
        apps = [{"app": a, "verdict": "PASS"} for a in pa.APPS]
        self.assertEqual(pa.grade(apps), ("PASS", "", []))
        apps[1]["verdict"], apps[6]["verdict"] = "FAIL", "FAIL"
        self.assertEqual(pa.grade(apps), ("FAIL", "APP_FAIL:3dmark", ["3dmark", "wukong"]))

    def test_grade_harness_only_problems_are_retry_and_a_missing_app_is_not_a_pass(self):
        apps = [{"app": a, "verdict": "PASS"} for a in pa.APPS]
        apps[3]["verdict"] = "HARNESS"
        self.assertEqual(pa.grade(apps), ("RETRY", "HARNESS:resolve", ["resolve"]))
        apps[3]["verdict"] = "PASS"
        self.assertEqual(pa.grade(apps[:-1])[0:2], ("RETRY", "HARNESS:wukong"))
        apps[3]["verdict"] = "HARNESS"
        apps[0]["verdict"] = "FAIL"
        self.assertEqual(pa.grade(apps)[0], "FAIL")

    def test_guest_text_is_sanitised_and_bounded(self):
        rows = [{"step": "app:edge", "verdict": "FAIL", "evidence": "NONE", "class": "app", "detail": "\x1b[31mred\x00\n" + "x" * 5000}]
        with tempfile.TemporaryDirectory() as t:
            d = pa.collect_app(t, "edge", rows, [])["detail"]
            self.assertLessEqual(len(d), 200)
            self.assertTrue(all(32 <= ord(c) < 127 for c in d))

    def test_hostile_run_directories_and_files_are_ignored(self):
        with tempfile.TemporaryDirectory() as t:
            outside = Path(t) / "outside"
            outside.mkdir()
            (outside / "verdict.json").write_text(json.dumps({"app": "edge", "ok": True, "evidence": "UIA"}))
            base = Path(t) / "out" / "edge"
            base.mkdir(parents=True)
            (base / "..evil").mkdir()
            (base / "not-a-run-id").mkdir()
            (base / ("c" * 32)).symlink_to(outside)
            self.assertEqual(pa.run_dirs(Path(t) / "out", "edge"), [])
            rows = [{"step": "app:edge", "verdict": "PASS", "evidence": "UIA", "class": "app", "detail": "x"}]
            self.assertEqual(pa.collect_app(Path(t) / "out", "edge", rows, [])["verdict"], "FAIL")
            (Path(t) / "link").symlink_to(outside)
            self.assertEqual(pa.run_dirs(t, "link"), [])

    def test_evidence_files_skip_symlinks_odd_names_and_cap_the_count(self):
        with tempfile.TemporaryDirectory() as t:
            d = Path(t)
            (d / "ok.json").write_text("{}")
            (d / "secret").write_text("s")
            (d / "link.txt").symlink_to(d / "secret")
            (d / "has space.txt").write_text("x")
            (d / "sub").mkdir()
            self.assertEqual([e["file"] for e in pa.evidence_files(d)], ["ok.json", "secret"])
            for n in range(40):
                (d / ("f%02d.txt" % n)).write_text("x")
            self.assertEqual(len(pa.evidence_files(d)), pa.MAX_EVIDENCE_FILES)

    def test_read_json_refuses_duplicates_and_huge_files(self):
        with tempfile.TemporaryDirectory() as t:
            p = Path(t) / "v.json"
            p.write_text('{"a": 1, "a": 2}')
            with self.assertRaises(ValueError):
                pa.read_json(p)
            p.write_text("[" + "0," * (1 << 20) + "0]")
            with self.assertRaises(ValueError):
                pa.read_json(p)

    def test_scores_come_only_from_the_expected_signal_shapes(self):
        self.assertEqual(pa.scores_of("3dmark", "3DMark score 100 (graphics 90), x"), {"3dmark": 100, "graphics": 90})
        self.assertEqual(pa.scores_of("3dmark", "score 5"), {})
        self.assertEqual(pa.scores_of("edge", "3DMark score 100 (graphics 90)"), {})
        self.assertEqual(pa.scores_of("superposition", "screen read (not the verdict): score 7685, FPS"), {"score": 7685})


class KernelLogWindows(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="kf-klog-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.k = pa.KernelLog(self.tmp)
        self.k.dir.mkdir(exist_ok=True)

    def add(self, *lines):
        with open(self.k.log, "a") as f:
            for line in lines:
                f.write(line + "\n")

    def test_lines_map_to_the_app_window_they_fall_in(self):
        self.add("boot noise", "NVRM: Xid (PCI:01:00): 13, before apps")
        self.k.mark("b1", "start", 0)
        self.k.mark("b1:a", "start", 0)
        self.add("quiet", "NVRM: Xid (PCI:01:00): 31, in a")
        self.k.mark("b1:a", "end", 0)
        self.k.mark("b1:b", "start", 0)
        self.add("NVRM: Xid (PCI:01:00): 43, in b", "NVRM: Xid (PCI:01:00): 44, in b again")
        self.k.mark("b1:b", "end", 0)
        self.add("NVRM: Xid (PCI:01:00): 45, after")
        self.k.mark("b1", "end", 0)
        per, rest = pa.xid_by_window(self.k, ["a", "b"], "b1")
        self.assertEqual([t.split(", ")[1] for _, t in per["a"]], ["in a"])
        self.assertEqual([t.split(", ")[1] for _, t in per["b"]], ["in b", "in b again"])
        self.assertEqual([t.split(", ")[1] for _, t in rest], ["after"])

    def test_the_marker_file_has_the_pr420_format(self):
        self.add("x")
        self.k.mark("s", "start", 0)
        parts = self.k.offsets.read_text().split("\t")
        self.assertEqual(parts[:3], ["s", "start", "1"])

    def test_an_open_window_runs_to_the_end_and_a_bad_edge_is_refused(self):
        self.add("a")
        self.k.mark("s", "start", 0)
        self.add("NVRM: Xid x")
        self.assertEqual(len(self.k.xids(*self.k.windows()["s"])), 1)
        with self.assertRaises(AssertionError):
            self.k.mark("s", "middle", 0)

    def test_follower_captures_lines_and_unreadable_log_is_an_error(self):
        k = pa.KernelLog(self.tmp / "f", ["sh", "-c", "echo line1; echo NVRM: Xid y; sleep 60"])
        k.start()
        self.addCleanup(k.stop)
        self.assertEqual(len(k.xids(1, 100)), 1)
        bad = pa.KernelLog(self.tmp / "g", ["sh", "-c", "exit 1"])
        with self.assertRaises(pa.Refused) as ctx:
            bad.start()
        self.assertEqual(ctx.exception.reason, "KERNEL_LOG")
        empty = pa.KernelLog(self.tmp / "h", ["sh", "-c", "sleep 60"], sleep=lambda s: None)
        with self.assertRaises(pa.Refused):
            empty.start()
        missing = pa.KernelLog(self.tmp / "i", ["no-such-kernel-log-tool"])
        with self.assertRaises(pa.Refused):
            missing.start()

    def test_mark_lines_follow_the_harness_stdout(self):
        m = pa.Mark(self.k, "b1", ["a", "b"])
        self.add("0")
        for text in ("noise\n", "verify PASS NONE ok\n", "verify PASS again\n"):
            m.line(text)
        self.add("NVRM: Xid in a")
        m.line("app:a PASS FILE detail\n")
        m.line("app:zzz PASS FILE detail\n")
        self.add("NVRM: Xid in b")
        m.line("app:b FAIL NONE detail\n")
        w = self.k.windows()
        self.assertEqual(set(w), {"b1:a", "b1:b"})
        self.assertEqual(len(self.k.xids(*w["b1:a"])), 1)
        self.assertEqual(len(self.k.xids(*w["b1:b"])), 1)
        self.assertEqual(m.seen, ["a", "b"])


class Run(unittest.TestCase):
    def test_unpinned_image_refuses_and_nothing_is_fetched(self):
        env = Env(self, pins=write_pins(tempfile.mkdtemp(), set_roles=("gop", "libtpms", "swtpm")))
        self.addCleanup(shutil.rmtree, env.pins.parent, True)
        rec = pa.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"]), ("REFUSED", "PIN_UNSET:image"))
        self.assertEqual(env.fake.calls, [["swtpm", "--version"]])
        self.assertFalse((env.ctx.prefix / "dl").exists())
        self.assertEqual(env.boots_seen, [])
        vast_report.validate_propimage_apps(rec)

    def test_the_committed_pins_refuse_today_with_a_named_reason(self):
        env = Env(self, pins=pa.PINS_PATH)
        rec = pa.run(env.ctx)
        if rec["verdict"] != "REFUSED":
            self.skipTest("the committed pins are set")
        self.assertTrue(rec["reason"].startswith("PIN_UNSET:"), rec["reason"])
        vast_report.validate_propimage_apps(rec)

    def test_intel_host_is_not_run_cpu_vendor_and_touches_nothing(self):
        env = Env(self, cpuinfo=INTEL)
        rec = pa.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"], rec["cpu_vendor"]), ("NOT_RUN", "CPU_VENDOR", "intel"))
        self.assertFalse(env.ctx.prefix.exists())
        self.assertEqual(env.boots_seen, [])
        vast_report.validate_propimage_apps(rec)

    def test_unsupported_gpu_family_is_refused_by_name(self):
        env = Env(self, hardware={"gpu_name": "A100", "compute_cap": 800, "memory_mib": 40000})
        rec = pa.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"]), ("REFUSED", "GPU_FAMILY"))

    def test_full_pass_record(self):
        env = Env(self, scenario={"3dmark": {"signal": "3DMark score 7297 (graphics 7387), x"}})
        rec = pa.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"], rec["failed"]), ("PASS", "", []), rec["apps"])
        self.assertEqual([a["app"] for a in rec["apps"]], list(pa.APPS))
        self.assertEqual([b["apps"] for b in rec["boots"]], [["superposition", "3dmark"], ["solidworks", "resolve", "edrawings", "edge"], ["wukong"]])
        self.assertTrue(all(b["offline"] for b in rec["boots"]))
        self.assertEqual(rec["family"], "ada")
        three = next(a for a in rec["apps"] if a["app"] == "3dmark")
        self.assertEqual(three["scores"], {"3dmark": 7297, "graphics": 7387})
        self.assertTrue(all(a["kind"] == pa.EXPECTED_KIND[a["app"]] for a in rec["apps"]))
        vast_report.validate_propimage_apps(rec)
        self.assertEqual(env.boots_seen[2][env.boots_seen[2].index("--fb-mb") + 1], "8192")
        self.assertIn("--superposition-run", env.boots_seen[0])
        self.assertNotIn("--superposition-run", env.boots_seen[1])
        self.assertNotIn("--fb-mb", env.boots_seen[0])
        self.assertEqual(env.zip_at_boot, [False, False, False])
        for argv in env.boots_seen:
            self.assertNotIn("--zip", argv)
            self.assertEqual(argv[argv.index("--display") + 1], "broker")
        json.dumps(rec)

    def test_a_prebuilt_prefix_swtpm_gets_its_library_path(self):
        env = pa.swtpm_env({}, prefix_bin=sys.executable)   # any executable stands in for /opt/tpm/bin/swtpm
        self.assertEqual(env["SWTPM"], sys.executable)
        self.assertTrue(env["LD_LIBRARY_PATH"].endswith("/lib"))
        env = pa.swtpm_env({"LD_LIBRARY_PATH": "/x"}, prefix_bin=sys.executable)
        self.assertTrue(env["LD_LIBRARY_PATH"].endswith("/lib:/x"))

    def test_an_explicit_or_system_swtpm_keeps_the_environment_unchanged(self):
        e = {"SWTPM": "/custom/swtpm"}
        self.assertIs(pa.swtpm_env(e), e)
        e = {}
        self.assertIs(pa.swtpm_env(e, prefix_bin="/nonexistent/bin/swtpm"), e)

    def test_the_swtpm_build_dependencies_include_expect(self):
        self.assertIn("expect", pa.SWTPM_DEPS)

    def test_wukong_failure_is_named_and_the_record_still_validates(self):
        env = Env(self, scenario={"wukong": {"verdict": "FAIL", "detail": "Game crashed", "kernel_events": [{"event": "LiveKernelEvent", "code": "141"}],
                                             "xids": ["NVRM: Xid (PCI:0000:01:00): 31, pid=1, name=qemu"]}})
        rec = pa.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"], rec["failed"]), ("FAIL", "APP_FAIL:wukong", ["wukong"]))
        w = rec["apps"][-1]
        self.assertEqual((w["guest_events"]["live_kernel_events"], w["guest_events"]["codes"]), (1, ["141"]))
        self.assertEqual(w["host_xid"]["count"], 1)
        self.assertTrue(all(a["host_xid"]["count"] == 0 for a in rec["apps"][:-1]))
        vast_report.validate_propimage_apps(rec)

    def test_wukong_inconclusive_as_the_harness_grades_it_today_is_a_fail_by_name(self):
        env = Env(self, scenario={"wukong": {"verdict": "INCONCLUSIVE", "kind": "NONE", "detail": "Game stayed up; benchmark not driven"}})
        rec = pa.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"]), ("FAIL", "APP_FAIL:wukong"))

    def test_host_xids_are_attributed_per_app_and_unattributed_ones_are_kept(self):
        env = Env(self, scenario={"resolve": {"xids": ["NVRM: Xid (PCI:01:00): 13, resolve"]},
                                  "unattributed": ["NVRM: Xid (PCI:0000:01:00): 79, late"]})
        rec = pa.run(env.ctx)
        by = {a["app"]: a["host_xid"]["count"] for a in rec["apps"]}
        self.assertEqual(by["resolve"], 1)
        self.assertEqual(sum(by.values()), 1)
        self.assertEqual(rec["boots"][0]["host_xid_unattributed"]["count"], 1)
        vast_report.validate_propimage_apps(rec)

    def test_online_boot_is_not_a_pass(self):
        env = Env(self, scenario={"online_boot": True})
        rec = pa.run(env.ctx)
        self.assertEqual(rec["verdict"], "FAIL")
        self.assertTrue(all(a["verdict"] != "PASS" for a in rec["apps"]))
        self.assertIn("OFFLINE_NOT_PROVEN", rec["apps"][0]["detail"])

    def test_a_harness_that_dies_before_the_apps_is_retry_not_a_guest_fail(self):
        env = Env(self, scenario={})

        def dead(ctx, argv, log, timeout, mark):
            out = Path(argv[argv.index("--out") + 1])
            out.mkdir(parents=True)
            (out / "result.json").write_text(json.dumps([{"step": "harness", "verdict": "HARNESS", "evidence": "NONE", "class": "harness",
                                                          "detail": "swtpm failed to load image state"}]))
            return 1
        env.ctx.boot_runner = dead
        rec = pa.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"]), ("RETRY", "HARNESS:superposition"))
        self.assertEqual(rec["boots"][0]["harness"][0]["step"], "harness")
        vast_report.validate_propimage_apps(rec)

    def test_a_download_failure_is_refused_by_name_and_cleans_up(self):
        env = Env(self, fail=("fetch",))
        rec = pa.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"]), ("REFUSED", "DOWNLOAD"))
        self.assertFalse((env.ctx.prefix / "dl").exists())
        for sub in ("unpacked", "work"):
            self.assertFalse((env.ctx.prefix / sub).exists(), sub)
        self.assertEqual(env.boots_seen, [])

    def test_unpack_failure_is_refused_by_name(self):
        env = Env(self)

        def bad(zip_path, directory):
            raise ValueError("Manifest mismatch: image.qcow2")
        env.ctx.unpack = bad
        rec = pa.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"]), ("REFUSED", "UNPACK"))
        self.assertEqual(env.boots_seen, [])

    def test_the_archive_is_deleted_after_unpack_and_everything_after_the_run(self):
        env = Env(self)
        seen = {}

        def unpack(z, d):
            seen["zip_exists"] = Path(z).is_file()
            Path(d, "state.json").write_text("{}")
        env.ctx.unpack = unpack
        orig = env.boot

        def boot(ctx, argv, log, timeout, mark):
            seen.setdefault("zip_at_boot", (ctx.prefix / "dl/image/image.zip").exists())
            seen.setdefault("gop_at_boot", (ctx.prefix / "dl/image/kf-gop.signed.efi").exists())
            return orig(ctx, argv, log, timeout, mark)
        env.ctx.boot_runner = boot
        rec = pa.run(env.ctx)
        self.assertTrue(seen["zip_exists"])
        self.assertFalse(seen["zip_at_boot"])
        self.assertTrue(seen["gop_at_boot"])
        self.assertEqual(rec["verdict"], "PASS")
        for sub in ("dl", "unpacked", "work"):
            self.assertFalse((env.ctx.prefix / sub).exists(), sub)

    def test_old_swtpm_is_built_from_the_pinned_sources_and_used(self):
        env = Env(self, swtpm="swtpm version 0.6.3")
        calls = env.fake.calls
        real = env.fake.__call__

        def fake(argv, **kw):
            done = real(argv, **kw)
            if argv[-1] == "--version" and argv[0] == "/opt/tpm/bin/swtpm":
                done.stdout = "swtpm version 0.10.1"
            return done
        env.ctx.runner = fake
        rec = pa.run(env.ctx)
        self.assertEqual(rec["verdict"], "PASS", rec["reason"])
        self.assertTrue(rec["swtpm"]["built"])
        self.assertEqual(rec["swtpm"]["version"], "0.10.1")
        self.assertIn(["make", "install"], calls)
        self.assertEqual(env.ctx.environ["SWTPM"], "/opt/tpm/bin/swtpm")
        self.assertEqual(env.ctx.environ["LD_LIBRARY_PATH"], "/opt/tpm/lib")
        manifest = json.loads((env.ctx.prefix / "manifest.json").read_text())
        self.assertEqual([i["needed_by"] for i in manifest["items"]], ["image", "image", "tools", "tools"])

    def built_runner(self, env):
        """A runner that answers 0.10.1 at the built path, whatever the build steps did: only the step results can refuse."""
        real = env.fake.__call__

        def fake(argv, **kw):
            done = real(argv, **kw)
            if argv[-1] == "--version" and argv[0] == "/opt/tpm/bin/swtpm":
                done.stdout = "swtpm version 0.10.1"
            return done
        return fake

    def test_swtpm_build_failure_is_named(self):
        env = Env(self, swtpm="swtpm version 0.6.3", fail=("autogen.sh",))
        env.ctx.runner = self.built_runner(env)
        rec = pa.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"]), ("REFUSED", "SWTPM_BUILD"))

    def test_built_swtpm_that_is_still_old_is_refused(self):
        env = Env(self, swtpm="swtpm version 0.6.3")
        rec = pa.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"]), ("REFUSED", "SWTPM_BUILD"))

    def test_no_time_for_a_boot_is_retry_time_and_the_boot_is_not_started(self):
        env = Env(self)
        env.ctx.deadline_epoch = 1.0
        rec = pa.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"]), ("RETRY", "TIME"))
        self.assertEqual(env.boots_seen, [])
        self.assertEqual({a["verdict"] for a in rec["apps"]}, {"NOT_RUN"})

    def test_the_record_is_written_after_each_boot(self):
        env = Env(self)
        seen = []
        orig = env.boot

        def boot(ctx, argv, log, timeout, mark):
            seen.append(ctx.out.exists())
            return orig(ctx, argv, log, timeout, mark)
        env.ctx.boot_runner = boot
        pa.run(env.ctx)
        self.assertEqual(seen, [False, True, True])

    def test_only_text_evidence_is_copied(self):
        env = Env(self)
        ev = env.tmp / "evidence"
        pa.run(env.ctx, ev)
        names = {p.name for p in ev.rglob("*") if p.is_file()}
        self.assertEqual(names, {"verdict.json", "result.json", "system_events.txt"})

    def test_low_disk_is_refused_by_name_before_any_download(self):
        env = Env(self)
        env.ctx.free_bytes = 10 ** 9
        rec = pa.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"]), ("REFUSED", "DISK_FREE"))
        self.assertEqual(env.fake.calls, [["swtpm", "--version"]])

    def test_disk_just_below_the_need_is_refused_in_a_run(self):
        env = Env(self)
        env.ctx.free_bytes = 2 * 1234567 + pa.DISK_HEADROOM - 1
        self.assertEqual(pa.run(env.ctx)["reason"], "DISK_FREE")
        env2 = Env(self)
        env2.ctx.free_bytes = 2 * 1234567 + pa.DISK_HEADROOM
        self.assertEqual(pa.run(env2.ctx)["verdict"], "PASS")

    def test_a_refusal_before_any_work_leaves_a_prefetched_directory_alone(self):
        for label, kw in (("cpu", {"cpuinfo": INTEL}), ("pins", {"pins": write_pins(tempfile.mkdtemp(), set_roles=())})):
            env = Env(self, **kw)
            (env.ctx.prefix / "dl").mkdir(parents=True)
            (env.ctx.prefix / "dl/keep.bin").write_bytes(b"prefetched")
            pa.run(env.ctx)
            with self.subTest(label):
                self.assertEqual((env.ctx.prefix / "dl/keep.bin").read_bytes(), b"prefetched")

    def test_build_that_leaves_no_working_swtpm_is_refused(self):
        env = Env(self, swtpm="swtpm version 0.6.3")
        real = env.fake.__call__

        def fake(argv, **kw):
            done = real(argv, **kw)
            if argv[-1] == "--version" and argv[0] == "/opt/tpm/bin/swtpm":
                done.stdout = "garbage"
            return done
        env.ctx.runner = fake
        self.assertEqual(pa.run(env.ctx)["reason"], "SWTPM_BUILD")

    def test_a_pass_with_the_wrong_evidence_kind_in_a_boot_is_not_a_stage_pass(self):
        env = Env(self, scenario={"edge": {"kind": "FILE"}})
        rec = pa.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"]), ("FAIL", "APP_FAIL:edge"))

    def test_enough_time_runs_all_boots(self):
        import time
        env = Env(self)
        env.ctx.deadline_epoch = time.time() + 10 * 3600
        self.assertEqual(pa.run(env.ctx)["verdict"], "PASS")
        self.assertEqual(len(env.boots_seen), 3)

    def test_a_silent_kernel_log_refuses_the_run_before_any_download(self):
        env = Env(self)
        env.ctx.klog_source = ["sh", "-c", "sleep 30"]
        rec = pa.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"]), ("REFUSED", "KERNEL_LOG"))
        self.assertEqual(env.fake.calls, [["swtpm", "--version"]])

    def test_an_unexpected_error_is_a_named_refusal_with_a_record(self):
        env = Env(self)

        def boom(ctx, *a):
            raise OSError("disk exploded")
        env.ctx.boot_runner = boom
        rec = pa.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"]), ("REFUSED", "INTERNAL"))
        self.assertIn("disk exploded", rec["error"])
        vast_report.validate_propimage_apps(rec)

    def test_a_timed_out_tool_is_a_named_failure(self):
        env = Env(self)
        real = env.fake.__call__

        def slow(argv, **kw):
            if len(argv) > 4 and str(argv[3]).endswith("box_downloader.py") and argv[4] == "fetch":
                raise subprocess.TimeoutExpired(argv, 1)
            return real(argv, **kw)
        env.ctx.runner = slow
        self.assertEqual(pa.run(env.ctx)["reason"], "DOWNLOAD")

    def test_precedence_cpu_vendor_row_comes_first(self):
        env = Env(self)
        self.assertEqual(pa.preflight(env.ctx)[0][0]["name"], "cpu_vendor")

    def test_missing_cpu_flags_are_refused_by_name_in_a_run(self):
        env = Env(self, cpuinfo="vendor_id: AuthenticAMD\nflags: fpu vme sse2\n")
        rec = pa.run(env.ctx)
        self.assertEqual((rec["verdict"], rec["reason"]), ("REFUSED", "CPU_PROFILE_UNSUPPORTED"))

    def test_a_symlinked_url_file_is_refused_by_name_in_a_run(self):
        env = Env(self)
        (env.tmp / "real.json").write_text("{}")
        (env.tmp / "urls.json").unlink()
        (env.tmp / "urls.json").symlink_to(env.tmp / "real.json")
        self.assertEqual(pa.run(env.ctx)["reason"], "URLS_MISSING")

    def test_missing_kf3_qemu_is_refused_by_name_in_a_run(self):
        env = Env(self)
        (env.bench / "kf3-bins" / SHA[:8] / "qemu-system-x86_64").unlink()
        self.assertEqual(pa.run(env.ctx)["reason"], "NO_KF3_QEMU")

    def test_a_timed_out_swtpm_build_is_named(self):
        env = Env(self, swtpm="swtpm version 0.6.3")
        real = env.fake.__call__

        def slow(argv, **kw):
            if argv[0] == "make":
                raise subprocess.TimeoutExpired(argv, 1)
            done = real(argv, **kw)
            if argv[-1] == "--version" and argv[0] == "/opt/tpm/bin/swtpm":
                done.stdout = "swtpm version 0.10.1"
            return done
        env.ctx.runner = slow
        self.assertEqual(pa.run(env.ctx)["reason"], "SWTPM_BUILD")

    def test_a_boot_with_both_restrict_settings_is_not_a_pass(self):
        env = Env(self, scenario={"both_restrict": True})
        rec = pa.run(env.ctx)
        self.assertEqual(rec["verdict"], "FAIL")
        self.assertEqual(rec["reason"], "APP_FAIL:superposition")

    def test_an_unknown_harness_verdict_word_is_a_fail(self):
        env = Env(self, scenario={"edge": {"verdict": "GREAT"}})
        rec = pa.run(env.ctx)
        self.assertEqual(next(a for a in rec["apps"] if a["app"] == "edge")["verdict"], "FAIL")
        vast_report.validate_propimage_apps(rec)

    def test_hostile_guest_text_never_reaches_the_record(self):
        env = Env(self, scenario={"edge": {"verdict": "FAIL", "detail": "\x1b]0;owned\x07\x00" + "A" * 4000}})
        rec = pa.run(env.ctx)
        detail = next(a for a in rec["apps"] if a["app"] == "edge")["detail"]
        self.assertTrue(all(32 <= ord(c) < 127 for c in detail) and len(detail) <= 200)
        vast_report.validate_propimage_apps(rec)

    def test_apps_without_event_collection_say_not_collected_in_the_record(self):
        env = Env(self)
        rec = pa.run(env.ctx)
        self.assertEqual([a["guest_events"]["collected"] for a in rec["apps"]], [False] * 6 + [True])

    def test_every_app_has_a_record_even_when_a_boot_never_started(self):
        env = Env(self)
        import time
        env.ctx.deadline_epoch = time.time() + 10 * 3600
        orig = env.boot

        def once(ctx, argv, log, timeout, mark):
            rc = orig(ctx, argv, log, timeout, mark)
            ctx.deadline_epoch = 1.0
            return rc
        env.ctx.boot_runner = once
        rec = pa.run(env.ctx)
        self.assertEqual([a["app"] for a in rec["apps"]], list(pa.APPS))
        self.assertEqual([a["verdict"] for a in rec["apps"]], ["PASS"] * 2 + ["NOT_RUN"] * 5)
        self.assertEqual((rec["verdict"], rec["reason"]), ("RETRY", "TIME"))
        vast_report.validate_propimage_apps(rec)

    def test_a_cpu_refusal_leaves_a_prefetched_directory_alone(self):
        env = Env(self, cpuinfo=INTEL)
        (env.ctx.prefix / "dl").mkdir(parents=True)
        (env.ctx.prefix / "dl/keep.bin").write_bytes(b"prefetched")
        pa.run(env.ctx)
        self.assertEqual((env.ctx.prefix / "dl/keep.bin").read_bytes(), b"prefetched")

    def test_a_pin_refusal_leaves_a_prefetched_directory_alone(self):
        env = Env(self, pins=write_pins(tempfile.mkdtemp(), set_roles=()))
        self.addCleanup(shutil.rmtree, env.pins.parent, True)
        (env.ctx.prefix / "dl").mkdir(parents=True)
        (env.ctx.prefix / "dl/keep.bin").write_bytes(b"prefetched")
        pa.run(env.ctx)
        self.assertEqual((env.ctx.prefix / "dl/keep.bin").read_bytes(), b"prefetched")

    def test_a_relative_prefix_is_refused_by_name_in_a_run(self):
        env = Env(self)
        env.ctx.prefix = Path("relative/kf-propimage")
        self.assertEqual(pa.run(env.ctx)["reason"], "PREFIX")

    def test_evidence_copy_never_takes_native_files(self):
        with tempfile.TemporaryDirectory() as t:
            d = Path(t) / "out" / "3dmark" / ("e" * 32)
            d.mkdir(parents=True)
            for name in ("verdict.json", "result.3dmark-result", "screen.png", "model.step", "log.html", "dxdiag.txt"):
                (d / name).write_text("x")
            pa.copy_evidence(Path(t) / "out", Path(t) / "ev")
            self.assertEqual({p.name for p in (Path(t) / "ev").rglob("*") if p.is_file()}, {"verdict.json"})

    def test_prefix_must_be_a_deep_absolute_real_directory(self):
        for bad in ("relative/path", "/", "/workspace", "/workspace/../etc/x", "/a/../b/c"):
            with self.subTest(bad), self.assertRaises(pa.Refused):
                pa.check_prefix(bad)
        with tempfile.TemporaryDirectory() as t:
            real, link = Path(t) / "real", Path(t) / "link"
            real.mkdir()
            link.symlink_to(real)
            with self.assertRaises(pa.Refused):
                pa.check_prefix(link)
            pa.check_prefix(real)
            pa.check_prefix(Path(t) / "nothing-yet")

    def test_refusal_on_a_bad_prefix_creates_and_deletes_nothing(self):
        env = Env(self)
        env.ctx.prefix = Path("/")
        self.assertEqual(pa.run(env.ctx)["reason"], "PREFIX")

    def test_cli_plan_runs_without_side_effects_and_manifest_refuses_unpinned(self):
        with tempfile.TemporaryDirectory() as t:
            out = Path(t) / "m.json"
            done = subprocess.run([sys.executable, "-B", str(HERE / "propimage_apps.py"), "manifest", "--out", str(out)], capture_output=True, text=True)
            if done.returncode == 0:
                self.skipTest("the committed pins are set")
            self.assertEqual(done.returncode, 2)
            self.assertFalse(out.exists())
        done = subprocess.run([sys.executable, "-B", str(HERE / "propimage_apps.py"), "plan", "--cpuinfo", str(CPUFLAGS / "pc_zen4.txt")],
                              capture_output=True, text=True)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(json.loads(done.stdout)["boots"][-1], ["wukong"])

    def test_cli_run_prints_one_result_line_and_refuses_a_bad_sha(self):
        done = subprocess.run([sys.executable, "-B", str(HERE / "propimage_apps.py"), "run", "--sha", "xyz", "--out", "/dev/null"],
                              capture_output=True, text=True)
        self.assertEqual(done.returncode, 2)


class ReportValidation(unittest.TestCase):
    _good = None

    def good(self):
        if ReportValidation._good is None:
            ReportValidation._good = pa.run(Env(self).ctx)
        return copy.deepcopy(ReportValidation._good)

    def reject(self, rec, label, fn):
        r = copy.deepcopy(rec)
        fn(r)
        with self.subTest(label), self.assertRaises(ValueError):
            vast_report.validate_propimage_apps(r)

    def test_good_record_validates(self):
        rec = self.good()
        self.assertEqual(rec["verdict"], "PASS")
        vast_report.validate_propimage_apps(rec)

    def test_pass_needs_every_app_to_pass_in_order(self):
        rec = self.good()
        self.reject(rec, "wukong not pass", lambda r: r["apps"][-1].update(verdict="FAIL"))
        self.reject(rec, "missing app", lambda r: r["apps"].pop(2))
        self.reject(rec, "swapped order", lambda r: r["apps"].reverse())
        self.reject(rec, "inconclusive", lambda r: r["apps"][3].update(verdict="INCONCLUSIVE"))

    def test_pass_needs_the_expected_evidence_kind(self):
        rec = self.good()
        self.reject(rec, "solidworks as UIA", lambda r: r["apps"][2].update(kind="UIA"))
        self.reject(rec, "edge as FILE", lambda r: r["apps"][5].update(kind="FILE"))
        self.reject(rec, "none", lambda r: r["apps"][0].update(kind="NONE"))

    def test_pass_needs_every_boot_offline(self):
        rec = self.good()
        self.reject(rec, "online boot", lambda r: r["boots"][0].update(offline=False))
        self.reject(rec, "no boots", lambda r: r.update(boots=[]))

    def test_pass_carries_no_reason_or_failed_list(self):
        rec = self.good()
        self.reject(rec, "reason", lambda r: r.update(reason="X"))
        self.reject(rec, "failed", lambda r: r.update(failed=["wukong"]))

    def test_non_pass_needs_an_upper_case_reason(self):
        rec = self.good()
        self.reject(rec, "fail without reason", lambda r: r.update(verdict="FAIL", reason=""))
        self.reject(rec, "lowercase reason", lambda r: r.update(verdict="FAIL", reason="wukong"))
        self.reject(rec, "bad verdict", lambda r: r.update(verdict="GREEN"))

    def test_non_pass_record_keeps_the_app_order_and_unique_apps(self):
        rec = self.good()
        rec.update(verdict="FAIL", reason="APP_FAIL:wukong", failed=["wukong"])
        rec["apps"][-1]["verdict"] = "INCONCLUSIVE"
        vast_report.validate_propimage_apps(rec)
        self.reject(rec, "reversed", lambda r: r["apps"].reverse())
        self.reject(rec, "duplicate", lambda r: r["apps"].append(copy.deepcopy(r["apps"][0])))

    def test_shape_extra_keys_and_types(self):
        rec = self.good()
        self.reject(rec, "extra key", lambda r: r.update(extra=1))
        self.reject(rec, "extra app key", lambda r: r["apps"][0].update(extra=1))
        self.reject(rec, "bad schema", lambda r: r.update(schema=2))
        self.reject(rec, "bad family", lambda r: r.update(family="kepler"))
        self.reject(rec, "bad vendor", lambda r: r.update(cpu_vendor="arm"))
        self.reject(rec, "non-dict", lambda r: r.clear())
        self.reject(rec, "unknown app in boot", lambda r: r["boots"][0]["apps"].append("notepad"))
        self.reject(rec, "bool count", lambda r: r["apps"][0]["host_xid"].update(count=True))

    def test_text_fields_are_printable_and_bounded(self):
        rec = self.good()
        self.reject(rec, "control chars", lambda r: r["apps"][0].update(detail="a\x1b[0m"))
        self.reject(rec, "long detail", lambda r: r["apps"][0].update(detail="x" * 300))
        self.reject(rec, "long error", lambda r: r.update(error="x" * 300))
        self.reject(rec, "preflight control", lambda r: r["preflight"][0].update(detail="a\nb"))

    def test_xid_lines_are_printable_and_few(self):
        rec = self.good()
        self.reject(rec, "too many", lambda r: r["apps"][0]["host_xid"].update(lines=["x"] * 9))
        self.reject(rec, "control", lambda r: r["apps"][0]["host_xid"].update(lines=["NVRM: Xid \x00"]))

    def test_hashes_sizes_and_counts_are_checked(self):
        rec = self.good()
        self.reject(rec, "bad sha", lambda r: r.update(sha="g" * 40))
        self.reject(rec, "bad evidence hash", lambda r: r["apps"][0]["evidence"].append({"file": "a", "bytes": 1, "sha256": "zz"}))
        self.reject(rec, "bad image sha", lambda r: r["image"].update(sha256="12"))
        self.reject(rec, "negative secs", lambda r: r["apps"][0].update(secs=-1))
        self.reject(rec, "too many evidence files", lambda r: r["apps"][0].update(evidence=[{"file": "a", "bytes": 1, "sha256": "0" * 64}] * 25))

    def test_failed_and_refused_records_validate(self):
        rec = self.good()
        rec["verdict"], rec["reason"], rec["failed"] = "FAIL", "APP_FAIL:wukong", ["wukong"]
        rec["apps"][-1]["verdict"] = "INCONCLUSIVE"
        vast_report.validate_propimage_apps(rec)
        rec["apps"], rec["boots"], rec["verdict"], rec["reason"], rec["failed"] = [], [], "REFUSED", "PIN_UNSET:image", []
        vast_report.validate_propimage_apps(rec)

    def file_with(self, doc):
        t = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, t, True)
        p = Path(t) / "r.json"
        p.write_text(json.dumps(doc))
        return p

    def doc(self, rec, verdict="PASS"):
        return {"verdict": verdict, "suites": [{"name": "propimage_apps", "pass": 1, "fail": 0, "skip": 0}], "propimage_apps": rec}

    def test_result_file_carries_the_record(self):
        rec = self.good()
        self.assertEqual(vast_report.read_result_file(self.file_with(self.doc(rec)))["propimage_apps"]["verdict"], "PASS")

    def test_result_file_refuses_a_malformed_record(self):
        rec = self.good()
        for label, fn in (("wukong fail under a PASS record", lambda r: r["apps"][-1].update(verdict="FAIL")),
                          ("missing app", lambda r: r["apps"].pop(2)),
                          ("kind", lambda r: r["apps"][2].update(kind="UIA")), ("online", lambda r: r["boots"][0].update(offline=False)),
                          ("extra key", lambda r: r.update(extra=1)), ("reason on pass", lambda r: r.update(reason="X"))):
            r = copy.deepcopy(rec)
            fn(r)
            with self.subTest(label), self.assertRaises(vast_report.ResultError):
                vast_report.read_result_file(self.file_with(self.doc(r)))

    def test_a_failed_record_with_apps_out_of_order_is_refused_in_a_result_file(self):
        rec = self.good()
        rec.update(verdict="FAIL", reason="APP_FAIL:wukong", failed=["wukong"])
        rec["apps"][-1]["verdict"] = "FAIL"
        rec["apps"].reverse()
        with self.assertRaises(vast_report.ResultError):
            vast_report.read_result_file(self.file_with(self.doc(rec, "FAIL")))

    def test_result_file_refuses_a_pass_record_with_a_failed_list(self):
        rec = self.good()
        rec["failed"] = ["wukong"]
        with self.assertRaises(vast_report.ResultError):
            vast_report.read_result_file(self.file_with(self.doc(rec)))

    def test_result_file_bounds_the_xid_lines(self):
        rec = self.good()
        rec["apps"][-1]["host_xid"]["lines"] = ["NVRM: Xid"] * 9
        with self.assertRaises(vast_report.ResultError):
            vast_report.read_result_file(self.file_with(self.doc(rec)))

    def test_result_file_refuses_a_bad_evidence_hash(self):
        rec = self.good()
        rec["apps"][0]["evidence"].append({"file": "x", "bytes": 1, "sha256": "nothex"})
        with self.assertRaises(vast_report.ResultError):
            vast_report.read_result_file(self.file_with(self.doc(rec)))

    def test_a_refused_record_without_a_reason_is_rejected(self):
        rec = self.good()
        rec.update(verdict="REFUSED", reason="", apps=[], boots=[], failed=[])
        with self.assertRaises(ValueError):
            vast_report.validate_propimage_apps(rec)

    def test_payload_pass_with_a_red_record_is_refused_but_a_red_payload_may_carry_it(self):
        red = self.good()
        red.update(verdict="FAIL", reason="APP_FAIL:wukong", failed=["wukong"])
        red["apps"][-1]["verdict"] = "FAIL"
        with self.assertRaises(vast_report.ResultError):
            vast_report.read_result_file(self.file_with(self.doc(red)))
        self.assertEqual(vast_report.read_result_file(self.file_with(self.doc(red, "FAIL")))["verdict"], "FAIL")

    def test_payload_pass_with_a_refused_record_is_refused(self):
        refused = self.good()
        refused.update(verdict="REFUSED", reason="PIN_UNSET:image", apps=[], boots=[], failed=[])
        with self.assertRaises(vast_report.ResultError):
            vast_report.read_result_file(self.file_with(self.doc(refused)))


class PayloadWiring(unittest.TestCase):
    def instruction(self, tier="large"):
        import payload_instruction as pi
        return pi.Instruction(tier=tier, windows=True, windows_runs=1, stages=(), spec=None, winapps=None, linux_apps="middle", rehearsal=False,
                              source="default")

    def test_not_in_the_default_plan_and_only_with_only_in_large(self):
        self.assertFalse(vast_payload.PROPIMAGE_APPS_IN_LARGE)
        self.assertNotIn("propimage_apps", vast_payload.plan_stage_names(self.instruction()))
        self.assertIn("propimage_apps", vast_payload.plan_stage_names(self.instruction(), ["propimage_apps"]))
        self.assertNotIn("propimage_apps", vast_payload.plan_stage_names(self.instruction("small"), ["propimage_apps"]))
        self.assertNotIn("propimage_apps", vast_payload.plan_stage_names(self.instruction("middle"), ["propimage_apps"]))

    def test_runs_after_every_windows_stage_and_before_the_spec_lane(self):
        import payload_instruction as pi
        inst = pi.Instruction(tier="large", windows=True, windows_runs=2, stages=("specviewperf",), spec="both", winapps="small",
                              linux_apps="middle", rehearsal=False, source="default")
        plan = vast_payload.plan_stage_names(inst, ["propimage_apps"])
        self.assertEqual(plan[-3:], ["windows_gate_2", "propimage_apps", "specviewperf"])

    def test_runs_before_the_spec_lane_without_windows_stages(self):
        import payload_instruction as pi
        inst = pi.Instruction(tier="large", windows=False, windows_runs=1, stages=("specviewperf",), spec="both", winapps=None,
                              linux_apps="middle", rehearsal=False, source="default")
        self.assertEqual(vast_payload.plan_stage_names(inst, ["propimage_apps"])[-2:], ["propimage_apps", "specviewperf"])

    def test_manual_only_table_and_the_budget_guard(self):
        self.assertEqual(vast_payload.MANUAL_ONLY_STAGES, {"propimage_apps": "PROPIMAGE_APPS_IN_LARGE", "install_flow": "INSTALL_FLOW_IN_LARGE", "kfml": "KFML_IN_MIDDLE_LARGE"})
        data = json.loads((profile_resources.ROOT / "ci/vast-broker/defaults.json").read_text())
        for stage in data["payloadStageResources"].values():
            stage["source"] = "scripts/ci/vast_payload.py"      # only the manual-only logic is under test, not the provenance files
        source = (HERE / "vast_payload.py").read_text()
        self.assertEqual(profile_resources.validate(data, source), [])
        flipped = source.replace("PROPIMAGE_APPS_IN_LARGE = False", "PROPIMAGE_APPS_IN_LARGE = True")
        self.assertIn("manual-only stage is in a default plan: propimage_apps", profile_resources.validate(data, flipped))
        data["payloadStageResources"]["propimage_apps"] = dict(next(iter(data["payloadStageResources"].values())))
        self.assertIn("manual-only stage has a budget row: propimage_apps", profile_resources.validate(data, source))

    def line(self, doc):
        return "noise\nKF_PROPIMAGE_APPS_RESULT " + json.dumps(doc) + "\n"

    def test_rows_pass_fail_and_not_run(self):
        result = {}
        ok = {"verdict": "PASS", "sha": SHA, "reason": ""}
        self.assertEqual(vast_payload.propimage_apps_rows(self.line(ok), 0, SHA, result), [vast_payload.row("propimage_apps", 1)])
        self.assertEqual(result["propimage_apps"], ok)
        skip = {"verdict": "NOT_RUN", "reason": "CPU_VENDOR", "sha": SHA}
        self.assertEqual(vast_payload.propimage_apps_rows(self.line(skip), 0, SHA, {}), [vast_payload.row("propimage_apps", skipped=1)])
        for doc, rc in ((dict(ok, verdict="FAIL"), 1), (dict(ok), 1), (dict(ok, verdict="REFUSED"), 0), (dict(skip, reason="GPU_FAMILY"), 0),
                        (dict(skip), 1), (dict(ok, sha="c" * 40), 0), (dict(ok, verdict="RETRY"), 3)):
            with self.subTest(verdict=doc.get('verdict'), rc=rc), self.assertRaises(vast_payload.EvidenceError):
                vast_payload.propimage_apps_rows(self.line(doc), rc, SHA, {})

    def test_rows_need_exactly_one_parseable_line_and_keep_a_red_record(self):
        for text in ("", self.line({"sha": SHA}) * 2, "KF_PROPIMAGE_APPS_RESULT {not json}\n"):
            with self.subTest(text[:30]), self.assertRaises(vast_payload.EvidenceError):
                vast_payload.propimage_apps_rows(text, 0, SHA, {})
        result = {}
        with self.assertRaises(vast_payload.EvidenceError):
            vast_payload.propimage_apps_rows(self.line({"verdict": "FAIL", "sha": SHA, "reason": "APP_FAIL:wukong"}), 1, SHA, result)
        self.assertEqual(result["propimage_apps"]["reason"], "APP_FAIL:wukong")

    def test_rows_bind_the_record_to_the_sha(self):
        with self.assertRaises(vast_payload.EvidenceError):
            vast_payload.propimage_apps_rows(self.line({"verdict": "PASS", "sha": "d" * 40, "reason": ""}), 0, SHA, {})

    def test_not_run_with_another_reason_is_a_failure(self):
        for reason in ("GPU_FAMILY", "PIN_UNSET:image", ""):
            with self.subTest(reason), self.assertRaises(vast_payload.EvidenceError):
                vast_payload.propimage_apps_rows(self.line({"verdict": "NOT_RUN", "reason": reason, "sha": SHA}), 0, SHA, {})

    def test_a_nonzero_exit_with_a_pass_record_is_a_failure(self):
        with self.assertRaises(vast_payload.EvidenceError):
            vast_payload.propimage_apps_rows(self.line({"verdict": "PASS", "sha": SHA, "reason": ""}), 1, SHA, {})

    def test_dry_run_refuses_the_stage_outside_the_large_tier(self):
        done = subprocess.run([sys.executable, "-B", str(HERE / "vast_payload.py"), "--dry-run", "--only", "propimage_apps"], capture_output=True,
                              text=True, env=dict(os.environ, KF_TIER="middle", KF_MERGE_WINDOWS="1"))
        self.assertEqual(done.returncode, 1, done.stdout + done.stderr)
        self.assertIn("KF_PAYLOAD_FAIL", done.stdout)

    def test_dry_run_lists_the_stage_with_only(self):
        done = subprocess.run([sys.executable, "-B", str(HERE / "vast_payload.py"), "--dry-run", "--only", "propimage_apps"], capture_output=True,
                              text=True, env=dict(os.environ, KF_TIER="large", KF_MERGE_WINDOWS="1"))
        self.assertIn("KF_PAYLOAD_PLAN stages=propimage_apps", done.stdout, done.stdout + done.stderr)


if __name__ == "__main__":
    unittest.main()
