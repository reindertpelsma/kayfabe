#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Rootless proof sampler (traces/rootless_proof_20261125): read /proc while a stage runs and record the identity and
capability sets of every thread of every process that matters.

Watched: every process whose real or effective uid is the run user, and every process whose comm names a kayfabe VMM or
helper (qemu-system-*, swtpm, kf-broker*) whatever its uid. For each thread: Uid, Gid, Groups, CapInh, CapPrm, CapEff,
CapBnd, CapAmb, NoNewPrivs, Seccomp. A record is written the first time a thread is seen and again when any of those
fields changes. Violations: a watched VMM or helper thread with any uid 0, any watched thread with a non-zero capability
set, or NoNewPrivs other than 1 on a watched VMM or helper thread.

Stops when the stop file exists. The last line is `PROCSAMPLE_SUMMARY {json}`. Stdlib only.
"""
import argparse
import json
import os
import pwd
import re
import sys
import time

FIELDS = ("Uid", "Gid", "Groups", "CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb", "NoNewPrivs", "Seccomp")
CAPS = ("CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb")
HELPER = re.compile(r"^(qemu-system-|swtpm|kf-broker)")


def status(path):
    out = {}
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                k, _, v = line.partition(":")
                if k in FIELDS or k == "Name":
                    out[k] = " ".join(v.split())
    except OSError:
        return None
    return out


def violations(st, helper):
    why = []
    uids = st.get("Uid", "").split()
    if helper and (not uids or "0" in uids):
        why.append("uid0")
    for c in CAPS:
        v = st.get(c)
        if v is None or int(v, 16) != 0:
            why.append(c.lower())
    if helper and st.get("NoNewPrivs") != "1":
        why.append("nnp")
    return why


def scan(uid, seen, out, counts, bad):
    for pid in os.listdir("/proc"):
        if not pid.isdigit():
            continue
        top = status(f"/proc/{pid}/status")
        if not top:
            continue
        uids = top.get("Uid", "").split()
        helper = bool(HELPER.match(top.get("Name", "")))
        if not helper and str(uid) not in uids[:2]:
            continue
        try:
            tids = os.listdir(f"/proc/{pid}/task")
        except OSError:
            continue
        for tid in tids:
            st = status(f"/proc/{pid}/task/{tid}/status")
            if not st:
                continue
            key = (pid, tid)
            snap = tuple(st.get(k) for k in FIELDS)
            if seen.get(key) == snap:
                continue
            first = key not in seen
            seen[key] = snap
            why = violations(st, helper)
            rec = {"t": round(time.time(), 1), "pid": int(pid), "tid": int(tid), "comm": top.get("Name"),
                   "thread": st.get("Name"), "first": first, "helper": helper, "violations": why}
            rec.update({k: st.get(k) for k in FIELDS})
            out.write(json.dumps(rec, sort_keys=True) + "\n")
            out.flush()
            name = top.get("Name", "?")
            c = counts.setdefault(name, {"processes": set(), "threads": 0})
            c["processes"].add(pid)
            c["threads"] += 1 if first else 0
            if why:
                bad.append({"pid": int(pid), "tid": int(tid), "comm": name, "violations": why})


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--user", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--stop-file", required=True)
    p.add_argument("--interval", type=float, default=1.0)
    a = p.parse_args()
    uid = pwd.getpwnam(a.user).pw_uid
    seen, counts, bad = {}, {}, []
    with open(a.out, "a", encoding="utf-8") as out:
        out.write(f"PROCSAMPLE_START user={a.user} uid={uid} t={time.time():.0f}\n")
        while not os.path.exists(a.stop_file):
            scan(uid, seen, out, counts, bad)
            time.sleep(a.interval)
        scan(uid, seen, out, counts, bad)
        summary = {"user": a.user, "uid": uid, "threads_seen": len(seen),
                   "by_comm": {k: {"processes": len(v["processes"]), "threads": v["threads"]} for k, v in sorted(counts.items())},
                   "violations": len(bad), "first_violations": bad[:20]}
        line = "PROCSAMPLE_SUMMARY " + json.dumps(summary, sort_keys=True)
        out.write(line + "\n")
    print(line)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
