#!/usr/bin/env python3
"""Parser, statistics and baton detector for the output of ./baton (baton.c).

No GPU needed.  Usage:
  baton_parse.py report A.log[:A.dump.csv] B.log[:B.dump.csv] [--min-gap 4]
  baton_parse.py report A.log                     (single client)

A client's iterations are (idx, start_s, dur_ms, done_s), times relative to the client's own
t0 (the start of its timed loop).  t0_epoch (CLOCK_REALTIME) in the log header aligns clients.
"""
import argparse
import math
import sys

__all__ = ["parse_log", "parse_dump", "compute_stats", "detect_batons", "share_of_work",
           "format_table"]


def _kv(tokens):
    out = {}
    for t in tokens:
        if "=" in t:
            k, v = t.split("=", 1)
            out[k] = v
    return out


def _num(v):
    try:
        return int(v)
    except ValueError:
        return float(v)


def parse_log(text):
    """Parse one ./baton stdout.  A log with no SUMMARY (killed run) has summary None."""
    log = {"label": None, "pid": None, "k_ms": None, "cycles": None, "duration_s": None,
           "t0_epoch": None, "per_sec": [], "summary": None, "slow": []}
    seen = False
    in_slow = False
    in_table = False
    for raw in text.splitlines():
        line = raw.strip()
        if not line:
            continue
        if in_slow:
            if line.startswith("SLOW_CSV_END"):
                in_slow = False
                continue
            p = line.split(",")
            if len(p) == 4:
                try:
                    log["slow"].append((int(p[0]), float(p[1]), float(p[2]), float(p[3])))
                except ValueError:
                    pass
            continue
        if line.startswith("# baton"):
            seen = True
        elif line.startswith("# label="):
            kv = _kv(line[2:].split())
            log["label"] = kv.get("label")
            log["pid"] = int(kv["pid"]) if "pid" in kv else None
            log["k_ms"] = float(kv["k_ms"]) if "k_ms" in kv else None
            log["cycles"] = int(kv["cycles"]) if "cycles" in kv else None
            log["duration_s"] = _num(kv["duration_s"]) if "duration_s" in kv else None
            seen = True
        elif line.startswith("# t0_epoch="):
            log["t0_epoch"] = float(line.split("=", 1)[1])
        elif line.startswith("sec iterations"):
            in_table = True
        elif line.startswith("SUMMARY"):
            log["summary"] = {k: _num(v) for k, v in _kv(line.split()[1:]).items()}
            in_table = False
        elif line.startswith("SLOW_CSV_BEGIN"):
            in_slow = True
            in_table = False
        elif in_table:
            p = line.split()
            if len(p) == 3:
                try:
                    log["per_sec"].append((int(p[0]), int(p[1]), float(p[2])))
                except ValueError:
                    pass
    if not seen:
        raise ValueError("not a baton log (no header)")
    return log


def parse_dump(text):
    """Parse the --dump-all CSV: idx,start_s,dur_ms,done_s (header optional)."""
    out = []
    for raw in text.splitlines():
        p = raw.strip().split(",")
        if len(p) != 4 or p[0] == "idx":
            continue
        out.append((int(p[0]), float(p[1]), float(p[2]), float(p[3])))
    return out


def compute_stats(iters, duration_s):
    """Same definitions as the C summary.

    longest_gap_ms: largest interval between successive completions, the first interval
    being t0 -> first completion.  zero_sec: seconds in [0, duration_s) with no completion.
    """
    done = sorted(i[3] for i in iters)
    n = len(done)
    nsec = int(math.ceil(duration_s))
    buckets = [0] * nsec
    for d in done:
        b = int(d)
        if 0 <= b < nsec:
            buckets[b] += 1
    gap, gap_end, prev = 0.0, 0.0, 0.0
    for d in done:
        g = d - prev
        if g > gap:
            gap, gap_end = g, d
        prev = d
    busy = sum(i[2] for i in iters)
    return {
        "iterations": n,
        "busy_ms": busy,
        "longest_gap_ms": gap * 1000.0,
        "longest_gap_end_s": gap_end,
        "first_done_ms": done[0] * 1000.0 if n else 0.0,
        "zero_sec": sum(1 for b in buckets if b == 0),
        "mean_iter_ms": busy / n if n else 0.0,
        "iters_per_s": n / duration_s if duration_s else 0.0,
    }


def _gaps(iters):
    done = sorted(i[3] for i in iters)
    prev = 0.0
    for d in done:
        yield prev, d
        prev = d


def detect_batons(clients, min_gap_s=4.0, min_other_completions=3):
    """clients: {label: (t0_epoch, iterations)}.

    A baton event: client X has a gap between successive completions >= min_gap_s while at
    least one other client completes >= min_other_completions iterations strictly inside
    that gap.  (If every client is silent the GPU as a whole stalled; that is not a baton.)
    from_s/to_s are relative to the earliest t0_epoch.
    """
    base = min(t0 for t0, _ in clients.values())
    abs_done = {lab: sorted(t0 - base + i[3] for i in its) for lab, (t0, its) in clients.items()}
    events = []
    for lab, (t0, its) in clients.items():
        off = t0 - base
        for a, b in _gaps(its):
            if b - a < min_gap_s:
                continue
            lo, hi = a + off, b + off
            others = {}
            for olab, od in abs_done.items():
                if olab == lab:
                    continue
                c = sum(1 for d in od if lo + 1e-6 < d < hi - 1e-6)
                others[olab] = c
            if others and max(others.values()) >= min_other_completions:
                events.append({"starved": lab, "from_s": lo, "to_s": hi, "gap_s": b - a,
                               "other_completions": max(others.values()), "others": others})
    events.sort(key=lambda e: e["from_s"])
    return events


def share_of_work(clients):
    """clients: {label: (iterations, cycles_per_iteration)} -> fraction of GPU work done."""
    work = {k: it * cy for k, (it, cy) in clients.items()}
    tot = sum(work.values())
    return {k: (w / tot if tot else 0.0) for k, w in work.items()}


def format_table(rows):
    cols = ["label", "iterations", "iters_per_s", "share", "longest_gap_ms", "zero_sec"]
    widths = [max(len(c), *(len(_fmt(r.get(c))) for r in rows)) for c in cols]
    head = "  ".join(c.ljust(w) for c, w in zip(cols, widths))
    body = ["  ".join(_fmt(r.get(c)).ljust(w) for c, w in zip(cols, widths)) for r in rows]
    return "\n".join([head] + body)


def _fmt(v):
    if v is None:
        return "-"
    if isinstance(v, float):
        return "%.3f" % v if abs(v) < 10 else "%.1f" % v
    return str(v)


def _load(spec):
    path, _, dump = spec.partition(":")
    with open(path) as f:
        log = parse_log(f.read())
    its = None
    if dump:
        with open(dump) as f:
            its = parse_dump(f.read())
    return log, its


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("cmd", choices=["report"])
    ap.add_argument("clients", nargs="+", help="LOG[:DUMP.csv] per client")
    ap.add_argument("--min-gap", type=float, default=4.0)
    a = ap.parse_args(argv)
    logs = [_load(s) for s in a.clients]
    clients, rows, shares_in = {}, [], {}
    for n, (log, its) in enumerate(logs):
        lab = log["label"] or "c%d" % n
        dur = log["duration_s"]
        if its is None:
            its = list(log["slow"])  # only slow iterations: stats from the C summary instead
            st = log["summary"] or {}
            full = False
        else:
            st = compute_stats(its, dur)
            full = True
        clients[lab] = (log["t0_epoch"] or 0.0, its)
        shares_in[lab] = (st.get("iterations", 0), log["cycles"] or 1)
        rows.append(dict(label=lab, full=full, **{k: st.get(k) for k in
                    ("iterations", "iters_per_s", "longest_gap_ms", "zero_sec", "busy_ms", "mean_iter_ms")}))
    for r in rows:
        r["share"] = share_of_work(shares_in)[r["label"]]
    print(format_table(rows))
    if len(clients) > 1:
        ev = detect_batons(clients, a.min_gap)
        print("BATON events (>= %.1f s starved while another client completes): %d" % (a.min_gap, len(ev)))
        for e in ev:
            print("  %s starved %.2f..%.2f s (%.2f s); other completions inside: %s"
                  % (e["starved"], e["from_s"], e["to_s"], e["gap_s"], e["others"]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
