#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Unattended app checks for the proprietary Windows image (run on the machine that hosts the VM).

STATUS: DESIGN-ONLY, 2026-11-13. The plumbing (overlay, boot, ssh wait, scheduled-task runner, screenshots, result table,
shutdown) is unit-tested without a VM (scripts/ci/test_propimage_apps.py). NO app check below has been run on hardware:
every `signal` is a hypothesis with a stated falsifier, not a measured exit signal. Design: V3_WINDOWS_PROPRIETARY_IMAGE.md.

Rules it enforces (owner, binding): always OFFLINE (propimage_run.sh `offline`: slirp restrict=on, faked RTC), always a
throwaway overlay of image.base.qcow2 (re-created before each run), never writes the base, never signs in anywhere.

  propimage_apps.py run   [--apps a,b] [--out DIR]   boot, check, pull, shut down, print the table
  propimage_apps.py shot  OUT.png                    one screenshot of the live VM (calibration)
  propimage_apps.py guest 'powershell text'          one command in the guest (calibration)
"""
import argparse
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

PROP_DIR = Path(os.environ.get("PROP_DIR", "/workspace/bench/propimage_20261112"))
KEY = PROP_DIR / "keys/id_ed25519"
RUN_SH = Path(__file__).with_name("propimage_run.sh")
USER, PORT = "kfimage", 33037
SSH_OPTS = ["-i", str(KEY), "-p", str(PORT), "-o", "IdentitiesOnly=yes", "-o", "StrictHostKeyChecking=no",
            "-o", "UserKnownHostsFile=/dev/null", "-o", "BatchMode=yes", "-o", "LogLevel=ERROR", "-o", "ConnectTimeout=10"]
GUEST_DIR = "C:\\kfapps"

# ------------------------------------------------------------------ guest-side PowerShell (generated, logic in Python)
# Input injection: SendInput from inside the interactive session (a scheduled task with the kfimage interactive
# principal). Hypothesis H-input: this works unattended. Falsifier: a click on a known window does not change it.
PS_LIB = r'''
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public class KfIn {
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern void mouse_event(uint f,int dx,int dy,uint d,UIntPtr e);
 [DllImport("user32.dll")] public static extern void keybd_event(byte vk,byte sc,uint f,UIntPtr e);
 [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
}
"@
[KfIn]::SetProcessDPIAware() | Out-Null
$D = 'C:\kfapps'
function Shot($n){ $b=[System.Windows.Forms.SystemInformation]::VirtualScreen; $bmp=New-Object System.Drawing.Bitmap $b.Width,$b.Height
  $g=[System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($b.Left,$b.Top,0,0,$bmp.Size); $bmp.Save("$D\$n.png",[System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose() }
function Click($x,$y){ [KfIn]::SetCursorPos($x,$y)|Out-Null; Start-Sleep -m 150; [KfIn]::mouse_event(2,0,0,0,[UIntPtr]::Zero); Start-Sleep -m 80; [KfIn]::mouse_event(4,0,0,0,[UIntPtr]::Zero) }
function Key($vk){ [KfIn]::keybd_event($vk,0,0,[UIntPtr]::Zero); Start-Sleep -m 60; [KfIn]::keybd_event($vk,0,2,[UIntPtr]::Zero) }
function Say($t){ "$(Get-Date -f s) $t" | Add-Content "$D\log.txt" }
function Titles(){ (Get-Process | ? { $_.MainWindowTitle } | % { $_.ProcessName + '|' + $_.MainWindowTitle }) -join ';' }
function Kill($re){ Get-Process | ? { $_.ProcessName -match $re } | Stop-Process -Force -ErrorAction SilentlyContinue }
'''

# One check = a PowerShell body (runs after PS_LIB, in the interactive session) that must write C:\kfapps\<name>.json
# {"ok":bool,"signal":"...", ...}. `signal` is the exit signal that shows the app did real work.
CHECKS = {
    "superposition": dict(timeout=900, body=r'''
$exe='C:\Program Files\Unigine\Superposition Benchmark\bin\launcher.exe'; $t0=Get-Date
Start-Process $exe; Start-Sleep 20; Shot sp_launcher
# H-sp: Custom preset, windowed, then RUN; coordinates are calibrated per DPI (SP_* below, filled by calibration)
foreach($c in $env:SP_CLICKS -split ';'){ if($c){ $xy=$c -split ','; Click ([int]$xy[0]) ([int]$xy[1]); Start-Sleep 2 } }
Start-Sleep 5; Shot sp_run_start
$deadline=(Get-Date).AddMinutes(10); $eng=$false
while((Get-Date) -lt $deadline){ if(Get-Process -Name superposition -ErrorAction SilentlyContinue){$eng=$true}elseif($eng){break}; Start-Sleep 5 }
Start-Sleep 5; Shot sp_result
$f=Get-ChildItem "$env:USERPROFILE\Superposition" -Recurse -ErrorAction SilentlyContinue | ? { $_.LastWriteTime -gt $t0 -and $_.Name -match 'log' }
$log=($f | % { Get-Content $_.FullName -Raw -ErrorAction SilentlyContinue }) -join "`n"
$stopped = $log -match 'Benchmark stopped'; $gpu = $log -match 'NVIDIA'
@{ok=($eng -and $stopped -and $gpu); engine_ran=$eng; benchmark_stopped=$stopped; renderer_nvidia=$gpu; secs=[int]((Get-Date)-$t0).TotalSeconds;
  signal='engine process ran, log has Benchmark stopped and an NVIDIA renderer line; FPS is read from sp_result.png'} | ConvertTo-Json | Set-Content "$D\superposition.json"
Kill 'superposition|launcher'
'''),
    "3dmark": dict(timeout=1500, body=r'''
$t0=Get-Date; Start-Process 'C:\Program Files\UL\3DMark\3DMark.exe'; Start-Sleep 45; Shot tdm_home
# H-3dm: Time Spy offline, then SAVE. Signal = a result file (.3dmark-result) newer than $t0 under Documents\3DMark or AppData.
foreach($c in $env:TDM_CLICKS -split ';'){ if($c){ $xy=$c -split ','; Click ([int]$xy[0]) ([int]$xy[1]); Start-Sleep 3 } }
$deadline=(Get-Date).AddMinutes(20); $f=$null
while((Get-Date) -lt $deadline -and -not $f){ Start-Sleep 20
  $f=Get-ChildItem "$env:USERPROFILE\Documents\3DMark","$env:LOCALAPPDATA\UL","$env:PROGRAMDATA\UL" -Recurse -ErrorAction SilentlyContinue | ? { $_.LastWriteTime -gt $t0 -and $_.Extension -match '3dmark-result|3dr|xml' } | select -First 1 }
Shot tdm_result
@{ok=[bool]$f; file=$f.FullName; bytes=$f.Length; secs=[int]((Get-Date)-$t0).TotalSeconds; titles=(Titles);
  signal='saved result file newer than start'} | ConvertTo-Json | Set-Content "$D\3dmark.json"
Kill '3DMark'
'''),
    "3dmark_settings": dict(timeout=300, body=r'''
Start-Process 'C:\Program Files\UL\3DMark\3DMark.exe'; Start-Sleep 45
foreach($c in $env:TDM_SET_CLICKS -split ';'){ if($c){ $xy=$c -split ','; Click ([int]$xy[0]) ([int]$xy[1]); Start-Sleep 3 } }
Shot tdm_settings
@{ok=[bool](Get-Process 3DMark -ErrorAction SilentlyContinue); titles=(Titles); signal='3DMark window up, settings page screenshot (read by eye)'} | ConvertTo-Json | Set-Content "$D\3dmark_settings.json"
Kill '3DMark'
'''),
    "solidworks": dict(timeout=900, body=r'''
$t0=Get-Date; $p=Start-Process 'C:\Program Files\SOLIDWORKS Corp\SOLIDWORKS\SLDWORKS.exe' -PassThru; Start-Sleep 150; Shot sw_start
$part=(Get-ChildItem 'C:\Program Files\SOLIDWORKS Corp\SOLIDWORKS\samples' -Recurse -Filter *.sldprt -ErrorAction SilentlyContinue | select -First 1)
if($part){ Start-Process $part.FullName; Start-Sleep 60 }; Shot sw_part
$cx=[int]$env:SW_CX; $cy=[int]$env:SW_CY; if($cx){ [KfIn]::SetCursorPos($cx,$cy)|Out-Null; Start-Sleep -m 200; [KfIn]::mouse_event(0x20,0,0,0,[UIntPtr]::Zero); for($i=0;$i -lt 40;$i++){ [KfIn]::mouse_event(1,6,0,0,[UIntPtr]::Zero); Start-Sleep -m 40 }; [KfIn]::mouse_event(0x40,0,0,0,[UIntPtr]::Zero) }
Start-Sleep 3; Shot sw_rotated; $ti=Titles
@{ok=[bool]($ti -match 'SOLIDWORKS' -and $part); titles=$ti; part=$part.Name; secs=[int]((Get-Date)-$t0).TotalSeconds;
  signal='sldworks window title contains the part name; sw_part.png vs sw_rotated.png differ'} | ConvertTo-Json | Set-Content "$D\solidworks.json"
Kill 'SLDWORKS|sldworks|SLDSTART|sldexitapp|swShellFn'
'''),
    "resolve": dict(timeout=1200, body=r'''
$t0=Get-Date; Start-Process 'C:\Program Files\Blackmagic Design\DaVinci Resolve\Resolve.exe'; Start-Sleep 90; Shot rs_start
foreach($c in $env:RS_CLICKS -split ';'){ if($c){ $xy=$c -split ','; Click ([int]$xy[0]) ([int]$xy[1]); Start-Sleep 4 } }
Start-Sleep 20; Shot rs_project
$o=Get-ChildItem "$env:USERPROFILE\Videos","$env:USERPROFILE\Documents","C:\kfapps\render" -Recurse -ErrorAction SilentlyContinue | ? { $_.LastWriteTime -gt $t0 -and $_.Extension -match 'mp4|mov|mkv|mxf' -and $_.Length -gt 10000 } | select -First 1
@{ok=[bool]$o; file=$o.FullName; bytes=$o.Length; titles=(Titles); secs=[int]((Get-Date)-$t0).TotalSeconds;
  signal='rendered output file newer than start and larger than 10 kB'} | ConvertTo-Json | Set-Content "$D\resolve.json"
Kill 'Resolve'
'''),
    "edrawings": dict(timeout=300, body=r'''
$e=(Get-ChildItem 'C:\Program Files\SOLIDWORKS Corp\eDrawings','C:\Program Files\eDrawings' -Filter eDrawings.exe -Recurse -ErrorAction SilentlyContinue | select -First 1)
$s=(Get-ChildItem 'C:\Program Files\SOLIDWORKS Corp','C:\Program Files\eDrawings' -Recurse -Include *.eprt,*.easm,*.edrw -ErrorAction SilentlyContinue | select -First 1)
if($e){ if($s){ Start-Process $e.FullName -ArgumentList ('"'+$s.FullName+'"') }else{ Start-Process $e.FullName } }; Start-Sleep 40; Shot ed_start
@{ok=[bool](Get-Process eDrawings -ErrorAction SilentlyContinue); exe=$e.FullName; sample=$s.FullName; titles=(Titles);
  signal='eDrawings process alive with a window title after 40 s; screenshot'} | ConvertTo-Json | Set-Content "$D\edrawings.json"
Kill 'eDrawings'
'''),
    "wukong": dict(timeout=600, body=r'''
# Steam is signed out and must stay so: this only reports whether the tool starts. Never sign in.
$t0=Get-Date; Start-Process 'C:\Users\kfimage\Desktop\Black Myth Wukong Benchmark Tool.url'; Start-Sleep 90; Shot bm_start
$pr=(Get-Process | ? { $_.ProcessName -match 'b1|BlackMyth|steam' } | % { $_.ProcessName }) -join ','
@{ok=($pr -match 'b1|BlackMyth'); procs=$pr; titles=(Titles); signal='game/benchmark process present (not just steam); a Steam login prompt means FAIL (reported, not driven)'} | ConvertTo-Json | Set-Content "$D\wukong.json"
Kill 'b1|BlackMyth|steam'
'''),
}


def env_prefix(env):
    """PowerShell lines that set calibration env vars for a check (click lists like '640,400;700,420')."""
    for k, v in env.items():
        if not re.fullmatch(r"[A-Z_]+", k) or not re.fullmatch(r"[0-9,;]*", v):
            raise ValueError("bad calibration var %r" % k)
    return "".join("$env:%s='%s'\n" % kv for kv in env.items())


def render_check(name, env=None):
    if name not in CHECKS:
        raise KeyError(name)
    return PS_LIB + env_prefix(env or {}) + CHECKS[name]["body"]


def task_wrapper(name):
    """Runs C:\\kfapps\\<name>.ps1 in the interactive session, once, and removes the task (no -RunLevel surprises)."""
    return ("$n='kf_%s'; Unregister-ScheduledTask -TaskName $n -Confirm:$false -ErrorAction SilentlyContinue\n"
            "$a=New-ScheduledTaskAction -Execute powershell.exe -Argument '-NoProfile -ExecutionPolicy Bypass -File %s\\%s.ps1'\n"
            "$p=New-ScheduledTaskPrincipal -UserId %s -LogonType Interactive -RunLevel Highest\n"
            "Register-ScheduledTask -TaskName $n -Action $a -Principal $p | Out-Null; Start-ScheduledTask -TaskName $n; 'started'\n"
            % (name, GUEST_DIR, name, USER))


# ------------------------------------------------------------------ host side
def run(cmd, **kw):
    return subprocess.run(cmd, text=True, capture_output=True, **kw)


def ssh(cmd=None, stdin=None, timeout=120, runner=run):
    argv = ["ssh"] + SSH_OPTS + ["%s@127.0.0.1" % USER] + (["powershell", "-NoProfile", "-NonInteractive", "-Command", "-"] if cmd is None else [cmd])
    return runner(argv, input=stdin, timeout=timeout)


def scp(src, dst, to_guest, runner=run):
    a = ["scp", "-q"] + [("-P" if o == "-p" else o) for o in SSH_OPTS]
    r = "%s@127.0.0.1:%s" % (USER, dst if to_guest else src)
    return runner(a + ([src, r] if to_guest else [r, dst]), timeout=300)


def wait_ssh(deadline_s, runner=run, sleep=time.sleep):
    t0 = time.time()
    while time.time() - t0 < deadline_s:
        p = ssh("echo kf-up", timeout=30, runner=runner)
        if p.returncode == 0 and "kf-up" in p.stdout:
            return True
        sleep(10)
    return False


def check_overlay_rule(prop_dir):
    """Refuse to boot unless image.qcow2 is an overlay whose backing file is image.base.qcow2."""
    info = json.loads(run(["qemu-img", "info", "--output=json", str(prop_dir / "image.qcow2")]).stdout or "{}")
    if Path(info.get("full-backing-filename", "")).name != "image.base.qcow2":
        raise SystemExit("refuse: image.qcow2 is not an overlay of image.base.qcow2")


def recreate_overlay(prop_dir):
    base = prop_dir / "image.base.qcow2"
    ov = prop_dir / "image.qcow2"
    ov.unlink(missing_ok=True)
    p = run(["qemu-img", "create", "-q", "-f", "qcow2", "-b", str(base), "-F", "qcow2", str(ov)])
    if p.returncode:
        raise SystemExit("overlay: " + p.stderr)
    check_overlay_rule(prop_dir)


def run_check(name, env, out, runner=run):
    """Push the check, start it in the interactive session, wait for its JSON, pull screenshots. Returns the row."""
    spec = CHECKS[name]
    ssh(stdin="New-Item -Force -ItemType Directory %s | Out-Null; Remove-Item %s\\%s.json -ErrorAction SilentlyContinue" % (GUEST_DIR, GUEST_DIR, name), runner=runner)
    tmp = out / ("%s.ps1" % name)
    tmp.write_text(render_check(name, env))
    scp(str(tmp), "%s/%s.ps1" % (GUEST_DIR.replace("\\", "/"), name), True, runner)
    t0 = time.time()
    ssh(stdin=task_wrapper(name), runner=runner)
    row = {"app": name, "ok": False, "signal": "", "secs": 0}
    while time.time() - t0 < spec["timeout"]:
        time.sleep(10)
        p = ssh(stdin="if(Test-Path %s\\%s.json){Get-Content %s\\%s.json -Raw}" % (GUEST_DIR, name, GUEST_DIR, name), runner=runner)
        if p.returncode == 0 and p.stdout.strip().startswith("{"):
            try:
                row.update(json.loads(p.stdout))
            except ValueError:
                continue
            break
    else:
        row["error"] = "timeout %ds" % spec["timeout"]
    row["secs"] = int(time.time() - t0)
    ssh(stdin="Unregister-ScheduledTask -TaskName kf_%s -Confirm:$false -ErrorAction SilentlyContinue" % name, runner=runner)
    d = out / name
    d.mkdir(exist_ok=True)
    pngs = ssh(stdin="(Get-ChildItem %s\\*.png | % { $_.Name }) -join ' '" % GUEST_DIR, runner=runner).stdout.split()
    row["shots"] = []
    for f in pngs:
        if scp("%s/%s" % (GUEST_DIR.replace("\\", "/"), f), str(d / f), False, runner).returncode == 0:
            row["shots"].append(str(d / f))
        ssh(stdin="Remove-Item %s\\%s -ErrorAction SilentlyContinue" % (GUEST_DIR, f), runner=runner)
    (out / (name + ".json")).write_text(json.dumps(row, indent=1))
    return row


def table(rows):
    lines = ["app\tok\tsecs\tsignal / error"]
    lines += ["%s\t%s\t%s\t%s" % (r["app"], "yes" if r.get("ok") else "NO", r.get("secs"), r.get("error") or r.get("signal", "")) for r in rows]
    return "\n".join(lines)


def boot():
    """propimage_run.sh offline as root (it starts swtpm + the broker as the session user, then execs QEMU)."""
    recreate_overlay(PROP_DIR)
    env = dict(os.environ, PROP_DIR=str(PROP_DIR))
    log = open(PROP_DIR / "run/apps_boot.log", "w")
    p = subprocess.Popen(["bash", str(RUN_SH), "offline"], env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    return p


def shutdown(proc, runner=run):
    ssh(stdin="Stop-Computer -Force", timeout=60, runner=runner)
    try:
        proc.wait(timeout=180)
        return True
    except subprocess.TimeoutExpired:
        return False  # caller reports; it never kills processes it did not start, and uses the exact PID it holds


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--apps", default=",".join(k for k in CHECKS if k != "3dmark_settings") + ",3dmark_settings")
    r.add_argument("--out", default="pcapps_out")
    r.add_argument("--env", default="", help="calibration vars, e.g. SP_CLICKS=640,400;700,420")
    r.add_argument("--no-boot", action="store_true", help="VM already running (calibration); no overlay, no shutdown")
    s = sub.add_parser("shot")
    s.add_argument("out")
    g = sub.add_parser("guest")
    g.add_argument("text")
    a = ap.parse_args(argv)
    if a.cmd == "guest":
        p = ssh(stdin=a.text)
        print(p.stdout, p.stderr, end="")
        return p.returncode
    if a.cmd == "shot":
        ssh(stdin="New-Item -Force -ItemType Directory %s | Out-Null" % GUEST_DIR)
        out = Path(a.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        tmp = out.with_suffix(".ps1")
        tmp.write_text(PS_LIB + "Shot calib")
        scp(str(tmp), "%s/calib.ps1" % GUEST_DIR.replace("\\", "/"), True)
        ssh(stdin=task_wrapper("calib"))
        time.sleep(8)
        return scp("%s/calib.png" % GUEST_DIR.replace("\\", "/"), str(out), False).returncode
    env = dict(kv.split("=", 1) for kv in a.env.split() if "=" in kv)
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    proc = None if a.no_boot else boot()
    rows = []
    try:
        if not wait_ssh(900):
            print("FAIL: ssh never came up")
            return 2
        for name in a.apps.split(","):
            rows.append(run_check(name, env, out))
    finally:
        if proc:
            print("shutdown clean:", shutdown(proc))
    txt = table(rows)
    (out / "result.tsv").write_text(txt + "\n")
    print(txt)
    return 0 if all(r.get("ok") for r in rows) else 1


if __name__ == "__main__":
    sys.exit(main())
