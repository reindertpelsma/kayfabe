# V3 — sparse residency: why residency cases hang in the guest (root cause, GPU-free reproduction)

**STATUS: LIVE, 2026-11-18. Root cause found in the code and CONFIRMED on hardware (§6, one RTX 4090 box: off, the residency cases hang with Xid 109; with option B on, they pass). Option B ships as an opt-in property (#494). Option C (the exact fix): C1 is MEASURED infeasible. C2's host costs are MEASURED (§8.1, one RTX 3090 box, bare host). Its design is in §8, and stage 1 (the GPU-free engine and its proof) is built: S1 and S2 are both green in the model. C2 is not wired into apply and not run with a guest. The hardware link from an INVALID host PTE to Xid 109 is INFERRED.**
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

**Update 2026-11-18, later (coordinator decision; stacked PR `claude/sparse-zero-default-20261118`): the
default is now `auto`.** `GuestVaFillPolicy::ByProfile` gives ZERO to the spaces of a Windows-profile guest
(the floor-started spaces) and FAULT to every other guest; `on` and `off` override it for every space. ABI 36.
- Linux keeps S2 by default: MEASURED (the Linux guests above).
- Windows gets S1 by default. That sparse reads work there, and that the silent-zero hazard is smaller
  there (no demand-paged UVM/HMM on WDDM), is INFERRED: no Windows sparse run exists yet.

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

**Update 2026-11-18 (later): this staging is superseded by §8.7.** Stage 1 there is the tiling engine
(`crates/kf-mem/src/tiling.rs`), not the ABI flag. The walker ABI moved to stage 2, and the wiring to stage 3.
What follows is the original text.

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

## 6. Hardware confirmation

**Update 2026-11-18 (MEASURED, box `kf-openguest-180343`, RTX 4090 (Ada), host and guest driver 580.159.04,
kf3 `5ee2aa5a` = `claude/sparse-box-tree-20261118`).** The tree is the accept tree plus #494, with one binary
for both boots. Each case ran alone in its own `deqp-vk` process. The host ran the same list as the oracle.
Text evidence: `/workspace/bench/sparse_box_20261118/box2/raw` (coordinator side).

The prediction and the falsifier were stated before the run:
- off: the two residency cases time out with Xid 109;
- on: everything passes;
- Xid 109 with the property on falsifies the mechanism.

| case (`dEQP-VK.sparse_resources.`) | host | guest, property off | guest, `unmapped-reads-zero=on` |
|---|---|---|---|
| `multisampled_image_sparse_binding.rgba16ui.samples_4` | Pass | Pass | Pass |
| `multisampled_image_sparse_residency.rgba16ui.samples_4` | Pass | **Timeout (300 s), 1 host Xid 109** | **Pass (13 s), 0 Xid** |
| `buffer.texel_buffers.sparse_residency..._size_2_16_r32ui` | Pass | Pass | Pass |
| `buffer.texel_buffers.sparse_residency..._size_2_24_r32ui` | Pass | **Timeout (300 s), 1 host Xid 109** | **Pass (14 s), 0 Xid** |
| `mipmap_sparse_residency.2d.rgba16ui.1024_128_1` | Pass | Pass | Pass |
| `mipmap_sparse_residency.cube.r8_snorm.128_128_1` | Pass | Pass | Pass |
| `image_sparse_residency.mutable.2d.r16g16_unorm_r16g16_snorm_r16g16_uint` | Pass | Pass | Pass |
| `device_group_image_sparse_residency.2d.rgba16ui.1024_128_1` | NotSupported | NotSupported | NotSupported |
| `shader_intrinsics.2d_sparse_sample_implicit_lod.g16_b16r16_2plane_444_unorm.128_128_1` | NotSupported | NotSupported | NotSupported |
| `image_rebind.2d_array.r8.128_128_8` | NotSupported | NotSupported | NotSupported |
| `image_sparse_binding.multiple_bind_sparse_info.3d.rgba32ui.512_256_6` | Pass | Pass | Pass |
| `image_sparse_memory_aliasing.2d.rgba32ui.11_37_1` | Pass | Pass | Pass |

- Both Xid 109 were `CTX SWITCH TIMEOUT` on channel 0x31 with `Info 0x6c022`, as on the RTX 3090. They were
  the only two Xid lines of the whole box.
- **Verdict: the mechanism holds, and the falsifier was not observed.** An INVALID host PTE under the guest's
  SPARSE pages hangs the channel. A SPARSE host PTE there makes the cases pass. Option C (exact S1 with S2
  kept) is therefore worth building for this symptom.
- The first box rented for this run (an RTX 4090) wedged on bare metal before any kayfabe code ran: RC
  watchdog `GPPut < WATCHDOG_GPFIFO_ENTRIES` asserts, then a host CUDA ladder binary stuck in the kernel.
  It was destroyed and replaced.

**Still open, non-blocking:** why an INVALID read stalls the context (Xid 109) instead of faulting (Xid 31).
A candidate (INFERRED): the twin's TEX faults are replayable in the instance block that host GSP builds, and
nobody services them. The fix does not depend on the answer.

(Superseded 2026-11-18 by the update above:)

One run of `msresid` with a kf3 that holds SPARSE at the guest-SPARSE slots. Prediction: Pass and no Xid.
Falsifier: Xid 109 again, which would refute the mechanism. Under option B this is one flag (a control build of `next` against B, the image pair and the texel pair).
The run should come after the §5 decision; before that it would only repeat §1. A B build that still
hangs refutes the mechanism and moves the question to Xid 109 vs Xid 31.

## 7. Option C research (2026-11-18, time-boxed): SPARSE for exactly the guest's sparse runs

**Invariants.** S1: every guest-SPARSE VA is SPARSE on the host. S2: every guest-INVALID VA is INVALID on
the host. Every guest-mapped VA stays mapped as today, including the guard ("an unchanged VA stays
mapped at all times").

**What blocks it** (§5): host RM can make a VA SPARSE only by allocating a `_SPARSE` `NV50` block there. kf's
static fence blocks already own the guest VA, and an `NV50` block can be neither split nor nested.

### C1: drop the fence (`VA_INTERNAL_LIMIT`)

If host RM's own per-channel context buffers could be kept out of the guest ranges some other way, there
would be no fence. A guest-SPARSE run would then be one exact `_SPARSE` `NV50` block, and a guest row would
be placed as today (micro reservations, `batch.rs`).

The one unprivileged lever is `NV_VASPACE_ALLOCATION_FLAGS_VA_INTERNAL_LIMIT`. It routes RM-internal
allocations into `[4 GiB, 4.5 GiB)` (`vaspace_api.c:370-374`), a window that every GSP-client guest RM also
withholds from its own clients, so no guest row ever lands there. On a GSP host, however, client RM
reserves exactly that window for server RM (`gpu_vaspace.c:340-344`, `_gvaspaceReserveVaForClientRm`).

- Code-reading prediction (INFERRED): GR context creation fails with the flag.
- Probe: `crates/kf-harness/src/bin/kf-va-fence-probe.rs`, on the sparse box after the guest runs. It runs
  two arms: `default` (the positive control: host RM's buffers should occupy guest-range chunks) and
  `internal_limit`.
- Falsifier of the prediction: `internal_limit` builds the compute object and the whole first guest range
  reserves cleanly, while the `default` arm shows occupied chunks.

### C2: exact block tiling

Keep a fence, but tile the guest ranges so that every byte belongs to exactly one host block:
- a **fence** block: empty, INVALID;
- a **row** block: a guest mapping placed through it;
- a **sparse** block: `_SPARSE`.

A fence block never holds a mapping, so it can be freed and re-reserved around a new row or sparse run
without unmapping anything. That gives S1 and S2 exactly, at the guest's own granularity, with
unprivileged verbs only.

**Update 2026-11-18 (later): the costs below are now MEASURED (§8.1), and the design that answers them is in §8.**
Summary of what changed: a carve costs exactly 5 RM calls. Per-row tiling costs 3.9x to 5.5x the map cost of
the static fence (up to 16000 blocks). The eheap is linear, but no ceiling appeared below 64000 blocks. The
race is real (§8.1, M3). A `_SPARSE` block cannot be smaller than 64 KiB.

The costs and unknowns (as first written, before the measurements):
- **RM calls.** A map or unmap in a fenced gap costs about 5 RM calls instead of 1: free the fence, two
  fence remnants, the block, the map. The micro-reservation path already pays a similar cost per batch,
  not per row.
- **Allocator cost.** RM's VA heap is an `eheap` whose block lookup is a list search
  (`eheap_old.c:1005-1027`). Thousands of blocks per space make every host map slower, so this needs a
  host-side measurement before any design commitment.
- **A race.** Between freeing a fence and re-reserving it, a host-RM placement in the same space (a
  twin's GR context, created on an act thread) could land in the gap. The carve and every host object
  allocation in that space need one kf-owned lock. That is allowed: no vCPU takes it (CLAUDE.md rule).
- **Hostile-guest bounds.** Blocks per space are at most 2 × (rows + sparse runs) + 1, bounded by the
  space's slot capacity (the walk pool). SPARSE runs come out of the same budget. RM call rate is bounded
  by the invalidate rate, as today. All arithmetic is in checked `u64`.
- **Walker.** The walker must emit SPARSE runs (stage 2 of §5: an ABI field and the PTX, which is
  regenerable locally). VER3 PDE sparse must be included (`kf_pde_sparse`).

### C1 probe result (MEASURED 2026-11-18, same box, bare host after the guest runs, 0 new Xid)

- Arm `default` (positive control): space, GR channel, compute object `0xc9c0` and schedule all OK. A FIXED
  reservation of the whole first guest range is refused (`NoMemory`). Host RM's own buffers occupy
  `0x120000000` (exactly 4.5 GiB), `0x120200000` and `0x124000000`-`0x124a00000`. This is the gfx7
  collision, seen directly by the probe (`kf-va-fence-probe` on box `kf-openguest-180343`).
- Arm `internal_limit` (flags `0x80`): space and channel OK, **compute object refused (`NoMemory`)**, as
  predicted.

**C1 is infeasible:** with the flag, host RM cannot build a GR context at all.

### Verdict

C1 is infeasible (box `kf-openguest-180343`, kf3 `5ee2aa5a`, probe output above). C2 is feasible with unprivileged verbs only (no NVIDIA-side change), but it is a
redesign of how `kf-mem` places guest rows, whose per-map cost is unknown until a host-side test exists.
DX12 reserved/tiled resources and UE virtual textures on Windows need the same S1+S2 property, so C (not
B) is the real fix.

## 8. Option C2: measurements and design (2026-11-18)

### 8.1 Host measurements (MEASURED, `kf-va-tiling-probe`, box `kf-openguest-180543`, RTX 3090, 580.159.04, bare host)

Evidence: `traces/sparse_c2_tiling_probe_20261118/` (probe revision `62b063433`, then `c07bd23dc` for the row
sweep). The box showed 0 Xid lines, before and after every arm. Range `L` = 4.5 GiB, span 64 GiB. Maps are
64 KiB device-local, FIXED, with deferred TLB invalidation.

**Falsifier, stated before the run:** if M2 grows linearly to more than about 1 ms per call at 16k blocks, or
M1 costs more than about 10x M0, then C2 needs coarser tiling (one row block per walk batch, not one per
row). **Result: the falsifier was not triggered.**

| arm | result |
|---|---|
| M0: one static block, N maps and N unmaps through it | 25.8 us per map, 21.2 us per unmap (p50 24.4 / 20.6). Flat from N = 1000 to 8000. |
| M1: exact tiling, eager merge | **5.00 RM calls per map and per unmap.** Map mean: 102 us at 2000 blocks (3.9x M0), 111 us at 4096 (4.3x), 143 us at 16000 (5.5x). Unmap mean: 106 us, 124 us (5.7x), 219 us (10.3x). |
| M1d: the same, default attr everywhere | **The first carve is refused (`NoMemory`).** See M5. |
| M2: K disjoint 64 KiB blocks | Reserve and free are **O(K)** in the blocks below the address, about 13 ns per block. Worst reserve: 66 us at K = 4k, 215 us at 16k, 420 us at 32k, 850 us at 64k. Worst free: 54 us at 4k, 165 us at 16k, 790 us at 64k. Map and unmap through an existing block stay flat at 35-39 us up to 32k. **No refusal up to 64000 blocks** (no per-client ceiling was reached). |
| M3: a second thread places non-FIXED maps during M1 | **The race is real.** On the second carve, a placement landed at `0x120010000`, inside the fence just freed, and the re-reserve of the remnant was refused. The window (fence freed until the last remnant is reserved) is 55-65 us at p50, 84-197 us at p99, and 539 us at most (16k blocks). |
| M4: `_SPARSE` micro blocks | 64 KiB and 2 MiB `_SPARSE` blocks work next to a fence, next to each other and next to a mapped row block. A map through a sparse block works, and so does its unmap. **A 4 KiB `_SPARSE` block is refused (`NoMemory`), even with `_ATTR_PAGE_SIZE_4KB` + `_ALIGNMENT_FORCE`.** |
| M5: alignment | A default-attr block of 4 MiB at a 64 KiB edge is **rounded outward to 2 MiB** (the 64 KiB below it is then taken). The same block with `_ATTR_PAGE_SIZE_BIG` + `_ALIGNMENT_FORCE` (alignment 64 KiB) is **exact**, `_SPARSE` or not. A 192 KiB default block is exact. A 4 KiB map inside a BIG-attr block works. |

What follows from these results:
- M5 matches `gpu_vaspace.c:1835-1930`: `_memmgrPickDefaultGpuPageSize` picks 2 MiB for a request of
  2 MiB or more.
- The tile is 64 KiB. This is also the sparse block size the guest's Vulkan driver uses for these cases:
  `SLOT` in §4. That match is INFERRED for other titles.

### 8.2 Placement (built in stage 1: `crates/kf-mem/src/tiling.rs`)

- **One engine per guest range.** `Tiling` owns `[lo, hi)`, which replaces that range's static reservation.
  Its blocks partition the range exactly.
- **Each block has a base**: INVALID (a plain reservation) or SPARSE (`_SPARSE`).
- **The guest's wish** is one interval map, `set_base`:
  - SPARSE runs are rounded inward to tiles, INVALID runs outward;
  - so a mixed tile is INVALID: S2 wins, and a real fault never turns into a silent zero.
- **Lifecycle.**
  - **Fence**: an empty INVALID block.
  - **Sparse block**: an empty SPARSE block. A guest bind or unbind inside it is one map or unmap: no
    re-tile, because host RM restores sparse on unmap (M4).
  - **Row block**: a block that holds a mapping. It is never freed while it holds one (the guard).
    `prepare_row` carves the row's tiles out of the empty block that holds them, with the same base. A
    partial edge tile, which a neighbouring row may share, gets a block of its own, so a row pins only
    tiles it touches. `release_row` brings the emptied tiles to the wish and merges them.
- **Re-tiling.**
  - Every change is one `replace(old empty blocks → new layout)`.
  - The parts of the end blocks outside the changed span keep their base, and an empty neighbour with
    the same base is merged in.
  - Blocks that already match at either end are left alone.
  - A carve is free 1 + reserve up to 3 (M1: 5 calls with the map).
- **Exact layouts** (`layout`). A block under 2 MiB, or one 2 MiB-aligned at both ends, takes the
  default attr. An EMPTY block otherwise takes `_ATTR_PAGE_SIZE_BIG` + `_ALIGNMENT_FORCE` (M5). A row
  block is never BIG-only: it is cut into an aligned 2 MiB core plus edges under 2 MiB, so 4 KiB, 64 KiB
  and 2 MiB guest pages all fit it.

### 8.3 The per-space tiling lock (DESIGN, not built)

M3 shows that a host-RM placement in the same space can land in a freed fence. Nothing else can place
there: the space belongs to kf's client, and only kf's own calls place in it. The design:
- **One kf-owned lock per host VA space** (a `kf_util` lock rank below the VA manager's).
- **It is held across every re-tile**, from the free to the last reserve or the rollback.
- **It is also held across every host call that can place in the space without FIXED**:
  - `birth_channel`, `alloc_compute_object` and the rest of host RM's context-buffer placements;
  - `map_window*`, and `raw_map_dma_slice(.., None, ..)`.
- **Hold time** is the M3 window: p99 at most 197 us at 16k blocks, and below 100 us at the 4096-block cap.
- **No vCPU takes it.** Re-tiles run on the VA-manager (apply) thread, and object allocation runs on act
  threads (CLAUDE.md: nothing blocks on a vCPU or under a lock a vCPU takes).
- **A gap left by a refused restore is healed first** by whoever takes the lock next: a placement
  heals before it places.

### 8.4 Hostile-guest bounds (built in stage 1, except the log line)

- Every guest value is clipped to the range with checked arithmetic. A wrapping, empty or outside
  request is refused by name before any host call (`c2_hostile_requests_are_refused_before_any_host_call`).
- **Blocks per range are capped at 4096** (`TilingCfg::max_blocks`; M2: the worst reserve is 66 us
  there). Optional growth stops at the cap.
  - Mandatory S2 work may exceed the cap by the pieces of one layout, at most 3 per step.
  - At the cap, a row is mapped through the empty block that holds it without a carve (`pinned`): this
    is correct for S1 and S2 now, but that block cannot be re-tiled until the row goes.
- **The wish map is capped** (`max_wish_runs`, 32768). Beyond it a SPARSE wish is refused and counted,
  and the tile stays INVALID (loud).
- **Host-call budget per batch** (`batch_calls`, 4096; at the M1 rate that is about 800 carves, or about
  0.1 s of host time per walk batch). It cuts optional work: SPARSE carves, row carves (which then pin),
  and merges. **S2 restores are never cut**, and `begin_batch` retries them first, ignoring the budget.
  Cut work stays visible in `Tiling::pending()`.
- **Log** (stage 2): one `rate_eprintln!` line per batch with the stats (cut, pinned, rollbacks, gaps).
  Nothing per tile, so no unbounded emit.

### 8.5 Failure modes (built in stage 1)

- **A refused reserve or free in the middle of a re-tile** rolls the step back to the old layout: the
  new blocks are freed and the old ones restored (`c2_a_refused_reserve_mid_retile_rolls_back`).
- **A refused restore** leaves an unreserved gap. The gap reads INVALID, so S2 still holds. It is
  counted (`gaps_left`) and healed by the next call over it, or by `begin_batch`
  (`c2_a_refused_restore_leaves_a_gap_that_heals`).
- **A refused S2 restore** (the host keeps the SPARSE block) stays pending and is retried first, whatever
  the budget (`c2_a_refused_s2_restore_is_retried_first`).
- **The one S2 hazard C2 cannot fix in place** is a mixed tile under a live row: a guest maps one 4 KiB
  page inside a sparse tile, then returns another page of that tile to INVALID. The row's block is
  pinned, so the tile reads zero until the row goes. The case is reported as pending and fixed on
  release (`c2_a_mixed_tile_is_pending_and_fixed_when_the_row_goes`).
  - Guest RM always unbinds back to sparse inside a sparse resource (`gpu_vaspace.c:2279-2284`), so only
    a guest that bypasses its RM gets here. It only affects that guest's own VA.

### 8.6 Stage 1 proof (GPU-free, `crates/kf-mem/src/sim/tiling.rs`, 20 tests; mutants `scripts/ci/tiling_mutants.py`)

**The host model.** `sim`'s host model takes the engine's base (`_SPARSE`) and attr (the M5 rounding).
`SimRm::host_pte` answers per page. A guard is armed over every other live row, and it is checked after
every host call.

**The invariants**, checked against a brute-force per-page oracle:
- exact tiling: the engine's blocks are the host's reservations, unrounded, with no leak;
- mapped pages translate exactly;
- **S1** and **S2** hold per tile;
- every miss is reported pending.

**The tests:**
- `c2_s1_residency_slots_are_sparse_and_s2_holds_around_them`: the §4 pattern under C2; S1 and S2 are
  both green.
- The edge cases: the unbind without a re-tile, edges, merging, the remap/unmap/re-sparse sequence,
  partial tiles, 2 MiB rounding, budget cuts, the cap, refusals, the shared edge tile, and hostile input.
- The model-based random test: 24 seeds × 2 attr modes × 250 ops, strict (no miss at all), with the host
  calls of one op bounded by a constant.
- The same test with injected refusals and a small budget, which then heals to strict.

**Mutants:** 11. Each is killed by at least 2 named tests (run 2026-11-18: weak = 0). §4's KNOWN-RED tests and
the ratchet are unchanged: production still uses the static fence until stage 2.

### 8.7 Staging

1. **Stage 1 (this change).** The engine, the host model and the proof. No production caller.
2. **Stage 2: the walker.**
   - **The run.** `kf_walk.cu` emits a run with a new `KFWR_RF_SPARSE` flag (bit 7) where it now only
     counts `sparse_slots`.
   - **Its granularity.** A SPARSE run is tile-granular, rounded inward. It also comes from a VER3 sparse
     PDE (`kf_pde_sparse`), which covers its whole 2 MiB or 512 MiB.
   - **The diff.** The diff emits SPARSE when a PTE becomes sparse and UNSPARSE when it goes from sparse
     to INVALID.
   - **The ABI.** The bump goes to `KF_ABI_VERSION` and `KFWR_VERSION`. `kf_walk.h` mirrors the
     constants, and the `.cu`, `.h` and regenerated `.ptx` are committed together (NVRTC 12.2.140, §5; the
     CI ptx job must stay green).
   - **The budget.** SPARSE runs come out of the walk's existing run budget.
3. **Stage 3: wiring, behind a property that is off by default** (`exact-sparse=on`, next to
   `unmapped-reads-zero`).
   - `apply` routes rows of a tiled range through `prepare_row`, `commit_row` and `release_row`, and SPARSE
     and UNSPARSE runs through `set_base`.
   - `HostRm` gains `reserve_va_attr`: production's form of the probe-only verb, with the attr and alignment.
   - The per-space lock is added (§8.3).
   - Off by default, nothing changes.
4. **Stage 4: the default.** `exact-sparse=on` becomes the default and replaces `unmapped-reads-zero=auto`.
   The evidence needed first:
   - the §6 table on one box with exact on and zero off: the residency cases pass with 0 Xid;
   - the fast suite at 30/30 and v3_gates at 9/9 with exact on;
   - a UVM/HMM negative control that still fails loudly (S2 on hardware);
   - one Windows sparse title or DX12 tiled-resource case;
   - the per-map cost on a CUDA app row with many allocations, against today's.

**Open (INFERRED, not measured):**
- whether Unreal's virtual textures and DXVK's sparse binding stay within the 4096-block cap per range;
- whether a 64 KiB tile is fine-grained enough for every title.
