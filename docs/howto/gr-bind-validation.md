# Validate USER GR context binds

The experimental USER GR bind adapter is disabled by default. Set `KF3_GR_BINDS=1` in the VMM environment for a scheduled validation run. Unset it or set `KF3_GR_BINDS=0` to retain the existing path. No boot flag is required.

Use `KF3_TSG_FACTS=1` alongside it to collect native mode readbacks. The regular heartbeat appends:

```text
gr_binds[served=... refused=... bytes=...]
```

These are cumulative adapter outcomes. `served` counts successful native binds; `refused` counts adapter admission/native failures and queued cancellations. `bytes` sums the validated buffer extents of successful binds, including repeated binds to the same VA. It is neither allocated memory nor unique resident memory. Malformed controls rejected before adapter admission are outside these counters. The fields are absent when the adapter is disabled.

A registered deferred bundle has no host effect until its stream triggers it. Completion waits for the VA worker's real result. A nonzero `served` count proves that a bind returned success; it does not prove a particular effective preemption mode or a latency improvement. Keep the corresponding readback and native status.

The first implementation forwards only authored binds to owned USER GR contexts over unchanged, already mapped writable guest VAs. It refuses unknown native extents, unsupported layouts, invalid ranges, CPU pool-control pointers, stale targets and ambiguous multi-member TSG targets. It creates no replacement buffer. Ordinary Linux direct zcull retains its existing adapter.

Hardware validation is pending. Use the matched Linux contention and Windows workload plan in [the design](../design/V3_GR_PREEMPT_BINDS.md) before considering default enablement. Preserve bind/readback statuses, heartbeat timestamps, guest timeout events and host Xids with each arm.
