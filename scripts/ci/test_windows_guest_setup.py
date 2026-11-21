#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""scripts/windows/kayfabe-windows-setup.ps1: the table, the docs, and the behaviour against an in-memory registry.

STATUS: LIVE, 2026-11-20.

FALSIFIER (written before the code): the script sets a value other than the one the docs list (the table in the script, the JSON
next to it and the docs tables disagree); a write that does not stick is reported as done; a run without elevation changes the
registry; `auto` picks `pre` for a device that has NVIDIA's driver bound (or `post` for the Basic Display driver); a second run
writes again; `-InstallDriver` enables the device before the class-key value is written, or enables it after a failed write.

The script is dot-sourced by PowerShell 7 (`pwsh`, preinstalled on GitHub's ubuntu runners) and its primitives (registry, device
list, pnputil, elevation) are replaced by an in-memory fake; `Invoke-Setup` then runs the real logic. The real registry is the job of
the `windows-guest-setup` job of ci.yml (scripts/windows/ci_check_setup.ps1). `KF_WINSETUP_DIR` points the tests at a mutated copy of
the scripts/windows directory (scripts/ci/windows_guest_setup_mutants.py); `KF_PWSH` names the pwsh binary; `KF_REQUIRE_PWSH=1`
turns a missing pwsh from a skip into a failure.
"""

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
REAL_DIR = ROOT / "scripts" / "windows"
SCRIPT_DIR = Path(os.environ.get("KF_WINSETUP_DIR") or REAL_DIR)
PWSH = os.environ.get("KF_PWSH") or shutil.which("pwsh")
DOCS = [ROOT / "docs/howto/windows-guest.md", ROOT / "docs/WINDOWS_GUEST_SETUP.md"]
CLASS = "{4d36e968-e325-11ce-bfc1-08002be10318}"
CLASS_PATH = r"SYSTEM\CurrentControlSet\Control\Class\%s" % CLASS
SERVICE = r"SYSTEM\CurrentControlSet\Services\nvlddmkm"
DWM = r"SOFTWARE\Microsoft\Windows\Dwm"

PREAMBLE = r'''
$ErrorActionPreference = 'Stop'
$spec = $env:KF_SCEN | ConvertFrom-Json
. (Join-Path $env:KF_WINSETUP_DIR 'kayfabe-windows-setup.ps1')
$global:Spec = $spec
$global:Regs = @{}
$global:Keys = New-Object 'System.Collections.Generic.HashSet[string]'
$global:Ops = New-Object System.Collections.Generic.List[string]
$global:Props = @{}
foreach ($e in $spec.reg) {
    $global:Regs[($e.path + '|' + $e.name).ToLowerInvariant()] = [int]$e.value
    $global:Keys.Add($e.path.ToLowerInvariant()) | Out-Null
}
foreach ($k in $spec.keys) { $global:Keys.Add($k.ToLowerInvariant()) | Out-Null }
$global:Props['DEVPKEY_Device_Service'] = $spec.service
$global:Props['DEVPKEY_Device_Driver'] = $spec.driver
function Test-Elevated { return [bool]$global:Spec.elevated }
function Test-RegKey([string]$Path) { return $global:Keys.Contains($Path.ToLowerInvariant()) }
function Get-RegDword([string]$Path, [string]$Name) {
    $global:Ops.Add("get|$Path|$Name")
    $k = ($Path + '|' + $Name).ToLowerInvariant()
    if ($global:Regs.ContainsKey($k)) { return [int]$global:Regs[$k] }
    return $null
}
function Set-RegDword([string]$Path, [string]$Name, [int]$Value) {
    $global:Ops.Add("set|$Path|$Name|$Value")
    $global:Keys.Add($Path.ToLowerInvariant()) | Out-Null
    $drop = $false
    foreach ($d in $global:Spec.drop) { if ($Path -like "*$d*") { $drop = $true } }
    if (-not $drop) { $global:Regs[($Path + '|' + $Name).ToLowerInvariant()] = $Value }
}
function Get-NvidiaDevice {
    if ($null -eq $global:Spec.dev) { return $null }
    return [pscustomobject]@{ InstanceId = $global:Spec.dev; Class = 'Display' }
}
function Get-DeviceProp([string]$InstanceId, [string]$KeyName) {
    $v = $global:Props[$KeyName]
    if ($null -eq $v) { return '' }
    return [string]$v
}
function Invoke-Pnputil([string[]]$Arguments) {
    $global:Ops.Add('pnputil|' + ($Arguments -join ' '))
    if ($Arguments[0] -eq '/add-driver' -and $global:Spec.bind_on_add) {
        $global:Props['DEVPKEY_Device_Service'] = 'nvlddmkm'
        $global:Props['DEVPKEY_Device_Driver'] = $global:Spec.bind_on_add
    }
    return 0
}
$runs = @()
for ($i = 0; $i -lt $spec.repeat; $i++) {
    $SayLog.Clear()
    $global:Ops.Clear()
    $rc = Invoke-Setup $spec.phase $spec.install $spec.driverkey
    $runs += @{ rc = [int]$rc; log = @($SayLog); ops = @($global:Ops) }
}
$final = @{}
foreach ($k in @($global:Regs.Keys)) { $final[$k] = $global:Regs[$k] }
Write-Output ('KFJSON:' + (@{ runs = $runs; regs = $final } | ConvertTo-Json -Compress -Depth 6))
'''

BOUND = "PCI\\VEN_10DE&DEV_2786&SUBSYS_00000000&REV_A1\\4&0&0&0"


def pwsh(args, env=None, timeout=120):
    if not PWSH:
        raise AssertionError("pwsh is not installed and KF_REQUIRE_PWSH=1")
    e = dict(os.environ)
    e["KF_WINSETUP_DIR"] = str(SCRIPT_DIR)
    e.update(env or {})
    return subprocess.run([PWSH, "-NoProfile", "-NonInteractive"] + args, capture_output=True, text=True, env=e, timeout=timeout)


def scenario(**kw):
    spec = {
        "elevated": True, "dev": None, "service": "", "driver": "", "reg": [], "keys": [], "drop": [],
        "bind_on_add": "", "phase": "auto", "install": "", "driverkey": "", "repeat": 1,
    }
    spec.update(kw)
    r = pwsh(["-Command", PREAMBLE], env={"KF_SCEN": json.dumps(spec)})
    lines = [l for l in r.stdout.splitlines() if l.startswith("KFJSON:")]
    if not lines:
        raise AssertionError("no result (rc=%s)\nstdout:\n%s\nstderr:\n%s" % (r.returncode, r.stdout, r.stderr))
    return json.loads(lines[-1][len("KFJSON:"):])


def has(run, text):
    return any(text in line for line in run["log"])


def value(result, path, name):
    return result["regs"].get((path + "|" + name).lower())


def nvidia(**kw):
    base = {"dev": BOUND, "service": "nvlddmkm", "driver": CLASS + "\\0001"}
    base.update(kw)
    return base


def basic(**kw):
    base = {"dev": BOUND, "service": "BasicDisplay", "driver": CLASS + "\\0000"}
    base.update(kw)
    return base


class SetUp(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not PWSH:
            if os.environ.get("KF_REQUIRE_PWSH") == "1":
                raise AssertionError("pwsh is not installed and KF_REQUIRE_PWSH=1")
            raise unittest.SkipTest("pwsh is not installed (set KF_REQUIRE_PWSH=1 to make that a failure)")


class Files(SetUp):
    def test_script_parses_without_errors(self):
        code = (
            "$e = $null; $t = $null; [void][System.Management.Automation.Language.Parser]::ParseFile($env:KF_FILE, [ref]$t, [ref]$e); "
            "if ($e.Count -gt 0) { $e | ForEach-Object { $_.ToString() }; exit 1 }; 'PARSED'"
        )
        r = pwsh(["-Command", code], env={"KF_FILE": str(SCRIPT_DIR / "kayfabe-windows-setup.ps1")})
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("PARSED", r.stdout)

    def test_bat_wrapper_calls_the_script_with_the_policy_and_passes_arguments(self):
        text = (SCRIPT_DIR / "kayfabe-windows-setup.bat").read_bytes().decode("ascii")
        self.assertIn("\r\n", text)
        self.assertRegex(text, r'powershell\.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0kayfabe-windows-setup\.ps1" %\*')
        self.assertIn("exit /b %KF_SETUP_RC%", text)

    def test_script_is_windows_powershell_5_compatible_in_syntax(self):
        text = (SCRIPT_DIR / "kayfabe-windows-setup.ps1").read_text()
        code = "\n".join(l for l in text.splitlines() if not l.lstrip().startswith("#"))
        self.assertNotIn("&&", code)
        self.assertNotIn("??", code)
        self.assertNotRegex(code, r"\?\.")
        self.assertNotRegex(code, r"\$\w+\s+\?\s")


class Table(SetUp):
    def test_the_script_table_equals_the_json(self):
        r = pwsh(["-Command", ". (Join-Path $env:KF_WINSETUP_DIR 'kayfabe-windows-setup.ps1'); ConvertTo-Json -Compress -Depth 4 -InputObject @($KfSettings)"])
        self.assertEqual(r.returncode, 0, r.stderr)
        script = sorted(json.loads(r.stdout), key=lambda s: s["Id"])
        data = json.loads((REAL_DIR / "kayfabe-windows-setup.json").read_text())["settings"]
        want = [{"Id": s["id"], "Phase": s["phase"], "Path": s["path"], "Name": s["name"], "Type": s["type"], "Value": s["value"]} for s in data]
        self.assertEqual(script, sorted(want, key=lambda s: s["Id"]))

    def test_the_json_is_well_formed(self):
        data = json.loads((REAL_DIR / "kayfabe-windows-setup.json").read_text())["settings"]
        self.assertEqual([s["id"] for s in data], list(range(1, len(data) + 1)))
        for s in data:
            self.assertIn(s["phase"], ("pre", "post"))
            self.assertEqual(s["type"], "DWord")
            self.assertIsInstance(s["value"], int)
            self.assertFalse(s["path"].upper().startswith("HKLM"), s["path"])
            self.assertTrue(s["why"].strip())
        self.assertEqual({s["phase"] for s in data}, {"pre", "post"})
        self.assertEqual([s["id"] for s in data if s["path"].endswith("<index>")], [s["id"] for s in data if s["phase"] == "post"])

    def test_the_docs_tables_are_the_json(self):
        for doc in DOCS:
            text = doc.read_text()
            for row in docs_rows():
                self.assertTrue(row in text, "%s lacks the row generated from kayfabe-windows-setup.json:\n%s" % (doc.name, row))


def docs_rows():
    data = json.loads((REAL_DIR / "kayfabe-windows-setup.json").read_text())["settings"]
    return [
        "| %d | `HKLM\\%s` `%s` | `%s` | %s | %s |" % (s["id"], s["path"], s["name"], s["value"], s["phase"], s["why"])
        for s in data
    ]


class Pre(SetUp):
    def test_pre_sets_the_service_and_dwm_values_and_creates_the_missing_service_key(self):
        res = scenario(phase="pre", **basic())
        run = res["runs"][0]
        self.assertEqual(run["rc"], 0, run["log"])
        self.assertEqual(value(res, SERVICE, "EnableGpuFirmware"), 1)
        self.assertEqual(value(res, DWM, "OverlayTestMode"), 5)
        self.assertTrue(any(op.startswith("set|" + SERVICE) for op in run["ops"]))

    def test_pre_does_not_touch_the_class_key_and_says_what_comes_next(self):
        res = scenario(phase="pre", **basic(keys=[CLASS_PATH + "\\0000"]))
        run = res["runs"][0]
        self.assertFalse([k for k in res["regs"] if k.startswith(CLASS_PATH.lower())], res["regs"])
        self.assertTrue(has(run, "phase post"))

    def test_pre_needs_no_nvidia_device_at_all(self):
        res = scenario(phase="pre")
        self.assertEqual(res["runs"][0]["rc"], 0)


class Post(SetUp):
    def test_post_sets_the_class_key_at_the_discovered_index(self):
        res = scenario(phase="post", **nvidia())
        run = res["runs"][0]
        self.assertEqual(run["rc"], 0, run["log"])
        self.assertEqual(value(res, CLASS_PATH + "\\0001", "EnableGpuFirmware"), 1)
        self.assertEqual(value(res, SERVICE, "EnableGpuFirmware"), 1)
        self.assertEqual(value(res, DWM, "OverlayTestMode"), 5)
        self.assertFalse([k for k in res["regs"] if k.startswith((CLASS_PATH + "\\0000").lower())])

    def test_post_says_a_restart_is_needed(self):
        run = scenario(phase="post", **nvidia())["runs"][0]
        self.assertTrue(has(run, "RESTART REQUIRED"), run["log"])

    def test_post_with_a_given_driver_key_says_a_restart_is_needed_and_skips_discovery(self):
        res = scenario(phase="post", driverkey=CLASS + "\\0007")
        run = res["runs"][0]
        self.assertEqual(run["rc"], 0, run["log"])
        self.assertEqual(value(res, CLASS_PATH + "\\0007", "EnableGpuFirmware"), 1)
        self.assertTrue(has(run, "RESTART REQUIRED"))

    def test_post_refuses_when_the_device_is_on_the_basic_display_driver(self):
        res = scenario(phase="post", **basic())
        run = res["runs"][0]
        self.assertEqual(run["rc"], 3)
        self.assertTrue(has(run, "REFUSED"))
        self.assertEqual(res["regs"], {})

    def test_post_refuses_when_there_is_no_nvidia_device(self):
        res = scenario(phase="post")
        self.assertEqual(res["runs"][0]["rc"], 3)
        self.assertEqual(res["regs"], {})

    def refuses_key(self, keys):
        for bad in keys:
            res = scenario(phase="post", driverkey=bad)
            self.assertEqual(res["runs"][0]["rc"], 2, bad)
            self.assertEqual(res["regs"], {}, bad)

    def test_post_refuses_a_driver_key_that_walks_out_of_the_class_key(self):
        self.refuses_key([r"{4d36e968-e325-11ce-bfc1-08002be10318}\..\..\Run", r"{4d36e968-e325-11ce-bfc1-08002be10318}\0001\x"])

    def test_post_refuses_a_driver_key_of_another_class_or_without_an_index(self):
        self.refuses_key([r"{00000000-0000-0000-0000-000000000000}\0001", r"\0001", r"{4d36e968-e325-11ce-bfc1-08002be10318}"])


class Auto(SetUp):
    def test_auto_picks_post_when_the_nvidia_driver_is_bound(self):
        res = scenario(phase="auto", **nvidia())
        run = res["runs"][0]
        self.assertTrue(has(run, "auto -> post"), run["log"])
        self.assertEqual(value(res, CLASS_PATH + "\\0001", "EnableGpuFirmware"), 1)

    def test_auto_on_a_bound_driver_asks_for_a_restart_and_sets_all_three(self):
        res = scenario(phase="auto", **nvidia())
        run = res["runs"][0]
        self.assertTrue(has(run, "RESTART REQUIRED"), run["log"])
        self.assertEqual(len([k for k in res["regs"]]), 3)

    def test_a_basic_display_device_is_not_a_bound_nvidia_driver(self):
        res = scenario(phase="auto", **basic())
        run = res["runs"][0]
        self.assertTrue(has(run, "auto -> pre"), run["log"])
        self.assertEqual(run["rc"], 0)
        self.assertFalse([k for k in res["regs"] if k.startswith(CLASS_PATH.lower())])

    def test_auto_picks_pre_without_any_nvidia_device(self):
        run = scenario(phase="auto")["runs"][0]
        self.assertTrue(has(run, "auto -> pre"))


class Idempotent(SetUp):
    def test_the_second_run_writes_nothing_and_ends_in_the_same_state(self):
        res = scenario(phase="post", repeat=2, **nvidia())
        first, second = res["runs"]
        self.assertEqual(first["rc"], 0)
        self.assertEqual(second["rc"], 0)
        self.assertTrue([op for op in first["ops"] if op.startswith("set|")])
        self.assertEqual([op for op in second["ops"] if op.startswith("set|")], [])
        self.assertEqual(value(res, SERVICE, "EnableGpuFirmware"), 1)

    def test_the_second_run_says_already_and_still_reads_back(self):
        second = scenario(phase="pre", repeat=2)["runs"][1]
        self.assertTrue(has(second, "already  HKLM\\" + SERVICE))
        self.assertTrue(has(second, "readback HKLM\\" + SERVICE))

    def test_a_wrong_existing_value_is_corrected(self):
        res = scenario(phase="pre", reg=[{"path": SERVICE, "name": "EnableGpuFirmware", "value": 0}, {"path": DWM, "name": "OverlayTestMode", "value": 1}])
        self.assertEqual(value(res, SERVICE, "EnableGpuFirmware"), 1)
        self.assertEqual(value(res, DWM, "OverlayTestMode"), 5)
        self.assertTrue(has(res["runs"][0], "(was 0)"))


class Readback(SetUp):
    def test_a_write_that_does_not_stick_fails_the_run(self):
        res = scenario(phase="pre", drop=["nvlddmkm"])
        run = res["runs"][0]
        self.assertEqual(run["rc"], 1, run["log"])
        self.assertTrue(has(run, "FAILED   HKLM\\" + SERVICE))
        self.assertFalse(has(run, "done     pre phase"))

    def test_every_write_is_followed_by_a_read_of_the_same_value(self):
        run = scenario(phase="post", **nvidia())["runs"][0]
        ops = run["ops"]
        sets = [i for i, op in enumerate(ops) if op.startswith("set|")]
        self.assertEqual(len(sets), 3)
        for i in sets:
            _, path, name, _v = ops[i].split("|")
            self.assertTrue(any(op == "get|%s|%s" % (path, name) for op in ops[i + 1:]), ops[i])
        for path, name, v in ((SERVICE, "EnableGpuFirmware", 1), (DWM, "OverlayTestMode", 5), (CLASS_PATH + "\\0001", "EnableGpuFirmware", 1)):
            self.assertTrue(has(run, "readback HKLM\\%s %s=%d OK" % (path, name, v)), run["log"])


class Elevation(SetUp):
    def refused(self, **kw):
        res = scenario(elevated=False, **kw)
        run = res["runs"][0]
        self.assertEqual(run["rc"], 2, run["log"])
        self.assertTrue(has(run, "elevated PowerShell") and has(run, "Nothing was changed"), run["log"])
        self.assertEqual(res["regs"], {})
        self.assertEqual([op for op in run["ops"] if op.split("|")[0] in ("set", "pnputil")], [])

    def test_unelevated_pre_is_refused_and_nothing_is_touched(self):
        self.refused(phase="pre")

    def test_unelevated_post_is_refused_and_nothing_is_touched(self):
        self.refused(phase="post", **nvidia())

    def test_unelevated_auto_and_install_driver_are_refused_and_nothing_is_touched(self):
        self.refused(phase="auto", **nvidia())
        with tempfile.TemporaryDirectory() as d:
            Path(d, "nv_dispi.inf").write_text("x")
            self.refused(install=d, **basic())

    def test_refusal_happens_before_the_device_list_or_registry_is_read(self):
        run = scenario(elevated=False, phase="auto", **nvidia())["runs"][0]
        self.assertEqual(run["ops"], [])


class Install(SetUp):
    def run_install(self, **kw):
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, True)
        Path(d, "nv_dispi.inf").write_text("; fake")
        return scenario(install=d, **kw), d

    def test_the_order_is_pre_disable_add_class_value_enable(self):
        res, d = self.run_install(**basic(bind_on_add=CLASS + "\\0002"))
        run = res["runs"][0]
        self.assertEqual(run["rc"], 0, run["log"])
        ops = [op for op in run["ops"] if op.startswith(("set|", "pnputil|"))]
        kinds = []
        for op in ops:
            if op.startswith("pnputil|/disable-device"):
                kinds.append("disable")
            elif op.startswith("pnputil|/add-driver"):
                kinds.append("add")
            elif op.startswith("pnputil|/enable-device"):
                kinds.append("enable")
            elif "|" + CLASS_PATH in op:
                kinds.append("class")
            else:
                kinds.append("pre")
        self.assertEqual(kinds, ["pre", "pre", "disable", "add", "class", "enable"], ops)
        self.assertEqual([k for k in kinds if k != "pre"], ["disable", "add", "class", "enable"], ops)
        self.assertLess(kinds.index("pre"), kinds.index("disable"))
        self.assertEqual(value(res, CLASS_PATH + "\\0002", "EnableGpuFirmware"), 1)

    def test_the_device_is_disabled_before_the_driver_is_added(self):
        res, d = self.run_install(**basic(bind_on_add=CLASS + "\\0002"))
        ops = [op for op in res["runs"][0]["ops"] if op.startswith("pnputil|")]
        self.assertTrue(ops[0].startswith("pnputil|/disable-device"), ops)
        self.assertTrue(ops[1].startswith("pnputil|/add-driver"), ops)
        self.assertTrue(ops[1].endswith("/install"), ops)

    def test_a_failed_class_write_leaves_the_device_disabled(self):
        res, d = self.run_install(drop=["Control\\Class"], **basic(bind_on_add=CLASS + "\\0002"))
        run = res["runs"][0]
        self.assertEqual(run["rc"], 1, run["log"])
        self.assertFalse([op for op in run["ops"] if op.startswith("pnputil|/enable-device")], run["ops"])
        self.assertTrue(has(run, "LEFT DISABLED"))

    def test_a_failed_class_write_says_how_to_recover(self):
        res, d = self.run_install(drop=["Control\\Class"], **basic(bind_on_add=CLASS + "\\0002"))
        run = res["runs"][0]
        self.assertEqual(run["rc"], 1)
        self.assertTrue(has(run, "recover  "), run["log"])
        self.assertTrue(has(run, "FAILED   HKLM\\" + CLASS_PATH + "\\0002"), run["log"])

    def test_a_failed_pre_value_does_not_touch_the_device(self):
        res, d = self.run_install(drop=["nvlddmkm"], **basic(bind_on_add=CLASS + "\\0002"))
        run = res["runs"][0]
        self.assertEqual([op for op in run["ops"] if op.startswith("pnputil|")], [])

    def test_a_failed_pre_value_ends_the_install_with_one_and_a_message(self):
        res, d = self.run_install(drop=["nvlddmkm"], **basic(bind_on_add=CLASS + "\\0002"))
        run = res["runs"][0]
        self.assertEqual(run["rc"], 1)
        self.assertTrue(has(run, "the device was not touched"), run["log"])

    def test_a_missing_directory_or_inf_is_refused_before_anything_changes(self):
        res = scenario(install=os.path.join(tempfile.gettempdir(), "kf-no-such-dir-xyz"), **basic())
        self.assertEqual(res["runs"][0]["rc"], 2)
        self.assertEqual(res["regs"], {})
        with tempfile.TemporaryDirectory() as d:
            res = scenario(install=d, **basic())
            self.assertEqual(res["runs"][0]["rc"], 2)
            self.assertEqual(res["regs"], {})

    def test_the_inf_is_found_in_a_nested_directory(self):
        with tempfile.TemporaryDirectory() as d:
            nested = Path(d, "pkg", "Display.Driver")
            nested.mkdir(parents=True)
            (nested / "nv_dispi.inf").write_text("; fake")
            res = scenario(install=d, **basic(bind_on_add=CLASS + "\\0002"))
            self.assertEqual(res["runs"][0]["rc"], 0, res["runs"][0]["log"])

    def test_install_driver_cannot_be_combined_with_a_phase(self):
        with tempfile.TemporaryDirectory() as d:
            res = scenario(install=d, phase="pre", **basic())
            self.assertEqual(res["runs"][0]["rc"], 2)
            self.assertEqual(res["regs"], {})


class Output(SetUp):
    def test_nothing_sensitive_is_printed(self):
        env = {"COMPUTERNAME": "KFSECRETHOST", "USERNAME": "kfsecretuser", "USERPROFILE": "C:\\Users\\kfsecretuser"}
        spec = nvidia(phase="post")
        spec.update({"elevated": True, "reg": [], "keys": [], "drop": [], "bind_on_add": "", "install": "", "driverkey": "", "repeat": 1})
        r = pwsh(["-Command", PREAMBLE], env={"KF_SCEN": json.dumps(spec), **env})
        for secret in env.values():
            self.assertNotIn(secret, r.stdout)
        self.assertNotIn("SUBSYS", r.stdout)
        self.assertNotIn("4&0&0&0", r.stdout)
        self.assertIn("VEN_10DE&DEV_2786", r.stdout)

    def test_a_device_is_named_by_vendor_and_device_id_only(self):
        run = scenario(phase="post", **nvidia())["runs"][0]
        self.assertTrue(has(run, "device   VEN_10DE&DEV_2786 bound to nvlddmkm"), run["log"])
        self.assertFalse(any("PCI\\" in line or "REV_" in line for line in run["log"]), run["log"])

    def test_every_line_has_the_prefix(self):
        r = pwsh(["-Command", PREAMBLE], env={"KF_SCEN": json.dumps({**{"elevated": True, "dev": None, "service": "", "driver": "", "reg": [], "keys": [], "drop": [], "bind_on_add": "", "phase": "pre", "install": "", "driverkey": "", "repeat": 1}})})
        lines = [l for l in r.stdout.splitlines() if l and not l.startswith("KFJSON:")]
        self.assertTrue(lines)
        for l in lines:
            self.assertTrue(l.startswith("KAYFABE-SETUP "), l)


class Workflow(unittest.TestCase):
    def test_the_windows_job_runs_the_check_on_windows_powershell_and_the_gate_waits_for_it(self):
        import yaml

        wf = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text())
        job = wf["jobs"]["windows-guest-setup"]
        self.assertEqual(job["runs-on"], "windows-latest")
        self.assertIsInstance(job["timeout-minutes"], int)
        runs = [step for step in job["steps"] if "run" in step]
        self.assertEqual(len(runs), 1)
        self.assertEqual(runs[0]["shell"], "powershell")
        self.assertIn("scripts\\ci\\windows_setup_check.ps1", runs[0]["run"])
        self.assertNotIn("continue-on-error", job)
        self.assertIn("windows-guest-setup", wf["jobs"]["gate"]["needs"])
        self.assertIn("needs.windows-guest-setup.result", wf["jobs"]["gate"]["steps"][0]["env"]["MUST_RUN"])
        mutants = "\n".join(str(step.get("run", "")) for step in wf["jobs"]["mutants-a"]["steps"])
        self.assertIn("python3 scripts/ci/windows_guest_setup_mutants.py", mutants)

    def test_the_check_script_parses(self):
        if not PWSH:
            self.skipTest("pwsh is not installed")
        code = (
            "$e = $null; $t = $null; [void][System.Management.Automation.Language.Parser]::ParseFile($env:KF_FILE, [ref]$t, [ref]$e); "
            "if ($e.Count -gt 0) { $e | ForEach-Object { $_.ToString() }; exit 1 }"
        )
        r = pwsh(["-Command", code], env={"KF_FILE": str(ROOT / "scripts/ci/windows_setup_check.ps1")})
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)


class Docs(unittest.TestCase):
    def test_the_manual_links_the_script_and_has_no_hand_typed_reg_add(self):
        text = (ROOT / "docs/howto/windows-guest.md").read_text()
        self.assertIn("scripts/windows/kayfabe-windows-setup.ps1", text)
        self.assertIn("kayfabe-windows-setup.bat", text)
        self.assertNotIn("reg add", text)
        self.assertIn("kayfabe-windows-setup", (ROOT / "docs/howto/installing.md").read_text())

    def test_the_manual_page_carries_no_dated_agent_notes_for_the_script(self):
        text = (ROOT / "docs/howto/windows-guest.md").read_text()
        start = text.index("## The settings it needs")
        end = text.index("\n## ", start + 4)  # the settings section ends at the next heading (the harness moved to its own page)
        section = text[start:end]
        self.assertNotRegex(section, r"20\d\d-\d\d-\d\d")
        self.assertNotRegex(section, r"[Ss]ince 20|\(20\d\d|evidence|STATUS")

    def test_the_markdown_table_rows_match_the_phase_column_header(self):
        text = (ROOT / "docs/howto/windows-guest.md").read_text()
        self.assertIn("| # | Key | Value | Phase | Why |", text)


if __name__ == "__main__":
    sys.exit(unittest.main())
