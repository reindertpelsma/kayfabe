#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""One proprietary-image pipeline: verified unpack, offline overlay, identity/GPU checks,
interactive app evidence, and shutdown. STATUS: DESIGN-ONLY, 2026-11-13; unified
pipeline is not hardware-qualified. Measured predecessor facts: traces/hwrun_20261113/README.md.
A fresh overlay uses the frozen RTC; --resume advances its .clock sidecar across clean
stops and verifies against that derived base. In-process guest reboots keep clock=vm.
No window-exists passes. No authentication, runtime downloads or binary reverse engineering.
"""
import argparse
import importlib.util
import json
import math
import os
import re
import shlex
import subprocess
import sys
import tempfile
import time
import uuid
import warnings
from contextlib import ExitStack
from datetime import timedelta
from pathlib import Path

HERE = Path(__file__).resolve().parent


def _load_propimage():
    spec = importlib.util.spec_from_file_location("propimage", HERE / "propimage.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


PI = _load_propimage()

GUEST_DIR = r"C:\kfapps"


def run(cmd, **kw):
    kw.setdefault("capture_output", True)
    kw.setdefault("text", True)
    return subprocess.run(cmd, check=False, **kw)


def require_swtpm(output):
    match = re.search(r"\bversion\s+(\d+)\.(\d+)(?:\.(\d+))?", output)
    version = tuple(int(x or 0) for x in match.groups()) if match else (0, 0, 0)
    if version < (0, 10, 0):
        raise ValueError(
            "Prerequisite: swtpm >= 0.10 required for this image TPM state; set SWTPM to a compatible binary (also requires compatible libtpms)"
        )
    return version


def boot_age_guard(age, budget):
    if not all(type(x) in (int, float) for x in (age, budget)):
        raise TypeError("Invalid boot-age type")
    if not all(math.isfinite(x) and x >= 0 for x in (age, budget)):
        raise ValueError("Invalid boot age or check budget")
    return age + budget > 55 * 60


def classify_shutdown(events):
    return (
        "harness"
        if any(
            e.get("event_id") == 1074 and re.search(r"(?i)\bwlms\.exe\b", str(e.get("process", "")))
            for e in events
        )
        else None
    )


def shutdown_events(g):
    script = r"""
$boot=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime
$events=@(Get-WinEvent -FilterHashtable @{LogName='System';Id=1074;StartTime=$boot} -ErrorAction SilentlyContinue | ForEach-Object {
 @{event_id=$_.Id;process=[string]$_.Properties[0].Value;time=$_.TimeCreated.ToUniversalTime().ToString('o')}
})
ConvertTo-Json -InputObject $events -Compress
"""
    p = g.ps(script)
    if p.returncode:
        raise ValueError("Guest event log unavailable; shutdown cause unverified")
    events = json.loads(p.stdout.lstrip("\ufeff"))
    if not isinstance(events, list):
        raise TypeError("Invalid shutdown event record")
    return events


def render_gpu_fix(expected):
    if not re.fullmatch(r"GPU-[A-Za-z0-9-]+", expected):
        raise ValueError("Invalid expected GPU UUID")
    return (
        "$ErrorActionPreference='Stop'\n$expected='"
        + expected
        + "'\n"
        + r"""
$base='HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}'
$changed=$false
Get-ChildItem $base | Where-Object {$_.PSChildName -match '^\d{4}$'} | ForEach-Object {
 $ip=Get-ItemProperty $_.PSPath
 if(($ip.DriverDesc -match 'NVIDIA' -or $ip.ProviderName -match 'NVIDIA') -and $ip.EnableGpuFirmware -ne 1){
  New-ItemProperty -Path $_.PSPath -Name EnableGpuFirmware -PropertyType DWord -Value 1 -Force | Out-Null
  $changed=$true
 }
}
if($changed){
 Get-PnpDevice -Class Display | Where-Object {$_.FriendlyName -match 'NVIDIA'} | ForEach-Object {
  pnputil /restart-device $_.InstanceId | Out-Null
  if($LASTEXITCODE -ne 0){throw 'NVIDIA display device restart failed'}
 }
 Start-Sleep 15
}
$ids=@(& nvidia-smi --query-gpu=uuid --format=csv,noheader)
if($LASTEXITCODE -ne 0 -or $ids.Count -ne 1 -or $ids[0].Trim() -cne $expected){throw 'Post-fix seed GPU UUID mismatch'}
$ids[0].Trim()
"""
    )


# Data used both to render the repair and to validate its independent readback.
POWER_SETTINGS = {
    "sleep": ("SUB_SLEEP", "STANDBYIDLE"),
    "hibernate_after": ("SUB_SLEEP", "HIBERNATEIDLE"),
    "unattended_sleep": ("SUB_SLEEP", "7bc4a2f9-d8fc-4469-b07b-33eb785aaca0"),
    "hybrid_sleep": ("SUB_SLEEP", "HYBRIDSLEEP"),
    "monitor": ("SUB_VIDEO", "VIDEOIDLE"),
    "disk": ("SUB_DISK", "DISKIDLE"),
    "wake_signin": ("SUB_NONE", "CONSOLELOCK"),
}
POWER_REGISTRY = {
    "hibernate_enabled": (r"HKLM:\SYSTEM\CurrentControlSet\Control\Power", "HibernateEnabled", 0),
    "fast_startup": (
        r"HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Power",
        "HiberbootEnabled",
        0,
    ),
    "lock_screen": (
        r"HKLM:\SOFTWARE\Policies\Microsoft\Windows\Personalization",
        "NoLockScreen",
        1,
    ),
    "inactivity": (
        r"HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System",
        "InactivityTimeoutSecs",
        0,
    ),
}
for _prefix, _path in (
    ("desktop", r"HKCU:\Control Panel\Desktop"),
    ("policy", r"HKCU:\Software\Policies\Microsoft\Windows\Control Panel\Desktop"),
):
    for _key, _value in (
        ("active", "ScreenSaveActive"),
        ("secure", "ScreenSaverIsSecure"),
        ("timeout", "ScreenSaveTimeOut"),
    ):
        POWER_REGISTRY[_prefix + "_" + _key] = (_path, _value, "0")


def render_power_fix():
    settings = [{"name": n, "subgroup": s, "setting": v} for n, (s, v) in POWER_SETTINGS.items()]
    registry = [
        {
            "name": n,
            "path": p,
            "key": k,
            "value": v,
            "kind": "DWord" if type(v) is int else "String",
        }
        for n, (p, k, v) in POWER_REGISTRY.items()
    ]
    data = json.dumps({"settings": settings, "registry": registry}).replace("'", "''")
    return (
        "$plan=ConvertFrom-Json '"
        + data
        + "'\n"
        + r"""
$ErrorActionPreference='Stop'
$r=@{queries=@{};registry=@{};sources=@{};errors=@();sid=[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value}
function Power { param([Parameter(ValueFromRemainingArguments=$true)][string[]]$argsList)
 $text=(& powercfg @argsList 2>&1 | Out-String)
 if($LASTEXITCODE -ne 0){$r.errors+=('powercfg '+($argsList -join ' ')+' failed: '+$text)}
 return $text
}
try {
 $guid='[0-9a-fA-F]{8}(?:-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}'
 $scheme=[regex]::Match((Power @('/getactivescheme')),$guid).Value
 if(-not $scheme){throw 'Cannot identify active power scheme'}
 $aliases=Power @('/aliases')
 function Resolve-Guid($value){
  if($value -match ('^'+$guid+'$')){return $value}
  $m=[regex]::Match($aliases,('(?im)('+ $guid +')\s+'+[regex]::Escape($value)+'\s*$'))
  if(-not $m.Success){throw ('Missing powercfg alias '+$value)}
  return $m.Groups[1].Value
 }
 $old=Get-ItemPropertyValue 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System' -Name InactivityTimeoutSecs -ErrorAction SilentlyContinue
 $boot=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToUniversalTime().ToString('o')
 $marker='HKLM:\SOFTWARE\Kayfabe\PropimageChecks'
 $priorBoot=Get-ItemPropertyValue $marker -Name InactivityRestartBoot -ErrorAction SilentlyContinue
 $r.restart_required=(($null -ne $old -and $old -ne 0) -or $priorBoot -ceq $boot)
 if($r.restart_required){
  if(-not (Test-Path $marker)){New-Item $marker -Force | Out-Null}
  New-ItemProperty $marker -Name InactivityRestartBoot -PropertyType String -Value $boot -Force | Out-Null
 }
 $null=Power @('/hibernate','off')
 foreach($s in $plan.settings){
  $null=Power @('/setacvalueindex',$scheme,$s.subgroup,$s.setting,'0')
  $null=Power @('/setdcvalueindex',$scheme,$s.subgroup,$s.setting,'0')
 }
 foreach($v in $plan.registry){
  # powercfg is authoritative for hibernation; do not forge its readback by writing HibernateEnabled.
  if($v.name -ne 'hibernate_enabled'){
   if(-not (Test-Path $v.path)){New-Item -Path $v.path -Force | Out-Null}
   New-ItemProperty -Path $v.path -Name $v.key -PropertyType $v.kind -Value $v.value -Force | Out-Null
  }
 }
 $null=Power @('/setactive',$scheme)
 $r.raw_query=Power @('/q',$scheme)
 foreach($s in $plan.settings){
  $sg=Resolve-Guid $s.subgroup;$vg=Resolve-Guid $s.setting
  # Read explicit indices written by powercfg; avoid localised /q labels and hidden-setting omissions.
  $path="HKLM:\SYSTEM\CurrentControlSet\Control\Power\User\PowerSchemes\$scheme\$sg\$vg"
  $ac=Get-ItemPropertyValue $path -Name ACSettingIndex
  $dc=Get-ItemPropertyValue $path -Name DCSettingIndex
  $r.queries[$s.name]=('AC 0x{0:x8} DC 0x{1:x8}' -f $ac,$dc)
  $r.sources[$s.name]='active-scheme registry; powercfg /q retained in raw_query'
 }
 foreach($v in $plan.registry){$r.registry[$v.name]=Get-ItemPropertyValue $v.path -Name $v.key}
 $r.hiberfile_exists=Test-Path ($env:SystemDrive+'\hiberfil.sys')
 if([regex]::Match((Power @('/getactivescheme')),$guid).Value -ine $scheme){throw 'Active scheme changed during power repair'}
}catch{$r.errors+=$_.Exception.Message}
ConvertTo-Json -InputObject $r -Depth 6 -Compress
"""
    )


def render_screen_power(task):
    if not re.fullmatch("power_[a-f0-9]{32}", task):
        raise ValueError("Invalid power task")
    return (
        r"""
$r=@{sid=[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value}
try {
 Add-Type @'
using System; using System.Runtime.InteropServices;
public class KfPower {
 [DllImport("user32.dll",EntryPoint="SystemParametersInfoW",SetLastError=true)] public static extern bool Set(uint action,uint value,IntPtr data,uint flags);
 [DllImport("user32.dll",EntryPoint="SystemParametersInfoW",SetLastError=true)] public static extern bool Get(uint action,uint unused,out int value,uint flags);
}
'@
 foreach($action in @(0x0011,0x0077,0x000f)){
  if(-not [KfPower]::Set($action,0,[IntPtr]::Zero,3)){throw 'Cannot disable interactive screensaver'}
 }
 $active=0;$secure=0;$timeout=0
 if(-not [KfPower]::Get(0x0010,0,[ref]$active,0) -or -not [KfPower]::Get(0x0076,0,[ref]$secure,0) -or -not [KfPower]::Get(0x000e,0,[ref]$timeout,0)){throw 'Cannot read interactive screensaver'}
 $r.active=[bool]$active;$r.secure=[bool]$secure;$r.timeout=$timeout
}catch{$r.error=$_.Exception.Message}
"""
        + f'ConvertTo-Json -InputObject $r -Compress | Set-Content -Encoding UTF8 "C:\\kfhw\\{task}.tmp"\nMove-Item "C:\\kfhw\\{task}.tmp" "C:\\kfhw\\{task}.json"\n'
    )


def power_verdict(data):
    values = {}
    errors = []
    try:
        if type(data.get("errors")) is not list:
            errors.append("Repair error list missing")
        elif data["errors"]:
            errors.extend(str(error) for error in data["errors"])
        for name in POWER_SETTINGS:
            indices = re.fullmatch(
                r"AC 0x([0-9a-fA-F]{8}) DC 0x([0-9a-fA-F]{8})", data["queries"][name]
            )
            if not indices:
                raise ValueError("Missing AC/DC indices for " + name)
            ac, dc = (int(v, 16) for v in indices.groups())
            values[name] = {"ac": ac, "dc": dc}
            if ac != 0 or dc != 0:
                errors.append(name + " not never/off")
        for name, (_, _, expected) in POWER_REGISTRY.items():
            actual = data["registry"][name]
            values[name] = actual
            if type(actual) is not type(expected) or actual != expected:
                errors.append(name + " not disabled")
        values["hiberfile_exists"] = data.get("hiberfile_exists")
        values["restart_required"] = data.get("restart_required")
        screen = data.get("screen", {})
        values["screen_active"] = screen.get("active")
        values["screen_secure"] = screen.get("secure")
        values["screen_timeout"] = screen.get("timeout")
        if data.get("hiberfile_exists") is not False or data.get("restart_required") is not False:
            errors.append("Hibernation file present, or inactivity-policy restart required/unknown")
        if (
            bool(screen.get("error"))
            or not data.get("sid")
            or screen.get("sid") != data["sid"]
            or screen.get("active") is not False
            or screen.get("secure") is not False
            or type(screen.get("timeout")) is not int
            or screen["timeout"] != 0
        ):
            errors.append("Interactive session screensaver readback failed or wrong user")
    except (AttributeError, KeyError, TypeError, ValueError) as exc:
        errors.append(str(exc))
    return {"ok": not errors, "values": values, "errors": errors, "readback": data}


def power_fix(g, sleep=time.sleep, clock=time.monotonic):
    """SSH repairs plus same-user interactive runtime apply/readback; no reboot or credentials."""
    try:
        result = g.ps(render_power_fix())
        if result.returncode:
            return {
                "ok": False,
                "values": {},
                "errors": ["SSH power repair failed: " + result.stderr],
            }
        data = json.loads(result.stdout.lstrip("\ufeff"))
        task = "power_" + uuid.uuid4().hex
        try:
            p = g.interactive(task, render_screen_power(task))
            if p.returncode:
                raise ValueError("Cannot start interactive power readback")
            start = clock()
            while clock() - start < 60:
                p = g.ps(
                    f"if(Test-Path 'C:/kfhw/{task}.json'){{Get-Content 'C:/kfhw/{task}.json' -Raw}}"
                )
                if p.returncode:
                    raise ValueError("Interactive power readback transport failed")
                if p.stdout.strip():
                    data["screen"] = json.loads(p.stdout.lstrip("\ufeff"))
                    break
                sleep(2)
            else:
                raise ValueError("Interactive power readback timed out")
        finally:
            g.ps(
                f"Stop-ScheduledTask -TaskName kf_{task} -ErrorAction SilentlyContinue; Unregister-ScheduledTask -TaskName kf_{task} -Confirm:$false -ErrorAction SilentlyContinue"
            )
        return power_verdict(data)
    except (AttributeError, TypeError, ValueError, OSError, subprocess.SubprocessError) as exc:
        return {"ok": False, "values": {}, "errors": [str(exc)]}


def gpu_uuid_matches(actual, expected):
    return bool(actual.strip()) and actual.strip() == expected


def gpu_fix(g, expected):
    result = g.ps(render_gpu_fix(expected))
    return (
        result.returncode == 0 and gpu_uuid_matches(result.stdout, expected),
        result.stdout + result.stderr,
    )


# Measured anchor: rectangle 1024x640, click at offset (927,602), sp_run.log.
# These are dimensionless fractions, not absolute screen coordinates. New UI versions
# and result-screen OCR remain hypotheses until a hardware lane qualifies them.
SUPERPOSITION = r"""
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
Add-Type @'
using System; using System.Runtime.InteropServices;
public class KfClick {
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern void mouse_event(uint f,int x,int y,uint d,UIntPtr e);
 [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
}
'@
[KfClick]::SetProcessDPIAware() | Out-Null
$cond=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$p.Id)
$wins=[System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$cond)
if($wins.Count -ne 1){throw 'Launcher window missing or ambiguous'}
$rect=$wins[0].Current.BoundingRectangle
if($rect.IsEmpty -or $rect.Width -le 0 -or $rect.Height -le 0){throw 'No UIA window rectangle'}
if(-not [KfClick]::SetForegroundWindow($p.MainWindowHandle)){throw 'Cannot focus launcher'}
Save-Screen "$D\before.png"
$x=[int]($rect.Left+$rect.Width*(927.0/1024.0))
$y=[int]($rect.Top+$rect.Height*(602.0/640.0))
[KfClick]::SetCursorPos($x,$y) | Out-Null
[KfClick]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
[KfClick]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
$deadline=(Get-Date).AddMinutes(10)
while((Get-Date) -lt $deadline){
 if(Test-Path $source){$text=Get-Content $source -Raw;if($text -match 'Benchmark running' -and $text -match 'Benchmark stopped'){break}}
 Start-Sleep 5
}
Save-Screen "$D\result.png"
$f=Get-Item $source
Copy-Item $source "$D\log.html"
$r.kind='FILE';$r.artifact='log.html';$r.modified=Epoch $f.LastWriteTimeUtc;$r.bytes=$f.Length
"""


def extract_screen_text(directory):
    try:
        p = run(["tesseract", str(directory / "result.png"), str(directory / "screen")], timeout=60)
        return not p.returncode
    except (OSError, subprocess.TimeoutExpired):
        return False


def validate_superposition(r, directory, start, end):
    if r.get("artifact") != "log.html" or not start < r.get("modified", 0) <= end:
        raise ValueError("Stale or missing benchmark log")
    raw = (directory / "log.html").read_bytes()
    if not raw or len(raw) != r.get("bytes"):
        raise ValueError("Benchmark log length mismatch")
    log = raw.decode("utf-8-sig")
    running = log.find("Benchmark running")
    stopped = log.find("Benchmark stopped")
    if (
        running < 0
        or stopped <= running
        or log.count("Benchmark running") != 1
        or log.count("Benchmark stopped") != 1
    ):
        raise ValueError("No single completed benchmark interval")
    before = (directory / "before.png").read_bytes()
    after = (directory / "result.png").read_bytes()
    if (
        not before.startswith(b"\x89PNG\r\n\x1a\n")
        or not after.startswith(b"\x89PNG\r\n\x1a\n")
        or before == after
    ):
        raise ValueError("Missing or unchanged result screen")
    text = (directory / "screen.txt").read_text()
    # No guessed score from pixels: require a named positive score and all three FPS fields.
    score = re.search(r"(?i)\bscore\s*[:=]?\s*(\d+)", text)
    fps = [
        re.search(r"(?i)\b" + label + r"\s*[:=]?\s*(\d+(?:\.\d+)?)", text)
        for label in ("min", "avg", "max")
    ]
    if not score or int(score[1]) <= 0 or not all(fps):
        raise ValueError("Result screen score/FPS not readable; no verdict")
    values = [float(x[1]) for x in fps]
    if not 0 < values[0] <= values[1] <= values[2] or not re.search(r"(?i)\bFPS\b", text):
        raise ValueError("Invalid result screen FPS")


def load_image(archive, directory):
    """Check both required members; an already unpacked directory also needs its manifest."""
    import zipfile

    directory = Path(directory)
    if archive:
        with zipfile.ZipFile(archive) as z:
            manifest = json.loads(z.read("manifest.json"))
    else:
        manifest = json.loads((directory / "manifest.json").read_text())
    members = manifest.get("members", {})
    if set(members) != {"image.qcow2", "state.json"} or not all(
        re.fullmatch("[0-9a-f]{64}", x) for x in members.values()
    ):
        raise ValueError(
            "Manifest must hash image.qcow2 and state.json; supply --zip to unpack and verify"
        )
    if archive:
        state = PI.unpack(archive, directory)
        (directory / "manifest.json").write_text(json.dumps(manifest))
    else:
        for name, digest in members.items():
            if PI.sha256_file(directory / name) != digest:
                raise ValueError("Manifest mismatch: " + name)
        state = json.loads((directory / "state.json").read_text())
        PI.validate_state(state, require_password=True)
        if not PI.seed_matches(state):
            raise ValueError("Seed mismatch")
    return state


def stage_gop(work, directory, explicit=None):
    """The signed boot GOP driver is not in state.json: copy it into work/firmware (found by --gop-efi, else
    <dir>/kf-gop.signed.efi or <dir>/firmware/kf-gop.signed.efi). A missing file is a harness error before QEMU starts."""
    dst = Path(work) / "firmware/kf-gop.signed.efi"
    if dst.is_file():
        return dst
    cands = [Path(explicit)] if explicit else [Path(directory) / "kf-gop.signed.efi", Path(directory) / "firmware/kf-gop.signed.efi"]
    for c in cands:
        if c.is_file():
            dst.parent.mkdir(parents=True, exist_ok=True)
            dst.write_bytes(c.read_bytes())
            return dst
    raise ValueError("kf-gop.signed.efi not found (pass --gop-efi): " + ", ".join(map(str, cands)))


def stop_process(proc):
    if proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=10)


class Guest:
    def __init__(self, state, key, port):
        self.key, self.port, self.user = key, port, state["guest"]["user"]

    def ssh(self, argv, stdin=None, timeout=120):
        base = [
            "ssh",
            "-i",
            self.key,
            "-p",
            str(self.port),
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "UserKnownHostsFile=/dev/null",
            "-o",
            "LogLevel=ERROR",
            "-o",
            "ConnectTimeout=20",
            "-o",
            "BatchMode=yes",
            f"{self.user}@127.0.0.1",
        ]
        return run(base + argv, input=stdin, timeout=timeout)

    def ps(self, script, timeout=120):
        """PowerShell. A one-liner goes over stdin; a multi-line script is copied to C:\\kfhw and run with -File (stdin parsing of
        multi-line blocks silently printed nothing on the first hardware run)."""
        if "\n" not in script.strip():
            return self.ssh(
                ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", "-"],
                stdin=script,
                timeout=timeout,
            )
        with tempfile.TemporaryDirectory(prefix="kf-ps-") as td:
            tmp = Path(td) / "script.ps1"
            tmp.write_text(script)
            prep = self.ps(r"New-Item -Force -ItemType Directory C:\kfhw | Out-Null")
            if prep.returncode:
                return prep
            sent = self.scp(str(tmp), "C:/kfhw/ps_run.ps1", True)
            if sent.returncode:
                return sent
            return self.ssh(
                [
                    "powershell",
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                    r"C:\kfhw\ps_run.ps1",
                ],
                timeout=timeout,
            )

    def scp(self, src, dst, to_guest):
        base = [
            "scp",
            "-q",
            "-i",
            self.key,
            "-P",
            str(self.port),
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "UserKnownHostsFile=/dev/null",
            "-o",
            "LogLevel=ERROR",
        ]
        remote = f"{self.user}@127.0.0.1:{dst if to_guest else src}"
        return run(base + ([src, remote] if to_guest else [remote, dst]), timeout=300)

    def interactive(self, name, body, wait_s=0):
        """Run PowerShell `body` in the logged-in desktop session (a scheduled task whose principal is the interactive user)."""
        with tempfile.TemporaryDirectory(prefix="kf-interactive-") as td:
            tmp = Path(td) / (name + ".ps1")
            tmp.write_text(body)
            prep = self.ps(r"New-Item -Force -ItemType Directory C:\kfhw | Out-Null")
            if prep.returncode:
                return prep
            sent = self.scp(str(tmp), "C:/kfhw/" + name + ".ps1", True)
            if sent.returncode:
                return sent
            result = self.ps(task_wrapper(name, self.user))
        if wait_s:
            time.sleep(wait_s)
        return result


# Guest scripts are authored here; selectors must come from a collected UIA tree.
# No authentication, online requests or closed-binary inspection. Only the explicit
# Superposition fallback uses input derived from a measured UIA window rectangle.
PS_LIB = r"""
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
function Save-Screen($path){
 $b=[System.Windows.Forms.SystemInformation]::VirtualScreen
 $bmp=New-Object System.Drawing.Bitmap $b.Width,$b.Height
 $g=[System.Drawing.Graphics]::FromImage($bmp)
 try {$g.CopyFromScreen($b.Left,$b.Top,0,0,$bmp.Size);$bmp.Save($path,[System.Drawing.Imaging.ImageFormat]::Png)} finally {$g.Dispose();$bmp.Dispose()}
}
function Epoch($d) { ([DateTimeOffset]$d.ToUniversalTime()).ToUnixTimeMilliseconds()/1000.0 }
function Dump-Uia($pidValue) {
 $cond=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$pidValue)
 $roots=[System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$cond)
 $rows=New-Object System.Collections.Generic.List[object]
 $queue=New-Object System.Collections.Queue
 foreach($root in $roots){$queue.Enqueue($root)}
 while($queue.Count -gt 0 -and $rows.Count -lt 2048){
  $el=$queue.Dequeue()
  try {
   $c=$el.Current
   $rows.Add(@{Name=$c.Name;AutomationId=$c.AutomationId;ControlType=$c.ControlType.ProgrammaticName;ProcessId=$c.ProcessId;Patterns=@($el.GetSupportedPatterns() | ForEach-Object {$_.ProgrammaticName})})
   foreach($child in $el.FindAll([System.Windows.Automation.TreeScope]::Children,[System.Windows.Automation.Condition]::TrueCondition)){
    if($queue.Count -lt 2048){$queue.Enqueue($child)}
   }
  }catch{}
 }
 ConvertTo-Json -InputObject @($rows.ToArray()) -Depth 6 | Set-Content -Encoding UTF8 "$D\uia.json"
}
function Find-Control($selector,$pidValue) {
 $cond=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$pidValue)
 $roots=[System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$cond)
 $found=@(foreach($root in $roots){
  $all=$root.FindAll([System.Windows.Automation.TreeScope]::Subtree,[System.Windows.Automation.Condition]::TrueCondition)
  foreach($el in $all){$c=$el.Current
   if($c.AutomationId -ceq $selector.id -and (-not $selector.name -or $c.Name -ceq $selector.name) -and $c.ControlType.ProgrammaticName -ceq $selector.type){$el}
  }
 })
 if($found.Count -ne 1){throw 'UIA selector missing or ambiguous'}
 return $found[0]
}
"""

CHECKS = {
    "superposition": {
        "timeout": 900,
        "exe": r"C:\Program Files\Unigine\Superposition Benchmark\bin\launcher.exe",
    },
    "3dmark": {"timeout": 1500, "exe": r"C:\Program Files\UL\3DMark\3DMark.exe"},
    "solidworks": {
        "timeout": 900,
        "exe": r"C:\Program Files\SOLIDWORKS Corp\SOLIDWORKS\SLDWORKS.exe",
    },
    "resolve": {
        "timeout": 1200,
        "exe": r"C:\Program Files\Blackmagic Design\DaVinci Resolve\Resolve.exe",
    },
    "edrawings": {
        "timeout": 300,
        "exe": r"C:\Program Files\SOLIDWORKS Corp\eDrawings\eDrawings.exe",
    },
    "wukong": {
        "timeout": 300,
        "exe": r"C:\Users\kfimage\Desktop\Black Myth Wukong Benchmark Tool.url",
    },
    "edge": {
        "timeout": 300,
        "exe": r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    },
}

SOLIDWORKS = r"""
$sw=New-Object -ComObject SldWorks.Application
$sw.Visible=$true
$part=Get-ChildItem 'C:\Program Files\SOLIDWORKS Corp\SOLIDWORKS\samples' -Recurse -Filter *.sldprt | Select-Object -First 1
if(-not $part){throw 'No sample part; supply a licensed sample in the image'}
$err=0;$warn=0
$model=$sw.OpenDoc6($part.FullName,1,1,'',[ref]$err,[ref]$warn)
if(-not $model -or $err -ne 0){throw 'OpenDoc6 failed'}
$activeErr=0
$model=$sw.ActivateDoc3($part.Name,$false,0,[ref]$activeErr)
if(-not $model -or $activeErr -ne 0){throw 'ActivateDoc3 failed'}
$model.ClearSelection2($true)
$model.ViewZoomtofit2()
$model.GraphicsRedraw2()
$ok=$model.Extension.SaveAs("$D\model.step",0,1,$null,[ref]$err,[ref]$warn)
$f=Get-Item "$D\model.step"
$r.kind='FILE';$r.artifact='model.step';$r.modified=Epoch $f.LastWriteTimeUtc;$r.bytes=$f.Length
$r.api_ok=([bool]$ok -and $err -eq 0)
$sw.CloseDoc($model.GetTitle())
$sw.ExitApp()
"""

# UIA is deliberately data-driven: no invented AutomationIds or completion labels.
UIA_RUN = r"""
$status=Find-Control $plan.status $p.Id
$r.before=$status.Current.Name
if($r.before -ceq $plan.finished){throw 'Finished state predates this run'}
$button=Find-Control $plan.run $p.Id
$invoke=$button.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
$invoke.Invoke()
$deadline=(Get-Date).AddSeconds($plan.timeout)
$r.progress='';$r.after=''
while((Get-Date) -lt $deadline){
 $status=Find-Control $plan.status $p.Id
 $text=$status.Current.Name
 if($text -ceq $plan.running){$r.progress=$text}
 if($r.progress -and $text -ceq $plan.finished){$r.after=$text;break}
 Start-Sleep -Milliseconds 500
}
$r.kind='UIA'
$r.running=$plan.running;$r.finished=$plan.finished
"""


def env_prefix(env):
    """Old coordinate calibration is intentionally refused."""
    if env:
        raise ValueError("Coordinate calibration removed; collect UIA selectors instead")
    return ""


def validate_plan(plan):
    if not isinstance(plan, dict) or set(plan) != {
        "run",
        "status",
        "running",
        "finished",
        "timeout",
    }:
        raise ValueError("UIA plan needs run/status selectors, running/finished text and timeout")
    for key in ("run", "status"):
        sel = plan[key]
        if not isinstance(sel, dict) or set(sel) != {"id", "name", "type"}:
            raise ValueError("Selector needs id/name/type")
        if not all(isinstance(v, str) and len(v) <= 256 for v in sel.values()):
            raise ValueError("Invalid selector")
        if not sel["type"].startswith("ControlType."):
            raise ValueError("Invalid control type")
    if not all(
        isinstance(plan[k], str) and 0 < len(plan[k]) <= 256 for k in ("running", "finished")
    ):
        raise ValueError("Missing state text")
    if (
        plan["running"] == plan["finished"]
        or type(plan["timeout"]) is not int
        or not 1 <= plan["timeout"] <= 1200
    ):
        raise ValueError("Invalid state transition or timeout")
    return plan


def render_check(
    name,
    env=None,
    run_id="0" * 32,
    plan=None,
    accessibility_experiment=False,
    solidworks_export=False,
    superposition_run=False,
):
    spec = CHECKS[name]
    env_prefix(env or {})
    if not re.fullmatch("[a-f0-9]{32}", run_id):
        raise ValueError("Invalid run id")
    directory = GUEST_DIR + "\\" + run_id + "\\" + name
    script = PS_LIB + f"\n$D='{directory}'\n"
    script += "New-Item -ItemType Directory -Path $D -ErrorAction Stop | Out-Null\n"
    script += (
        f"$r=@{{app='{name}';run_id='{run_id}';started=(Epoch (Get-Date));kind='PROBE'}}\ntry {{\n"
    )
    if name == "solidworks" and solidworks_export:
        script += SOLIDWORKS
    else:
        if name == "superposition" and superposition_run:
            script += (
                r"$source=Join-Path $env:USERPROFILE 'Superposition\log.html'; Remove-Item $source -ErrorAction SilentlyContinue"
                + "\n"
            )
        args = (
            " -ArgumentList '--force-renderer-accessibility'"
            if name == "3dmark" and accessibility_experiment
            else ""
        )
        script += f"$p=Start-Process '{spec['exe']}'{args} -PassThru; Start-Sleep 30\n"
        if name != "wukong":
            script += "$p.Refresh();$r.kind='LAUNCH_ONLY';$r.launched=(-not $p.HasExited);$r.title=$p.MainWindowTitle\n"
        # URI launchers need a process tree from hardware before their UI can be scoped safely.
        if name == "wukong":
            script += r"""$fresh=@(Get-Process -Name b1,b1-Win64-Shipping,BlackMythWukong -ErrorAction SilentlyContinue | Where-Object {(Epoch $_.StartTime) -gt $r.started})
$r.kind='LAUNCH_ONLY';$r.launched=($fresh.Count -gt 0);$r.process_ids=@($fresh | ForEach-Object {$_.Id})
$r.note='Launch probe only; no benchmark verdict or Steam interaction'
"""
        else:
            script += "try {Dump-Uia $p.Id} catch {$r.uia_error=$_.Exception.Message}\n"
            if name == "superposition" and superposition_run:
                script += SUPERPOSITION
            elif plan:
                validate_plan(plan)
                literal = json.dumps(plan).replace("'", "''")
                script += f"$plan=ConvertFrom-Json '{literal}'\n" + UIA_RUN
            else:
                script += "$r.note='UIA tree collected; verified run controls and result schema pending'\n"
    script += '} catch { $r.error=$_.Exception.Message } finally {\ntry {Save-Screen "$D\\screen.png"} catch {$r.screen_error=$_.Exception.Message}\n$r.ended=Epoch (Get-Date)\n'
    script += 'ConvertTo-Json -InputObject $r -Depth 6 | Set-Content -Encoding UTF8 "$D\\result.tmp"\nMove-Item "$D\\result.tmp" "$D\\result.json"\n}\n'
    return script


def validate_transition(record, running, finished):
    return (
        isinstance(running, str)
        and bool(running)
        and isinstance(finished, str)
        and bool(finished)
        and running != finished
        and record.get("before") not in (None, "", running, finished)
        and record.get("progress") == running
        and record.get("after") == finished
    )


def verdict(name, run_id, directory, plan=None):
    """Grade pulled bytes, never a guest-authored ok flag. Guest clocks stay guest-relative."""
    row = {"app": name, "ok": False, "evidence": "NONE", "signal": "", "secs": 0}
    try:
        r = json.loads((directory / "result.json").read_text(encoding="utf-8-sig"))
        if r.get("app") != name or r.get("run_id") != run_id:
            raise ValueError("Result identity mismatch")
        start, end = r["started"], r["ended"]
        if (
            not all(type(v) in (int, float) and math.isfinite(v) for v in (start, end))
            or not start < end
        ):
            raise ValueError("Invalid guest timestamps")
        row["secs"] = int(end - start)
        if name == "wukong":
            row["launched"] = r.get("launched") is True
        if r.get("error"):
            raise ValueError(str(r["error"]))
        if r.get("kind") == "FILE" and name == "solidworks":
            artifact = r.get("artifact")
            if artifact != "model.step":
                raise ValueError("Unexpected artifact path")
            modified = r.get("modified")
            if type(modified) not in (int, float) or not start < modified <= end:
                raise ValueError("Stale artifact")
            raw = (directory / artifact).read_bytes()
            if not raw or len(raw) != r.get("bytes"):
                raise ValueError("Artifact length mismatch")
            if r.get("api_ok") is not True:
                raise ValueError("API export failed")
            text = raw.decode("ascii")
            if not (
                text.startswith("ISO-10303-21;")
                and text.rstrip().endswith("END-ISO-10303-21;")
                and "DATA;" in text
                and re.search(r"#[0-9]+\s*=", text)
            ):
                raise ValueError("Invalid STEP exchange structure")
            row.update(
                ok=True,
                evidence="FILE",
                signal="API exported STEP with entity data (not a GPU/render benchmark)",
            )
        elif r.get("kind") == "UIA" and plan and name != "wukong":
            validate_plan(plan)
            if not validate_transition(r, plan["running"], plan["finished"]):
                raise ValueError("No fresh progress-to-finished transition")
            row.update(
                ok=True,
                evidence="UIA",
                signal="Observed progress then finished; selector semantics require hardware qualification",
            )
        elif r.get("kind") == "FILE" and name == "superposition":
            validate_superposition(r, directory, start, end)
            row.update(
                ok=True,
                evidence="FILE",
                signal="Fresh benchmark log plus OCR-checked result screen; SAVE not claimed",
            )
        else:
            if r.get("kind") == "LAUNCH_ONLY" and r.get("launched") is True:
                row["status"] = "LAUNCH_ONLY"
            raise ValueError(r.get("note", "No supported work evidence"))
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as exc:
        row["error"] = str(exc)
    return row


def task_wrapper(name, user="kfimage"):
    """Runs C:\\kfapps\\<name>.ps1 in the interactive session, once, and removes the task (no -RunLevel surprises)."""
    if not re.fullmatch(r"[a-z0-9_]+", name):
        raise ValueError("Invalid task name")
    if not re.fullmatch(r"[A-Za-z][A-Za-z0-9_-]{0,19}", user):
        raise ValueError("Invalid interactive user")
    return (
        f"$n='kf_{name}'; Unregister-ScheduledTask -TaskName $n -Confirm:$false -ErrorAction SilentlyContinue\n"
        f"$a=New-ScheduledTaskAction -Execute powershell.exe -Argument '-NoProfile -ExecutionPolicy Bypass -File C:\\kfhw\\{name}.ps1'\n"
        f"$p=New-ScheduledTaskPrincipal -UserId {user} -LogonType Interactive -RunLevel Highest\n"
        "Register-ScheduledTask -TaskName $n -Action $a -Principal $p | Out-Null; Start-ScheduledTask -TaskName $n; 'started'\n"
    )


def run_check(name, env, out, guest, plan=None, sleep=time.sleep, clock=time.monotonic, **options):
    """Run in a unique guest directory; pull only authored, bounded artifact names."""
    run_id = uuid.uuid4().hex
    remote = GUEST_DIR.replace("\\", "/") + "/" + run_id + "/" + name
    d = out / name / run_id
    d.mkdir(parents=True)
    tmp = d / (name + ".ps1")
    tmp.write_text(render_check(name, env, run_id, plan, **options))
    task = "app_" + run_id
    t0 = clock()
    try:
        p = guest.interactive(task, tmp.read_text())
        if p.returncode:
            raise ValueError("Task start failed")
        while clock() - t0 < CHECKS[name]["timeout"]:
            p = guest.ps(f"if(Test-Path '{remote}/result.json'){{'ready'}}")
            if p.returncode == 0 and p.stdout.strip() == "ready":
                break
            if p.returncode:
                raise ValueError("Guest transport lost; not an app failure")
            sleep(5)
        else:
            raise ValueError("Task timeout; no verdict")
        # Native exchange file, UIA tree and metadata, never guest-selected filenames or executables.
        for f in (
            "result.json",
            "uia.json",
            "model.step",
            "log.html",
            "before.png",
            "result.png",
            "screen.png",
        ):
            limit = 16 * 1024 * 1024 if f == "model.step" or f.endswith(".png") else 1024 * 1024
            p = guest.ps(
                f"if(Test-Path '{remote}/{f}'){{$f=Get-Item '{remote}/{f}';if($f.Length -le {limit}){{'bounded'}}}}",
            )
            if (
                p.returncode == 0
                and p.stdout.strip() == "bounded"
                and guest.scp(remote + "/" + f, str(d / f), False).returncode
            ):
                raise ValueError("Artifact transfer failed")
        if name == "superposition" and (d / "result.png").exists() and not extract_screen_text(d):
            raise ValueError(
                "Harness prerequisite: result-screen OCR unavailable or failed; no app verdict"
            )
        row = verdict(name, run_id, d, plan)
    except (ValueError, TypeError, OSError, subprocess.TimeoutExpired) as exc:
        row = {"class": "harness", "app": name, "ok": False, "evidence": "NONE", "error": str(exc)}
    finally:
        # Stop only this authored task, never process-name sweeps.
        try:
            guest.ps(
                f"Stop-ScheduledTask -TaskName kf_{task} -ErrorAction SilentlyContinue; Unregister-ScheduledTask -TaskName kf_{task} -Confirm:$false -ErrorAction SilentlyContinue",
            )
        except (OSError, subprocess.TimeoutExpired):
            pass
    (d / "verdict.json").write_text(json.dumps(row, indent=2))
    return row


def table(rows):
    lines = ["app\tok\tevidence\tsecs\tsignal / error"]
    for r in rows:
        lines.append(
            "\t".join(
                str(v).replace("\t", " ").replace("\n", " ")
                for v in (
                    r["app"],
                    "yes" if r.get("ok") else "NO",
                    r.get("evidence", "NONE"),
                    r.get("secs", 0),
                    r.get("error") or r.get("signal", ""),
                )
            )
        )
    return "\n".join(lines)


def result_markdown(rows):
    lines = ["| step | verdict | evidence | class | detail |", "|---|---|---|---|---|"]
    for item in rows:
        lines.append(
            "| "
            + " | ".join(
                str(item[k]).replace("|", "/").replace("\n", " ")
                for k in ("step", "verdict", "evidence", "class", "detail")
            )
            + " |"
        )
    return "\n".join(lines)


def app_classification(app):
    if app.get("class") == "harness":
        return "HARNESS", "harness"
    if app.get("ok") is True:
        return "PASS", "app"
    return ("LAUNCH_ONLY", "unqualified") if app.get("status") == "LAUNCH_ONLY" else ("FAIL", "app")


def overlay_clock(overlay, frozen, guest_utc=None, resume=False):
    """Persist the RTC base, monotonic guest high-water mark and start counter."""
    path = Path(str(overlay) + ".clock")
    try:
        data = json.loads(path.read_text())
        base = PI.utc(data["base_utc"])
        last = PI.utc(data["last_guest_utc"]) if data["last_guest_utc"] else None
        if type(data["boots"]) is not int or data["boots"] < 0:
            raise ValueError("invalid boots")
    except (OSError, ValueError, TypeError, KeyError, PI.PropError):
        if resume or path.exists():
            warnings.warn(
                "Overlay clock missing/corrupt: frozen fallback", RuntimeWarning, stacklevel=2
            )
        data, base, last = {"boots": 0}, frozen, None
    base = max(frozen, base)
    if guest_utc is None:
        base = max(base, last + timedelta(seconds=60) if last else frozen)
        data["boots"] += 1
    else:
        last = max(PI.utc(guest_utc), base, last or base)
    data.update(base_utc=PI.fmt(base), last_guest_utc=PI.fmt(last) if last else None)
    temporary = Path(str(path) + ".tmp")
    temporary.write_text(json.dumps(data))
    temporary.replace(path)
    return base


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--zip", help="Private image archive already present locally; no downloads")
    ap.add_argument("--dir", required=True, help="Verified unpack directory")
    ap.add_argument("--work", required=True, help="Fresh directory; --resume reuses a stopped run")
    ap.add_argument("--out", required=True)
    ap.add_argument("--qemu", required=True)
    ap.add_argument("--stop-utc")
    ap.add_argument("--resume", action="store_true", help="Manual: reuse stopped --work; no --zip")
    ap.add_argument("--flags", default=str(HERE / "kf3_flags.sample.env"))
    ap.add_argument("--broker", default=str(HERE.parents[2] / "broker/nvkvm-display-broker"))
    ap.add_argument("--gop-efi", help="signed boot GOP driver to stage into the work directory")
    ap.add_argument("--display", choices=("broker", "off"), default="broker")
    ap.add_argument("--apps", default=",".join(CHECKS))
    ap.add_argument("--plans", type=Path)
    ap.add_argument(
        "--experiment-3dmark-accessibility",
        action="store_true",
        help="UNPROVEN Chromium accessibility experiment, not an established fix",
    )
    ap.add_argument(
        "--solidworks-export",
        action="store_true",
        help="Unqualified COM sample/STEP work probe; otherwise LAUNCH_ONLY",
    )
    ap.add_argument(
        "--superposition-run",
        action="store_true",
        help="Repeat measured rectangle-derived RUN click; requires local tesseract for result-screen validation",
    )
    a = ap.parse_args(argv)
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    rows = []

    def record(step, status, detail="", evidence="NONE", classification="harness"):
        item = dict(
            step=step,
            verdict=status,
            detail=str(detail)[:1000],
            evidence=evidence,
            **{"class": classification},
        )
        rows.append(item)
        print(step, status, evidence, str(detail)[:200], flush=True)

    try:
        if a.resume and a.zip:
            raise ValueError("--resume needs the verified unpacked --dir, without --zip")
        names = list(CHECKS) if a.apps == "all" else a.apps.split(",")
        # Preserve the measured predecessor's app spelling as an input alias only.
        names = ["wukong" if n == "blackmyth" else n for n in names]
        if any(n not in CHECKS for n in names) or len(set(names)) != len(names):
            raise ValueError("Unknown or duplicate app")
        plans = json.loads(a.plans.read_text()) if a.plans else {}
        for name, plan in plans.items():
            if name not in CHECKS or name == "wukong":
                raise ValueError("Unsupported plan app")
            validate_plan(plan)
        swtpm = os.environ.get("SWTPM", "swtpm")
        try:
            version = run([swtpm, "--version"], timeout=30)
        except FileNotFoundError as exc:
            raise ValueError(
                "Prerequisite: swtpm >= 0.10 not found; set SWTPM to a compatible binary"
            ) from exc
        if version.returncode:
            raise ValueError("Prerequisite: cannot query swtpm version; swtpm >= 0.10 required")
        require_swtpm(version.stdout + version.stderr)
        record("prerequisite", "PASS", "swtpm >= 0.10 (libtpms state compatibility still required)")
        state = load_image(a.zip, a.dir)
        record("unpack", "PASS", "SHA256 of image and state; derived seed verified")
        if a.stop_utc:
            if PI.utc(a.stop_utc) < PI.utc(state["clock"]["last_time_at_stop_utc"]):
                raise ValueError("Refusing backwards stop-time override")
            state["clock"]["last_time_at_stop_utc"] = PI.fmt(PI.utc(a.stop_utc))
        work = Path(a.work).resolve()
        work.mkdir(parents=True, exist_ok=a.resume)
        (work / "run").mkdir(exist_ok=a.resume)
        base = work / "image.base.qcow2"
        clone = work / "run/clone.qcow2"
        if a.resume:
            if not clone.is_file() or base.resolve() != (Path(a.dir) / "image.qcow2").resolve():
                raise ValueError("Resumed overlay/base missing or belongs to another image")
        else:
            PI.restore_files(state, str(work))
            base.symlink_to((Path(a.dir) / "image.qcow2").resolve())
            create = ["qemu-img", "create", "-q", "-f", "qcow2"]
            made = run(create + ["-b", str(base), "-F", "qcow2", str(clone)])
            if made.returncode:
                raise ValueError("Overlay creation failed: " + made.stderr)
        info = run(["qemu-img", "info", "--output=json", str(clone)])
        if (
            info.returncode
            or Path(json.loads(info.stdout).get("full-backing-filename", "")) != base
        ):
            raise ValueError("Overlay backing-file mismatch")
        record("overlay", "PASS", "throwaway clone of image.base.qcow2; base not booted")
        if a.resume:
            for artifact in ("swtpm.sock", "run/broker.sock", "run/broker.fifo"):
                (work / artifact).unlink(missing_ok=True)
        frozen = PI.boot_clock(state, "ci")
        rtc_base = overlay_clock(clone, frozen, resume=a.resume)
        # Same adjusted copy feeds both QEMU's RTC and compare()'s 1800-second window.
        state["clock"]["last_time_at_stop_utc"] = PI.fmt(
            rtc_base - timedelta(seconds=state["clock"]["gap_s"])
        )
        (work / "state.json").write_text(json.dumps(state))
        kind = "resumed" if a.resume else "fresh"
        record("clock", "PASS", f"RTC base {PI.fmt(rtc_base)} ({kind} overlay)")
        env = dict(os.environ)
        for line in Path(a.flags).read_text().splitlines():
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            key, sep, value = line.partition("=")
            if not sep or not re.fullmatch(r"KF3_[A-Z0-9_]+", key) or not value.isdigit():
                raise ValueError("Refused KF3 flag line")
            env[key] = value
        if any(
            row[1] != "MATCH" for row in PI.cpu_preflight(state, Path("/proc/cpuinfo").read_text())
        ):
            raise ValueError("CPU_PROFILE_UNSUPPORTED")
        with ExitStack() as stack:
            r = work / "run"
            sw = subprocess.Popen(
                [
                    swtpm,
                    "socket",
                    "--tpm2",
                    "--tpmstate",
                    f"dir={work}/tpm,mode=0600",
                    "--ctrl",
                    f"type=unixio,path={work}/swtpm.sock,mode=0600",
                    "--log",
                    f"file={r}/swtpm.log,level=20",
                    "--terminate",
                ],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            stack.callback(stop_process, sw)
            for _ in range(50):
                if (work / "swtpm.sock").exists() or sw.poll() is not None:
                    break
                time.sleep(0.2)
            if not (work / "swtpm.sock").exists():
                raise ValueError(
                    "swtpm failed to load image state; verify swtpm/libtpms compatibility"
                )
            sock = str(r / "broker.sock")
            kw = {}
            if a.display == "broker":
                stage_gop(work, a.dir, a.gop_efi)
                fifo = r / "broker.fifo"
                os.mkfifo(fifo)
                fd = os.open(fifo, os.O_RDWR)
                stack.callback(os.close, fd)
                log = stack.enter_context(open(r / "broker.log", "w"))
                bp = subprocess.Popen(
                    [
                        a.broker,
                        "--socket",
                        sock,
                        "--backend",
                        "test",
                        "--persist",
                        "--verbose",
                        "--size",
                        "1920x1080",
                    ],
                    stdin=fd,
                    stdout=log,
                    stderr=subprocess.STDOUT,
                )
                stack.callback(stop_process, bp)
                for _ in range(50):
                    if Path(sock).exists() or bp.poll() is not None:
                        break
                    time.sleep(0.2)
                if not Path(sock).exists():
                    raise ValueError("Display broker did not become ready")
                kw = {
                    "broker_sock": sock,
                    "gop_efi": str(work / "firmware/kf-gop.signed.efi"),
                    "uid": 0,
                }
            command = PI.build_argv(
                state, "ci", str(clone), str(work), qemu=a.qemu, display=a.display, **kw
            )
            if "restrict=on" not in " ".join(command) or str(clone) not in " ".join(command):
                raise ValueError("Offline overlay invariant violated")
            (r / "last_cmd.txt").write_text(shlex.join(command) + "\n")
            log = stack.enter_context(open(r / "qemu.log", "w"))
            q = subprocess.Popen(
                command, env=env, stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL
            )
            stack.callback(stop_process, q)
            booted = time.monotonic()
            record("boot", "PASS", "ci mode, restrict=on, disposable overlay")
            key = work / "id_kf"
            key.write_text(state["guest"]["ssh_private_key"])
            key.chmod(0o600)
            stack.callback(key.unlink, missing_ok=True)
            g = Guest(state, str(key), state["ids"]["net"]["ssh_port"])
            try:
                while time.monotonic() - booted < 420 and q.poll() is None:
                    if g.ssh(["echo", "up"], timeout=40).returncode == 0:
                        break
                    time.sleep(5)
                else:
                    tail = ""
                    if q.poll() is not None:
                        tail = " (QEMU exited rc=%s: %s)" % (q.returncode, Path(r / "qemu.log").read_text()[-300:].strip())
                    raise ValueError("Guest SSH unavailable" + tail)
                record("ssh", "PASS", "loopback guest transport ready")
                time.sleep(60)
                power = power_fix(g)
                (out / "power.json").write_text(json.dumps(power, indent=2))
                power_detail = (
                    ("OK" if power["ok"] else "NOT OK")
                    + " values="
                    + json.dumps(power["values"], separators=(",", ":"))
                )
                if power["errors"]:
                    power_detail += " errors=" + ("; ".join(power["errors"]))
                record("power", "OK" if power["ok"] else "NOT OK", power_detail, "FILE")
                if not power["ok"]:
                    raise ValueError(
                        "Power prerequisites NOT OK; app checks not run (see power.json)"
                    )
                fixed, detail = gpu_fix(g, state["ids"]["gpu_uuid"])
                (out / "gpu_fix.txt").write_text(detail)
                record(
                    "gpu",
                    "PASS" if fixed else "FAIL",
                    "post-fix seed UUID exact match" if fixed else "repair or UUID check failed",
                )
                if not fixed:
                    raise ValueError("GPU prerequisite failed; app checks not run")
                facts = PI.run_guest_probe(state, str(key))
                matches = PI.compare(state, facts)
                (out / "verify.txt").write_text(
                    "\n".join(" ".join(str(v) for v in row) for row in matches)
                )
                if any(row[1] == "MISMATCH" for row in matches):
                    raise ValueError("Blocking identity mismatch; app checks not run")
                record("verify", "PASS", "identity, clock, TPM and seed GPU checked")
                for name in names:
                    if boot_age_guard(time.monotonic() - booted, CHECKS[name]["timeout"]):
                        record(
                            "app:" + name,
                            "NOT_RUN",
                            "Boot-age warning: check budget would cross 55 minutes; fresh boot required",
                        )
                        continue
                    events = shutdown_events(g)
                    (out / "shutdown_events.json").write_text(json.dumps(events, indent=2))
                    if classify_shutdown(events) == "harness":
                        record(
                            "license",
                            "HARNESS",
                            "wlms.exe event 1074: evaluation shutdown, not app FAIL",
                        )
                        break
                    app = run_check(
                        name,
                        {},
                        out,
                        g,
                        plan=plans.get(name),
                        accessibility_experiment=a.experiment_3dmark_accessibility,
                        solidworks_export=a.solidworks_export,
                        superposition_run=a.superposition_run,
                    )
                    try:
                        events = shutdown_events(g)
                        (out / "shutdown_events.json").write_text(json.dumps(events, indent=2))
                        license_shutdown = classify_shutdown(events) == "harness"
                        if license_shutdown:
                            record(
                                "app:" + name, "HARNESS", "wlms.exe event 1074; not an app failure"
                            )
                            break
                    except (ValueError, TypeError, OSError, subprocess.TimeoutExpired):
                        record(
                            "app:" + name,
                            "HARNESS",
                            "Guest event log/transport lost; shutdown cause unverified, not app FAIL",
                        )
                        break
                    status, classification = app_classification(app)
                    record(
                        "app:" + name,
                        status,
                        app.get("error") or app.get("signal", ""),
                        app["evidence"],
                        classification,
                    )
            finally:
                try:
                    utc = g.ps("(Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')")
                    if utc.returncode:
                        raise ValueError("Guest UTC unavailable before shutdown")
                    overlay_clock(clone, frozen, guest_utc=utc.stdout.strip())
                except (PI.PropError, ValueError, OSError, subprocess.SubprocessError) as exc:
                    record("clock-save", "WARN", "Guest clock not saved: " + str(exc))
                try:
                    g.ps("Stop-Computer -Force", timeout=30)
                except (OSError, subprocess.TimeoutExpired):
                    pass
                try:
                    q.wait(timeout=180)
                except subprocess.TimeoutExpired:
                    stop_process(q)
                record(
                    "stop",
                    "PASS" if q.poll() is not None else "FAIL",
                    "own QEMU stopped; other own helpers cleaned by ExitStack",
                )
    except (
        PI.PropError,
        ValueError,
        TypeError,
        OSError,
        KeyError,
        subprocess.SubprocessError,
    ) as exc:
        record("harness", "HARNESS", exc)
    (out / "result.json").write_text(json.dumps(rows, indent=2))
    (out / "result.md").write_text(result_markdown(rows) + "\n")
    return 0 if rows and all(r["verdict"] in ("PASS", "OK") for r in rows) else 1


if __name__ == "__main__":
    sys.exit(main())
