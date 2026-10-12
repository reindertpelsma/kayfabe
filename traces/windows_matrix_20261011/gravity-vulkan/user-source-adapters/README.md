# Live USERD declaration and owned command store views

STATUS: RESEARCH, 2026-10-12 10:47 UTC. GPU-free source adapters only.

Hypothesis: a channel can retain its generated-layout USERD handle/offset and
cross-check the physical descriptor against the exact live logical allocation;
command-store snapshots can prepare bounded owned views outside mapping guards
and copy without native calls. Falsifiers: reused/revoked memory passes a location
check, a copy arms a view, an over-budget request mutates native state, or a
failed release permits view reuse/replacement without confirmed cleanup.

MEASURED: the channel decoder preserves both explicit-zero and nonzero original
USERD declarations. The live logical descriptor supplies page base + adjustment
+ logical offset, with bounded nonblocking RAM topology validation. This is a
cross-check, not a persistent pin. Wrong locations, contention, topology change
and same-handle replacement are tested. Uses the existing audited public layout;
no new per-driver/per-die interface or constant.

Four store-cache tests cover cross-view copying, complete request preflight,
trusted copy/view budgets, failed native release, and partial mmap failure.
Cleanup failures retain view/cookie ownership and revoke readability before
replacement. Independent review found a native protection mismatch (read-only
RM node with writable mmap); corrected to map_read_only. Remaining source review
found no blocker. Full abi/rm/qemu/chan suite PASS (RM lib140, QEMU lib195).

Runtime caller is absent. Future ownership must retain this cache on cleanup
failure and bind it to the exact VM-owned store/bound. No hardware run or Windows
PASS is claimed here. Latest exact Linux hardware evidence remains 7f3f9fe16,
gates9/9 + gate10 + fast30/30. Gravity Kayfabe still FAIL/TIMEOUT; VFIO179/183s PASS.
