#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Summarize the S3 wake MSR control: per variant and cycle, each MSR before/after, QEMU's host writes at the wake, the
guest's Hyper-V MSR writes after it, the TSC offset step, timer-event rates and clock deltas.

  analyze.py RUN_DIR > summary.md   (RUN_DIR holds one directory per variant, as wakeprobe.py wrote them)
"""
import gzip
import json
import re
import sys
from pathlib import Path

PART = ("GUEST_OS_ID", "HYPERCALL", "REFERENCE_TSC", "TIME_REF_COUNT")
PERVP = ("SCONTROL", "SIEFP", "SIMP", "SINT2", "STIMER0_CONFIG", "STIMER0_COUNT")
NAMES = {0x40000000: "GUEST_OS_ID", 0x40000001: "HYPERCALL", 0x40000021: "REFERENCE_TSC", 0x40000080: "SCONTROL",
         0x40000082: "SIEFP", 0x40000083: "SIMP", 0x400000B0: "STIMER0_CONFIG", 0x400000B1: "STIMER0_COUNT"}
LINE = re.compile(r"^\s*(.+?)-(\d+)\s+\[\d+\].*?\s(\d+\.\d+): (\w+): (.*)$")
SYMPTOM = re.compile(r"clocksource|watchdog|TSC.*unstable|soft lockup|rcu.*stall|hrtimer: interrupt took", re.I)


def short(v):
    if not isinstance(v, str) or not v.startswith("0x"):
        return str(v)
    return hex(int(v, 16))


def trace_facts(path):
    text = gzip.open(path, "rt", errors="replace").read()
    wake_ts = None
    host, guest, tsc, oneshot = [], [], [], []
    for line in text.splitlines():
        m = LINE.match(line)
        if not m:
            continue
        comm, ts, ev, body = m[1].strip(), float(m[3]), m[4], m[5]
        if ev == "tracing_mark_write" and "system_wakeup" in body:
            wake_ts = ts
        after = wake_ts is not None and ts >= wake_ts
        vcpu = re.search(r"CPU (\d+)/KVM", comm)
        vc = vcpu[1] if vcpu else "?"
        if ev in ("kvm_hv_synic_set_msr", "kvm_hv_stimer_set_config", "kvm_hv_stimer_set_count") and body.rstrip().endswith("host 1"):
            host.append((after, body))
        elif ev == "kvm_msr" and body.startswith("msr_write") and after:
            mm = re.match(r"msr_write (\w+) = (0x\w+)(.*)", body)
            if mm:
                guest.append("vcpu%s %s=%s%s" % (vc, NAMES.get(int(mm[1], 16), mm[1]), short(mm[2]), mm[3]))
        elif ev == "kvm_write_tsc_offset" and after:
            mm = re.search(r"vcpu=(\d+) prev=(\d+) next=(\d+)", body)
            if mm:
                d = (int(mm[3]) - int(mm[2]) + 2**63) % 2**64 - 2**63
                tsc.append((int(mm[1]), d))
        elif ev == "kvm_hv_stimer_start_one_shot" and after:
            oneshot.append(ts)
    peak = 0
    j = 0
    for i, t in enumerate(oneshot):
        while oneshot[j] < t - 1.0:
            j += 1
        peak = max(peak, i - j + 1)
    wake_host = [b for a, b in host if a]
    return {"wake_marker": wake_ts is not None, "host_writes_at_wake": wake_host, "guest_hv_writes_after": guest,
            "tsc_offset_steps": tsc, "oneshot_peak_per_s": peak, "oneshot_after": len(oneshot)}


def cycle_rows(vdir, n):
    try:
        pre = json.loads((vdir / ("c%d_pre.json" % n)).read_text())
        post = json.loads((vdir / ("c%d_post.json" % n)).read_text())
    except (OSError, ValueError):
        return None
    rows = []
    for name in PART:
        rows.append(("cpu0", name, pre["msrs"]["0"][name], post["msrs"]["0"][name]))
    for cpu in sorted(pre["msrs"]):
        for name in PERVP:
            rows.append(("cpu" + cpu, name, pre["msrs"][cpu][name], post["msrs"][cpu][name]))
    return pre, post, rows


def main(root):
    root = Path(root)
    out = []
    for vdir in sorted(p for p in root.iterdir() if (p / "result.json").exists()):
        res = json.loads((vdir / "result.json").read_text())
        out.append("## %s\n\n`%s`, cpu `%s`, seeded=%s, boot ssh %.0f s\n" % (
            vdir.name, res.get("qemu_version", "").splitlines()[0] if res.get("qemu_version") else "?", res["cpu"],
            res["seeded"], res.get("boot_ssh_s") or -1))
        for cyc in res["cycles"]:
            n = cyc["n"]
            out.append("### cycle %d: status %s, ssh back %s s" % (n, cyc.get("status_before_wake"), cyc.get("ssh_back_s")))
            got = cycle_rows(vdir, n)
            if got is None:
                out.append("(no pre/post JSON)\n")
                continue
            pre, post, rows = got
            out.append("\n| cpu | MSR | before suspend | after wake |\n|---|---|---|---|")
            for cpu, name, a, b in rows:
                out.append("| %s | %s | %s | %s |" % (cpu, name, short(a), short(b)))
            mono_g = post["t_monotonic"] - pre["t_monotonic"]
            boot_g = post["t_boottime"] - pre["t_boottime"]
            out.append("\nclocksource %s -> %s; guest MONOTONIC +%.2f s, BOOTTIME +%.2f s across the cycle; guest REALTIME "
                       "post - host wake time = %.2f s; TSC page %s -> %s; suspend_error %s" % (
                           pre["clocksource"], post["clocksource"], mono_g, boot_g,
                           post["t_realtime"] - cyc["host_realtime_wake"], pre["tsc_page"], post["tsc_page"],
                           post.get("suspend_error")))
            try:
                dm = (vdir / ("c%d_dmesg.txt" % n)).read_text()
            except OSError:
                dm = ""
            hits = [ln for ln in dm.splitlines() if SYMPTOM.search(ln)]
            out.append("guest kernel log symptom lines: %d%s" % (len(hits), "".join("\n    " + h for h in hits[:6])))
            tf = vdir / ("c%d_trace.txt.gz" % n)
            if tf.exists():
                f = trace_facts(tf)
                out.append("host (QEMU) Hyper-V writes after the system_wakeup marker: %d%s" % (
                    len(f["host_writes_at_wake"]), "".join("\n    " + h for h in f["host_writes_at_wake"][:12])))
                out.append("guest Hyper-V MSR writes after the wake: %d%s" % (
                    len(f["guest_hv_writes_after"]), "".join("\n    " + g for g in f["guest_hv_writes_after"][:12])))
                out.append("TSC offset steps after the wake (vcpu, cycles): %s" % f["tsc_offset_steps"])
                out.append("stimer one-shots after the wake: %d, peak %d per 1 s\n" % (f["oneshot_after"], f["oneshot_peak_per_s"]))
    print("\n".join(out))


if __name__ == "__main__":
    main(sys.argv[1])
