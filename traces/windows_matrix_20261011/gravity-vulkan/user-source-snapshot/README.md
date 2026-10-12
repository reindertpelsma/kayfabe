# Confirmed source snapshots for the Windows USER relay

STATUS: RESEARCH, 2026-10-12 10:36 UTC. GPU-free adapter; no runtime caller or Windows PASS claim.

Hypothesis: a bounded worker copy can retain the exact confirmed placement and
birth RAM topology through command publication without blocking a vCPU or
arming native views under placement/RAM guards. Falsifier: a changed placement,
revoked RAM generation, ambiguous RAM span, or contended guard yields an admitted
snapshot/publication; or native view preparation runs under either guard.

MEASURED: seven unit tests cover owned copies, holes/bounds, contention,
topology replacement, preparation outside guards, logged and unlogged row
changes, partial RAM aliases, mixed RAM/store copying, admission after return,
and fragmented-span budgets. The unlogged-row and partial-alias cases failed
before their fixes. Full kf-qemu/kf-rm/kf-chan/kf-abi suite passed (QEMU lib191,
RM lib138). Independent source review found no remaining adapter blocker.

The adapter re-resolves the complete bounded span after preparation and at
admission: RowsLog alone is insufficient because the existing steer path can
remove a placed row without committing a log entry. Admission holds placement
and RAM guards only across a bounded CPU publication callback. A private PUT
write is publication; the authored doorbell follows after releasing guards.

Runtime prerequisites remain: exact channel/mirror/USERD lifetimes, VA-manager
ordering, an owned bounded store-view adapter, channel birth, command submission,
and in-stream migration. This helper supplies none of those by itself. No GPU
job was run for this new source. Previous exact 7f3f9fe16 Linux gates9/9 + gate10
and fast30/30 evidence is in ../user-memory-access/linux-result.

At 10:36 UTC the trusted host was idle, DMA-FQ, lock free, stop marker absent,
and /var/lib had 117343203328 bytes free. Vast authenticated CLI returned [];
there were no unused instances to destroy. Personal Windows image preserved.
