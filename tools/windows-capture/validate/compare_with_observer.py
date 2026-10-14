#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Compare a kfwc capture (KFGSPCAP) with the VFIO/kf3 observer's gsp.jsonl of the SAME boot.

    compare_with_observer.py capture.kfgspcap gsp.jsonl [--json]

Home-host validation, step 2 of the plan (docs/design/V3_WINDOWS_CAPTURE_MODULE.md): kayfabe's
observer (WIN_GSP_OBSERVER=1) records the guest's driver<->GSP messages from the host while the
capture module records them from inside the guest. Falsifier: any message the module recorded
that the observer did not record with identical bytes, or any (direction, transport seq) with two
different byte strings. Messages only the observer has are reported (expected for the prefix the
module attached too late to see, and for anything the module announced as lost); messages only
the module has are a failure.

Messages are paired with the observer per GSP boot (queue_id <-> generation, by most identical messages), because every boot of the same driver sends nearly the same bytes at the same sequence numbers and a flat (direction, seq) match would accept a message from the wrong boot.

Pre-610 element layout only (the observer's profile is 580). Bytes compared: the element header,
the RPC envelope and the body, zero padded to 8 (the observer's payload).
"""
import json
import struct
import sys

MAGIC = b"KFGSPCAP"
KINDS = {1: "bind", 2: "elements", 3: "loss_overflow", 4: "loss_unreadable", 5: "loss_unknown", 6: "end"}


def read_capture(path):
    data = open(path, "rb").read()
    if data[:8] != MAGIC or struct.unpack_from("<III", data, 8) != (1, 128, 32):
        raise SystemExit("not a KFGSPCAP v1 file")
    at, recs = 128, []
    while at < len(data):
        if at + 32 > len(data):
            raise SystemExit("truncated record header")
        magic, kind, d, qid, plen, counter, ts, first, count = struct.unpack_from("<IBBHIIQII", data, at)
        if magic != 0x4345524B or kind not in KINDS or d > 1:
            raise SystemExit("bad record at %d" % at)
        payload = data[at + 32: at + 32 + plen]
        if len(payload) != plen:
            raise SystemExit("truncated payload")
        recs.append(dict(kind=KINDS[kind], dir=d, qid=qid, counter=counter, first=first, count=count, payload=payload))
        at += 32 + plen
    return recs


def module_messages(recs):
    """Reassemble messages per (queue_id, dir) from ELEMENTS, like the decoder; returns [(dir, seq, bytes)]."""
    ring, buf, out, losses = {}, {}, [], 0
    for r in recs:
        key = (r["qid"], r["dir"])
        if r["kind"] == "bind":
            ring[key] = struct.unpack_from("<I", r["payload"], 12)[0]
            buf[key] = b""
        elif r["kind"].startswith("loss"):
            buf[key] = b""
            losses += 1
        elif r["kind"] == "elements":
            buf[key] = buf.get(key, b"") + r["payload"]
            while len(buf[key]) >= 4096:
                b = buf[key]
                hdr = struct.unpack_from("<I", b, 48)[0], struct.unpack_from("<I", b, 52)[0]
                if hdr != (0x03000000, 0x43505256):
                    buf[key] = b[4096:]
                    continue
                length = struct.unpack_from("<I", b, 56)[0]
                elems = struct.unpack_from("<I", b, 40)[0]
                if elems == 0 or len(b) < elems * 4096:
                    break
                seq = struct.unpack_from("<I", b, 36)[0]
                size = (48 + length + 7) & ~7
                out.append((r["dir"], seq, bytes(b[:size]), r["qid"]))
                buf[key] = b[elems * 4096:]
    return out, losses


def observer_messages(path):
    """[(direction, queue_sequence, bytes, generation)]; generation is 0 for an export that has none."""
    out = []
    with open(path, encoding="utf-8-sig") as f:
        for line in f:
            row = json.loads(line)
            if row.get("kind") == "record":
                out.append((row["direction"], row["queue_sequence"], bytes.fromhex(row["payload_hex"]),
                            row.get("generation", 0)))
    return out


def compare(mod, obs):
    """Flat comparison keyed by (direction, seq). LENIENT across GSP boots: every boot of the same driver
    sends nearly the same bytes at the same sequence numbers, so a message may match a record of another
    boot. Use compare_by_generation for the verdict."""
    obs_by = {}
    for d, s, b, _g in obs:
        obs_by.setdefault((d, s), set()).add(b)
    matched = module_only = conflicts = 0
    for d, s, b, _q in mod:
        got = obs_by.get((d, s))
        if got is None:
            module_only += 1
        elif b in got:
            matched += 1
        else:
            conflicts += 1
    mod_keys = {(d, s) for d, s, _, _ in mod}
    observer_only = sum(1 for k in obs_by if k not in mod_keys)
    multi = sum(1 for v in obs_by.values() if len(v) > 1)
    return dict(module_messages=len(mod), observer_messages=len(obs), matched=matched, module_only=module_only,
                byte_conflicts=conflicts, observer_only_keys=observer_only, observer_keys_with_two_byte_strings=multi)


def compare_by_generation(mod, obs):
    """Each module queue instance (queue_id) is paired with the observer generation it shares the most
    byte-identical (direction, seq) messages with; then module_only (key absent in that generation) and
    byte_conflicts (key present, bytes differ) are counted inside the pair only. A module message of a
    queue instance with no pairing counts as module_only. Returns the summary and a detail list."""
    gens = {}
    for d, s, b, g in obs:
        gens.setdefault(g, {}).setdefault((d, s), []).append(b)
    qids = sorted({q for _, _, _, q in mod})
    pair, detail = {}, []
    for q in qids:
        best, best_n = None, 0
        for g, keyed in gens.items():
            n = sum(1 for d, s, b, qq in mod if qq == q and b in keyed.get((d, s), ()))
            if n > best_n:
                best, best_n = g, n
        pair[q] = best
    matched = module_only = conflicts = 0
    per = {}
    for q in qids:
        per[q] = dict(observer_generation=pair[q], messages=0, matched=0, module_only=0, byte_conflicts=0)
    last_match = {}
    for i, (d, s, b, q) in enumerate(mod):
        row = per[q]
        row["messages"] += 1
        keyed = gens.get(pair[q], {})
        got = keyed.get((d, s))
        if got is not None and b in got:
            matched += 1
            row["matched"] += 1
            last_match[q] = i
        elif got is None:
            module_only += 1
            row["module_only"] += 1
            detail.append(dict(kind="module_only", queue_id=q, dir=d, seq=s, index=i, bytes=len(b)))
        else:
            conflicts += 1
            row["byte_conflicts"] += 1
            detail.append(dict(kind="byte_conflict", queue_id=q, dir=d, seq=s, index=i, bytes=len(b)))
    for x in detail:  # a difference after the last identical message of its queue instance is a "tail" one
        x["after_last_matched_message_of_its_queue"] = x["index"] > last_match.get(x["queue_id"], -1)
    return dict(module_messages=len(mod), observer_messages=len(obs), matched=matched, module_only=module_only,
                byte_conflicts=conflicts, pairing=per, differences=detail)


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    recs = read_capture(argv[0])
    mod, losses = module_messages(recs)
    obs = observer_messages(argv[1])
    res = compare_by_generation(mod, obs)
    flat = compare(mod, obs)
    res["flat_lenient"] = {k: flat[k] for k in ("matched", "module_only", "byte_conflicts", "observer_only_keys",
                                              "observer_keys_with_two_byte_strings")}
    res["module_loss_records"] = losses
    # the falsifier is strict: any module message the observer lacks, or with other bytes, fails
    res["pass"] = res["module_only"] == 0 and res["byte_conflicts"] == 0 and res["matched"] > 0
    if "--json" in argv:
        print(json.dumps(res, indent=1))
    else:
        for k, v in res.items():
            if k not in ("pairing", "differences", "flat_lenient"):
                print("%s=%s" % (k, v))
        for q, row in res["pairing"].items():
            print("queue_id %s -> observer generation %s: %s" % (q, row["observer_generation"], row))
        for x in res["differences"]:
            print("DIFFERENCE %s" % x)
    return 0 if res["pass"] else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
