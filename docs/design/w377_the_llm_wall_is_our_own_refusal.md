# ★★★★★ THE LLM WALL IS OUR OWN REFUSAL PREDICATE — not a missing mapping signal

**STATUS — 2026-09-06 — ⊘ PARTIALLY SUPERSEDED THE SAME DAY.** §1/§3 refuted; see the correction block below. §5–§8 LIVE. Diagnosis from the `w376llmd` boot's own census, plus a
census of the committed bare-metal reference trace. Blockers (1)/(3)/(4)/(5) are named and
localized; **(2) is named and NOT root-caused** — do not read this doc as a complete
explanation.

> # ⊘⊘⊘ CORRECTED 2026-09-06, HOURS LATER — **§1 IS WRONG ON THE MECHANISM, AND IT
> # CONTRADICTS AN EARLIER RULING IN THIS TREE THAT IT DOES NOT CITE.**
> Read this block before anything below it. The title of this file is **retained deliberately
> as a tombstone** — the wall *is* ours, but it is not the refusal predicate.
>
> ## ★★★★★ THE ACTUAL CAUSE: THE FB-JOIN STORE IS KEYED BY PHYSICAL FRAME ALONE
>
> `install_join(phys, region)` / `release_join(phys)` / `fb_join_installed_at(phys)`
> (`kayfabe-device/src/plane.rs:1685,1704`; `kayfabe-device/src/fbwin.rs:401,461,1111,1203`).
> **One framebuffer frame can be host-backed at exactly ONE GPU VA.** The guest holds **17
> frames aliased at two VAs each**, so publishing either VA *revokes the other*.
>
> ⇒ **The Xid 31 is at a range WE UN-PUBLISHED OURSELVES, and our own log predicted it
> verbatim** (`shim.rs:10742-10771`, `kayfabe-rt/src/device.rs:3751-3798`):
> *"the guest re-pointed this frame from `va=0x748027600000` to `va=0x7480ae000000`. Old row
> UNBOUND, join RELEASED. ⊘ The old VA is still DESCRIBED by the guest and now resolves with
> no host backing — an engine still pointed there takes a CONTAINED fault."*
> The fault is at **`0x7480_27604000`** = that VA + `0x4000`.
>
> Measured this boot: **127 supersedes over 17 frames** in symmetric A→B/B→A pairs of 4
> (`SUPERSEDE_CAP_PER_FRAME = 4`), then **28 108 `⊘ SUPERSEDE CAPPED`** across 14 (frame,VA)
> pairs. The code's own comment already named it — *"an uncapped takeover is a ping-pong"*
> (`shim.rs:10770`). ⚠ **The cap does not stop the ping-pong; it FREEZES it**, leaving one VA
> of each pair permanently unbacked while the guest still describes it.
>
> ## ⊘ EVERY LOAD-BEARING CLAIM IN §1 AND §3 IS REFUTED
>
> - ⊘ **`runs=0` carries NO information.** `rows += 1` and `runs.push(..)` are the *same
>   iteration* (`kayfabe-rt/src/device.rs:3483-3496`), so `rows == 0 ⟹ runs == 0` by
>   construction. It is not a second model and cannot discriminate *"never ran"* from
>   *"published nothing"*. **This field is what sent the whole diagnosis to proc 0.**
> - ⊘ **`proc=0` is `SYSTEM_PROC`, and skipping it is deliberate.** The pass DID run and took
>   a full census; it prints its own reason:
>   `⊘SYSTEM-PROC:NEVER-ATTEMPTED(§12.26 …) total=6254 … candidates=6144 published=0
>   refused=0 sum_ok=true` (`shim.rs:9522-9548`, `gpu.rs:5114`). Not a defect.
> - ⊘ **The FAULTING VAS IS 99.93 % COVERED** — `proc=3 pdb=0x201000 total=18539
>   already_host=1226 already_pinned=17300 candidates=7 refused=6`. **Under-publication is
>   not the wall.** §1's `host_rows=1226 of 18539` reading ignored `already_pinned`.
> - ⊘ **`StraddlesLiveBinding` is SKIPPED, not rejected.** `populate` returns
>   `PopulateOutcome`, not `Result` (`kayfabe-mmu/src/walker.rs:1029,1082-1097`); **no branch
>   anywhere reads it and changes control flow.** `InsideLarger`+`SameMemory` means the memory
>   *is already covered by the row we kept* — 254 of 255 are benign.
> - ⊘ **Relaxing it would publish nothing.** 240 of the rows are 4 KiB and die one gate later
>   on 64 KiB granularity (`FB_LEAF_GRANULE = 0x1_0000`, `kayfabe-rt/src/device.rs:3694`);
>   the covering rows fail too (`0xea000` = 14.625 granules, `0x8600` not even 4 KiB-aligned).
>   And `refusals=255` is **constant across the entire boot** — it does not grow with the
>   workload.
> - ⊘⊘ **THIS WAS ALREADY RULED, AND I DID NOT CITE IT.** Commit `ff58c961` (w277):
>   *"the live extents are `0x4000 / 0x8600 / 0x80000 / 0xea000` — NOT ONE is a page size …
>   The refusal is CORRECT. Relaxing it would have shredded 255 correct rows and bought
>   nothing."* ⚠ Exactly the failure this repo's doc-hygiene rules exist to prevent: a
>   confident new doc re-deriving a settled question in the opposite direction.
> - ⊘ **The 9 parked promotes were refuted at w290** (`traces/boots/w290/RESULT.md`,
>   `5768e956`). They are structurally unresolvable — we park on `size == 0` and the UVM
>   promote never carries a physical half — **and they would publish nothing if resolved**,
>   since `apply_promote_ctx` binds with `host: None` (`promote.rs:1183`).
> - ⊘ **`pub0` over-read.** It means *"carries no `HostBacking` record"*; a row covered by a
>   live guest-RAM pin IS mapped host-side with `Binding::host == None`
>   (`kayfabe-rt/src/device.rs:3684-3689`). `pub0` does not prove unmapped.
>
> ## ★ THE FIX, AND THE DISCRIMINATOR THAT MUST PRECEDE IT
>
> ### ✔ BUILT AND BOOTED — `w380`, 2026-09-06. See `w380_one_frame_n_addresses.md`.
> The discriminator below was answered by §9 (the guest holds every alias live) and again,
> independently, by `w379` on bare metal (RM maps one allocation at two VAs; unmapping one
> leaves the other). ⇒ *"allow N VAs per frame"* was built as `RmBackend::alias_fb_leaf`.
> `[measured w380llm, real GA106]` **`SUPERSEDED` 127 → 0, `⊘ SUPERSEDE CAPPED` 28 108 → 0,
> host `Xid` 1 → 0**, and eight frames aliased at a second VA with `placed_as_asked=true`.
> ⊘ `LLM_TOKENS` is still **UNMEASURED**, not `0` — see that doc's §5.
>
> **Fix:** key the FB join by **`(phys, va)`**, or allow N VAs per frame — one host
> `OS_DESCRIPTOR` over the frame, mapped at every VA the guest describes. **The aliasing is
> the guest's and it is legal.**
>
> ### ★★★★★ THE DISCRIMINATOR'S NATIVE HALF IS MEASURED — w379, 2026-09-06, real GA106
> `traces/real_ga106/w379_mapping_plane_real_ga106.txt`, `kayfabe-rm-ladder --w379`, bare
> metal, 5 of 5 rungs PASS, source `d24f182`. Three facts the fix rests on, none of them
> inferred:
> - ★ **One allocation IS live at two GPU VAs at once** (`--alias-two-vas`). A release
>   through `VA_B` landed at offset `0x40` of the **one object** the rung allocated, so the
>   aliasing is *measured* rather than asked for; a release through `VA_A` then landed
>   **again**, after `VA_B` was mapped. **RM does not revoke on the second map.**
>   ⇒ *"The aliasing is the guest's and it is legal"* is now a measurement, not a reading.
> - ★ **Unmapping one alias leaves the other live** (`--alias-unmap-observe`), and the
>   unmapped VA probes `Free` again. **RM tracks the two aliases independently.** ⇒ a host
>   store keyed by **physical frame alone** is strictly weaker than the driver it stands in
>   for, which is the `(a)` side of the fix — *allow N VAs per frame* — with the driver's
>   own behaviour behind it. ⊘ It does **not** settle whether *our decode* holds a stale VA;
>   that half is guest-side and is still open.
> - ⚠ **`pde_info` is NOT a publication oracle, and it answered `PDE_COVERS` at a VA the run
>   NEVER MAPPED.** `NV0080_CTRL_CMD_DMA_GET_PDE_INFO` reports whether a page *table* covers
>   the address, so it says `PDE_COVERS` for any VA inside a live table — including
>   unmapped ones, and including one whose leaf was just torn down. The w379 rungs record it
>   and **grade on `probe_va` and on hardware instead**. Anything that reads `PDE_COVERS` as
>   *"this VA is published"* is reading page-table granularity as a leaf fact.
>
> ⊘ **Scope.** Bare metal only, and deliberately so: the same probe cannot run in the guest,
> because the Mode-2 CPU copy-engine emulator decodes `PushMethod::SemRelease` and
> **deliberately does not act on it** (`kayfabe-rt/src/ceutils.rs:677-679`, restated
> `:1080`) — only `LAUNCH_DMA` is served. The guest-side version of these questions needs a
> `LAUNCH_DMA` probe and is **not built**.
>
> ⚠ **Run the discriminator first.** *"The old VA is still DESCRIBED by the guest"* is an
> **unconditional string literal** (`shim.rs:10766-10769`), not a check — suspect the
> instrument. Either the guest genuinely holds both VAs live (⇒ allow N VAs), or one is stale
> in our decode and we never learned to drop it (⇒ observe the unmap). Since we see no TLB
> invalidates on the compute path, **both models fit the data**, and they select different
> fixes.
>
> ## ⚠ TWO INSTRUMENTS ARE ACTIVELY MISLEADING — fix in the same cycle
> - **`runs`** is redundant with `host_rows` and reads as a pass counter. It is not one.
> - **`DIRTY-GATE publish[fired=15996 skipped=0]`** counts **VASes visited**, not gate
>   firings — `gate_fired += 1` is unconditional per visited VAS (`shim.rs:9520`), and
>   `gate=off` on every pass this boot.
>
> ## ✔ WHAT SURVIVES FROM BELOW
> §5 (198 static map events, zero migrations, no managed memory), §6 (the five banked ogkm
> corrections), §7 (the invalidate hook and its measured scope) and §8 (the bare-metal
> late-map result) are **unaffected** — they rest on their own measurements, not on §1.

## §1 THE HEADLINE

`traces/guest_boots/` `run_w376llmd_qemu.log`, verbatim from the `PT-DECODE` line:

```
refusals=255   by_kind={"StraddlesLiveBinding": 255}
straddles=255  contradicting=0
sigs={ InsideLarger/SameMemory/lvl5/leaf0x1000/live0xea000/pub0 = 233
       InsideLarger/SameMemory/lvl4/leaf0x10000/live0x80000/pub0 = 7
       InsideLarger/SameMemory/lvl5/leaf0x1000/live0x8600/pub0  = 7
       SameStartShorter/SameMemory/...                          = 4
       CrossesEnd/SameMemory/...                                = 1 }
```

**Every refusal is `StraddlesLiveBinding`. `contradicting=0`. Every signature says
`SameMemory`** — same physical address, same aperture. We refuse rows that *agree* with what
we already hold, solely because their **extent** differs from the live binding's.

And `pub0` on 255 of 255 means the straddled binding **was itself never published**. So the
loop closes against us:

> we hold a binding → we never publish it → we refuse every finer row that would have covered
> the same memory, *because* we hold it.

`host_rows=1 of 13348`, later `523 of 13348`. Proc 0's main VAS: `host_rows=0 of 6254
runs=0` — publication for that address space never ran at all. `MERGE-AGREES=false` on the
row that did grow. Nine promote entries sit `AwaitingPhysical`, never bound, at VAs in the
same region as the refusals.

⇒ **The Xid 31 `FAULT_PDE` is downstream of a self-inflicted deadlock in our extent
matching.** It is not evidence about invalidates, doorbells, races, or the guest.

## §2 WHY THE C DID NOT HAVE THIS

The C rounded every promote-derived mapping **up to a page size** before mapping it
(`nvkvm_gpu_emul.c:7920`, `asize = (size + 0xffff) & ~0xffffull`), so its rows aligned with
each other by construction. This port binds at the **declared length**, so rows arrive at
mismatched extents and our own straddle check converts every misalignment into a refusal.

⊘ **CORRECTION to what this repo has been carrying.** The C's round-up is real but is NOT the
flat 64 KiB the notes claim, and copying it blindly would over-cover. In ogkm the round-up
lives in the *mapping*, at the **memdesc's own page size** —
`mapLength = RM_ALIGN_UP(pageOffs + memdescGetSize(pMemDesc), pageSize)`
(`ogkm-580: virt_mem_allocator_gm107.c:2781`), `vaSize = RM_ALIGN_UP(mapLength, vaAlign)`
with `vaAlign = NV_MAX(pageSize, compAlign)` (`:2785`). And the **GR MAIN ctx buffer is
explicitly forced to 4 KiB**: *"Force page size to 4KB, we can change this later when RM
access method support 64k pages"* (`gr/kernel_graphics_context.c:1168-1173`). A blanket
`(size + 0xffff) & ~0xffff` **over-covers exactly the buffer** the notes flagged as
load-bearing. The fix is extent *reconciliation*, not imported rounding.

## §3 THE BLOCKERS, AS OF THIS DOC

1. **`StraddlesLiveBinding` refuses agreeing rows.** `contradicting=0` proves there is no
   conflict. `InsideLarger` / `SameStartShorter` over `SameMemory` at the same phys+aperture
   is a **refinement**, not a collision.
2. **The covering bindings are unpublished** (`pub0` 255/255; `runs=0` on the 6254-row VAS).
   ⚠ **NOT ROOT-CAUSED.** Fixing (1) is necessary and not sufficient.
3. **9 promote entries parked `AwaitingPhysical`**, never bound, co-located with the refusals.
   Plausibly the same cause as (2); not established.
4. **Publication runs on the vCPU thread under a budget.** `inline_exceptions` 38 → 675 →
   61 865 across one boot; `worst_trap=1750538us` against the instrument's own
   `(target: inline_exceptions=0)`. The budget that keeps the guest alive is what truncates
   publication.
5. **The census cannot certify coverage.** `⚠⚠ CAPPED at 24 of 255 distinct`,
   `CAPPED at 48 of 69 runs`. A capped list cannot distinguish a fix from a coincidence.
   Needs a predicate (`declared ⊆ published` + residual), not a longer list.

## §4 WHAT IS *NOT* BLOCKING — each retired with evidence

- **Recoverable faults are unavailable and unnecessary.** Three independent hard blockers:
  `ENABLE_PAGE_FAULTING` *requires* `IS_EXTERNALLY_OWNED` (`vaspace_api.c:686-690`); the
  fault-buffer class is `RS_FLAGS_ALLOC_KERNEL_PRIVILEGED` (`resource_list.h:975`), not even
  root; ownership transfer is `RMCTRL_FLAGS_PRIVILEGED` and UVM takes it at first GPU
  registration. ★ And it is unnecessary: **the C went green without servicing or forwarding a
  single GPU fault.**
- **The invalidate-elision holes are on a road nobody drives.** `NVOS46_FLAGS_DEFER_TLB_
  INVALIDATION` (bit 31) and the `pTgtPteMem` conjunction both gate the invalidate at
  `virt_mem_allocator_gm107.c:2611-2617`, and both are client-settable. But
  **`NV_ESC_RM_MAP_MEMORY_DMA` (0x57) appears ZERO times in 2704 records** of the committed
  bare-metal reference trace. libcuda does not use that path.
- **`NV_MEMORY_MAPPER` (0xFE)** — same DMA-map path, same zero traffic. Whether libcuda
  allocates the class at all is still **NOT ESTABLISHED**; it is a hostile-guest concern, not
  a live-path one.
- **`fbsr` resume creates no new mapping.** `FBSR_OP_RESTORE` blits back byte-identical
  content captured by `FBSR_OP_SAVE`; the VA→phys set after resume equals the set before.
  The requirement is that *our* table survives suspend, not that we detect anything.
- **No managed memory, no migration.** See §5.

## §5 THE MAPPING SURFACE IS 198 STATIC EVENTS — measured

Every `nvidia-uvm` escape in `traces/host_reference_ga106/` (both replicates), named from
`uvm_ioctl.h`:

```
UVM_FREE                       214      UVM_RESERVE_VA            12
UVM_CREATE_EXTERNAL_RANGE      198      UVM_MM_INITIALIZE         12
UVM_MAP_EXTERNAL_ALLOCATION    198      UVM_PAGEABLE_MEM_ACCESS   12   <- a capability QUERY
UVM_REGISTER_CHANNEL           128      UVM_REGISTER_GPU          12
UVM_UNREGISTER_CHANNEL         128      UVM_REGISTER_GPU_VASPACE   8
UVM_CREATE_RANGE_GROUP          68      UVM_ALLOC_SEMAPHORE_POOL   8
UVM_DESTROY_RANGE_GROUP         56
```

**`UVM_MIGRATE`: zero. `UVM_MIGRATE_RANGE_GROUP`: zero.**

⇒ This is `cudaMalloc` device memory allocated through RM and **registered** with UVM as an
*external* range — `CREATE_EXTERNAL_RANGE` and `MAP_EXTERNAL_ALLOCATION` pair 198:198. UVM
is the VA-space registrar here, not the pager. **No managed allocation, no fault servicing,
no migration is on the LLM's path.**

★ Three properties follow, and together they are what makes the design tractable:
1. **The mapping surface is finite and enumerable** — 198 discrete events for the whole
   program. A coverage predicate over 198 events is assertable; a stream is not.
2. **Nothing moves after it is mapped** (zero migrations) — publish once at a known event and
   it stays true. This is the single property whose absence would have made this intractable.
3. **Every map has two observable predecessors** — the RM allocation and the explicit VA range
   creation. Both operands are known before the binding.

⊘ **Scope.** That trace is `cuInit`/`cuCtxCreate`/alloc/launch/CE. It does not speak for
graphics, Vulkan, or any workload not yet traced. ⚠ And *"UVM is later, not skipped"* (owner,
2026-09-06): the UVM **driver** is unavoidably in the path as registrar; only its **managed
memory machinery** is deferred.

## §6 CORRECTIONS BANKED — things this repo asserted that are wrong

- ⊘ **`bIsContextBound` is UVM-ONLY.** It was offered (by me) as a general RM-enforced
  ordering gate. The predicate is `gvaspaceIsExternallyOwned(pGVAS) && IS_GR(engineDesc) &&
  !bIsContextBound` (`kernel_channel.c:2200`), and the only `NV_TRUE` setter in the tree is
  UVM's bind path (`nv_gpu_ops.c:10903`). **A stock channel is never in an externally-owned
  VAS and never trips it.** What gates non-UVM submission is
  `kfifoGenerateWorkSubmitTokenHal_GA100`, which refuses the token unless
  `kchannelIsRunlistSet` (`kernel_fifo_ga100.c:214-220`).
- ⊘ **The *"hammer of always invalidating"* comment is ATS/ARM-only.** It occurs exactly once,
  `uvm_ats_faults.c:559`, inside a block whose else-branch asserts `NVCPU_IS_AARCH64`. GA106
  x86 with ATS off never reaches it — and it states the **opposite** hardware fact to the one
  banked: *"The GPU will re-fetch an entry on access if the PTE is invalid and the page size
  is not 4K."* ★ The over-invalidation conclusion survives by a different route:
  `update_type = (bUnmap || (LOCK_FALSE == tlbLock) || readOnly) ? PTE_DOWNGRADE :
  PTE_UPGRADE` (`virt_mem_allocator_gm107.c:2254`), and `tlbLock` is set only by
  `NVOS46_FLAGS_TLB_LOCK_ENABLE`, which ordinary maps do not set. **Every ordinary map is
  DOWNGRADE — the hammer is the default branch, not a comment.**
- ⊘ **The fn-200 `NV_RM_RPC_INVALIDATE_TLB` is NOT dead because it is `_STUB`.**
  `rpcSetIpVersion` overwrites the stub with `rpcInvalidateTlb_v23_03`
  (`g_rpc_private.h:3131-3132`, driven from `rpc_common.c:99-105`; build ipVersion
  `0x2B130000`). It is dead because `bDoVgpuRpc` needs `GPU_GET_VGPU(pGpu) != NULL`, and
  `vgpuCreateObject` runs only under `IS_VIRTUAL` (`gpu.c:529-550`). **A falsifier built on
  the stub line tests an object that is replaced at init.**
- ⊘ **The UVM promote carries almost nothing.** `nvGpuOpsBindChannelResources` fills *only*
  `bufferId` and `gpuVirtAddr` over a zeroing memset (`nv_gpu_ops.c:10886-10889`) —
  `gpuPhysAddr`, `size` and `physAttr` are all **zero**. And on an externally-owned VAS RM
  sends **no VA entries at all** (`kernel_graphics_context.c:1884`). ⇒ a promote-capture keyed
  on `{physAddr, size, attr}` — which is what the C did and what this port ported — gets
  nothing on the libcuda path.
- ⊘ **On GA106 upgrade and downgrade are byte-identical at the invalidate register.** The
  membar WAR binds to the stub `_4a4dee` (`g_kern_gmmu_nvoc.c:578-585`), so no `_SYS_MEMBAR`,
  no `_ACK_GLOBALLY`, `flushCount = 0`. **Only the PDB distinguishes anything** — do not try
  to recover update type from the write.

## §7 THE HOOK, FOR WHEN PUBLICATION MOVES OFF THE vCPU THREAD

Every RM invalidate funnels to BAR0 `0x00B830A0`/`0x00B830A4` (PDB) then `0x00B830B0`
(`_TRIGGER`) — `kern_gmmu_tu102.c:143-153,117`. Always `_ALL_VA _TRUE`
(*"Not using range-based invalidate"*, `kern_gmmu_gm107.c:187`), so **the signal names a VAS
and a time, never a range.**

★ **RM blocks there, twice** — a pre-wait and a post-wait spin on `GPU_VREG_RD32(0x00B830B0)`
until `_TRIGGER FALSE` (`kern_gmmu_tu102.c:50-83`), under
`BYPASS_THREAD_STATE|BYPASS_CPU_YIELD`, budget 4 s graphics / 30 s compute, returning quietly
on timeout. ⇒ **holding `TRIGGER` while we publish is parity, not a stall we invented** — and
it is categorically different from blocking inside an MMIO write handler, because a guest
spinning on a poll loop still exits, takes interrupts, and lets sibling vCPUs run.

Measured scope (`w326m1`, cup3): `triggers=377 all_pdb=0 all_va=377 hubtlb_only=232
gpu_vas=145 polls=754 distinct_pdbs=8 doorbells=480`. It never invalidates the whole GPU; it
always names one PDB. **232 of 377 are BAR VA spaces and must not be treated as compute
publications.** `crates/kayfabe-device/src/mmuinval.rs` already decodes all of this and
already has a `TriggerAction::Publish` arm that holds `TRIGGER` — **and it is disarmed**
(`armed=false`, and `polls=754` = exactly 2 per trigger, the floor, confirming we have never
held it).

## §8 THE HARDWARE FACT, MEASURED (w377, bare metal, 3/3, zero Xids)

A mapping created **after** the doorbell, into a channel already running, with **no second
doorbell**, is walked by the engine. `traces/real_ga106/w377_late_map_race_real_ga106.txt`.

⊘ This bounds the **hardware**, not ogkm. The probe issued the map at a moment of its own
choosing, deliberately bypassing whatever ordering discipline ogkm imposes. It says the GPU
will not save us if ogkm ever produces that ordering; it does not say ogkm does.
