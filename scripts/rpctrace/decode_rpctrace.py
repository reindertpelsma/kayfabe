#!/usr/bin/env python3
"""decode_rpctrace.py — read a binary GSP-RM RPC trace, or REFUSE it.

Task #178, `replay-conformance`. Spec: docs/design/rpc_trace_capture.md.
Format: scripts/rpctrace/nv_rpctrace.h.

★★★ THIS IS A REFUSER FIRST AND A PRETTY-PRINTER SECOND.

The reason the recorder exists is that the project's previous hardware oracle —
the C artifact's `mode2_initctrl_ga106.h` — has 11 rows out of 56 that carry a
length with no body, and every one checked against a real GA106 is contradicted.
`0x20802a08` decodes from its empty row as size 0 where hardware answers 20480.
Nothing about that row *looked* wrong. It was cited as corroboration by a gate
that demanded a citation, and the citation was satisfied.

So the decoder's job is to make the analogous failures here IMPOSSIBLE TO READ
PAST. A trace whose ring overflowed has a hole, and a hole shifts every later
index in a positional diff, so it is refused outright rather than reported with a
warning somebody scrolls past. A file whose length disagrees with its own header
is refused. A record whose magic is wrong is refused. ⊘ There is no `--force`.

  exit 0 — the trace is complete and internally consistent
  exit 2 — REFUSED, with the reason on stderr
  exit 3 — usage / IO error

USAGE
  decode_rpctrace.py TRACE.bin                        # verify + summary
  decode_rpctrace.py TRACE.bin --list                 # every record, one line each
  decode_rpctrace.py TRACE.bin --list --dir req       # requests only
  decode_rpctrace.py TRACE.bin --hexdump 1234         # one record's bytes
  decode_rpctrace.py TRACE.bin --json out.json        # machine-readable summary
  decode_rpctrace.py TRACE.bin --names rpc_function_names.tsv
  decode_rpctrace.py --extract-names OGKM_DIR > rpc_function_names.tsv
"""

import argparse
import collections
import json
import os
import re
import struct
import signal
import sys

# Keep `| head` from producing a BrokenPipeError traceback after the useful output.
try:
    signal.signal(signal.SIGPIPE, signal.SIG_DFL)
except (AttributeError, ValueError):
    pass

FILE_MAGIC = 0x5452564E   # "NVRT"
REC_MAGIC = 0x52435052    # "RPCR"
VERSION = 1

FILE_HDR_FMT = "<4I9Q2I32s"
FILE_HDR_SIZE = struct.calcsize(FILE_HDR_FMT)      # 128
REC_HDR_FMT = "<I2H2IQ6I"
REC_HDR_SIZE = struct.calcsize(REC_HDR_FMT)        # 48

FF_OVERFLOWED = 0x0001
FF_DISABLED = 0x0002

F_CC_ENABLED = 0x0001
F_LEN_DISAGREE = 0x0002
F_NOT_SENT = 0x0004

DIR_REQ = 0
DIR_REP = 1
DIR_NAME = {DIR_REQ: "CPU->GSP", DIR_REP: "GSP->CPU"}

# The GSP_MSG_QUEUE_ELEMENT header: authTag[16] + aad[16] + checkSum + seqNum +
# elemCount, then `rpc` aligned to 8. Repeated here, with the derivation, rather
# than imported — this script must run without an ogkm checkout.
ELEM_HDR_SIZE = 48


class Refused(Exception):
    pass


def refuse(msg):
    raise Refused(msg)


# --------------------------------------------------------------------------- #
# names
# --------------------------------------------------------------------------- #

def extract_names(ogkm_dir):
    """Derive number->name from ogkm's X-macro enum. Names only: no NVIDIA code
    is copied, and the map is regenerable from any checkout."""
    path = os.path.join(ogkm_dir, "src/nvidia/inc/kernel/vgpu/rpc_global_enums.h")
    if not os.path.exists(path):
        sys.exit("no rpc_global_enums.h under %s" % ogkm_dir)
    names = {}
    with open(path) as fh:
        for line in fh:
            m = re.match(r"\s*X\((RM|GSP),\s*([A-Z0-9_]+),\s*(\d+)\)", line)
            if m:
                names[int(m.group(3))] = m.group(2)
                continue
            m = re.match(r"\s*E\(([A-Z0-9_]+),\s*(0x[0-9a-fA-F]+)\)", line)
            if m:
                names[int(m.group(2), 16)] = "EVENT_" + m.group(1)
    return names


def load_names(path):
    names = {}
    with open(path) as fh:
        for line in fh:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            num, name = line.split("\t", 1)
            names[int(num, 0)] = name
    return names


def fn_name(names, fn):
    return names.get(fn, "fn_%u" % fn)


# --------------------------------------------------------------------------- #
# parse + verify
# --------------------------------------------------------------------------- #

def parse_file_hdr(blob):
    if len(blob) < FILE_HDR_SIZE:
        refuse("file is %d bytes, shorter than the %d-byte header — TRUNCATED"
               % (len(blob), FILE_HDR_SIZE))
    f = struct.unpack_from(FILE_HDR_FMT, blob, 0)
    h = dict(zip(
        ("magic", "version", "file_hdr_size", "rec_hdr_size",
         "capacity", "used", "n_records", "n_payload_bytes",
         "n_dropped", "n_dropped_bytes", "n_refused_empty", "n_rx_failed",
         "t0_ns", "flags", "reserved0", "drv_version"), f))
    if h["magic"] != FILE_MAGIC:
        refuse("bad file magic 0x%08x (expected 0x%08x) — not an rpctrace file"
               % (h["magic"], FILE_MAGIC))
    if h["version"] != VERSION:
        refuse("file version %d, this decoder speaks %d" % (h["version"], VERSION))
    if h["file_hdr_size"] != FILE_HDR_SIZE:
        refuse("file header size %d != %d — the recorder and this decoder disagree "
               "about the format" % (h["file_hdr_size"], FILE_HDR_SIZE))
    if h["rec_hdr_size"] != REC_HDR_SIZE:
        refuse("record header size %d != %d — format mismatch"
               % (h["rec_hdr_size"], REC_HDR_SIZE))
    h["drv_version"] = h["drv_version"].split(b"\0", 1)[0].decode("ascii", "replace")
    return h


def verify_and_parse(blob, allow_rx_errors=False, expect_version=None):
    h = parse_file_hdr(blob)

    if h["flags"] & FF_DISABLED:
        refuse("the recorder was never armed (NVreg_RpcTraceKB=0) — this file "
               "records nothing, which is NOT the same as recording that nothing "
               "happened")

    # ★★★ THE WRAP REFUSAL. Fill-and-stop means a full ring keeps its ordered
    # PREFIX and refuses the rest; nothing is overwritten. That still makes the
    # trace unusable for a positional diff, because every element after the first
    # drop is missing. Report both numbers and stop.
    if h["n_dropped"] or (h["flags"] & FF_OVERFLOWED):
        refuse("RING OVERFLOWED: %d records (%d bytes) were dropped after the ring "
               "filled at %d bytes. The trace is a PREFIX, not a capture. Re-run "
               "with a larger NVreg_RpcTraceKB. ⊘ A trace with a hole cannot be "
               "replayed positionally — one drop shifts every later index."
               % (h["n_dropped"], h["n_dropped_bytes"], h["capacity"]))

    expected_len = FILE_HDR_SIZE + h["used"]
    if len(blob) < expected_len:
        refuse("TRUNCATED: header declares %d bytes of records, file carries only "
               "%d. Missing %d bytes."
               % (h["used"], len(blob) - FILE_HDR_SIZE, expected_len - len(blob)))
    if len(blob) > expected_len:
        refuse("TRAILING GARBAGE: header declares %d bytes of records, file carries "
               "%d extra bytes beyond them."
               % (h["used"], len(blob) - expected_len))

    if expect_version and h["drv_version"] != expect_version:
        refuse("driver version is %s, expected %s"
               % (h["drv_version"], expect_version))

    if h["n_refused_empty"]:
        # Not fatal — it is the recorder REFUSING to write an empty row, which is
        # the behaviour we want — but it must be visible, because it means some
        # element was not captured.
        print("⚠ the recorder refused %d call(s) that had no bytes to record. "
              "No empty rows were written (that is the point), but those elements "
              "are ABSENT from this trace." % h["n_refused_empty"], file=sys.stderr)

    if h["n_rx_failed"] and not allow_rx_errors:
        refuse("%d GSP->CPU read(s) FAILED after retries. Those are holes in the "
               "reply stream with no bytes behind them. Pass --allow-rx-errors if "
               "you mean to read the trace anyway." % h["n_rx_failed"])

    recs = []
    off = FILE_HDR_SIZE
    end = FILE_HDR_SIZE + h["used"]
    idx = 0
    while off < end:
        if off + REC_HDR_SIZE > end:
            refuse("record %d at offset %d: %d bytes left, need %d for a header — "
                   "TRUNCATED mid-record" % (idx, off, end - off, REC_HDR_SIZE))
        (magic, direction, flags, seq, elem_seq, ts_ns,
         rpc_fn, rpc_len, rpc_status, outcome, cap_len,
         _r) = struct.unpack_from(REC_HDR_FMT, blob, off)
        if magic != REC_MAGIC:
            refuse("record %d at offset %d: bad magic 0x%08x (expected 0x%08x) — "
                   "the stream is not aligned; everything after this point is "
                   "unreadable" % (idx, off, magic, REC_MAGIC))
        if seq != idx:
            refuse("record %d at offset %d carries seq %d — the recorder's own "
                   "counter is not consecutive, so a record is MISSING"
                   % (idx, off, seq))
        # ⊘ THE ROW THAT MUST NOT EXIST.
        if cap_len == 0:
            refuse("record %d at offset %d has cap_len 0 — a length with no bytes. "
                   "This is structurally impossible in the recorder; a file "
                   "containing one is corrupt." % (idx, off))
        padded = (cap_len + 7) & ~7
        if off + REC_HDR_SIZE + padded > end:
            refuse("record %d at offset %d declares %d payload bytes but only %d "
                   "remain — TRUNCATED mid-record"
                   % (idx, off, cap_len, end - off - REC_HDR_SIZE))
        body = blob[off + REC_HDR_SIZE: off + REC_HDR_SIZE + cap_len]
        recs.append(dict(seq=seq, dir=direction, flags=flags, elem_seq=elem_seq,
                         ts_ns=ts_ns, rpc_fn=rpc_fn, rpc_len=rpc_len,
                         rpc_status=rpc_status, outcome=outcome,
                         cap_len=cap_len, off=off, body=body))
        off += REC_HDR_SIZE + padded
        idx += 1

    if len(recs) != h["n_records"]:
        refuse("header says %d records, the stream contains %d"
               % (h["n_records"], len(recs)))
    total_payload = sum(r["cap_len"] for r in recs)
    if total_payload != h["n_payload_bytes"]:
        refuse("header says %d payload bytes, the records sum to %d"
               % (h["n_payload_bytes"], total_payload))

    return h, recs


# --------------------------------------------------------------------------- #
# reporting
# --------------------------------------------------------------------------- #

def rec_line(r, names):
    tags = []
    if r["flags"] & F_CC_ENABLED:
        tags.append("CC")
    if r["flags"] & F_LEN_DISAGREE:
        tags.append("LENDIFF")
    if r["flags"] & F_NOT_SENT:
        tags.append("NOT_SENT")
    if r["outcome"]:
        tags.append("outcome=0x%x" % r["outcome"])
    if r["rpc_status"]:
        tags.append("status=0x%x" % r["rpc_status"])
    return "%7d  %-9s eseq=%-6d %+12.6fms  fn=0x%-4x %-38s cap=%-6d rpc_len=%-6d %s" % (
        r["seq"], DIR_NAME.get(r["dir"], "dir%d" % r["dir"]), r["elem_seq"],
        r["ts_ns"] / 1e6, r["rpc_fn"], fn_name(names, r["rpc_fn"]),
        r["cap_len"], r["rpc_len"], " ".join(tags))


def split_sessions(recs):
    """★ A TRACE IS NOT ONE BOOT, and the first read of this capture said it was.

    MEASURED 2026-08-03 on `rpctrace_ga106_boot1`: every per-function count came
    out EVEN and 479 records shared an `elem_seq` with another. Read naively that
    is 479 retransmits, which would be alarming and would be wrong. What actually
    happened is that with persistence mode off, RM tears the GPU down when the
    last client closes — so each `nvidia-smi` invocation is a COMPLETE GSP
    bring-up and shutdown, the message queue is recreated, and `seqNum` restarts
    at 0.

    ⇒ sessions are cut where a direction's `elem_seq` goes BACKWARDS. Duplicates
    are then counted within a session, where they would genuinely mean a
    retransmit. The alternative — reporting a number that is technically true and
    reliably misread — is how a trace becomes a wrong answer.
    """
    sessions = []
    cur = []
    last = {}
    for r in recs:
        if r["dir"] in last and r["elem_seq"] < last[r["dir"]]:
            sessions.append(cur)
            cur = []
            last = {}
        last[r["dir"]] = r["elem_seq"]
        cur.append(r)
    if cur:
        sessions.append(cur)
    return sessions


def summarize(h, recs, names):
    out = {}
    out["driver_version"] = h["drv_version"]
    out["ring_capacity"] = h["capacity"]
    out["bytes_used"] = h["used"]
    out["file_bytes"] = FILE_HDR_SIZE + h["used"]
    out["n_records"] = h["n_records"]
    out["n_payload_bytes"] = h["n_payload_bytes"]
    out["n_dropped"] = h["n_dropped"]
    out["n_refused_empty"] = h["n_refused_empty"]
    out["n_rx_failed"] = h["n_rx_failed"]
    out["wrapped"] = bool(h["n_dropped"] or (h["flags"] & FF_OVERFLOWED))

    reqs = [r for r in recs if r["dir"] == DIR_REQ]
    reps = [r for r in recs if r["dir"] == DIR_REP]
    out["n_requests"] = len(reqs)
    out["n_replies"] = len(reps)

    if recs:
        biggest = max(recs, key=lambda r: r["cap_len"])
        out["largest_element"] = dict(
            seq=biggest["seq"], bytes=biggest["cap_len"],
            dir=DIR_NAME.get(biggest["dir"]), rpc_fn=biggest["rpc_fn"],
            name=fn_name(names, biggest["rpc_fn"]))
        out["span_ms"] = (recs[-1]["ts_ns"] - recs[0]["ts_ns"]) / 1e6
        out["min_element"] = min(r["cap_len"] for r in recs)
        out["mean_element"] = out["n_payload_bytes"] / len(recs)

    out["n_not_sent"] = sum(1 for r in recs if r["flags"] & F_NOT_SENT)
    out["n_len_disagree"] = sum(1 for r in recs if r["flags"] & F_LEN_DISAGREE)
    out["n_cc"] = sum(1 for r in recs if r["flags"] & F_CC_ENABLED)

    # ★ Split by direction. A REQUEST carries rpc_result = NV_ERR_GENERIC
    # (0xffffffff) as the sentinel RM writes in rpcWriteCommonHeader before GSP
    # has answered — it is not an error and counting it with the replies makes
    # "every request failed" out of a completely healthy boot. The number that
    # means something is the reply one.
    out["n_request_sentinel_status"] = sum(
        1 for r in recs if r["dir"] == DIR_REQ and r["rpc_status"] == 0xFFFFFFFF)
    out["n_reply_error_status"] = sum(
        1 for r in recs if r["dir"] == DIR_REP and r["rpc_status"])
    out["reply_error_functions"] = sorted({
        "0x%x %s" % (r["rpc_fn"], fn_name(names, r["rpc_fn"]))
        for r in recs if r["dir"] == DIR_REP and r["rpc_status"]})

    sessions = split_sessions(recs)
    out["n_sessions"] = len(sessions)
    out["sessions"] = []
    dup_total = 0
    for i, sess in enumerate(sessions):
        dup = collections.Counter((r["dir"], r["elem_seq"]) for r in sess)
        d = sum(v - 1 for v in dup.values() if v > 1)
        dup_total += d
        out["sessions"].append(dict(
            index=i,
            first_seq=sess[0]["seq"], last_seq=sess[-1]["seq"],
            n_records=len(sess),
            n_requests=sum(1 for r in sess if r["dir"] == DIR_REQ),
            n_replies=sum(1 for r in sess if r["dir"] == DIR_REP),
            duplicate_elem_seq=d,
            span_ms=(sess[-1]["ts_ns"] - sess[0]["ts_ns"]) / 1e6,
            distinct_functions=len({r["rpc_fn"] for r in sess}),
        ))
    out["n_duplicate_elem_seq_within_session"] = dup_total

    by_fn = collections.Counter()
    for r in recs:
        by_fn[r["rpc_fn"]] += 1
    out["distinct_functions"] = len(by_fn)
    out["top_functions"] = [
        dict(rpc_fn=fn, name=fn_name(names, fn), count=n)
        for fn, n in by_fn.most_common(25)]

    # Every function seen, so a reader can diff the DEMAND LIST against what our
    # port serves without re-parsing the trace.
    out["functions_seen"] = sorted(
        [dict(rpc_fn=fn, name=fn_name(names, fn), count=by_fn[fn]) for fn in by_fn],
        key=lambda d: d["rpc_fn"])
    return out


def print_summary(s):
    print("driver version      : %s" % s["driver_version"])
    print("ring capacity       : %d bytes" % s["ring_capacity"])
    print("file size           : %d bytes (%.2f MiB)"
          % (s["file_bytes"], s["file_bytes"] / 1048576.0))
    print("records             : %d  (%d requests / %d replies)"
          % (s["n_records"], s["n_requests"], s["n_replies"]))
    print("payload bytes       : %d" % s["n_payload_bytes"])
    print("wrapped / dropped   : %s / %d" % (s["wrapped"], s["n_dropped"]))
    print("refused-empty       : %d" % s["n_refused_empty"])
    print("rx read failures    : %d" % s["n_rx_failed"])
    if "largest_element" in s:
        le = s["largest_element"]
        print("largest element     : %d bytes  (seq %d, %s, fn 0x%x %s)"
              % (le["bytes"], le["seq"], le["dir"], le["rpc_fn"], le["name"]))
        print("smallest / mean     : %d / %.1f bytes"
              % (s["min_element"], s["mean_element"]))
        print("span                : %.1f ms" % s["span_ms"])
    print("NOT_SENT records    : %d" % s["n_not_sent"])
    print("len-disagreements   : %d" % s["n_len_disagree"])
    print("CC-enabled records  : %d" % s["n_cc"])
    print("request sentinel    : %d  (rpc_result=0xffffffff before GSP answers — not errors)"
          % s["n_request_sentinel_status"])
    print("REPLY error status  : %d %s"
          % (s["n_reply_error_status"], s["reply_error_functions"] or ""))
    print("distinct functions  : %d" % s["distinct_functions"])
    print()
    print("sessions (a session = one full GSP bring-up; elem_seq restarts at 0):")
    for e in s["sessions"]:
        print("  #%d  records %d..%d  (%d recs: %d req / %d rep)  %.1f ms  "
              "%d fns  dup_elem_seq=%d"
              % (e["index"], e["first_seq"], e["last_seq"], e["n_records"],
                 e["n_requests"], e["n_replies"], e["span_ms"],
                 e["distinct_functions"], e["duplicate_elem_seq"]))
    print("  ⇒ retransmits (duplicate elem_seq WITHIN a session): %d"
          % s["n_duplicate_elem_seq_within_session"])
    print()
    print("top functions by element count:")
    for e in s["top_functions"]:
        print("  0x%-4x %-42s %6d" % (e["rpc_fn"], e["name"], e["count"]))


# --------------------------------------------------------------------------- #
# GSP_RM_CONTROL
# --------------------------------------------------------------------------- #

# rpc_message_header_v: header_version, signature, length, function, rpc_result,
# rpc_result_private, sequence, u — eight NvU32 — then the payload.
RPC_HDR_SIZE = 32
# rpc_gsp_rm_control_v03_00: hClient, hObject, cmd, status, paramsSize,
# rmapiRpcFlags, rmctrlFlags, rmctrlAccessRight, reserved0(u64), params[].
CTRL_FMT = "<8IQ"
CTRL_HDR_SIZE = struct.calcsize(CTRL_FMT)   # 40


def decode_controls(recs, names, gsp_rm_control_fn=0x4C):
    """★ THE ROWS THAT REPLACE `mode2_initctrl_ga106.h`.

    That table has 56 rows and 11 of them carry `dlen = 0` — a control command
    with no reply body — and every one checked against real hardware is
    contradicted. Here a control's reply body is `cap_len - 48 - 32 - 40` bytes
    that came out of the element the driver actually received, so there is no
    representation for "we have the command but not the answer": either the
    record exists with its bytes, or it does not exist.

    ⊘ What this still does NOT tell you is whether a given control is DATA (the
    reply is the whole answer) or an ACT (the reply acknowledges something that
    must actually have happened). `0x20800a6c` and `0xa06f0103` are known acts and
    they look exactly like data here. Classifying them is a separate static pass
    over ogkm — see docs/design/rpc_trace_capture.md §4.
    """
    out = []
    for r in recs:
        if r["rpc_fn"] != gsp_rm_control_fn:
            continue
        off = ELEM_HDR_SIZE + RPC_HDR_SIZE
        if len(r["body"]) < off + CTRL_HDR_SIZE:
            continue
        (h_client, h_object, cmd, status, params_size,
         rpc_flags, ctrl_flags, access, _res) = struct.unpack_from(CTRL_FMT, r["body"], off)
        body_off = off + CTRL_HDR_SIZE
        params = r["body"][body_off:body_off + params_size]
        out.append(dict(seq=r["seq"], dir=r["dir"], cmd=cmd, status=status,
                        params_size=params_size, params_present=len(params),
                        h_client=h_client, h_object=h_object,
                        ctrl_flags=ctrl_flags, access=access,
                        ts_ns=r["ts_ns"], params=params))
    return out


def print_controls(ctrls):
    print("GSP_RM_CONTROL elements: %d" % len(ctrls))
    reqs = [c for c in ctrls if c["dir"] == DIR_REQ]
    reps = [c for c in ctrls if c["dir"] == DIR_REP]
    print("  %d requests / %d replies" % (len(reqs), len(reps)))
    empty = [c for c in reps if c["params_size"] and not c["params_present"]]
    print("  replies declaring params with NO bytes present: %d  "
          "(this is the `dlen=0` class; it must be 0)" % len(empty))
    by_cmd = collections.OrderedDict()
    for c in reps:
        e = by_cmd.setdefault(c["cmd"], dict(n=0, sizes=set(), statuses=set()))
        e["n"] += 1
        e["sizes"].add(c["params_size"])
        e["statuses"].add(c["status"])
    print("  distinct control commands (replies): %d" % len(by_cmd))
    print()
    print("  %-12s %5s  %-22s %s" % ("cmd", "n", "reply param bytes", "status"))
    for cmd in sorted(by_cmd):
        e = by_cmd[cmd]
        print("  0x%08x %5d  %-22s %s"
              % (cmd, e["n"], ",".join(str(s) for s in sorted(e["sizes"])),
                 ",".join("0x%x" % s for s in sorted(e["statuses"]))))


def hexdump(body, base=0):
    for i in range(0, len(body), 16):
        chunk = body[i:i + 16]
        hexs = " ".join("%02x" % b for b in chunk)
        text = "".join(chr(b) if 32 <= b < 127 else "." for b in chunk)
        print("  %06x  %-47s  %s" % (base + i, hexs, text))


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("trace", nargs="?", help="binary trace from /proc/driver/nvidia/rpctrace")
    ap.add_argument("--names", help="TSV of number<TAB>name")
    ap.add_argument("--extract-names", metavar="OGKM_DIR",
                    help="write a name TSV derived from an ogkm checkout and exit")
    ap.add_argument("--list", action="store_true", help="one line per record")
    ap.add_argument("--dir", choices=("req", "rep"), help="filter --list")
    ap.add_argument("--fn", type=lambda s: int(s, 0), help="filter --list by function")
    ap.add_argument("--hexdump", type=int, metavar="SEQ", help="dump one record's bytes")
    ap.add_argument("--controls", action="store_true",
                    help="decode GSP_RM_CONTROL elements: command, reply size, status")
    ap.add_argument("--json", metavar="OUT", help="write the summary as JSON")
    ap.add_argument("--allow-rx-errors", action="store_true")
    ap.add_argument("--expect-version", help="refuse unless the driver version matches")
    ap.add_argument("--quiet", action="store_true")
    args = ap.parse_args()

    if args.extract_names:
        names = extract_names(args.extract_names)
        print("# @generated by scripts/rpctrace/decode_rpctrace.py --extract-names")
        print("# source: open-gpu-kernel-modules rpc_global_enums.h")
        print("# Identifier names only. Regenerate from any checkout.")
        for num in sorted(names):
            print("%d\t%s" % (num, names[num]))
        return 0

    if not args.trace:
        ap.error("a trace file is required")

    try:
        with open(args.trace, "rb") as fh:
            blob = fh.read()
    except OSError as e:
        print("‼ %s" % e, file=sys.stderr)
        return 3

    names = {}
    if args.names:
        names = load_names(args.names)
    else:
        default = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                               "rpc_function_names.tsv")
        if os.path.exists(default):
            names = load_names(default)

    try:
        h, recs = verify_and_parse(blob, args.allow_rx_errors, args.expect_version)
    except Refused as e:
        print("‼ REFUSED: %s" % e, file=sys.stderr)
        return 2

    s = summarize(h, recs, names)

    if args.hexdump is not None:
        r = next((x for x in recs if x["seq"] == args.hexdump), None)
        if r is None:
            print("no record with seq %d" % args.hexdump, file=sys.stderr)
            return 3
        print(rec_line(r, names))
        print("  element header (%d bytes) then rpc header then payload:" % ELEM_HDR_SIZE)
        hexdump(r["body"])
        return 0

    if args.controls:
        print_controls(decode_controls(recs, names))
        return 0

    if args.list:
        for r in recs:
            if args.dir == "req" and r["dir"] != DIR_REQ:
                continue
            if args.dir == "rep" and r["dir"] != DIR_REP:
                continue
            if args.fn is not None and r["rpc_fn"] != args.fn:
                continue
            print(rec_line(r, names))
        return 0

    if not args.quiet:
        print_summary(s)
    if args.json:
        with open(args.json, "w") as fh:
            json.dump(s, fh, indent=2)
            fh.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
