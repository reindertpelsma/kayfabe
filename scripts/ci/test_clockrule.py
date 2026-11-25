#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free, VM-free tests for the uniform clock rule: scripts/bench/windows/clockrule.py and its use in win_vm.sh.

STATUS: LIVE as tests, 2026-11-16. Proves (CODE-READ and run here, not on a VM): the matrix {offline, online} x {sidecar, no
sidecar} x {strict, lenient}; the floor (recorded clock vs the creation time of LOADED SOFTWARE, never scripts or assets);
the refusals KF_CLOCK_BEFORE_INPUT and online + recorded; the lenient fallback line KF_CLOCK_NO_RECORD; win_vm.sh's dry-run
argv. Not proven: that a real guest boots with these values (a hardware confirmation is listed in the PR).
"""
import contextlib
import importlib.util
import io
import json
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "scripts/bench/windows/clockrule.py"
WINVM = ROOT / "scripts/bench/windows/win_vm.sh"


def load():
    spec = importlib.util.spec_from_file_location("fixture_clockrule", SRC)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


T0 = "2026-11-16T10:00:00Z"


class Base(unittest.TestCase):
    def setUp(self):
        self.c = load()
        self.tmp = Path(tempfile.mkdtemp(prefix="clockrule-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.sidecar = self.tmp / "clock.json"

    def make_record(self, utc=T0, inputs=()):
        self.c.record(str(self.sidecar), utc, "test", "rev0", inputs)

    def decide(self, net, record=True, strict=False, **kw):
        if record:
            self.make_record()
        return self.c.clock_for(net, "img", str(self.sidecar), strict=strict, **kw)

    def refused(self, reason, fn, *a, **kw):
        with self.assertRaises(self.c.ClockError) as cm:
            fn(*a, **kw)
        self.assertEqual(cm.exception.reason, reason)


class Matrix(Base):
    """{offline, online} x {sidecar, no sidecar} x {strict, lenient}."""

    def test_offline_with_sidecar_is_recorded_in_both_strictness(self):
        for strict in (False, True):
            r = self.decide("offline", True, strict)
            self.assertEqual((r["kind"], r["verdict"]), ("recorded", "PASS"))
            self.assertEqual(r["rtc"], "base=2026-11-16T10:00:00,clock=vm,driftfix=slew")

    def test_offline_without_sidecar_lenient_is_real_with_one_named_warn(self):
        r = self.decide("offline", False, False)
        self.assertEqual((r["kind"], r["verdict"], r["rtc"]), ("real", "WARN", "base=utc,driftfix=slew"))
        self.assertEqual(r["lines"], ["KF_CLOCK_NO_RECORD image=img using=real"])

    def test_offline_without_sidecar_strict_is_refused(self):
        self.refused("offline_real_clock_strict", self.decide, "offline", False, True)

    def test_online_with_sidecar_is_real_in_both_strictness(self):
        for strict in (False, True):
            r = self.decide("online", True, strict)
            self.assertEqual((r["kind"], r["verdict"], r["rtc"]), ("real", "PASS", "base=utc,driftfix=slew"))
            self.assertFalse([l for l in r["lines"] if "NO_RECORD" in l])

    def test_online_without_sidecar_is_real_and_not_a_warning(self):
        for strict in (False, True):
            r = self.decide("online", False, strict)
            self.assertEqual((r["kind"], r["verdict"]), ("real", "PASS"))

    def test_online_with_a_recorded_clock_is_always_refused(self):
        for strict in (False, True):
            self.refused("online_recorded_clock", self.c.clock_for, "online", "img", None, strict=strict, base=T0)
        self.refused("online_recorded_clock", self.c.assert_rule, "online", "recorded", False)

    def test_assert_rule_table(self):
        a = self.c.assert_rule
        a("offline", "recorded", True), a("online", "real", True), a("offline", "real", False)
        self.refused("offline_real_clock_strict", a, "offline", "real", True)
        self.refused("bad_net", a, "on", "real")
        self.refused("bad_kind", a, "offline", "fake")

    def test_build_phase_runs_the_real_clock_without_a_warning_even_strict(self):
        r = self.c.clock_for("offline", "img", str(self.sidecar), strict=True, phase="build")
        self.assertEqual((r["kind"], r["verdict"]), ("real", "PASS"))

    def test_a_bad_sidecar_is_refused_not_read_as_none(self):
        self.sidecar.write_text("{not json")
        self.refused("bad_record", self.c.clock_for, "offline", "img", str(self.sidecar))
        self.sidecar.write_text(json.dumps({"schema": 2, "recorded_utc": T0, "source": "x"}))
        self.refused("bad_record", self.c.clock_for, "offline", "img", str(self.sidecar))
        self.sidecar.write_text(json.dumps({"schema": 1, "recorded_utc": "yesterday", "source": "x"}))
        self.refused("bad_record", self.c.clock_for, "offline", "img", str(self.sidecar))

    def test_image_name_phase_and_net_are_validated(self):
        self.refused("bad_image", self.c.clock_for, "offline", "a b")
        self.refused("bad_net", self.c.clock_for, "yes", "img")
        self.refused("bad_phase", self.c.clock_for, "offline", "img", phase="x")


class Floor(Base):
    """clock_floor = max(recorded, max(created_utc of loaded software) + margin); only loaded software counts."""

    def floor(self, *specs, **kw):
        return self.decide("offline", True, False, loaded_software=specs, **kw)

    def test_no_software_the_floor_is_the_recorded_clock(self):
        r = self.floor()
        self.assertEqual((r["base_utc"], r["floor_input"]), (T0, None))

    def test_software_older_than_the_record_does_not_move_the_clock(self):
        r = self.floor("spec.tar=2026-11-16T07:00:00Z")  # +1 h = 08:00 < 10:00
        self.assertEqual((r["base_utc"], r["floor_input"]), (T0, None))

    def test_software_newer_than_the_record_raises_the_clock_by_the_margin(self):
        r = self.floor("spec.tar=2026-11-16T12:00:00Z")
        self.assertEqual((r["base_utc"], r["floor_input"]), ("2026-11-16T13:00:00Z", "spec.tar"))
        self.assertTrue(r["rtc"].startswith("base=2026-11-16T13:00:00,clock=vm"))

    def test_software_created_at_the_recorded_instant_still_gets_the_margin(self):
        r = self.floor("spec.tar=" + T0)
        self.assertEqual((r["base_utc"], r["floor_input"]), ("2026-11-16T11:00:00Z", "spec.tar"))

    def test_software_just_inside_the_margin_raises_just_to_the_margin(self):
        r = self.floor("spec.tar=2026-11-16T09:00:01Z")
        self.assertEqual(r["base_utc"], "2026-11-16T10:00:01Z")

    def test_many_inputs_the_newest_sets_the_floor(self):
        r = self.floor("a=2026-11-16T01:00:00Z", "b=2026-11-17T01:00:00Z", "c=2026-11-16T22:00:00Z", "d=2026-11-15T01:00:00Z")
        self.assertEqual((r["base_utc"], r["floor_input"]), ("2026-11-17T02:00:00Z", "b"))
        self.assertEqual(len(r["inputs"]), 4)

    def test_margin_is_a_parameter(self):
        r = self.floor("a=2026-11-16T12:00:00Z", margin_s=0)
        self.assertEqual(r["base_utc"], "2026-11-16T12:00:00Z")
        r = self.floor("a=2026-11-16T12:00:00Z", margin_s=60)
        self.assertEqual(r["base_utc"], "2026-11-16T12:01:00Z")
        self.refused("bad_margin", self.floor, "a=2026-11-16T12:00:00Z", margin_s=-1)

    def test_the_sidecar_remembers_which_input_set_the_floor(self):
        self.make_record("2026-11-16T10:00:00Z", ["drv.exe=2026-11-16T09:30:00Z", "old.msi=2026-11-01T00:00:00Z"])
        doc = json.loads(self.sidecar.read_text())
        self.assertEqual(doc["floor_input"], "drv.exe")
        self.assertEqual([i["name"] for i in doc["inputs"]], ["drv.exe", "old.msi"])
        r = self.c.clock_for("offline", "img", str(self.sidecar))
        self.assertEqual((r["base_utc"], r["floor_input"]), ("2026-11-16T10:30:00Z", "drv.exe"))  # the record's own inputs count

    def test_record_with_no_floor_input_says_null(self):
        self.make_record(T0, ["old.msi=2026-11-01T00:00:00Z"])
        self.assertIsNone(json.loads(self.sidecar.read_text())["floor_input"])

    def test_record_earlier_than_a_loaded_input_is_refused(self):
        self.refused("KF_CLOCK_BEFORE_INPUT", self.c.record, str(self.sidecar), T0, "t", "r", ["x=2026-11-16T10:00:01Z"])
        self.assertFalse(self.sidecar.exists())

    def test_explicit_base_earlier_than_an_input_is_refused(self):
        self.refused("KF_CLOCK_BEFORE_INPUT", self.c.clock_for, "offline", "img", None, loaded_software=["s=2026-11-16T10:00:01Z"], base=T0)

    def test_explicit_base_inside_the_margin_is_raised_not_refused(self):
        r = self.c.clock_for("offline", "img", None, loaded_software=["s=" + T0], base=T0)
        self.assertEqual((r["base_utc"], r["floor_input"]), ("2026-11-16T11:00:00Z", "s"))

    def test_explicit_base_after_the_margin_is_kept(self):
        r = self.c.clock_for("offline", "img", None, loaded_software=["s=2026-11-16T08:00:00Z"], base=T0)
        self.assertEqual((r["base_utc"], r["floor_input"]), (T0, None))

    def test_a_stale_record_is_raised_not_refused(self):
        r = self.floor("s=2027-01-01T00:00:00Z")  # a software newer than the record: raise
        self.assertEqual(r["base_utc"], "2027-01-01T01:00:00Z")

    # ---- where created_utc comes from
    def pins(self, items):
        p = self.tmp / "pins.json"
        p.write_text(json.dumps({"items": items}))
        return str(p)

    def test_the_pin_gives_created_utc_only_for_software(self):
        pins = self.pins([{"name": "spec.tar", "loads_software": True, "created_utc": "2026-11-16T12:00:00Z"}])
        r = self.floor("/x/dir/spec.tar", pins=[pins])
        self.assertEqual((r["base_utc"], r["inputs"][0]["source"], r["verdict"]), ("2026-11-16T13:00:00Z", "pin", "PASS"))

    def test_a_pin_without_loads_software_is_ignored_for_the_floor(self):
        pins = self.pins([{"name": "assets.iso", "created_utc": "2026-12-31T00:00:00Z"}])
        f = self.tmp / "assets.iso"
        f.write_text("x")
        os.utime(f, (1700000000, 1700000000))
        r = self.floor(str(f), pins=[pins])
        self.assertEqual(r["inputs"][0]["source"], "mtime")  # the pin did not apply
        self.assertEqual(r["base_utc"], T0)

    def test_an_explicit_created_beats_the_pin(self):
        pins = self.pins([{"name": "s", "loads_software": True, "created_utc": "2026-11-16T12:00:00Z"}])
        r = self.floor("s=2026-11-16T01:00:00Z", pins=[pins])
        self.assertEqual((r["base_utc"], r["inputs"][0]["source"]), (T0, "arg"))

    def test_missing_created_utc_falls_back_to_mtime_with_a_warn(self):
        f = self.tmp / "driver.exe"
        f.write_text("x")
        os.utime(f, (1800000000, 1800000000))  # 2027-01-15T08:00:00Z
        r = self.floor(str(f))
        self.assertEqual((r["verdict"], r["inputs"][0]["source"]), ("WARN", "mtime"))
        self.assertIn("KF_CLOCK_INPUT_NO_CREATED input=driver.exe using=mtime", r["lines"])
        self.assertEqual(r["base_utc"], "2027-01-15T09:00:00Z")

    def test_pin_with_null_created_utc_is_a_missing_created(self):
        pins = self.pins([{"name": "s.tar", "loads_software": True, "created_utc": None}])
        r = self.floor(str(self.tmp / "s.tar"), pins=[pins])  # no file either: skipped, named
        self.assertEqual(r["inputs"], [])
        self.assertIn("KF_CLOCK_SOFTWARE_MISSING path=%s" % (self.tmp / "s.tar"), r["lines"])

    def test_two_pin_values_for_one_software_are_refused(self):
        pins = self.pins([{"name": "s", "loads_software": True, "created_utc": "2026-11-16T12:00:00Z"},
                          {"file": "s", "loads_software": True, "created_utc": "2026-11-17T12:00:00Z"}])
        self.refused("bad_pin", self.floor, "s", pins=[pins])

    def test_offline_without_a_record_has_no_floor_at_all(self):
        r = self.c.clock_for("offline", "img", str(self.sidecar), loaded_software=["s=2030-01-01T00:00:00Z"])
        self.assertEqual((r["kind"], r["inputs"]), ("real", []))


class Cli(Base):
    def run_cli(self, *args):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            rc = self.c.main(list(args))
        return rc, out.getvalue(), err.getvalue()

    def test_rtc_prints_one_value_and_named_lines_on_stderr(self):
        rc, out, err = self.run_cli("rtc", "--net", "offline", "--image", "img", "--sidecar", str(self.sidecar))
        self.assertEqual((rc, out), (0, "base=utc,driftfix=slew\n"))
        self.assertEqual(err, "KF_CLOCK_NO_RECORD image=img using=real\n")

    def test_strict_refusal_exits_2_with_a_named_line_and_no_value(self):
        rc, out, err = self.run_cli("rtc", "--net", "offline", "--image", "img", "--sidecar", str(self.sidecar), "--strict", "1")
        self.assertEqual((rc, out), (2, ""))
        self.assertIn("KF_CLOCK_REFUSED reason=offline_real_clock_strict", err)

    def test_before_input_is_named_on_the_line(self):
        rc, _, err = self.run_cli("rtc", "--net", "offline", "--image", "img", "--base", "2026-11-16T10:00:00",
                                  "--loaded-software", "s=2026-11-16T11:00:00Z")
        self.assertEqual(rc, 2)
        self.assertIn("reason=KF_CLOCK_BEFORE_INPUT", err)

    def test_base_without_z_is_utc(self):
        rc, out, _ = self.run_cli("rtc", "--net", "offline", "--image", "img", "--base", "2026-11-16T10:00:00")
        self.assertEqual((rc, out), (0, "base=2026-11-16T10:00:00,clock=vm,driftfix=slew\n"))

    def test_a_corrupt_record_exits_2_not_a_real_clock(self):
        self.sidecar.write_text("[]")
        rc, out, err = self.run_cli("rtc", "--net", "offline", "--image", "img", "--sidecar", str(self.sidecar))
        self.assertEqual((rc, out), (2, ""))
        self.assertIn("reason=bad_record", err)

    def test_record_inputs_set_the_floor_input_and_the_next_boot(self):
        self.run_cli("record", "--sidecar", str(self.sidecar), "--utc", T0, "--source", "t", "--input", "drv=2026-11-16T09:30:00Z")
        js = self.tmp / "v.json"
        rc, out, _ = self.run_cli("rtc", "--net", "offline", "--image", "img", "--sidecar", str(self.sidecar), "--json", str(js))
        self.assertEqual((rc, out), (0, "base=2026-11-16T10:30:00,clock=vm,driftfix=slew\n"))
        v = json.loads(js.read_text())
        self.assertEqual((v["floor_input"], json.loads(self.sidecar.read_text())["floor_input"]), ("drv", "drv"))

    def test_pins_option_applies_only_to_software_pins(self):
        pins = self.tmp / "pins.json"
        pins.write_text(json.dumps({"items": [{"name": "sw.zip", "loads_software": True, "created_utc": "2026-11-16T12:00:00Z"},
                                              {"name": "data.bin", "created_utc": "2030-01-01T00:00:00Z"}]}))
        data = self.tmp / "data.bin"
        data.write_text("x")
        os.utime(data, (1700000000, 1700000000))
        rc, out, err = self.run_cli("rtc", "--net", "offline", "--image", "img", "--base", "2026-11-16T12:30:00Z", "--pins", str(pins),
                                    "--loaded-software", "sw.zip", str(data))
        self.assertEqual((rc, out), (0, "base=2026-11-16T13:00:00,clock=vm,driftfix=slew\n"))
        self.assertIn("KF_CLOCK_INPUT_NO_CREATED input=data.bin using=mtime", err)  # the data pin did not apply

    def test_a_pin_with_two_values_exits_2_and_two_pin_files_that_disagree_too(self):
        a, b = self.tmp / "a.json", self.tmp / "b.json"
        a.write_text(json.dumps([{"name": "s", "loads_software": True, "created_utc": "2026-11-16T12:00:00Z"},
                                 {"name": "s", "loads_software": True, "created_utc": "2026-11-16T13:00:00Z"}]))
        b.write_text(json.dumps([{"name": "s", "loads_software": True, "created_utc": "2026-11-16T14:00:00Z"}]))
        for pins in ([str(a)], [str(a), str(b)]):
            rc, _, err = self.run_cli("rtc", "--net", "offline", "--image", "img", "--base", T0, "--pins", *pins, "--loaded-software", "s")
            self.assertEqual(rc, 2, err)
            self.assertIn("reason=bad_pin", err)
        c = self.tmp / "c.json"
        c.write_text(json.dumps([{"name": "s", "loads_software": True, "created_utc": "2026-11-16T15:00:00Z"}]))
        rc, _, err = self.run_cli("rtc", "--net", "offline", "--image", "img", "--base", T0, "--pins", str(b), str(c), "--loaded-software", "s")
        self.assertEqual(rc, 2, err)

    def test_mtime_fallback_gives_warn_in_the_json_verdict(self):
        f = self.tmp / "tool.exe"
        f.write_text("x")
        os.utime(f, (1700000000, 1700000000))
        js = self.tmp / "v.json"
        rc, _, err = self.run_cli("rtc", "--net", "offline", "--image", "img", "--base", T0, "--loaded-software", str(f), "--json", str(js))
        self.assertEqual(rc, 0)
        self.assertEqual(json.loads(js.read_text())["verdict"], "WARN")
        self.assertIn("KF_CLOCK_INPUT_NO_CREATED", err)

    def test_json_verdict_and_record_command(self):
        rc, _, err = self.run_cli("record", "--sidecar", str(self.sidecar), "--utc", T0, "--source", "t", "--input", "a=2026-11-01T00:00:00Z")
        self.assertEqual(rc, 0)
        self.assertIn("KF_CLOCK_RECORDED", err)
        js = self.tmp / "v.json"
        rc, out, _ = self.run_cli("rtc", "--net", "offline", "--image", "img", "--sidecar", str(self.sidecar), "--json", str(js))
        self.assertEqual(rc, 0)
        v = json.loads(js.read_text())
        self.assertEqual((v["verdict"], v["kind"], v["base_utc"]), ("PASS", "recorded", T0))
        rc, _, err = self.run_cli("record", "--sidecar", str(self.sidecar), "--utc", "garbage", "--source", "t")
        self.assertEqual(rc, 2)


@unittest.skipUnless(WINVM.exists() and shutil.which("bash"), "needs bash and the repository's scripts")
class WinVmUsesTheRule(unittest.TestCase):
    """win_vm.sh in dry-run mode: the -rtc value follows the network and the sidecar."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="winvm-clock-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.vmdir = self.tmp / "root" / "kfwin"
        for d in ("secrets", "logs", "disk"):
            (self.vmdir / d).mkdir(parents=True)
        (self.vmdir / "installed").write_text("t\n")

    def sidecar(self, utc=T0):
        (self.vmdir / "disk/clock.json").write_text(json.dumps({"schema": 1, "recorded_utc": utc, "source": "t", "tool_rev": "r"}))

    def dry(self, verb=("run", "--dry-run", "--tag", "t"), **extra):
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": str(self.tmp), "LANG": "C",
               "WINVM_ROOT": str(self.tmp / "root"), "KF_GPU_LOCK": str(self.tmp / "gpu.lock"),
               "BENCH": str(self.tmp / "bench"), "WINVM_QEMU": "/fake/qemu", **extra}
        r = subprocess.run(["bash", str(WINVM), *verb], env=env, stdin=subprocess.DEVNULL, capture_output=True, text=True,
                           timeout=60, check=False)
        q = [l for l in r.stdout.splitlines() if l.startswith("DRYRUN_QEMU ")]
        argv = shlex.split(q[0][12:]) if q else []
        rtc = argv[argv.index("-rtc") + 1] if "-rtc" in argv else None
        return r, rtc

    def test_offline_with_a_record_boots_the_recorded_clock(self):
        self.sidecar()
        r, rtc = self.dry()
        self.assertEqual(rtc, "base=2026-11-16T10:00:00,clock=vm,driftfix=slew", r.stdout + r.stderr)
        self.assertNotIn("KF_CLOCK_NO_RECORD", r.stdout)

    def test_online_boots_the_real_clock_even_with_a_record(self):
        self.sidecar()
        r, rtc = self.dry(WINVM_NET="online")
        self.assertEqual(rtc, "base=utc,driftfix=slew", r.stdout + r.stderr)

    def test_offline_without_a_record_warns_once_and_runs_the_real_clock(self):
        r, rtc = self.dry()
        self.assertEqual((r.returncode, rtc), (0, "base=utc,driftfix=slew"), r.stdout + r.stderr)
        self.assertEqual(r.stdout.count("KF_CLOCK_NO_RECORD image=kfwin using=real"), 1)

    def test_strict_refuses_offline_without_a_record_and_starts_nothing(self):
        r, rtc = self.dry(WINVM_CLOCK_STRICT="1")
        self.assertEqual((r.returncode, rtc), (2, None))
        self.assertIn("offline_real_clock_strict", r.stdout + r.stderr)

    def test_strict_does_not_refuse_online(self):
        r, rtc = self.dry(WINVM_NET="online", WINVM_CLOCK_STRICT="1")
        self.assertEqual((r.returncode, rtc), (0, "base=utc,driftfix=slew"))

    def test_online_with_an_explicit_recorded_base_is_refused(self):
        r, rtc = self.dry(WINVM_NET="online", WINVM_RTC_BASE="2026-11-16T10:00:00")
        self.assertEqual((r.returncode, rtc), (2, None))
        self.assertIn("online_recorded_clock", r.stdout + r.stderr)

    def test_offline_explicit_base_is_the_recorded_clock(self):
        r, rtc = self.dry(WINVM_RTC_BASE="2026-11-16T10:00:00")
        self.assertEqual(rtc, "base=2026-11-16T10:00:00,clock=vm,driftfix=slew", r.stdout + r.stderr)

    def test_loaded_software_raises_the_clock_and_assets_do_not(self):
        self.sidecar()
        r, rtc = self.dry(WINVM_LOADED_SOFTWARE="spec.tar=2026-11-16T12:00:00Z")
        self.assertEqual(rtc, "base=2026-11-16T13:00:00,clock=vm,driftfix=slew", r.stdout + r.stderr)

    def asset_iso(self):
        iso = self.tmp / "apps.iso"
        iso.write_text("x")  # an mtime of now: later than the recorded clock
        return iso

    def test_the_app_disk_asset_never_moves_the_clock(self):
        self.sidecar()
        r, rtc = self.dry(WINVM_APPS_ISO=str(self.asset_iso()))
        self.assertEqual(rtc, "base=2026-11-16T10:00:00,clock=vm,driftfix=slew", r.stdout + r.stderr)

    def test_an_asset_does_not_even_warn_about_a_missing_created_utc(self):
        self.sidecar()
        r, _ = self.dry(WINVM_APPS_ISO=str(self.asset_iso()), WINVM_OFFICIAL_APPS_ISO=str(self.asset_iso()))
        self.assertNotIn("KF_CLOCK_INPUT_NO_CREATED", r.stdout)

    def test_a_software_pin_file_and_an_asset_pin_in_the_same_file(self):
        self.sidecar()
        pins = self.tmp / "pins.json"
        pins.write_text(json.dumps({"items": [{"name": "spec.tar", "loads_software": True, "created_utc": "2026-11-16T12:00:00Z"},
                                              {"name": "kfapps.iso", "created_utc": "2031-01-01T00:00:00Z"}]}))
        r, rtc = self.dry(WINVM_LOADED_SOFTWARE="/x/spec.tar /x/kfapps.iso=2026-11-01T00:00:00Z", WINVM_CLOCK_PINS=str(pins))
        self.assertEqual(rtc, "base=2026-11-16T13:00:00,clock=vm,driftfix=slew", r.stdout + r.stderr)

    def test_a_corrupt_clock_json_is_refused_not_ignored(self):
        (self.vmdir / "disk/clock.json").write_text("garbage")
        r, rtc = self.dry()
        self.assertEqual((r.returncode, rtc), (2, None))
        self.assertIn("bad_record", r.stdout + r.stderr)

    def test_the_strict_value_must_be_zero_or_one(self):
        self.sidecar()
        r, rtc = self.dry(WINVM_CLOCK_STRICT="maybe")
        self.assertEqual((r.returncode, rtc), (2, None))

    def test_strict_with_a_record_boots_the_recorded_clock(self):
        self.sidecar()
        r, rtc = self.dry(WINVM_CLOCK_STRICT="1")
        self.assertEqual(rtc, "base=2026-11-16T10:00:00,clock=vm,driftfix=slew", r.stdout + r.stderr)

    def test_the_install_boot_ignores_the_record(self):
        self.sidecar()
        r, rtc = self.dry(verb=("install", "--dry-run"))
        self.assertEqual(rtc, "base=utc,driftfix=slew", r.stdout + r.stderr)

    def test_the_install_boot_runs_the_real_clock_without_a_warning(self):
        r, rtc = self.dry(verb=("install", "--dry-run"), WINVM_CLOCK_STRICT="1")
        self.assertEqual((r.returncode, rtc), (0, "base=utc,driftfix=slew"), r.stdout + r.stderr)
        self.assertNotIn("KF_CLOCK_NO_RECORD", r.stdout)


if __name__ == "__main__":
    unittest.main()
