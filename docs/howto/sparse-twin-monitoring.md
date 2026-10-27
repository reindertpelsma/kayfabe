# Monitor sparse-twin coverage

The selective sparse twin remains disabled by default. Use `KF3_SPARSE_TWIN=1` only in the scheduled validation configuration. This change adds no required boot flag.

The existing `fb-mb` setting now determines the finite reservation-object budget. An8GiB guest framebuffer gives65536 objects per space and131072 across the device's host session. Smaller framebuffers scale down to a floor of8192; larger framebuffers cannot raise the maximum. Configure framebuffer size before starting the device; the policy is fixed for its lifetime.

Read these fields in the regular heartbeat:

- `budget_space` and `budget_vm`: the active hard limits.
- `live_blocks` and `sparse_space_blocks`: current object usage and the busiest per-space rows.
- `current_unhonoured_bytes`: unique guest-sparse VA still lacking native sparse coverage. Successful retry and retirement reduce it.
- `missing_capped`: when true, the current byte value is a conservative upper bound because interval tracking filled. Treat coverage as incomplete.
- `withheld_windows`: permanent excluded windows encountered, counted once per window per logical space life.

The existing `unhonoured`/`unhonoured_bytes` counters are cumulative. They do not describe current missing coverage.

A `SPARSE_UNHONOURED` line reports the native space, current missing bytes, cap flag and first four ranges. Warnings begin at1MiB of missing coverage, have a30s per-space cooldown and are capped at32 over the session. Keep this line and nearby heartbeat timestamps when reporting a stall. Absence of further warning lines after the print cap does not establish full coverage; keep reading the heartbeat gauge.

The RM low window `[1MiB,4.5GiB)` and the host-placement hole below1TiB cannot receive sparse reservations. Declines there install no replacement mapping. An otherwise invalid VA still faults; a counted decline is not equivalent to successful sparse coverage.

For the policy rationale, measured native object costs and model limits, see [the ENS7 design](../design/V3_ENS7_SPARSE_BUDGET.md).


## Walk holds

CODE READ in `crates/kf-qemu/src/mem/walk_scope.rs` (ENS10, 2026-10-27): relayed USER rings can continue reading and publishing while another host VA-space target is walked. Their own pending changes and splits still hold future publication; unknown/global operations retain the conservative hold. No configuration flag is required. Already-published fetch and marker observations continue before the source gate.

The `WALK_GATE` heartbeat separates `own`, `split` and `global` hold counts, cumulative microseconds and maximum observed duration. `other_allowed` counts checks admitted during other-space work. `ref_changes_walk` counts changed reference-word samples while the aggregate walk state is busy, including guest SET_REFERENCE values; it is not a packet-completion count. `completion_wb_busy_walk` and `completion_wb_busy_max_us` describe guest-GET writeback admission contention during a busy walk. They do not timestamp the GPU's finish or prove that a walk caused the contention. A hold closes in these counters when the relay probes again, resumes or retires.

Long same-space or global holds can still occur. Keep the host log and matching guest trace when reporting a stall; the counters alone do not establish Windows interrupt delivery or explain a TDR. ENS10 does not impose a hard wall-time bound on a GPU walk or host RM call.
