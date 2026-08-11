# The GMMU publication discipline — what a real driver does while compute is running

> ### STATUS — 2026-08-11 (w258 doc-hygiene sweep) / **LIVE — built TO, with one measured refinement it does not carry**
>
> Checked from git history. Three later commits build *to* this doc rather than past it, all
> 2026-08-08: `5af87b9` (*"§6.3/§7 rule 1: walk-on-demand is safe"*), `8cdde02` (*"§7 rule 6 is
> 'never cache the walk'"*), `754e393` (*"§7 rule 6 plus the measured vacuity of rule 7 on this
> path"*) — four obstacles closed and the doorbell served.
>
> ⚠ **One refinement measured elsewhere and never written back here: `754e393` found §7's RULE 7
> VACUOUS on that path.** The rule is not wrong; it simply had no members there. ⊘ Do not read
> rule 7's presence below as evidence it fires — check the path before relying on it.

**Question this answers (the owner's words):** *"How does a real GPU handle GMMU updates if compute
is still running, as the page can look corrupted? Does it use a fence to sync, or rely only on TLB?
Something safe must exist."*

**Decision it gates:** whether `mode2_address_table.md` §6's *"a miss is a fault, never a walk,
never a guess"* can be relaxed to **walk-on-miss**, which would in turn let `#102`'s witness
requirement relax and make the guest framebuffer 100 % passthrough with no page-table-write
trapping.

**Answer in one line.** Something safe does exist, it is **not atomicity**, and it is **not the TLB
alone**: it is a *publication protocol* — *a reachable entry is never uninitialised* + *flush* +
*an explicit, per-PDB TLB invalidate* — and on this driver that invalidate is **guest-visible,
trappable, and fires on the compute path**, through a transport `mode2_address_table.md` §5 never
enumerated.

**Epistemic classes used below** (per `claim_ledger.md`): `[src@580]` = read out of
ogkm-580.159.04, nothing ran. `[meas]` = read out of a committed capture in
`traces/mode2_c_reference/`, which is a recording of a real stock guest driver.

---

## 0. The finding that changes the question

`mode2_address_table.md` §5's ★ CORRECTION enumerates **two** invalidate transports — the
`INVALIDATE_TLB` RPC (fn 200) and the `MEM_OP`/`MMU_TLB_INVALIDATE` pushbuffer method — records
both at zero on the compute path, and concludes that on that path *"the guest commits nothing"*.
§6 then reasons from that absence.

**There is a third transport, and it is the one this driver uses.** On Turing and later, a
bare-metal-presenting driver issues the GMMU TLB invalidate as a **BAR0 PRI register write**, not
as an RPC and not as a pushbuffer method:

```c
GPU_VREG_WR32(pGpu, NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE, pParams->regVal);
// Wait for the invalidate command to complete.
status = kgmmuCheckPendingInvalidates_HAL(pGpu, pKernelGmmu, &pParams->timeout);
```
ogkm-580: `src/nvidia/src/kernel/gpu/mmu/arch/turing/kern_gmmu_tu102.c:117-119` `[src@580]`

The RPC arm is reached **only** when `bDoVgpuRpc` — a vGPU guest whose host has set
`VF_INVALIDATE_TLB_TRAP_ENABLED` — ogkm-580:
`src/nvidia/src/kernel/gpu/mmu/arch/maxwell/kern_gmmu_gm107.c:152-165` and `:236-243`. So §5's
statement that *"In GSP mode the privileged MMU register is owned by GSP, so the invalidate is
RPC'd"* does not hold for the configuration we emulate. `[src@580]`

`GPU_VREG_*` on bare metal adds `DRF_BASE(NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET)` = `0x00B80000`
(ogkm-580: `src/nvidia/src/kernel/gpu/arch/turing/kern_gpu_tu102.c:93-100`;
`src/common/inc/swref/published/turing/tu102/dev_vm.h:28`;
`src/nvidia/generated/g_gpu_access_nvoc.h:257`), so the three registers are, in BAR0:

| BAR0 offset | register |
|---|---|
| `0x00B830A0` | `NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE_PDB` (aperture bit 1, addr 31:4, shift 12) |
| `0x00B830A4` | `..._MMU_INVALIDATE_UPPER_PDB` |
| `0x00B830B0` | `..._MMU_INVALIDATE` (TRIGGER bit 31) |

ogkm-580: `src/common/inc/swref/published/ampere/ga100/dev_vm.h:63-118`. `[src@580]`

### It fires, and it fires on the compute path

Scanning the committed captures for BAR0 traffic at those three offsets (decoder:
`scripts/mode2_diag/rec_dump.py` record format; scan run 2026-07-31 over
`traces/mode2_c_reference/cap1_coldboot_hermetic` and `cap3_matmul_forwarding`) `[meas]`:

| capture | `MMU_INVALIDATE` writes | of which `HUBTLB_ONLY=1` (a BAR VAS) | of which `HUBTLB_ONLY=0` (a GPU VAS) | distinct PDBs |
|---|---|---|---|---|
| `cap1_coldboot_hermetic` (boot only) | 139 | 89 | 50 | 6 |
| `cap3_matmul_forwarding` (`cup8`, `bad=0 maxerr=0`) | 308 | 177 | **131** | 9 |

Every write carried `TRIGGER=1`, `ALL_VA=1`, `ALL_PDB=0`, `INVAL_SCOPE=NON_LINK_TLBS` — i.e. each
one names **exactly one PDB**, written into `0x00B830A0/0xA4` immediately before the trigger. Three
PDBs (`0x3400000` ×51, `0x3114000` ×29, `0x3110000` ×1 — 81 invalidates) appear in
`cap3_matmul_forwarding` and in **neither** boot-only capture: those are the CUDA context's own
address spaces. `[meas]`

The full six-step sequence is verbatim in the trace, here at `cap3_matmul_forwarding` record
204590 (and identically in the hermetic `cap1_coldboot_hermetic` at 204569, so it is the guest and
not a forwarding artefact) `[meas]`:

```
MmioWr bar2 0x26358 = 0x12b5f90d   <- leaf entry, low half  (VALID + low address)
MmioWr bar2 0x2635c = 0x06000000   <- leaf entry, high half (high address + kind)
MmioRd bar2 0xfea000               <- read-to-flush: push BAR2 posted writes into FB
MmioRd bar2 0xfea000
MmioRd bar0 0xb830b0 -> 0           <- is an invalidate already pending?
MmioWr bar0 0xb830a0 = 0x02efba50   <- PDB of the VAS being committed
MmioWr bar0 0xb830a4 = 0x0
MmioWr bar0 0xb830b0 = 0x80010001   <- TRIGGER.  THIS IS THE COMMIT POINT.
MmioRd bar0 0xb830b0 -> 0           <- poll until TRIGGER clears
```

The read-to-flush is `MEM_RD32(pKernelBus->pReadToFlush)` — a CPU read through a long-lived BAR2
mapping of an FB page created for exactly this purpose (ogkm-580:
`src/nvidia/src/kernel/gpu/bus/arch/volta/kern_bus_gv100.c:66-92` and `:381-395`). `[src@580]`

**Consequence for the decision.** The commit signal §6 says the guest never emits *is emitted*,
131 times on the compute path, at one 4-byte BAR0 offset, carrying the PDB it applies to. Whether
that changes §6's ruling is the owner's call; §9 below records the inconsistency without resolving
it.

---

## 1. The atomicity unit — there isn't one

**Entry widths (GMMU FMT V2, which is what Ampere uses).** `NV_MMU_VER2_PDE__SIZE` = 8,
`NV_MMU_VER2_DUAL_PDE__SIZE` = 16, `NV_MMU_VER2_PTE__SIZE` = 8 — ogkm-580:
`src/common/inc/swref/published/pascal/gp100/dev_mmu.h:97`, `:112`, `:157`. Level table:
ogkm-580: `src/nvidia/src/kernel/gpu/mmu/arch/pascal/kern_gmmu_fmt_gp10x.c:61-102`; Ampere reuses
it wholesale and only marks PD1 as also being a page table (512 MB leaf) — ogkm-580:
`src/nvidia/src/kernel/gpu/mmu/arch/ampere/kern_gmmu_fmt_ga10x.c:46-53`. UVM agrees:
`entry_size_pascal()` returns 16 at depth 3 and 8 elsewhere — ogkm-580:
`kernel-open/nvidia-uvm/uvm_pascal_mmu.c:172-179`. `[src@580]`

**Nothing enforces atomic publication of an entry.** Not one `portAtomic*`, not one
`MEMORY_BARRIER`, not one comment claiming atomicity appears on any of the entry-write paths:

- RM composes the whole entry in a stack buffer and then issues **one** write of `entrySize`
  bytes — ogkm-580: `src/nvidia/src/kernel/gpu/mmu/gmmu_walk.c:731` (`portMemSet(entry.v8, …)`) and
  `:797-803` (`memmgrMemWrite(… entry.v8, pLevelFmt->entrySize …)`). But that "one write" bottoms
  out, for the CPU transfer type, in `portMemCopy(pDst, size, pBuf, size)` into a BAR2 mapping —
  ogkm-580: `src/nvidia/src/kernel/gpu/mem_mgr/mem_utils.c:786-794`. A `memcpy`, with no width
  guarantee.
- UVM's CPU writer is a `memcpy` of a 16-byte struct with only an alignment assert — ogkm-580:
  `kernel-open/nvidia-uvm/uvm_mmu.c:339-359`. Its GPU writer is one CE `memcopy` of
  `entry_count * entry_size` — ogkm-580: `kernel-open/nvidia-uvm/uvm_mmu.c:436`.
- The only width rule UVM states is a *semantic* one, not a tearing one: never read-modify-write
  half of a dual PDE — *"GPU PDEs are always entirely re-written using make_pde"* — ogkm-580:
  `kernel-open/nvidia-uvm/uvm_mmu.h:238-241`; and the 8-vs-16-byte hazard note at ogkm-580:
  `kernel-open/nvidia-uvm/uvm_va_block.c:6533-6537`. `[src@580]`

**And on the wire it is definitively not atomic.** In `cap1_coldboot_hermetic` and
`cap3_matmul_forwarding`, every 8-byte leaf entry reaches BAR2 as **two 4-byte MMIO writes, low
half first** — and the low half is the half that carries `NV_MMU_VER2_PTE_VALID` (bit 0). `[meas]`

That matters because the address field **crosses the dword boundary**:
`NV_MMU_VER2_PTE_ADDRESS_VID` is bits `(35-3):8` = `32:8` and `..._ADDRESS_SYS` is `53:8` —
ogkm-580: `src/common/inc/swref/published/pascal/gp100/dev_mmu.h:119`, `:140`; the PDE fields are
the same shape at `:113`/`:119`. So an observer that samples between the two halves can see
**`VALID=1` with the high address bits missing** — a well-formed entry pointing at the wrong page.
This is the concrete tearing window, and the driver does nothing to close it.

> ⚠ **A correction to something I previously told the owner.** I asserted that "valid-bit-last is
> the standard discipline". That was inference and it is **wrong for this driver on both counts**:
> the valid bit is written *first* (low dword first, §1), and RM's walker publishes the *parent*
> before the leaves (§2.1). The safety does not come from bit ordering.

---

## 2. The publication order, as the code actually does it

Two paths, two *different* orders, both satisfying one invariant.

### 2.1 RM's walker — parent first, but pointing at an all-invalid table

`_mmuWalkLevelInstAcquire` allocates the new level and then fills it to
`MMU_WALK_FILL_INVALID` — ogkm-580: `src/nvidia/src/libraries/mmu/mmu_walk.c:1179-1189`
(`LevelAlloc`) and `:1230-1241` (`FillEntries(..., newEntryState)`), where the contract on
`LevelAlloc` is explicitly *"The contents of the memory need not be initialized. The walker will
initialize entries before use."* — ogkm-580:
`src/nvidia/inc/libraries/mmu/mmu_walk.h:170-172`. Only **after** every sub-level is acquired does
`_mmuWalkPdeAcquire` write the parent PDE — ogkm-580:
`src/nvidia/src/libraries/mmu/mmu_walk.c:1365-1406`. The leaf PTEs are written later still, when
the recursion reaches the target level in `_mmuWalkMap` — ogkm-580:
`src/nvidia/src/libraries/mmu/mmu_walk_map.c:163-169`. `[src@580]`

So RM's order is **child-cleared-to-invalid → parent PDE published → leaves written**. It is *not*
valid-bit-last and *not* children-before-parent. It is safe because the table the parent points at
contains nothing but faults.

⚠ **One documented hole.** `mmuWalkReserveEntries(..., bInvalidate = NV_FALSE)` turns the
clear-to-invalid off — *"Whether to skip invalidation of PTEs during reservation (for example, when
sparsifying immediately afterwards)"* — ogkm-580:
`src/nvidia/src/libraries/mmu/mmu_walk_reserve.c:57-63` and `:85`. For that window a level is
reachable with **uninitialised backing store**, and the invariant is upheld only by the caller
sparsifying immediately. An external walker has no way to know it is inside that window.

**Teardown is the mirror image and is correctly ordered:** `_mmuWalkPdeRelease` clears/rewrites the
parent PDE first (ogkm-580: `src/nvidia/src/libraries/mmu/mmu_walk.c:1509-1540`) and frees the
sub-level backing store second (`:1542-1552`). There is **no TLB invalidate between the two** — the
invalidate happens later, at the caller (ogkm-580:
`src/nvidia/src/kernel/mem_mgr/gpu_vaspace.c:1803-1811`). §8 explains why that gap matters to us.

### 2.2 UVM's page tree — children first, fenced, parents published bottom-up

The opposite discipline, and the only place in either driver where the ordering is *stated*:

```c
    // Only a single membar is needed between the memsets of the page tables
    // and the writes of the PDEs pointing to those page tables.
    // The membar can be local if all of the page tables and PDEs are in GPU
    // memory, but must be a sysmembar if any of them are in sysmem.
    uvm_hal_wfi_membar(&push, membar_after_writes);
    …
    // write entries bottom up, so that they are valid once they're inserted
    // into the tree
```
ogkm-580: `kernel-open/nvidia-uvm/uvm_mmu.c:771-782` (GPU path `write_gpu_state_gpu`, `:732-821`);
the CPU path is the same shape with a plain `mb()` — ogkm-580:
`kernel-open/nvidia-uvm/uvm_mmu.c:681-730` (fence at `:706`). `[src@580]`

New directories are memset to *invalid* (`phys_mem_init`, ogkm-580:
`kernel-open/nvidia-uvm/uvm_mmu.c:461-511`), never to sparse. And UVM writes the rule down:

```c
    // We must enforce the following ordering between operations:
    // PDE write -> TLB invalidate -> MMU fills.
```
ogkm-580: `kernel-open/nvidia-uvm/uvm_mmu.c:1502-1507`. `[src@580]`

### 2.3 The invariant both satisfy

> **A page-table entry that is reachable from the root is never uninitialised.**

A walker descending at any moment sees, for any entry: the previous valid value, or an
*invalid/sparse* value, or (per §1) a torn value. It never sees allocator garbage. The safety
argument rests entirely on this — not on atomicity, not on ordering of bits within an entry.

---

## 3. The fence, in the order it is emitted

`kbusFlush` before invalidate is a written rule: *"NOTE: Must call kbusFlush BEFORE any calls to
busInvalidate"* — ogkm-580:
`src/nvidia/src/kernel/gpu/bus/arch/maxwell/kern_bus_gm107.c:3370-3380`. `[src@580]`

The RM map path, in emission order (ogkm-580:
`src/nvidia/src/kernel/gpu/mem_mgr/arch/maxwell/virt_mem_allocator_gm107.c:2609-2617`, and the
identical shape at `:3057-3062`; same pattern in `gpu_vaspace.c:1605-1608`, `:1628-1633`,
`:1806-1811`):

1. `mmuWalkMap(...)` — entries written (via BAR2, §1).
2. `kbusFlush_HAL(pGpu, pKernelBus, BUS_FLUSH_VIDEO_MEMORY | BUS_FLUSH_SYSTEM_MEMORY)`
   → `portAtomicMemoryFenceFull()` for the sysmem half, then `MEM_RD32(pReadToFlush)` — a CPU read
   through BAR2 — for the vidmem half. ogkm-580:
   `src/nvidia/src/kernel/gpu/bus/arch/volta/kern_bus_gv100.c:360-363`, `:381-395`.
3. `gvaspaceInvalidateTlb(pGVAS, pGpu, update_type)` → `kgmmuInvalidateTlb_HAL` — ogkm-580:
   `src/nvidia/src/kernel/mem_mgr/gpu_vaspace.c:2325-2345`.
4. Inside that: poll `0xB830B0` for no pending invalidate (`kgmmuCheckPendingInvalidates_TU102`,
   ogkm-580: `src/nvidia/src/kernel/gpu/mmu/arch/turing/kern_gmmu_tu102.c:47-82`), write the PDB
   (`kgmmuSetPdbToInvalidate_TU102`, `:127-152`), write `TRIGGER` (`:117`), poll again (`:119`).
5. For a **downgrade** only, `SYS_MEMBAR=TRUE` and `ACK=GLOBALLY` are folded into the same register
   write — ogkm-580: `src/nvidia/src/kernel/gpu/mmu/arch/turing/kern_gmmu_tu102.c:157-179` — plus,
   pre-Turing, trailing `kbusSendSysmembar` loops — ogkm-580:
   `src/nvidia/src/kernel/gpu/mmu/arch/maxwell/kern_gmmu_gm107.c:250-256`.

Steps 1→4 are exactly what the trace shows (§0). All 308 invalidate writes in
`cap3_matmul_forwarding` had `SYS_MEMBAR=0, ACK=NONE_REQUIRED` — i.e. **every one was an upgrade**;
no `PTE_DOWNGRADE` invalidate appears anywhere in the capture. `[meas]`

UVM's fence is the in-push `uvm_hal_wfi_membar` of §2.2, and its invalidate is a pushbuffer
`MEM_OP_A..D` (`NVC56F_MEM_OP_A` = `0x28`, ogkm-580: `kernel-open/nvidia-uvm/clc56f.h:119`) emitted
by `uvm_hal_ampere_host_tlb_invalidate_all/_va` — ogkm-580:
`kernel-open/nvidia-uvm/uvm_ampere_host.c:213-271`, `:274-376`. Notably UVM passes
`UVM_MEMBAR_NONE` on an **upgrade** invalidate: *"Upgrades don't have to flush out accesses, so no
membar is needed on the TLB invalidate."* — ogkm-580:
`kernel-open/nvidia-uvm/uvm_mmu.c:804-809`. Downgrades pick GPU-vs-SYS via
`uvm_hal_downgrade_membar_type()` — ogkm-580: `kernel-open/nvidia-uvm/uvm_hal.c:964-979`.
`[src@580]`

---

## 4. The live-compute case, per path

**The driver does not quiesce, and it does not rely on atomicity. It relies on faulting.**

- **Page tables are updated while user channels run.** UVM pushes page-table work on a dedicated
  internal channel, `UVM_CHANNEL_TYPE_MEMOPS` — *"Memops and small memsets/copies for writing
  PTEs"* — ogkm-580: `kernel-open/nvidia-uvm/uvm_channel.h:87-88`;
  `kernel-open/nvidia-uvm/uvm_mmu.c:46-67`. `[src@580]`
- **The hardware contract that makes that safe is fault-and-stall.** *"replayable faults block
  preemption of the channel until software (UVM) services the fault … Note that replayable faults
  prevent the execution of other channels, which are stalled until the fault is serviced."* —
  ogkm-580: `kernel-open/nvidia-uvm/uvm_gpu_non_replayable_faults.c:78-81`. Service → replay:
  `push_replay_on_gpu()`, ogkm-580: `kernel-open/nvidia-uvm/uvm_gpu_replayable_faults.c:503-544`.
  `[src@580]`
- **UVM says outright that an in-flight access is allowed to fault.**
  *"No membar is needed, any in-flight access to this range may fault and a lazy or delayed
  invalidate will evict the potential stale/invalid TLB entry."* — ogkm-580:
  `kernel-open/nvidia-uvm/uvm_mmu.c:1519-1527`. `[src@580]`
- **Where a live mapping must genuinely change, the driver stages through an explicit invalid
  state** rather than trusting a single-entry swap: *"We can't directly transition from a valid 2M
  PTE to valid lower PTEs, because that could cause the GPU TLBs to cache the same VA in different
  cache lines. That could cause memory ordering to not be maintained."* — ogkm-580:
  `kernel-open/nvidia-uvm/uvm_va_block.c:6646-6651`; same shape at `:6478-6483`, `:6696-6699`,
  `:7063-7066`. `[src@580]`
- **The one place quiescence is assumed is a SW contract, not a HW one:** *"All writes can be
  pipelined as put_ptes() cannot be called with any operations pending on the affected PTEs and
  PDEs."* — ogkm-580: `kernel-open/nvidia-uvm/uvm_mmu.c:1292-1293`. `[src@580]`
- **RM's remap case downgrades.** `bRemap` on a re-map drives `update_type = PTE_DOWNGRADE` —
  ogkm-580: `src/nvidia/src/kernel/gpu/mem_mgr/arch/maxwell/virt_mem_allocator_gm107.c:2602-2606`,
  and downgrade is what pulls in the sysmembar/ACK bits (§3.5). `[src@580]`

**Per path, then:**

| path | who writes the entries | fence | commit signal | live-work strategy |
|---|---|---|---|---|
| **RM / kernel VASes (incl. the CUDA context's RM-managed VAS)** | CPU-RM, through BAR2 | `portAtomicMemoryFenceFull` + read-to-flush | **BAR0 `0xB830B0` per-PDB register invalidate** | new VA only; remap ⇒ `PTE_DOWNGRADE` + sysmembar |
| **UVM (managed / fault-driven)** | CE, on `UVM_CHANNEL_TYPE_MEMOPS` | in-push `WFI + membar` | `MEM_OP` `MMU_TLB_INVALIDATE` **inside the same push** | replayable fault → service → replay; invalid-first staging |
| **CE-utility (RM's `memmgrMemWrite` with a CE transfer type)** | CE, via `ceutilsMemcopy` | release semaphore is *waited on* | still the subsequent flush + invalidate | caller-serialised |

---

## 5. The CE-written case — is the release semaphore the commit point?

**For the RM/CE transfer path: effectively yes, and the driver does treat it that way.**
`memmgrMemWrite` → `TRANSFER_TYPE_CE` → `ceutilsMemcopy`, which (unless `_FLAGS_ASYNC`) blocks in
`channelWaitForFinishPayload()` polling `READ_CHANNEL_PAYLOAD_SEMA` before returning — ogkm-580:
`src/nvidia/src/kernel/gpu/mem_mgr/mem_utils.c:620-628`, `:806-814`;
`src/nvidia/src/kernel/gpu/mem_mgr/ce_utils.c:800-814`;
`src/nvidia/src/kernel/gpu/mem_mgr/channel_utils.c:344-383`. RM does not proceed to its flush and
invalidate until that semaphore has advanced. `[src@580]`

**For UVM: no — the semaphore is *later* than the driver's own commit point.** UVM emits the TLB
invalidate as a method **inside the same push**, before `push_end`'s semaphore release, and says so:

```c
    // We just did the appropriate membar after the WFI, so no need for another
    // one in push_end().
```
ogkm-580: `kernel-open/nvidia-uvm/uvm_mmu.c:800-816`. `[src@580]`

So the C artefact's *"latched and decoded at the CE release semaphore"* is a **safe
over-approximation** for the UVM/CE case — it latches strictly after the driver considers the
mapping published — but it is our attribution, not theirs. If we ever need to be tighter, the
driver's own point is the `MEM_OP_D.OPERATION == MMU_TLB_INVALIDATE` method, which sits in the same
pushbuffer a few methods earlier.

---

## 6. Is walk-on-miss safe?

**Split the question, because the answer splits.**

### 6.1 Walk *on a fault* — safe, under a stated discipline

If the walk is triggered because the GPU asked for a translation of VA *v*, then the guest has
already let work touch *v*. For the guest to have done that legitimately, it must already have run
the whole of §3 for that VA: entries written, flushed, and the per-PDB invalidate triggered and
polled to completion. A walker running strictly *after* a demand for *v* therefore cannot land in
the middle of §2's publication window for *v*.

Under that trigger condition, a walker that obeys the rules in §7 can only ever produce:

- the correct physical page, or
- a **fault** (it hit an invalid/sparse entry), or
- a **stale-but-valid** mapping (the guest downgraded but we walked before the downgrade landed) —
  which is a *correctness* exposure identical to a stale TLB entry on real hardware, and which the
  guest's own `PTE_DOWNGRADE` + `SYS_MEMBAR` handshake (§3.5) exists to bound.

It cannot produce a *wrong* physical page, because §2.3's invariant means every reachable entry it
reads was written by the guest for that slot.

### 6.2 Walk *ahead* — unsafe, and here is the exact interleaving

A walk performed at a moment of **our** choosing rather than the GPU's — a prefetch, a background
scan, a "resolve it now while we have the lock" — has none of that protection. Three concrete
breaks, in decreasing likelihood:

1. **Torn entry (§1).** The 8-byte entry arrives as two 4-byte writes, low half first, and the
   address field spans the boundary (`ADDRESS_VID` = `32:8`, `ADDRESS_SYS` = `53:8`, ogkm-580:
   `src/common/inc/swref/published/pascal/gp100/dev_mmu.h:119`, `:140`). Reading between the two
   halves yields `VALID=1` with a truncated address ⇒ **wrong physical page**. `[meas]` for the
   split-write, `[src@580]` for the field layout.
2. **Uninitialised reachable level.** Inside a `mmuWalkReserveEntries(bInvalidate = NV_FALSE)`
   window (§2.1) the sub-table backing store is reachable and has never been written ⇒ the walker
   reads allocator residue as PTEs ⇒ **wrong physical page, and a cross-context read**. `[src@580]`
3. **Freed sub-level.** `_mmuWalkPdeRelease` frees the child's backing store after clearing the
   parent PDE but **before** any TLB invalidate (§2.1). A walker that read the parent PDE before
   the clear and the child after the free reads recycled memory. `[src@580]`

Only (1) is a race we could plausibly lose often; (2) and (3) are narrow. But all three yield the
same class of outcome — a *plausible-looking wrong physical address*, which is precisely §6's
security argument, and it survives.

### 6.3 The bottom line for the architecture question

- **Walk-on-miss is safe *if and only if* "miss" means "the GPU faulted on this VA".** That is a
  meaningfully weaker requirement than §6's current ruling, and it is compatible with 100 %
  framebuffer passthrough with no page-table-write trapping — because the guest's own commit signal
  (§0) is available on BAR0 to tell us *when* a PDB's mappings became publishable.
- **Walk-ahead is not safe**, and no ordering rule in this driver makes it safe.
- Which means the practical choice is not "witness vs. walk". It is **"witness the CE/BAR2 writes"
  vs. "trap the per-PDB invalidate at `0xB830B0` and walk lazily on fault"** — and the second was
  never on the table because §5 recorded the invalidate as absent.

---

## 7. If walk-on-miss is adopted: the walker's spec

A walker satisfying all of the following is safe against §2.3's invariant, given the §6.1 trigger
condition:

1. **Trigger only on a real translation demand.** Never prefetch, never scan, never resolve
   speculatively. The fault *is* the permission to walk.
2. **Read each entry once, whole, at its natural width** — 8 bytes for PDE3/2/1 and PTE, 16 bytes
   for PDE0 — and never re-read it mid-walk. Two reads of the same entry can straddle a write.
3. **Descend only on an explicitly-valid entry.** For V2: `APERTURE != INVALID` on a PDE
   (`NV_MMU_VER2_PDE_APERTURE`, bits `2:1`), `VALID = 1` on a PTE (bit `0`). Treat all-zero as
   invalid — that is exactly what `MMU_WALK_FILL_INVALID` writes (ogkm-580:
   `src/nvidia/src/kernel/gpu/mmu/gmmu_walk.c:901-903`, `portMemSet(pEntries, 0, …)`).
4. **Treat sparse as "no binding, but do not fault the guest"** — sparse is a distinct fill state
   (`MMU_WALK_FILL_SPARSE`) with its own templates (ogkm-580:
   `src/nvidia/src/kernel/gpu/mmu/gmmu_walk.c:904-935`); conflating it with valid or with invalid
   are different bugs.
5. **Reject an entry whose address field decodes above the guest's FB/GPA limit** — this is the
   cheap detector for the §6.2(1) torn read, since a truncated high half moves the address *down*
   and a stale high half moves it *out of range*.
6. **Never cache the walk.** The result is valid for this fault only. Anything longer-lived must be
   invalidated by the guest's own `0xB830B0` write for that PDB.
7. **Serialise against the observed invalidate.** A walk in flight when the guest triggers an
   invalidate for that PDB must be discarded, not completed — this is the only defence against
   §6.2(3).
8. **Do not honour `PDE_IS_PTE`/hybrid entries by guessing**; RM has real hybrid PDE-PTE entries
   (ogkm-580: `src/nvidia/src/libraries/mmu/mmu_walk_map.c:128-160`) and Ampere additionally makes
   PD1 a 512 MB leaf (ogkm-580:
   `src/nvidia/src/kernel/gpu/mmu/arch/ampere/kern_gmmu_fmt_ga10x.c:46-53`). A walker that assumes
   "PDE levels are never leaves" is wrong on GA10x specifically.

---

## 8. ★ The `mode2_address_table.md` §5 / §6 inconsistency, recorded

**This section exists so the next reader does not inherit a refuted premise. It does not change
§6's ruling — that is the owner's call.**

- **§5's ★ CORRECTION (2026-07-22, audit S3) enumerates two invalidate transports and finds both at
  zero.** That measurement is not in dispute here.
- **The enumeration is incomplete.** A third transport exists — the BAR0 PRI register write at
  `NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE` — and ogkm-580:
  `src/nvidia/src/kernel/gpu/mmu/arch/turing/kern_gmmu_tu102.c:117` shows it is the *default* on
  Turing+, with the RPC arm reserved for trapped-VF vGPU guests (ogkm-580:
  `src/nvidia/src/kernel/gpu/mmu/arch/maxwell/kern_gmmu_gm107.c:152-165`).
- **§5's sentence *"In GSP mode the privileged MMU register is owned by GSP, so the invalidate is
  RPC'd"* is contradicted by that code** for the bare-metal-presenting GSP-client configuration we
  emulate.
- **The committed captures agree with the code, not with the doc.** 308 register invalidates in
  `cap3_matmul_forwarding`, of which 131 target non-BAR (GPU) address spaces and 81 target PDBs
  that exist only in that capture. `[meas]`
- **Therefore §5's inference — "on that path the guest commits nothing" — is false as stated**, and
  §6, which reasons *"a lookup that finds no binding means the guest never committed (invalidated)
  that VA"*, is built on it.
- **What survives in §6 regardless of the above:** the *torn multi-level walk → wrong physical page*
  security argument. §6.2 above reconstructs it from the register field layout and the observed
  split writes, and it holds — for **walk-ahead**. §6.1 argues it does *not* hold for
  **walk-on-fault**. That distinction is the decision the owner now has evidence to make.
- **What survives in §5 regardless:** the two-co-equal-populate-sources model, and the CE-write
  capture feed. Nothing here says the CE feed is unnecessary; it says the invalidate is not absent.

---

## 9. What I could not determine

Stated plainly, because the owner is making an architecture decision on this. None of the
following was settled, and no plausible mechanism has been substituted for any of them.

1. **Why every 8-byte entry is written to BAR2 twice.** In both `cap1_coldboot_hermetic` and
   `cap3_matmul_forwarding` the pattern is `lo, hi, lo, hi` with identical values — the same entry
   written twice in a row. `[meas]` The leading hypothesis is a second pass through
   `_gmmuWalkCBUpdatePde` from `mmuWalkCommitPDEs` (ogkm-580:
   `src/nvidia/src/libraries/mmu/mmu_walk_commit.c:34-68`, which sets `bCommit` and re-invokes
   `UpdatePde` at `src/nvidia/src/libraries/mmu/mmu_walk.c:1396-1406`), but I did not confirm it,
   and it could equally be a mirrored page directory or a shadow-buffer flush. It matters only if a
   witness counts writes.
2. **Whether the compute VAS's leaf PTEs in `cap3_matmul_forwarding` were written by CPU-RM through
   BAR2 or by GSP-side CE.** I established that a GSP client **splits** the VA space — a reserved
   region is server-RM-owned (ogkm-580: `src/nvidia/src/kernel/mem_mgr/gpu_vaspace.c:395-430`,
   `:960-968`) — and that on the GSP platform `memmgrGetMemTransferType` cannot select the
   processor path (ogkm-580: `src/nvidia/src/kernel/gpu/mem_mgr/mem_utils.c:71-79`, the
   `!RMCFG_FEATURE_PLATFORM_GSP` conjunct), which is consistent with the C artefact's CE-write
   observation. But I did not establish **which side owns the CUDA context's mapping range**, and
   that is the question that decides whether the BAR0 invalidate covers the whole compute working
   set or only part of it. **This is the single most important open item here.**
3. **Whether UVM issues any `MEM_OP` TLB invalidate at all in `cap3_matmul_forwarding`.** §5
   recorded that transport at zero; I did not re-measure it and I do not know whether the C's
   pushbuffer scanner covered UVM's own channels. Method offsets for a re-measure:
   `NVC56F_MEM_OP_A` = `0x28`, `_B` = `0x2c`, `_C` = `0x30`, `_D` = `0x34`, with
   `MEM_OP_D.OPERATION == MMU_TLB_INVALIDATE` (`0x9`) — ogkm-580:
   `kernel-open/nvidia-uvm/clc56f.h:119-126`.
4. **Whether a real GA10x MMU can observe a torn entry**, as opposed to the *host observer* plane
   where the split writes were recorded. The two 4-byte BAR2 writes are ordered through the GPU's
   L2 before the MMU can read them; whether the MMU can ever sample between them is a hardware
   property I have no way to establish from source, and no hardware run was available to me. §6.2's
   argument is therefore stated about **our** walker, which definitely can, and not about silicon.
5. **What the `PTE_DOWNGRADE` invalidate looks like on the wire.** None appears in any capture —
   all 308 in `cap3_matmul_forwarding` are upgrades. `[meas]` Its predicted encoding from source is
   `SYS_MEMBAR=1, ACK=GLOBALLY` (ogkm-580:
   `src/nvidia/src/kernel/gpu/mmu/arch/turing/kern_gmmu_tu102.c:166-179`), i.e. `0x800100c1` /
   `0x800100c5`, but that is unverified and the unmap path may simply have run after the capture
   ended.
6. **Whether `mmuWalkReserveEntries(bInvalidate = NV_FALSE)` (§2.1's hole) is ever reached on the
   compute path.** I found the flag and its callers' contract but did not trace the call graph to a
   CUDA-driven caller.
7. **The BAR2-offset → PDB attribution.** The trace shows the entry writes at BAR2 offsets and the
   invalidate carrying a PDB, but I did not build the map from one to the other, so I cannot state
   that the writes immediately preceding a given invalidate belong to the PDB it names. The
   adjacency in §0 is suggestive and nothing more.

---

## 10. Provenance

- Source read: `ogkm-580.159.04`, i.e. NVIDIA open kernel modules 580.159.04, which is the bench's
  tag (per `ogkm_is_versioned` — the vendored 610.43.02 tree disagrees on the GSP queue and was not
  used here).
- Captures scanned 2026-07-31: `traces/mode2_c_reference/cap1_coldboot_hermetic` (359 062 records),
  `cap2b_stalequeue_nofn47` (862 940), `cap3_matmul_forwarding` (532 824). All three parsed dense
  and complete against `scripts/mode2_diag/rec_dump.py`'s record format; record counts matched their
  headers exactly.
- No hardware run was available for this note. Everything marked `[src@580]` is a reading;
  everything marked `[meas]` is a scan of a committed recording of a real driver, not a live run.
