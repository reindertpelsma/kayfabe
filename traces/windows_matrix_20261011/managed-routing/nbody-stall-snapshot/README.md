# Nbody bounded stall snapshot

**STATUS: RESEARCH, 2026-10-11.** Hardware pending. Product and harness72741e2b, production launcher, MPO off, unchanged nbody GUI workload and deadline. Only measurement flag KF3_PT_STALL_SNAPSHOT=1 differs from ordinary1341.

Hypothesis: snapshots identify a SW/deferred method on the faulting GR channel near its DEVICE fault. Falsifier: a valid late snapshot of that channel excludes such a method. The four-stall/32-free per-boot budgets may expire before the app; missing coverage is inconclusive. Existing bounds are12 GPFIFO entries, three segments and256 words per segment. No snapshot state authors GPU work or releases a wait. Logging may affect timing; preserve the ordinary TIMEOUT/Xid and do not substitute a diagnostic pass.

Queued after current coverage and existing native-event/direct-stream probes. Own1360..1364 only; reclaim stopped baseline-backed overlays, keep evidence.
