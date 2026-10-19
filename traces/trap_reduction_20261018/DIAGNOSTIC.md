# Prepared interrupt-state diagnostic (not run on hardware)

STATUS: RESEARCH, 2026-10-19.

CODE READ: compile `kf-qemu` with `--features diag-intr-audit`, then opt in at realize with
`KF3_DIAG_INTR_AUDIT=1 KF3_DIAG_INTR_AUDIT_REQUEST=$OUT/intr`.
Neither compile nor runtime switch is needed to boot. Both default off. Ordinary builds contain
no audit field/calls. The optional runtime-on recorder allocates at realize; the vCPU and
announcing threads use only a fixed atomic buffer. No changed interrupt semantics or delivery.

CODE READ: the existing drainer diagnostic service point checks `$OUT/intr.arm`; creating that
host-authored request arms one capture of at most 256 writes/latches. Create `$OUT/intr.dump`
after the probe; the drainer stops new reservations and writes `$OUT/intr.out`. Use a new prefix
and fresh device for every capture. The lane did not create these files or run this diagnostic.
Output, allocation, request checks and environment parsing never run on a vCPU.

CODE READ: each sample carries the written offset/value or announced vector, 16 pending words,
16 leaf-enable words and top enable before/after, actual Raise, begin index and finish order.
`full`, `contended` and `IN_FLIGHT` explicitly limit completeness. Producers make one reservation
attempt; they drop diagnostic samples on contention/full capacity without affecting interrupts.
The snapshots are independent atomic observations, NOT a multiword transaction; begin/end
ordering exposes only some overlap. Even a loss-free dump cannot prove a physical-edge census
or linearizability when operations overlap. Use serial isolated fixtures for exact pending-to-enable
classification and retain concurrent intervals as unknown. Paired BAR0 read tracing is separately
needed to observe the guest's read order. Do not use this instrumented arm for performance claims.

MEASURED: the first diagnostic API test commit failed to compile with E0432 (missing API),
explicitly a feature implementation gate, not proof of a production interrupt bug.
`AUDIT_MUTATIONS.json` records assertion-failing restored-source mutants after implementation.
