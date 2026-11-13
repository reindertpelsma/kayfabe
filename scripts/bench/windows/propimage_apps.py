#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Unattended app checks for the proprietary Windows image (run on the machine that hosts the VM).

STATUS: DESIGN-ONLY, 2026-11-13. The plumbing (overlay, boot, ssh wait, scheduled-task runner, UIA diagnostics, result table,
shutdown) is unit-tested without a VM (scripts/ci/test_propimage_apps.py). NO app check below has been run on hardware:
app behavior is unqualified; see V3_PROPIMAGE_APP_AUTOMATION.md for falsifiers and evidence gaps. Design: V3_WINDOWS_PROPRIETARY_IMAGE.md.

Rules it enforces (owner, binding): always OFFLINE (propimage_run.sh `offline`: slirp restrict=on, faked RTC), always a
throwaway overlay of image.base.qcow2 (re-created before each run), never writes the base, never signs in anywhere.

  propimage_apps.py run   [--apps a,b] [--out DIR]   boot, check, pull, shut down, print the table
  propimage_apps.py guest 'powershell text'          one command in the guest (calibration)
"""

import argparse
import json
import math
import os
import re
import subprocess
import sys
import time
import uuid
from pathlib import Path

PROP_DIR = Path(os.environ.get("PROP_DIR", "/workspace/bench/propimage_20261112"))
KEY = PROP_DIR / "keys/id_ed25519"
RUN_SH = Path(__file__).with_name("propimage_run.sh")
USER, PORT = "kfimage", 33037
SSH_OPTS = [
    "-i",
    str(KEY),
    "-p",
    str(PORT),
    "-o",
    "IdentitiesOnly=yes",
    "-o",
    "StrictHostKeyChecking=no",
    "-o",
    "UserKnownHostsFile=/dev/null",
    "-o",
    "BatchMode=yes",
    "-o",
    "LogLevel=ERROR",
    "-o",
    "ConnectTimeout=10",
]
GUEST_DIR = "C:\\kfapps"

# Guest scripts are authored here; selectors must come from a collected UIA tree.
# No pixel input, authentication, online requests, or closed-binary inspection.
PS_LIB = r"""
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
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


def render_check(name, env=None, run_id="0" * 32, plan=None):
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
    if name == "solidworks":
        script += SOLIDWORKS
    else:
        script += "$p=Start-Process '{}' -PassThru; Start-Sleep 30\n".format(spec["exe"])
        # URI launchers need a process tree from hardware before their UI can be scoped safely.
        if name == "wukong":
            script += r"""$fresh=@(Get-Process -Name b1,b1-Win64-Shipping,BlackMythWukong -ErrorAction SilentlyContinue | Where-Object {(Epoch $_.StartTime) -gt $r.started})
$r.launched=($fresh.Count -gt 0);$r.process_ids=@($fresh | ForEach-Object {$_.Id})
$r.note='Launch probe only; no benchmark verdict or Steam interaction'
"""
        else:
            script += "Dump-Uia $p.Id\n"
            if plan:
                validate_plan(plan)
                literal = json.dumps(plan).replace("'", "''")
                script += f"$plan=ConvertFrom-Json '{literal}'\n" + UIA_RUN
            else:
                script += "$r.note='UIA tree collected; verified run controls and result schema pending'\n"
    script += "} catch { $r.error=$_.Exception.Message } finally {\n$r.ended=Epoch (Get-Date)\n"
    script += 'ConvertTo-Json -InputObject $r -Depth 6 | Set-Content -Encoding UTF8 "$D\\result.json"\n}\n'
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
        else:
            raise ValueError(r.get("note", "No supported work evidence"))
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as exc:
        row["error"] = str(exc)
    return row


def task_wrapper(name):
    """Runs C:\\kfapps\\<name>.ps1 in the interactive session, once, and removes the task (no -RunLevel surprises)."""
    if not re.fullmatch(r"[a-z0-9_]+", name):
        raise ValueError("Invalid task name")
    return (
        f"$n='kf_{name}'; Unregister-ScheduledTask -TaskName $n -Confirm:$false -ErrorAction SilentlyContinue\n"
        f"$a=New-ScheduledTaskAction -Execute powershell.exe -Argument '-NoProfile -ExecutionPolicy Bypass -File {GUEST_DIR}\\{name}.ps1'\n"
        f"$p=New-ScheduledTaskPrincipal -UserId {USER} -LogonType Interactive -RunLevel Highest\n"
        "Register-ScheduledTask -TaskName $n -Action $a -Principal $p | Out-Null; Start-ScheduledTask -TaskName $n; 'started'\n"
    )


# ------------------------------------------------------------------ host side
def run(cmd, **kw):
    return subprocess.run(cmd, text=True, capture_output=True, check=False, **kw)


def ssh(cmd=None, stdin=None, timeout=120, runner=run):
    argv = (
        ["ssh"]
        + SSH_OPTS
        + [f"{USER}@127.0.0.1"]
        + (
            ["powershell", "-NoProfile", "-NonInteractive", "-Command", "-"]
            if cmd is None
            else [cmd]
        )
    )
    return runner(argv, input=stdin, timeout=timeout)


def scp(src, dst, to_guest, runner=run):
    a = ["scp", "-q"] + [("-P" if o == "-p" else o) for o in SSH_OPTS]
    r = f"{USER}@127.0.0.1:{dst if to_guest else src}"
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
    info = json.loads(
        run(["qemu-img", "info", "--output=json", str(prop_dir / "image.qcow2")]).stdout or "{}"
    )
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


def run_check(name, env, out, runner=run, plan=None, sleep=time.sleep):
    """Run in a unique guest directory; pull only authored, bounded artifact names."""
    run_id = uuid.uuid4().hex
    guest = GUEST_DIR.replace("\\", "/") + "/" + run_id + "/" + name
    d = out / name / run_id
    d.mkdir(parents=True)
    tmp = d / (name + ".ps1")
    tmp.write_text(render_check(name, env, run_id, plan))
    # Scripts remain in the task-wrapper's fixed location; results never do.
    prep = ssh(stdin=f"New-Item -Force -ItemType Directory '{GUEST_DIR}' | Out-Null", runner=runner)
    if (
        prep.returncode
        or scp(
            str(tmp), GUEST_DIR.replace("\\", "/") + "/" + name + ".ps1", True, runner
        ).returncode
    ):
        return {"app": name, "ok": False, "evidence": "NONE", "error": "Script transfer failed"}
    p = ssh(stdin=task_wrapper(name), runner=runner)
    t0 = time.monotonic()
    try:
        if p.returncode:
            raise ValueError("Task start failed")
        while time.monotonic() - t0 < CHECKS[name]["timeout"]:
            p = ssh(stdin=f"if(Test-Path '{guest}/result.json'){{'ready'}}", runner=runner)
            if p.returncode == 0 and p.stdout.strip() == "ready":
                break
            sleep(5)
        else:
            raise ValueError("Task timeout; no verdict")
        # Native exchange file, UIA tree and metadata, never guest-selected filenames or executables.
        for f in ("result.json", "uia.json", "model.step"):
            limit = 16 * 1024 * 1024 if f == "model.step" else 1024 * 1024
            p = ssh(
                stdin=f"if(Test-Path '{guest}/{f}'){{$f=Get-Item '{guest}/{f}';if($f.Length -le {limit}){{'bounded'}}}}",
                runner=runner,
            )
            if (
                p.returncode == 0
                and p.stdout.strip() == "bounded"
                and scp(guest + "/" + f, str(d / f), False, runner).returncode
            ):
                raise ValueError("Artifact transfer failed")
        row = verdict(name, run_id, d, plan)
    except ValueError as exc:
        row = {"app": name, "ok": False, "evidence": "NONE", "error": str(exc)}
    finally:
        # Stop only this authored task, never process-name sweeps.
        ssh(
            stdin=f"Stop-ScheduledTask -TaskName kf_{name} -ErrorAction SilentlyContinue; Unregister-ScheduledTask -TaskName kf_{name} -Confirm:$false -ErrorAction SilentlyContinue",
            runner=runner,
        )
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


def boot():
    """propimage_run.sh offline as root (it starts swtpm + the broker as the session user, then execs QEMU)."""
    recreate_overlay(PROP_DIR)
    env = dict(os.environ, PROP_DIR=str(PROP_DIR))
    with open(PROP_DIR / "run/apps_boot.log", "w") as log:
        p = subprocess.Popen(
            ["bash", str(RUN_SH), "offline"],
            env=env,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
    return p


def shutdown(proc, runner=run):
    ssh(stdin="Stop-Computer -Force", timeout=60, runner=runner)
    try:
        proc.wait(timeout=180)
        return True
    except subprocess.TimeoutExpired:
        return False  # caller reports; it never kills processes it did not start, and uses the exact PID it holds


def main(argv=None):
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--apps", default=",".join(CHECKS))
    r.add_argument("--out", default="pcapps_out")
    r.add_argument("--env", default="", help="deprecated; nonempty calibration input is refused")
    r.add_argument("--plans", type=Path, help="JSON app-to-UIA-plan map from hardware findings")
    r.add_argument(
        "--no-boot",
        action="store_true",
        help="VM already running (calibration); no overlay, no shutdown",
    )
    g = sub.add_parser("guest")
    g.add_argument("text")
    a = ap.parse_args(argv)
    if a.cmd == "guest":
        p = ssh(stdin=a.text)
        print(p.stdout, p.stderr, end="")
        return p.returncode
    env = dict(kv.split("=", 1) for kv in a.env.split() if "=" in kv)
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    plans = json.loads(a.plans.read_text()) if a.plans else {}
    for name, plan in plans.items():
        if name not in CHECKS or name == "wukong":
            raise ValueError("Unsupported plan app")
        validate_plan(plan)
    names = a.apps.split(",")
    if not names or any(n not in CHECKS for n in names):
        raise ValueError("Unknown app")
    env_prefix(env)
    proc = None if a.no_boot else boot()
    rows = []
    try:
        if not wait_ssh(900):
            print("FAIL: ssh never came up")
            return 2
        for name in names:
            rows.append(run_check(name, env, out, plan=plans.get(name)))
    finally:
        if proc:
            print("shutdown clean:", shutdown(proc))
    txt = table(rows)
    (out / "result.tsv").write_text(txt + "\n")
    print(txt)
    return 0 if all(r.get("ok") for r in rows) else 1


if __name__ == "__main__":
    sys.exit(main())
