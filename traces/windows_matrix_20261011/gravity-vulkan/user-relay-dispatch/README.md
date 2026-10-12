# Three-mode CPU dispatch prototype

**STATUS: RESEARCH, 2026-10-12.** Revision: containing commit, parent93dd13847.
No runtime caller, new Windows run or GravityMark PASS. Linux hardware remains
7f3f9fe16 gates9+10/fast30; no exact-hardware claim for this prototype.

Hypothesis stated before the runs: one companion atomic word can exclude CPU
publishers, preserve pending work and allocation identity, and prevent a fresh
worker handoff after opaque native publication. Falsifier: Loom finds overlapping
owners, stale metadata use, lost final work, reuse before cleanup, or admission
of fresh worker inspection over a prior native publication.

MEASURED: 13 dispatch tests PASS, including seven finite Loom programs compiling
the SAME implementation against modeled atomics. No preemption/permutation/time
cap; branch bound1000 and thread bound4; all programs finite. Full trap/core/abi/
rm/chan/qemu GPU-free suite PASS, exit0. See model-green.txt and gpu-free-summary.txt.
The model covers CPU atomics, not arbitrary program lengths or real GPU/MMIO
memory ordering, source views, integrated bitmap/eventfd delivery or native handoff.

Failing-first checks (deliberately broken variants, restored before full PASS):

1. Remove the PUBLISHED history refusal from finish_fresh_handoff: cutover model
   FAILS (Done versus NativeHandoffPending), history-red.txt.
2. Separately remove its BUSY exclusion: the same model FAILS after an in-flight
   publisher is admitted over a cleared fresh gate, publisher-red.txt.

Independent review found and fixed Direct contention that lacked a drain and
an unconditional Worker wake that could requeue idle work. Direct now has a
separate opaque-token drain, without inspection or mapping conversion. Added
models pin final-ring service, delayed callback claiming a new generation only
with post-claim metadata, and later PUT observed-or-durable-work. Re-review found
no remaining blocker in this unconnected CPU scope; runtime activation not cleared.

INFERRED: fresh selection at 5080 object-allocation admission may fit GravityMark
run1361, which allocates that object before its first recorded submission. This
has not been implemented/tested in a VM. Previously published opaque streams
remain NativeHandoffPending; no timeout or idle-cursor shortcut clears it.

See docs/design/V3_WINDOWS_USER_RING_RELAY_ARCHITECTURE_REVIEW_20261012.md for
public-source evidence and the rationale against v3 constraints.
