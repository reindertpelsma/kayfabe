#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""s3tdr.py -- Windows S3 sleep/wake cycles judged with the post-wake TDR (STATUS: RESEARCH, 2026-11-23; evidence tool of
traces/win11_s3_tdr_20261123/, run by hand as root on a rented box; not a gate).

  s3tdr.py run --label L --arm A|B --backing OVERLAY --out DIR [--cycles N] [--qemu BIN] [--cpu CPU]

The 2026-11-23 probe (traces/win11_s3_wake_20261123/s3probe.py: boot, S3 wrapper, tracefs, QMP) with its cycle replaced:
the suspend is still a scheduled task in the signed-in session, but after the guest answers ssh the cycle waits until at
least TDR_WAIT_S past the wake (the TDR came 34-45 s after it), then reads the System log for the window since THIS
cycle's suspend request, and a 153 (nvlddmkm TDR) fails the cycle (PREREGISTRATION.md, CLEAN definition). It also counts
the kf3 lines that judge H1 (RAM add/del, USER_RING_REFUSED ... Topology, fn 47 unloads) and keeps the cycle's key kf3
lines. Guest and box output is data, only parsed.
"""
import re
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "win11_s3_wake_20261123"))
import s3probe as base  # noqa: E402

TDR_WAIT_S = 120
KEY = re.compile(r"S3 WAKE|GSP-RESUME|RAM (add|del)|USER_RING_REFUSED|fn 47|UNSERVICED|GSP REFUSED|vCPUs out|STALL|"
                 r"flip timeout|Xid")
KEY_MAX = 600


def kf3_counts(path):
    try:
        t = path.read_text(errors="replace")
    except OSError:
        return {}
    return {"s3wake": t.count("(S3 WAKE)"), "kept": len(re.findall(r"S3 WAKE\).*KEPT", t)),
            "gsp_resume": t.count("GSP-RESUME"), "cold": len(re.findall(r"device reset #\d+:? vCPUs out", t)),
            "unserviced": t.count("UNSERVICED"), "ram_add_kept": len(re.findall(r"RAM add .*translation kept", t)),
            "ram_changed": len(re.findall(r"RAM add .*translation CHANGED", t)), "ram_del": t.count("kf3: RAM del"),
            "user_ring_topology": len(re.findall(r"USER_RING_REFUSED.*Topology", t)),
            "user_ring_refused": t.count("USER_RING_REFUSED"),
            "fn47_unload": len(re.findall(r"fn 47 UNLOADING_GUEST_DRIVER.*newLevel=0", t)),
            "lines": t.count("\n")}


def keep_keys(log, start_line, dst):
    try:
        lines = log.read_text(errors="replace").splitlines()[start_line:]
    except OSError:
        return
    kept = [ln[:300] for ln in lines if KEY.search(ln)]
    dst.write_text("\n".join(kept[:KEY_MAX]) + ("\n[... %d more]\n" % (len(kept) - KEY_MAX) if len(kept) > KEY_MAX else "\n"))


def cycle(vm, tag, n, out):
    say = base.say
    log = base.kf3_log(vm, tag)
    rc, boot0 = vm.guest('(Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToString("o")', 90)
    boot0 = boot0.strip().splitlines()[-1] if boot0.strip() else ""
    k0 = kf3_counts(log)
    say(f"c{n} boot0={boot0} kf3_before={k0}")
    t_req = time.time()
    rc, o = vm.guest('schtasks /create /tn kfs3 /tr "rundll32.exe powrprof.dll,SetSuspendState 0,1,0" /sc once /st 23:59 '
                     '/ru kf /it /f | Out-Null; schtasks /run /tn kfs3; "KF_SCHED"', 60)
    say(f"c{n} suspend requested rc={rc} {o.strip()[-60:]}")
    st, t = "", time.time()
    while time.time() - t < 120:
        st = vm.status()
        if st == "suspended":
            break
        time.sleep(1)
    say(f"c{n} status={st} after {time.time() - t:.0f}s")
    res = {"cycle": n, "boot0": boot0, "suspended": st == "suspended", "kf3_before": k0}
    if st != "suspended":
        res.update(verdict="FAIL", reasons=["never suspended"])
        return res
    time.sleep(5)
    host_wake = time.time()
    vm.qmp("system_wakeup")
    say(f"c{n} system_wakeup sent")
    ok, boot1 = False, ""
    while time.time() - host_wake < 150:
        rc, o = vm.guest('(Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToString("o")', 20)
        if rc == 0 and o.strip():
            ok, boot1 = True, o.strip().splitlines()[-1]
            break
        time.sleep(5)
    back = round(time.time() - host_wake)
    say(f"c{n} ssh back={ok} after {back}s")
    rest = TDR_WAIT_S - (time.time() - host_wake)
    if ok and rest > 0:
        time.sleep(rest)
    base.sh(["bash", str(base.WINVM), "shot", f"s3_c{n}_after"], env=vm.env)
    k1 = kf3_counts(log)
    res.update(ssh_back=ok, back_after_s=back, boot1=boot1, kf3_after=k1, status_end=vm.status(),
               judged_after_wake_s=round(time.time() - host_wake))
    smi, evlog = "", ""
    if ok:
        _, smi = vm.guest("nvidia-smi --query-gpu=name --format=csv,noheader", 60)
        window = int(time.time() - t_req) + 15
        _, evlog = vm.guest('Get-WinEvent -FilterHashtable @{LogName="System"; StartTime=(Get-Date).AddSeconds(-WINDOW_S)} -EA 0 '
                            '| ? { $_.Id -in 41,1001,1,107,42,4101,153,14 } | % { "$($_.TimeCreated.ToString("o")) '
                            '$($_.Id) $($_.ProviderName)" }'.replace("WINDOW_S", str(window)), 90)
        res["event_window_s"] = window
    res["nvidia_smi"] = smi.strip()[-200:]
    res["events"] = evlog.strip().splitlines()[-30:]
    keep_keys(log, k0.get("lines", 0), out / f"c{n}_kf3_keys.txt")
    reasons = []
    if not ok:
        reasons.append("no ssh within 150 s")
    if ok and boot1 != boot0:
        reasons.append("new boot (LastBootUpTime changed)")
    for key, why in (("kept", "no (S3 WAKE) ... KEPT"), ("gsp_resume", "no GSP-RESUME")):
        if k1.get(key, 0) <= k0.get(key, 0):
            reasons.append(why)
    if k1.get("cold", 0) > k0.get("cold", 0):
        reasons.append("cold kf3 reset")
    if ok and not re.search(r"NVIDIA|RTX|GeForce", smi):
        reasons.append("nvidia-smi names no GPU")
    for ev in ("41", "1001", "153"):
        if any(re.search(rf" {ev} ", " " + e + " ") for e in res["events"]):
            reasons.append(f"event {ev}")
    res["verdict"] = "CLEAN" if not reasons else "FAIL"
    res["reasons"] = reasons
    return res


base.kf3_counts = kf3_counts
base.cycle = cycle

if __name__ == "__main__":
    base.main()
