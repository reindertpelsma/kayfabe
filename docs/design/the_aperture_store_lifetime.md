# The aperture store: when to punch, how big it gets, and what exhaustion does

**STATUS: LIVE (2026-11-14: every `ogkm-610:` citation below re-checked against `third_party/ogkm`
@ `57130a2` = 610.43.02; two citations and one claim corrected, see the 2026-11-14 notes; written
2026-09-14, w719b).** Read from the `ogkm` clone at **`610.43.02`**
(`version.mk:1`) — ⚠ **not** the 580 series most of this tree's citations were taken at. In this
document `ogkm-610: file.c:N` means `third_party/ogkm` at that revision, found by file name (the
submodule is not checked out by default, `third_party/README.md`). The 580 line numbers differ and
were not re-checked.

**2026-11-14 re-verification.** Checked in the 610.43.02 source: every cited file exists; every
cited line range shows the construct the text names, except the three corrections below.
(1) The GP10x small page table (512 entries, `virtAddrBitHi 20` / `Lo 12`, PTE-sized entries) is at
`ogkm-610: kern_gmmu_fmt_gp10x.c:99-104`; the old `:92-95` is the *big* page table. (2) The
`UVM_CHANNEL_TYPE_MEMOPS` declaration and its comment are at `ogkm-610: uvm_channel.h:87-88`, not
`88-89`. (3) See the correction at the top of §5: UVM does not select MEMOPS for page-tree
writes in general. The scenario in §3-§4 (PRAMIN, BAR2 eviction, silent `void` fill) is *code
reading only*, unchanged: not reproduced, and not re-run on hardware.

Answers the four questions the owner raised on 2026-09-14 about the sparse-memfd aperture store.

## 1 — When is a page table observably dead? (the punch trigger)

★ **There is a clean observable event, and it differs by walker.**

**RM's GMMU walker** — the parent PDE write **strictly precedes** the free, in `_mmuWalkPdeRelease`:

1. the parent PDE is overwritten with `MMU_WALK_FILL_SPARSE`/`FILL_INVALID` — `ogkm-610: mmu_walk.c:1514-1521`
   (`FILL_INVALID` → `portMemSet(pEntries, 0, sizeOfEntries)`, `ogkm-610: gmmu_walk.c:861-863`)
2. **then** `// Free up the actual sublevels from the PDE` → `LevelFree` — `ogkm-610: mmu_walk.c:1542-1552`

⊘ **`LevelFree` writes nothing to the page.** It returns a packed sub-memdesc to a per-VAS free
list (`ogkm-610: gmmu_walk.c:578`) or calls `rmMemPoolFree` (`:586-588`). No ioctl, no RPC, no register
touch. ⇒ **The parent PDE write is the ONLY externally visible artefact of a table dying.**

**UVM's walker** gives a strictly better event — `uvm_page_tree_put_ptes_async`:
`pde_clear` (`ogkm-610: uvm_mmu.c:1314`) → `wfi_membar` (`:1348`) → **`tlb_invalidate_all` as a pushbuffer
method** (`:1350-1353`) → *then* `phys_mem_deallocate` (`:1369`).

⇒ **The punch trigger is: a parent PDE going invalid, actioned at the next synchronisation point.**
Never an inference from disuse.

⚠ **And the recycle side is the hazard.** RM reuses PT pages from its caches, and **non-root
levels are NOT scrubbed at allocation** — `if (pLevelFmt == pFmt->pRoot) status = _gmmuScrubMemDesc(...)`
(`ogkm-610: gmmu_walk.c:341-343`). Clearing is done by `FillEntries` gated on `bInvalidateOnReserve`
(`ogkm-610: mmu_walk.c:1233-1239`). ⇒ A punched-and-reused page is fine *because* the guest re-initialises
it, but only through that path.

## 2 — ⊘⊘ THE ~40 MiB BOUND IS REFUTED, and BAR2 is smaller than assumed

> Owner's hypothesis: *"all tables that are needed by the driver are mapped in bar2, since bar2 is
> 32MB, I don't think it makes sense to have more than 40MB of allocations in fake FB."*

**Two independent reasons it does not hold.**

**(a) The window is 16 MiB on a GA106, not 32.** `BUS_BAR2_APERTURE_MB 32` /
`BUS_BAR2_RM_APERTURE_MB 16` (`ogkm-610: g_kern_bus_nvoc.h:185-186`); the RM-addressable limit takes the
full 32 only when `bIsEntireBar2RegionVirtuallyAddressible`, which is **TRUE only for
GH100/GB1xx/GB2xx/GR1xx** (`ogkm-610: g_kern_bus_nvoc.c:260-268`) ⇒ **FALSE on GA106**. The upper half is
VESA space (`ogkm-610: kern_bus_gm107.c:4337-4340`).

**(b) ★★★ BAR2 is an EVICTING LRU CACHE, so residency is not liveness.**
`kbusMapBar2ApertureCached_VBAR2` (`ogkm-610: kern_bus_vbar2.c:529`): on `eheapAlloc` failure it evicts one
big-enough cached mapping (`:623-639`), else **evicts them all** (`:645-659`), retries, and only
then refuses (`:661-672`). An eviction tears down the *mapping*, not the table.

⇒ **A page table can be live and have no BAR2 mapping at all.** The 16 MiB bounds the
*simultaneously mapped* set, never the total.

★ **How big can it get?** GP10x/Ampere v2 small-PT level is 512 entries × 8 B
(`ogkm-610: kern_gmmu_fmt_gp10x.c:99-104` (corrected 2026-11-14; was `:92-95`)) ⇒ **PT bytes = VA bytes / 512**. A 12 GiB space fully mapped at
4 KiB is **24 MiB of small page tables alone**, before PD0–PD3 and before big-page tables — already
past the window, and bounded only by how much VA the guest maps.

⇒ **The aperture store must be sized by policy and enforced, not assumed small.**

## 3 — PRAMIN: the prior ruling is confirmed, and its summary was misleading

`TRANSFER_TYPE_BAR0` is returned from exactly one place, under
`IS_SIMULATION(pGpu) && pSrc != NULL && !KBUS_BAR0_PRAMIN_DISABLED(pGpu)` (`ogkm-610: mem_utils.c:82-91`) —
**confirmed verbatim**. ⊘ But `IS_SIMULATION` does almost no work: the real PRAMIN traffic is
gated on **`bBootstrap`**, and **a bare-metal driver takes that path**, because the alternative
`bUsePhysicalBar2InitPagetable` is set **only** under `IS_VIRTUAL_WITH_SRIOV` (`ogkm-610: kern_bus.c:57-60`).

⇒ **A stock driver writes BAR2's own page tables through PRAMIN** (`ogkm-610: bar2_walk.c:868-910`;
window re-pointed at `bar2[PF].pdeBase & ~0xffff`, `ogkm-610: kern_bus_gm107.c:2138-2152`), and writes
instance blocks through it before BAR2 exists (`ogkm-610: kern_bus_gp100.c:1023`, `:1055`, `:1085`).

★ **Yes, PRAMIN reaches FB that BAR2 structurally cannot** — it is a re-pointable window over the
whole FB, and the bootstrap is the proof by construction: BAR2's page tables are written through
it *before any BAR2 mapping exists*. ⊘ Every PRAMIN user found is gated on *"BAR2 is not up yet"*,
on `IS_SIMULATION`, or on Hopper-only code; **no steady-state PRAMIN path was found** — a grep
negative, so weaker than a positive.

## 4 — Quota exhaustion: mostly graceful, with ONE silent path

**Graceful:** `LevelAlloc` failure unwinds through `_mmuWalkLevelInstAcquire`
(`ogkm-610: mmu_walk.c:1187`, `:1254-1258`), `_mmuWalkPdeAcquire` (`:1372-1377`), `mmuWalkProcessPdes`'
`cleanupIter` (`:549-630`), `gvaspaceMap_IMPL` (`ogkm-610: gpu_vaspace.c:2033-2039`) and
`dmaUpdateVASpace_GF100` (`ogkm-610: virt_mem_allocator_gm107.c:2554-2588`) to an ioctl status.
⊘ Note an FB shortage often becomes *"page tables in sysmem"* rather than an error
(`ogkm-610: gmmu_walk.c:159-161`, `:253-259`).

⊘⊘⊘ **The silent one.** `_gmmuWalkCBFillEntries` returns **`void`** (`ogkm-610: gmmu_walk.c:781`); its only
failure signal is leaving `*pProgress` unwritten after
`NV_ASSERT_OR_RETURN_VOID(pEntries != NULL)` (`:840-842`), where `pEntries` is NULL exactly when
`kbusMapBar2Aperture` fails — i.e. **BAR2 exhaustion** (`ogkm-610: kern_bus_vbar2.c:668-671`). At the
page-table-growth site the check is `NV_ASSERT(progress == …)` (`ogkm-610: mmu_walk.c:1233-1240`), and
`NV_ASSERT` expands to log + `PORT_BREAKPOINT_CHECKED()`, **empty in a non-checked build**
(`ogkm-610: nvport/debug.h:179`). It alters no control flow.

⇒ **A page table can be published into its parent PDE having never been initialised**, and per §1
non-root levels are not scrubbed. The GMMU then walks whatever the recycled page contained.

⚠ **Code-reading only, not reproduced**, and likely rare (eviction-then-retry makes the NULL
return unlikely for a ≤4 KiB request against a 16 MiB heap). Read it as *"this failure mode exists
and is silent"*, never as *"this will happen"*.

★ **For us the actionable half is the mirror image:** if our aperture store ever refuses a page,
the guest's driver may not report it. ⇒ **We must refuse loudly on our side** — a named refusal —
because the guest's own error path cannot be relied on to carry it.

## 5 — Is UVM's CE the same CE userspace copies use? **No** (w719e)

> Owner, 2026-09-14: *"is the kernel uvm ce also used for userspace ce copies?"*

**Separate, two independent ways.**

1. **UVM's channels are kernel-allocated.** `nvUvmInterfaceChannelAllocate`
   (`ogkm-610: uvm_channel.c:2366`) is the RM↔UVM **kernel** interface. The only channel traffic crossing
   the userspace ABI runs the **other way**: `uvm_api_register_channel` (`ogkm-610: uvm_api.h:249`) is
   userspace telling UVM about a channel *it* created so UVM can service faults on it. Nothing
   hands a UVM channel to userspace, and `UVM_CHANNEL_TYPE` does not appear in `ogkm-610: uvm_ioctl.h` at
   all.
2. ⚠ **Corrected 2026-11-14 (read in 610.43.02):** the text of this point below overstates the
   source. `page_tree_begin_acquire` (`ogkm-610: uvm_mmu.c:55-64`; the comment above it is at `:47-50`) picks `UVM_CHANNEL_TYPE_MEMOPS`
   **only** under `uvm_parent_gpu_is_virt_mode_sriov_heavy`; in every other case it uses
   `UVM_CHANNEL_TYPE_GPU_INTERNAL`, and that comment says exactly that (MEMOPS "when in
   SR-IOV heavy", GPU_INTERNAL "preferred" otherwise). So on a non-SR-IOV-heavy GPU page-tree
   writes are **not** on the MEMOPS channel type, and the conclusion "separate from userspace
   copies" still stands but rests on point 1 alone. The cite below is the macro and its comment, not an
   unconditional selection.
   Original text, kept for the record and corrected by the paragraph above: *page-tree writes have their own channel type
   inside UVM* — `UVM_CHANNEL_TYPE_MEMOPS`,
   declared as *"Memops and small memsets/copies for writing PTEs"* (`ogkm-610: uvm_channel.h:87-88`) and
   selected explicitly for page-tree manipulation (`ogkm-610: uvm_mmu.c:47-64`: *"…because it is used to
   manipulate the page tree"*).

⊘ **Do NOT build a page-table oracle on the channel type.** MEMOPS also carries fault servicing
(`ogkm-610: uvm_gpu_replayable_faults.c:363,372,449,502`, `ogkm-610: uvm_gpu_non_replayable_faults.c:299`,
`ogkm-610: uvm_ats_faults.c:742`), migration (`ogkm-610: uvm_migrate.c:722`), `ogkm-610: uvm_va_block.c` (×6) and
`ogkm-610: uvm_map_external.c:231`. *"Arrived on MEMOPS"* is **necessary, not sufficient**.

### ★★★ Why this SHRINKS the recycling hazard of constraint 19(b)

UVM's page tables are **CE-written and never CPU-read through BAR2**. ⇒ Under 19(a)'s
reserved-object default they never qualify for the aperture store **at all**: they are ordinary
engine-visible video memory from birth. Nothing to classify, nothing to migrate, nothing to
revoke.

⇒ **The recycle-changes-role hazard is confined to RM's pool** — which is precisely where the
observable event exists (the parent PDE going invalid, strictly before the free, §1). ★ The hard
case and the available signal are the same case, and it is **one allocator, not two**.
