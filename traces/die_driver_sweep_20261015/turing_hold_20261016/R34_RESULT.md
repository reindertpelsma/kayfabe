# W3 original-depth R34 callback versus GPU-event experiment

**STATUS: ANSWERED, 2026-10-16 03:58 UTC. Both candidates fail the180second budget.**

One owned TU106 lease56180232, unprivileged arch0x160/impl6 verified. Host
595.91.07, Linux guest580.159.04/kernel6.8.0-142. Original13000 decoy rows,
180second deadline unchanged. Actual QEMU private source/binary/SHA/argv and
KF3 environment asserted by process receipts fast-53961.json and
fast-54562.json. Both use exactly KF3_PROF=1; its two clock reads per trap are
measurement overhead. Safe harness59a290a0f never sends SIGKILL or broad kills.

| Measured quantity | Callback ca0ad248 | GPU event5b523ed5 |
|---|---:|---:|
| R34 verdict | TIMEOUT/crash at180s | TIMEOUT/crash at180s,181s including cleanup |
| Last profiling heartbeat |175.068s|175.066s|
| Completed walks at heartbeat |45261|41849|
| Mean walk time, microseconds |667|1108|
| Mean GPU time, microseconds |408|387|
| Mean apply time, microseconds |181|193|
| Host calls |90491|83667|
| Raised IRQ count |45190|3|
| IRQ register writes |587092|500977|
| BAR0 writes |1415277|1267742|
| VA worker timeout count |271|42146|

These are last heartbeat snapshots, not full completed-arm totals. Neither
arm completed; raw total counts alone are not throughput comparisons. At
matched175second snapshots the event candidate completes about7.5% fewer
walks. BAR0 writes per completed walk are about31.27 versus30.29, IRQ register
writes per walk12.97 versus11.97. Removing the callback removes the large
raised-IRQ count, but leaves most guest register-write traffic and increases
walk latency. GR0 wakes46223 are observed in the callback log; no nonzero GR0
entry/wake is observed in the event log. Do not reinterpret absence as dropped
GPU work or filter real host nonstall events.

**Falsifier result:** H-W1's performance prediction is falsified for this
candidate: no R34 budget success or throughput improvement. The notification
sub-hypothesis is supported by the raised-IRQ reduction, and is insufficient
to explain the runtime wall. GPU-event polling's1ms pending sleep is a ranked
candidate for the larger walk latency, an inference rather than proven cause.
Native Linux R34 passes on this same lease as part of raw30/30; this guest
failure remains kayfabe's. No hardware-limit explanation is established.

Next bounded experiments, stated before any future run:

1. Measure a bounded shorter pending-event wait on the VA worker while
   preserving actual GPU-event success/error, storage lifetime and all host
   interrupts. Falsifier: no decrease in walk latency or original-depth R34
   time at unchanged180seconds. No unbounded spin/read or vCPU blocking.
2. Decode dominant0x160/0x180 writes from the public family register sources
   and compare guest/native behavior before authoring any optimization. The
   names remain unresolved in this profile. No guessed semantics or interrupt
   filtering. These two offsets dominate counts in both variants.
3. Attribute the approximately two host mapping calls per walk and nested
   VM-exit cost with existing profiling before changing batching/order. Preserve
   map-before-clear and hostile-guest admission. Falsifier: measured costs too
   small to account for the remaining wall.

Full Linux guest raw30/30/CUDA4/broker remain UNRUN and required before any
green performance or whole-cell claim. Event Windows580.88 regression passes
at sourceeb4dfad0 (same product5b plus QGA deadline fix):421.25s,11 rounds,
zeroSystem4101TDR, Code0/DWM/Explorer, CUDA3/D3D11+12 before+after and nonblank
screenshot receipts. Clean stop04:08:03; rental absence verified04:08:27. Code-only PR19 remains draft;
CPU3236pass/14existing ignored and native9/9+gate10 pass do not override this
negative hardware result. No lease extension or budget relaxation.

Evidence: guest-wave/turinghold_r34_ab.log, per-arm suite/serial/QEMU logs,
actual process receipts, and r34-last-snapshots.json. The orchestration
R34_AB_EXIT=0 means both measurements were collected, not that either passed.
