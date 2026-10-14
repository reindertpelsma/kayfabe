# User-stream invalidate ordering and guarded USERD admission

STATUS: RESEARCH, 2026-10-12 11:05 UTC. GPU-free; no runtime caller.

Hypothesis and falsifier before the parser run: a guest TLB-invalidate packet
becomes an own-space software action. It is disproved if guest PDB bytes reach
native commands or later work issues before the VA-manager success result.

MEASURED: failing-first tests introduced the missing typed action. Independent
review then found replay modes being collapsed, SYSMEMBAR on the wrong side of
the action, and insufficient output reservation after expansion. New replay and
barrier-order tests failed before correction. Clean subset now requires A-C
before D, GPC_ENABLE/REPLAY_NONE/GLOBAL_ACK/all page levels/all scopes, valid PDB
shape/aperture and non-targeted target fields zero. Unsupported shapes refuse.
SYSMEMBAR follows the completed action. Checked five-times input reservation
covers expansion plus ordinary tails; immediate-tail test exceeds the old reserve.
Malformed suffixes roll back state. Ordered receipt test proves preceding HOST
receipts are required and later native publication stays blocked. Existing kernel
T-mode behavior is unchanged; its existing source constants/membar author are reused.

Primary contracts: NVIDIA open-gpu-kernel-modules595.84 clc56f.h115-193;
NVIDIA open-gpu-doc manuals/ampere/ga100/dev_pbdma.ref4161-4291. No captured
per-die/driver fact or interface was added. Independent re-review found no further
source blocker. All kf-abi/kf-rm/kf-qemu/kf-chan tests pass (RM lib141).

Lease.admit now retains exact graph/resource authority through a bounded CPU
callback. Required runtime order: graph/access -> try-locked channel metadata ->
source rows -> birth RAM topology -> private CPU writes/PUT; authored doorbell
after guards. No RM/wait/nested Access in callback. A private worker-state mutex
is permitted only if graph/RAM/metadata/drainer never acquire it and Busy unwinds.
Zero-byte topology validation alone is not continuous RAM authority: source and
birth RAM guards remain necessary through PUT. Failing-first and contention,
revocation/reuse tests pass; independent review cleared the stated seam.

OPEN: the new typed GLOBAL_ACK action has no executor. Before wiring its success,
prove equivalent native acknowledgement semantics; existing host control return
alone is insufficient. Public dma.c867-901 discards the lower invalidate status;
turing/kern_gmmu_tu102.c188-200 disables the membar-WAR helper on Ampere+.
This is an explicit backend review/measurement gate, not a completed action.

No new GPU test, Windows relay/migration, Gravity PASS, matrix or driver-sweep
claim. Latest hardware remains exact7f3f9fe16 Linux gates9/9+gate10/fast30/30.
Gravity Kayfabe FAIL/TIMEOUT; same-GPU VFIO179/183s PASS. Personal image retained.
