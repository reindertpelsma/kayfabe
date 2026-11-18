# V3 — sparse residency: why residency cases hang in the guest (root cause, GPU-free reproduction)

**STATUS: LIVE, 2026-11-18. Root cause found in the code, red GPU-free test committed, fix NOT in this change: it forks on an owner decision (§5). The hardware link from an INVALID host PTE to Xid 109 is INFERRED.**
Parent evidence: `traces/open_rows_guest_20261117/confirm/` (PR #479) and `docs/design/V3_VULKAN_CTS_ROW.md` §12
(tree branch). Pattern: `V3_REPLAY_TESTS.md`.

## 1. Symptom (MEASURED, one box, RTX 3090, driver 580.159.04)

`multisampled_image_sparse_residency.rgba16ui.samples_4` and
`texel_buffers.sparse_residency.storage_texel_buffer_read_size_2_24_r32ui` time out in the kf3 guest
with host `Xid 109 CTX SWITCH TIMEOUT` (channel 0x31). Their fully bound twins
(`multisampled_image_sparse_binding.rgba16ui.samples_4`, `..._size_2_16_r32ui`) pass in the same boot,
both before and after the fault. The host passes all four cases. The host dmesg slices of the failing
cases contain no Xid 31 (MMU fault) line.

## 2. Hypotheses, falsifiers, verdicts

Each falsifier was stated before the evidence was read. Sources: the full kf3 logs of box 2
(`C1/msbind_a` against `C1/msresid`, and `B1`), plus the code.

| id | hypothesis | falsifier | verdict |
|---|---|---|---|
| H-C | the guest issues a sparse-specific RM control or alloc that kf-rm refuses or ignores | identical RPC traffic in the passing and failing twins | **FALSIFIED.** Both twins issue the same 288 `RmAlloc` / 289 `Free`. The census differs in one id only: `0x2080a0d1` (5 in the passing twin, 599 in the failing one), which grows with the 300 s hang and so follows it rather than causing it. `0x20802a12` is refused 128 times per case in passing and failing cases alike. |
| H-E | a size or ceiling effect | the size-matched image pair also splits pass/fail | **FALSIFIED** (the image pair has the same format and sample count, and only one of them fails). |
| H-F | the walk/apply budget cuts the many 64 KiB holes, leaving stale PTEs | the failing case shows a cut, a supersede or an unreconciled walk | **NO SUPPORT.** `walks=3485/3485 superseded=0 unreconciled=0 clipped=0x0`. The only `REFUSED` lines are the carve-out leaf, which also appears in passing rows. |
| H-B/H-D | the host twin holds the wrong state where the guest has SPARSE PTEs | the host holds SPARSE at those VAs | **HELD, in the code** (§3). The guest's SPARSE PTE becomes an INVALID host PTE. |

The coordinator's refinement was checked as well: "the guest's own VA reservation carries
`NVOS32_ALLOC_FLAGS_SPARSE`, and kf drops it". **The request never reaches kf.** A GSP-client guest defaults to
`bSplitVasManagementServerClientRm = NV_TRUE` (`ogkm-580 src/nvidia/src/kernel/gpu/gpu_registry.c:171-182`),
which makes `NV50_MEMORY_VIRTUAL` skip the RPC (`src/nvidia/src/kernel/mem_mgr/virtual_mem.c:458-470`). The
equal `RmAlloc` counts above are consistent with this. The guest's RM sparsifies the range in its own page
tables, so the sparse state reaches kf only as PTE bytes (`VALID=0 VOL=1`, `kern_gmmu_gm200.c:46-70`). It is
not a map-time attribute of any verb kf serves.

## 3. Mechanism (code, cited)

1. The guest reserves the sparse resource's VA with `_SPARSE`. Its RM writes SPARSE PTEs and, on every
   unbind, restores them (`ogkm-580 src/nvidia/src/kernel/mem_mgr/gpu_vaspace.c:1577-1600`, `2279-2284`).
2. The walker **counts** a SPARSE PTE and publishes nothing for it (`cuda/walk/kf_walk.cu` `kf_pte_sparse`, which
   only increments `sparse_slots`). `kf-mem` never sees the slot.
3. The host twin's guest ranges are `NV50_MEMORY_VIRTUAL` reservations made with
   `FIXED_ADDRESS_ALLOCATE | LAZY | VIRTUAL` and **without `_SPARSE`**
   (`crates/kf-host/src/channel.rs` `NVOS32_RESERVE_FLAGS`). In such a block an unmapped VA is INVALID:
   "plain old unmap" (`gpu_vaspace.c:2286-2289`), against `nvos.h:1415-1419`.
4. So a shader that reads a non-resident texel gets zero on bare metal and on the host (which pass), and
   meets an INVALID PTE on the kf3 twin.

**INFERRED, open:** why the host shows a context-switch timeout (Xid 109) and not an MMU fault (Xid 31).
Other twin faults have shown Xid 31 (`V3_APP_MATRIX.md` row 4). A candidate is that the twin's TEX/GCC
faults are replayable in the instance block that host GSP builds (`FAULT_REPLAY_TEX`,
`kern_gmmu_gv100.c:373-376`) and that nobody replays or cancels them. That code is GSP firmware, outside
ogkm. The fix does not depend on the answer.

## 4. GPU-free reproduction (`crates/kf-mem/src/sim/sparse.rs`)

The host model gains `SimRm::host_pte(va)`, which returns `Mapped`, `Sparse` or `Invalid` according to the
ogkm rule above. It reads production's `kf_host::channel::NVOS32_RESERVE_FLAGS` rather than a copy. The
residency pattern (32 slots of 64 KiB, even slots bound as 64 KiB leaves, odd slots SPARSE, which gives no
walker run) is replayed through the real `apply_entry` path and `BatchedVas`, on all three paths.

| test | invariant | today |
|---|---|---|
| `s1_guest_sparse_slots_are_sparse_on_the_host` | **S1**: a guest-SPARSE VA is SPARSE on the host | **RED** (96 pages `Invalid`), `#[ignore]` KNOWN-RED |
| `s1_an_unbound_slot_returns_to_sparse_on_the_host` | S1 after an unbind back to sparse | **RED**, `#[ignore]` KNOWN-RED |
| `ratchet_guest_sparse_slots_are_invalid_on_the_host_today` | pins today's gap; bound slots are mapped exactly | green; a fix flips it |
| `s2_guest_invalid_vas_stay_invalid_on_the_host` | **S2**: a guest-INVALID VA stays INVALID, so it fails loudly (a forwarded RC; UVM demand paging is not serviced, `V3_BUILD.md` C) instead of reading zero | green |

Mutant (blanket fix: add `_SPARSE` to `NVOS32_RESERVE_FLAGS`): S1 turns green but the mutant is **killed** by
`s2_...` and `ratchet_...`. A blanket sparse reservation therefore cannot pass for the fix.

## 5. Fix design (DESIGN-ONLY; the fix forks on an owner decision)

**Update 2026-11-18 (coordinator decision): option B is implemented as an OPT-IN, default OFF.** The
`unmapped-reads-zero` device property (`kf_host::channel::GuestVaFill`, `guest_range_plan`; ABI 35;
`docs/howto/installing.md`) applies to the static guest ranges only. Its tests are in
`crates/kf-mem/src/sim/sparse.rs`:
- `on_s1_*` are green with the property on;
- `on_s2_consequence_*` pins the documented price (with it on, a guest-INVALID VA reads zero);
- with the property off, S2 and the ratchet are unchanged.

Mutants are in `scripts/ci/sparse_fill_mutants.py` (7, each killed by at least 2 named tests). Two wiring
lines have no GPU-free killer: the plan's flags reaching `reserve_va_flags`, and realize calling
`set_guest_va_fill`. The box run covers them (the `unmapped-reads-zero=on` log line and the verdict).
Option C is still open. Not run on hardware.

**Requirement.** The host must hold SPARSE exactly where the guest's PTE is SPARSE (S1), and INVALID
wherever the guest's PTE is INVALID (S2).

**The host verbs that exist (ogkm-580, read 2026-11-18).** An unprivileged host client can make a VA
SPARSE in exactly one way: by allocating it as a `_SPARSE` `NV50_MEMORY_VIRTUAL` block. That block reads
zero from the moment it is allocated, and an unmap inside it returns to sparse
(`gpu_vaspace.c:1577-1600`, `2279-2289`). The two other routes are closed to kf:
- `NV0080_CTRL_CMD_DMA_UPDATE_PDE_2 _SPARSE` is registered NON_PRIVILEGED, but its handler refuses any
  caller below `RS_PRIV_LEVEL_KERNEL` (`dma.c:301-323`).
- `VA_INTERNAL_LIMIT` routes host RM's own placements into the server-RM split window
  (`vaspace_api.c:370-384`), which is reserved, so it cannot replace the static reservations.

**The blocker.** kf's static guest reservations (`GUEST_VA_RANGES`, one `NV50` block each) already own
every guest VA. They must exist before host RM places anything (`channel.rs` §v3-gfx records the gfx7 and gfx8 runs that showed why).
A FIXED `NV50` allocation over an existing block is refused (`gpu_vaspace.c:1374-1386`; `sim.rs` models
this), and freeing or splitting the reservation unmaps every guest mapping inside it, which breaks the
guard that "an unchanged VA stays mapped at all times". **No unprivileged sequence produces SPARSE at
a sub-range of a live static reservation while leaving the rest of it INVALID.** S1 and S2 cannot both
hold with today's fence.

**The options**, each with its cost:

| option | S1 | S2 | cost / risk |
|---|---|---|---|
| **B: `_SPARSE` static reservations** (one flag in `NVOS32_RESERVE_FLAGS`) | green | **red** | Every guest-INVALID access reads zero instead of faulting. UVM demand paging and HMM (`V3_BUILD.md` C), which fail loudly today (a forwarded RC, CUDA 719), would become **silent wrong answers**, as would application bugs. No walker or ABI change. |
| **C: a different fence** (no static reservation over guest VA, so per-run `_SPARSE` micro reservations become possible) | green | green | Needs a new way to keep host RM's own per-channel context buffers out of guest VA. None was found in ogkm-580, and the two recorded failures (gfx7 collision, gfx8 >2^47 placement, `channel.rs`) are exactly what the fence prevents. Research, not a patch. |
| **K: known limitation** (today) | red | green | Sparse residency hangs the channel (Xid 109); the guest sees an RC. |

**Recommendation:** keep K as the default (loud failure) and do not ship B by default. A silent wrong
answer is worse than a hang, and it would hit managed-memory workloads that have nothing to do with
sparse. If the owner wants the sparse row to pass, B could come as a documented power-user switch scoped to
one VM. The cost to state for it: S2 fails for that VM. C stays open research.

**Staging, if C is ever found.** Stage 1 (GPU-free, Rust):
- a `KFWR_RF_SPARSE` run flag in the walk ABI (`kf_cuda::abi`, `KF_ABI_VERSION` bump), mirrored in
  `kf_walk.h`;
- `apply` places a SPARSE run as a `_SPARSE` micro reservation through a new `SpaceVerbs` verb;
- the sim models it, S1 turns green, and S2 stays green.

Stage 2 (kernel): `kf_walk.cu` emits the SPARSE run where it now only increments `sparse_slots`, then the
PTX is regenerated locally with `NVRTC_SO=<nvidia-cuda-nvrtc-cu12==12.2.140>/libnvrtc.so.12 python3
cuda/walk/make_ptx.py cuda/walk/kf_walk.cu cuda/walk/kf_walk.ptx compute_75`. `[measured 2026-11-18]`
Regenerating from the current source with that NVRTC is byte-identical to the committed PTX. No nvcc or GPU
is needed.

Hostile-guest bounds for the stages: SPARSE runs come out of the same per-walk run budget as MAP runs, so
there is no new unbounded emit. Each run costs one micro reservation, bounded by the slot pool. A SPARSE
host PTE maps no memory.

## 6. Hardware confirmation still needed

One run of `msresid` with a kf3 that holds SPARSE at the guest-SPARSE slots. Prediction: Pass and no Xid.
Falsifier: Xid 109 again, which would refute the mechanism. Under option B this is one flag (a control build of `next` against B, the image pair and the texel pair).
The run should come after the §5 decision; before that it would only repeat §1. A B build that still
hangs refutes the mechanism and moves the question to Xid 109 vs Xid 31.
