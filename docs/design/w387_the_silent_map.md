# w387 — THE SILENT MAP: there is no universal publish trigger, and the aperture is not a choke point

> **STATUS — 2026-09-07 — LIVE.** Source investigation against `ogkm-580.159.04` (the version the
> guest runs), cross-checked in `ogkm-610.43.02` — **no divergence found on any load-bearing
> point**. No code landed. This is the answer to the question the owner had held open all day:
> *"what's the invariant the ogkm driver gives that it always does after updating PTE/PDB
> mappings? Which interception points do we need so no mapping slips?"*
>
> ### ★★★★★ THE ANSWER IN THREE LINES
> 1. **There is no universal signal.** The TLB invalidate is conditional on two
>    userspace-settable flags, and for GSP-managed mappings it executes on the falcon.
> 2. **The aperture is not a choke point either.** Page tables can live in **sysmem**, written
>    by plain CPU stores to DRAM — no BAR, no register, no RPC. Reachable by **one unprivileged
>    ioctl flag**.
> 3. ⇒ **With registers / apertures / RPCs as the vocabulary, 100 % coverage is UNREACHABLE.**
>    The only mechanism that can reach it is **write-protection over a self-closing page set**,
>    bootstrapped from the root PDB. That is a proposal (§6), not a measurement.
>
> **Parents this folds into** (corrections belong *in* them, not only here):
> `mode2_address_table.md` §5 (its two co-equal sources are incomplete — see §2/§3 here);
> `the_publish_trigger_measured.md` §0.4 (*"RM does not change a mapping without one"* is
> **refuted** — §1.1); `publication_off_the_bql.md` §1 (the RPC-anchor reading is **refuted**
> for the default configuration — §2).

---

## 0. Why this rung happened

The doorbell trap measures **29.4 ms mean** on a real GA106, and a `kftime` census attributes it:

| segment | mean µs | share | shape |
|---|---|---|---|
| `vas_publish` | 19 822 | **67.5 %** | work |
| `ringproj` | 5 719 | 19.5 % | work |
| `pt_vascensus` | 2 590 | 8.8 % | work |
| `pt_decode` | 642 | 2.2 % | work |
| `operand_join` | 145 | 0.5 % | work |
| **`core` (the real host RM forward)** | **112** | **0.4 %** | **host** |
| `bindcensus` | 60 | 0.2 % | work |
| `pt_sweep` | 30 | 0.1 % | work |

`[measured, doorbell_fwd, n=22 600]`. ⇒ **96 % of a doorbell is publication and page-table work;
the actual host interaction is 0.4 %.** ⊘ And `pt_sweep` at 0.1 % **refutes** the standing
suspicion that the whole-VAS sweep was the cost — that was wrong, and `the_breadth_is_free_and_idle.md`
was right.

⚠ **Two of my own claims from this session are RETRACTED here**, both stated to the owner before
being checked:
- *"the expensive thing sits in the dispatch, not the publish"* — **false**. `kft.mark("plane")`
  wraps the entire `plane.write()` call, so its 100 % share is a tautology, not a measurement.
- *"QEMU MMIO trap, BQL held"* — **false**. The QOM shim calls
  `memory_region_enable_lockless_io()` on every region (`nvkvm.c:931`) and **asserts at realize**
  that every non-MSIX region got it (`:1155`). There is no BQL to remove.

The owner's reading was the correct one: *"either you do a large operation in the doorbell or you
are stuck waiting on a lock."* It is the **large operation** — publication running per doorbell,
which is the wrong variable (§4 of `the_publish_trigger_measured.md`: the invalidate count is
workload-invariant at 377/377/331 while doorbells move 30×, 18 → 532).

⇒ That made the trigger question load-bearing, and the trigger question is this document.

---

## 1. The TLB invalidate is not the invariant

### 1.1 It is conditional, and both skips are client-settable

`dmaUpdateVASpace_GF100` gates it (`virt_mem_allocator_gm107.c:2610-2615`):

```c
done:
  if ((NULL == pTgtPteMem) && DMA_TLB_INVALIDATE == deferInvalidate) {
      kbusFlush_HAL(...);           // the flush is INSIDE the same if
      gvaspaceInvalidateTlb(...);   // -> BAR0 0x00B830B0
  }
```

- **`deferInvalidate`** comes from `NVOS46_FLAGS_DEFER_TLB_INVALIDATION` — **bit 31**
  (`nvos.h:2149-2151`; NVOS47 equivalent at `:2190-2192`), consumed at
  `virt_mem_allocator_gm107.c:414` and `:1574`. **Unprivileged guest userspace sets it on an
  ordinary map.**
- **`pTgtPteMem != NULL`** is the buffered `DMA_UPDATE_VASPACE_FLAGS_FILL_PTE_MEM` branch
  (`NVBIT(25)`, `g_virt_mem_allocator_nvoc.h:633`).

⇒ ★ **`the_publish_trigger_measured.md` §0.4's *"RM does not change a mapping without one"* is
REFUTED.** That sentence is the premise three campaigns rested on.

⊘ **The flush is NOT a better anchor.** It sits *inside* the same `if`, so it is skipped by both
conditions — strictly weaker than the invalidate, not more universal. And where it does fire its
transport is often useless: for sysmem it is `portAtomicMemoryFenceFull()`
(`kern_bus_gm107.c:3331`), a **CPU-local fence**, invisible to any hypervisor.

⊘ **No observable lock either.** `gvaspaceWalkUserCtxAcquire` is not a lock — it stores a context
pointer via `mmuWalkSetUserCtx` and asserts non-nesting. No hardware semaphore, no GPU-visible
memory location around the walk. ⚠ RM's *general* lock layer (`rmapiLockAcquire`,
`rmGpuLocksAcquire`) was **not** audited for an observable footprint — **unfound, not absent.**

### 1.2 Where it does fire, the transport is confirmed

`kgmmuCommitTlbInvalidate_TU102` (`kern_gmmu_tu102.c:117`) →
`GPU_VREG_WR32(pGpu, NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE, ...)`.
`NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE = 0x30B0` (`dev_vm.h:131`), and `GPU_VREG_*` adds
`DRF_BASE(NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET) = 0x00B8_0000` on a PF/passthrough GPU
(`kern_gpu_tu102.c:93-100`). ⇒ **absolute BAR0 `0x00B8_30B0`, confirmed from source.**
The RPC alternative (`NV_RM_RPC_INVALIDATE_TLB`, fn 200) is taken **only** under
`VF_INVALIDATE_TLB_TRAP_ENABLED` (`kern_gmmu_gm107.c:145-160`) — a vGPU/SR-IOV feature, **not**
passthrough.

★ It is a genuine **quiescence point** where it fires: `kgmmuCheckPendingInvalidates_TU102`
(`kern_gmmu_tu102.c:59-84`) spin-polls until `TRIGGER` reads false, so the guest is stopped.
`[measured w326]` `polls = 754 = 2 × 377` exactly — the protocol floor, proving the guest never
spun on a disarmed plane.

---

## 2. ⊘⊘⊘ SPLIT-VAS IS THE DEFAULT — the guest maps LOCALLY, with no RPC

**This refutes the *"we emulate the GSP, so every map is a call we serve"* argument**, which was
stated to the owner in this session before being checked.

`gpu_registry.c:171-183`:
```c
if ((pGpu->bSriovEnabled && !...) || RMCFG_FEATURE_PLATFORM_GSP || IS_GSP_CLIENT(pGpu)) {
    if (osReadRegistryDword(..._SPLIT_VAS_MGMT...) == NV_OK) ...
    else pGpu->bSplitVasManagementServerClientRm = NV_TRUE;   // the default
}
```

⇒ **`bSplitVasManagementServerClientRm` defaults to `NV_TRUE` for every GSP client.** Under it the
guest's own CPU-RM owns and writes its half of the page tables:

- VA alloc is local: `bRpcAlloc = !(gpuIsSplitVasManagementServerClientRmEnabled(pGpu) || ...)`
  → **FALSE** (`virtual_mem.c:466-468`).
- Map is local: `if (!pMemory->bRpcAlloc || gpuIsSplitVasManagementServerClientRmEnabled(pGpu))`
  → **TRUE** (`virtual_mem.c:1421, 1463`) ⇒ `dmaAllocMap` → `dmaUpdateVASpace` **in the guest**.
- ⇒ `NV_RM_RPC_MAP_MEMORY_DMA` at `virtual_mem.c:1522` **never fires** for that memory.

⚠ Guest-allocated backing does not force it either: sysmem alloc-RPC is gated `if (IS_VIRTUAL(pGpu))`
— false in passthrough (`system_mem.c:462`); the legacy vidmem alloc-RPC is gated
`if (!IS_GSP_CLIENT(pGpu))` (`video_mem.c:962`).

★ **And two RPCs we might have built on do not exist.** `NV_RM_RPC_DMA_FILL_PTE_MEM` (fn 27) is an
**empty inline stub with no override** (`rpc_vgpu.h:42`), as is `NV_RM_RPC_ALLOC_VIRTMEM`
(`rpc_vgpu.h:39`) **even though `virtual_mem.c:546` calls it** — that call compiles to nothing.
`UPDATE_GPU_PDES` (fn 61) has an enum entry (`rpc_global_enums.h:71`) and **no sender**.
⇒ This is the *"a `#if`/flag-guarded call may be compiled out"* trap firing for real. Do not build
detection on fns 27 or 61.

---

## 3. ⊘⊘⊘ THE APERTURE IS NOT A CHOKE POINT — page tables can live in SYSMEM

This is the finding that kills the fallback design (*"trap the aperture, since we emulate the
device"*).

### 3.1 The allocator picks the aperture per level, and sysmem enters four ways

`_gmmuWalkCBLevelAlloc` (`gmmu_walk.c:107`) builds a `memPoolList[]` of candidate apertures.
`ADDR_SYSMEM` enters it via:

| route | site | sysmem is… |
|---|---|---|
| root/PDB fallback, regkey `NV_REG_STR_RM_INST_LOC._PDE` | `gmmu_walk.c:179-182` | fallback |
| **`pBlock->flags.bPreferSysmemPageTables`** | `gmmu_walk.c:270-274` | ★ **FIRST in the list — preferred** |
| `VASPACE_FLAGS_RETRY_PTE_ALLOC_IN_SYS` | `gmmu_walk.c:281-285` | fallback (out-of-FB) |
| the default aperture itself, via `instLocOverrides` | `gmmu_walk.c:253-257` | can be sysmem |

**Guest userspace reaches this with one flag, no privilege:**
- `NVOS32_ALLOC_FLAGS_PREFER_PTES_IN_SYSMEMORY` = **`0x20000000`** (`nvos.h:1477`) on an NVOS32
  heap alloc → `vaspace.c:138` → `bPreferSysmemPageTables` → sysmem preferred at `gmmu_walk.c:272`.
- `NV_VASPACE_ALLOCATION_FLAGS_RETRY_PTE_ALLOC_IN_SYS` = `BIT(1)` (`nvos.h:3168`) on a
  `FERMI_VASPACE_A` alloc → `vaspace_api.c:601-603`.
- `NV_DEVICE_ALLOCATION_FLAGS_RETRY_PTE_ALLOC_IN_SYS` = `0x4` (`nvos.h:2809`) → `device_share.c:144-146`.
- ★ **RM arms it unconditionally on its own global device VA space**:
  `constructFlags |= VASPACE_FLAGS_RETRY_PTE_ALLOC_IN_SYS;` (`kern_gmmu.c:234`).
  ⇒ **This is not only a hostile-guest path.**

### 3.2 The sysmem write path touches no aperture at all

`memdescGetMapInternalType` (`mem_desc.c:2179-2205`) calls `kbusUseDirectSysmemMap_HAL`. On Ampere:

```c
kbusUseDirectSysmemMap_GA100(...)            // kern_bus_ga100.c:1164
{ *pbAllowDirectMap = NV_FALSE;
  if ((memdescGetAddressSpace(pMemDesc) != ADDR_FBMEM))
      *pbAllowDirectMap = NV_TRUE;           // :1175 — UNCONDITIONAL for non-FB
  return NV_OK; }
```
→ `MEMDESC_MAP_INTERNAL_TYPE_SYSMEM_DIRECT` → `memdescMapOld` → **a plain kernel VA into system
RAM**. `_gmmuWalkCBMapNextEntries_Direct`'s `portMemCopy` (`virt_mem_allocator_gm107.c:2021`) then
writes PTE bytes as ordinary CPU stores to DRAM. **No BAR0, no BAR1, no BAR2, no PRI, no RPC.**

⚠ GA100's version is strictly **weaker** than Maxwell's `kbusUseDirectSysmemMap_GM107`
(`kern_bus_gm107.c:4985-5005`), which required `!kbusIsBar2SysmemAccessEnabled` and uncached attrs.
**On the guest's GA10x, any sysmem memdesc is direct-mapped.**

### 3.3 ✔ BAR1 is never a page-table transport — so the owner's ruling is safe

`TRANSFER_FLAGS_USE_BAR1` is the only route to BAR1 in the transfer layer
(`mem_utils.c:1308`, `:1386`). **Complete enumeration of its setters** — `ce_utils.c:337,487`;
`sec2_utils.c:329,342,355,468`; `mem_utils_gm107.c:549,1114`; `mem_mapper.c:398,447,472`;
`channel_utils.c:272,474,915` — **all CE/SEC2 channel plumbing** (pushbuffers, USERD, error
notifiers, semaphores). None is a page-table path. The walker passes only
`SHADOW_ALLOC | SHADOW_INIT_MEM` (`virt_mem_allocator_gm107.c:2063-2074`). And there is **no
`MEMDESC_MAP_INTERNAL_TYPE_BAR1`** — the enum admits only `GSP`, `SYSMEM_DIRECT`,
`COHERENT_FBMEM`, `BAR2` (`mem_desc.c:2179-2205`).

⚠ This is an enumeration of one flag's setters, not a proof no other BAR1 mapping ever carries PT
bytes. **Unfound, not absent** — but it is a complete enumeration of the only known route.

---

## 4. ★★★★★ THE FULLY-SILENT CONJUNCTION IS REACHABLE — measured

The owner's exact worry, realized:

> *"there is a possible guest userspace ioctl call that can literally let the ogkm kernel module
> only do a PCIe BAR write to GPU PHYS … without any register write, and then ask the GPU to
> populate it on fault dynamically."*

**With BOTH flags set on the same `NV04_MAP_MEMORY_DMA` — sysmem page-table level *and*
`DEFER_TLB_INVALIDATION` — the whole operation reduces to CPU stores into guest DRAM followed by
`return`.**

The two choices are **orthogonal**, which is why they compose: the defer flag is read at intake
(`:414`) and carried to the tail gate; the aperture is chosen much later, inside the walker, by the
level memdesc's own address space. They meet only at `:2610-2615`, where **`kbusFlush_HAL` and
`gvaspaceInvalidateTlb` are inside the same `if`** and are therefore skipped together.

**Every other side effect on the path was checked and is clear:**
- `memdescFlushGpuCaches` (`mem_desc.c:2158`) issues `kmemsysCacheOp_HAL` **only** when
  `memdescGetGpuCacheAttrib == NV_MEMORY_CACHED` (`:2166`). GMMU page-table memory defaults to
  **`NV_MEMORY_WRITECOMBINED`** (`kern_gmmu.c:124,130`) ⇒ **no-op, no register.**
  ⚠ Holds unless an `instLocOverrides` regkey forces CACHED, which the guest does not set.
- `bNeedL2InvalidateAtUnmap` is computed and consumed on the **unmap** branch only
  (`virt_mem_allocator_gm107.c:1480`).
- Post-map bookkeeping is `intermapRegisterDmaMapping` — a software list insert.
- `_gmmuScrubMemDesc` is **root-level only** and at allocation (`gmmu_walk.c:348`).

⇒ **Neither flag alone suffices, and that is the point.** Drop DEFER → the `0xB830B0` invalidate
fires (observable). Drop sysmem → the PTE bytes go back through **BAR2** (trappable). **Only the
conjunction is simultaneously aperture-free and register-free.**

⚠ **Provenance note, and it is a lesson in its own right.** The two halves were verified in
*separate* passes and I stated the conjunction to the owner as established **before it was
traced**. The owner challenged it (*"so you are sure…? nothing has fired meanwhile"*), the join was
then traced, and it happened to hold. **It held; the reasoning that asserted it did not.** Same
class as `a_measurement_compressed_into_an_impossibility` — two true findings joined into a third
nobody measured.

---

## 5. ⊘ AND THE ALLOCATION SIDE DOES NOT RESCUE IT

The obvious fallback — *"even if we can't see the writes, we see the page being allocated"* — fails.

A sysmem level is allocated by `memdescCreate` + `memdescTagAlloc` (`gmmu_walk.c:341`), which for
`ADDR_SYSMEM` bottoms out at **`osAllocPages(pMemDesc)`** (`_memdescAllocInternal`,
`case ADDR_SYSMEM`) — **plain guest-kernel pages**. It is **not** `rmMemPoolAllocate` (that is the
`ADDR_FBMEM` PMA branch only, `gmmu_walk.c:329-335`), so the pages do not come from a pool a
hypervisor provisioned. **No `NV_ESC_RM_ALLOC`, no RPC, no GSP memdesc registration** (searched
`mem_desc.c` for `NV_RM_RPC_*` / GSP registration — unfound). Under split-VAS the intermediate
levels are never conveyed to GSP at all.

★ **What the emulated-GSP side does see is ONE pointer**: the root PDB physical address, at channel
bind / `SET_PAGE_DIRECTORY` / instance-block time.

⇒ **Not seeing the writes also means not reliably knowing which pages they are** — unless §6 holds.

---

## 6. ◐ WHAT SURVIVES — the induction from the root, and it is a PROPOSAL not a measurement

⚠ **This section is reasoning, not source evidence. It is the only route to 100 % this rung found,
and every clause below needs checking before anything is built on it.**

A page-table page is only *usable* once it is **linked into the tree** — an unlinked level is
unreachable by the GPU's walker. And it can only be linked by a **PDE write into a page that is
already part of the tree**. Therefore:

- **Bootstrap:** the root PDB is visible at bind (`SET_PAGE_DIRECTORY` / instance block / promote).
- **Induction step:** write-protect every page reachable from the root. A new level announces
  itself by the very write that makes it reachable — the PDE store into its already-protected
  parent.
- **Closure:** the protected set closes under itself, **by induction rather than by enumerating
  driver behaviours a new version can invalidate.**

★ This is strictly stronger than an interception-point list, and it is what the C approximated with
its re-swept `m2_gr_pt_set`.

**Mechanism.** Not trap-every-store — the semantics needed is *"did this page change since I last
read it"*. So: uffd-WP the page → first write faults → handler records dirty and **immediately
unprotects** → guest resumes. **One fault per page per epoch, not per store.** Re-read dirty pages
at the next point work could consume them, then re-protect.

⊘ **This is designed and UNBUILT.** `UFFDIO_WRITEPROTECT` has **zero** occurrences in the tree.
`kayfabe-vmm/src/lib.rs:617,744` reference `UFFDIO_REGISTER` for a *different* mechanism (locking a
window VMA). The owner's #48 ruling (*"uffd everywhere"*, 2026-07-27) never had its write-protect
half wired.

**Why it should be cheap.** Page tables are ~one leaf page per 2 MiB mapped — thousands of pages
against millions of data pages. And in steady state it is near-free: the current publication census
already reports `published=0 refused=8` on **every** pass, i.e. nothing changes during a launch
loop. Dirty-tracking would find zero and the doorbell would do nothing — the 29 ms going to
approximately nothing by doing **less**, not by deferring.

⊘ **KVM dirty logging is NOT the right tool**: it is per-memslot, so arming it on guest RAM logs
every guest write. uffd-WP over the specific ranges is the precise instrument.

---

## 7. OWNER RULINGS, 2026-09-07

1. ★★★★★ **DO NOT TRAP BAR1.** *"bar1 is what is also PCIe bar that can be mapped to userspace if
   someone asks cuda to alloc GPU VA and map it into the CPU process. If bar1 can remain
   passthrough maps, then I am fine that bar2 is write trapped, since bar2 is only used for control
   and maybe isn't a heavy used I/O path."* Also: *"nvkvm-pv has bars that are real passthrough,
   this is needed to get parity."*
   ⇒ **§3.3 confirms this is safe**: BAR1 is never a page-table transport.
   ⚠ **Obligation it carries:** a passthrough BAR1 must not create cross-process leakage — standing
   constraint that two processes in one guest do not trust each other.
2. ★★★★★ **100 % coverage is a hard constraint.** *"we need 100% coverage, there is no argument
   against it, correctness is paramount. Any coverage thats flakey also probably needs a raw client
   excercising that path … All the ways that obtain 100% coverage, within that, you can hard
   optimize."*
   ⇒ Cost may not be used to justify dropping a coverage point. Optimize **within** a complete set.
3. **RPC endpoints are valid interception points** — *"we can let those hang until the map
   completes."* ★ Correct where they fire (`_issueRpcAndWait`, `rpc.c:1821`, means the guest is
   already blocked — the quiescence is handed to us). ⊘ But §2: under split-VAS most maps are not
   RPCs at all.

⚠ **The RPC hang has a budget**: `GPU_TIMEOUT_DEFAULT` → `osGetTimeoutParams` = **4 s (GRAPHICS) /
30 s (COMPUTE)**, re-armed at `gpuChangeComputeModeRefCount`. Overrun is an Xid and a reset, not a
slow path.

---

## 8. WHAT IS TODAY'S STATE

- `KAYFABE_VAS_PUBLISH` defaults to **`off`**; the arm is 6-valued and never defaulted.
- ★ It is **LOAD-BEARING**: `[w298 ablation, real GA106, row 2]` `drain` → `assert` gives
  `CUP3_VAL` **absent**, `RC=1`, **Xid=1**, `host_rows` 23/16425. Publication cannot simply be
  deleted.
- ★ **Deferring is fine; COALESCING is not.** `[w383, three boots, one variable]` `off` → `43`,
  0 Xid; `nocoalesce` → `43`, 0 Xid; `on` (coalescing) → `CUP3_VAL` absent, `cuCtxCreate → 999`,
  **2 × Xid 31**.
- Every BAR is `memory_region_init_io` in the QOM shim (`nvkvm.c:673`), asserted at realize — so
  **BAR1 is trapped and served through the GMMU in software today** (`plane.rs` `bar1_reads` /
  `bar1_writes` / `bar1_faults`). That is the anti-parity cost ruling 7.1 targets. Overlaps
  task #271 (*MAP VIDMEM INTO THE GPA*), pending since 2026-08-27.
- `pt_witness` is armed by `FbStore::writes_by` — **framebuffer** writes. **Sysmem page tables
  never touch it.** A live gap, not a theoretical one.

---

## 9. RESIDUALS — flagged, none of them cleared

⚠ **Every item here is `unfound` or `inferred`. None may be cited as `absent` or `measured`.**

1. **UVM bypasses everything.** Own page tables; PTEs by CPU store or CE copy; invalidates as
   **pushbuffer methods** (`uvm_hopper_host.c:108-165`, `NVC86F … MEM_OP_* TLB_INVALIDATE_TARGETED`,
   `clc86f.h:73-120`). No `0xB830B0`, no RM RPC. We have the decoder (`ga10x.rs:1446`,
   `PushMethod::TlbInvalidate`) and our own census recorded it firing **zero** times on the Mode-2
   compute path — and *a census over transports is only as complete as its list.*
2. **Negative caching.** Whether GA10x GMMU caches *invalid* entries decides whether a fresh map is
   architecturally signal-free. **INFERRED** from the downgrade-only membar WAR
   (`kern_gmmu_gm107.c:222-226`) and the client-exposed defer flag. Searched
   `swref/published/{ampere,turing,hopper,blackwell}/**/dev_mmu.h`, `dev_fault.h`, and `kern_gmmu*`
   for "negative"/"invalid entry"/"cache" — **no hardware statement found.**
3. **ATS/PASID.** Gated on `PDB_PROP_GPU_ATS_SUPPORTED`, set from GSP static info
   (`kern_mem_sys.c:183-187`). Believed unset on PCIe GA10x; **not proven unreachable.** If it were
   on, a mapping goes live on a **CPU** page-table change with nothing GPU-side at all.
4. **Non-UVM replayable-fault population.** The fault buffer and `MMU_TLB_INVALIDATE … REPLAY`
   machinery exist; no non-UVM graphics/compute channel was traced relying on replay to make a
   newly-mapped VA usable. **Unfound.**
5. **A page can stop being a page table.** `_gmmuWalkCBLevelAlloc` grows a level by allocating a
   **new, larger** memdesc (`gmmu_walk.c:144-148`, sets `*pBChanged`). The free / `CopyEntries` side
   was **not traced** — is the transition signalled? If not, §6 leaves stale protections that fault
   forever on unrelated writes.
6. **Per-page granularity may not hold.** Levels reserve ≥1 aligned 4 KiB page via
   `gmmu_walk.c:307` (non-PMA) and `:336` (PMA). ⚠ **`rmMemPoolAllocate` was not read** to confirm
   it honours `ActualSize` rather than the pool's 256 B granule (`poolAllocSizes[]` bottoms at
   `0x100`, `pool_alloc.c:101-104`). If it does not, PMA levels can **share** a 4 KiB page and
   per-page protection breaks outright. **Check before building §6.**
7. **Channel-bind re-walk.** Whether binding a channel over a client-managed **sysmem** tree forces
   anything observable was not traced. This is the one place left that could restore a signal.
8. **RM's general lock layer** (`rmapiLockAcquire`, `rmGpuLocksAcquire`) unaudited for an observable
   footprint.
9. **`GET_PTE_INFO` consistency.** `NV0080_CTRL_CMD_DMA_GET_PTE_INFO` (`0x801801`) and
   `SET_PTE_INFO` (`0x801802`) let a client read and write the real page-table image, so our
   emulation must keep bytes that agree with what we reported. ★ Partly answered already:
   `[measured 2026-08-13, real GA106]` `GET_PTE_INFO` is **disabled in production**, nvoc flags
   `0x100008` (`submit.rs:4321-4328`).

---

## 10. THE TWO RAW CLIENTS THIS COMMISSIONS

Per ruling 7.2 — *"Any coverage thats flakey also probably needs a raw client excercising that path
doing allocs and work in such an order to trigger a race."* Both are **unbuilt**; nearest existing
item is task #272.

- **RC-1 — the silent map.** Alloc with `NVOS32_ALLOC_FLAGS_PREFER_PTES_IN_SYSMEMORY` (`0x20000000`),
  map with `NVOS46_FLAGS_DEFER_TLB_INVALIDATION` (bit 31), then submit work touching the new VA.
  **Pre-registered:** our side observes **no** BAR2 write, **no** `0xB830B0`, **no** RPC, and **no**
  alloc that identifies the page. That converts §4 from a source trace into a **demonstrated** hole
  — and if anything *does* fire, that is our interception point, proven rather than argued.
- **RC-2 — the late mapping into a running channel.** Map while a passthrough channel is already
  running (`GP_GET != GP_PUT`), so no doorbell is rung. Tests the hole the owner named directly:
  *"a doorbell may not be needed to advance GPFIFO by GPU silently."*

---

## 11. LESSONS THIS RUNG PAID FOR

- ★★★★★ **A signal-based coverage argument is unbounded; a writer-based one is bounded.**
  Enumerating *what the driver emits* is version-dependent and was proved incomplete **three times
  in one day** (invalidate → RPC → aperture). Enumerating *what can write page-table bytes* is a
  property of our own device model. **Enumerate writers, not signals.**
- ★★★★ **Two verified halves are not a verified conjunction** (§4's provenance note).
- ★★★ **A compiled-out call reads exactly like a live one.** Fns 27 and 61 have call sites and no
  bodies. Detection built on either would have been built on nothing.
- ★★★ **An instrument that wraps the whole call cannot attribute anything inside it.**
  `kft.mark("plane")` made `plane share=100 %` a tautology and I reported it as a finding.

---

## 12. ★★★★★ THE SOLUTION SPACE — five candidates, measured, and only two survive

**Added 2026-09-07, same rung.** §6 proposed write-protection as *the* route. The owner asked
*"are there more solutions to get the coverage?"* — there are, the space is wider than
*observation*, and enumerating it killed three candidates and left a real fork.

⚠ **Framing correction that produced the extra candidates:** every earlier attempt asked *how do
we observe the write*. Three of the five below do not observe anything — they make the silent
path **not exist**, make a missing mapping **safe**, or make our **blindness detectable**. The
observation framing was itself the narrowing.

| # | candidate | verdict |
|---|---|---|
| **S1** | **fault-driven** — let it fault, service, replay | ⊘ **DEAD**, three independent fatal links |
| **S2** | **externally-owned VAS** — give the host GPU the guest's tables | ◐ **CONDITIONAL** on identity FB |
| **S3** | more **forcings** below Hopper | ⊘ **DEAD** — setter list enumerated and closed |
| **S4** | **blindness detection** → named refusal | ★ **VIABLE, cheap, arch-stable — take it regardless** |
| **CC** | the **CC-bit forcing** (§11) | ◐ **VIABLE, costs GA10x** |

### 12.1 ⊘ S1 IS DEAD — and the owner called it before the evidence

> Owner: *"but we can't resume a fault from userspace right"*

Correct, and RM closes it deliberately — the guard even carries a bug number:

```c
// 1766112: Prevent channels in fault-capable VAS from running unless bound
//  User-mode clients can allocate a fault-capable VAS and schedule it
//  without registering it with the UVM driver ... This will cause what
//  looks like a hang on the GPU
NV_CHECK_OR_RETURN(LEVEL_WARNING,
    !((flags & VASPACE_FLAGS_ENABLE_FAULTING) &&
      !(flags & VASPACE_FLAGS_IS_EXTERNALLY_OWNED)),
    NV_ERR_INVALID_ARGUMENT);            // vaspace_api.c:686-691
```

**Three independent links, any one fatal:**

1. ★ **The fault buffer is KERNEL-privileged.** `MMU_FAULT_BUFFER` (class `c369`) carries
   `RS_FLAGS_ALLOC_KERNEL_PRIVILEGED` (`resource_list.h:975-983`), enforced at
   `alloc_free.c:661-668`. `RS_PRIV_LEVEL_KERNEL` is **above** `USER_ROOT` — an unprivileged
   client *or even root* is rejected. Only an in-kernel client (UVM) clears it. Identical in 610.
2. ★★★ **The regime is GPU-GLOBAL, not per-VAS.** Enable state is one register,
   `NV_PFB_PRI_MMU_FAULT_BUFFER_SIZE._ENABLE`, **per GFID** (`kern_gmmu_gv100.c:1490`), keyed
   `mmuFaultBuffer[gfid].hwFaultBuffers[index]`. ⇒ **a single host GPU cannot mix fatal and
   replayable regimes across tenants.** This kills S1 on multi-tenancy grounds *independently of
   privilege*, and it is the finding with reach beyond this question.
3. **Base RM does not replay at all** — it *cancels*:
   `// Replayable Faults - These faults will be cancelled as RM doesn't support replaying such
   faults. Cancelling these faults will bring them back as non-replayable faults.`
   (`kern_gmmu_gv100.c:2539-2542`, → `_kgmmuHandleReplayablePrivFault_GV100` cancel at `:2555`).
   Replay servicing lives in the **UVM kernel driver**, not on the RMAPI surface.

⊘ **AND A PRIOR RESULT MUST NOT BE MISCITED HERE.** `w288 Q2` (2026-08-13, C tree, commit
`c896ed4`) measured that an unprivileged isolate **can learn of a fault by OS event** — path
`NV_ESC_ALLOC_OS_EVENT` → `NV01_EVENT_OS_EVENT` on a Subdevice →
`NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION(event=37, REPEAT)` → `poll()` →
`NV_ESC_RM_GET_EVENT_DATA`, every gate NON_PRIVILEGED, `info32 = exceptType`.
★ **But event 37 is `NV2080_NOTIFIERS_RC_ERROR` — Robust Channel, the FATAL recovery path.**
That is notification that a channel **already died**: a tombstone, not a resumable state. It says
nothing about replayable faults. ⇒ **Do not cite w288 Q2 as evidence that faults can be
serviced.** Same class as this file's §4 provenance note — a true result, one inferential step
past what it measured.

⇒ **S1 does not dissolve the problem; it RELOCATES it into S2**, because the only door to
fault-driven operation is the externally-owned VAS.

### 12.2 ◐ S2 — real, unprivileged-reachable, and it reduces to publication unless FB is identity

The mechanism exists and RM genuinely stands down:
- `gvaspaceReserveVA` → `NV_ERR_NOT_SUPPORTED`, *"Cannot reserve VA on an externally owned
  VASPACE"* (`gpu_vaspace.c:1419-1425`).
- The PDB becomes the client's: `gvaspaceGetPDB` returns `pGVAS->pExternalPDB`
  (`:2084-2086`), set via `gvaspaceSetExternalPDB` (`:4813-4816`), reached through
  `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` (`0x801813`) → `gvaspaceExternalRootDirCommit`
  (`:3024`). ★ **We already serve `0x801813`.**
- ★ **Unprivileged**: `NV_VASPACE_ALLOCATION_FLAGS_IS_EXTERNALLY_OWNED` at
  `vaspace_api.c:617-621` has **no `bKernelClient` check** — unlike the ATS flag immediately
  above it (`:672-679`), which does. The contrast is the evidence.

⊘ **THE BLOCKER HOLDS.** PTE address fields are filled by
`gmmuFieldSetAddress(pIter->pAddrField, kgmmuEncodePhysAddr(..., pIter->aperture,
pIter->physAddr, ...))` (`virt_mem_allocator_gm107.c:2012-2016`) — a **raw GPU physical
address**, aperture chosen per entry, **no indirection layer between PTE and FB**. So guest
tables are directly host-usable **only if guest GPU-physical == host GPU-physical for every page
they name.** Sysmem can get that identity from an IOMMU; **vidmem has no translation stage at
all.** Relocate or partition FB and every vidmem PTE must be rewritten — **which is publication
wearing a different hat.**

⚠ **AND THE TENSION THAT DECIDES IT IS NOT TECHNICAL.** Identity FB means **one guest owns the
whole GPU**. `hostile_guest_isolation_is_the_value_proposition` and the owner's standing
per-process-isolation constraint point the other way. **S2 may buy correctness at the cost of the
product's premise** — an owner call, not an engineering one.

### 12.3 ★ S4 — TAKE THIS REGARDLESS. It is not coverage; it is knowing when we lack it.

The PDE aperture is a **hardware field** with an explicit four-state encoding:

```
NV_MMU_PDE_APERTURE_BIG_INVALID                     0x00000000
NV_MMU_PDE_APERTURE_BIG_VIDEO_MEMORY                0x00000001
NV_MMU_PDE_APERTURE_BIG_SYSTEM_COHERENT_MEMORY      0x00000002
NV_MMU_PDE_APERTURE_BIG_SYSTEM_NON_COHERENT_MEMORY  0x00000003
```

`turing/tu102/dev_mmu.h:26-30` (`_SMALL` at `:38-42`), **identical back to
`maxwell/gm107/dev_mmu.h:26-30`** ⇒ arch-stable, on the chip seam, not a GA10x special case. RM
reads it the same way we would: `gmmuFieldGetAperture(&pFmt->pPde->fldAperture, entry.v8)`, field
wired at `kern_gmmu_fmt_gm10x.c:98,107`.

⇒ Walk from the PDB we already learn at bind / promote / `SET_PAGE_DIRECTORY`. **Any PDE reading
`SYSTEM_*` means that VAS has page-table storage we cannot see** ⇒ **refuse the channel by
name** instead of running it wrong. That converts the campaign's actual danger — *silent
corruption* — into a **named refusal**, which is the owner's *"not found, not denied"* discipline
applied to our own blindness.

⚠ **Two limits, and they scope it rather than sink it:**
- **Point-in-time.** A level allocated in sysmem *later* appears with no signal. ⇒ re-walk at
  every event we DO observe, and treat *"vidmem-only"* as **provisional, never permanent**.
- **The walk races the guest's own writes.** ★ But the error is **asymmetric**: a false *"blind"*
  is safe (we refuse work that would have been fine); only a false *"clear"* is dangerous. Bounded
  by re-walking at bind.

### 12.4 ⊘ S3 — no pre-Hopper forcing exists. Complete enumeration.

Every writer of `pGpu->instLocOverrides`:
1. `gpu_registry.c:258-261` — the CC/NVLE branch. **Device-read but Hopper+ only** (pre-Hopper
   binds the stub `gpuIsCCEnabledInHw_3dd2c9 { return NV_FALSE; }`, `g_gpu_nvoc.h:5197-5199`).
2. `gpu_registry.c:273-276` — `osReadRegistryDword` of `NV_REG_STR_RM_INST_LOC{,_2,_3,_4}`.
   **Host module param only, never device-supplied.**
3. `_gpuInitGlobalSurfaceOverride` (`:382-420`) — gated on `bInstLoc47bitPaWar`, applies
   `GP100_BYPASS_47BIT_PA_WAR`. **Declines to act when any override is already non-zero**
   (`:387-397`) and does not set PDE/PTE to `_VID`. Not a lever.

No GSP-static-info, VBIOS or InfoROM field constrains page-table placement. ⚠ **Unfound, not
proven absent** — but `bAllowSysmem`'s only inputs are `instLocOverrides` plus the
`VASPACE_FLAGS_BAR`/`_PMU` checks (`gmmu_walk.c:172-176, 255-257, 266-268`), and all three trace
to the setters above.

### 12.5 WHERE THIS LEAVES THE DECISION

- **S4 now, unconditionally.** Cheap, arch-stable, unprivileged, composes with every other
  option, and it is the only one that improves the *failure mode* rather than the coverage.
- **The coverage fork is S2 vs CC**, and both have a non-technical price:
  **S2 keeps GA10x and costs multi-tenancy; CC is clean and costs everything below Hopper.**
- **§6's write-protection remains the third path** — it keeps both GA10x *and* multi-tenancy, and
  pays in per-page uffd traps. It is the only survivor that costs no architecture.

⚠ **Two items still open and NOT cleared:** `_confComputeInitRegistryOverrides`
(`conf_compute.c:127`) decides whether the CC bit alone suffices without a guest regkey; and the
host-side replayable-fault notification path, which only becomes live again if S2 is chosen.

---

## 13. ⊘⊘ CORRECTION TO §11 — THE CC BIT'S TWO EFFECTS ARE ONE WIN WITH TWO NECESSARY PARTS

**2026-09-07, same rung, found by re-reading `memmgrGetMemTransferType` in full.** §11 presents the
CC bit as **two independent forcings** — *"(1) it kills every sysmem page-table route … (2) it
turns the remaining PTE writes into an RPC we serve."* ⊘ **That framing is wrong, and it was
relayed to the owner before being checked.**

`memmgrGetMemTransferType` (`mem_utils.c:60-125`) **short-circuits on sysmem in its FIRST branch,
before `kbusIsBarAccessBlocked` is ever tested.** ⇒ `TRANSFER_TYPE_GSP_DMA` only ever covers
**FB-resident** page tables. **A sysmem page table is still written by a direct CPU store even
with BAR access blocked.**

⇒ ★ **Forcing (2) is only complete BECAUSE forcing (1) removed sysmem.** Neither half alone gives
coverage. They are not two wins; they are one win that needs both parts. ⚠ Same class as this
file's §4 provenance note and §12.1's `RC_ERROR` miscitation — **the third time this rung a true
finding was carried one inferential step past what it measured.**

### 13.1 ★ THE FALLBACK IS REAL — and it is a fallback we cannot trigger

Owner's hypothesis: *"unless the gpu chooses a fallback path"* — i.e. if a failed sysmem
page-table allocation falls back to vidmem, then making it fail is a **forcing**, not a refusal,
and the app keeps running. **Confirmed at source.** `gmmu_walk.c:323-361`:

```c
while (memPoolList[j] != ADDR_UNKNOWN) {
    memdescSetAddressSpace(pMemDescTemp, memPoolList[j]);
    switch (memPoolList[j]) {
        case ADDR_FBMEM:  ... rmMemPoolAllocate(...);              break;
        case ADDR_SYSMEM: memdescTagAlloc(status, ..., pMemDescTemp); break;
    }
    if (NV_OK == status) { ...; break; }   // success -> stop
    j++;                                    // FAILURE -> TRY THE NEXT APERTURE
}
```

★ **And `bPreferSysmemPageTables` is NOT terminal.** The non-root list is
`[SYSMEM, FBMEM, (SYSMEM if RETRY), UNKNOWN]` (`gmmu_walk.c:270-285`) — sysmem is placed *first*,
but `ADDR_FBMEM` is still appended after it. A failed sysmem allocation falls straight through to
vidmem, the level is created in FB, and **the application keeps running with page tables we can
see over BAR2.**

★★ **Corollary that makes §12.3's switch cheap:** for the **root/PDB** the order is REVERSED —
`[FBMEM (default), SYSMEM (fallback)]` (`gmmu_walk.c:176-182`) — so the root is in sysmem only if
vidmem allocation *failed*. **The root is therefore almost always FB-resident and readable**,
which is exactly what a detector needs to bootstrap from.

⊘ **BUT WE HAVE NO LEVER.** `memdescTagAlloc` → `_memdescAllocInternal` →
`case ADDR_SYSMEM: osAllocPages(pMemDesc)` — ordinary **guest-kernel** page allocation. As the
emulated GPU we are not on that path. Searched for a device-reported constraint on page-level
sysmem allocation (`kgmmuGetPDEAperture/Attr`, `kgmmuGetPTEAperture/Attr`,
`kgmmuGetPDBAllocSize_HAL`, `bAllowSysmem`) — none. `dma_dev->addressable_range`
(`kernel-open/nvidia/nv-dma.c:53-54`) does constrain sysmem DMA but is host-driver/IOMMU state,
and narrowing it would fail **all** sysmem users nondeterministically — which breaks apps, and the
owner has ruled that out.

⇒ **A fallback we cannot trigger.** ★ It is still worth recording, because it means **any**
mechanism that makes sysmem PT allocation fail is *automatically safe* — if a lever is ever
found, no further design work is needed.

### 13.2 ⊘ NO OTHER ROUTE TO A BLOCKING CALL — complete enumeration

Every value `memmgrGetMemTransferType` can return:

| return | condition | lever? |
|---|---|---|
| `PROCESSOR` | **first branch** — dst/src both sysmem, `!RMCFG_FEATURE_PLATFORM_GSP` | ⊘ the silent default |
| `CE` | needs the caller's `TRANSFER_FLAGS_PREFER_CE` **and** `pCeUtils != NULL` | ⊘ caller-supplied only |
| `BAR0` | `IS_SIMULATION(pGpu) && pSrc != NULL`, *inside* the PREFER_CE branch | ⊘ **DEAD CODE** |
| `GSP_DMA` | `kbusIsBarAccessBlocked(pKernelBus)` | CC only |
| `PROCESSOR` | fallthrough default | — |

- **`bBarAccessBlocked` has exactly two assignments in the whole tree**: `kern_bus_gm107.c:392`
  (TRUE, under `IS_GSP_CLIENT && gpuIsCCFeatureEnabled && !bForceBarAccessOnHcc`) and `:398`
  (FALSE). ⇒ **CC is the only route. Confirmed complete.**
- **CE is unreachable for page tables**: the walker passes only
  `TRANSFER_FLAGS_SHADOW_ALLOC | TRANSFER_FLAGS_SHADOW_INIT_MEM`
  (`virt_mem_allocator_gm107.c:2063-2074`); nothing adds `PREFER_CE` on its behalf.
- ★ **`BAR0` is dead code, and this is the compiled-out trap again**: it needs
  `pGpu->bIsSimulation`, **declared at `g_gpu_nvoc.h:1419` and never assigned anywhere in `src/`**.
  Permanently zero. A design built on that branch would have been built on nothing.

### 13.3 ⊘ THE REGISTRY IS MODULE-PARAMETERS ONLY — the device cannot inject a key

Traced in the Linux open module rather than assumed: `osReadRegistryDword` →
`osReadRegistryDwordBase` (`os.c:1857`) → `RmReadRegistryDword` (`registry.c:239`) →
`regFindRegistryEntry` over an in-memory list, populated in **exactly one** initializer,
`os_registry_init` (`kernel-open/nvidia/os-registry.c:309-357`), from four module-parameter
sources: `NVreg_RmNvlinkBandwidth`, `NVreg_RmMsg`,
`rm_parse_option_string(NVreg_RegistryDwords)`, and the `nv_parms[]` table.

- **Per-device keys exist but are host-supplied**: `NVreg_RegistryDwordsPerDevice`
  (`os-registry.c:193`, documented `nv-reg.h:225-254`) is keyed by PCI BDF, but the BDF *and* the
  values come from the host admin's parameter string. **The device supplies nothing.**
- **RM writes registry keys at runtime, but never this one.** Every `osWriteRegistryDword` caller
  enumerated (`gpu.c:5281,6234`, `gpu_registry.c:125`,
  `subdevice_ctrl_gpu_kernel.c:2095,3751,3756`, `subdevice_ctrl_vgpu.c:98`, `kernel_gsp.c:3920`,
  `objvgpu.c:186,201`) — **none targets `NV_REG_STR_RM_INST_LOC*`**.
- No VBIOS / InfoROM / GSP-static-info path into the registry found.

⚠ Search stated so it is auditable: `osReadRegistryDword`, `osReadRegistryDwordBase`,
`RmReadRegistryDword`, `regFindRegistryEntry`, `regCreateNewRegistryKey`, `RmInitRegistry`,
`os_registry_init`, `NVreg_RegistryDwords`, `NVreg_RegistryDwordsPerDevice`, `nv_parms`, all
`osWriteRegistryDword` callers, `NV_REG_STR_RM_INST_LOC` writers — across
`src/nvidia/arch/nvalloc/unix/`, `kernel-open/nvidia/`, `src/nvidia/src/kernel/`.

---

## 14. ★★★ THE MECHANISM QUESTION — uffd IS NOT ACCEPTABLE, AND KVM DIRTY LOGGING MAY BE

> Owner, 2026-09-07: *"uffd is not recommended if it requires privileges."*

★ **He is right, and it is worse than a preference — it is a deployment requirement imposed on
every host.** `userfaultfd(2)` needs `CAP_SYS_PTRACE` unless `vm.unprivileged_userfaultfd = 1`
(**Ubuntu ships 0**) or `/dev/userfaultfd` exists with a permissive udev rule (Linux 6.1+).
⚠ This tree already knew: `kvm_unsafe.rs:119` and `:645` name *"§6.8.1's `/dev/userfaultfd` udev
rule: **no type and no CI grep can observe it**"* — a known-unobservable deployment dependency.

⊘ **Nothing is committed either way**: `UFFDIO_WRITEPROTECT` and `DIRTY_LOG` both have **zero**
occurrences in the tree; the only `KVM_CAP` we probe is `KVM_CAP_NR_MEMSLOTS`
(`kvm_unsafe.rs:58-59`).

★ **The privilege-free alternative: `KVM_CAP_MANUAL_DIRTY_LOG_PROTECT2`.** Enable dirty tracking
*without* auto write-protect, then `KVM_CLEAR_DIRTY_LOG` over exactly the page-table pages to
protect precisely those. A guest write takes an EPT violation, KVM sets the bitmap bit and
unprotects **entirely in-kernel**, guest continues; we read the bitmap at the consumption point
(doorbell / bind). Better than uffd on four axes:
- **no privilege** — the VM fd we already hold, no sysctl, no udev rule, no capability;
- **no vCPU stall into our process** — uffd parks the faulting thread until *our* handler answers,
  a synchronous round-trip on the critical path and exactly the class of thing that has quietly
  grown before here;
- **well-trodden** — it is QEMU's live-migration mechanism, not novel plumbing;
- **arm64 works**, keeping that axis green.

⊘ It is a **poll, not a trap** — we learn *"this page changed"* when we look. For us that is the
right shape: the requirement is *before work runs*, and the consumption points already exist.

⚠ **THREE THINGS UNVERIFIED — this section is API recollection, not a source check:**
1. Whether `KVM_CLEAR_DIRTY_LOG` genuinely **re-protects** a named sub-range or merely clears
   bits. **If it only clears, the mechanism does not work as described and §14 collapses.**
2. Reaching it through **both** VMM backends — direct ioctls on the KVM adapter, the memory API
   on QEMU — through a neutral seam per the *"no QEMU-only mechanism"* directive.
3. Bitmap granularity against how guest RAM is sliced into memslots today.

⇒ ★ **AND #48 GOES BACK ON THE TABLE.** The *"uffd everywhere"* ruling (owner, 2026-07-27) was
made for the **isolate's own window VMA** — `kayfabe-vmm/src/lib.rs:617,744`, *"`UFFDIO_REGISTER`
on **our own** window VMA, which needs no cooperation from any"* — which is a different problem
from write-protecting **guest RAM**. Per `a_rulings_date_is_part_of_the_citation`: ask *why* it
decided that, and whether the why survives today's use. **Here it does not obviously survive, so
it should be re-decided rather than inherited.**

---

## 15. ⊘⊘⊘ TWO CORRECTIONS THAT CHANGE THE THREAT MODEL — the silent path is (a) NOT on the CUDA path, and (b) NOT DEMONSTRATED

**2026-09-07, same rung, both prompted by the owner.** §4 established the silent conjunction is
*reachable*. It said nothing about whether **anything takes it**, or whether taking it **works**.
Both questions have now been asked and both cut against this document's own framing.

### 15.1 ★★★★★ libcuda NEVER ISSUES `MAP_MEMORY_DMA` — measured, from committed data

> Owner: *"even if the silent path is possible, it also depends on if libcuda does it."*

`[measured 2026-09-07 from `traces/host_reference_ga106/`, committed 2026-08-10]` — real GA106
(RTX 3060), **unvirtualised host**, open driver **580.173.02**, CUDA 12.6.2,
`libcuda.so.580.173.02`, captures untruncated at `NVDIFF_MAXBUF=65536`:

| stage | `NV_ESC_RM_MAP_MEMORY_DMA` (0x57) | `UNMAP` (0x58) | UVM ioctls |
|---|---|---|---|
| `init` ×2 | **0** | 0 | 5 |
| `dev` ×2 | **0** | 0 | 5 |
| `ctx` ×2 | **0** | 0 | 131 |
| `alloc` ×2 | **0** | 0 | 134 |
| `ce` ×2 | **0** | 0 | 134 |
| `launch` ×2 | **0** | 0 | 134 |

⇒ **ZERO across all twelve traces, every stage, both replicates.**

★ **And this is not an empty-capture artefact — there is a positive control.** The same recorder
logs **131–134 UVM ioctls per trace** in exactly the stages that map memory. The instrument is
demonstrably seeing the mapping traffic; it simply is not `MAP_MEMORY_DMA`.
(`a_census_zero_needs_a_known_positive`, satisfied.)

⇒ ★★★★★ **`NVOS46_FLAGS_DEFER_TLB_INVALIDATION` IS A FLAG ON AN IOCTL CUDA NEVER ISSUES.**

**This re-scopes the whole rung:**
- The **DEFER + sysmem silent map** is a **hostile-guest / raw-client** concern. It does **not**
  gate phase 1 (*compute works*) or phase 2 (*CUDA apps must pass*). It matters for
  `hostile_guest_isolation_is_the_value_proposition`, which is real but is not the roadmap's
  current gate.
- **The path CUDA actually uses is UVM** — §9 residual 1, listed all night and never opened.
  UVM manages its own page tables, writes PTEs by CPU store or CE copy, and invalidates as
  **pushbuffer methods**. It is uncovered for *entirely different reasons* than everything in
  §1–§14.

⇒ **The priority inverts.** This rung characterised the path that does not block the roadmap; the
one that does was carried as a residual.

> ### ⊘⊘⊘ CORRECTED WITHIN THE HOUR (owner, 2026-09-07) — **THE SCOPING BELOW IS OVER-CLAIMED,
> ### AND THE CAVEAT WAS DOING ALL THE WORK.**
> Owner: *"the only library issuing ogkm ioctl calls on bare metal is libcuda, so if the ioctl
> exists, there is probably a path in cuda to invoke it/trigger it."*
> ★ **The prior is right and it is the correct one**: RM serves `MAP_MEMORY_DMA` fully, with a
> documented **client-facing** flag on it. Maintained code with no caller is the exception.
> ⊘ **The precise error:** §15.1 has a positive control for the **RECORDER** (UVM ioctls were
> captured, so the instrument works) and **NONE for the WORKLOAD**. `nvd_prog.c` is the
> `cup2`/`cup3` shape — context, plain `cuMemAlloc`, one CE copy, one launch. **Zero-for-this-
> program is not zero-for-libcuda**, and letting a census over one workload stand as a claim
> about a library is `a_census_over_transports_is_as_complete_as_its_list` in a new coat.
> ⇒ **The verdict "hostile-guest scope only" is WITHDRAWN pending the widening below.** What
> survives is narrower and still useful: *the `cup2`/`cup3` compute shape does not take this
> path.*
>
> ★★ **AND THE SURFACE IS WIDER THAN "libcuda" ANYWAY.** On a bare-metal box, RM ioctls are also
> issued by **NVML** (which is how `nvidia-smi` works — one of our own milestones),
> **libGL/libEGL**, **NVENC/NVDEC**, and **nvidia-modeset**. Graphics and video do **not** go
> through UVM; they are the classic RM DMA-mapping clients, which is plausibly *why*
> `MAP_MEMORY_DMA` exists and is maintained.
>
> ★★★ **THE WIDENING EXPERIMENT — cheap, bare metal, no guest, no new instrument.** Extend the
> probe program and re-run the existing `nvdiff` `LD_PRELOAD` shim, counting `0x57` **per API**:
> - the **CUDA VMM API** — `cuMemAddressReserve` / `cuMemCreate` / `cuMemMap` / `cuMemSetAccess`
>   ⇐ ★ rank 1: its entire purpose is client-managed explicit mapping, which is exactly what
>   `NV04_MAP_MEMORY_DMA` provides
> - `cuMemHostRegister` (pinned host memory)
> - `cuIpcGetMemHandle` / `cuIpcOpenMemHandle`
> - peer access across two GPUs
> - graphics interop
>
> **If any lights up, §15.1's conclusion INVERTS and the DEFER path is back on the application
> path.** Until then treat §15.1 as *"unmeasured outside the compute shape"*, not as a scoping
> ruling.

⚠ **Scope of the claim, stated rather than buried:** one workload (`nvd_prog.c`, the `cup2`/`cup3`
shape), one driver version, one arch. Graphics, NVENC, or a broader CUDA surface could still reach
`MAP_MEMORY_DMA`. The honest claim is **"not on this workload's path"**, NOT *"libcuda never does
it"*. Widening it is a cheap `nvdiff` re-run against a richer program.

### 15.2 ⊘⊘ AND (A) WAS NEVER ESTABLISHED — this document asserted it

> Owner: *"so we know confirmed a cache miss is a walk on real gpu?"*

**No.** §9 residual 2 marks negative caching as *inferred*; but §4 and §12 then reason throughout
as if the silent map **works**. That is (A) treated as fact. **The correct status is UNDECIDED,
and the driver's own behaviour models (B).**

**The fork, restated:**
- **(A)** a TLB miss walks and picks up a freshly-written PTE ⇒ silent path real, w387 stands.
- **(B)** a fresh PTE is not live without an invalidate ⇒ DEFER is a **batching contract**: the
  client must still invalidate before use. Then **either the invalidate arrives (we see it) or the
  mapping never works (the guest has only broken itself)** — and the silent path stops being a
  correctness hole for us at all.

**Q1 — the contract, and it leans (A).** `nvos.h:2144-2148`, immediately above the flag:
> *"This flag must be used with caution. Improper use can leave **stale entries in the TLB**, and
> allow access to memory no longer owned by the RM client or cause page faults."*

Read precisely: the documented hazard is **stale positives** — a revoked or remapped page still
reachable. It does **not** say a freshly-inserted valid PTE fails to take effect. The control-side
prose confirms batching-with-obligation: `NV2080/NV0080_CTRL_CMD_DMA_INVALIDATE_TLB` are
*"intended to be used by RM clients that manage their own TLB consistency … or with
DEFER_TLB_INVALIDATION options"* (`ctrl2080dma.h:37-42`, `ctrl0080dma.h:456-461`), the 2080 form
usable with class `0x5080`.

★ **And a fact this document should have had: NO in-tree RM or UVM code sets `DEFER = TRUE`.**
Grepped all of `src/` — the only consumer is the passthrough of the *client's* flag at
`virt_mem_allocator_gm107.c:417`. So **no in-tree caller's behaviour defines the contract by
example**; deferral is a pure client-facing accommodation, and **nothing asserts or times out if
the deferred invalidate never arrives.** The client is trusted and unchecked.

**Q2 — the hardware, and every driver path models (B).** No hardware statement found (searched
`dev_mmu.h`, `dev_fault.h` across ampere/turing/… for `prefetch`, `cache`, `negative`, `speculat`,
`invalid.*cache` — the published headers do not document TLB fill policy on a miss). But:

- **RM always invalidates on a fresh valid map.** `PTE_UPGRADE` still drives `kgmmuInvalidateTlb`
  → the `0xB830B0` write; the only thing skipped for upgrade-vs-downgrade is the extra
  **sysmembar** (`kern_gmmu_gm107.c:222-224`), **not the invalidate**. RM never relies on *"the
  walk will pick it up."*
- ★★★ **UVM's own comment is the sharpest evidence in either direction.**
  `uvm_mmu.c:805-808`: *"Upgrades don't have to flush out accesses, so **no membar** is needed on
  the TLB invalidate"* — **and it still issues `tlb_invalidate_all`.** If the MMU never cached the
  non-present result, that invalidate on a pure upgrade would be **dead work**, and authors who
  hand-tune every membar and pipeline flag two lines above would have omitted it. **They keep it.**
- UVM's ATS-deinit comment (`uvm_mmu.c:1194-1201`) distinguishes already-invalidated GMMU entries
  from stale ATS entries needing eviction — the authors reason concretely about what is cached,
  and still invalidate defensively.

**Synthesis, stated carefully because (B) is the convenient answer:**
- a VA/PDE range **never walked since becoming invalid** ⇒ first touch misses, walks, reads the
  current PTE ⇒ **(A)**, silent. Unrefuted, and supported by nvos.h's stale-only hazard framing.
- a range **previously walked while invalid or sparse** ⇒ a cached negative exists and the
  invalidate is **required** ⇒ **(B)**.

⇒ ⊘ **Do not build on (A) as measured.** The silent hole is *plausible* for genuinely fresh VAs
and is *not demonstrated by any code path*, while the entire NVIDIA stack — RM and UVM both —
treats a fresh map as requiring an invalidate.

### 15.3 ★★ THE DECIDING EXPERIMENT — and the PRIMING STEP is the whole point

Owner: *"we can first test it on bare metal by doing the exact silent path using raw
ioctls/client."* One real GA106, host-side, raw RM client, **no guest, no UVM**.

**Setup:** `NV01_ROOT` → `NV01_DEVICE_0` → `NV20_SUBDEVICE_0`; a **fresh** `FERMI_VASPACE_A` (own
PDB, no unrelated cached state); one 2 MiB `NV01_MEMORY_LOCAL_USER` data page + one semaphore
page; a `*_DMA_COPY_A` CE channel bound to that VAS.

1. Reserve VA range `V` in the space, left **sparse/invalid**. ⚠ **All** page-level
   instantiation must happen here — see confounder 2.
2. ★ **PRIME THE NEGATIVE CACHE:** CE-copy *from* `V`. It faults (non-replayable), the channel
   RCs — but the MMU has now **walked `V` and may have cached the non-present result**. Re-create
   the channel for step 5; the VAS/TLB state persists.
3. Map the data page at `V` via `NV04_MAP_MEMORY_DMA` with **`DEFER_TLB_INVALIDATION = TRUE`**.
4. **Issue no invalidate by any transport.**
5. CE-copy from `V` to a known buffer; poll the semaphore.

| step 5 result | means | verdict |
|---|---|---|
| **faults / stale** | the primed non-present entry survived the DEFER map | **(B)** — DEFER is a batching contract we are protected by |
| **completes correctly** | a cached non-present entry did not block the fresh PTE | **(A)** — the silent path is real, w387 stands |

**Negative control (must PASS, else the rig is broken):** identical 1–3, then **do** issue
`NV2080_CTRL_CMD_DMA_INVALIDATE_TLB` before step 5.

⚠ **Confounders, named because two of them make a naive run worthless:**
1. ★★★ **WITHOUT STEP 2 THE EXPERIMENT PROVES NOTHING.** A pass would then mean *"no entry was
   ever cached for `V`"* — **physically identical** to *"fresh VA, the walk picks it up."* It
   cannot distinguish (A) from (B). The priming touch is what makes step 5 test **eviction**
   rather than **first-fill**.
2. **RM must not sneak an invalidate in between 3 and 5.** The DEFER map itself provably skips it
   (`:2612`), but **VA setup does not**: `gvaspaceIncAllocRefCnt`/sparsify calls
   `gvaspaceInvalidateTlb` (`gpu_vaspace.c:1608`). So all page-level instantiation belongs in
   step 1, and step 3 must write **only a leaf PTE**. Verify with `GET_PTE_INFO` that step 3
   changed only the leaf.
3. **Any global invalidate anywhere on the GPU** — another tenant, another VAS — clears the primed
   state. Run on an otherwise-idle GPU.
4. **Scope:** this tests the **leaf** level on **GA10x**. It does not settle PDE-level behaviour
   (where the UVM evidence already leans (B)) nor other arches.

⇒ ★ And note the experiment's urgency changed with §15.1: it is now an **isolation** question, not
a **correctness-for-real-apps** one. Same recipe, different priority.

### 15.4 WHERE THIS LEAVES w387

| claim | status after §15 |
|---|---|
| the silent conjunction is **reachable** | ✔ measured (§4) — unchanged |
| anything **takes** it | ◐ **not on the `cup2`/`cup3` compute shape** (§15.1) — ⊘ the broader *"not on the CUDA path"* reading is **WITHDRAWN**; widening experiment specified |
| taking it **works** | ⊘ **UNDECIDED** (§15.2) — driver behaviour models (B) |
| **UVM** is uncovered | ✔ unchanged — and it is the path CUDA **actually uses** |

⇒ **The next rung is UVM, not this.** §1–§14 characterised a path no application takes, and may
not even function. The path that gates the roadmap was residual 1 the whole time.
