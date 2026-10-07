#!/usr/bin/env python3
"""rpc_channel_census.py — scan an nvkvm_m2_rec .rec trace for GSP RPC messages the
emulated GSP read out of guest RAM, and census NV_CHANNEL_ALLOC_PARAMS.

Ground truth for every constant, all opened:
  ogkm-580.159.04:
    src/nvidia/generated/g_rpc-message-header.h:41-52   rpc_message_header_v03_00
    src/nvidia/inc/kernel/vgpu/rpc_headers.h:61         NV_VGPU_MSG_SIGNATURE_VALID 0x43505256
    src/nvidia/inc/kernel/vgpu/rpc_global_enums.h:113   GSP_RM_ALLOC = 103
    src/nvidia/generated/g_rpc-structures.h:1491-1502   rpc_gsp_rm_alloc_v03_00
    src/common/sdk/nvidia/inc/alloc/alloc_channel.h:296-342  NV_CHANNEL_ALLOC_PARAMS
    src/nvidia/generated/g_kernel_channel_nvoc.h:181-204     internalFlags bit layout
  kayfabe: crates/kayfabe-abi/src/submit.rs:259-270 (580 offsets),
           crates/kayfabe-abi/src/notifier.rs:182-184 (internalFlags @ +244)
"""
import argparse
import struct
import sys
import json
from collections import Counter, defaultdict

MAGIC = 0x4352434552564B4E
HDR_FMT = "<QIIIIQQQQQQ3Q"
HDR_SIZE = struct.calcsize(HDR_FMT)
ENT_FMT = "<QBBBBIQQ"
ENT_SIZE = struct.calcsize(ENT_FMT)

SIG = 0x43505256          # "VRPC"
FN_GSP_RM_ALLOC = 103
RPC_HDR_LEN = 32          # 7 x NvU32 + union NvU32

# NV_CHANNEL_ALLOC_PARAMS, 580 layout (NV_MAX_SUBDEVICES = 8)
O_hObjectError   = 0
O_hObjectBuffer  = 4
O_gpFifoOffset   = 8
O_gpFifoEntries  = 16
O_flags          = 20
O_hContextShare  = 24
O_hVASpace       = 28
O_hUserdMemory   = 32     # NvHandle[8]
O_userdOffset    = 64     # NvU64[8]
O_engineType     = 128
O_cid            = 132
O_subDeviceId    = 136
O_hObjectEccError= 140
O_instanceMem    = 144    # NV_MEMORY_DESC_PARAMS {NvU64 base; NvU64 size; NvU32 addressSpace; NvU32 cacheAttrib}
O_userdMem       = 168
O_ramfcMem       = 192
O_mthdbufMem     = 216
O_hPhysChannelGroup = 240
O_internalFlags  = 244
O_errorNotifierMem = 248
O_eccErrorNotifierMem = 272
O_ProcessID      = 296
O_SubProcessID   = 300
SZ_580           = 368

PRIV = {0: "USER", 1: "ADMIN", 2: "KERNEL", 3: "RESERVED3"}
# ogkm-580: src/nvidia/inc/kernel/vgpu/rm_plugin_shared_code.h:65-67
APER = {0: "ADDR_UNKNOWN", 1: "NV_ADDR_SYSMEM", 2: "NV_ADDR_FBMEM", 3: "?3"}

CHANNEL_CLASSES = {
    0xA06F: "KEPLER_CHANNEL_GPFIFO_A",
    0xA16F: "KEPLER_CHANNEL_GPFIFO_B",
    0xA26F: "KEPLER_CHANNEL_GPFIFO_C",
    0xB06F: "MAXWELL_CHANNEL_GPFIFO_A",
    0xC06F: "PASCAL_CHANNEL_GPFIFO_A",
    0xC36F: "VOLTA_CHANNEL_GPFIFO_A",
    0xC46F: "TURING_CHANNEL_GPFIFO_A",
    0xC56F: "AMPERE_CHANNEL_GPFIFO_A",
    0xC86F: "HOPPER_CHANNEL_GPFIFO_A",
    0xC96F: "BLACKWELL_CHANNEL_GPFIFO_A",
}


def iter_records(path):
    with open(path, "rb") as fh:
        blob = fh.read()
    (magic, ver, hdrlen, recsize, _r0, props, mask,
     nrec, nbytes, nerr, t0, _a, _b, _c) = struct.unpack_from(HDR_FMT, blob, 0)
    assert magic == MAGIC, "bad magic in %s" % path
    off = hdrlen
    while off + ENT_SIZE <= len(blob):
        e = struct.unpack_from(ENT_FMT, blob, off)
        ln = e[5]
        pstart = off + ENT_SIZE
        pend = pstart + ln
        if pend > len(blob):
            break
        yield e, blob[pstart:pend]
        off = pend + ((8 - (ln & 7)) & 7)


def scan(path, kinds=(3, 4)):
    """Yield (seq, kind, gpa, msg_off, hdr, body) for every RPC signature found."""
    for e, payload in iter_records(path):
        seq, kind, width, bar, _pad, ln, a, b = e
        if kind not in kinds or not payload:
            continue
        # 4-byte-aligned scan for the signature; header_version is the dword before.
        for off in range(4, len(payload) - 4 + 1, 4):
            if struct.unpack_from("<I", payload, off)[0] != SIG:
                continue
            h = off - 4
            if h + RPC_HDR_LEN > len(payload):
                continue
            hv, sig, length, fn, res, resp, sequ, u = struct.unpack_from("<8I", payload, h)
            yield (seq, kind, a, h, dict(header_version=hv, length=length,
                                         function=fn, rpc_result=res,
                                         rpc_result_private=resp, sequence=sequ, u=u),
                   payload[h + RPC_HDR_LEN:])


def desc(body, off):
    if off + 24 > len(body):
        return None
    base, size = struct.unpack_from("<QQ", body, off)
    ap, ca = struct.unpack_from("<II", body, off + 16)
    return dict(base=base, size=size, aperture=ap, cache=ca)


def decode_channel(params):
    d = {}
    n = len(params)

    def u32(o):
        return struct.unpack_from("<I", params, o)[0] if o + 4 <= n else None

    def u64(o):
        return struct.unpack_from("<Q", params, o)[0] if o + 8 <= n else None

    d["hObjectError"] = u32(O_hObjectError)
    d["gpFifoOffset"] = u64(O_gpFifoOffset)
    d["gpFifoEntries"] = u32(O_gpFifoEntries)
    d["flags"] = u32(O_flags)
    d["hContextShare"] = u32(O_hContextShare)
    d["hVASpace"] = u32(O_hVASpace)
    d["hUserdMemory"] = [u32(O_hUserdMemory + 4 * i) for i in range(8)]
    d["userdOffset"] = [u64(O_userdOffset + 8 * i) for i in range(8)]
    d["engineType"] = u32(O_engineType)
    d["cid"] = u32(O_cid)
    d["subDeviceId"] = u32(O_subDeviceId)
    for name, o in (("instanceMem", O_instanceMem), ("userdMem", O_userdMem),
                    ("ramfcMem", O_ramfcMem), ("mthdbufMem", O_mthdbufMem),
                    ("errorNotifierMem", O_errorNotifierMem),
                    ("eccErrorNotifierMem", O_eccErrorNotifierMem)):
        d[name] = desc(params, o)
    d["hPhysChannelGroup"] = u32(O_hPhysChannelGroup)
    inf = u32(O_internalFlags)
    d["internalFlags"] = inf
    if inf is not None:
        d["privilege"] = inf & 0x3
        d["errorNotifierType"] = (inf >> 2) & 0x3
        d["eccErrorNotifierType"] = (inf >> 4) & 0x3
        d["gspOwned"] = (inf >> 6) & 0x1
        d["uvmOwned"] = (inf >> 7) & 0x1
    d["ProcessID"] = u32(O_ProcessID)
    d["SubProcessID"] = u32(O_SubProcessID)
    # flags decode
    f = d["flags"]
    if f is not None:
        d["flag_privileged_5_5"] = (f >> 5) & 1
        d["flag_userd_index_fixed_9_9"] = None
        d["flag_chid_bits_21_8"] = (f >> 8) & 0x3FFF
    return d


def main():
    """Census every channel alloc on the wire, splitting REQUEST from REPLY.

    ⊘⊘⊘ THE ONE TRAP THIS TOOL EXISTS TO PREVENT, measured 2026-08-13 (w286).
    Every `GSP_RM_ALLOC` appears TWICE in a `.rec` capture:
      kind 3 `GuestRead`  — the guest's REQUEST, which the emulated GSP read.  ← the data
      kind 4 `GuestWrite` — OUR OWN REPLY, which the emulator wrote back.      ← all zeros
    All 68 replies in `traces/mode2_c_reference/` carry `internalFlags == 0` and
    `hUserdMemory[0] == 0`.  Censusing the stream without splitting on direction therefore
    reports exactly 50 % `PRIVILEGE=USER` and 50 % `hUserdMemory[0]==0` — two plausible,
    wrong, mutually corroborating numbers produced entirely by counting our own silence.
    The census below therefore ALWAYS prints the two directions separately, and
    always prints the reply column too, so its zeros stay visible rather than
    silently averaged into the answer.
    """
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("files", nargs="+")
    ap.add_argument("--json", help="write per-channel-alloc rows here")
    ap.add_argument("--fn-census", action="store_true")
    args = ap.parse_args()

    rows = []
    for path in args.files:
        fn_counts = Counter()
        class_counts = Counter()
        seen_sig = 0
        for seq, kind, gpa, moff, hdr, body in scan(path):
            seen_sig += 1
            fn_counts[hdr["function"]] += 1
            if hdr["function"] != FN_GSP_RM_ALLOC:
                continue
            if len(body) < 32:
                continue
            hClient, hParent, hObject, hClass, status, paramsSize, aflags = \
                struct.unpack_from("<7I", body, 0)
            class_counts[hClass] += 1
            if (hClass & 0xFFFF) not in CHANNEL_CLASSES:
                continue
            params = body[32:32 + paramsSize] if paramsSize else body[32:]
            row = dict(file=path.split("/")[-1], seq=seq, kind=kind, gpa=gpa,
                       dir="REQUEST" if kind == 3 else "our REPLY",
                       hClient=hClient, hParent=hParent, hObject=hObject,
                       hClass=hClass, className=CHANNEL_CLASSES[hClass & 0xFFFF],
                       status=status, paramsSize=paramsSize, allocFlags=aflags,
                       paramsAvail=len(params), rpcSeq=hdr["sequence"])
            row.update(decode_channel(params))
            rows.append(row)
        print("### %s" % path, file=sys.stderr)
        # ★ Known-positive on the scanner itself: a zero here means the decisive grep ran
        # over nothing, which is the failure this whole rung was briefed to avoid.
        print("    rpc signatures found : %d %s" %
              (seen_sig, "  <<< ZERO — THE SCAN FOUND NOTHING, DO NOT READ ON"
               if seen_sig == 0 else ""), file=sys.stderr)
        if args.fn_census:
            print("    functions: %s" % dict(fn_counts.most_common(25)), file=sys.stderr)
        print("    GSP_RM_ALLOC(103)    : %d" % fn_counts.get(103, 0), file=sys.stderr)
        print("    alloc classes        : %s" % dict(class_counts.most_common(40)),
              file=sys.stderr)

    req = [r for r in rows if r["kind"] == 3]
    rep = [r for r in rows if r["kind"] == 4]
    print("\n=== channel allocs: %d REQUESTS, %d our-own REPLIES ===" % (len(req), len(rep)))
    for label, rs in (("REQUEST (the guest's)", req), ("our REPLY (scrubbed)", rep)):
        if not rs:
            continue
        print("  -- %s, n=%d" % (label, len(rs)))
        print("     privilege internalFlags[1:0] : %s"
              % dict(Counter(PRIV[r["privilege"]] for r in rs)))
        print("     hUserdMemory[0] == 0         : %d / %d"
              % (sum(1 for r in rs if r["hUserdMemory"][0] == 0), len(rs)))
        print("     flags[5:5] PRIVILEGED_CHANNEL: %s"
              % dict(Counter(r["flag_privileged_5_5"] for r in rs)))
        print("     internalFlags UVM_OWNED[7]   : %s"
              % dict(Counter(r["uvmOwned"] for r in rs)))
        print("     engineType                   : %s"
              % dict(Counter(r["engineType"] for r in rs)))
        print("     userdMem.addressSpace        : %s"
              % dict(Counter(APER.get(r["userdMem"]["aperture"], "?") for r in rs)))
    if args.json:
        with open(args.json, "w") as fh:
            for r in rows:
                fh.write(json.dumps(r) + "\n")


if __name__ == "__main__":
    main()
