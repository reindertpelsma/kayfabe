#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""winboot.py -- one watched Windows kf3 boot for Experiment 2 of PREREGISTRATION.md (display off vs on). Evidence tool of
traces/fill_probe_20261124/, run by hand as root on a rented box; not a gate.

  winboot.py --label L --display on|off --backing GATE_OVERLAY --out DIR [--watch 300] [--name kfwin]

Boots a child overlay of the gate's overlay through `win_vm.sh run --detach --once` (arm B, guest-driver 580.65.06,
WINVM_KF3_FB_MB=4096, WINVM_KF3_DISPLAY as given, product defaults for every kf3 flag), waits for ssh, then for WATCH
seconds polls QEMU's status and the guest's System log every 30 s. At the end it keeps the QMP events of the boot, the
new host dmesg lines and the kf3 log lines about the flat FB alias, and stops the VM. Guest and box output is data, only
parsed.
"""
import argparse
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
WINDIR = REPO / "scripts" / "bench" / "windows"
sys.path.insert(0, str(WINDIR))
import qmp as qmpmod  # noqa: E402  (scripts/bench/windows/qmp.py)

WINVM = WINDIR / "win_vm.sh"
EVENT_IDS = "153,141,4101,41,1074,6008,1001,13,14"
EV_PS = ('Get-WinEvent -FilterHashtable @{LogName="System"; StartTime=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime} '
         '-EA 0 | ? { $_.Id -in ' + EVENT_IDS + ' } | % { "$($_.TimeCreated.ToString("o")) $($_.Id) $($_.ProviderName) "'
         ' + (($_.Message -replace "\\s+"," ") -replace "^(.{0,160}).*","$1") }')
DMESG_KEEP = re.compile(r"NVRM|Xid|mmuWalk|NO_MEMORY", re.I)
KF3_KEEP = re.compile(r"0x120000000|HELD|0x4b69|VA_ALREADY_MAPPED|Xid|REFUSED|STALL|display:", re.I)
T0 = time.time()
OUT = Path(".")


def say(msg):
    line = f"[{time.time() - T0:7.1f}s] {msg}"
    print(line, flush=True)
    with open(OUT / "winboot.log", "a") as f:
        f.write(line + "\n")


def sh(cmd, timeout=120, env=None):
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, env=env)
        return r.returncode, (r.stdout or "").replace("\r", ""), r.stderr or ""
    except subprocess.TimeoutExpired:
        return 124, "", "timeout"


class Vm:
    def __init__(self, name):
        self.root = Path(os.environ.get("WINVM_ROOT", "/workspace/winvm")) / name
        self.env = dict(os.environ, WINVM_NAME=name)

    def guest(self, ps, timeout=60):
        rc, o, _ = sh(["timeout", str(timeout), "bash", str(WINVM), "ssh", ps], timeout=timeout + 10, env=self.env)
        return rc, o

    def status(self):
        try:
            s, f = qmpmod.qmp_open(str(self.root / "run" / "qmp.sock"), timeout=10)
            try:
                return qmpmod.qmp_cmd(f, "query-status", None).get("return", {}).get("status", "?")
            finally:
                s.close()
        except Exception as e:  # a QEMU that is gone has no socket
            return f"gone({type(e).__name__})"

    def qpid_alive(self):
        try:
            pid = int((self.root / "run" / "qemu.pid").read_text().strip())
            return Path(f"/proc/{pid}").exists()
        except (OSError, ValueError):
            return False


def main():
    global OUT
    p = argparse.ArgumentParser()
    p.add_argument("--label", required=True)
    p.add_argument("--display", choices=["on", "off"], required=True)
    p.add_argument("--backing", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--watch", type=int, default=300)
    p.add_argument("--name", default="kfwin")
    a = p.parse_args()
    OUT = Path(a.out)
    OUT.mkdir(parents=True, exist_ok=True)
    vm = Vm(a.name)
    tag = f"fp_{a.label}"
    vmuuid = ""
    for ln in (vm.root / "identity.env").read_text().splitlines():
        if ln.startswith("VM_UUID="):
            vmuuid = ln.split("=", 1)[1].strip()
    d0 = len(sh(["dmesg"])[1].splitlines())
    env = dict(vm.env, WINVM_OVERLAY_BACKING=a.backing, WINVM_KF3_DISPLAY=a.display, WINVM_KF3_FB_MB="4096")
    rc, o, e = sh(["bash", str(WINVM), "run", "--detach", "--once", "--overlay", tag, "--arm", "B", "--guest-driver",
                   "580.65.06", "--kf3-extra", f"gpu-uuid=GPU-{vmuuid}", "--tag", tag], timeout=120, env=env)
    say(f"WINBOOT_START label={a.label} display={a.display} launch_rc={rc} {o.strip()[-160:]} {e.strip()[-160:]}")
    t_launch = time.time()
    up = False
    while time.time() - t_launch < 1200:
        rc, o = vm.guest("echo KF_UP", 20)
        if "KF_UP" in o:
            up = True
            break
        if time.time() - t_launch > 60 and not vm.qpid_alive():
            break
        time.sleep(10)
    t_up = time.time()
    say(f"guest up={up} after {t_up - t_launch:.0f}s")
    res = {"label": a.label, "display": a.display, "up": up, "up_after_s": round(t_up - t_launch), "polls": []}
    if up:
        _, gpu = vm.guest('$g=Get-CimInstance Win32_VideoController | ? { $_.Name -like "*NVIDIA*" }; '
                          '"ADAPTER " + $g.Name + " code=" + $g.ConfigManagerErrorCode; '
                          '"NVSMI " + ((& nvidia-smi --query-gpu=name,driver_version --format=csv,noheader 2>&1) -join " | ")',
                          120)
        res["gpu_at_up"] = gpu.strip()
        say("GPU " + gpu.strip().replace("\n", " ; "))
        while time.time() - t_up < a.watch:
            time.sleep(30)
            st = vm.status()
            rc, ev = vm.guest(EV_PS, 60) if st == "running" else (None, "")
            evs = [x for x in ev.strip().splitlines() if x.strip()]
            poll = {"t_since_up_s": round(time.time() - t_up), "qemu_status": st, "guest_rc": rc, "events": evs}
            res["polls"].append(poll)
            say(f"POLL t={poll['t_since_up_s']}s status={st} guest_rc={rc} events={len(evs)} "
                f"153={sum(' 153 ' in x for x in evs)}")
            if st != "running" and not st.startswith("paused"):
                if not vm.qpid_alive():
                    break
            if st in ("guest-panicked", "shutdown") or st.startswith("gone"):
                break
    res["final_status"] = vm.status()
    res["qemu_alive_at_end"] = vm.qpid_alive()
    logs = vm.root / "logs"
    qev = logs / f"{tag}_b1_qmp_events.jsonl"
    evlines = qev.read_text(errors="replace").splitlines() if qev.exists() else []
    (OUT / "qmp_events.jsonl").write_text("\n".join(evlines) + "\n")
    res["qmp_event_names"] = [json.loads(x).get("event") for x in evlines if x.startswith("{")]
    res["guest_panicked"] = [x for x in evlines if "GUEST_PANICKED" in x]
    new = [ln for ln in sh(["dmesg"])[1].splitlines()[d0:] if DMESG_KEEP.search(ln)]
    (OUT / "host_dmesg.txt").write_text("\n".join(new) + "\n")
    res["dmesg_mmuwalk"] = sum("mmuWalk" in ln for ln in new)
    res["dmesg_xid"] = sum("Xid" in ln for ln in new)
    kl = logs / f"{tag}_b1_qemu.log"
    kept = []
    if kl.exists():
        for ln in kl.read_text(errors="replace").splitlines():
            if KF3_KEEP.search(ln):
                kept.append(ln[:300])
    (OUT / "kf3_keys.txt").write_text("\n".join(kept[:800]) + "\n")
    res["kf3_alias_lines"] = sum("0x120000000" in x for x in kept)
    run_log = logs / f"{tag}_run.log"
    if run_log.exists():
        (OUT / "run_log.txt").write_text(run_log.read_text(errors="replace"))
    cmd = logs / f"{tag}_b1_cmdline.txt"
    if cmd.exists():
        (OUT / "cmdline.txt").write_text(cmd.read_text(errors="replace"))
    all153 = sorted({e for pl in res["polls"] for e in pl["events"] if " 153 " in e})
    res["n153"] = len(all153)
    res["died"] = not res["qemu_alive_at_end"] or res["final_status"] in ("guest-panicked", "shutdown")
    (OUT / "result.json").write_text(json.dumps(res, indent=1))
    say(f"WINBOOT_RESULT label={a.label} display={a.display} up={up} n153={res['n153']} died={res['died']} "
        f"final={res['final_status']} panicked={len(res['guest_panicked'])} mmuwalk={res['dmesg_mmuwalk']} xid={res['dmesg_xid']}")
    sh(["bash", str(WINVM), "stop"], timeout=400, env=vm.env)
    if vm.qpid_alive():
        try:
            s, f = qmpmod.qmp_open(str(vm.root / "run" / "qmp.sock"), timeout=10)
            qmpmod.qmp_cmd(f, "quit", None)
            s.close()
        except Exception:
            pass
    say("WINBOOT_EXIT")


if __name__ == "__main__":
    main()
