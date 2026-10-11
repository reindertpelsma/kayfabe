# Actual-demand walk-slot allocation

**STATUS: RESEARCH, 2026-10-11.** GPU-free correction validated; hardware pending.

Windows run1284 (product/harness72741e2b) reboots during torch_ai_bench after450s
and one GSPcycle. Before recovery, the page-table allocator refuses slot38:
walk need665758, 662528 current slot capacity, 998912 preferred slot growth,
1371648 free runs but largest free region983808. Exact subsequent diff demand
is not logged. INFERRED from the allocator's 1.5×/256-grain formula: the logged
growth implies diff demand at most665941, rounded requirement at most666112,
which fits983808. Split ticket7935 is refused and channel token0x80c dies. The bounded
sequence and full-log SHA are saved in
../managed-routing/remaining-apps/torch-ai-capacity-failure.txt. This establishes
an allocator defect before recovery, not the complete reboot cause. The owner
also observed the run1284 black screen; observation timestamp is not a fault timestamp.

Correction: retain the existing preferred growth when possible, otherwise
allocate the rounded actual request. Both attempts preserve the old region
until a disjoint replacement exists; CUDA copies and ordering are unchanged.
Pool sizes, per-space ceiling, work budgets, completion semantics and guest
test parameters are unchanged. No new constant or timeout releases stuck work.
This is allocation of the actual demand, not an increase to the VM GPU budget.

Regression tests construct a fragmented layout reproducing the observed free
counts and the greatest demand consistent with the logged preferred growth
(offsets and exact diff demand are constructed, not measured), verify acceptance,
disjoint source/destination, preservation of the other region, pool accounting
and complete teardown. A genuinely insufficient disjoint region must still
refuse without changing the source, hint or pool. Existing headroom/churn tests
also pass. GPU-free validation:421 tests PASS,3 ignored across kf-cuda, kf-mem
and kf-qemu; full output gpu-free-tests.log. Targeted file rustfmt check passes.

Hardware hypothesis: eliminating the false capacity refusal lets the original
AI workload complete without recovery. Falsifier: the same capacity failure,
another allocation refusal, guest reboot/recovery, or an original workload
check failing. Run exact-revision Linux gates9+10 and fast30 first, then the
unchanged Windows AI test. A distinct remaining fault disproves sufficiency;
do not erase run1284 or increase its deadline. No hardware pass claimed.

The queued3252 timer-default/manual campaign remains frozen; this correction
is not silently substituted into that campaign or the owner's manual session.

Exact product65530e6a is staged in trusted host /data/kf-capacity-host. Controller
session80736 runs run_after_manual.sh: waits for the original campaign PID209237
to finish and requires both MANUAL_SESSION_READY and MANUAL_SESSION_EXIT before
it acquires the GPU lock. It neither interrupts nor expires the owner VM. If
manual preparation fails, this follow-up also stops rather than bypassing it.
Then run_regression.sh performs Linux validation, followed by run_focus.sh,
original torch_ai_bench plus gears/vectorAdd/CUP8, runs1480..1484. Original20
production flags, diagnostics off, native timer enabled as in72741e2b. Preserve
all verdicts, including another failure. Source/runners are regeneratable from git.
