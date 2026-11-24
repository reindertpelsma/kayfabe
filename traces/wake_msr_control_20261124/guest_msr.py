#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Guest side of the S3 wake MSR control (runs as root inside the Linux guest; stdlib only).

  guest_msr.py seed            write the known Hyper-V MSR values (variant K only), print the result as JSON
  guest_msr.py read            print every probed MSR of every CPU, the clocks and the TSC page bytes as JSON
  guest_msr.py cycle OUTDIR N  read -> OUTDIR/cN_pre.json, suspend to RAM, read the moment the suspend returns ->
                               OUTDIR/cN_post.json, then the kernel log lines of the cycle -> OUTDIR/cN_dmesg.txt
"""
import glob
import json
import mmap
import os
import struct
import subprocess
import sys
import time

MSRS = {
    0x40000000: "GUEST_OS_ID", 0x40000001: "HYPERCALL", 0x40000021: "REFERENCE_TSC",
    0x40000080: "SCONTROL", 0x40000082: "SIEFP", 0x40000083: "SIMP", 0x40000092: "SINT2",
    0x400000B0: "STIMER0_CONFIG", 0x400000B1: "STIMER0_COUNT", 0x40000020: "TIME_REF_COUNT",
}
PAGE_BASE = 0x3D000000
TSC_PAGE = PAGE_BASE + 0x1000


def cpus():
    return sorted(int(p.split("/")[-1]) for p in glob.glob("/dev/cpu/[0-9]*"))


def rdmsr(cpu, msr):
    try:
        fd = os.open("/dev/cpu/%d/msr" % cpu, os.O_RDONLY)
        try:
            return "0x%016x" % struct.unpack("<Q", os.pread(fd, 8, msr))[0]
        finally:
            os.close(fd)
    except OSError as e:
        return "ERR:%s" % (e.strerror or e.errno)


def wrmsr(cpu, msr, value):
    try:
        fd = os.open("/dev/cpu/%d/msr" % cpu, os.O_WRONLY)
        try:
            os.pwrite(fd, struct.pack("<Q", value), msr)
            return "ok"
        finally:
            os.close(fd)
    except OSError as e:
        return "ERR:%s" % (e.strerror or e.errno)


def seed_plan():
    plan = [(0, 0x40000000, 0x8100000000000001), (0, 0x40000001, PAGE_BASE | 1), (0, 0x40000021, TSC_PAGE | 1)]
    for c in cpus():
        plan += [(c, 0x40000080, 1), (c, 0x40000082, (PAGE_BASE + 0x2000 + c * 0x2000) | 1),
                 (c, 0x40000083, (PAGE_BASE + 0x3000 + c * 0x2000) | 1),
                 (c, 0x400000B1, 0x12345678 + c), (c, 0x400000B0, 0x20000)]
    return plan


def tsc_page():
    try:
        fd = os.open("/dev/mem", os.O_RDONLY | os.O_SYNC)
        try:
            m = mmap.mmap(fd, 4096, mmap.MAP_SHARED, mmap.PROT_READ, offset=TSC_PAGE)
            seq, _res, scale, off = struct.unpack("<IIQq", m[:24])
            m.close()
            return {"sequence": seq, "scale": "0x%016x" % scale, "offset": off}
        finally:
            os.close(fd)
    except (OSError, ValueError) as e:
        return {"error": str(e)}


def snapshot():
    def rd(path):
        try:
            with open(path) as f:
                return f.read().strip()
        except OSError as e:
            return "ERR:%s" % e
    out = {"t_realtime": time.clock_gettime(time.CLOCK_REALTIME), "t_monotonic": time.clock_gettime(time.CLOCK_MONOTONIC),
           "t_boottime": time.clock_gettime(time.CLOCK_BOOTTIME)}
    out["msrs"] = {str(c): {MSRS[m]: rdmsr(c, m) for m in MSRS} for c in cpus()}
    out["clocksource"] = rd("/sys/devices/system/clocksource/clocksource0/current_clocksource")
    out["clocksource_available"] = rd("/sys/devices/system/clocksource/clocksource0/available_clocksource")
    out["tsc_page"] = tsc_page()
    return out


def dump(path, obj):
    with open(path, "w") as f:
        json.dump(obj, f, indent=1, sort_keys=True)
        f.flush()
        os.fsync(f.fileno())


def cycle(outdir, n):
    os.makedirs(outdir, exist_ok=True)
    subprocess.run(["dmesg", "-C"], check=False)
    pre = snapshot()
    dump(os.path.join(outdir, "c%s_pre.json" % n), pre)
    os.sync()
    with open("/sys/power/mem_sleep", "w") as f:
        f.write("deep")
    err = None
    try:
        with open("/sys/power/state", "w") as f:
            f.write("mem")
    except OSError as e:
        err = str(e)
    post = snapshot()
    post["suspend_error"] = err
    dump(os.path.join(outdir, "c%s_post.json" % n), post)
    log = subprocess.run(["dmesg"], capture_output=True, text=True, check=False).stdout
    with open(os.path.join(outdir, "c%s_dmesg.txt" % n), "w") as f:
        f.write(log)
    with open(os.path.join(outdir, "c%s_done" % n), "w") as f:
        f.write("done\n")


def main(argv):
    subprocess.run(["modprobe", "msr"], check=False)
    if argv[0] == "seed":
        res = [{"cpu": c, "msr": MSRS[m], "value": "0x%x" % v, "result": wrmsr(c, m, v)} for c, m, v in seed_plan()]
        print(json.dumps(res))
    elif argv[0] == "read":
        print(json.dumps(snapshot(), sort_keys=True))
    elif argv[0] == "cycle":
        cycle(argv[1], argv[2])
    else:
        sys.exit("usage: guest_msr.py seed|read|cycle OUTDIR N")


if __name__ == "__main__":
    main(sys.argv[1:])
