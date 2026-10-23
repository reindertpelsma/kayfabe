# SA-06 CPU window placement admission

INFERRED hypothesis before runs: a shared RAM/store placement budget can refuse
new host-resource acquisition while preserving live views and returning capacity
on successful unmap. FALSIFIERS: a placement or host verb beyond the cap, a RAM
bypass, eviction, lost existing mapping, or capacity not returned after unmap.

MEASURED tests-first history: d763bb7f7 imports the security audit unchanged;
its200000-placement assertion fails on master. Four additional recorder tests
also fail by assertion before implementation. Store-only, RAM-only and mixed
placements share8192 slots, and successful unmap permits one replacement. (recorded in commit 2950d3736, 2026-10-22)

CODE READ: admission follows existing geometry/duplicate checks and precedes
both arm_store and place_ram. Refusal uses the existing named-error/stat path;
there is no eviction, reaping, new flag, timeout or poll. The boot-display seed
is separate (at most one per window);8192 is a security resource budget, not a
symptom/timing adjustment. The PR does not alter existing failed-unmap/release
semantics or claim that quota exhaustion completes an invalidate.

MEASURED whole kf-mem release suite191 passed /0 failed /3 existing ignored debug
replays. Store-cap bypass mutant4 killing tests; RAM-cap bypass2. Restored focused
regressions5 passed. Clippy all targets136 ->136 source fingerprints, added0.
No tests, deny sets, lint baselines or existing assertions loosened. Every Cargo
run held the owner's shared lock, used release/two jobs/nice19 and own target. (recorded in commit 2950d3736, 2026-10-22)

INFERRED hardware validation later: normal Windows/Linux BAR fragmentation stays
within the quota, real fd/VMA use and map/unmap churn. This is a per-window live
placement bound (two windows plus seeds); unrelated process resources and host
release failures are not covered. No actual host mmap, GPU or VM was used.
