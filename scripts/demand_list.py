#!/usr/bin/env python3
"""demand_list.py — extract the guest driver's GSP **demand list** from a §6 replay trace.

Task #179, the `replay-conformance` line.

## What "demand" means here

The command queue is CPU-RM → GSP.  In a §6 trace (`nvkvm_m2_rec.h`) the emulated device
reads those elements out of guest RAM through the `nvkvm_dmar` chokepoint, so **every
command element the driver posted appears as a `GuestRead` record whose length is exactly
one queue element** (`GSP_MSG_QUEUE_ELEMENT_SIZE_MIN == RM_PAGE_SIZE == 4096`,
`ogkm-580: src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:88-90`).  The reply
direction (`GuestWrite`) is what the *emulator answered*, which is a fact about the C, not
about the driver — this tool ignores it.

## Layout, cited

`ogkm-580: src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:42-51`

    GSP_MSG_QUEUE_ELEMENT { u8 authTag[16]; u8 aad[16]; u32 checkSum; u32 seqNum;
                            u32 elemCount; rpc_message_header_v rpc __align(8); }

  ⇒ 16+16+4+4+4 = 44, `rpc` is 8-aligned ⇒ **hdr size 48**.

`ogkm-580: src/nvidia/generated/g_rpc-message-header.h:41-52`

    rpc_message_header_v { u32 header_version, signature, length, function,
                           rpc_result, rpc_result_private, sequence; union u; }  // 32 bytes

  ⇒ `function` at 48+12 = **60**; `length` at 56; `rpc_result` at 64; `sequence` at 72.
    Body starts at 48+32 = **80**.

`ogkm-580: src/nvidia/generated/g_rpc-structures.h:1506-1520`

    rpc_gsp_rm_control_v { NvHandle hClient, hObject; u32 cmd, status, paramsSize,
                           rmapiRpcFlags, rmctrlFlags, rmctrlAccessRight;
                           u64 reserved0; u8 params[]; }

  ⇒ for `function == 76` (`GSP_RM_CONTROL`, `ogkm-580:
    src/nvidia/inc/kernel/vgpu/rpc_global_enums.h:86`) the control id `cmd` is at
    80+8 = **88** and `paramsSize` at **96**.

`ogkm-580: src/nvidia/generated/g_rpc-structures.h:1491-1502`

    rpc_gsp_rm_alloc_v { NvHandle hClient, hParent, hObject; u32 hClass, status,
                         paramsSize, flags; u8 reserved[4]; u8 params[]; }

  ⇒ for `function == 103` (`GSP_RM_ALLOC`) the class id `hClass` is at 80+12 = **92**.

## ⚠ What this tool does NOT tell you

`cap1` is a trace of a boot that **fails** — it ends where the C emulator stopped.  The
list this prints is a **lower bound** on the demand of a successful boot.  See
`docs/reference/gsp_demand_list_cap1.md`.

Usage:
    scripts/demand_list.py traces/cap1_coldboot_hermetic.rec [--tsv OUT.tsv] [--json OUT.json]
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import struct
import sys
from collections import Counter, OrderedDict
from pathlib import Path

# ── §6 trace container (mirrors scripts/mode2_diag/rec_dump.py in the C repo) ──
MAGIC = 0x4352434552564B4E
HDR_FMT = "<QIIIIQQQQQQ3Q"
HDR_SIZE = struct.calcsize(HDR_FMT)
ENT_FMT = "<QBBBBIQQ"
ENT_SIZE = struct.calcsize(ENT_FMT)
assert HDR_SIZE == 96 and ENT_SIZE == 32

KIND_GUEST_RD = 3

# ── GSP_MSG_QUEUE_ELEMENT / rpc_message_header_v offsets (cited above) ──
ELEM_SIZE = 4096
OFF_SEQNUM = 36
OFF_ELEMCOUNT = 40
OFF_RPC = 48
OFF_RPC_LENGTH = OFF_RPC + 8
OFF_RPC_FUNCTION = OFF_RPC + 12
OFF_RPC_RESULT = OFF_RPC + 16
OFF_RPC_SEQUENCE = OFF_RPC + 24
OFF_BODY = OFF_RPC + 32
RPC_HDR_SIZE = 32          # sizeof(rpc_message_header_v)

FN_GSP_RM_CONTROL = 76
FN_GSP_RM_ALLOC = 103

OFF_CTRL_HCLIENT = OFF_BODY + 0
OFF_CTRL_HOBJECT = OFF_BODY + 4
OFF_CTRL_CMD = OFF_BODY + 8
OFF_CTRL_PARAMSIZE = OFF_BODY + 16
OFF_ALLOC_HCLASS = OFF_BODY + 12
OFF_ALLOC_PARAMSIZE = OFF_BODY + 20

DEFAULT_OGKM = Path("/workspace/nvidia-gpu-passthrough/research_clones/ogkm-580.159.04")
ENUM_H = "src/nvidia/inc/kernel/vgpu/rpc_global_enums.h"
CLASS_H = "src/nvidia/generated/g_allclasses.h"
X_RE = re.compile(r"^\s*X\(\s*(\w+)\s*,\s*(\w+)\s*,\s*(\d+)\s*\)")
CLASS_RE = re.compile(r"^#define\s+(\w+)\s+\((0x[0-9a-fA-F]{8})\)\s*$")


def rpc_function_names(ogkm: Path) -> dict[int, str]:
    """`NV_VGPU_MSG_FUNCTION_*`, read from the driver rather than transcribed."""
    names: dict[int, str] = {}
    p = ogkm / ENUM_H
    if not p.exists():
        return names
    for line in p.read_text(errors="replace").splitlines():
        m = X_RE.match(line)
        if m:
            names[int(m.group(3))] = m.group(2)
    return names


def class_names(ogkm: Path) -> dict[int, str]:
    """Class ids from the generated canonical list, first (non-`// alias`) name wins."""
    names: dict[int, str] = {}
    p = ogkm / CLASS_H
    if not p.exists():
        return names
    for line in p.read_text(errors="replace").splitlines():
        m = CLASS_RE.match(line.rstrip())
        if m:
            names.setdefault(int(m.group(2), 16), m.group(1))
    return names


def control_names(ogkm: Path) -> dict[int, str]:
    """Control macro names, via the shared `rm_ctrl_index` evidence tool."""
    try:
        sys.path.insert(0, str(Path(__file__).resolve().parent))
        from rm_ctrl_index import Index          # noqa: PLC0415
    except Exception:
        return {}
    try:
        idx = Index(ogkm)
        return {cid: rows[0][0] for cid, rows in idx.defs.items() if rows}
    except Exception:
        return {}


def read_records(blob: bytes):
    (magic, ver, hdrlen, recsize, _r0, props, mask,
     nrec, nbytes, nerr, t0, _a, _b, _c) = struct.unpack_from(HDR_FMT, blob, 0)
    if magic != MAGIC:
        sys.exit("FATAL: bad magic 0x%016x" % magic)
    prov = blob[HDR_SIZE:hdrlen].split(b"\0", 1)[0].decode("utf-8", "replace")
    meta = dict(version=ver, props=props, mask=mask, n_records=nrec,
                n_errors=nerr, provenance=prov)
    off = hdrlen
    idx = 0
    expected = 0
    dense = True
    while off + ENT_SIZE <= len(blob):
        seq, kind, width, bar, _pad, ln, a, b = struct.unpack_from(ENT_FMT, blob, off)
        pstart = off + ENT_SIZE
        pend = pstart + ln
        if pend > len(blob):
            break
        payload = blob[pstart:pend]
        off = pend + ((8 - (ln & 7)) & 7)
        if seq != expected:
            dense = False
        expected = seq + 1
        yield idx, seq, kind, a, payload
        idx += 1
    meta["scanned"] = idx
    meta["dense"] = dense
    read_records.meta = meta          # type: ignore[attr-defined]


def u32(buf: bytes, off: int) -> int:
    return struct.unpack_from("<I", buf, off)[0]


def extract(path: Path, ogkm: Path):
    blob = path.read_bytes()
    fn_names = rpc_function_names(ogkm)
    elements = []
    continuations = 0
    meta = {}
    gen = read_records(blob)
    for idx, seq, kind, gpa, payload in gen:
        if kind != KIND_GUEST_RD or len(payload) != ELEM_SIZE:
            continue
        # ★ A CONTINUATION SLOT IS NOT A DEMAND, and it decodes as a plausible-looking
        # `NOP`.  A message spanning `elemCount > 1` slots carries the
        # `rpc_message_header_v` in slot 0 ONLY; the rest are raw payload.  The driver's
        # own validity test is `entryLength >= sizeof(rpc_message_header_v)`
        # (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:2192`), and the length it writes is
        # `sizeof(rpc_message_header_v) + paramLength`
        # (`ogkm-580: src/nvidia/src/kernel/rmapi/rpc_common.c:183`), so a header is valid
        # iff `rpc.length >= 32`.  Applying that test rather than counting slots is what
        # keeps `cap1` right: the C **skips** its continuation reads (GSP-D6), so a
        # slot-counting rule would swallow real later commands there, while `cap1b`
        # witnesses all 32 of them and they would otherwise enter the list as `NOP`.
        if u32(payload, OFF_RPC_LENGTH) < RPC_HDR_SIZE:
            continuations += 1
            continue
        e = {
            "record_index": idx,
            "seq": seq,
            "gpa": gpa,
            "elem_seqnum": u32(payload, OFF_SEQNUM),
            "elem_count": u32(payload, OFF_ELEMCOUNT),
            "rpc_length": u32(payload, OFF_RPC_LENGTH),
            "function": u32(payload, OFF_RPC_FUNCTION),
            "rpc_result": u32(payload, OFF_RPC_RESULT),
            "rpc_sequence": u32(payload, OFF_RPC_SEQUENCE),
        }
        e["function_name"] = fn_names.get(e["function"], "UNKNOWN_FN_%d" % e["function"])
        if e["function"] == FN_GSP_RM_CONTROL:
            e["cmd"] = u32(payload, OFF_CTRL_CMD)
            e["params_size"] = u32(payload, OFF_CTRL_PARAMSIZE)
            e["hclient"] = u32(payload, OFF_CTRL_HCLIENT)
            e["hobject"] = u32(payload, OFF_CTRL_HOBJECT)
        elif e["function"] == FN_GSP_RM_ALLOC:
            e["hclass"] = u32(payload, OFF_ALLOC_HCLASS)
            e["params_size"] = u32(payload, OFF_ALLOC_PARAMSIZE)
        elements.append(e)
    meta = getattr(read_records, "meta", {})
    meta["continuation_slots_skipped"] = continuations
    meta["file"] = str(path)
    meta["sha256"] = hashlib.sha256(blob).hexdigest()
    meta["ogkm"] = str(ogkm)
    return meta, elements


def summarise(elements, cnames=None, klass=None):
    """Ordered demand: (kind, id) → count + first occurrence."""
    order: "OrderedDict[tuple, dict]" = OrderedDict()
    for ordinal, e in enumerate(elements):
        if e["function"] == FN_GSP_RM_CONTROL:
            key = ("control", e["cmd"])
            label = "0x%08x" % e["cmd"]
        elif e["function"] == FN_GSP_RM_ALLOC:
            key = ("alloc", e["hclass"])
            label = "0x%08x" % e["hclass"]
        else:
            key = ("rpc", e["function"])
            label = "%d" % e["function"]
        row = order.get(key)
        if row is None:
            order[key] = {
                "kind": key[0], "id": key[1], "label": label,
                "rpc_function": e["function"], "rpc_function_name": e["function_name"],
                "count": 1,
                "first_record_index": e["record_index"],
                "first_element_ordinal": ordinal,
                "params_sizes": Counter([e.get("params_size", 0)]),
            }
        else:
            row["count"] += 1
            row["params_sizes"][e.get("params_size", 0)] += 1
    cnames = cnames or {}
    klass = klass or {}
    for i, (k, row) in enumerate(order.items()):
        row["demand_rank"] = i
        row["params_sizes"] = dict(sorted(row["params_sizes"].items()))
        if row["kind"] == "control":
            row["name"] = cnames.get(row["id"], "UNRESOLVED")
        elif row["kind"] == "alloc":
            row["name"] = klass.get(row["id"], "UNRESOLVED")
        else:
            row["name"] = row["rpc_function_name"]
    return order


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("file")
    ap.add_argument("--ogkm", default=str(DEFAULT_OGKM))
    ap.add_argument("--tsv")
    ap.add_argument("--json")
    ap.add_argument("--sequence", action="store_true",
                    help="print the full ordered element sequence")
    args = ap.parse_args()

    ogkm = Path(args.ogkm)
    meta, elements = extract(Path(args.file), ogkm)
    order = summarise(elements, control_names(ogkm), class_names(ogkm))

    print("trace        : %s" % meta["file"])
    print("sha256       : %s" % meta["sha256"])
    print("records      : %d   dense=%s   n_errors=%d"
          % (meta["scanned"], meta["dense"], meta["n_errors"]))
    print("command elems: %d   (+%d continuation slots skipped)"
          % (len(elements), meta["continuation_slots_skipped"]))
    print("distinct     : %d  (%d RPC fns other than 76/103, %d controls, %d alloc classes)"
          % (len(order),
             sum(1 for k in order if k[0] == "rpc"),
             sum(1 for k in order if k[0] == "control"),
             sum(1 for k in order if k[0] == "alloc")))
    print("-" * 78)
    print("%-4s %-8s %-12s %-62s %6s %10s" %
          ("#", "kind", "id", "name", "count", "first_rec"))
    for row in order.values():
        print("%-4d %-8s %-12s %-62s %6d %10d" %
              (row["demand_rank"], row["kind"], row["label"], row["name"],
               row["count"], row["first_record_index"]))

    if args.sequence:
        print("-" * 78)
        for i, e in enumerate(elements):
            extra = ""
            if e["function"] == FN_GSP_RM_CONTROL:
                extra = " cmd=0x%08x psize=%d" % (e["cmd"], e["params_size"])
            elif e["function"] == FN_GSP_RM_ALLOC:
                extra = " hClass=0x%08x psize=%d" % (e["hclass"], e["params_size"])
            print("%4d rec=%-8d seqNum=%-5d elemCount=%d fn=%-3d %-28s rpclen=%-6d%s"
                  % (i, e["record_index"], e["elem_seqnum"], e["elem_count"],
                     e["function"], e["function_name"], e["rpc_length"], extra))

    if args.tsv:
        with open(args.tsv, "w") as fh:
            fh.write("# generated by scripts/demand_list.py from %s\n" % meta["file"])
            fh.write("# trace sha256 %s\n" % meta["sha256"])
            fh.write("# ⚠ cap1 is a trace of a boot that FAILS: this is a LOWER BOUND.\n")
            fh.write("demand_rank\tkind\tid\tname\trpc_function\trpc_function_name\t"
                     "count\tfirst_record_index\tfirst_element_ordinal\tparams_sizes\n")
            for row in order.values():
                fh.write("%d\t%s\t%s\t%s\t%d\t%s\t%d\t%d\t%d\t%s\n" % (
                    row["demand_rank"], row["kind"], row["label"],
                    row["name"], row["rpc_function"], row["rpc_function_name"],
                    row["count"], row["first_record_index"], row["first_element_ordinal"],
                    ",".join("%d:%d" % kv for kv in row["params_sizes"].items())))
        print("wrote %s" % args.tsv)

    if args.json:
        out = {
            "meta": meta,
            "warning": ("cap1 is a trace of a boot that FAILS — it ends where the C "
                        "emulator stopped. This demand list is a LOWER BOUND on what a "
                        "successful boot demands, not the ladder."),
            "demand": [dict(row, id="0x%08x" % row["id"]) for row in order.values()],
            "sequence": elements,
        }
        Path(args.json).write_text(json.dumps(out, indent=1) + "\n")
        print("wrote %s" % args.json)


if __name__ == "__main__":
    main()
