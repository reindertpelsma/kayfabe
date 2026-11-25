# V3 — sparse residency: why residency cases hang in the guest (root cause, GPU-free reproduction)

**STATUS: LIVE, 2026-11-25. Root cause found in the code and CONFIRMED on hardware (§6, one RTX 4090 box: off, the residency cases hang with Xid 109; with option B on, they pass). Option B ships as the property `unmapped-reads-zero`, which is ON by default for every guest and profile (owner ruling 2026-11-24, §5; it was an opt-in, #494). Windows runs of it do not exist. Option C (the exact fix): C1 is MEASURED infeasible; C2 is designed, not built (§7). The hardware link from an INVALID host PTE to Xid 109 is INFERRED. The fast suite's two fault arms (`--defer-liveness`, `--missing-page-fault`) boot with the property off (§8).**
⊘ **Correction 2026-11-25 (TENTATIVE, release-candidate branch `integration/release-rc1-20261125` only; pending the owner's ruling on decision B1; `OWNER_RULINGS.md` §BH is unchanged and still says ON for every guest):** on this branch `unmapped-reads-zero` is `on`/`off`/`auto` and defaults to `auto` (`KF3_ABI` 38): ON for a default-start mirror space (every Linux guest), OFF for a floor-started space (a guest under the Windows support profile). Reason: with the fill on, every Windows boot of 2026-11-24 lost the guest kernel's flat framebuffer alias to a host `0x51` (pc RTX 4070 at `63e46069`, `traces/pc_hwgates_20261124`; Vast RTX 4090 at `4a6a2108e`, `traces/fill_probe_20261124`), and none did with it off. On the same branch a `0x51` on a FIXED map routed through a reservation of ours is refused by name (`kf_host::channel::VA_RESERVED_MAP_REFUSED`) instead of being acknowledged as held by the host; §9 has both. Neither change has run on a GPU. To restore §BH's default: `DEFINE_PROP_ON_OFF_AUTO(..., ON_OFF_AUTO_ON)` in `kf3.c` and `UNMAPPED_READS_ZERO_DEFAULT = UnmappedReadsZero::On`. Sentences below that say "ON by default for every guest and profile" describe `next`, not this branch.
⊘ **Correction 2026-11-24 (owner, `OWNER_RULINGS.md` §BH), implemented 2026-11-24 by PR #580:** The earlier status wording "Option B ships as an opt-in property" is superseded as a default: `unmapped-reads-zero` defaults to ON for every guest, Linux included (the owner's statement: real host sparse residency proves the ioctl exists on Linux, and Linux Vulkan sparse-residency cases hang with the fill off). The code and the details are in §5.

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

**Correction 2026-11-24 (owner ruling): `unmapped-reads-zero` defaults to ON for every guest and profile; the
text below that says "opt-in" or "default OFF" describes #494 and is superseded by this paragraph.** Why:
Linux needs the fill too. A real host GPU maps sparse pages through the same RM verb, so the verb exists on
Linux, and the Linux Vulkan sparse-residency cases hang with host Xid 109 with the fill off and pass with it on
(MEASURED: the hang with the fill off on an RTX 3090, §1, and on an RTX 4090, §6; the pass with the fill on on the RTX 4090, §6). A proposed `auto` default that kept Linux off and gave Windows
the fill (PR #507) was never merged and is dropped: with Linux on there is nothing left for `auto` to decide, so the
property stays a plain on/off (`DEFINE_PROP_BOOL`, default `true`; `KF3_ABI` 37 because the default is C text and a
QEMU built from the old `kf3.c` would otherwise keep the old default silently).
- **Reachability (the security reading).** The fill changes only what a guest's OWN unmapped reads return, zero instead
  of a fault, and drops its writes there. It adds no mapping to host memory and no host verb: the reservation is the
  guest's own static VA range (`GUEST_VA_RANGES` and the low range), allocated `_SPARSE`, and a bound page still maps
  exactly what the guest bound. No guest byte selects the fill; it is set once at realize from the property.
- **The owner-visible hazard (MEASURED for one case, otherwise INFERRED).** With the fill on, an access the guest
  expects to fault reads zero instead: CUDA managed memory or HMM pages that need demand paging (not served by kayfabe
  today), and an application that reads freed memory. A run that failed loudly with CUDA error 719 or a forwarded
  RC (Xid 31) can now finish with wrong numbers. The one measured instance of silent wrong output is
  `conjugateGradientUM` after a host Xid 31 (`docs/howto/troubleshooting.md`); the effect of the fill on the four Linux
  UVM applications has not been run. S2 below is therefore broken on purpose by the default.
- **Turn it off per VM:** `-device kf3,...,unmapped-reads-zero=off` restores the fault: S2 holds, and Vulkan sparse
  residency hangs again. The unconfigured host (the harness, the raw client, the kf-mem model) keeps the fault, so a
  bare-metal reference run is unchanged.
- **Not run:** a Windows guest (DX12 reserved/tiled resources, a Windows build of the Vulkan CTS sparse list, a
  desktop smoke test of the default). That DX12 and UE virtual textures hit the same INVALID-versus-SPARSE gap is
  INFERRED, and so is a smaller silent-zero hazard on WDDM (no demand-paged UVM/HMM).
- Tests: the GPU-free tests are `kf-host` `guest_va_fill_tests`, `kf-qemu` `config_tests` and
  `tests/unmapped_reads_zero_use_sites.rs` (the C default is pinned to `UNMAPPED_READS_ZERO_DEFAULT`, which is `true`),
  and `scripts/ci/sparse_fill_mutants.py` (12 mutants, each killed by at least two named tests, among them
  `linux-space-faults`, `default-const-off` and `c-default-off`, which restore the old behaviour).

**Update 2026-11-18 (coordinator decision): option B is implemented as an OPT-IN, default OFF.** The
`unmapped-reads-zero` device property (`kf_host::channel::GuestVaFill`, `guest_range_plan`; ABI 35;
`docs/howto/installing.md`) applies to the static guest ranges only. Its tests are in
`crates/kf-mem/src/sim/sparse.rs`:
- `on_s1_*` are green with the property on;
- `on_s2_consequence_*` pins the documented price (with it on, a guest-INVALID VA reads zero);
- with the property off, S2 and the ratchet are unchanged.

Mutants are in `scripts/ci/sparse_fill_mutants.py` (7 then, 12 now, each killed by at least 2 named tests). Two wiring
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

The costs and unknowns, none of them measured:
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

## 8. The fast suite's two fault arms boot with `unmapped-reads-zero=off`

Two of the 30 fast-suite arms test the FAULT of a read through a VA that nothing maps: `--defer-liveness` (does a PTE written with the
deferred-invalidate flag go live, asked of hardware through a VA whose non-present result is a fault) and `--missing-page-fault` (a deliberate
missing page must be contained and named, with a real Xid 31). With `unmapped-reads-zero` ON, which is the default for every guest, an unmapped
read returns zeros and never faults, so neither arm can pass: that is the property working as designed, not a regression of the arms.

- **The default stays ON.** The two arms are the only suite arms that assert the OFF behaviour, so the suite, not the product, changes.
- **Where it is set.** `scripts/fastguest/run_fast_guest.sh` derives the property from the arm names (`FAULT_ARMS`) and adds
  `unmapped-reads-zero=off` to the kf3 device line of every device of a boot that runs either arm; no other boot has it. It is not an
  environment variable. An explicit `unmapped-reads-zero=` in `KF3_DEV_EXTRA` that is not `off` is refused for those arms (exit 2, QEMU not
  started), because a boot that cannot pass should not start.
- **What fails if it is lost.** `scripts/ci/test_fastsuite_fill.py` runs the lane with a recording fake QEMU (no GPU, no guest) and fails when
  either arm boots without the property, when any other suite arm boots with it, or when the multi-device and combined-arm cases lose it.
  `scripts/ci/fastsuite_fill_mutants.py` kills nine one-site edits of the lane, each by at least two named tests.
- **Evidence the arms need it.** [inferred, from a hardware run reported in PR #595, not in this tree] on one RTX 4070 box the suite at `next`
  63e46069 passed 28 of 30, the two failures being exactly these arms, and both passed with `unmapped-reads-zero=off` on the same binary. A
  fast-suite run with this change on hardware has not been done.
- **Other lanes that could depend on an unmapped-read fault** (checked by reading the scripts, not run): none of the Windows lanes asserts one;
  `scripts/bench/rmladder_suite.sh` and `gpu_fault_containment.sh` run on the host, outside the kf3 device; the lanes that call
  `fast_suite.sh` or `run_fast_guest.sh` (the driver matrix, `dbfast_lane.sh`, `perf_compare.sh`, the box merge bar and the Vast payload) get
  the property through the lane script; `readmostly_hook.sh` and `llm_hook2.sh` are diagnostic hooks that read host Xid lines and assert
  no verdict on an unmapped read.

## 9. Windows: the fill is off under `auto`, and a `0x51` inside a reservation of ours is refused (TENTATIVE, 2026-11-25)

Tentative default of the release-candidate branch, pending the owner's ruling on decision B1. Nothing in this section has run on a GPU; the measurements cited are of the binaries named.

**What was measured before this change.** With the fill on (the default of `next` since #580), the Windows guest kernel's flat framebuffer alias (VA `0x120000000`, 3.74 GiB at `fb-mb` 4096, 4 KiB pages, inside the first static guest range) failed in host RM with `mmuWalkMap ... NV_ERR_NO_MEMORY` (`0x51`): 4 of 4 default boots and 0 of 4 with the fill off on the pc run of 2026-11-24 (RTX 4070, host 595.91.07, kf3 `63e46069`, `traces/pc_hwgates_20261124`), and again in the gate, display-on and display-off boots of the Vast run of 2026-11-24 (RTX 4090, host 595.84, kf3 `4a6a2108e`, `traces/fill_probe_20261124`, Experiment 2), not with the fill off. kf3 reported the answer as `VA_ALREADY_MAPPED`, the memory plane acknowledged the row HELD, and the alias was absent: under the SPARSE range it read zero with no fault and no refusal line. Why the fill triggers the failure in a Windows guest is not settled (the host-only probe of the same run mapped the alias in 22 of 22 arms). The same run measured two costs of the fill that do not depend on Windows: about 2038 MiB more host video memory in use while a fill-on space exists, and 4 KiB of host page table kept per 2 MiB region a guest ever mapped in the first range until the space is freed (1029 MiB after 262144 regions; 1 MiB with the fill off).

**(a) `auto`.** `unmapped-reads-zero` takes `on`, `off` or `auto` (QEMU `OnOffAuto`; the word passed to `kf3_realize` is 0 auto, 1 on, 2 off; another word refuses realize by name). `kf_qemu::device::Config::guest_va_fill_by_start` turns it into a `kf_host::channel::GuestVaFillByStart`: `auto` is ZERO for a default-start space and FAULT for a floor-started one, `on` and `off` are uniform. `HostRm::alloc_vaspace_at` plans each space with the fill of its own start. A space is floor-started exactly when the guest declared the measured Windows build and `windows-support` is on (`MirrorVaStart::select`), so a Linux guest keeps the fill and its sparse-residency cases, and a guest that declares Windows falsely changes only what its own unmapped reads return (a fault instead of zero). INFERRED, not run: the guest kernel space that holds the alias is floor-started, because the guest declares its build at fn 1 before it allocates any space. A boot of this branch has to confirm it (no `0x51`, no refusal line, with the default).

**(b) Refusal.** `kf_host::channel::map_pieces` and `HostRm::map_in` answer a piece routed through a live guest reservation, or through a reservation the caller made, with `own_reservation_answer`: `VA_ALREADY_MAPPED` becomes `VA_RESERVED_MAP_REFUSED` (`0x4B7B`) and one rate-limited line names it. Host RM's allocator places nothing inside a reservation of ours, so there the `0x51` is host capacity or a mapping of ours still there, never a host buffer, and the row is absent either way. The memory plane then refuses the row (`KFWR_ACK_FAILED`) instead of acknowledging it HELD. A piece in the space's own range, where host RM places its own objects, and a range whose reservation was refused keep the old answer. Consequence to expect: a Windows guest started with `unmapped-reads-zero=on` now gets a named refusal for the alias; an earlier refusal of the whole alias row killed the guest RM's kernel copy-engine channel (`kf-mem` `apply.rs`, the carve-out note), so such a guest may fail to start its driver instead of running with a silent hole.

**Not done here.** Option C (fill only what the guest declares sparse) and a bound on the page tables a fill-on Linux guest can make the host keep.

**Tests (GPU-free).** `kf-host` `split_row_tests` (the alias row answered `0x51` inside the first reservation is refused; every live reservation; the space's range and an unreserved range keep `VA_ALREADY_MAPPED`; rollback of a split row; a caller's reservation; `map_in` held as source text), `guest_va_fill_tests` (per-start fill), `kf-qemu` `config_tests` and `tests/unmapped_reads_zero_use_sites.rs`; mutants in `scripts/ci/sparse_fill_mutants.py`.
