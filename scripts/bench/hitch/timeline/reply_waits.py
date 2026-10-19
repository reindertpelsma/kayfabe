# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Same-clock RPC/held-reply phases. Posting is not guest consumption."""
import collections

PHASE_FIELDS = {"rpc_arrive": "arrival_ns", "reply_held": "held_ns",
                "rows_applied": "rows_applied_ns", "act_resolved": "act_resolved_ns",
                "reply_posted": "reply_posted_ns"}
KINDS = set(PHASE_FIELDS) | {"reply_post_attempt", "rm_call", "guest_thread_wait"}


def _metadata(events, name):
    values = [e[name] for e in events if e.get(name) is not None]
    return values[0] if values and all(x == values[0] for x in values) else None


def _elapsed(start, end):
    return end-start if start is not None and end is not None and end >= start else None


def overlap(row, lo, hi, complete=False):
    """Exact closed overlap; open waits require coverage for an exposure lower bound."""
    start = row["arrival_ns"] if row["arrival_ns"] is not None else row["held_ns"]
    posted = row["reply_posted_ns"]
    if start is not None and posted is not None and posted >= start:
        ns = max(0, min(hi, posted) - max(lo, start))
        return dict(overlap_ns=ns, overlap_lower_bound_ns=ns, overlaps_stall=ns > 0)
    if posted is not None and posted <= lo:
        return dict(overlap_ns=0, overlap_lower_bound_ns=0, overlaps_stall=False)
    lower = max(0, hi-max(lo, start)) if complete and start is not None and posted is None else None
    return dict(overlap_ns=None, overlap_lower_bound_ns=lower,
                overlaps_stall=lower > 0 if lower is not None else None)


def inventory(events, lo, hi, *, coverage=False, lost=0, all_requests=False):
    """Pair only explicit RPC identities, scoped by device and driver generation.

    Existing untimestamped depth/count log lines cannot be paired here. An open
    interval stays right-censored, and unknown phases never become zero duration.
    """
    groups = collections.defaultdict(list)
    unidentified = 0
    for event in events:
        if event.get("kind") not in KINDS:
            continue
        if "rpc_id" not in event:
            unidentified += lo <= event["ts_ns"] < hi
            continue
        groups[(event.get("device"), event.get("generation"), event["rpc_id"])].append(event)
    requests = []
    complete = bool(coverage and not lost)
    for (device, generation, rpc_id), records in groups.items():
        phase_records = {kind: [e for e in records if e["kind"] == kind] for kind in PHASE_FIELDS}
        row = {field: phase_records[kind][0]["ts_ns"] if len(phase_records[kind]) == 1 else None
               for kind, field in PHASE_FIELDS.items()}
        duplicate = any(len(es) > 1 for es in phase_records.values())
        row.update(device=device, generation=generation, rpc_id=rpc_id,
                   right_censored=not phase_records["reply_posted"],
                   ambiguous_phases=duplicate, phase_events=records)
        for name in ("function", "hold_reason", "guest_client", "guest_object", "object_class",
                     "row_count", "page_count", "guest_waiter_pid", "guest_waiter_tid",
                     "guest_thread_wait_verified"):
            row[name] = _metadata(records, name)
        row["arrive_to_post_ns"] = _elapsed(row["arrival_ns"], row["reply_posted_ns"]) if not duplicate else None
        row["held_to_post_ns"] = _elapsed(row["held_ns"], row["reply_posted_ns"]) if not duplicate else None
        row["held_to_rows_ns"] = _elapsed(row["held_ns"], row["rows_applied_ns"]) if not duplicate else None
        row["rows_to_post_ns"] = _elapsed(row["rows_applied_ns"], row["reply_posted_ns"]) if not duplicate else None
        calls = [e for e in records if e["kind"] == "rm_call"]
        row["host_calls"] = calls
        row["host_call_ns"] = sum(e["end_ns"]-e["ts_ns"] for e in calls) if calls and all(
            "end_ns" in e and e["end_ns"] >= e["ts_ns"] for e in calls) else None
        row.update(overlap(row, lo, hi, complete and not duplicate and generation is not None))
        if not all_requests and row["overlaps_stall"] is False:
            continue
        if not all_requests and row["arrival_ns"] is not None and row["arrival_ns"] >= hi:
            continue
        requests.append(row)
    requests.sort(key=lambda r: (r["arrival_ns"] if r["arrival_ns"] is not None else hi,
                                 str(r["device"]), str(r["generation"]), str(r["rpc_id"])))
    return dict(requests=requests, observed_count=len(requests), unidentified_events=unidentified,
                complete=bool(complete and not unidentified and all(r["generation"] is not None
                              and not r["ambiguous_phases"] for r in requests)))
