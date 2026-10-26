"""Replay external observations beside the production-relay adversarial Rust harness.

The synthetic scheduler has no RM/GSP/guest-driver implementation. Missing observations
remain censored. A synthetic dropped edge demonstrates the detector, not a current-code bug.
"""
import argparse
import hashlib
import json
import random
import re
from pathlib import Path

LIMIT = 100_000
OPS = {"announce", "apply_begin", "apply_end", "publish", "host_complete", "edge",
       "guest_observe", "horizon", "migration", "credit_wait", "credit_return",
       "hold", "pass", "entry_publish"}


def classify(published, host_complete, guest_observed, coverage):
    if not coverage:
        return "unknown"
    if not published:
        return "i_unpublished"
    if host_complete is None:
        return "unknown"
    if not host_complete:
        return "iii_host_pending"
    if guest_observed is None:
        return "ii_candidate"
    return "delivered" if guest_observed else "ii_undelivered"


def replay(events, bound=12):
    if len(events) > LIMIT or bound < 0:
        raise ValueError("replay budget")
    published, completed, observed = set(), {}, {}
    failures, prev = set(), -1
    for e in events:
        ns, op, packet = e["ns"], e["op"], e["packet"]
        if not isinstance(ns, int) or ns < prev or op not in OPS:
            raise ValueError("noncanonical event")
        prev = ns
        if op == "publish":
            if packet in published:
                failures.add("DUPLICATE_PUBLICATION")
            published.add(packet)
        elif op == "host_complete":
            if packet not in published:
                failures.add("COMPLETION_WITHOUT_PUBLICATION")
            completed.setdefault(packet, ns)
        elif op == "guest_observe":
            observed.setdefault(packet, ns)
        # A trace's last record is not a complete horizon. Only an explicit complete
        # observation interval or an actual guest observation establishes lateness.
        for p, start in completed.items():
            end = observed.get(p)
            if (end is not None and end - start > bound) or (
                end is None and op == "horizon" and ns - start > bound
            ):
                failures.add("COMPLETION_DELIVERY_LATE")
    return sorted(failures)


def schedule(seed, drop_edge=False):
    # A seeded topological scheduler for external actors. Apply begin/end cannot
    # freeze unrelated completion actors. The Rust sibling explores real relay steps.
    rng = random.Random(seed)
    deps = {"announce": set(), "apply_begin": set(), "apply_end": {"apply_begin"},
            "publish": {"announce"}, "host_complete": {"publish"},
            "edge": {"host_complete"}, "guest_observe": {"edge"},
            "migration": {"announce"}, "credit_wait": {"publish"},
            "credit_return": {"credit_wait", "host_complete"}}
    done, events = set(), []
    while len(done) < len(deps):
        ready = sorted(op for op, needs in deps.items() if op not in done and needs <= done)
        op = ready[rng.randrange(len(ready))]
        done.add(op)
        if not drop_edge or op not in {"edge", "guest_observe"}:
            events.append({"ns": len(done), "op": op, "packet": "synthetic:1"})
    events.append({"ns": 100, "op": "horizon", "packet": "synthetic:1"})
    return events


def minimize(events, bound=12):
    # One-minimal subsequence with identical failure signature, retaining real times.
    target = replay(events, bound)
    if not target:
        return list(events)
    out, i = list(events), 0
    while i < len(out):
        candidate = out[:i] + out[i + 1:]
        if replay(candidate, bound) == target:
            out = candidate
            i = 0
        else:
            i += 1
    return out


def convert(text):
    if len(text.encode()) > 16 * 1024 * 1024:
        raise ValueError("trace byte budget")
    events, notes = [], []
    for line in text.splitlines():
        fields = dict(re.findall(r"([a-z_]+)=([^ ]+)", line))
        if "GATELOG REC" in line and "t_ns" in fields:
            token = fields.get("token", "unknown")
            # Accept the canonical numeric fixture form and the production renderer.
            kind = fields.get("kind")
            if kind is None:
                match = re.search(r"t_ms=\S+ ([a-z_]+)", line)
                kind = match[1] if match else None
            ns = int(fields["t_ns"])
            if kind in {"marker_published", "marker_seen"}:
                marker = fields.get("marker", fields.get("a", "0"))
                op = "publish" if kind == "marker_published" else "host_complete"
                events.append({"ns": ns, "op": op, "packet": f"{token}:{marker}"})
            elif kind in {"published", "put_advance"}:
                # Raw guest-entry/PUT records are useful ordering landmarks but do not
                # identify the driver's timed-out packet or establish marker coverage.
                if fields.get("held_by_gate") != "2":
                    op = "entry_publish" if kind == "published" else "announce"
                    events.append({"ns": ns, "op": op, "packet": token})
            elif kind in {"gate_start", "gate_end"}:
                op = "hold" if kind == "gate_start" else "pass"
                events.append({"ns": ns, "op": op, "packet": token})
        elif "ENS3 APPLY" in line and "start_ns" in fields:
            packet = "apply:" + fields.get("walk", "unknown")
            events.append({"ns": int(fields["start_ns"]), "op": "apply_begin",
                           "packet": packet})
            end = re.fullmatch(r"Some\((\d+)\)", fields.get("end_ns", ""))
            if end:
                events.append({"ns": int(end[1]), "op": "apply_end", "packet": packet})
        elif "ENS3 PROGRESS" in line or "NSI" in line:
            notes.append("snapshot/cumulative NSI is not packet-specific guest delivery")
    if len(events) > LIMIT:
        raise ValueError("trace event budget")
    events.sort(key=lambda e: (e["ns"], 0 if e["op"] == "publish" else 1))
    return {"events": events, "coverage": "incomplete", "result": "no reproduction",
            "sha256": hashlib.sha256(text.encode()).hexdigest(),
            "notes": sorted(set(notes + ["guest delivery and complete horizon require sidecar"]))}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--trace", type=Path)
    p.add_argument("--seed", type=int, default=42)
    p.add_argument("--synthetic-drop-edge", action="store_true")
    p.add_argument("--bound-ns", type=int, default=12)
    p.add_argument("--replay", type=Path, help="canonical schedule including guest sidecar events")
    a = p.parse_args()
    if a.trace:
        if a.trace.stat().st_size > 16 * 1024 * 1024:
            p.error("trace byte budget")
        out = convert(a.trace.read_text())
    else:
        if a.replay and a.replay.stat().st_size > 16 * 1024 * 1024:
            p.error("replay byte budget")
        events = (json.loads(a.replay.read_text())["events"] if a.replay
                  else schedule(a.seed, a.synthetic_drop_edge))
        failures = replay(events, a.bound_ns)
        out = {"seed": a.seed, "events": events, "failures": failures,
               "minimized": minimize(events, a.bound_ns),
               "result": "synthetic detector failure" if failures else "no reproduction"}
    print(json.dumps(out, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
