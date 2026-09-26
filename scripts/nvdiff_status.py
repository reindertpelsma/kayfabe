#!/usr/bin/env python3
"""nvdiff_status.py -- the STATUS differential over nvdiff captures (v3-refusals, 2026-09-26).

Why: bare metal returns a non-OK RM status exactly once in 613 records (0x2080012f, in cuInit);
every other non-OK status the kf3 guest reads is a divergence until proven harmless
(docs/design/V3_REFUSAL_AUDIT.md). The positional differ (archive/nvkvm/tests/mode2/nvdiff/
nvdiff.py) aligns ONE program's stream; real workloads (torch, llama.cpp, ffmpeg, Vulkan) are
multi-threaded and timing-dependent, so their streams do not align record for record. This tool
compares STATUS CENSUSES instead: per operation (escape + control id / class / UVM ioctl), which
statuses each side read and how often. A guest (op, status) the host never produced for that op
is a divergence, whatever the order.

Sub-commands
  census  CAP...                         per-op status census of one side (all captures pooled)
  compare --host CAP... --guest CAP...   guest non-OK statuses the host never read for that op
  matrix  DIR [--json OUT]               DIR/<workload>/host*.jsonl* vs DIR/<workload>/guest*.jsonl*,
                                         one row per divergent (op, status), across workloads

Status sources, per record (four states, never a bool -- nvdiff_inband.py's rule):
  RM escapes      the NVOS* in-band `status` at its escape's offset (ogkm nvos.h); header captured
                  AFTER the call. The ioctl rc is kept too (rc<0 = errno).
  UVM ioctls      `rmStatus` at offsetof(<X>_PARAMS, rmStatus), generated from ogkm uvm_ioctl.h
                  (UVM_STATUS_AT below; regenerate with --gen-uvm OGKM).
  others          rc/errno only.
  Truncated       the header was captured too short to hold the status: UNMEASURED, never OK.

Control names are read from the ogkm headers at run time (--ogkm, default: the research clone or
third_party/ogkm-580) -- derived, not captured.
"""
import argparse
import glob
import io
import json
import os
import re
import subprocess
import sys
from collections import Counter, defaultdict

# ------------------------------------------------------------------ escape layouts (ogkm nvos.h)
ESC = {0x27: "RM_ALLOC_MEMORY", 0x28: "RM_ALLOC_OBJECT", 0x29: "RM_FREE", 0x2A: "RM_CONTROL",
       0x2B: "RM_ALLOC", 0x32: "RM_CONFIG_GET", 0x33: "RM_CONFIG_SET", 0x34: "RM_DUP_OBJECT",
       0x35: "RM_SHARE", 0x37: "RM_CONFIG_GET_EX", 0x38: "RM_CONFIG_SET_EX", 0x39: "RM_I2C_ACCESS",
       0x41: "RM_IDLE_CHANNELS", 0x4A: "RM_VID_HEAP_CONTROL", 0x4D: "RM_ACCESS_REGISTRY",
       0x4E: "RM_MAP_MEMORY", 0x4F: "RM_UNMAP_MEMORY", 0x52: "RM_GET_EVENT_DATA",
       0x54: "RM_ALLOC_CONTEXT_DMA2", 0x56: "RM_ADD_VBLANK_CALLBACK", 0x57: "RM_MAP_MEMORY_DMA",
       0x58: "RM_UNMAP_MEMORY_DMA", 0x59: "RM_BIND_CONTEXT_DMA", 0x5C: "RM_EXPORT_OBJECT_TO_FD",
       0x5D: "RM_IMPORT_OBJECT_FROM_FD", 0x5E: "RM_UPDATE_DEVICE_MAPPING_INFO",
       0x5F: "RM_LOCKLESS_DIAGNOSTIC", 0xC8: "CARD_INFO", 0xC9: "REGISTER_FD",
       0xCA: "ALLOC_OS_EVENT_OLD", 0xCE: "ALLOC_OS_EVENT", 0xCF: "FREE_OS_EVENT",
       0xD2: "CHECK_VERSION_STR", 0xD3: "IOCTL_XFER_CMD", 0xD6: "SYS_PARAMS", 0xD8: "NUMA_INFO",
       0xD9: "SET_NUMA_STATUS", 0xDA: "EXPORT_TO_DMABUF_FD", 0xDB: "WAIT_OPEN_COMPLETE"}

NVSTATUS = {0x00: "NV_OK"}   # filled from ogkm nvstatuscodes.h by load_status_names()


def load_status_names(ogkm):
    """NV_STATUS value -> name, from `NV_STATUS_CODE(NAME, 0x..., "...")` (derived, never typed)."""
    if not ogkm:
        return
    p = os.path.join(ogkm, "src/common/sdk/nvidia/inc/nvstatuscodes.h")
    try:
        txt = open(p, errors="replace").read()
    except OSError:
        return
    for m in re.finditer(r"NV_STATUS_CODE\((NV_[A-Z0-9_]+),\s*(0x[0-9a-fA-F]+)", txt):
        NVSTATUS.setdefault(int(m.group(2), 16), m.group(1))


# escape -> (status offset) given iocsize; nvdiff_inband.py's table plus the os-event escapes
def status_off(nr, iocsize):
    return {0x27: 40, 0x29: 12, 0x2A: 28, 0x34: 24, 0x35: 20, 0x4A: 20, 0x4E: 40, 0x4F: 24,
            0x58: 40, 0x5E: 32, 0xCE: 12, 0xCF: 12, 0xDB: 4}.get(nr) if nr not in (0x2B, 0x57) else (
        (40 if iocsize >= 48 else 28) if nr == 0x2B else (56 if iocsize >= 64 else 48))


# UVM: request -> (offsetof(rmStatus), sizeof(params), name). GENERATED from ogkm-580.159.04
# uvm_ioctl.h / uvm_linux_ioctl.h by `--gen-uvm OGKM` (a compiled offsetof, never hand-typed).
UVM_STATUS_AT = {
    10: (20, 24, "ADD_SESSION"),
    78: (52, 56, "ALLOC_DEVICE_P2P"),
    68: (9240, 9248, "ALLOC_SEMAPHORE_POOL"),
    41: (264, 272, "ALLOW_MIGRATION_RANGE_GROUPS"),
    69: (0, 4, "CLEAN_UP_ZOMBIE_RESOURCES"),
    79: (0, 4, "CLEAR_ALL_ACCESS_COUNTERS"),
    14: (28, 32, "CREATE_EVENT_QUEUE"),
    73: (16, 24, "CREATE_EXTERNAL_RANGE"),
    23: (8, 16, "CREATE_RANGE_GROUP"),
    24: (8, 16, "DESTROY_RANGE_GROUP"),
    30: (32, 36, "DISABLE_PEER_ACCESS"),
    45: (16, 24, "DISABLE_READ_DUPLICATION"),
    55: (16, 20, "DISABLE_SYSTEM_WIDE_ATOMICS"),
    80: (24, 32, "DISCARD"),
    12: (904, 908, "ENABLE_COUNTERS"),
    29: (32, 36, "ENABLE_PEER_ACCESS"),
    44: (16, 24, "ENABLE_READ_DUPLICATION"),
    54: (16, 20, "ENABLE_SYSTEM_WIDE_ATOMICS"),
    17: (16, 20, "EVENT_CTRL"),
    34: (16, 24, "FREE"),
    20: (516, 520, "GET_GPU_UUID_TABLE"),
    2047: (4, 8, "IS_8_SUPPORTED"),
    13: (40, 48, "MAP_COUNTER"),
    65: (32, 40, "MAP_DYNAMIC_PARALLELISM_REGION"),
    16: (48, 56, "MAP_EVENT_QUEUE"),
    33: (9260, 9264, "MAP_EXTERNAL_ALLOCATION"),
    74: (32, 40, "MAP_EXTERNAL_SPARSE"),
    35: (16, 24, "MEM_MAP"),
    51: (72, 80, "MIGRATE"),
    53: (24, 32, "MIGRATE_RANGE_GROUP"),
    75: (4, 8, "MM_INITIALIZE"),
    39: (4, 8, "PAGEABLE_MEM_ACCESS"),
    70: (20, 24, "PAGEABLE_MEM_ACCESS_ON_GPU"),
    71: (20, 24, "POPULATE_PAGEABLE"),
    40: (264, 272, "PREVENT_MIGRATION_RANGE_GROUPS"),
    3: (40, 48, "REGION_COMMIT"),
    4: (16, 24, "REGION_DECOMMIT"),
    27: (48, 56, "REGISTER_CHANNEL"),
    37: (36, 40, "REGISTER_GPU"),
    25: (28, 32, "REGISTER_GPU_VASPACE"),
    19: (8, 16, "REGISTER_MPS_CLIENT"),
    18: (528, 536, "REGISTER_MPS_SERVER"),
    2: (16, 24, "RELEASE_VA"),
    15: (8, 12, "REMOVE_EVENT_QUEUE"),
    11: (4, 8, "REMOVE_SESSION"),
    1: (16, 24, "RESERVE_VA"),
    9: (40, 44, "RUN_TEST"),
    46: (32, 40, "SET_ACCESSED_BY"),
    42: (36, 40, "SET_PREFERRED_LOCATION"),
    31: (24, 32, "SET_RANGE_GROUP"),
    6: (8, 16, "SET_STREAM_RUNNING"),
    7: (264, 272, "SET_STREAM_STOPPED"),
    61: (8, 16, "TOOLS_DISABLE_COUNTERS"),
    60: (8, 16, "TOOLS_ENABLE_COUNTERS"),
    59: (8, 16, "TOOLS_EVENT_QUEUE_DISABLE_EVENTS"),
    58: (8, 16, "TOOLS_EVENT_QUEUE_ENABLE_EVENTS"),
    67: (0, 4, "TOOLS_FLUSH_EVENTS"),
    64: (8, 16, "TOOLS_GET_PROCESSOR_UUID_TABLE"),
    56: (48, 56, "TOOLS_INIT_EVENT_TRACKER"),
    62: (32, 40, "TOOLS_READ_PROCESS_MEMORY"),
    57: (4, 8, "TOOLS_SET_NOTIFICATION_THRESHOLD"),
    63: (32, 40, "TOOLS_WRITE_PROCESS_MEMORY"),
    66: (32, 40, "UNMAP_EXTERNAL"),
    28: (24, 28, "UNREGISTER_CHANNEL"),
    38: (16, 20, "UNREGISTER_GPU"),
    26: (16, 20, "UNREGISTER_GPU_VASPACE"),
    47: (32, 40, "UNSET_ACCESSED_BY"),
    43: (16, 24, "UNSET_PREFERRED_LOCATION"),
    72: (16, 24, "VALIDATE_VA_RANGE"),
    0x30000001: (8, 16, "INITIALIZE"),
}


def open_text(path):
    if path.endswith(".zst"):
        try:
            import zstandard
            return io.TextIOWrapper(zstandard.ZstdDecompressor().stream_reader(open(path, "rb")),
                                    errors="replace")
        except ImportError:
            p = subprocess.run(["zstd", "-dc", path], stdout=subprocess.PIPE, check=True)
            return io.StringIO(p.stdout.decode("utf-8", "replace"))
    if path.endswith(".gz"):
        import gzip
        return gzip.open(path, "rt", errors="replace")
    return open(path, "r", errors="replace")


def u32(hexs, off):
    if len(hexs) < 2 * (off + 4):
        return None
    try:
        return int.from_bytes(bytes.fromhex(hexs[2 * off:2 * off + 8]), "little")
    except ValueError:
        return None


def devclass(dev):
    if dev.startswith("nvidia-uvm"):
        return "uvm"
    if dev == "nvidiactl":
        return "ctl"
    if dev.startswith("nvidia-modeset"):
        return "modeset"
    if re.fullmatch(r"nvidia\d+", dev):
        return "gpu"
    return dev


def classify(rec):
    """-> (op, outcome) or None. outcome: 'OK' | 'st=0x..' | 'rc=..' | 'TRUNC' | 'NOSTATUS'."""
    if rec.get("t") != "ioctl":
        return None
    dev = rec.get("dev") or ""
    hpre = rec.get("hpre") or ""
    hpost = rec.get("hpost") or ""
    rc = rec.get("rc", 0)
    err = rec.get("errno", 0)
    try:
        req = int(rec.get("req", "0"), 16)
    except ValueError:
        req = 0
    nr = rec.get("nr", 0)
    iocsize = rec.get("iocsize", 0) or 0
    dc = devclass(dev)
    if dc == "uvm":
        ent = UVM_STATUS_AT.get(req)
        name = rec.get("uvm") or (ent[2] if ent else "nr%d" % req)
        op = "uvm:" + name
        if rc and rc < 0:
            return op, "rc=%d/errno=%d" % (rc, err)
        if not ent:
            return op, "NOSTATUS"
        st = u32(hpost, ent[0])
        if st is None:
            return op, "TRUNC"
        return op, "OK" if st == 0 else "st=0x%x" % st
    esc = ESC.get(nr, "nr0x%02x" % nr)
    if nr == 0x2A:
        cmd = u32(hpre, 8)
        op = "%s:RM_CONTROL:0x%08x" % (dc, cmd if cmd is not None else 0)
    elif nr in (0x2B, 0x27):
        cls = u32(hpre, 12)
        op = "%s:%s:cls=0x%04x" % (dc, esc, cls if cls is not None else 0)
    elif nr == 0x4A:
        fn = u32(hpre, 8)
        op = "%s:%s:fn=%s" % (dc, esc, fn)
    else:
        op = "%s:%s" % (dc, esc)
    if rc and rc < 0:
        return op, "rc=%d/errno=%d" % (rc, err)
    off = status_off(nr, iocsize) if dc in ("ctl", "gpu") else None
    if off is None:
        return op, "OK" if not rc else "rc=%d" % rc
    st = u32(hpost, off)
    if st is None:
        return op, "TRUNC"
    if nr == 0xDB:   # WAIT_OPEN_COMPLETE: rc@0 (int) and adapterStatus@4
        r0 = u32(hpost, 0)
        return op, "OK" if (st == 0 and not r0) else "st=0x%x/rc=%d" % (st, r0 or 0)
    return op, "OK" if st == 0 else "st=0x%x" % st


class Census(object):
    def __init__(self):
        self.ops = defaultdict(Counter)    # op -> outcome -> n
        self.first = {}                    # (op, outcome) -> (file, i)
        self.records = 0
        self.files = []

    def add_file(self, path):
        self.files.append(path)
        with open_text(path) as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                try:
                    rec = json.loads(line)
                except ValueError:
                    continue
                c = classify(rec)
                if c is None:
                    continue
                self.records += 1
                op, oc = c
                self.ops[op][oc] += 1
                self.first.setdefault((op, oc), (os.path.basename(path), rec.get("i")))
        return self


def load_names(ogkm):
    """control id -> name, from `#define NVxxxx_CTRL_CMD_FOO (0x...)` in the ogkm SDK headers."""
    names = {}
    if not ogkm:
        return names
    root = os.path.join(ogkm, "src/common/sdk/nvidia/inc/ctrl")
    rx = re.compile(r"#define\s+(NV[0-9A-Z]+_CTRL_CMD_[A-Z0-9_]+)\s+\((0x[0-9a-fA-F]+)U?\)")
    for dp, _, fs in os.walk(root):
        for f in fs:
            if not f.endswith(".h"):
                continue
            try:
                txt = open(os.path.join(dp, f), errors="replace").read()
            except OSError:
                continue
            for m in rx.finditer(txt):
                v = int(m.group(2), 16)
                if v > 0xffff:
                    names.setdefault(v, m.group(1))
    return names


def op_label(op, names):
    m = re.search(r"RM_CONTROL:0x([0-9a-f]{8})", op)
    if m:
        v = int(m.group(1), 16)
        return "%s %s" % (op, names.get(v, "?"))
    return op


def oc_label(oc):
    m = re.fullmatch(r"st=0x([0-9a-f]+)", oc)
    return "%s(%s)" % (oc, NVSTATUS.get(int(m.group(1), 16), "?")) if m else oc


def is_bad(oc):
    return oc not in ("OK", "NOSTATUS")


def divergences(host, guest):
    """Guest (op, outcome) pairs, non-OK, that the host never read for that op."""
    out = []
    for op, ocs in guest.ops.items():
        for oc, n in ocs.items():
            if not is_bad(oc):
                continue
            h = host.ops.get(op, Counter())
            if h.get(oc, 0):
                continue            # the host reads the same status for this op: not a divergence
            out.append({"op": op, "outcome": oc, "guest_n": n, "guest_calls": sum(ocs.values()),
                        "host_ok": h.get("OK", 0), "host_calls": sum(h.values()),
                        "host_outcomes": dict(h), "first": guest.first.get((op, oc))})
    out.sort(key=lambda d: (-d["guest_n"], d["op"]))
    return out


def reverse(host, guest):
    """Host non-OK (op, outcome) the guest never read -- informational (e.g. 0x2080012f)."""
    out = []
    for op, ocs in host.ops.items():
        for oc, n in ocs.items():
            if is_bad(oc) and not guest.ops.get(op, Counter()).get(oc, 0):
                out.append((op, oc, n, sum(guest.ops.get(op, Counter()).values())))
    return out


def cmd_census(args, names):
    c = Census()
    for p in args.caps:
        c.add_file(p)
    print("records=%d ops=%d files=%d" % (c.records, len(c.ops), len(c.files)))
    for op in sorted(c.ops):
        ocs = c.ops[op]
        bad = {k: v for k, v in ocs.items() if is_bad(k)}
        if bad or args.all:
            print("  %-70s %s" % (op_label(op, names)[:70], " ".join("%s:%d" % kv for kv in sorted(ocs.items()))))


def cmd_compare(args, names):
    h = Census()
    for p in args.host:
        h.add_file(p)
    g = Census()
    for p in args.guest:
        g.add_file(p)
    print("host: %d records in %d files   guest: %d records in %d files" % (
        h.records, len(h.files), g.records, len(g.files)))
    if h.records < 50 or g.records < 50:
        print("★ WARNING: a side has < 50 records -- an empty or tiny capture is UNMEASURED, not clean")
    d = divergences(h, g)
    print("DIVERGENT guest non-OK statuses (host never read that status for that op): %d" % len(d))
    for x in d:
        print("  %-64s %-22s guest %d/%d  host ok %d/%d  first=%s" % (
            op_label(x["op"], names)[:64], oc_label(x["outcome"]), x["guest_n"], x["guest_calls"],
            x["host_ok"], x["host_calls"], x["first"]))
    r = reverse(h, g)
    if r:
        print("host-only non-OK (informational):")
        for op, oc, n, gcalls in r:
            print("  %-64s %-22s host %d  guest calls %d" % (op_label(op, names)[:64], oc_label(oc), n, gcalls))
    return d


def cmd_matrix(args, names):
    rows = defaultdict(lambda: {"workloads": {}, "host_ok": 0, "host_calls": 0})
    per = {}
    for wdir in sorted(glob.glob(os.path.join(args.dir, "*"))):
        if not os.path.isdir(wdir):
            continue
        w = os.path.basename(wdir)
        hs = sorted(glob.glob(os.path.join(wdir, "host*.jsonl*")))
        gs = sorted(glob.glob(os.path.join(wdir, "guest*.jsonl*")))
        if not hs or not gs:
            per[w] = "SKIP (host=%d guest=%d captures)" % (len(hs), len(gs))
            continue
        h = Census()
        for p in hs:
            h.add_file(p)
        g = Census()
        for p in gs:
            g.add_file(p)
        d = divergences(h, g)
        per[w] = "host %d rec, guest %d rec, %d divergent" % (h.records, g.records, len(d))
        if h.records < 50 or g.records < 50:
            per[w] += "  ★ TINY CAPTURE -- UNMEASURED"
        for x in d:
            k = (x["op"], x["outcome"])
            rows[k]["workloads"][w] = x["guest_n"]
            rows[k]["host_ok"] += x["host_ok"]
            rows[k]["host_calls"] += x["host_calls"]
    print("== per workload")
    for w, s in per.items():
        print("  %-22s %s" % (w, s))
    print("== divergent (op, status), by number of workloads then calls")
    order = sorted(rows.items(), key=lambda kv: (-len(kv[1]["workloads"]), -sum(kv[1]["workloads"].values())))
    for (op, oc), r in order:
        print("  %-70s %-18s host_ok=%d/%d  in: %s" % (
            op_label(op, names)[:70], oc_label(oc), r["host_ok"], r["host_calls"],
            " ".join("%s:%d" % kv for kv in sorted(r["workloads"].items()))))
    if args.json:
        json.dump([{"op": op, "label": op_label(op, names), "outcome": oc, **r}
                   for (op, oc), r in order], open(args.json, "w"), indent=1)


def selftest(ref):
    """Negative + positive control, offline (no GPU), over the committed host reference captures.

    NEGATIVE: host r1 vs host r2 of each stage must report ZERO divergent statuses.
    POSITIVE: a copy of r1 with ONE RM_CONTROL status flipped to 0x56 must report EXACTLY that
    one -- a census that reports zero must first be shown to report one.
    """
    import tempfile
    fail = 0
    for st in ("init", "dev", "ctx", "alloc", "ce", "launch"):
        a = os.path.join(ref, "%s_r1.jsonl.zst" % st)
        b = os.path.join(ref, "%s_r2.jsonl.zst" % st)
        if not (os.path.exists(a) and os.path.exists(b)):
            print("  %-8s SKIP (missing)" % st)
            continue
        d = divergences(Census().add_file(a), Census().add_file(b))
        print("  NEG %-8s %s  (%d divergent)" % (st, "PASS" if not d else "FAIL", len(d)))
        fail |= bool(d)
    src = os.path.join(ref, "ce_r1.jsonl.zst")
    recs = [json.loads(l) for l in open_text(src) if l.strip()]
    victim = None
    for r in recs:
        c = classify(r)
        if c and c[1] == "OK" and ":RM_CONTROL:" in c[0] and len(r.get("hpost", "")) >= 64:
            victim = r
            break
    h = victim["hpost"]
    victim["hpost"] = h[:56] + "56000000" + h[64:]      # NVOS54.status @28 := 0x56
    fd, path = tempfile.mkstemp(suffix=".jsonl")
    with os.fdopen(fd, "w") as fh:
        for r in recs:
            fh.write(json.dumps(r) + "\n")
    d = divergences(Census().add_file(src), Census().add_file(path))
    ok = len(d) == 1 and d[0]["outcome"] == "st=0x56" and d[0]["op"] == classify(victim)[0]
    print("  POS flipped %-40s %s  (%d divergent: %s)" % (
        classify(victim)[0], "PASS" if ok else "FAIL", len(d), [x["op"] for x in d]))
    os.unlink(path)
    fail |= not ok
    print("SELFTEST", "FAIL" if fail else "PASS")
    return 1 if fail else 0


def gen_uvm(ogkm):
    """Print the UVM_STATUS_AT table from ogkm headers (compiled offsetof)."""
    import tempfile
    uvm = os.path.join(ogkm, "kernel-open/nvidia-uvm")
    txt = open(os.path.join(uvm, "uvm_ioctl.h")).read()
    nums = sorted(set(re.findall(r"#define\s+(UVM_[A-Z0-9_]+)\s+UVM_IOCTL_BASE\((\d+)\)", txt)))
    lines = ['#include <stdio.h>', '#include <stddef.h>', '#include "uvm_ioctl.h"',
             '#include "uvm_linux_ioctl.h"', 'int main(void){']
    for name, num in nums:
        m = re.search(r"typedef struct\s*\{((?:(?!typedef struct).)*?)\}\s*%s_PARAMS;" % name, txt, re.S)
        if m and "rmStatus" in m.group(1):
            lines.append('printf("    %%u: (%%u, %%u, \\"%s\\"),\\n", (unsigned)%s, '
                         '(unsigned)offsetof(%s_PARAMS, rmStatus), (unsigned)sizeof(%s_PARAMS));'
                         % (name[4:], num, name, name))
    lines.append('printf("    0x%x: (%u, %u, \\"INITIALIZE\\"),\\n", (unsigned)UVM_INITIALIZE, '
                 '(unsigned)offsetof(UVM_INITIALIZE_PARAMS, rmStatus), (unsigned)sizeof(UVM_INITIALIZE_PARAMS));')
    lines.append("return 0;}")
    d = tempfile.mkdtemp()
    src = os.path.join(d, "g.c")
    open(src, "w").write("\n".join(lines))
    inc = ["-I" + uvm, "-I" + os.path.join(ogkm, "kernel-open/common/inc"),
           "-I" + os.path.join(ogkm, "src/common/inc")]
    # ⊘ Some *_PARAMS typedefs exist only under `#if defined(WIN32)`; drop what the compiler
    # names undeclared and retry (gen_uvm_sizes.sh's iterated-compile rule).
    for _ in range(10):
        r = subprocess.run(["cc", "-o", os.path.join(d, "g"), src] + inc, env=dict(os.environ, LC_ALL="C"),
                           stderr=subprocess.PIPE)
        if r.returncode == 0:
            break
        bad = set(re.findall(r"'(UVM_[A-Z0-9_]+_PARAMS)'", r.stderr.decode(errors="replace")))
        if not bad:
            sys.stderr.write(r.stderr.decode(errors="replace"))
            raise SystemExit(1)
        body = open(src).read().split("\n")
        open(src, "w").write("\n".join(l for l in body if not any(b in l for b in bad)))
    sys.stdout.write(subprocess.run([os.path.join(d, "g")], stdout=subprocess.PIPE, check=True).stdout.decode())


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    default_ogkm = next((p for p in ("/workspace/nvidia-gpu-passthrough/research_clones/ogkm-580.159.04",
                                     os.path.join(here, "../third_party/ogkm-580"))
                         if os.path.isdir(os.path.join(p, "src"))), None)
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--ogkm", default=default_ogkm)
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("census"); s.add_argument("caps", nargs="+"); s.add_argument("--all", action="store_true")
    s = sub.add_parser("compare"); s.add_argument("--host", nargs="+", required=True)
    s.add_argument("--guest", nargs="+", required=True)
    s = sub.add_parser("matrix"); s.add_argument("dir"); s.add_argument("--json")
    s = sub.add_parser("gen-uvm")
    s = sub.add_parser("selftest")
    s.add_argument("ref", nargs="?", default=next((p for p in (
        os.path.join(here, "../archive/nvkvm/traces/host_reference_ga106"),
        "/workspace/nvidia-gpu-passthrough/traces/host_reference_ga106") if os.path.isdir(p)), None))
    args = ap.parse_args()
    if args.cmd == "gen-uvm":
        gen_uvm(args.ogkm)
        return 0
    if args.cmd == "selftest":
        return selftest(args.ref)
    names = load_names(args.ogkm)
    load_status_names(args.ogkm)
    if args.cmd == "census":
        cmd_census(args, names)
    elif args.cmd == "compare":
        cmd_compare(args, names)
    else:
        cmd_matrix(args, names)
    return 0


if __name__ == "__main__":
    sys.exit(main())
