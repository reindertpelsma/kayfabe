#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Pure H1 attribution over normalized CLOCK_MONOTONIC-ns JSONL evidence.

An interval needs a frame_seq, stage and tid linking it to the next content
present. Mere temporal overlap with a global MMU/RM operation is insufficient.
Classification is INFERRED; raw intervals and gaps are MEASURED observations.
"""
import argparse
import collections
import json
import math
import pathlib

KINDS = {"mmu_wait": "S1", "mmu_walk": "S1", "mmu_plan": "S1",
         "mmu_apply": "S1", "rm_call": "S1", "tick_wait": "S2",
         "credit_wait": "S2", "broker_wait": "S3", "compositor_wait": "S3",
         "cpu_stall": "S4", "gpu_wait": "S5", "guest_wait": "S6"}
RESOURCE = {"reclaim", "compaction", "runqueue"}


def qpc_to_host(qpc, calibration):
    """Integer conversion; calibration is measured per boot, never a timezone."""
    frequency = int(calibration["frequency"])
    if frequency <= 0:
        raise ValueError("QPC frequency must be positive")
    return int(calibration["host_ns0"]) + ((int(qpc) - int(calibration["qpc0"]))
                                          * 1_000_000_000 // frequency)


def _overlap(e, lo, hi):
    return max(0, min(int(e.get("end_ns", e["ts_ns"])), hi) - max(int(e["ts_ns"]), lo))


def _classify(window, lo, hi, seq, threshold_ns, coverage, lost, clock_error_ns):
    if not coverage or lost:
        return "UNKNOWN", "missing coverage or recorder loss"
    candidates = [e for e in window if e.get("kind") in KINDS
                  and e.get("frame_seq") == seq and "stage" in e and "tid" in e
                  and _overlap(e, lo, hi) > threshold_ns
                  and int(e.get("end_ns", e["ts_ns"])) <= hi]
    if not candidates:
        return "UNKNOWN", "no correlated delayed stage"
    first_stage = min(e["stage"] for e in candidates)
    first = [e for e in candidates if e["stage"] == first_stage]
    labels = {KINDS[e["kind"]] for e in first}
    if len(labels) != 1:
        return "UNKNOWN", "competing causes at the first delayed stage"
    e = min(first, key=lambda r: r["ts_ns"])
    for resource in window:
        if (resource.get("kind") in RESOURCE and resource.get("tid") == e["tid"]
                and _overlap(resource, max(lo, e["ts_ns"]), min(hi, e["end_ns"])) > 5_000_000):
            return "S7", "same-thread " + resource["kind"] + " covers first delayed stage"
    label = labels.pop()
    if label == "S6" and (clock_error_ns is None or clock_error_ns * 2 >= threshold_ns):
        return "UNKNOWN", "guest clock uncertainty prevents ordering"
    return label, "correlated first delayed stage: " + e["kind"]


def _percentile(values, fraction):
    return sorted(values)[max(0, math.ceil(len(values) * fraction) - 1)] if values else 0


def analyze(events, broker, *, threshold_ms=80, coverage=False, lost=0, clock_error_ns=None):
    """Return every new-content gap, its event window, metrics and class rates.

    broker rows: ts_ns, seq, new. Event intervals: ts_ns, end_ns, kind,
    frame_seq, stage, tid. Unlinked events remain visible in the window.
    coverage is an explicit assertion from collection validation, never guessed.
    """
    if threshold_ms <= 0 or not math.isfinite(threshold_ms):
        raise ValueError("gap threshold must be finite and positive")
    if lost < 0:
        raise ValueError("lost must be nonnegative")
    threshold = int(threshold_ms * 1_000_000)
    times = [int(b["ts_ns"]) for b in broker]
    if any(a > b for a, b in zip(times, times[1:])):
        raise ValueError("broker clock regressed; split resets/sessions before analysis")
    content = [b for b in broker if b["new"]]
    intervals = [int(b["ts_ns"]) - int(a["ts_ns"]) for a, b in zip(content, content[1:])]
    span = (times[-1] - times[0]) if len(times) >= 2 else 0
    seconds = span / 1_000_000_000
    gaps = []
    for left, right, duration in zip(content, content[1:], intervals):
        if duration <= threshold:
            continue
        lo, hi = int(left["ts_ns"]), int(right["ts_ns"])
        window = sorted([e for e in events if int(e["ts_ns"]) <= hi + threshold
                         and int(e.get("end_ns", e["ts_ns"])) >= lo - threshold],
                        key=lambda e: (int(e["ts_ns"]), int(e.get("tid", 0))))
        label, reason = _classify(window, lo, hi, right["seq"], threshold,
                                  coverage, lost, clock_error_ns)
        gaps.append(dict(start_ns=lo, end_ns=hi, duration_ns=duration,
                         seq=right["seq"], **{"class": label}, reason=reason, events=window))
    summary = {}
    for label in ["S" + str(i) for i in range(1, 8)] + ["UNKNOWN"]:
        selected = [g for g in gaps if g["class"] == label]
        summary[label] = dict(count=len(selected), per_min=len(selected) * 60 / seconds if seconds else 0,
                              total_ms=sum(g["duration_ns"] for g in selected) / 1e6,
                              max_ms=max((g["duration_ns"] for g in selected), default=0) / 1e6,
                              **{f"gt{ms}_per_min": sum(g["duration_ns"] > ms*1e6 for g in selected)*60/seconds
                                 if seconds else 0 for ms in (80, 100, 200)})
    p99, p999 = _percentile(intervals, .99), _percentile(intervals, .999)
    metrics = dict(observed_s=seconds, new_content_fps=(len(content)-1)/seconds if seconds and content else 0,
                   repeat_per_s=sum(not b["new"] for b in broker)/seconds if seconds else 0,
                   low_1pct_fps=1e9/p99 if p99 else None, low_01pct_fps=1e9/p999 if p999 else None,
                   max_gap_ms=max(intervals, default=0)/1e6,
                   **{f"gt{ms}_per_min": sum(d > ms*1e6 for d in intervals)*60/seconds
                      if seconds else 0 for ms in (33, 50, 80, 100, 200)})
    return dict(gaps=gaps, summary=summary, metrics=metrics, lost=lost,
                classification="INFERRED", clock_error_ns=clock_error_ns)


def read_jsonl(path):
    """Read data only; malformed evidence is a failure, never silently dropped."""
    with pathlib.Path(path).open() as stream:
        return [json.loads(line) for line in stream if line.strip()]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--events", required=True)
    parser.add_argument("--broker", required=True)
    parser.add_argument("--guest", help="guest Present/ETW JSONL with qpc and other event fields")
    parser.add_argument("--calibration", help="per-boot QPC calibration JSON with error_ns")
    parser.add_argument("--threshold-ms", type=float, default=80)
    parser.add_argument("--coverage", action="store_true", help="assert all required collection gates passed")
    parser.add_argument("--lost", type=int, default=0)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    events = read_jsonl(args.events)
    error = None
    if args.guest:
        if not args.calibration:
            parser.error("--guest requires measured --calibration")
        calibration = json.loads(pathlib.Path(args.calibration).read_text())
        error = int(calibration["error_ns"])
        if error < 0:
            parser.error("negative calibration uncertainty")
        for row in read_jsonl(args.guest):
            row = dict(row)
            row["ts_ns"] = qpc_to_host(row.pop("qpc"), calibration)
            if "end_qpc" in row:
                row["end_ns"] = qpc_to_host(row.pop("end_qpc"), calibration)
            events.append(row)
    result = analyze(events, read_jsonl(args.broker), threshold_ms=args.threshold_ms,
                     coverage=args.coverage, lost=args.lost, clock_error_ns=error)
    pathlib.Path(args.out).write_text(json.dumps(result, indent=2) + "\n")
    for gap in result["gaps"]:
        print(f"MEASURED gap {gap['start_ns']}..{gap['end_ns']} {gap['duration_ns']/1e6:.3f} ms "
              f"seq={gap['seq']} INFERRED {gap['class']}: {gap['reason']}")
        for event in gap["events"]:
            print("  " + json.dumps(event, sort_keys=True))
    print("class count gaps/min total_ms max_ms >80/min >100/min >200/min")
    for label, row in result["summary"].items():
        print(label, row["count"], *(f"{row[k]:.3f}" for k in
              ("per_min", "total_ms", "max_ms", "gt80_per_min", "gt100_per_min", "gt200_per_min")))
    print("MEASURED", json.dumps(result["metrics"], sort_keys=True))


if __name__ == "__main__":
    main()
