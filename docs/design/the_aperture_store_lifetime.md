# The aperture store: when to punch, how big it gets, and what exhaustion does

**STATUS: LIVE (2026-09-14, w719b).** Read from `research_clones/ogkm` at **`610.43.02`**
(`version.mk:1`) — ⚠ **not** the 580 series most of this tree's citations were taken at.

Answers the four questions the owner raised on 2026-09-14 about the sparse-memfd aperture store.

## 1 — When is a page table observably dead? (the punch trigger)

★ **There is a clean observable event, and it differs by walker.**

**RM's GMMU walker** — the parent PDE write **strictly precedes** the free, in `_mmuWalkPdeRelease`:

1. the parent PDE is overwritten with `MMU_WALK_FILL_SPARSE`/`FILL_INVALID` — `mmu_walk.c:1514-1521`
   (`FILL_INVALID` → `portMemSet(pEntries, 0, sizeOfEntries)`, `gmmu_walk.c:861-863`)
2. **then** `// Free up the actual sublevels from the PDE` → `LevelFree` — `mmu_walk.c:1542-1552`

⊘ **`LevelFree` writes nothing to the page.** It returns a packed sub-memdesc to a per-VAS free
list (`gmmu_walk.c:578`) or calls `rmMemPoolFree` (`:586-588`). No ioctl, no RPC, no register
touch. ⇒ **The parent PDE write is the ONLY externally visible artefact of a table dying.**

**UVM's walker** gives a strictly better event — `uvm_page_tree_put_ptes_async`:
`pde_clear` (`uvm_mmu.c:1314`) → `wfi_membar` (`:1348`) → **`tlb_invalidate_all` as a pushbuffer
method** (`:1350-1353`) → *then* `phys_mem_deallocate` (`:1369`).

⇒ **The punch trigger is: a parent PDE going invalid, actioned at the next synchronisation point.**
Never an inference from disuse.

⚠ **And the recycle side is the hazard.** RM reuses PT pages from its caches, and **non-root
levels are NOT scrubbed at allocation** — `if (pLevelFmt == pFmt->pRoot) status = _gmmuScrubMemDesc(...)`
(`gmmu_walk.c:341-343`). Clearing is done by `FillEntries` gated on `bInvalidateOnReserve`
(`mmu_walk.c:1233-1239`). ⇒ A punched-and-reused page is fine *because* the guest re-initialises
it, but only through that path.

## 2 — ⊘⊘ THE ~40 MiB BOUND IS REFUTED, and BAR2 is smaller than assumed

> Owner's hypothesis: *"all tables that are needed by the driver are mapped in bar2, since bar2 is
> 32MB, I don't think it makes sense to have more than 40MB of allocations in fake FB."*

**Two independent reasons it does not hold.**

**(a) The window is 16 MiB on a GA106, not 32.** `BUS_BAR2_APERTURE_MB 32` /
`BUS_BAR2_RM_APERTURE_MB 16` (`g_kern_bus_nvoc.h:185-186`); the RM-addressable limit takes the
full 32 only when `bIsEntireBar2RegionVirtuallyAddressible`, which is **TRUE only for
GH100/GB1xx/GB2xx/GR1xx** (`g_kern_bus_nvoc.c:260-268`) ⇒ **FALSE on GA106**. The upper half is
VESA space (`kern_bus_gm107.c:4337-4340`).

**(b) ★★★ BAR2 is an EVICTING LRU CACHE, so residency is not liveness.**
`kbusMapBar2ApertureCached_VBAR2` (`kern_bus_vbar2.c:529`): on `eheapAlloc` failure it evicts one
big-enough cached mapping (`:623-639`), else **evicts them all** (`:645-659`), retries, and only
then refuses (`:661-672`). An eviction tears down the *mapping*, not the table.

⇒ **A page table can be live and have no BAR2 mapping at all.** The 16 MiB bounds the
*simultaneously mapped* set, never the total.

★ **How big can it get?** GP10x/Ampere v2 small-PT level is 512 entries × 8 B
(`kern_gmmu_fmt_gp10x.c:92-95`) ⇒ **PT bytes = VA bytes / 512**. A 12 GiB space fully mapped at
4 KiB is **24 MiB of small page tables alone**, before PD0–PD3 and before big-page tables — already
past the window, and bounded only by how much VA the guest maps.

⇒ **The aperture store must be sized by policy and enforced, not assumed small.**

## 3 — PRAMIN: the prior ruling is confirmed, and its summary was misleading

`TRANSFER_TYPE_BAR0` is returned from exactly one place, under
`IS_SIMULATION(pGpu) && pSrc != NULL && !KBUS_BAR0_PRAMIN_DISABLED(pGpu)` (`mem_utils.c:82-91`) —
**confirmed verbatim**. ⊘ But `IS_SIMULATION` does almost no work: the real PRAMIN traffic is
gated on **`bBootstrap`**, and **a bare-metal driver takes that path**, because the alternative
`bUsePhysicalBar2InitPagetable` is set **only** under `IS_VIRTUAL_WITH_SRIOV` (`kern_bus.c:57-60`).

⇒ **A stock driver writes BAR2's own page tables through PRAMIN** (`bar2_walk.c:868-910`;
window re-pointed at `bar2[PF].pdeBase & ~0xffff`, `kern_bus_gm107.c:2138-2152`), and writes
instance blocks through it before BAR2 exists (`kern_bus_gp100.c:1023`, `:1055`, `:1085`).

★ **Yes, PRAMIN reaches FB that BAR2 structurally cannot** — it is a re-pointable window over the
whole FB, and the bootstrap is the proof by construction: BAR2's page tables are written through
it *before any BAR2 mapping exists*. ⊘ Every PRAMIN user found is gated on *"BAR2 is not up yet"*,
on `IS_SIMULATION`, or on Hopper-only code; **no steady-state PRAMIN path was found** — a grep
negative, so weaker than a positive.

## 4 — Quota exhaustion: mostly graceful, with ONE silent path

**Graceful:** `LevelAlloc` failure unwinds through `_mmuWalkLevelInstAcquire`
(`mmu_walk.c:1187`, `:1254-1258`), `_mmuWalkPdeAcquire` (`:1372-1377`), `mmuWalkProcessPdes`'
`cleanupIter` (`:549-630`), `gvaspaceMap_IMPL` (`gpu_vaspace.c:2033-2039`) and
`dmaUpdateVASpace_GF100` (`virt_mem_allocator_gm107.c:2554-2588`) to an ioctl status.
⊘ Note an FB shortage often becomes *"page tables in sysmem"* rather than an error
(`gmmu_walk.c:159-161`, `:253-259`).

⊘⊘⊘ **The silent one.** `_gmmuWalkCBFillEntries` returns **`void`** (`gmmu_walk.c:781`); its only
failure signal is leaving `*pProgress` unwritten after
`NV_ASSERT_OR_RETURN_VOID(pEntries != NULL)` (`:840-842`), where `pEntries` is NULL exactly when
`kbusMapBar2Aperture` fails — i.e. **BAR2 exhaustion** (`kern_bus_vbar2.c:668-671`). At the
page-table-growth site the check is `NV_ASSERT(progress == …)` (`mmu_walk.c:1233-1240`), and
`NV_ASSERT` expands to log + `PORT_BREAKPOINT_CHECKED()`, **empty in a non-checked build**
(`nvport/debug.h:179`). It alters no control flow.

⇒ **A page table can be published into its parent PDE having never been initialised**, and per §1
non-root levels are not scrubbed. The GMMU then walks whatever the recycled page contained.

⚠ **Code-reading only, not reproduced**, and likely rare (eviction-then-retry makes the NULL
return unlikely for a ≤4 KiB request against a 16 MiB heap). Read it as *"this failure mode exists
and is silent"*, never as *"this will happen"*.

★ **For us the actionable half is the mirror image:** if our aperture store ever refuses a page,
the guest's driver may not report it. ⇒ **We must refuse loudly on our side** — a named refusal —
because the guest's own error path cannot be relied on to carry it.
