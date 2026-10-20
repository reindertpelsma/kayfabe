# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""S9 describes pre-TDR degradation; it never substitutes for causal S1..S8."""
from bisect import bisect_left

SEC = 1_000_000_000


def _count(times, lo, hi):
    return max(0, bisect_left(times, hi) - bisect_left(times, lo))


def _measure(times, lo, hi, complete, margin):
    observed = _count(times, lo, hi)
    return dict(observed_count=observed,
                count_lower=_count(times, lo+margin, hi-margin) if complete else None,
                count_upper=_count(times, lo-margin, hi+margin) if complete else None,
                per_s=observed*SEC/(hi-lo) if complete else None)


def _active_covers(events, pid, lo, hi):
    intervals = sorted((e["ts_ns"], e["end_ns"]) for e in events
                       if e.get("kind") == "workload_active" and e.get("pid") == pid
                       and "end_ns" in e and e["end_ns"] >= e["ts_ns"])
    cursor = lo
    for start, end in intervals:
        if start > cursor:
            break
        cursor = max(cursor, end)
        if cursor >= hi:
            return True
    return False


def pre_tdr_windows(events, broker, gaps, coverage, lost, clock_error_ns, threshold_ns,
                    vm_start_ns):
    """Retain every supplied TDR and ten half-open one-second observation bins.

    guest_tdr is an H1 normalized observation, never parsed from Event 153 alone.
    Guest exposure and active-workload intervals must be explicitly validated.
    Two calibration errors conservatively bound relative guest/TDR boundaries.
    Nominal counts remain observations; only covered rates can be measured zero.
    """
    windows = []
    guest_by_pid = {}
    for e in events:
        if e.get("kind") == "guest_present":
            guest_by_pid.setdefault(e.get("pid"), []).append(e["ts_ns"])
    for times in guest_by_pid.values():
        times.sort()
    broker_times = [b["ts_ns"] for b in broker if b["new"]]
    clock_ok = (type(clock_error_ns) is int and 0 <= 2*clock_error_ns < threshold_ns)
    margin = 2*clock_error_ns if clock_ok else 0
    for tdr in sorted((e for e in events if e.get("kind") == "guest_tdr"),
                      key=lambda e: e["ts_ns"]):
        hi = tdr["ts_ns"]
        lo = hi - 10*SEC
        pid = tdr.get("pid")
        pid_ok = type(pid) is int and pid > 0
        times = guest_by_pid.get(pid, []) if pid_ok else []
        capture_start, capture_end = coverage.get("guest_start_ns"), coverage.get("guest_end_ns")
        guest_complete = (coverage.get("guest") is True and not lost and clock_ok and pid_ok
                          and type(capture_start) is int and type(capture_end) is int
                          and capture_start <= lo-margin and capture_end >= hi+margin)
        broker_complete = (coverage.get("broker") is True and not lost and clock_ok
                           and len(broker) >= 2 and broker[0]["ts_ns"] <= lo-margin
                           and broker[-1]["ts_ns"] >= hi+margin)
        active = pid_ok and _active_covers(events, pid, lo-margin, hi+margin)
        flags = dict(guest_exposure=guest_complete, broker_exposure=broker_complete,
                     clock=clock_ok, pid=pid_ok, active=active,
                     verified_tdr=tdr.get("tdr_verified") is True)
        early = _measure(times, lo, lo+2*SEC, guest_complete, margin)
        late = _measure(times, hi-2*SEC, hi, guest_complete, margin)
        status = "UNKNOWN"
        if guest_complete and active and flags["verified_tdr"]:
            if early["count_lower"] > late["count_upper"]:
                status = "DECLINE_OBSERVED"
            elif early["count_upper"] > 0 and late["count_lower"] >= early["count_upper"]:
                status = "NO_DECLINE_OBSERVED"
        bins = []
        for index in range(10):
            start, end = lo+index*SEC, lo+(index+1)*SEC
            guest = _measure(times, start, end, guest_complete, margin)
            host = _measure(broker_times, start, end, broker_complete, margin)
            bins.append(dict(start_ns=start, end_ns=end,
                             guest_present_count=guest["observed_count"],
                             guest_present_count_lower=guest["count_lower"],
                             guest_present_count_upper=guest["count_upper"],
                             guest_present_per_s=guest["per_s"],
                             broker_new_count=host["observed_count"],
                             broker_new_per_s=host["per_s"]))
        gap_indices = [i for i, g in enumerate(gaps) if g["start_ns"] < hi and g["end_ns"] > lo]
        windows.append(dict(phenomenon="S9", status=status, tdr_event=dict(tdr),
                            start_ns=lo, end_ns=hi, pid=pid,
                            vm_uptime_ns=hi-vm_start_ns if vm_start_ns is not None else None,
                            start_uptime_ns=lo-vm_start_ns if vm_start_ns is not None else None,
                            clock_error_ns=clock_error_ns, boundary_error_ns=margin if clock_ok else None,
                            coverage=flags, early=early, late=late, bins=bins,
                            gap_indices=gap_indices, causal_class="UNKNOWN"))
    return windows
