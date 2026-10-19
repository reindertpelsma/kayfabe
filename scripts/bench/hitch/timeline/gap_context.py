# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Observed MMU, interrupt and guest-residency context; never assigns a cause."""
import collections

from reply_waits import inventory

MMU_KINDS = {"mmu_arrive", "mmu_walk", "mmu_plan", "mmu_apply", "mmu_clear"}
ENGINES = ("CE2", "CE3", "GR0")


def _duration(events):
    if not events or any("end_ns" not in e for e in events):
        return None
    durations = [int(e["end_ns"]) - int(e["ts_ns"]) for e in events]
    if any(d < 0 for d in durations):
        raise ValueError("negative operation duration")
    return sum(durations)


def _invalidates(events, lo, hi, complete):
    groups = collections.defaultdict(list)
    unidentified = 0
    for e in events:
        if e.get("kind") not in MMU_KINDS:
            continue
        if "inval_seq" not in e:
            unidentified += lo <= e["ts_ns"] < hi
            continue
        groups[(e.get("device"), e.get("generation"), e["inval_seq"])].append(e)
    requests = []
    arrivals_in_gap = 0
    for (device, generation, seq), rows in groups.items():
        arrivals = [e for e in rows if e["kind"] == "mmu_arrive"]
        clears = [e for e in rows if e["kind"] == "mmu_clear"]
        arrive = arrivals[0]["ts_ns"] if len(arrivals) == 1 else None
        clear = clears[0]["ts_ns"] if len(clears) == 1 else None
        touches = any(e["ts_ns"] < hi and e.get("end_ns", e["ts_ns"]) >= lo for e in rows)
        pending_across = arrive is not None and arrive < hi and (clear is None or clear >= lo)
        if not touches and not pending_across:
            continue
        arrivals_in_gap += sum(lo <= e["ts_ns"] < hi for e in arrivals)
        elapsed = clear - arrive if arrive is not None and clear is not None and clear >= arrive else None
        state = "completed" if elapsed is not None else "pending" if not clears else "incomplete"
        if len(arrivals) > 1 or len(clears) > 1:
            state = "ambiguous"
        steps = {kind: [e for e in rows if e["kind"] == "mmu_" + kind]
                 for kind in ("walk", "plan", "apply")}
        requests.append(dict(device=device, generation=generation, inval_seq=seq,
                             arrival_ns=arrive, clear_ns=clear, state=state,
                             arrive_to_clear_ns=elapsed,
                             walk_ns=_duration(steps["walk"]), plan_ns=_duration(steps["plan"]),
                             apply_ns=_duration(steps["apply"]), steps=steps,
                             clear_outcomes=[e.get("outcome") for e in clears]))
    requests.sort(key=lambda r: (r["arrival_ns"] if r["arrival_ns"] is not None else hi,
                                 str(r["device"]), str(r["generation"]), r["inval_seq"]))
    exact = bool(complete and not unidentified and all(r["generation"] is not None
                 and r["state"] != "ambiguous" for r in requests))
    return dict(observed_count=len(requests), requests=requests, unidentified_events=unidentified,
                complete=exact, observed_arrivals_in_gap=arrivals_in_gap,
                exact_arrivals_in_gap=arrivals_in_gap if exact else None)


def gap_context(events, lo, hi, coverage, lost):
    """Full intervals and bracketing samples, with separate completeness assertions.

    Interval request identities include device AND driver generation. Counts
    describe observations even when incomplete. Exact rates require coverage
    and no recorder loss. A lack of events without coverage never means zero.
    """
    duration_s = (hi - lo) / 1e9
    counts = {engine: 0 for engine in ENGINES}
    for e in events:
        if e.get("kind") == "completion" and lo <= e["ts_ns"] < hi and e.get("engine") in counts:
            n = int(e.get("count", 1))
            if n < 0:
                raise ValueError("negative completion count")
            counts[e["engine"]] += n
    completion_complete = bool(coverage.get("completions") and not lost)
    completions = {engine: dict(observed_count=n, complete=completion_complete,
                               per_s=n/duration_s if completion_complete else None)
                   for engine, n in counts.items()}
    vram = sorted((e for e in events if e.get("kind") == "guest_vram"), key=lambda e: e["ts_ns"])
    before = [e for e in vram if e["ts_ns"] < lo]
    inside = [e for e in vram if lo <= e["ts_ns"] <= hi]
    after = [e for e in vram if e["ts_ns"] > hi]
    samples = [dict(e, position=position) for position, rows in
               (("before", before[-1:]), ("inside", inside), ("after", after[:1])) for e in rows]
    present_count = sum(e.get("kind") == "guest_present" and lo <= e["ts_ns"] < hi for e in events)
    return dict(invalidates=_invalidates(events, lo, hi, coverage.get("mmu") and not lost),
                reply_waits=inventory(events, lo, hi, coverage=coverage.get("replies"), lost=lost),
                completions=completions, guest_vram=samples,
                guest_present_observed_count=present_count,
                guest_present_count=present_count if coverage.get("guest") and not lost else None,
                stall_confirmed_by_counters=False)
