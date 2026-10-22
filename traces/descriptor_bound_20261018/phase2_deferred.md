# G1 Phase 2 — deferred prototype

**STATUS: RESEARCH — RESUMED, 2026-10-22.**

CODE READ: subsequent owner guidance authorizes Phase 2 and adds worker fairness,
wrap characterization and the corrected never-blocking inline boundary. The
current plan is `V3_USER_DESCRIPTOR_PHASE2_20261022.md`. The deferred checkpoint
below records the earlier stop; its validation limits remain until replaced by
new evidence.

MEASURED: Phase 2 started after Phase 1 was pushed and green, before the owner's
subsequent instruction to defer it pending hardware evidence. Work stopped on
that instruction. This branch preserves unfinished code, not an accepted fix.
Phase 1 draft PR #76 and its green branch are unchanged.

MEASURED: tests-only RED `469251ded` compiled and failed all seventeen new
production-Relay stream regressions on Phase 1 admission. With the prototype,
those seventeen tests passed (zero failed, thirty-six filtered out).

CODE READ: the prototype introduces bounded parser parts, continuation state,
receipt metadata and a proposed guarded publication adapter. INFERRED: it is
incomplete and needs review before further work. Direct receipt tests, the full
GPU-free suites, Phase 2 mutations and final clippy comparison were NOT DONE.
The new kf-qemu receipt/adapter code was not built or exercised. No Phase 2
security, integration, native execution or compatibility validation is claimed.

INFERRED: resume only if the owner authorizes it after hardware shows
Scan/Window descriptors above 917504 bytes. Phase 1 exposes that case as
`USER_RING_SCAN_BUDGET`. Catia-07 subtest 03 on the run-3440 configuration and
`pointer_large_entries > 0` remain hardware handoff requirements for Phase 1.
No GPU/VM was used by G1.
