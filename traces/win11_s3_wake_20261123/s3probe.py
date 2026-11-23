#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""s3probe.py -- one traced Windows S3 sleep/wake run on a box (STATUS: RESEARCH, 2026-11-23; evidence tool of
traces/win11_s3_wake_20261123/, run by hand as root on a rented box; not a gate).

  s3probe.py run --label L --arm A|B --backing OVERLAY --out DIR [--cycles N] [--qemu BIN] [--cpu CPU]

Boots a child overlay of OVERLAY (the Windows gate's overlay, driver installed) through win_vm.sh with S3 enabled
(a wrapper appends `-global ICH9-LPC.disable_s3=0` and, with --cpu, a later `-cpu` that wins), arms tracefs for the
guest's Hyper-V MSR writes before the boot, then runs N cycles: suspend via SetSuspendState in the guest, QMP
`query-status` = suspended, `system_wakeup`, then judges the cycle by the PREREGISTRATION.md definition. Everything it
learns goes to DIR as small text files; the guest's and the box's output is data, never interpreted beyond parsing.
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
TR = Path("/sys/kernel/tracing")
if not (TR / "events").is_dir():
    TR = Path("/sys/kernel/debug/tracing")

# Hyper-V MSRs whose guest WRITES we keep from boot on (rare); the hot ones (EOI/ICR/TPR 0x70-0x72, EOM 0x84,
# the stimer MSRs 0xb0-0xb7) are excluded here and covered after the wake by the kvm_hv_stimer_* events.
MSR_FILTER = ("write == 1 && ecx >= 0x40000000 && ecx < 0x40000200 && ecx != 0x40000070 && ecx != 0x40000071 "
              "&& ecx != 0x40000072 && ecx != 0x40000084 && (ecx < 0x400000b0 || ecx > 0x400000b7)")
BOOT_EVENTS = ["kvm/kvm_msr", "kvm/kvm_write_tsc_offset", "kvm/kvm_hv_synic_set_msr", "kvm/kvm_hv_stimer_set_config"]
WAKE_EVENTS = ["kvm/kvm_hv_stimer_start_one_shot", "kvm/kvm_hv_stimer_start_periodic", "kvm/kvm_hv_stimer_expiration",
               "kvm/kvm_hv_stimer_callback", "kvm/kvm_hv_stimer_set_count", "kvm/kvm_hv_stimer_cleanup",
               "kvm/kvm_hv_hypercall"]
HV_NAMES = {0x40000000: "GUEST_OS_ID", 0x40000001: "HYPERCALL", 0x40000021: "REFERENCE_TSC", 0x40000073: "APIC_ASSIST",
            0x40000080: "SCONTROL", 0x40000082: "SIEFP", 0x40000083: "SIMP", 0x40000102: "CRASH_P2",
            0x40000105: "CRASH_CTL"}
for _i in range(16):
    HV_NAMES[0x40000090 + _i] = f"SINT{_i}"

T0 = time.time()
OUT = Path(".")


def say(msg):
    line = f"[{time.time() - T0:8.1f}s] {msg}"
    print(line, flush=True)
    with open(OUT / "s3probe.log", "a") as f:
        f.write(line + "\n")


def sh(cmd, timeout=120, check=False, env=None):
    r = subprocess.run(cmd, shell=isinstance(cmd, str), capture_output=True, text=True, timeout=timeout, env=env)
    if check and r.returncode != 0:
        raise RuntimeError(f"{cmd!r} rc={r.returncode}: {r.stderr[-400:]}")
    return r


def wput(rel, val):
    try:
        (TR / rel).write_text(val)
        return True
    except OSError as e:
        say(f"TRACEFS_WRITE_FAILED {rel}={val!r}: {e}")
        return False


def tr_events(names, on):
    for n in names:
        p = TR / "events" / n / "enable"
        if p.exists():
            wput(f"events/{n}/enable", "1" if on else "0")
        elif on:
            say(f"TRACE_EVENT_MISSING {n}")


class Vm:
    def __init__(self, name):
        self.name = name
        self.root = Path(os.environ.get("WINVM_ROOT", "/workspace/winvm")) / name
        self.env = dict(os.environ, WINVM_NAME=name)

    def guest(self, ps, timeout=60):
        r = sh(["timeout", str(timeout), "bash", str(WINVM), "ssh", ps], timeout=timeout + 10, env=self.env)
        return r.returncode, (r.stdout or "").replace("\r", "")

    def qmp(self, name, args=None, tries=5):
        last = None
        for _ in range(tries):
            try:
                s, f = qmpmod.qmp_open(str(self.root / "run" / "qmp.sock"), timeout=10)
                try:
                    return qmpmod.qmp_cmd(f, name, args)
                finally:
                    s.close()
            except Exception as e:  # the screenshot loop holds the one-client socket now and then
                last = e
                time.sleep(0.7)
        raise RuntimeError(f"QMP {name}: {last}")

    def hmp(self, line):
        return self.qmp("human-monitor-command", {"command-line": line})

    def status(self):
        try:
            return self.qmp("query-status").get("status", "?")
        except Exception as e:
            return f"qmp-error:{e}"

    def qpid(self):
        try:
            return int((self.root / "run" / "qemu.pid").read_text().strip())
        except (OSError, ValueError):
            return None


def kvm_stats(pid):
    base = Path("/sys/kernel/debug/kvm")
    if pid is None or not base.is_dir():
        return {}
    for d in base.iterdir():
        if d.name.startswith(f"{pid}-"):
            out = {}
            for k in ("exits", "halt_exits", "io_exits", "mmio_exits", "halt_wakeup", "irq_injections"):
                try:
                    out[k] = int((d / k).read_text())
                except (OSError, ValueError):
                    pass
            return out
    return {}


def rates(pid, secs=2.0):
    a = kvm_stats(pid)
    time.sleep(secs)
    b = kvm_stats(pid)
    return {k: round((b[k] - a[k]) / secs) for k in a if k in b}


LINE = re.compile(r"^\s*(?P<task>.+?)-(?P<tid>\d+)\s+\[(?P<cpu>\d+)\].*?\s(?P<ts>\d+\.\d+):\s(?P<ev>\w+):\s(?P<body>.*)$")


def parse_trace(text):
    out = []
    for ln in text.splitlines():
        m = LINE.match(ln)
        if m:
            out.append((float(m["ts"]), m["ev"], m["body"], m["task"]))
    return out


def summarize(events, wake_ts):
    """The facts the hypotheses need, from parsed trace lines (pure: tested in test_s3probe.py)."""
    s = {"events": {}, "hv_writes_after_wake": [], "hv_writes_before_wake": [], "stimer_first": [], "stimer_distinct": {},
         "tsc_offset": [], "synic_host_writes_at_wake": 0, "expirations_ok": 0, "expirations_fail": 0}
    for ts, ev, body, task in events:
        s["events"][ev] = s["events"].get(ev, 0) + 1
        after = wake_ts is not None and ts >= wake_ts
        if ev == "kvm_msr":
            m = re.match(r"msr_write ([0-9a-f]+) = (0x[0-9a-f]+)", body)
            if m:
                msr = int(m[1], 16)
                rec = f"{ts:.6f} {task} {HV_NAMES.get(msr, hex(msr))}={m[2]}"
                (s["hv_writes_after_wake"] if after else s["hv_writes_before_wake"]).append(rec)
        elif ev == "kvm_hv_stimer_start_one_shot":
            m = re.search(r"vcpu_id (\d+) timer (\d+) time_now (\d+) count (\d+)", body)
            if m and after:
                v, t, now, cnt = (int(x) for x in m.groups())
                if len(s["stimer_first"]) < 40:
                    s["stimer_first"].append({"ts": ts, "vcpu": v, "timer": t, "time_now": now, "count": cnt,
                                              "count_s64": cnt - (1 << 64) if cnt >= (1 << 63) else cnt,
                                              "delta_u64": (cnt - now) % (1 << 64),
                                              "wraps_x100": ((cnt - now) % (1 << 64)) >= (1 << 64) // 100})
                k = f"{v}/{t}/{cnt}"
                s["stimer_distinct"][k] = s["stimer_distinct"].get(k, 0) + 1
        elif ev == "kvm_write_tsc_offset":
            s["tsc_offset"].append(f"{ts:.6f} {'AFTER' if after else 'before'} {body}")
        elif ev == "kvm_hv_synic_set_msr" and "host 1" in body and after:
            s["synic_host_writes_at_wake"] += 1
        elif ev == "kvm_hv_stimer_expiration" and after:
            m = re.search(r"msg send (-?\d+)", body)
            if m and int(m[1]) == 0:
                s["expirations_ok"] += 1
            else:
                s["expirations_fail"] += 1
    top = sorted(s["stimer_distinct"].items(), key=lambda kv: -kv[1])[:20]
    s["stimer_distinct"] = dict(top)
    return s


def tsc_page_gpa(hv_writes):
    gpa = None
    for rec in hv_writes:
        m = re.search(r"REFERENCE_TSC=(0x[0-9a-f]+)", rec)
        if m:
            v = int(m[1], 16)
            gpa = (v & ~0xFFF) if v & 1 else None
    return gpa


def read_trace():
    try:
        return (TR / "trace").read_text(errors="replace")
    except OSError as e:
        return f"# trace read failed: {e}\n"


def arm_boot_trace():
    wput("tracing_on", "0")
    wput("events/enable", "0")
    wput("buffer_size_kb", "32768")
    wput("options/overwrite", "0")
    wput("trace", "")
    wput("events/kvm/kvm_msr/filter", MSR_FILTER)
    tr_events(BOOT_EVENTS, True)
    wput("tracing_on", "1")


def kf3_log(vm, tag):
    return vm.root / "logs" / f"{tag}_b1_qemu.log"


def kf3_counts(path):
    try:
        t = path.read_text(errors="replace")
    except OSError:
        return {}
    return {"s3wake": t.count("(S3 WAKE)"), "kept": len(re.findall(r"S3 WAKE\).*KEPT", t)),
            "gsp_resume": t.count("GSP-RESUME"), "cold": len(re.findall(r"device reset #\d+:? vCPUs out", t)),
            "refused": t.count("REFUSED"), "unserviced": t.count("UNSERVICED"), "lines": t.count("\n")}


def wedge_sample(vm, out, n):
    """1 s of kvm_exit / kvm_pio / kvm_mmio, counted (the guest is not back: where is it?)."""
    pid = vm.qpid()
    res = {"rates": rates(pid, 2.0)}
    wput("tracing_on", "0")
    wput("events/enable", "0")
    wput("options/overwrite", "1")
    wput("trace", "")
    tr_events(["kvm/kvm_exit", "kvm/kvm_pio", "kvm/kvm_mmio"], True)
    wput("tracing_on", "1")
    time.sleep(1.0)
    wput("tracing_on", "0")
    text = read_trace()
    tr_events(["kvm/kvm_exit", "kvm/kvm_pio", "kvm/kvm_mmio"], False)
    reasons, rips, ports, gpas = {}, {}, {}, {}
    for _ts, ev, body, _t in parse_trace(text):
        if ev == "kvm_exit":
            m = re.search(r"reason (\S+) rip (0x[0-9a-f]+)", body)
            if m:
                reasons[m[1]] = reasons.get(m[1], 0) + 1
                rips[m[2]] = rips.get(m[2], 0) + 1
        elif ev == "kvm_pio":
            m = re.search(r"pio_(read|write) at (0x[0-9a-f]+) size \d+ count \d+ val (0x[0-9a-f]+)", body)
            if m:
                k = f"{m[1]} {m[2]} = {m[3]}"
                ports[k] = ports.get(k, 0) + 1
        elif ev == "kvm_mmio":
            m = re.search(r"mmio (\w+) len \d+ gpa (0x[0-9a-f]+)", body)
            if m:
                k = f"{m[1]} {m[2]}"
                gpas[k] = gpas.get(k, 0) + 1

    def top(d):
        return dict(sorted(d.items(), key=lambda kv: -kv[1])[:12])
    res.update(exit_reasons=top(reasons), rips=top(rips), pio=top(ports), mmio=top(gpas))
    (out / f"c{n}_wedge_sample.json").write_text(json.dumps(res, indent=1))
    say(f"WEDGE c{n} rates={res['rates']} reasons={top(reasons)} pio={list(top(ports).items())[:4]}")


def make_wrapper(out, qemu, cpu):
    w = out / "qemu-s3-wrapper.sh"
    extra = f" -cpu '{cpu}'" if cpu else ""
    w.write_text(f"#!/bin/sh\nexec '{qemu}' \"$@\" -global ICH9-LPC.disable_s3=0{extra}\n")
    w.chmod(0o755)
    return w


def cycle(vm, tag, n, out):
    log = kf3_log(vm, tag)
    rc, boot0 = vm.guest('(Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToString("o")', 90)
    boot0 = boot0.strip().splitlines()[-1] if boot0.strip() else ""
    pre_text = read_trace()
    (out / f"c{n}_boot_trace.txt").write_text(pre_text[-400000:])
    pre = summarize(parse_trace(pre_text), None)
    gpa = tsc_page_gpa(pre["hv_writes_before_wake"])
    say(f"c{n} boot0={boot0} hv_writes_before={len(pre['hv_writes_before_wake'])} tsc_page_gpa={hex(gpa) if gpa else None}")
    page = {}
    if gpa:
        page["before_suspend"] = vm.hmp(f"xp /4gx {gpa:#x}")
    k0 = kf3_counts(log)
    # the suspend window and the wake keep the low-rate events; the trace restarts here (earliest kept: overwrite=0)
    wput("tracing_on", "0")
    wput("trace", "")
    wput("tracing_on", "1")
    rc, o = vm.guest('Start-Process cmd.exe -ArgumentList "/c timeout /t 4 /nobreak & rundll32.exe '
                     'powrprof.dll,SetSuspendState 0,1,0" -WindowStyle Hidden; "KF_SCHED"', 60)
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
        res["verdict"] = "FAIL"
        res["reasons"] = ["never suspended"]
        return res
    if gpa:
        page["while_suspended"] = vm.hmp(f"xp /4gx {gpa:#x}")
    tr_events(WAKE_EVENTS, True)
    wput("trace_marker", "KF_S3_WAKE")
    host_wake = time.time()
    vm.qmp("system_wakeup")
    time.sleep(4)
    tr_events(WAKE_EVENTS, False)
    res["rates_wake+5s"] = rates(vm.qpid(), 2.0)
    if gpa:
        page["after_wake"] = vm.hmp(f"xp /4gx {gpa:#x}")
    res["tsc_page"] = page
    text = read_trace()
    (out / f"c{n}_wake_trace_head.txt").write_text("\n".join(text.splitlines()[:30000]) + "\n")
    evs = parse_trace(text)
    wake_ts = next((ts for ts, ev, body, _ in evs if ev in ("print", "tracing_mark_write") and "KF_S3_WAKE" in body), None)
    res["trace"] = summarize(evs, wake_ts)
    say(f"c{n} woke; status={vm.status()} rates={res['rates_wake+5s']} stimer_first={res['trace']['stimer_first'][:2]}")
    # back?
    ok, boot1, waited = False, "", 0
    while time.time() - host_wake < 150:
        rc, o = vm.guest('(Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToString("o")', 20)
        if rc == 0 and o.strip():
            ok, boot1 = True, o.strip().splitlines()[-1]
            break
        if time.time() - host_wake > 60 and "wedge" not in res:
            res["wedge"] = True
            wedge_sample(vm, out, n)
        time.sleep(5)
    waited = round(time.time() - host_wake)
    sh(["bash", str(WINVM), "shot", f"s3_c{n}_after"], env=vm.env)
    k1 = kf3_counts(log)
    res.update(ssh_back=ok, back_after_s=waited, boot1=boot1, kf3_after=k1, status_end=vm.status())
    smi, evlog = "", ""
    if ok:
        _, smi = vm.guest("nvidia-smi --query-gpu=name --format=csv,noheader", 60)
        _, evlog = vm.guest('Get-WinEvent -FilterHashtable @{LogName="System"; StartTime=(Get-Date).AddMinutes(-6)} -EA 0 '
                            '| ? { $_.Id -in 41,1001,1,107,42,4101,153,14 } | % { "$($_.TimeCreated.ToString("o")) '
                            '$($_.Id) $($_.ProviderName)" }', 90)
    res["nvidia_smi"] = smi.strip()[-200:]
    res["events"] = evlog.strip().splitlines()[-30:]
    reasons = []
    if not ok:
        reasons.append("no ssh within 150 s")
    if ok and boot1 != boot0:
        reasons.append("new boot (LastBootUpTime changed)")
    if k1.get("kept", 0) <= k0.get("kept", 0):
        reasons.append("no (S3 WAKE) ... KEPT")
    if k1.get("gsp_resume", 0) <= k0.get("gsp_resume", 0):
        reasons.append("no GSP-RESUME")
    if k1.get("cold", 0) > k0.get("cold", 0):
        reasons.append("cold kf3 reset")
    if ok and not re.search(r"NVIDIA|RTX|GeForce", smi):
        reasons.append("nvidia-smi names no GPU")
    if any(re.search(r" (41|1001) ", " " + e + " ") for e in res["events"]):
        reasons.append("event 41/1001")
    res["verdict"] = "CLEAN" if not reasons else "FAIL"
    res["reasons"] = reasons
    return res


def cmd_run(a):
    global OUT
    OUT = Path(a.out)
    OUT.mkdir(parents=True, exist_ok=True)
    vm = Vm(a.name)
    say(f"S3PROBE_START label={a.label} arm={a.arm} backing={a.backing} cpu={a.cpu or '-'} qemu={a.qemu}")
    host = {"uname": sh("uname -r").stdout.strip(),
            "clocksource": sh("cat /sys/devices/system/clocksource/clocksource0/current_clocksource").stdout.strip(),
            "tsc_dmesg": sh("dmesg | grep -iE 'tsc.*(unstable|clocksource)' | tail -5").stdout.strip().splitlines(),
            "cpu_flags_has_constant_tsc": "constant_tsc" in Path("/proc/cpuinfo").read_text(),
            "cpu_flags_has_nonstop_tsc": "nonstop_tsc" in Path("/proc/cpuinfo").read_text(),
            "repo_rev": sh(["git", "-C", str(REPO), "rev-parse", "--short=9", "HEAD"]).stdout.strip(),
            "gpu": sh("nvidia-smi --query-gpu=name,driver_version --format=csv,noheader").stdout.strip()}
    (OUT / "host.json").write_text(json.dumps(host, indent=1))
    say(f"host {host}")
    wrapper = make_wrapper(OUT, a.qemu, a.cpu)
    tag = f"s3_{a.label}"
    vmuuid = ""
    for ln in (vm.root / "identity.env").read_text().splitlines():
        if ln.startswith("VM_UUID="):
            vmuuid = ln.split("=", 1)[1].strip()
    arm_boot_trace()
    env = dict(vm.env, WINVM_QEMU=str(wrapper), WINVM_OVERLAY_BACKING=a.backing)
    r = sh(["bash", str(WINVM), "run", "--detach", "--once", "--overlay", tag, "--arm", a.arm, "--guest-driver",
            a.guest_driver, "--kf3-extra", f"gpu-uuid=GPU-{vmuuid}", "--tag", tag], env=env, timeout=120)
    say(f"launch rc={r.returncode} {r.stdout.strip()[-200:]} {r.stderr.strip()[-200:]}")
    t = time.time()
    up = False
    while time.time() - t < 1200:
        rc, o = vm.guest("echo KF_UP", 20)
        if "KF_UP" in o:
            up = True
            break
        time.sleep(10)
    say(f"guest up={up} after {time.time() - t:.0f}s")
    results = []
    if up:
        time.sleep(a.settle)
        _, pa = vm.guest("powercfg /hibernate off; powercfg /change standby-timeout-ac 0; powercfg /a", 90)
        (OUT / "powercfg_a.txt").write_text(pa)
        say("powercfg /a: " + " | ".join(x.strip() for x in pa.splitlines() if x.strip())[:400])
        for n in range(1, a.cycles + 1):
            res = cycle(vm, tag, n, OUT)
            results.append(res)
            (OUT / f"c{n}_result.json").write_text(json.dumps(res, indent=1, default=str))
            say(f"S3_CYCLE label={a.label} n={n} verdict={res['verdict']} reasons={res.get('reasons')}")
            if res["verdict"] != "CLEAN":
                break
            time.sleep(30)
    clean = sum(1 for r in results if r["verdict"] == "CLEAN")
    say(f"S3PROBE_DONE label={a.label} clean={clean}/{len(results)} of {a.cycles} planned")
    wput("tracing_on", "0")
    wput("events/enable", "0")
    if not a.keep:
        sh(["bash", str(WINVM), "stop"], env=vm.env, timeout=400)
        if vm.qpid() and Path(f"/proc/{vm.qpid()}").exists():
            say("guest did not stop; QMP quit")
            try:
                vm.qmp("quit")
            except Exception:
                pass
    say("S3PROBE_EXIT")


def main():
    p = argparse.ArgumentParser()
    sp = p.add_subparsers(dest="cmd", required=True)
    r = sp.add_parser("run")
    r.add_argument("--label", required=True)
    r.add_argument("--arm", choices=["A", "B"], required=True)
    r.add_argument("--backing", required=True)
    r.add_argument("--out", required=True)
    r.add_argument("--cycles", type=int, default=1)
    r.add_argument("--qemu", required=True)
    r.add_argument("--cpu", default="")
    r.add_argument("--name", default="kfwin")
    r.add_argument("--guest-driver", default="580.65.06")
    r.add_argument("--settle", type=int, default=90)
    r.add_argument("--keep", action="store_true")
    a = p.parse_args()
    if a.cmd == "run":
        cmd_run(a)


if __name__ == "__main__":
    main()
