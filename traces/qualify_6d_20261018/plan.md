# Round 6D qualification plan

CODE READ: pin candidate 70da04ffaebdee3b51e5eca6aa1eb3919e92e80d; evidence branch codex/qualify-6d-20261023. The owner approved cleanly stopping run 4501 and restarting GDM if needed. Start 2026-10-22 22:29:08 UTC; six-hour deadline 2026-10-23 04:29:08 UTC. Stop qualification at the first red stage with evidence. No product changes or candidate-branch pushes are planned.

CODE READ: stage 1 requires hosted CI for this exact head. Its aarch64 job has already reported E0063 at two missing GrConfig.ce initializers. Preserve the check names and results, source sites and compiler diagnostic. Do not represent a build failure as an executed failing hardware test.

INFERRED H1-local: the exact candidate also fails the full GPU-free release command on the dev box due to the missing ce fields. FALSIFIER BEFORE local execution: that unchanged command builds and completes successfully. Independently record rustfmt and clippy results/delta; compile failures make a complete diagnostic delta unavailable.

INFERRED H2: native gates 9/9 plus gate 10 and the broker-backed Linux fast suite 30/30 work at the exact candidate. FALSIFIER BEFORE execution: any gate failure, unexpected per-arm Xid, missing intentional-control match, or failed broker oracle. Pending stage-1 clearance; never run a second VM.

INFERRED H3: stock Windows 580.88 remains stable on a fresh owner-disk CoW with the perf8 production flags. FALSIFIER BEFORE execution: any event 153/4101/TDR, reboot, failed 10-minute desktop hold, failed installed app/game oracle, non-updating console during colour controls, or broker cursor/input failure. Pending prior gates.

INFERRED H4: CE class at birth removes the Energy NoCeObject failure. FALSIFIER BEFORE execution: any Energy failure with NoCeObject; minimum four fresh trials, compare with the previous 6b 2/5 aggregate separately from its 1/4 fresh subset. SA-02 and CATIA/Creo x2 also require clean event/counter oracles. Pending prior gates.

INFERRED H5: matched framebuffer allocation explains a material part of the historical VFIO gap. FALSIFIER BEFORE execution: the matched-VRAM blender/CATIA/Creo/Energy arm does not improve the valid matched ratios beyond observed spread. The existing kf-prof doorbell-to-submit profile determines whether inline stage 2 warrants a new implementation; no speculative implementation or timer edits. Perf is separate and non-blocking, eligible only when correctness stages 1–6 are green or red with diagnosed causes. Unrun stages are not red-with-cause.

CODE READ: all GPU/VM work uses the bounded fastguest lock; baseline Xids are snapshotted and only per-arm deltas judged. All cargo uses the bounded cargo lock, nice 19, two jobs, own targets, disk guard and release mode. Final host state: no Q4 VM, NVIDIA bound, fresh-process cuInit 0, GDM active, ubuntu logged in. Evidence-only commits carry [skip ci], scoped claims new=0 and a secret/path scan; no merge or candidate-branch change.
