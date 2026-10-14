#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Independent check of `kf-census scrub` on an observer export (no kf code; prints counts only).

STATUS: RESEARCH, 2026-10-13. Reads an original `gsp.jsonl` and its scrubbed copy, record by record, and verifies
from the byte layout alone (element header 48 B, RPC envelope 32 B, control header 40 B at 580.x) that:

  * every byte of the scrubbed message that is not on the keep list is the fill pattern `KFSCRUB!` (body bytes) or zero;
  * every byte on the keep list (envelope, control header, allow-listed bodies, seq/elemCount) is unchanged;
  * the checksum folds to zero;
  * the bodies of non-allow-listed messages hold no random-looking 16-byte window (>= 14 distinct values, >= 12
    non-zero; stride 4 from the body start) in the scrubbed file, while the original has some (the test has power).

usage: python3 -I scrub_check.py ORIGINAL.jsonl SCRUBBED.jsonl ALLOW_LIST.txt GSP_RM_CONTROL_FUNCTION_ID
"""
import json
import re
import sys

FILL = b"KFSCRUB!"
HDR, ENV, CTL = 48, 32, 40


def allow_ids(path):
    out = set()
    for l in open(path):
        l = l.split("#")[0].strip()
        if l:
            out.add(int(l, 16))
    return out


def fold(b, n):
    n = min((n + 7) // 8 * 8, len(b))
    acc = 0
    for i in range(0, n, 8):
        acc ^= int.from_bytes(b[i : i + 8], "little")
    return (acc >> 32) ^ (acc & 0xFFFFFFFF)


def hent(w):
    return len(set(w)) >= 14 and sum(1 for x in w if x) >= 12


def records(path):
    with open(path, "rb") as f:
        for line in f:
            if line.startswith(b'{"kind":"record"'):
                yield json.loads(line)


def main():
    orig_p, scr_p, allow_p, ctl_fn = sys.argv[1:5]
    allow, ctl_fn = allow_ids(allow_p), int(ctl_fn)
    n = bad_keep = bad_fill = bad_sum = 0
    orig_w = scr_w = 0   # random-looking windows in non-allow-listed bodies: original / scrubbed
    kept_bodies = 0
    for a, b in zip(records(orig_p), records(scr_p)):
        o, s = bytes.fromhex(a["payload_hex"]), bytes.fromhex(b["payload_hex"])
        n += 1
        if len(o) != len(s):
            bad_keep += 1
            continue
        rpc_len = int.from_bytes(o[56:60], "little")
        extent = HDR + rpc_len
        fn = int.from_bytes(o[60:64], "little")
        keep = set(range(HDR, HDR + ENV))                       # envelope
        keep |= set(range(36, 44))                               # seqNum, elemCount (580 layout)
        body_from = HDR + ENV
        allowed = False
        if fn == ctl_fn:
            keep |= set(range(HDR + ENV, HDR + ENV + CTL))      # control header
            body_from = HDR + ENV + CTL
            cmd = int.from_bytes(o[88:92], "little")
            allowed = cmd in allow
        elif fn == 0x47:                                         # continuation record: payload is body
            pass
        if allowed:
            keep |= set(range(body_from, extent))
            kept_bodies += 1
        if fold(s, extent) != 0:
            bad_sum += 1
        for i in range(len(o)):
            if 32 <= i < 36:
                continue  # the checksum word is recomputed (checked by the fold above)
            if i in keep:
                bad_keep += o[i] != s[i]
            elif i >= extent:
                bad_fill += s[i] != 0
            elif i >= body_from:
                bad_fill += s[i] not in FILL
            else:
                bad_fill += s[i] != 0
        if not allowed:
            # random-looking 16-byte windows (stride 4 from the body start) in the body region
            for i in range(body_from, max(body_from, extent - 15), 4):
                orig_w += hent(o[i : i + 16])
                scr_w += hent(s[i : i + 16])
    print(f"records={n} allow_listed_bodies_kept={kept_bodies} keep_violations={bad_keep} fill_violations={bad_fill} "
          f"checksum_nonzero={bad_sum} random_looking_windows_original={orig_w} random_looking_windows_scrubbed={scr_w}")


if __name__ == "__main__":
    main()
