#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""s3kf3pc.py -- Windows S3 sleep/wake cycles under kf3 on the owner's pc, no GPU bind (STATUS: RESEARCH, 2026-11-25; evidence
tool of traces/pc_s3_4070_20261125/, run by hand as root on the pc; not a gate).

  s3kf3pc.py session --vmroot DIR --prep PREP.qcow2 --broker BIN --gop EFI --out DIR \
      --plan LABEL:ARM:QEMU:CYCLES:ZERO[,...] [--no-new-boot-after HH:MM]

ARM A = CPU `host,-vmx`; ARM B = A plus the Hyper-V enlightenments of the 2026-11-23 gate. ZERO is `default` or `off`
(`unmapped-reads-zero=off`). Each plan item boots a fresh thin child overlay of PREP (never written), the kf3 device with the
display plane on (`display=on,gop=on`, a headless `nvkvm-display-broker --backend test`), `-global ICH9-LPC.disable_s3=0`, a QMP
event recorder on a second socket, and runs CYCLES S3 cycles in that boot; a cycle that is not CLEAN ends the boot, and the next
boot of the same item starts (at most CYCLES boots).
Reuses: traces/vfio_s3_control_20261124/s3vfio.py (Vm, cycle, guest helpers) and the argv builder of
scripts/bench/windows/propimage.py, as traces/pc_displayexp_20261124/displayexp.py did. The cycle is s3vfio.cycle (suspend by a
scheduled task in the signed-in session, QMP system_wakeup, ssh back, 150 s wait, System-log events, nvidia-smi, explorer) plus the kf3
log lines of the 2026-11-23 display probe (display STOPPED, STALL, RM unload, cold reset) and the QMP events; the verdict is
CLEAN only when none of them fired (PREREGISTRATION.md). The guest's and the host's output is data, only parsed.
"""
import argparse
import fcntl
import json
import os
import re
import signal
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(REPO / "traces" / "vfio_s3_control_20261124"))
import s3vfio as S  # noqa: E402  (also puts scripts/bench/windows and the 2026-11-23 probe on sys.path)

PI = S.PI
SP = S.SP
S.ARM_CPU["A"] = "host,-vmx"
KF3_COUNTS = {
    "s3wake": r"\(S3 WAKE\)", "kept": r"S3 WAKE\).*KEPT", "gsp_resume": r"GSP-RESUME", "cold": r"device reset #\d+:? vCPUs out",
    "disp_stopped": r"display: channel \d+ STOPPED", "disp_stall": r"display: STALL ", "rm_unload": r"fn 47 UNLOADING_GUEST_DRIVER.*newLevel=0",
    "ram_changed": r"RAM add .*translation CHANGED", "user_ring_topology": r"USER_RING_REFUSED.*Topology",
    "unserviced": r"UNSERVICED", "status_0x51": r"status=0x51|NV_ERR_NO_MEMORY|0x00000051",
}
KEY = re.compile(r"S3 WAKE|GSP-RESUME|RAM (add|del)|USER_RING_REFUSED|fn 47|STOPPED|STALL|vCPUs out|after the S3 wake|0x51|NO_MEMORY")


class PcVm(S.Vm):
    """One boot of the kf3 device with the display plane (as displayexp.py arm B), S3 enabled, a second QMP socket for events."""

    def __init__(self, *a, arm, zero, broker, gop, **kw):
        super().__init__(*a, **kw)
        self.arm, self.zero, self.broker_bin, self.gop = arm, zero, broker, gop

    def argv(self):
        a = PI.build_argv(self.state, PI.PLAIN, self.disk, self.work, qemu=self.qemu, display="broker", net="off",
                          broker_sock=str(self.work / "broker.sock"), gop_efi=str(self.gop), uid=0)
        i = a.index("-cpu")
        a[i + 1] = self.cpu
        if self.zero == "off":
            i = next(n for n, v in enumerate(a) if v.startswith("kf3-gpu"))
            a[i] += ",unmapped-reads-zero=off"
        a += ["-global", "ICH9-LPC.disable_s3=0", "-qmp", "unix:%s,server=on,wait=off" % (self.work / "qmp-ev.sock"),
              "-pidfile", str(self.work / "qemu.pid")]
        assert any(x.startswith("kf3-gpu") for x in a) and any("display=on" in x for x in a), "kf3 device or display plane missing"
        return a


def start_broker(vm):
    sock, fifo = vm.work / "broker.sock", vm.work / "broker.fifo"
    os.mkfifo(fifo)
    fd = os.open(fifo, os.O_RDWR)
    bp = subprocess.Popen([vm.broker_bin, "--socket", str(sock), "--backend", "test", "--persist", "--verbose", "--size", "1920x1080"],
                          stdin=fd, stdout=open(vm.work / "broker.log", "w"), stderr=subprocess.STDOUT)
    for _ in range(50):
        if sock.exists() or bp.poll() is not None:
            break
        time.sleep(0.2)
    if not sock.exists():
        raise RuntimeError("display broker did not become ready")
    return bp, fd


def kf3_counts(path):
    try:
        t = Path(path).read_text(errors="replace")
    except OSError:
        return {}
    c = {k: len(re.findall(v, t)) for k, v in KF3_COUNTS.items()}
    c["lines"] = t.count("\n")
    return c


def keep_keys(log, start_line, dst):
    try:
        lines = Path(log).read_text(errors="replace").splitlines()[start_line:]
    except OSError:
        return
    kept = [S.scrub(ln[:300]) for ln in lines if KEY.search(ln)]
    Path(dst).write_text("\n".join(kept[:600]) + ("\n[... %d more]\n" % (len(kept) - 600) if len(kept) > 600 else "\n"))


def events_since(path, t0):
    out = []
    try:
        for ln in Path(path).read_text().splitlines():
            m = json.loads(ln)
            if "event" in m and m.get("t_host", 0) >= t0:
                out.append({"event": m["event"], "t_host": round(m["t_host"], 1), "data": m.get("data", {})})
    except (OSError, ValueError):
        pass
    return out


def cycle(vm, n, out, label):
    log = vm.work / "qemu.log"
    if n == 1:
        # no unattended re-sleep after a wake no user input followed (the 2026-11-23 display probe's addendum 2)
        _, o = vm.guest("powercfg /setacvalueindex scheme_current sub_sleep 7bc4a2f9-d8fc-4469-b07b-33eb785aaca0 0; "
                        "powercfg /setactive scheme_current", 60)
    for _ in range(9):    # a cycle needs a baseline boot time; an ssh that fails here is retried, not judged
        if S.guest_facts(vm).get("boot"):
            break
        time.sleep(10)
    k0 = kf3_counts(log)
    t0 = time.time()
    res = S.cycle(vm, n, out, label)
    k1 = kf3_counts(log)
    t_end = time.time()
    res["cycle_start_utc"] = time.strftime("%H:%M:%SZ", time.gmtime(t0))
    res["kf3_delta"] = {k: k1.get(k, 0) - k0.get(k, 0) for k in KF3_COUNTS}
    res["qmp_events"] = [e for e in events_since(vm.work / "events.jsonl", t0) if e["event"] != "NIC_RX_FILTER_CHANGED"]
    res["cycle_end_utc"] = time.strftime("%H:%M:%SZ", time.gmtime(t_end))
    keep_keys(log, k0.get("lines", 0), out / ("c%d_kf3_keys.txt" % n))
    reasons = list(res.get("reasons", []))
    d = res["kf3_delta"]
    if res.get("suspended") and res.get("guest_alive", True):
        if d["kept"] < 1:
            reasons.append("no (S3 WAKE) ... KEPT")
        if d["gsp_resume"] < 1:
            reasons.append("no GSP-RESUME")
    for key, why in (("cold", "cold kf3 reset"), ("disp_stopped", "display channel STOPPED"), ("disp_stall", "core STALL"),
                     ("rm_unload", "RM unload (fn 47 newLevel=0)"), ("user_ring_topology", "USER_RING_REFUSED Topology")):
        if d[key] > 0:
            reasons.append(why)
    bad = [e["event"] for e in res["qmp_events"] if e["event"] in ("GUEST_PANICKED", "SHUTDOWN", "RESET")]
    if bad:
        reasons.append("QMP " + ",".join(bad))
    if res.get("event153_nvlddmkm"):
        reasons.append("nvlddmkm 153 x%d" % res["event153_nvlddmkm"])
    res["reasons"] = reasons
    if res["verdict"] == "CLEAN" and reasons:
        res["verdict"] = "FAIL-OTHER"
    return res


def run_item(state, a, label, arm, qemu, cycles, zero, deadline, results):
    done, boot_n = 0, 0
    while done < cycles and boot_n < cycles:
        if deadline and time.time() > deadline:
            S.say("ITEM %s: no-new-boot time reached, %d of %d cycles run" % (label, done, cycles))
            break
        boot_n += 1
        blabel = "%s_b%d" % (label, boot_n)
        out = S.OUT / blabel
        out.mkdir(parents=True)
        vm = PcVm(state, a.vmroot, blabel, Path(a.prep), qemu, S.ARM_CPU[arm], gpu=False, arm=arm, zero=zero, broker=a.broker, gop=a.gop)
        vm.setup()
        up0 = float(open("/proc/uptime").read().split()[0])
        SP.arm_boot_trace()
        bp = fd = rec = None
        storm = False
        try:
            bp, fd = start_broker(vm)
            argv = vm.start()
            (out / "argv_scrubbed.txt").write_text(S.scrub(" ".join(argv)) + "\n")
            S.active("BOOT %s arm=%s zero=%s qemu=%s" % (blabel, arm, zero, qemu))
            rec = subprocess.Popen([sys.executable, str(HERE / "qmp_events.py"), str(vm.work / "qmp-ev.sock"), str(vm.work / "events.jsonl")])
            up = vm.wait_up()
            S.say("%s guest up=%s" % (blabel, up))
            if not up:
                (out / "boot_failed.txt").write_text("guest did not answer ssh in 900 s; qemu rc=%s\n" % vm.proc.poll())
                S.say("%s %s" % (blabel, vm.stop(clean=False)))
                results.append({"label": blabel, "cycle": 0, "verdict": "NOT-RUN", "reasons": ["boot failed"]})
                continue
            time.sleep(90)
            _, pa = vm.guest("powercfg /a", 60)
            (out / "powercfg_a.txt").write_text(S.scrub(pa))
            while done < cycles:
                done += 1
                r = cycle(vm, done, out, blabel)
                r.update(item=label, arm=arm, zero=zero, qemu=qemu)
                results.append(r)
                (out / ("c%d_result.json" % done)).write_text(json.dumps(r, indent=1, default=str))
                S.say("S3_CYCLE item=%s boot=%s n=%d verdict=%s 153=%s reasons=%s" % (label, blabel, done, r["verdict"],
                      r.get("event153_nvlddmkm"), r.get("reasons")))
                if not r.get("guest_alive", True) or r["verdict"] != "CLEAN":
                    storm = True
                    break
                time.sleep(30)
            S.say("%s stop: %s" % (blabel, vm.stop(clean=not storm)))
        finally:
            for p in (rec, bp):
                if p is not None and p.poll() is None:
                    p.terminate()
            if fd is not None:
                os.close(fd)
            if vm.proc and vm.proc.poll() is None:
                vm.proc.kill()
                vm.proc.wait(30)
        time.sleep(1)
        up1 = float(open("/proc/uptime").read().split()[0])
        d = [ln for ln in S.sh("dmesg")[1].splitlines() if re.match(r"\[\s*(\d+\.\d+)\]", ln)
             and up0 - 2 <= float(re.match(r"\[\s*(\d+\.\d+)\]", ln).group(1)) <= up1 + 2]
        (out / "host_dmesg_window.txt").write_text(S.scrub("\n".join(ln for ln in d if re.search(r"NVRM|Xid|mmuWalk|NV_ERR|BUG:|Oops|hung|kvm", ln, re.I))[-60000:]) + "\n")
        qlog = (vm.work / "qemu.log").read_text(errors="replace")
        (out / "qemu_log_tail.txt").write_text(S.scrub(qlog[-30000:]))
        c = {"host_mmuWalkMap_lines": sum(1 for ln in d if "mmuWalkMap" in ln), "host_no_memory_lines": sum(1 for ln in d if "NV_ERR_NO_MEMORY" in ln),
             "qemu_log": kf3_counts(vm.work / "qemu.log"), "qmp_all": [e["event"] for e in events_since(vm.work / "events.jsonl", 0)]}
        (out / "boot_summary.json").write_text(json.dumps(c, indent=1))
        S.say("%s boot summary %s" % (blabel, {k: v for k, v in c.items() if k != "qemu_log"}))
        subprocess.run(["pkill", "-u", "root", "-f", str(vm.work)], check=False)
        # the overlay is this boot's own scratch (a child of PREP): removed here, the evidence is in `out`
        for f in vm.work.glob("disk.qcow2"):
            f.unlink()
    return done


def cmd_session(a):
    S.OUT = Path(a.out)
    S.OUT.mkdir(parents=True, exist_ok=True)
    SP.OUT = S.OUT
    state = json.loads((Path(a.base) / "state.json").read_text())
    plan = []
    for item in a.plan.split(","):
        label, arm, q, n, zero = item.split(":")
        assert arm in ("A", "B") and zero in ("default", "off") and os.access(q, os.X_OK), item
        plan.append((label, arm, q, int(n), zero))
    deadline = None
    if a.no_new_boot_after:
        hh, mm = (int(x) for x in a.no_new_boot_after.split(":"))
        t = time.gmtime()
        deadline = time.mktime((t.tm_year, t.tm_mon, t.tm_mday, hh, mm, 0, 0, 0, 0)) - time.timezone
        if deadline < time.time():
            deadline += 86400    # HH:MM names the next such time of day
    fd = os.open(S.LOCK, os.O_RDWR | os.O_CREAT)
    fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)

    def term(sig, frm):
        raise SystemExit("signal %d" % sig)
    for s in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(s, term)
    S.active("START S3 on kf3 (plan %s): no GPU bind, VM only, lock held" % a.plan)
    results = []
    try:
        (S.OUT / "host_before.txt").write_text(S.host_state("before"))
        for label, arm, q, n, zero in plan:
            run_item(state, a, label, arm, q, n, zero, deadline, results)
        return 0
    finally:
        (S.OUT / "results.json").write_text(json.dumps(results, indent=1, default=str))
        (S.OUT / "host_after.txt").write_text(S.host_state("after"))
        rc, o = S.sh(["nvidia-smi", "--query-gpu=name,driver_version", "--format=csv,noheader"], 60)
        S.active("END S3 on kf3: %d cycle records; host GPU: %s" % (len(results), o.strip()[:60] if rc == 0 else "rc=%s" % rc))
        S.say("SESSION_EXIT")
        os.close(fd)


def main():
    p = argparse.ArgumentParser()
    sp = p.add_subparsers(dest="cmd", required=True)
    r = sp.add_parser("session")
    for k in ("base", "vmroot", "prep", "broker", "gop", "out", "plan"):
        r.add_argument("--" + k, required=True)
    r.add_argument("--no-new-boot-after", default="")
    a = p.parse_args()
    sys.exit(cmd_session(a))


if __name__ == "__main__":
    main()
