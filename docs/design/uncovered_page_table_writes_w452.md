# The page-table writes our three signals do NOT cover

**STATUS: LIVE, 2026-09-11.** Source audit of `ogkm-580.159.04`, not a census.

> **Owner:** *"at this point instrumentation is to confirm, not to reason from. I would start a
> subagent that checks if all our 3 paths are sufficient to cover any PTE/PDB update, except
> the defer one which we already know is for batching updates to flush at the end."*

Answer: **they are not sufficient.** The uncovered set is below.

## ★★★ The one-bit test that generates the RECEIVED set mechanically

Routing to us is generic, not per-control:

    resource.c:264-266   IS_FW_CLIENT(pGpu) && (ctrlFlags & RMCTRL_FLAGS_ROUTE_TO_PHYSICAL)
                            -> NV_RM_RPC_CONTROL -> rpc.h:230 -> GSP_RM_CONTROL = fn 76
    control.h:233        RMCTRL_FLAGS_ROUTE_TO_PHYSICAL = 0x40

⇒ **The complete set of controls we can ever receive is exactly those whose NVOC export flags
have bit `0x40` set.** `GPU_PROMOTE_CTX` has it (`0x10244`, `g_subdevice_nvoc.c:316-318`);
**every uncovered control below lacks it** (`0x18000`, `0x8`). Grep `generated/g_*_nvoc.c` for
that bit and the received set falls out — no enumeration to go stale.

⊘ Transport correction: controls arrive as **fn 76** and allocs as **fn 103**
(`rpc_global_enums.h:86, :113`), NOT the legacy vGPU function numbers. The one exception is
`UPDATE_BAR_PDE`, which really is **fn 70** direct (`rpc.c:9703-9711`, explicitly
`if (IS_GSP_CLIENT(pGpu))`).

## The uncovered set — RM, client VA spaces

| # | write | reached by | emitted after |
|---|---|---|---|
| **R1** ★★★ | `gpu_vaspace.c:1665` `mmuWalkReserveEntries` | `RmAllocMemory(NV50_MEMORY_VIRTUAL)` → `gvaspaceAlloc_IMPL:1640` | **nothing** — the sibling sparse branch invalidates at `:1608`, this one does not, and it is not RPC'd under split VAS |
| **R2** ★★ | `gpu_vaspace.c:5025/:5035` | `VASPACE_RESERVE_ENTRIES` (0x90f10103) | **nothing**; flags `0x18000`, no 0x40. ⊘ UNDETERMINED in use on Linux |
| **R3** ★★ | `gpu_vaspace.c:5125/:5131` | `VASPACE_RELEASE_ENTRIES` (0x90f10104) | **nothing** — a DOWNGRADE with no invalidate; PDE memory is freed for reuse while our host mappings stand |
| **R4** ★★★ | `gpu_vaspace.c:900` `mmuWalkReleaseEntries` | `RmFree(hVASpace)` → `gvaspaceDestruct_IMPL:962` | **nothing** — destruct contains no invalidate at all |
| **R5** ★★ | `gpu_vaspace.c:3750` `mmuWalkMap` | `DMA_UPDATE_PDE_2` (0x80180f) | only if `_FLUSH_PDE_CACHE_TRUE`, and the default is `0` ⇒ silent PDE write |
| **R6** ★★ | `gpu_vaspace.c:2145/:2183` | root PDB pin/unpin — BAR1, the **CeUtils scrubber**, HWPM | **nothing** |
| **R7** ★★ | `gpu_vaspace.c:4679` | ★ `DMA_GET_PDE_INFO` (0x801809) — **a READ control that materialises page tables** | **nothing** |
| **R8** ★★★ | `gpu_vaspace.c:3237/:3361/:3481` migrate | `SET/UNSET_PAGE_DIRECTORY`, `SET_VA_SPACE_SIZE` | **the invalidate fires BEFORE the write and never after** (`:3221/:3342/:3479`), and the HW-commit callback is a no-op on a GSP client (`gmmu_walk.c:665-669`) |
| **R9** ★★ | — | `gvaspaceInvalidateTlb_IMPL:2329-2332` | a PTE write on a VAS whose walker root is not materialised emits **zero MMIO** — no `else`, no assert |
| **R10** ★★ | any page table in **sysmem** | `memmgrGetMemTransferType` → `TRANSFER_TYPE_PROCESSOR` | **nothing at any level** — plain guest-RAM stores, not even a BAR write |
| ~~**R12**~~ ⊘ | `gpu_vaspace.c:258` `mmuWalkSparsify` | BAR1 VA-space construct | **nothing** — and **nothing is needed**: see *Why we do not want sparse ranges* below |

## The uncovered set — BAR2 / instance memory, and UVM

- **B1–B4**: the whole BAR2 bootstrap writes PDEs/PTEs through the BAR0 window with no
  invalidate; only PDE3[0] later leaves as `UPDATE_BAR_PDE` (fn 70).
- **V1–V6**: UVM never writes `NV_PFB_PRI_MMU_INVALIDATE` at all — zero occurrences under
  `kernel-open/nvidia-uvm/`. Its root PDB is CE-written before anything names it (**V1**); a
  newly allocated table is host-shadow only and never initialised in a release build (**V2**);
  and 16 sites deliberately pass `tlb_batch == NULL` (**V3**) ⇒ **an invalidate naming a 2M/big
  level implies the 4k PTEs beneath it may also have changed.**

## ⊘ Two questions I asked and their answers

**R12 is one-time per state-load cycle**, not recurring: `kbusStatePostLoad_GM107:702` →
`kbusInitBar1_GM107:884`, idempotent at `:907`, torn down at `kbusStatePreUnload`. ⇒ bring-up
handling suffices **provided we re-arm on every state-load cycle, not only the first boot**
(suspend/resume and GPU reset re-run it). Runtime BAR1 reservations DO invalidate (`:1608`).

**R14/B4 is not a breakout hazard and needs no action against the guest.** `bInvalidate=FALSE`
is passed at exactly four sites, all BAR2, two of them dead on our guest; the live two are
closed by straight-line code 2–4 lines later with no lock drop, no RPC and no yield, before any
guest channel exists.

⚠ **But it IS a trap for us, in the opposite direction from how it reads.** Those uninitialised
PTE bytes arrive as PRAMIN writes we trap. A sweep triggered on the raw page-table byte write
would decode FB residue as PTEs and **make mappings real on the host that the guest never
authored** — manufacturing an illegal mapping out of a benign guest transient.

⇒ **Publish on the invalidate or the doorbell. NEVER on the raw page-table byte write.**

## Named but inert on GA106 — do not chase

`DMA_FILL_PTE_MEM` (0x801802) and `DMA_SET_PTE_INFO` (0x80180a) are **structurally dead**:
neither is exported in `g_device_nvoc.c`, and `DMA_UPDATE_VASPACE_FLAGS_FILL_PTE_MEM` has no
writer anywhere, so `bFillPteMem` is always FALSE. ⇒ the project's measured
`DMA_FILL_PTE_MEM = 0` was right, and now for a known reason rather than an assumed one.

## Why the campaign's two measured zeros were both correct and both useless

- **`INVALIDATE_TLB` fn=200 = 0** — it *cannot* fire: its only caller is gated on `bDoVgpuRpc`,
  which needs `IS_VIRTUAL`, false for us by construction.
- **`MEM_OP` method = 0** — that is UVM's transport, not RM's; RM never reaches
  `TRANSFER_TYPE_CE` for page tables.

⇒ **The census had no row for the transport RM actually uses**: the raw BAR0 store at
`0x00B830B0`. Second firing of *a census over transports is only as complete as its list*.

## Two operational facts for signal 1

1. ★★★ **We must clear bit 31 (`_TRIGGER`) of `0x00B830B0` or the guest hangs.**
   `kgmmuCheckPendingInvalidates_HAL` spins on it with only a timeout check in the loop
   (`kern_gmmu_tu102.c:59-83`). A model that latches the write and never retires it wedges
   CPU-RM inside the invalidate.
2. ★★ **`PTE_UPGRADE` and `PTE_DOWNGRADE` are byte-identical at the register on GA106** — the
   only differentiator is a Turing-only membar WAR (`g_kern_gmmu_nvoc.c:578-585`). ⇒ do not read
   map-vs-unmap direction out of the trapped value. What IS readable: `_ALL_PDB=FALSE` means
   `0x00B830A0/A4` name the VAS; `_HUBTLB_ONLY=TRUE` means a BAR VAS.


---

# ⊘⊘ CORRECTION — *"why do you want sparse ranges?"* (owner). **We do not.**

I listed R12 as an uncovered gap. That was the wrong classification, and the owner's question is
what exposed it.

**What sparse IS.** A sparse PTE is *valid but unbacked*: a GPU access to it returns zeros and
is **dropped instead of faulting**. It exists to make a stray access to a declared-but-unmapped
range benign **on the guest's own GPU**. It is a statement about FAULT BEHAVIOUR, not about
memory.

**Why that means nothing to us.** Our job is to make the guest's real mappings reachable on the
host. A sparse entry maps **nothing**, so there is nothing to make real — it is precisely the
row we would skip. ⇒ A bulk sparsify at construction declares *"this whole range is unbacked"*
over a range that was never backed. **There is no work, so there is no gap.**

**Where sparse DOES matter, and it is the other direction.** `gvaspaceUnmap_IMPL:2279-2285`
sparsifies *instead of* unmapping when the range was originally sparse, or whenever the VAS is
BAR1 — *"Return back to Sparse if that was the original state of this allocation."* That is a
**REVOCATION**: a range we may have backed on the host is being taken away, and a stale host
mapping that outlives it is a real defect. ⊘ Covered — its caller (`dmaFreeMapping`) supplies
the invalidate.

⇒ **The rule, and it generalises past this one entry:** what we must observe is not *"a
page-table entry changed"* but *"real backing appeared or disappeared"*. Sparsify-at-construct
is neither. Sparsify-at-unmap is a disappearance, and it is signalled.

⚠ This is the second time in this audit that reading a write as *"a page-table change we
missed"* was the wrong frame. The first was B4, where the residue in a freshly parented page
table looked like a hazard **to us** and is in fact a hazard **from us** — a sweep that decodes
raw bytes would invent mappings the guest never authored. Both errors share a shape: **treating
a byte-level write as the event, when the event is a change in what is REACHABLE.**
