**STATUS: LIVE, GPU-free lane evidence, 2026-10-26.** Branch `codex/fx1-20261026`, base `6564dd597` (candidate 6d plus mirror). No hardware or guest run was authorized or performed. The directory date follows the overnight-lane evidence convention.

# Deferred TLB invalidate and backing lifetime

CODE READ at `6564dd597`: `batch.rs:2559-2566` calls `retired` immediately after a deferred unmap; `batch.rs:2769-2787` frees emptied batch handles. `GpuMirror::invalidate` directly calls `HostVas::invalidate` at `kf-qemu/src/mem.rs:1660-1662`, while the entry's explicit invalidate happens later in `kf-mem/src/apply.rs:1095`. Exact base excerpts are in [base-ordering.txt](base-ordering.txt).

CODE READ, open GPU kernel modules 595.84: an empty mapping-backref list skips auto-unmap in `src/nvidia/src/libraries/resserv/src/rs_client.c:1342-1395`. Memory destruction calls `memdescFree`/`memdescDestroy` (`src/nvidia/src/kernel/mem_mgr/mem.c:151-171`); sysmem release calls `osFreePages` (`src/nvidia/src/kernel/gpu/mem_mgr/mem_desc.c:1454-1463`). The optional cache eviction there is L2 eviction, not a replacement for the VA-space TLB invalidate (`mem_desc.c:2023-2040`). `NV2080_CTRL_CMD_DMA_INVALIDATE_TLB` explicitly serves deferred API updates (`ctrl2080dma.h:39-67`); its implementation requests `PTE_DOWNGRADE` (`gpu/mem_mgr/dma.c:864-904`).

INFERRED: if the descriptor's pin is the last pin, a VMM discard can return backing to the host before the stale GPU translation disappears. An IOMMU may fault instead; this lane models the no-IOMMU/last-pin case. It does not claim a hardware reproduction. CODE READ at `6564dd597`: the optional whole-RAM descriptor is not a prerequisite for `GpuMirror::map_batch` (`mem.rs:1481-1498`), so the batch cannot rely on its pin.

## Tests first

Pre-run hypotheses and falsifiers:

- H1: freeing an emptied batch before invalidate completion is observable, and an adversarial discard/reuse can make the stale translation leave old/new/fault. Falsifier: the original production ordering retains the pin through completion or the simulator cannot observe either failure.
- H2: a retry with no successful new rows can still owe resource retirement. Falsifier: an empty retry completes the pending retirement without an explicit pending-work hook.
- H3: pending reservation pins can be bounded by reservation rather than by repeated unmaps/retries. Falsifier: a second unmap or failed-invalidate retry needs a second retained pin for that same reservation.
- H4: the current inbox predicate blocks unrelated USER rings. Falsifier: a queued/running split or scoped statement for A leaves the current predicate clear for B.
- H5: a reservation pinned for a pending/in-flight invalidate is a temporary steer hold. Falsifier: the existing worker already retries both states instead of refusing them as `StillReserved`.

MEASURED in the 2026-10-26 runs summarized in [results.txt](results.txt):

| Stage | Passed | Failed | Meaning |
|---|---:|---:|---|
| Baseline full memory suite | 199 | 0 | 3 existing ignored tests |
| Original retirement EXPECTED RED | 1 | 6 | Negative control passes; original production ordering fails |
| Empty-retry EXPECTED RED | 0 | 2 | Pending resources require an otherwise-empty refresh to invalidate |
| Duplicate-pin EXPECTED RED | 0 | 2 | Intermediate fix retained 2 pins where 1 suffices |
| Temporary-steer-hold EXPECTED RED | 0 | 2 | Pending and in-flight invalidate pins require worker retry |
| Coalesced full memory suite | 217 | 0 | 3 existing ignored tests |
| Full channel suite | 678 | 0 | 4 existing OPEN model exclusions |
| Full QEMU suite | 440 | 0 | 4 new, explicitly parked scope questions |
| Proposed scoped gate, selected with `--ignored` | 0 | 4 | Production policy deliberately remains global |

The first two test-only commits are `47e12b9cc` and `ae59dc7f2`; the bounded-pin test-only commit is `8a4a1742a`, and the temporary-steer-hold test-only commit is `808158236`. Each is labelled EXPECTED RED. All Task 1 regressions are active default tests. The four Task 2 tests are deliberately OPEN and ignored because there is no scoped implementation or resolved dependency policy.

MEASURED in `green-mem-first.log` (2026-10-26; summary in `results.txt`): the initial fix's full run had seven failures. Five direct-call tests still expected immediate release or old free-call indices and were updated to assert the explicit completion boundary. The hostile-input fuzz test found the empty-refresh retirement hole, which received the two failing-first retry tests and a pending-invalidate hook. One existing lock-latency test also failed its wall-time assertion once; its thresholds were unchanged, and subsequent full memory runs passed. The cause of that one timing failure is unverified.

MEASURED in the final 2026-10-26 validation (`final-mem-qemu.log`, `reset-isolated.log` and `final-qemu-rerun.log`, summarized in `results.txt`): memory passed 217 tests. The first final QEMU run hit one unchanged reset-test timing assertion (`TimedOut(Idle)` versus `TimedOut(Release)` at a 50 ms deadline). That test passed unchanged in isolation, then the full default QEMU suite passed unchanged with 440 tests. `reset.rs` has no diff from the base; the scheduling-sensitivity hypothesis remains an inference, and its root cause is unverified. No timeout or threshold was changed. The final steer-aware run (`final-steer-mem-qemu.log`, summarized in `results.txt`) then passed all 657 memory/QEMU tests with 7 documented exclusions.

MEASURED in the 2026-10-26 targeted Clippy run (`validation.txt`): compilation succeeded with no new warning sites. Formatting, added-line length, diff whitespace and the claims gate also passed; no debt baseline was edited.

## Fix and model boundary

CODE READ in this lane's source snapshot ([hashes](source-sha256.json)): emptied deferred batch handles are queued; an invalidate takes a retirement snapshot before the host call and frees only that snapshot after success. Failure restores it. Deferred micro unmaps hold an existing reservation pin from before the host call until completion; repeated pending pins coalesce per reservation. Later retirements remain for the next invalidate. Host calls run outside ledger locks. `GpuMirror`, its target enum, and the simulator use this path. A pending-work hook handles empty retries, and drain invalidates even when a failed unmap left the batch booked instead of queued. A context steer that meets a reservation pin returns `Busy` through the existing worker retry path; it does not treat the pending/in-flight invalidate as a permanent reservation refusal. Once pins are gone, the existing reserved/free verdicts apply.

CODE READ in the same production source snapshot: no map/unmap sequence, GPU scheduling pause, timeout or vCPU operation was added. Existing unchanged-VA guards, ownership checks, hostile-input fuzzing, chunk bounds and lifecycle/model/property expectations remain active. Tests that call deferred APIs directly now complete the invalidate before asserting resource release.

INFERRED limits: the simulator models warmed stale translations, last batch pins, discarded/reused RAM, and explicit completion/refusal. It does not simulate actual DMA/IOMMU hardware, outstanding GPU transaction drainage, RM-family differences, or VMM balloon/discard behavior. Two direct early-free negative controls calibrate the model independently of the production fix.

## Mutation evidence

MEASURED, 2026-10-26: [all 15 mutants](mutations.md) compiled and were killed by at least two independent tests each. [mutations.json](mutations.json) preserves all killing names. The corpus includes concurrent retirement during an invalidate, refused/retried invalidates, failed unmaps that already acted, bounded duplicate pins and two simulator mutations. The original sources were restored before final validation.

## Walk gate: analysis only

CODE READ: [the design note](../../docs/design/V3_WALK_GATE_SCOPE_20261026.md) lists the turn-entry, publication, source-migration and Translated protections, anchors and I1–I8 obligations. It specifies trusted per-space/life identities, scoped pending counts, retained running-split targets, event-driven waiters and a conservative shared/unknown/global fallback. The four selected red tests use the current real inbox predicate; they are executable policy questions, not proof of an unimplemented scoped adapter.

INFERRED: distinct PDBs do not establish that user source mappings are independent, and the current root-statement ABI does not retain a subtree-dependency graph. Narrowing is parked for the scope/alias/relay-semantics decisions in that note. No gate behavior was changed.

Reproduce with a shared target directory and the lane's build lock, `nice -n 19`, `--release -j2`, and at least 4 GiB free. Default validation selects `kf-mem`, `kf-chan` and `kf-qemu` with `KAYFABE_NO_KVM=1`. Select `mem::walk_gate_tests` with `--ignored` only to inspect the four intentionally unsatisfied scope contracts. No hardware is needed for these tests.
