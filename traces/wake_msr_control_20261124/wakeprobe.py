#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Host side of the S3 wake MSR control (runs as root on the box; stdlib only). One variant per invocation:

  wakeprobe.py --variant K-stock --qemu BIN --cpu FLAGS [--seed] --base BASE.qcow2 --cidata SEED.iso --key KEY --out DIR

Boots a fresh overlay of BASE, waits for ssh, optionally seeds the Hyper-V MSRs (guest_msr.py seed), then runs N cycles:
the guest suspends itself (guest_msr.py cycle), QMP query-status must say `suspended`, then QMP system_wakeup; tracefs
records the KVM Hyper-V events and the TSC offset writes across each cycle. Everything lands in DIR as text/JSON.
"""
import argparse
import gzip
import json
import os
import re
import socket
import subprocess
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
TR = Path("/sys/kernel/tracing")
EVENTS = ["kvm/kvm_msr", "kvm/kvm_hv_synic_set_msr", "kvm/kvm_hv_stimer_set_config", "kvm/kvm_hv_stimer_set_count",
          "kvm/kvm_write_tsc_offset", "kvm/kvm_hv_stimer_start_one_shot", "kvm/kvm_hv_stimer_start_periodic",
          "kvm/kvm_hv_stimer_expiration", "kvm/kvm_hv_hypercall"]
MSR_FILTER = "ecx >= 0x40000000 && ecx < 0x40000200"
PORT = 10022


def wput(rel, val):
    try:
        (TR / rel).write_text(val)
    except OSError as e:
        print("tracefs write %s failed: %s" % (rel, e), flush=True)


def trace_arm():
    wput("tracing_on", "0")
    wput("events/enable", "0")
    wput("buffer_size_kb", "65536")
    wput("trace", "")
    wput("events/kvm/kvm_msr/filter", MSR_FILTER)
    for ev in EVENTS:
        wput("events/%s/enable" % ev, "1")
    wput("tracing_on", "1")


def trace_take(path):
    wput("tracing_on", "0")
    text = (TR / "trace").read_text(errors="replace")
    with gzip.open(path, "wt") as f:
        f.write(text)
    wput("trace", "")
    wput("tracing_on", "1")
    return text


class Qmp:
    def __init__(self, path):
        self.s = socket.socket(socket.AF_UNIX)
        for _ in range(100):
            try:
                self.s.connect(path)
                break
            except OSError:
                time.sleep(0.2)
        self.f = self.s.makefile("rw")
        self.events = []
        self.f.readline()
        self.cmd("qmp_capabilities")

    def cmd(self, name, **args):
        self.f.write(json.dumps({"execute": name, "arguments": args}) + "\n")
        self.f.flush()
        while True:
            line = self.f.readline()
            if not line:
                raise RuntimeError("QMP closed")
            msg = json.loads(line)
            if "event" in msg:
                self.events.append([time.time(), msg["event"]])
            if "return" in msg or "error" in msg:
                return msg


def acpi_io(qmp):
    """The ACPI PM I/O regions as the machine maps them now (HMP `info mtree -f`)."""
    r = qmp.cmd("human-monitor-command", **{"command-line": "info mtree -f"}).get("return", "")
    return sorted({ln.strip() for ln in r.splitlines() if re.search(r"acpi-(cnt|evt|tmr)|apm-io|pm-smi", ln)})


def xp_page(qmp, gpa):
    """The first 24 bytes of the TSC page as QEMU sees guest memory (HMP xp): sequence, reserved, scale, offset."""
    if gpa is None:
        return None
    r = qmp.cmd("human-monitor-command", **{"command-line": "xp /3gx 0x%x" % gpa}).get("return", "")
    return " ".join(r.split())


def ssh_argv(key, cmd):
    return ["ssh", "-i", key, "-p", str(PORT), "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null",
            "-o", "ConnectTimeout=5", "-o", "LogLevel=ERROR", "-o", "BatchMode=yes", "ubuntu@127.0.0.1", cmd]


def gssh(key, cmd, timeout=60, stdin=None):
    r = subprocess.run(ssh_argv(key, cmd), input=stdin, capture_output=True, text=True, timeout=timeout)
    return r.returncode, r.stdout, r.stderr


def wait_ssh(key, limit):
    t0 = time.time()
    while time.time() - t0 < limit:
        try:
            if gssh(key, "true", timeout=10)[0] == 0:
                return time.time() - t0
        except subprocess.TimeoutExpired:
            pass
        time.sleep(1)
    return None


def summarize_trace(text):
    """Host writes (QEMU) and guest MSR accesses, in order, plus TSC offset writes and timer-event counts."""
    out = {"host_synic": [], "host_stimer": [], "guest_msr": [], "tsc_offset": [], "counts": {}}
    for line in text.splitlines():
        m = re.search(r"\s(\d+\.\d+): (kvm_\w+): (.*)$", line)
        if not m:
            continue
        ts, ev, body = float(m[1]), m[2], m[3]
        out["counts"][ev] = out["counts"].get(ev, 0) + 1
        if ev == "kvm_hv_synic_set_msr":
            out["host_synic" if body.rstrip().endswith("host 1") else "guest_msr"].append([ts, ev, body])
        elif ev in ("kvm_hv_stimer_set_config", "kvm_hv_stimer_set_count"):
            out["host_stimer" if body.rstrip().endswith("host 1") else "guest_msr"].append([ts, ev, body])
        elif ev == "kvm_msr":
            out["guest_msr"].append([ts, ev, body])
        elif ev == "kvm_write_tsc_offset":
            out["tsc_offset"].append([ts, ev, body])
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--variant", required=True)
    ap.add_argument("--qemu", required=True)
    ap.add_argument("--cpu", required=True)
    ap.add_argument("--seed", action="store_true")
    ap.add_argument("--base", required=True)
    ap.add_argument("--cidata", required=True)
    ap.add_argument("--key", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--cycles", type=int, default=3)
    ap.add_argument("--machine", default="q35,accel=kvm,smm=on")
    ap.add_argument("--ovmf-code", default=None, help="OVMF code image (pflash unit 0; default: QEMU's SeaBIOS)")
    ap.add_argument("--ovmf-vars", default=None, help="OVMF variable store template, copied per variant (pflash unit 1)")
    a = ap.parse_args()
    out = Path(a.out) / a.variant
    out.mkdir(parents=True, exist_ok=True)
    run = Path("/root/wakerun") / a.variant
    run.mkdir(parents=True, exist_ok=True)
    disk = run / "overlay.qcow2"
    if disk.exists():
        disk.unlink()
    subprocess.run(["qemu-img", "create", "-q", "-f", "qcow2", "-F", "qcow2", "-b", a.base, str(disk)], check=True)
    qmp_path = str(run / "qmp.sock")
    argv = [a.qemu, "-name", "wakemsr-" + a.variant,
            "-machine", a.machine, "-m", "1024", "-smp", "2,sockets=1,cores=2,threads=1", "-cpu", a.cpu,
            "-global", "ICH9-LPC.disable_s3=0", "-global", "ICH9-LPC.disable_s4=1",
            "-rtc", "base=utc", "-global", "kvm-pit.lost_tick_policy=discard",
            "-drive", "file=%s,if=virtio,format=qcow2" % disk, "-drive", "file=%s,if=virtio,format=raw,readonly=on" % a.cidata,
            "-netdev", "user,id=n0,hostfwd=tcp:127.0.0.1:%d-:22" % PORT, "-device", "virtio-net-pci,netdev=n0",
            "-qmp", "unix:%s,server=on,wait=off" % qmp_path, "-serial", "file:%s" % (out / "serial.log"),
            "-display", "none", "-vga", "std", "-trace", "system_wakeup_request"]
    if a.ovmf_code:
        vars_copy = run / "OVMF_VARS.fd"
        vars_copy.write_bytes(Path(a.ovmf_vars).read_bytes())
        argv += ["-global", "driver=cfi.pflash01,property=secure,value=on",
                 "-drive", "if=pflash,format=raw,unit=0,readonly=on,file=%s" % a.ovmf_code,
                 "-drive", "if=pflash,format=raw,unit=1,file=%s" % vars_copy]
    (out / "qemu_argv.json").write_text(json.dumps(argv[1:], indent=1))
    ver = subprocess.run([a.qemu, "--version"], capture_output=True, text=True).stdout.strip()
    res = {"variant": a.variant, "cpu": a.cpu, "seeded": a.seed, "qemu_version": ver, "cycles": []}
    trace_arm()
    q = subprocess.Popen(argv, stdout=open(out / "qemu_stdout.log", "w"), stderr=subprocess.STDOUT)
    try:
        qmp = Qmp(qmp_path)
        res["boot_ssh_s"] = wait_ssh(a.key, 240)
        if res["boot_ssh_s"] is None:
            res["error"] = "no ssh after boot"
            return res
        helper = (HERE / "guest_msr.py").read_text()
        gssh(a.key, "cat > /tmp/guest_msr.py", stdin=helper)
        rc, so, _ = gssh(a.key, "uname -r; cat /proc/cmdline; sudo dmesg | grep -i -E 'hypervisor detected|hyper-v|clocksource' | head -20")
        (out / "guest_boot_info.txt").write_text(so)
        boot_trace = trace_take(out / "boot_trace.txt.gz")
        page_gpa = 0x3D001000 if a.seed else None
        for m in re.finditer(r"msr_write 40000021 = (0x[0-9a-f]+)", boot_trace):
            v = int(m[1], 16)
            page_gpa = (v & ~0xFFF) if v & 1 else page_gpa
        res["tsc_page_gpa"] = hex(page_gpa) if page_gpa is not None else None
        res["boot_trace"] = summarize_trace(boot_trace)["counts"]
        for n in range(1, a.cycles + 1):
            cyc = {"n": n}
            gssh(a.key, "sudo rm -rf /root/wm; sudo mkdir -p /root/wm")
            if a.seed:
                rc, so, se = gssh(a.key, "sudo python3 /tmp/guest_msr.py seed")
                (out / ("c%d_seed.json" % n)).write_text(so + se)
            qmp.events.clear()
            cyc["acpi_io_before"] = acpi_io(qmp)
            cyc["tsc_page_before"] = xp_page(qmp, page_gpa)
            wput("trace_marker", "WAKEPROBE cycle %d start" % n)
            subprocess.Popen(ssh_argv(a.key, "sudo sh -c 'setsid nohup python3 /tmp/guest_msr.py cycle /root/wm %d >/root/wm/c%d_helper.log 2>&1 &'" % (n, n)),
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            t0 = time.time()
            status = None
            while time.time() - t0 < 60:
                status = qmp.cmd("query-status").get("return", {}).get("status")
                if status == "suspended":
                    break
                time.sleep(0.5)
            cyc["status_before_wake"] = status
            cyc["qmp_events_before_wake"] = list(qmp.events)
            if status != "suspended":
                cyc["verdict"] = "no suspend"
                time.sleep(3)
                cyc["acpi_io_after"] = acpi_io(qmp)
                for name in ("c%d_helper.log" % n, "c%d_pre.json" % n, "c%d_post.json" % n, "c%d_dmesg.txt" % n):
                    rc, so, se = gssh(a.key, "sudo cat /root/wm/%s" % name)
                    (out / ("nosuspend_" + name)).write_text(so + se)
                res["cycles"].append(cyc)
                continue
            cyc["host_mono_suspended"] = time.clock_gettime(time.CLOCK_MONOTONIC)
            time.sleep(2)
            cyc["tsc_page_suspended"] = xp_page(qmp, page_gpa)
            time.sleep(3)
            wput("trace_marker", "WAKEPROBE cycle %d system_wakeup" % n)
            cyc["host_realtime_wake"] = time.time()
            cyc["host_mono_wake"] = time.clock_gettime(time.CLOCK_MONOTONIC)
            cyc["wakeup_reply"] = qmp.cmd("system_wakeup")
            cyc["ssh_back_s"] = wait_ssh(a.key, 90)
            for _ in range(30):
                if gssh(a.key, "sudo test -e /root/wm/c%d_done" % n)[0] == 0:
                    break
                time.sleep(1)
            cyc["host_realtime_after"] = time.time()
            cyc["host_mono_after"] = time.clock_gettime(time.CLOCK_MONOTONIC)
            for kind in ("pre", "post"):
                rc, so, _ = gssh(a.key, "sudo cat /root/wm/c%d_%s.json" % (n, kind))
                (out / ("c%d_%s.json" % (n, kind))).write_text(so)
            rc, so, _ = gssh(a.key, "sudo cat /root/wm/c%d_dmesg.txt" % n)
            (out / ("c%d_dmesg.txt" % n)).write_text(so)
            time.sleep(3)
            text = trace_take(out / ("c%d_trace.txt.gz" % n))
            s = summarize_trace(text)
            (out / ("c%d_trace_summary.json" % n)).write_text(json.dumps(s, indent=1))
            cyc["trace_counts"] = s["counts"]
            cyc["acpi_io_after"] = acpi_io(qmp)
            cyc["tsc_page_after"] = xp_page(qmp, page_gpa)
            qmp.cmd("query-status")
            cyc["qmp_events"] = list(qmp.events)
            res["cycles"].append(cyc)
            (out / "result.json").write_text(json.dumps(res, indent=1))
        try:
            qmp.cmd("quit")
        except (RuntimeError, OSError):
            pass
    finally:
        try:
            q.wait(timeout=20)
        except subprocess.TimeoutExpired:
            q.kill()
        wput("tracing_on", "0")
        wput("events/enable", "0")
        (out / "result.json").write_text(json.dumps(res, indent=1))
    return res


if __name__ == "__main__":
    r = main()
    print("WAKEPROBE_DONE variant=%s cycles=%d" % (r["variant"], len(r["cycles"])), flush=True)
