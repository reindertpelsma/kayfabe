#!/usr/bin/env python3
"""Desktop-level baton analysis for the Linux-desktop-under-kayfabe test (desktop_baton.sh).

No GPU needed.  Inputs are the per-frame logs written by glwin (same format as baton.c, so
baton_parse.py reads them too): one app client per window (A, B), an optional small vsynced
"canary" window (C) whose frame cadence is a proxy for what the guest compositor presents, and
optionally the VMM's qemu.log (host broker / display counters).

  desktop_baton_parse.py report --client A.log:A.csv --client B.log:B.csv \
        [--canary C.log:C.csv] [--qemu qemu.log] [--min-gap 4]

Differences from baton_parse.detect_batons: the TAIL of a run counts (a client silent from its
last completion to the end of the window is a right-censored gap, flagged `censored`), a client
with no completion at all is one censored gap, and every gap >= min_gap is classified:

  BATON                        an app is silent while another app completes >= 3 frames inside the gap
  APP_STALL_COMPOSITOR_ALIVE   no app completes, but the canary does (apps starved, compositor fine)
  COMPOSITOR_STALL_APPS_ALIVE  the canary is silent while an app completes frames
  GLOBAL_STALL                 every client is silent (whole GPU / guest stall, not a baton)
  SOLO_STALL                   a gap in a run with a single client
"""
import argparse
import math
import re
import sys

import baton_parse as bp

KIND_PRIORITY = ["BATON", "APP_STALL_COMPOSITOR_ALIVE", "COMPOSITOR_STALL_APPS_ALIVE",
                 "GLOBAL_STALL", "SOLO_STALL"]


def client_gaps(its, duration_s):
    """[(from_s, to_s, censored)] between successive completions, t0 -> first completion, and
    last completion -> duration_s (censored) when that tail is non-empty."""
    done = sorted(i[3] for i in its)
    gaps, prev = [], 0.0
    for d in done:
        gaps.append((prev, d, False))
        prev = d
    if duration_s - prev > 1e-9:
        gaps.append((prev, float(duration_s), True))
    return gaps


def _count_inside(sorted_done, lo, hi):
    return sum(1 for d in sorted_done if lo + 1e-6 < d < hi - 1e-6)


def classify(clients, min_gap_s=4.0, min_other=3):
    """clients: {label: {t0, its, duration, role('app'|'canary')}} -> list of events.

    Event: kind, clients [labels], from_s, to_s, gap_s (relative to the earliest t0), censored,
    others {label: completions inside the gap}, canary_alive (True/False, None without canary).
    """
    base = min(c["t0"] for c in clients.values())
    absdone = {lab: sorted(c["t0"] - base + i[3] for i in c["its"]) for lab, c in clients.items()}
    canaries = [lab for lab, c in clients.items() if c.get("role") == "canary"]
    raw = []
    for lab, c in clients.items():
        off = c["t0"] - base
        for a, b, cens in client_gaps(c["its"], c["duration"]):
            if b - a < min_gap_s:
                continue
            lo, hi = a + off, b + off
            others = {o: _count_inside(absdone[o], lo, hi) for o in clients if o != lab}
            apps_active = any(n >= min_other for o, n in others.items() if clients[o].get("role") != "canary")
            canary_active = any(others[o] >= min_other for o in canaries if o != lab)
            if not others:
                kind = "SOLO_STALL"
            elif clients[lab].get("role") == "canary":
                kind = "COMPOSITOR_STALL_APPS_ALIVE" if apps_active else "GLOBAL_STALL"
            elif apps_active:
                kind = "BATON"
            elif canary_active:
                kind = "APP_STALL_COMPOSITOR_ALIVE"
            else:
                kind = "GLOBAL_STALL"
            raw.append({"kind": kind, "clients": [lab], "from_s": lo, "to_s": hi, "gap_s": b - a,
                        "censored": cens, "others": others,
                        "canary_alive": (canary_active if canaries and lab not in canaries else
                                         (None if not canaries else False))})
    # merge overlapping events of one non-BATON kind across clients (one stall, several witnesses)
    merged = []
    for e in sorted(raw, key=lambda e: (e["from_s"], e["kind"])):
        if e["kind"] != "BATON":
            for m in merged:
                if m["kind"] == e["kind"] and e["from_s"] < m["to_s"] and m["from_s"] < e["to_s"]:
                    m["clients"] += e["clients"]
                    m["from_s"] = min(m["from_s"], e["from_s"])
                    m["to_s"] = max(m["to_s"], e["to_s"])
                    m["gap_s"] = m["to_s"] - m["from_s"]
                    m["censored"] = m["censored"] or e["censored"]
                    break
            else:
                merged.append(e)
        else:
            merged.append(e)
    return merged


def verdict(events):
    kinds = {e["kind"] for e in events}
    for k in KIND_PRIORITY:
        if k in kinds:
            return k
    return "NO_EVENT_OBSERVED"


def _pct(sorted_vals, p):
    if not sorted_vals:
        return 0.0
    k = max(0, min(len(sorted_vals) - 1, int(math.ceil(p / 100.0 * len(sorted_vals))) - 1))
    return sorted_vals[k]


def frame_stats(its, duration_s):
    done = sorted(i[3] for i in its)
    ints = [(b - a) * 1000.0 for a, b in zip(done, done[1:])]
    sints = sorted(ints)
    base = bp.compute_stats(its, duration_s)
    tail = max(0.0, (duration_s - done[-1]) * 1000.0) if done else duration_s * 1000.0
    return {
        "frames": len(done),
        "fps": len(done) / duration_s if duration_s else 0.0,
        "interval_p50_ms": _pct(sints, 50),
        "interval_p99_ms": _pct(sints, 99),
        "interval_max_ms": sints[-1] if sints else 0.0,
        "lead_gap_ms": done[0] * 1000.0 if done else duration_s * 1000.0,
        "tail_gap_ms": tail,
        "longest_gap_incl_tail_ms": max(base["longest_gap_ms"], tail),
        "zero_sec": base["zero_sec"],
    }


def per_second_fps(its, duration_s):
    n = int(math.ceil(duration_s))
    b = [0] * n
    for i in its:
        s = int(i[3])
        if 0 <= s < n:
            b[s] += 1
    return b


_FRAG = re.compile(r"\b([a-z_]+)\[([^\]]*)\]")


def _frags(text):
    for m in _FRAG.finditer(text):
        name, body = m.group(1), m.group(2)
        d = {}
        for tok in body.split():
            if "=" in tok:
                k, v = tok.split("=", 1)
                try:
                    d[k] = int(v)
                except ValueError:
                    d[k] = v
        if d:
            yield name, d


def parse_status_fragments(text):
    """{'broker': {...}, 'disp': {...}} - the LAST occurrence of each name[k=v ...] fragment."""
    out = {}
    for name, d in _frags(text):
        out[name] = d
    return out


def status_first_last(text, name):
    seq = [d for n, d in _frags(text) if n == name]
    return (seq[0], seq[-1]) if seq else (None, None)


def count_events(text):
    return {
        "stall": text.count("display: STALL"),
        "exception": text.count("Exception"),
        "scanout_refused": text.count("scanout REFUSED"),
        "cursor_refused": text.count("cursor REFUSED"),
        "broker_lines": text.count("kf3: broker"),
    }


def _load(spec, role):
    path, _, dump = spec.partition(":")
    with open(path) as f:
        log = bp.parse_log(f.read())
    its = None
    if dump:
        with open(dump) as f:
            its = bp.parse_dump(f.read())
    if its is None:
        raise SystemExit("desktop_baton_parse: %s needs a :dump.csv (every frame)" % path)
    return log, {"t0": log["t0_epoch"] or 0.0, "its": its, "duration": float(log["duration_s"] or 0),
                 "role": role}


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("cmd", choices=["report"])
    ap.add_argument("--client", action="append", default=[], help="LOG:DUMP.csv of an app window")
    ap.add_argument("--canary", action="append", default=[], help="LOG:DUMP.csv of the canary window")
    ap.add_argument("--qemu", help="the VMM's qemu.log (broker/display counters)")
    ap.add_argument("--min-gap", type=float, default=4.0)
    ap.add_argument("--per-second", action="store_true", help="print the per-second frame counts")
    a = ap.parse_args(argv)
    if not a.client:
        ap.error("at least one --client")
    clients, logs = {}, {}
    for spec, role in [(s, "app") for s in a.client] + [(s, "canary") for s in a.canary]:
        log, c = _load(spec, role)
        lab = log["label"] or "c%d" % len(clients)
        clients[lab], logs[lab] = c, log
    print("%-8s %-7s %7s %8s %9s %9s %10s %10s %12s %5s" % (
        "label", "role", "frames", "fps", "p50_ms", "p99_ms", "max_int_ms", "tail_ms", "longest_ms", "zero"))
    for lab, c in clients.items():
        s = frame_stats(c["its"], c["duration"])
        print("%-8s %-7s %7d %8.1f %9.2f %9.2f %10.1f %10.1f %12.1f %5d" % (
            lab, c["role"], s["frames"], s["fps"], s["interval_p50_ms"], s["interval_p99_ms"],
            s["interval_max_ms"], s["tail_gap_ms"], s["longest_gap_incl_tail_ms"], s["zero_sec"]))
        if a.per_second:
            print("  fps/s %s: %s" % (lab, " ".join(str(x) for x in per_second_fps(c["its"], c["duration"]))))
    ev = classify(clients, a.min_gap)
    print("GAP EVENTS (>= %.1f s): %d" % (a.min_gap, len(ev)))
    for e in ev:
        print("  %s clients=%s %.2f..%.2f s (%.2f s)%s others=%s canary_alive=%s" % (
            e["kind"], ",".join(e["clients"]), e["from_s"], e["to_s"], e["gap_s"],
            " CENSORED" if e["censored"] else "", e["others"], e["canary_alive"]))
    if a.qemu:
        with open(a.qemu, errors="replace") as f:
            text = f.read()
        first, last = status_first_last(text, "broker")
        if last:
            print("host broker sent=%s dropped=%s uncommitted=%s blocked=%s backstops=%s gpucopy=%s "
                  "(first status sent=%s; run totals for the whole boot, not per phase)" % (
                      last.get("sent"), last.get("dropped"), last.get("uncommitted"), last.get("blocked"),
                      last.get("backstops"), last.get("gpucopy"), first.get("sent")))
        else:
            print("host broker status: none in qemu.log")
        c = count_events(text)
        print("host events display_stall=%d exception=%d scanout_refused=%d cursor_refused=%d broker_lines=%d" % (
            c["stall"], c["exception"], c["scanout_refused"], c["cursor_refused"], c["broker_lines"]))
    print("DESKTOP_BATON_VERDICT=%s" % verdict(ev))
    return 0


if __name__ == "__main__":
    sys.exit(main())
