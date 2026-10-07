# The guest's invalidate discipline — when does a PTE write get a TLB invalidate, and can we use it as our publish boundary?

**STATUS: LIVE, 2026-08-14 (w324).** Read-only derivation from `research_clones/ogkm/`
(**610.43.02**) and `research_clones/ogkm-580.159.04/` (**580.159.04**). No bench, no build, no
GPU. Every row below quotes the line it rests on.

Supersedes, for the *invalidate* question only, the blanket claim *"on our compute path the guest
issues ZERO invalidates"* — see §7, which retracts it. Does **not** supersede
`mode2_address_table.md` §5's ★ CORRECTION as a statement about the **two transports it names**;
it shows those were **the wrong two transports for the RM plane on our chip**.

---

## 0. THE ANSWER IN FIVE LINES

1. ★★★★★ **Every PTE write in both drivers is followed by a TLB invalidate — MAP AS WELL AS
   UNMAP.** The asymmetry is in the **membar**, never in the invalidate.
   ⇒ NVIDIA's GMMU **negative-caches**: *"the GPU TLBs may cache invalid entries using any page
   size they decide"* (`uvm_mmu.c:1534-1536`).
2. ★★★★★ **There are two invalidate transports on GA106 and NEITHER is one of the two we
   measured.** RM's is a **BAR0 MMIO register write at `0xB830B0`**
   (`NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE`); UVM's is a **`MEM_OP` pushbuffer method on UVM's
   own internal channel**. The `INVALIDATE_TLB` GSP-RPC is `_STUB` on GA106 and **cannot fire**.
3. ★★★★★ **The owner's scenario cannot happen for a mapping the guest intends the engine to
   use.** UVM's invalidate rides a pushbuffer push that ends in `write_gpu_put` → **a work-submit
   token write, i.e. a doorbell**. RM's rides an MMIO write we already trap. **A PTE update is
   never silent.** ⊘ But the doorbell is on **UVM's internal channel in a different VAS**, not on
   the compute channel — so a publish keyed on *"the ringing channel's VAS"* misses it.
4. ⊘ **There are FIVE ways a PTE changes with no invalidate, and the guest controls three of
   them.** Headline: `NVOS46/47_FLAGS_DEFER_TLB_INVALIDATION` — *"Improper use can leave stale
   entries in the TLB, and allow access to memory no longer owned by the RM client"*
   (`nvos.h:2146-2148`). Full list in §4.
5. ★★★ **The guest BLOCKS on RM's invalidate** — it spin-polls `TRIGGER` until hardware clears it
   (`kern_gmmu_tu102.c:69-71`). ⇒ the boundary is not merely observable, it is a point at which
   the guest is **already stopped**, so publication done there **cannot race the engine** and
   needs no fence of our own. ⊘ And it is an obligation: if we never clear `TRIGGER`, the guest
   spins to timeout.
6. ⇒ **Soundness verdict (§8):** tier 1 (the guest's invalidate) is **sound and reachable for
   MAP**; it is **NOT sufficient for UNMAP**, because the signal is client-suppressible by design
   and a missed unmap is a stale host-GPU translation into reused guest pages.

---

## 1. ★★★★★ MAP-vs-UNMAP — SETTLED, AND IT IS SETTLED THE OTHER WAY FROM THE USUAL MMU INTUITION

**Question: does a new mapping (invalid → valid) require an invalidate on this hardware?**

### 1.1 YES — and the reason is stated as a hardware property, not a driver choice

`ogkm/kernel-open/nvidia-uvm/uvm_mmu.c:1533-1536`, in `page_tree_set_location`'s `map_remap`:

```c
    // Invalidate all the supported page sizes smaller than or equal to
    // range->page_size, because the GPU TLBs may cache invalid entries using
    // any page size they decide, including the smallest one.
    page_sizes = (range->page_size | (range->page_size - 1)) & tree->hal->page_sizes();
```

⇒ **The GMMU caches non-present entries.** That is the whole question. An MMU that negative-caches
requires an invalidate on INVALID→VALID, and NVIDIA's does.

★ **This is a HARDWARE statement, so it binds the closed driver too.** It is not an ogkm
implementation detail — it is a property of the TLB the closed driver programs identically. (See
§9 for what in this doc is *not* portable that way.)

Corroborated independently in `uvm_va_block.c:6196-6206` — the "never go VALID→VALID" invariant,
which only makes sense if stale entries persist:

```c
    // First make the big PTEs unmapped to disable future lookups of the 4ks
    // under it. We can't directly transition the entry from valid 4k PTEs to
    // valid big PTEs, because that could cause the GPU TLBs to cache the same
    // VA in different cache lines. That could cause memory ordering to not be
    // maintained.
```

And in `uvm_blackwell.c:85-89`, for the ATS/physical side, the same shape restated as a
translation prefetcher hazard:

```c
    // generally this can result in the GPU accessing a stale invalid physical
    // ATS translation after transitioning an IOMMU mapping from invalid to
    // valid, or in other words at dma_map_page() time.
```

★★ **RM corroborates from the other side of the driver, twice, and one of them is decisive.**

- **INVALID → SPARSE needs an invalidate.** Both states are "no translation"; if the TLB did not
  cache the absent entry there would be nothing to flush.
  `src/nvidia/src/kernel/mem_mgr/gpu_vaspace.c:1422-1425`:
  ```c
              // Invalidate TLB to apply new sparse state.
              kbusFlush_HAL(pGpu, pKernelBus, BUS_FLUSH_VIDEO_MEMORY  |
                                              BUS_FLUSH_SYSTEM_MEMORY);
              gvaspaceInvalidateTlb(pGVAS, pGpu, PTE_UPGRADE);
  ```
- **The GMMU speculatively READS page-directory entries it has no request for.**
  `src/nvidia/src/kernel/gpu/mmu/arch/pascal/kern_gmmu_gp100.c:46-50`:
  ```c
  /*!
   * @brief Get the size of PDB allocation for 5-level page table formats
   *
   * Because GMMU can prefetch uninitialized PDB entries and cause XVE to hang,
   * we need to allocate all entries of the PDB regardless of vaLimit.
  ```
  and `src/nvidia/src/kernel/gpu/mmu/gmmu_walk.c:337-344`:
  ```c
                  //
                  // Always scrub the allocation for the PDB allocation in case
                  // GMMU prefetches some uninitialized entries
                  //
  ```
  ⇒ ★★★ **A hardware unit that prefetches directory entries into a cache is a hardware unit that
  holds absent state.** This is why *"the mapping did not exist yet, so nothing can be stale"* is
  false on this GPU.

### 1.2 The asymmetry that DOES exist is the MEMBAR, and it is one function

`ogkm/kernel-open/nvidia-uvm/uvm_va_block.c:6468-6477`:

```c
static uvm_membar_t block_pte_op_membar(block_pte_op_t pte_op, uvm_gpu_t *gpu, uvm_processor_id_t resident_id)
{
    // Permissions upgrades (MAP) don't need membars
    if (pte_op == BLOCK_PTE_OP_MAP)
        return UVM_MEMBAR_NONE;

    UVM_ASSERT(UVM_ID_IS_VALID(resident_id));
    UVM_ASSERT(pte_op == BLOCK_PTE_OP_REVOKE);

    return uvm_hal_downgrade_membar_type(gpu, uvm_id_equal(gpu->id, resident_id));
}
```

Same rule, said twice more:
- `uvm_mmu.c:823-824` — `// Upgrades don't have to flush out accesses, so no membar is needed on the // TLB invalidate.`
- `uvm_map_external.c:243-246` — the CUDA external-allocation map path:
  ```c
    if (last_mapping) {
        // Do a TLB invalidate if this is the last mapping in the VA range
        // Membar: This is a permissions upgrade, so no post-invalidate membar
        //         is needed.
  ```

⇒ ★ **Read that comment carefully: it says "do a TLB invalidate … this is a permissions upgrade,
so no post-invalidate MEMBAR".** The invalidate is unconditional; only the fence is dropped.

### 1.3 RM says the same thing with a type, not a comment

RM carries the direction as a first-class enum, `VAS_PTE_UPDATE_TYPE ∈ {PTE_UPGRADE,
PTE_DOWNGRADE}`, and calls the **same** invalidate entry point for both. Full census of
`gvaspaceInvalidateTlb` call sites in 610 (`src/nvidia/src/kernel/mem_mgr/gpu_vaspace.c` unless
noted): `:1425` UPGRADE, `:1449` UPGRADE, `:1617` DOWNGRADE, `:2725` either
(`bDowngrade ? PTE_DOWNGRADE : PTE_UPGRADE`), `:3009` DOWNGRADE, `:3129` DOWNGRADE, `:3265`
DOWNGRADE, `:3545` UPGRADE, `:4273` DOWNGRADE; `virt_mem_allocator_gm107.c:2570` **either**
(`update_type`), `:3032` UPGRADE; `dma.c:899`/`:1010` DOWNGRADE; `kern_bus.c:823` DOWNGRADE;
`deferred_api.c:523` UPGRADE.

The direction is consumed **only** to decide a membar WAR, exactly as in UVM
(`kern_gmmu_gm107.c:229-233`):

```c
    // Perform membarWAR for non-BAR2 pte downgrades.
    if ((!(vaspaceFlags & VASPACE_FLAGS_BAR_BAR2) && (PTE_DOWNGRADE == update_type)) ||
        bForceSysmemBar)
    {
        flushCount = kgmmuSetTlbInvalidateMembarWarParameters_HAL(pGpu, pKernelGmmu, &params);
    }
```

⇒ ★★★★★ **VERDICT: a new mapping REQUIRES an invalidate on this hardware, and both drivers issue
one. Map and unmap are SYMMETRIC in whether they invalidate, and ASYMMETRIC only in the fence.**

### 1.4 The genuine exceptions — invalidate omitted because the level is UNREACHABLE

UVM omits an invalidate in exactly one class of case: writes to page-table levels the MMU cannot
currently walk to, because a larger entry above them is active. These are **not** counter-examples
to §1.1; they are writes that are not yet mappings.

- `uvm_va_block.c:6565-6567` (verbatim again at `:6681-6683`, `:7062-7064`):
  ```c
    // Since the 2M entry is active as a PTE, the GPU MMU can't fetch entries
    // from the lower levels. This means we don't need to issue a TLB invalidate
    // when writing those levels.
  ```
- `uvm_va_block.c:6750-6752` (again `:6859-6861`, `:7165-7167`): *"No TLB invalidate is needed
  since the big PTE is active."*
- `uvm_va_block.c:9047-9050`: *"These don't need TLB invalidates since the big PTEs above them are
  active."*
- `uvm_va_block.c:6367`: *"If the 2M PTE is already invalid, no TLB invalidate is needed."*

⇒ ★★ **For us this is load-bearing in the opposite direction from how it reads.** These are PTE
bytes that change in guest memory **with no invalidate**, and a design that publishes on the
guest's invalidate will **not** publish them — correctly, because they are not reachable. But a
design that *diffs page-table pages* will see them change and must not conclude a mapping
appeared. **Reachability, not byte-change, is the predicate.** (This is the same invariant
`kayfabe-mmu::reach::ReachShadow::witness_swept` already enforces.)

---

## 2. THE TABLE — every PTE-write path, its invalidate, its transport, its timing

Paths are relative to `research_clones/ogkm/` (610.43.02) unless a row says otherwise. **No row
without a citation.**

| # | who writes the PTE | PTE-write transport | invalidate issued? | invalidate transport | when |
|---|---|---|---|---|---|
| **R1** | RM walker, map — `dmaUpdateVASpace_GF100` → `gvaspaceMap` → `mmuWalkMap` | **CPU MMIO stores through BAR2** (`mem_utils.c:69` default `TRANSFER_TYPE_PROCESSOR`; w289 §2.5) | ★ **YES** | `gvaspaceInvalidateTlb(…, PTE_UPGRADE)` → `kgmmuInvalidateTlb_GM107` → **BAR0 reg `0xB830B0`** | **synchronous, at the tail of the same call**, after `kbusFlush_HAL` — `virt_mem_allocator_gm107.c:2562-2573` |
| **R2** | RM walker, unmap — same function, `update_type = PTE_DOWNGRADE` (chosen at `:2192-2199`) | same (CPU/BAR2) | ★ **YES** | same reg, plus the **membar WAR** + `kbusSendSysmembar` loop (`kern_gmmu_gm107.c:232-236`, `:270-275`) | synchronous, same site |
| **R3** | ⊘ **R1/R2 with `NVOS46/47_FLAGS_DEFER_TLB_INVALIDATION`** | same | ⊘ **NO** | — | **never**, unless the client itself later calls `NV0080_CTRL_CMD_DMA_INVALIDATE_TLB` (`0x80180c`) / `NV2080_CTRL_CMD_DMA_INVALIDATE_TLB` (`0x20802502`) |
| **R4** | ⊘ **R1 with a client-supplied PTE buffer** (`pTgtPteMem != NULL`, the `FILL_PTE_MEM` shape) | writes into the client's buffer, not live tables | ⊘ **NO** | — | guarded out by `(NULL == pTgtPteMem)` at `virt_mem_allocator_gm107.c:2564`. ★ Harmless *as long as* the buffer is not itself a live page table — **the client owns consistency** |
| **R5** | ⊘ **VA free with `bSkipTlbInvalidateOnFree`** | — | ⊘ **NO** | — | `gpu_vaspace.c:1609` gates `:1617`'s DOWNGRADE |
| **R6** | ⊘ **`NV0080_CTRL_CMD_DMA_UPDATE_PDE_2`** — direct client PDE stuffing | RM writes the PDE | ⊘ **only if the client asks** | reg | `gpu_vaspace.c:3545` fires **only** when `_FLAGS_FLUSH_PDE_CACHE` is `_TRUE` (`ctrl0080dma.h:685-688`). Header: *"It is also the client's responsibility to flush/invalidate the MMU when appropriate … This control does not flush automatically to allow batches of calls to be made before a single flush."* (`ctrl0080dma.h:614-618`) |
| **R7** | ⊘ **BAR2 RM-aperture rewrite** — `kbusRewritePTEsForExistingMapping_VBAR2` | CPU/BAR2 | ⊘ **NO** | — | passes `flags == 0` ⇒ `bInvalidate` false (`kern_bus_gm107.c:2569`); `kern_bus_vbar2.c:1291-1292` |
| **R8** | ⊘ **BAR2 RM-aperture unmap into the cache** | — | ⊘ **NO** — the PTEs **stay valid** and the mapping is parked | — | `kern_bus_vbar2.c:763-775`: moved to `cachedMapList`; the invalidate happens only under `TRANSFER_FLAGS_DESTROY_MAPPING`. A re-map is a pure cache hit with no invalidate (`:568-579`) |
| **R9** | RM, BAR2 bootstrap / bus paths / PDB swaps | PRAMIN + BAR2 | ★ YES (DOWNGRADE) | reg | `kern_bus.c:823`; `kern_bus_gm107.c:2774`, `:5836`; `kern_bus_tu102.c:817` passes `DMA_TLB_INVALIDATE` explicitly |
| **R10** | RM fabric VAS map | CPU/BAR2 | ★ YES, **batched** | reg | the per-PTE calls pass `DMA_DEFER_TLB_INVALIDATE` (`fabric_vaspace.c:1230`) and **one** invalidate follows the loop (`:1244`) — ★ RM's own precedent for deferral-then-batch |
| **U1** | UVM PTE write, **CE path** (the normal one on discrete GA106) — `uvm_page_table_range_vec_write_ptes_gpu` | **CE `memcopy`/`memset_8` with the PTE bytes inline in the pushbuffer** (`uvm_pte_batch.c:54-59`, `:71-76`) | ★ **YES** | `MEM_OP_A..D`, `OPERATION = MMU_TLB_INVALIDATE(_TARGETED)`, class `C56F` (`uvm_ampere_host.c:247-258`, `:350-363`) | **in the LAST push of the sequence** — `uvm_mmu.c:2219-2226`; earlier pushes carry only a membar (`:2228-2234`) |
| **U2** | UVM PTE clear (unmap) — `uvm_page_table_range_vec_clear_ptes_gpu` | CE, same | ★ YES | same | last push only: `uvm_mmu.c:2042-2043` |
| **U3** | UVM PTE write, **CPU path** (`uvm_mmu_use_cpu`, `uvm_mmu.c:269-277`: vidmem **and** `!ce_phys_vidmem_write_supported` **and** flat mapping not ready — i.e. bootstrap / coherent platforms) | **CPU stores** `uvm_mmu_page_table_cpu_memset_8/16` | ★ **YES** | still a **pushbuffer** `MEM_OP` — a push is opened *solely* to carry it | write: `uvm_mmu.c:2117-2147` (`mb()` at `:2117-2119`); clear: `:1959-1991`, whose `:1980-1981` is the contract — `// A CPU membar is needed between the PTE writes and the subsequent TLB // invalidate. Work submission guarantees such a membar.` (PDE twin at `:732-733`) |
| **U4** | UVM external-allocation map (**the CUDA path**, `UVM_MAP_EXTERNAL_ALLOCATION`) | CE on `UVM_CHANNEL_TYPE_MEMOPS` (`uvm_map_external.c:230-241`) | ★ YES | `MEM_OP` | at `last_mapping`, `uvm_map_external.c:243-252` |
| **U5** | UVM external-allocation unmap | CE clear | ★ YES, **with a downgrade membar** | `MEM_OP` + membar chosen by `va_range_downgrade_membar` | `uvm_map_external.c:1302-1306`, `:510` |
| **U6** | UVM PDE write — `uvm_page_tree_write_pde` | CE inline (`uvm_mmu.c:425-432`) or CPU (`:352-359`) | ⊘ **NOT by the callee** | — | `uvm_mmu.h:418-419`: `// This function performs no TLB invalidations.` **The caller must**; e.g. `uvm_va_block.c:6331-6336` |
| **U7** | UVM sub-level writes under an active big/2M entry | CE | ⊘ **NO — and correctly so** | — | §1.4 |
| **U8** | UVM ATS fault servicing | n/a (IOMMU) | ★ YES, **batched across a whole fault batch** | `MEM_OP` in a **separate dedicated push** | `uvm_ats_faults.c:138-143` begins the batch in the fault handler; `:741-752` ends it in its own push at replay time |
| **U9** | UVM PDB teardown (ATS PASID reuse) | no PTE write at all | ★ YES | `tlb_invalidate_all` | `uvm_mmu.c:1209-1225` |
| **X1** | ★ **explicit client invalidate, no PTE write of its own** — `NV0080/NV2080_CTRL_CMD_DMA_INVALIDATE_TLB` | — | YES | reg, **always as `PTE_DOWNGRADE`** | `dma.c:960-1005` / `:864-901`, both with the comment *"Although this function is used following PTE upgrades most of the time, we cannot guarantee that, nor can we easily determine the update type."* ⇒ conservative full-membar path. **Exposed to UVM** as `nvGpuOpsInvalidateTlb` (`nv_gpu_ops.c:9233-9246`, exported `nv_gpu_ops.h:194`) — ⊘ **unused by UVM in this tree**, but it is a live ioctl surface |
| **X2** | ★ **invalidate triggered FROM the pushbuffer** — `NV50_DEFERRED_API_CLASS` (0x5080) SW method carrying `NV2080_CTRL_CMD_DMA_INVALIDATE_TLB` | — | YES (`PTE_UPGRADE`) | SW method traps to CPU → reg | `deferred_api.c:483`, `:523-527`. ⚠ **The only path where a pushbuffer parser sees an invalidate request that is NOT a `MEM_OP`** — and it arrives as a class-5080 SW method, which a `MEM_OP`-only decoder will miss |
| **G1** | anything, **under vGPU with `VF_INVALIDATE_TLB_TRAP_ENABLED`** | — | YES | `NV_RM_RPC_INVALIDATE_TLB` (fn 200) | `kern_gmmu_gm107.c:247-249`, gated `bDoVgpuRpc` (`:151-159`) — ⊘ **NOT our configuration, and `_STUB` on GA106 regardless** |

### 2.1 The transport rows, quoted

**RM → MMIO register.** `src/nvidia/src/kernel/gpu/mmu/arch/turing/kern_gmmu_tu102.c:112-120`:

```c
    if (!FLD_TEST_DRF(_VIRTUAL_FUNCTION_PRIV, _MMU_INVALIDATE, _ALL_PDB, _TRUE, pParams->regVal))
    {
        kgmmuSetPdbToInvalidate_HAL(pGpu, pKernelGmmu, pParams);
    }

    GPU_VREG_WR32(pGpu, NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE, pParams->regVal);

    // Wait for the invalidate command to complete.
    status = kgmmuCheckPendingInvalidates_HAL(pGpu, pKernelGmmu, &pParams->timeout);
```

★★★ **And there is no GSP branch anywhere in it.** `kgmmuInvalidateTlb_GM107` is the **only**
non-stub HAL variant for every chip except T234D/T239D/T264D (`generated/g_kern_gmmu_nvoc.c:528-537`);
there is no `kgmmuInvalidateTlb_GA100`. Its only early bail is for **paravirt guests**, not for GSP
clients (`kern_gmmu_gm107.c:129-135`):

```c
    if (API_GPU_IN_RESET_SANITY_CHECK(pGpu) ||
        IS_VIRTUAL_WITHOUT_SRIOV(pGpu) ||
        (IS_VIRTUAL(pGpu) && gpuIsWarBug200577889SriovHeavyEnabled(pGpu)))
    {
        return status;
    }
```

⇒ **A GSP-offload CPU-RM takes the exact same MMIO path as a non-GSP driver.** The invalidate never
crosses to the GSP on any configuration we support. Blackwell is identical
(`kern_gmmu_gb100.c:60-64`).

The completion poll is a spin on the same register (`kern_gmmu_tu102.c:69-71`) — so we will see a
**read** of `0xB830B0` immediately after the write, which is a second, redundant signal:

```c
        regVal = GPU_VREG_RD32(pGpu, NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE);

        if (FLD_TEST_DRF(_VIRTUAL_FUNCTION_PRIV, _MMU_INVALIDATE, _TRIGGER, _FALSE, regVal))
            break;
```

⚠ **That poll is a correctness obligation on US, not just an observation.** If we decode the write
and never clear `TRIGGER`, the guest **spins to timeout**. Whatever we do with the signal, the
register must read back with `TRIGGER = FALSE` once we have applied it — and that read-back is our
natural place to make publication synchronous with the guest's own wait.

The address, derived from ogkm's own headers:
- `GPU_VREG_WR32(g,a,v) = GPU_REG_WR32(g, g->sriovState.virtualRegPhysOffset + a, v)`
  (`generated/g_gpu_access_nvoc.h:257`)
- `gpuGetVirtRegPhysOffset_TU102` returns `DRF_BASE(NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET)` when
  not a VF (`kern_gpu_tu102.c:96-100`), and that is `0x00B80000`
  (`src/common/inc/swref/published/turing/tu102/dev_vm.h:28`)
- `NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE = 0x000030B0`
  (`src/common/inc/swref/published/ampere/ga100/dev_vm.h:63`)

⇒ ★★★★★ **BAR0 `0xB830B0`**, with the PDB in `_MMU_INVALIDATE_PDB` / `_UPPER_PDB` written first
(`kern_gmmu_tu102.c:143-153`) and the trigger in bit 31
(`_MMU_INVALIDATE_TRIGGER 31:31`, `ga100/dev_vm.h:122`). The scope bits `_ALL_VA` (0:0) and
`_ALL_PDB` (1:1) are at `:64` and `:67`.

★ For orientation, the doorbell we already decode is the **same BAR0 window**:
`NV_VIRTUAL_FUNCTION_DOORBELL = 0x30090` (`ga100/dev_vm.h:131`) ⇒ `0xBB0090`, which is exactly
`NVKVM_VF_DOORBELL` in the C (`src/qemu/mode2_regs_ga10x.h:98`, *"CONFIRMED in trace"*).
**The invalidate register is 448 KiB below a register we have decoded since M5.**

**UVM → pushbuffer method.** `uvm_ampere_host.c:247-258`:

```c
    NV_PUSH_4U(C56F, MEM_OP_A, sysmembar_value |
                               HWCONST(C56F, MEM_OP_A, TLB_INVALIDATE_INVAL_SCOPE, NON_LINK_TLBS),
                     MEM_OP_B, 0,
                     MEM_OP_C, HWCONST(C56F, MEM_OP_C, TLB_INVALIDATE_PDB, ONE) |
                               HWVALUE(C56F, MEM_OP_C, TLB_INVALIDATE_PDB_ADDR_LO, pdb_lo) |
                               ...
                     MEM_OP_D, HWCONST(C56F, MEM_OP_D, OPERATION, MMU_TLB_INVALIDATE) |
                               HWVALUE(C56F, MEM_OP_D, TLB_INVALIDATE_PDB_ADDR_HI, pdb_hi));
```

**No third transport exists in UVM.** An exhaustive sweep of `kernel-open/nvidia-uvm/` for
`NV0080_CTRL_DMA_INVALIDATE_TLB`, `nvUvmInterfaceFlush*` and `uvm_rm_locked_call` finds **zero**
RM-mediated TLB invalidates; every `uvm_rm_locked_call` hit is channel/TSG/CSL/fault-buffer/PMA
setup. Even the test ioctl pushes a method (`uvm_mmu.c:3048-3049`).

### 2.2 Batching, and what flushes a batch

`uvm_tlb_batch_begin` / `_invalidate` are **pure bookkeeping** — nothing is emitted until
`uvm_tlb_batch_end` (`uvm_tlb_batch.c:97-108`). Two triggers degrade a batch to **invalidate-all
on that PDB** (`uvm_tlb_batch.c:89-95`):

```c
static bool tlb_batch_should_invalidate_all(uvm_tlb_batch_t *batch)
{
    if (batch->count > UVM_TLB_BATCH_MAX_ENTRIES)
        return true;

    return batch->total_ranges > batch->tree->gpu->parent->tlb_batch.max_ranges;
}
```

`UVM_TLB_BATCH_MAX_ENTRIES = 4` (`uvm_tlb_batch.h:36`); `max_ranges = 8` on Ampere
(`uvm_ampere.c:33`). And the HAL degrades again on its own when a targeted range spans the address
space (`uvm_ampere_host.c:296-303`).

⇒ ★★ **Good news for us: the common case of a big remap arrives as a single "this whole PDB
changed" event.** That is precisely the shape our whole-VAS sweep already consumes.

---

## 3. ★★★★★ THE OWNER'S EXACT SCENARIO, ANSWERED

> *"what if the pte is updated between 2 commands on gpfifo and doorbell isn't rung since libcuda
> expects the ring is emptied? does the kernel then execute tlb invalidate for us?"*

**The kernel DOES issue the invalidate — and on the UVM plane the invalidate itself RINGS A
DOORBELL. There is no silent window for a mapping the guest intends the engine to use.** The
chain, each link quoted:

1. The PTE write and its invalidate go in a **push** — `page_tree_begin_acquire`
   (`uvm_mmu.c:55-67`) on `UVM_CHANNEL_TYPE_GPU_INTERNAL`, or `UVM_CHANNEL_TYPE_MEMOPS`
   (*"Memops and small memsets/copies for writing PTEs"*, `uvm_channel.h:87-88`) for external
   allocations and SR-IOV heavy.
2. The push is closed by `uvm_channel_end_push` (`uvm_channel.c:1492`), which calls
   `internal_channel_submit_work` (`:1577`).
3. `internal_channel_submit_work` writes the GPFIFO entry, `mb()`, then
   `gpu->parent->host_hal->write_gpu_put(channel, new_gpu_put)` (`uvm_channel.c:1006-1014`).
4. `uvm_hal_turing_host_write_gpu_put` (`uvm_turing_host.c:222-235`) — **this is the doorbell**:

```c
    UVM_GPU_WRITE_ONCE(*channel->channel_info.gpPut, gpu_put);

    wmb();

    UVM_GPU_WRITE_ONCE(*channel->channel_info.workSubmissionOffset, channel->channel_info.workSubmissionToken);
```

   Ampere inherits this: `uvm_hal.c:187-194` overrides only `method_is_valid`,
   `sw_method_is_valid`, `clear_faulted_*`, `tlb_invalidate_*` and `l2_invalidate` — **not**
   `write_gpu_put`, which stays `uvm_hal_turing_host_write_gpu_put` (`uvm_hal.c:163`).

5. Several of the map/unmap paths additionally **block** until the GPU has consumed it —
   `page_tree_end_and_wait` (`uvm_mmu.c:2148`, `:1991`). So the mapping is not merely submitted,
   it is *retired*, before the ioctl returns.

⇒ ★★★ **So the answer to "does the kernel invalidate for us" is YES, and the answer to "is there a
doorbell" is ALSO YES — but it is the WRONG DOORBELL.**

⊘ **THE TRAP, AND IT IS THE REASON THIS RUNG MATTERS.** The doorbell that accompanies a PTE change
is rung on **UVM's internal channel**, which lives in **UVM's own page tree**, not in the user's
compute VAS. A publish keyed on *"sweep the VAS of the channel whose doorbell was rung"* sees a
doorbell **at the right instant on the wrong address space** and publishes nothing.

⇒ **A doorbell-triggered publish must be triggered by ANY doorbell and must consider EVERY dirty
VAS — not the ringing channel's.** Our current `plan_pt_sweep` triggers (never-swept / truncated /
dirty) already have the right shape; what must not creep in is a per-channel scoping of *which*
VAS gets swept.

### 3.1 The narrower version of the owner's question — is there a window at all?

Yes, exactly two, and both are named:

- **W1 — the RM defer flag (§4).** A client may ask RM to write PTEs and skip the invalidate. Then
  there is **no** signal of any kind until the client chooses to call `DMA_INVALIDATE_TLB`.
- **W2 — sub-level writes under an active larger entry (§1.4).** Bytes change with no invalidate,
  by design, because the MMU cannot reach them. **Not a hazard** — until the larger entry is
  cleared, and *that* clear does carry an invalidate.

⊘ **Neither is "the ring was already non-empty".** The premise of the owner's scenario — a PTE
updated while the engine keeps running off an un-rerung doorbell — is real for the **compute**
channel, but the PTE update is not silent: it is accompanied by its own submit on the driver's
channel. The exposure is *"we looked at the wrong channel"*, not *"nothing was emitted."*

---

## 4. ⊘⊘⊘ THE ESCAPE HATCHES — FIVE WAYS A PTE CHANGES WITH NO INVALIDATE, AND THE GUEST CONTROLS THREE

Ranked by how much they threaten a tier-1 boundary:

| # | hatch | who controls it | row |
|---|---|---|---|
| **E1** | `NVOS46/47_FLAGS_DEFER_TLB_INVALIDATION` on map/unmap | ★★★ **the guest**, one bit in an ioctl we forward | R3 |
| **E2** | `UPDATE_PDE_2` without `_FLUSH_PDE_CACHE` | ★★★ **the guest**, one bit in a control we forward | R6 |
| **E3** | `FILL_PTE_MEM`-shaped call with a client PTE buffer | ★★ **the guest**, by supplying `pTgtPteMem` | R4 |
| **E4** | BAR2 RM-aperture rewrite / cached unmap | ⊘ RM-internal, not guest-reachable | R7, R8 |
| **E5** | writes below an active larger entry | ⊘ UVM-internal, and **safe by construction** | U7 / §1.4 |

⇒ **E1–E3 are guest-controlled and E4 is RM-internal but still a real "PTE changed, no invalidate"
event in the host's own driver.** E5 is not a hazard.

### 4.1 E1, in NVIDIA's own words

`src/common/sdk/nvidia/inc/nvos.h:2145-2152`:

```c
//
// This flag must be used with caution. Improper use can leave stale entries in the TLB,
// and allow access to memory no longer owned by the RM client or cause page faults.
// Also see corresponding flag for NvUnmapMemoryDma.
//
#define NVOS46_FLAGS_DEFER_TLB_INVALIDATION                        31:31
```

and its unmap twin, `nvos.h:2191`:

```c
#define NVOS47_FLAGS_DEFER_TLB_INVALIDATION                        0:0
```

Consumed at `virt_mem_allocator_gm107.c:414-415` (map):

```c
    pLocals->deferInvalidate    = FLD_TEST_DRF(OS46, _FLAGS, _DEFER_TLB_INVALIDATION, _TRUE, flags) ?
                                               DMA_DEFER_TLB_INVALIDATE : DMA_TLB_INVALIDATE;
```

and `:1574` (unmap):

```c
    deferInvalidate = DRF_VAL(OS47, _FLAGS, _DEFER_TLB_INVALIDATION, flags) ? DMA_DEFER_TLB_INVALIDATE : DMA_TLB_INVALIDATE;
```

and gating the invalidate at `virt_mem_allocator_gm107.c:2563-2571`:

```c
done:
    // Invalidate VAS TLB entries.
    if ((NULL == pTgtPteMem) && DMA_TLB_INVALIDATE == deferInvalidate)
    {
        NV_STATUS tlbStatus;

        kbusFlush_HAL(pGpu, pKernelBus, BUS_FLUSH_VIDEO_MEMORY |
                                        BUS_FLUSH_SYSTEM_MEMORY);
        tlbStatus = gvaspaceInvalidateTlb(pGVAS, pGpu, update_type);
```

**There is no deferred-flush bookkeeping.** RM does not remember that an invalidate is owed; the
contract is that the client will issue `NV0080_CTRL_CMD_DMA_INVALIDATE_TLB` (`0x80180c`) or
`NV2080_CTRL_CMD_DMA_INVALIDATE_TLB` (`0x20802502`) itself. The control's own header says so
(`ctrl0080dma.h:455-461`):

```
 * This command invalidates the GPU TLB. This is intended to be used
 * for RM clients that manage their own TLB consistency when updating
 * page tables on their own, or with DEFER_TLB_INVALIDATION options
 * to other RM APIs.
```

⇒ ★★★★★ **THIS IS THE DECIDING FACT FOR THE ARCHITECTURE.** A guest can, **using a documented,
non-privileged RM API**, unmap a range and tell RM not to invalidate. If our publish/unpublish
boundary is the guest's invalidate, that guest keeps a **host-GPU translation alive into guest
pages it has freed** — with NVIDIA's own header describing the consequence as *"allow access to
memory no longer owned by the RM client."*

⊘ **In-tree usage today is `_FALSE`** (`virtual_mem.c:1406`, `:1448` both pass
`DRF_DEF(OS47, _FLAGS, _DEFER_TLB_INVALIDATION, _FALSE)`), and it is not on the CUDA path we have
traced. **That is a statement about a cooperative guest, not about the threat model.** A hostile
guest is in scope (`hostile_guest_isolation_is_the_value_proposition`), and this flag is one bit in
an ioctl we forward.

---

## 5. ★★★★★ THE OTHER INVALIDATE — WHAT *WE* OWE THE HOST GPU, WHICH IS A DIFFERENT QUESTION

The coordinator is right that the vocabulary merges two unrelated things. Naming them:

- **INV-G — the GUEST's invalidate.** A signal we may *observe* and use as a publish trigger.
  Everything above is about INV-G.
- **INV-H — the invalidate WE owe the HOST GPU** after our publication changes a host mapping.
  This is a correctness obligation **in every tier**; it does not go away if INV-G is a perfect
  trigger.

**Answer for INV-H, as our stack is built today: the obligation is DISCHARGED BY THE HOST DRIVER,
because we never write a host PTE.** We publish by calling the host RM through the isolate —
`NV_ESC_RM_MAP_MEMORY_DMA` (`kayfabe-isolate-host/src/rm.rs`, `raw_map_dma`) and
`NV_ESC_RM_UNMAP_MEMORY_DMA` (`rm.rs:2147-2162`). Those land in the host's own
`dmaUpdateVASpace_GM107`, which invalidates at its tail (row R1/R2 above) — **on the host RM's
side of the boundary, with the host's own PDB and the host's own MMIO register.**

⇒ **Three conditions keep that true, and all three are ours to hold:**

1. ⊘ **We must never pass `NVOS46/47_FLAGS_DEFER_TLB_INVALIDATION` through to the host.** If a
   guest sets bit 31 of an `NVOS46` flags word and we forward it verbatim, **we hand the guest
   control of the host's TLB coherence.** ⚠ **This must be a named refusal / mask in the forward
   path.** I did not find one; treat as an open item until someone greps the forward path for
   `NVOS46` flag sanitisation. (Not measured in this rung — stated as a mechanism with both sides
   cited.)
2. ⊘ **We must actually issue the host unmap.** `reap_retired` has **zero production callers**
   (banked: `cancellation_is_not_built_and_preempt_is_forged`). A host mapping we never unmap is
   never invalidated either — **INV-H is vacuously satisfied and the leak is total.** This is the
   live defect, and it is on the same axis as §4.
3. If we ever write host page tables directly (we do not today), INV-H becomes ours and there is
   **no** mechanism in either tree for an unprivileged process to write `0xB830B0` on the host.
   ⇒ **A design that needs its own host invalidate is a design that needs privilege.** That is a
   strong argument for keeping publication inside RM's map/unmap verbs.

★ **Verdict on INV-H: not currently a defect in the "we forgot to invalidate" sense, and a real
defect in the "we forgot to unmap" sense.** It is not a second explanation for the intermittent
faults — the host RM invalidates whenever it maps — but item 1 is a live hole in the *guest-hostile*
direction that nobody has closed.

---

## 6. 580.159.04 vs 610.43.02 — what differs

Diffed directly. **On GA106 the invalidate discipline is IDENTICAL.** Differences found:

| area | 580.159.04 | 610.43.02 | matters to us? |
|---|---|---|---|
| `tlb_batch_should_invalidate_all` | three-way: `!va_invalidate_supported` ⇒ all; `count > 4` ⇒ all; then `va_range_invalidate_supported ? total_ranges > max_ranges : total_pages > max_pages` | two-way: `count > 4`; `total_ranges > max_ranges` | ⊘ **NO.** The dropped arms are the Maxwell (`va_invalidate_supported = false`, `uvm_maxwell.c:30`) and Pascal (`va_range_invalidate_supported = false`, `uvm_pascal.c:36`) cases. **610 dropped pre-Turing entirely** — `uvm_maxwell*.c` / `uvm_pascal*.c` exist in 580 and are **absent** in 610. Ampere takes the same arm in both (`uvm_ampere.c`: `va_invalidate_supported = true`, `va_range_invalidate_supported = true`, `max_ranges = 8`). |
| `UVM_TLB_BATCH_MAX_ENTRIES` | 4 (`uvm_tlb_batch.h:36`) | 4 (`uvm_tlb_batch.h:36`) | no |
| Ampere `MEM_OP` encoding | identical | identical | no — the only diff is a comment (`"Pascal-Ampere" → "Turing-Ampere"`, `uvm_ampere_host.c:230`) |
| `clear_faulted_channel_register` | pokes `NV_RUNLIST_INTERNAL_DOORBELL` computed from `runlist_pri_base_register` | pokes `user_channel->work_submission_offset` with `work_submission_token` | ⊘ not an invalidate path, but ★ **note the direction: 610 moved a doorbell poke from a runlist register to the work-submit token** — anyone modelling faulted-channel clears must version that |
| `rpcInvalidateTlb` STUB list | `TU10X, GA100, GA102…GA106…AD102…GB20C, T234D, T26XD` (`g_rpc_private.h:323`) | same set (`g_rpc_private.h:320`) | no — **GA106 is STUBbed in both** |
| RM `gvaspaceInvalidateTlb` call sites | 14, same functions | 14, same functions | no (line numbers shift only) |

⇒ ★ **The 580-vs-610 answer is "nothing that changes our design."** ⚠ But the shape of the one
real difference is worth carrying: **610's simplification is only valid because it dropped
hardware**. On a Maxwell part, `va_invalidate_supported = false` means **every** invalidate is an
invalidate-all. Our Turing+ floor (`support_matrix_asymmetry`) makes that moot; a future floor
change would not.

---

## 7. ⊘⊘⊘ ADJUDICATING THE MEASURED ZERO — MY OVER-CLAIM WAS WRONG, AND WRONG IN THE WORST OF THE THREE POSSIBLE WAYS

**The claim under audit** (mine, made to the owner): *"on our compute path the guest issues ZERO
invalidates — you cannot hook the guest's invalidate as your commit boundary, it never arrives."*

**The evidence it cited** — `mode2_address_table.md` §5 ★ CORRECTION (2026-07-22, audit S3):
`INVALIDATE_TLB` RPC fn=200 = 0; `MEM_OP`/`MMU_TLB_INVALIDATE` pushbuffer method = 0;
`DMA_FILL_PTE_MEM` = 0.

**The verdict: (b) — an artefact, though not of emulation. It is an artefact of MEASURING THE
WRONG TWO TRANSPORTS.** Broken out:

- ★★★★★ **The `INVALIDATE_TLB` RPC zero is not evidence of anything. It is a STUB.**
  `src/nvidia/generated/g_rpc_private.h:320`:
  ```
  RpcInvalidateTlb                   rpcInvalidateTlb_STUB;    // TU10X, GA100, GA102, GA103, GA104, GA106, GA107, AD10X, GH10X, GB100, …
  ```
  `rpcInvalidateTlb_v23_03` is installed **only** by the vGPU RPC-version HAL
  (`g_rpc_private.h:3049-3050`), and reached **only** when `bDoVgpuRpc` is set, which requires
  `IS_VIRTUAL(pGpu)` **and** `VGPU_DEV_CAPS_VF_INVALIDATE_TLB_TRAP_ENABLED`
  (`kern_gmmu_gm107.c:155-159`). **We are a GSP client, not a vGPU guest.** The count could never
  have been anything but zero. ⇒ *A census over a code path that cannot execute is a
  known-negative, not a measurement* — the exact class of
  `a_census_zero_needs_a_known_positive`.
- ★★★★★ **The `MEM_OP` zero was measured on the right transport for UVM and the wrong one for RM.**
  RM's invalidate on GA106 is `GPU_VREG_WR32(… NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE …)`
  (`kern_gmmu_tu102.c:117`) — **a BAR0 MMIO store**, which no pushbuffer parser will ever see.
  And **nothing in either of our trees decodes it**: `grep -rn MMU_INVALIDATE src/qemu/ ../nvkvm-rs/crates/`
  returns **zero** hits in both. ⇒ The signal has been arriving at our BAR0 handler this whole
  time and falling through as an unmodelled register.
- The `MEM_OP` zero for **UVM** is, as far as it goes, a real observation — but it is an
  observation about *one workload on one path*, and §3 shows why UVM's invalidates could easily
  sit outside a parser scoped to the compute channel: they are pushed on **UVM's internal
  channel**.

⇒ ★★★★★ **PLAINLY: MY ADVICE TO THE OWNER WAS ACTIVELY MISLEADING.** The invalidate **is**
available to us. It is a single trappable BAR0 word for the entire RM plane, sitting 448 KiB from a
register we have decoded since M5. *"It never arrives"* was false; the truth is *"we never looked
where it arrives."*

⚠ **And note how it survived.** The claim carried a citation to a document in this tree, and the
document was **right about what it said** — it reported two transports at zero. What the claim did
was **drop the scope clause** (*"Read-at-invalidate still governs the kernel/UVM/RM paths, where
the transports do appear"*) and then **generalise a two-transport census into a claim about all
transports.** Same class as this tree's standing trap: **citing the oracle is not the oracle being
right**, and here the oracle was right and the *reading* generalised past it.

---

## 8. ★★★★★ SOUNDNESS VERDICT, AND WHICH TIER IS REACHABLE

The owner's ordering:

> **exact boundary from the GPU (e.g. TLB invalidate) > trapping write on PTE, as little as
> possible > deferred publish on doorbell > work under BQL in a doorbell.**

**★ THE ORDERING MUST BE ASYMMETRIC, and the source says why.** Ranked per direction:

### MAP (invalid → valid) — tier 1 is sound and reachable

- A missed trigger is **fail-safe**: the engine walks an unpublished mapping and faults. A GPU
  fault is contained (`gpu_fault_is_contained`: a bystander ran 2 675 519 verified iterations
  through one).
- The trigger exists, is exact, and is cheap: **BAR0 `0xB830B0` for RM**, **`MEM_OP` in the
  pushbuffer for UVM**. Both already reach code we own.
- ⇒ **Tier 1 REACHABLE for map.**

### UNMAP / downgrade (valid → invalid) — tier 1 is NOT sufficient, and §4 is why

- A missed trigger is **fail-dangerous**: a live host-GPU translation into guest pages the guest
  has freed and Linux has reused.
- ★★★★★ **The trigger is CLIENT-SUPPRESSIBLE BY DOCUMENTED API.**
  `NVOS47_FLAGS_DEFER_TLB_INVALIDATION` lets the guest unmap **with no invalidate at all**, and
  NVIDIA's own header names the consequence as *"allow access to memory no longer owned by the RM
  client"* (`nvos.h:2146-2148`). A guest that sets one bit removes our entire commit boundary.
- ⊘ This is **not** a hostile-guest hypothetical requiring a driver rewrite: it is a flag in an
  ioctl we already forward.
- ⇒ **Tier 1 is an OPTIMISATION on the unmap side and can never be the guarantee. Tier 2 (observe
  the write) is REQUIRED for unmap.**

### ⇒ THE ARCHITECTURE THAT FOLLOWS

| direction | mechanism | tier | why |
|---|---|---|---|
| **MAP** | publish on the guest's invalidate — decode BAR0 `0xB830B0` (RM) and `MEM_OP D.OPERATION ∈ {0x9, 0xa}` (UVM) | **1** | exact, cheap, fail-safe if missed |
| **UNMAP** | **unpublish on the observed PTE write**, latched O(1) at the sink (`FbStore::writes_by`, w318), committed later | **2** | the only mechanism that does not depend on guest cooperation |
| both, fallback | doorbell-driven dirty re-sweep | 3 | already built; keeps the self-healing bound of the `mode2_address_table.md` §6 ruling |

★ **And the tier-1-vs-tier-2 gap is smaller than the ordering implies.** Tier 2 is *trap → mark →
commit later*, not *do the work at the write*: one atomic on a write we already trap. Weighed
against tier 1's dependency on a signal the guest can legally withhold, **tier 2 for unmap is not
a concession — it is the cheaper of the two once correctness is priced in.**

### ⚠ TIER 1 IS NOT AUTOMATICALLY CHEAPER THAN TIER 4 — the axis is FREQUENCY, not mechanism

⊘ **Be honest about the shape.** *"Publish in the `0xB830B0` trap handler"* is, mechanically,
**exactly** the thing the owner ranked fourth: work on the vCPU thread inside an MMIO trap, under
the BQL. It is not a different kind of operation from the doorbell handler w315 measured at
**86.7 ms of a 90.9 ms `cuLaunchKernel`**.

★ **What makes it better is the RATE, and that is the whole win.** w315's breakdown of that
handler is `vas_publish` 55.7 % + `pt_decode` 25.7 % = **91.5 % page-table-and-publication work**,
against a **4.1 %** real host forward. That work is currently paid **per launch**. Moving it to
the invalidate pays it **per mapping change** — and a steady-state compute loop maps once and
launches thousands of times. ⇒ **Tier 1's value is that it deletes the sweep from the launch path,
not that the trap is cheap.** And the guest is blocked either way, so the latency is not hidden;
it is *moved to where it is rare*.

⚠ **This is a prediction, not a measurement.** The falsifier is the same single boot named in §11:
count `0xB830B0` writes per `cuLaunchKernel`. If that ratio is not ≪ 1, the argument collapses and
tier 1 buys nothing over tier 3.

### Is a doorbell-triggered publish SOUND?

**For MAP: yes, with one correction to how it is scoped.** §3 establishes that no engine-visible
mapping change reaches the GPU without *some* submit — so a publish that runs on **any** doorbell
and considers **every** dirty VAS cannot be beaten to the GPU by a mapping. ⊘ **It is UNSOUND if
scoped to the ringing channel's VAS**, because the PTE-carrying submit is on the driver's internal
channel in a different address space. **Name the exact window: between UVM's internal-channel
doorbell (which publishes the PTEs and the invalidate) and the compute channel's next doorbell,
the compute VAS holds mappings our table lacks — and a compute channel with `GP_GET != GP_PUT` is
running the whole time.** Fixing the scope closes it; fixing the trigger is not required.

**For UNMAP: no.** Not because of a race, but because §4's flag means an unmap can complete with
neither an invalidate nor a doorbell of its own, and the compute channel may never ring again
before the guest frees and reuses the pages.

---

## 9. ⚠ WHAT IN THIS DOC IS ogkm-SPECIFIC AND WHAT IS HARDWARE

The closed driver is the other half of the support matrix and we cannot read it. Splitting:

**Hardware / architectural — binds both drivers:**
- The GMMU **negative-caches** (§1.1) ⇒ an invalidate is required on map. This is a property of
  the TLB, stated as such in ogkm's comment.
- The **register** `NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE` at BAR0 `0xB830B0` and its
  `TRIGGER`/`ALL_VA`/`ALL_PDB`/`PDB` fields — these are `dev_vm.h` hardware definitions
  (`ampere/ga100/dev_vm.h:63-124`), not driver code. **Any driver that invalidates on Ampere
  writes this register.**
- The **`MEM_OP_A..D` / `MMU_TLB_INVALIDATE` method encoding** on class `C56F` — a hardware method,
  same for anyone.
- The **`NVOS46/47` flag bit positions and semantics** (§4) — a published ABI, in the SDK headers
  the closed driver ships too.

**ogkm-specific — do NOT build a guarantee on these:**
- *Where* in the call graph the invalidate is issued (`virt_mem_allocator_gm107.c:2570`) and the
  fact that it is synchronous with the ioctl.
- UVM's **batching parameters** (4 entries, 8 ranges) and its choice of channel type.
- The fact that UVM's PTE writes use CE rather than CPU stores on our part
  (`uvm_mmu_use_cpu`, `uvm_mmu.c:269-277`) — a policy, and one that flips on coherent platforms.
- ⚠ **`RMCFG_FEATURE_PLATFORM_GSP == 0` in this tree** (w289): this doc tells you what CPU-RM
  *sends*; it cannot tell you what GSP-RM does on receipt. Nothing above depends on that, because
  the invalidate never crosses to the GSP on our chip — but a future finding might.

⇒ ★ **The two facts our architecture rests on — "map requires an invalidate" and "the invalidate is
a BAR0 register write we already trap" — are both on the hardware side of that line.** The
scheduling details are not, and the design must not require them.

---

## 10. WHAT THIS RUNG FOUND THAT WAS ALREADY IN THE TREE, AND WHAT IS NEW

**Already there, and it was right:** w289's `ogkm_mapping_insert_and_teardown_census.md` §2.5
established that **RM writes PTEs as CPU stores through BAR2** and UVM through **its own CE
pushbuffer**, and that a CE-write watcher is **structurally blind to RM**. This rung is the
invalidate-side twin of that finding and reaches the same shape: **the RM plane's signal is a CPU
transport we were not decoding.**

**New here:**
1. The invalidate register, its BAR0 address, and the fact that **nothing in either tree decodes
   it** (`grep` = 0 in both).
2. `rpcInvalidateTlb` is `_STUB` on GA106 ⇒ the RPC census zero was structurally guaranteed.
3. The **map-vs-unmap symmetry** settled from the negative-caching comment.
4. `NVOS46/47_FLAGS_DEFER_TLB_INVALIDATION` as the client-suppressible escape hatch, with NVIDIA's
   own hazard statement.
5. The UVM **doorbell** chain — that a PTE update is never silent, but rings on the wrong channel.

---

## 11. ⊘ WHAT I DID NOT ESTABLISH

- **Not measured, only derived.** No boot, no trace. The claim *"a guest running CUDA writes
  `0xB830B0`"* is a source-level derivation; the falsifier is a single boot with a decoder on that
  offset. **That is the first thing to run.** Pre-register three numbers, not one
  (`falsifier_blocker_vs_only_blocker`): **(a)** total `0xB830B0` writes over a `cup3` run — a
  zero refutes the whole doc; **(b)** writes **per `cuLaunchKernel`** — this is the tier-1 perf
  argument and it needs to be ≪ 1; **(c)** how many carry `_ALL_PDB = TRUE` (whole-GPU) vs a
  specific PDB — the latter is what makes the boundary *scoped* rather than a blunt re-sweep.
- **The `MEM_OP` side is equally underdecoded and equally testable.** Count
  `MEM_OP_D.OPERATION ∈ {MMU_TLB_INVALIDATE 0x9, _TARGETED 0xa}` **across every channel we parse,
  not just the compute one** — §3's point is that they will be on UVM's internal channel.
- **The forward path's handling of `NVOS46/47` flags is unaudited** (§5 item 1). I did not grep
  our forward path for flag sanitisation; if bit 31 passes through, that is a live hole.
- **GSP-RM's own behaviour is unreadable** (`RMCFG_FEATURE_PLATFORM_GSP == 0`). Nothing above
  depends on it.
- **The closed driver** could in principle issue invalidates at different *times*; it cannot issue
  them through a different *mechanism* (§9).
- I did not enumerate RM's invalidate sites in the **display/BAR1/fabric** planes beyond listing
  the call sites; they are cited but not read.

---

## 12. ⊘ THINGS IN THE BRIEF THAT WERE WRONG — including the one that started it

1. ★★★★★ **My over-claim to the owner** — *"the guest issues ZERO invalidates; you cannot hook it
   as your commit boundary, it never arrives."* **False.** §7. The correction is the reason this
   rung exists and it is the most consequential item in this doc: the invalidate is one trappable
   BAR0 word away.
2. ★★★ **The brief's census reading is inverted AND is the wrong census.**
   > *"a recent census of ours reads `PRAMIN 21 / BAR1 9 / BAR2 88 / EXEC 3546`, so the CPU
   > transports dominate by ~99 %"*
   - **Inverted:** `21 + 9 + 88 = 118` against `3546`. `3546 / 3664 = 96.8 %` is **EXEC** — the
     *engine* transports dominate, by ~97 %, not the CPU ones by 99 %.
   - **Wrong census:** it is the **framebuffer FIRST-WRITER** census
     (`kayfabe-device/src/fbwin.rs:618-631`, `:678`; reported at
     `kayfabe-qemu-raw/src/shim.rs:8680`) — *who first wrote each FB page*, over **every**
     framebuffer page, buffers and all. It says nothing about **page-table** writes specifically,
     so it cannot rank PTE-write transports in either direction.
   - ⇒ ⚠ **Same class as the traps this tree already banks:** a number was carried across from a
     census that measures a different population, and then read with the ratio the wrong way up.
     **The right answer to "which transport writes PTEs" is w289 §2.5 + rows R1/U1 here: RM =
     CPU/BAR2, UVM = CE — and they are different, which is exactly why one instrument cannot see
     both.**
3. ⊘ **The brief's MMU intuition was the wrong way round.** *"Many MMUs require an invalidate only
   when a mapping becomes more restrictive; if NVIDIA's GMMU caches non-present entries, an
   invalidate is required on map too."* The conditional resolves **true** (§1.1), so the
   restrictive-only model does not apply here at all.
4. ⊘ **`uvm_page_tree.c` / `uvm_page_tree.h` do not exist** in either version. The page-tree code
   is `uvm_mmu.{c,h}`; the batching APIs are `uvm_tlb_batch.{c,h}` and `uvm_pte_batch.{c,h}`.
5. ⊘ **`dmaUpdateVASpace_GM107` does not exist.** The HAL implementation is
   `dmaUpdateVASpace_GF100`, in the file named `virt_mem_allocator_gm107.c`.
6. ⊘ **`kgmmuInvalidateTlb_GA100` does not exist.** `_GM107` is the only non-stub variant for every
   chip we support.
7. ⊘ **`NV0080_CTRL_CMD_DMA_FILL_PTE_MEM` has no handler in the open tree** — only the internal
   `DMA_UPDATE_VASPACE_FLAGS_FILL_PTE_MEM` flag survives. Consistent with
   `mode2_address_table.md`'s 2026-06-17 correction and with its measured zero.

---

## 13. ★★★ THE DIRECTIVE — the owner's preference ordering, recorded, with the amendment this rung earns

**Owner, 2026-08-14, verbatim:**

> **exact boundary from the GPU (e.g. TLB invalidate) > trapping write on PTE, as little as
> possible > deferred publish on doorbell > work under BQL in a doorbell.**

**AMENDMENT — the ordering is DIRECTION-DEPENDENT, and the source is why (§8):**

> - **MAP (invalid → valid):** the ordering stands as written. A missed trigger is fail-safe (a
>   contained GPU fault), so tier 1 is both correct and preferred.
> - **UNMAP / downgrade (valid → invalid):** ⊘ **tier 1 may not be the guarantee.** The guest can
>   legally suppress the invalidate (`NVOS47_FLAGS_DEFER_TLB_INVALIDATION`,
>   `UPDATE_PDE_2` without `_FLUSH_PDE_CACHE`), and a missed unmap is a live host-GPU translation
>   into reused guest pages. **Tier 2 — observe the write — is REQUIRED here; tier 1 is an
>   optimisation on top of it.**
> - **Corollary:** *"exact boundary"* means exact **and compelled**. A boundary the guest can
>   decline to emit is not a boundary in the direction where omission is dangerous.

**And a second, independent obligation that survives every tier** (§5): the invalidate **we** owe
the **host** GPU. Today it is discharged by the host RM because we publish through its map/unmap
verbs and never write host page tables. **Keeping it that way is a design constraint**, because no
unprivileged host process can write `0xB830B0`.
