# GPGA is ONE reserved object

**STATUS: LIVE (2026-09-10; amended in place 2026-09-21, w824).** Owner's design, settled in
conversation 2026-09-10 and **re-affirmed verbatim by the owner on 2026-09-21** (*"yes GPGA is
just one rm object"*). Supersedes the per-leaf join described in `fbwin.rs`'s module docs, which
is scheduled for deletion by it. ⚠ The w824 amendments below were folded in by fable with the
owner's authorisation (*"its ok to update parts"*); each is marked `[w824]`, each says whether
it is measured, cited, or inferred, and none deletes the 09-10 reasoning — where a 09-10
conclusion no longer holds, the supersession is recorded **above** the text it supersedes and
the text is kept for its rationale.

## `[w824]` What moved between 09-10 and 09-21 — read this first

Four owner rulings on 2026-09-21 (`THE_CONSTRAINTS.md` §56, verbatim: *"joins is dead, we
don't store va tables, we don't auto-map leaves in MMIO if the guest didn't told to, we don't
have per GPGA phys backings"*) are **consequences** this document implied and did not state.
They change the *mechanism* sections below and leave the rule, the reservation, the BAR1
finding and every measurement untouched:

| § below | 09-10 said | after §56 |
|---|---|---|
| The rule | one object, sliced by offset | **unchanged, affirmed** |
| Backing follows USE | learned *"at the moment a range is mapped into a GPU address space"* | **unchanged**, sharpened: that moment is the **guest's own map call** (rule 3), never our observation of an MMIO write |
| Page tables: promote/demote | copy tables to host RAM inside the invalidate window, dirty-track, demote on an observable event | ⊘ **SUPERSEDED by rules 2 and 3** — see the `[w824]` block above that section. What survives is the GPU-side walk **in place**; what does not is the copy, the dirty tracking, and the fake range |
| The scratchpad's address space | GPGA range + a *fake range* of promoted buffers | **GPGA range only** — the identity window. The fake range existed for promoted buffers and goes with them |
| The DoS this closes | host RAM bounded to *"page tables only, ~7 MiB"* | bounded to **our own per-mapping RM records** — no guest page is ever copied into host RAM by us |
| Sequencing | 1 reserve · 2 promote/demote · 3 delete the join | 1 reserve · **2 the identity window** · 3 delete the join and the walkers-as-storage |

### `[w824]` The two VA spaces — the owner's construction, stated here because the doc predates it

> **Owner, 2026-09-21:** *"you have 2 vas at startup atleast: the va for libcuda, the va manager
> thread with the ptx. created by libcuda, not us. the second is a va with only the GPGA mapped,
> one rm object, at the correct fb offset (so that if virtual phys is used then it works). then
> Translate does if the aperature is phys it translates to the va space and all GPGA is
> accessible."*

1. **libcuda's VA** — created by `cuCtxCreate` in the process that runs the manager thread and
   the PTX walk kernel (`cuda/walk/kf_walk.cu`). GPGA is imported into it so the walker reads the
   guest's page tables **where they are**, as `win.base + gpga` — the one flat window w731 said
   the kernel needed and could not have while the store was per-leaf
   (`the_kernel_cannot_be_pointed_at_the_tables_where_they_are`). ★ The relocation image
   (`GmmuFmt::relocate_entry`, `walkshadow::build_image`) was transitional and expires here.
2. **The GPGA-identity VA** — one `FERMI_VASPACE_A` we create, holding **one** FIXED
   `MapMemoryDma` of the whole reserved object at `GPGA_VA_BASE + fb_phys`. A guest operand
   in **physical** aperture (`NVC7B5_SET_{SRC,DST}_PHYS_MODE_TARGET_LOCAL_FB` +
   `LAUNCH_DMA_*_TYPE_PHYSICAL`, `clc7b5.h:66-83,122-126`) is served by flipping the type bit to
   `VIRTUAL` and adding `GPGA_VA_BASE`. Arithmetic; no lookup.
   ★ **NVIDIA does exactly this itself.** CeUtils' `bUseVasForCeCopy` rewrites an FB-physical
   operand to `addr + fbAliasVA - startFbOffset` and flips `_SRC_TYPE`/`_DST_TYPE` to `_VIRTUAL`
   (`ogkm-610: channel_utils.c:1053-1091`); its own log line reads *"FB (addr, size) identity
   mapped to VAS"* (`mem_utils_gm107.c:505-516`), with FB base and VA base **512 MiB-aligned**.
   The identity window is not our invention; it is the driver's own shape for this problem.
   ⚠ `[inferred, unmeasured]` `GPGA_VA_BASE` should be ≥ 512 MiB-aligned (the CeUtils rule) and
   above any VA a guest can name in that space; the tree's own reservation already tries
   1 GiB-aligned contiguous first (`rm.rs:reserve_gpga_inner`, w755c) so that VA ≡ phys at every
   page size and the map needs no small-page pin.

⚠ **Two things the two-VA construction does NOT decide, both recorded so they are not read as
settled.** (i) Which VA space a **Translated** host channel is bound to when one pushbuffer mixes
physical and virtual operands (UVM's kernel channels do: page-table writes are physical
`uvm_mmu.c:432,462`; the pushbuffer itself is sysmem, `uvm_pushbuffer.c:114-117`). Either the
identity window is mapped into every mirrored VAS above the guest's range, or such channels
run in the identity VA and their virtual operands are rewritten — the second is only possible
for operands that are *methods*, never for pointers inside kernel parameters. (ii) A
**virtual**-aperture operand needs a VA→GPGA answer and rule 2 says we keep none. §56.2 names
this as the open question; the answer this document takes is in the `[w824]` block above
"Page tables" below.

## The rule

At start, kayfabe allocates **one** device-local RM object for the whole of the guest's video
memory. If that fails, **the VM does not start**. The guest's advertised framebuffer size is
derived from the reservation that succeeded, never asserted ahead of it.

⇒ **GPGA is that object.** Every mapping — CPU or GPU, guest aperture or scratchpad or channel —
is a **slice** of it at an offset. There is no second memory for any address.

## What this deletes, and why it is deletion rather than repair

Today video memory is allocated **lazily, per 64 KiB leaf, after the guest has already written
into invented pages**. Once that is the shape a copy is unavoidable: the bytes are in the wrong
memory by the time we learn we need the right one. `install_join` is that copy, and every part of
the mechanism follows from it — the overlap refusal, the residency skip, the write-combining cost
it confesses to in its own comment, and the `not_granular` bucket that silently drops rows whose
length is under the granule.

⊘ **It is a shadow.** Two memories exist for one address across time, something must decide when
to copy, and a guest write that lands after the copy begins is lost. A correct guest races us —
not a hostile one, a correct one — and no lock of ours changes that, because the guest does not
participate in our locking.

Reserving up front removes the **cause**. There is no moment where bytes exist in the wrong
place, so there is nothing to copy, no moment to choose, and no window to lose a write in.

## Backing follows USE, not address

Below the firmware carve-out the framebuffer is an undifferentiated heap: page tables and user
buffers are **indistinguishable by address**, because RM allocates both from it. So backing
cannot be decided by where something is. It is decided by who reads it:

- A page an **engine** reads must be in the reserved object.
- A page **only we** read has no reason to be there.

We learn which is which at the moment a range is mapped into a GPU address space. That is the
same event that guarantees existence, so one rule does both jobs.

`[w824]` ★ **And rule 3 fixes WHOSE event that is: the guest's.** *"We don't auto-map leaves in
MMIO if the guest didn't told to."* The map exists because the guest asked for it — an RM map
call we serve, a channel or context RPC that carries addresses (`GPU_PROMOTE_CTX`, channel
alloc), or a synchronisation point the guest issues (`the_three_synchronization_points`:
TLB invalidate · RPC map calls · UVM set-up). Watching a page-table page get written is not a
request and does not create a mapping.

## `[w824]` ⊘ SUPERSEDED BY §56 RULES 2 AND 3 — the promote/demote protocol below does not survive; read this before it

**What the section below rests on, and which of it §56 removes:**

| premise of promote/demote (09-10) | status after §56 |
|---|---|
| we *refresh* by walking the guest's tables and *keep* what we learned (a mirror the views are re-pointed from) | ⊘ **rule 2** — we store no VA table. The guest's tables are read **in place, in GPGA, by the GPU walker**, at the guest's synchronisation point, and the answer is consumed by that one host map call, not retained |
| an unpromoted vidmem page is *"permanently dirty"*; promoted pages have dirty tracking, so edits are noticed | ⊘ **rule 3** — noticing an edit is not a mapping trigger. And the premise was **already broken for the tables that matter**: UVM writes its tables with the **copy engine** (`uvm_mmu.c:432,462`, w719b) and a DMA write into a promoted host page sets **no KVM dirty bit**. Under the identity window a CE page-table write lands in GPGA, in vidmem, where the walker reads it — the problem dissolves rather than being solved `[inferred from the mechanism; not yet measured on a live guest]` |
| promoted buffers live in a *fake range* above GPGA, and every view is re-pointed | ⊘ gone with promotion; the scratchpad VA is the identity window only. This is also what makes `GpgaViews`' *"real work"* (§"What promotion actually costs") unnecessary |
| demotion on an observable event | ⊘ nothing is promoted, nothing is demoted |

**What survives, and why the measurements below are still the reason for the design:** the
cost table (48 MiB/s CPU reads, 1872 pages, 1178 refreshes ⇒ ~10 min/boot) is exactly why the
walk is a **GPU kernel over the identity window** and never a CPU read — `the_walk_kernel`:
462 ms → **205.7 µs**; PTX validated **72/72** on hardware including hostile and racing tables
(`the_ptx_walker_is_validated_on_hardware`). The 09-10 text reached the right placement for the
wrong reason (to make promotion affordable); the placement stands on its own.

★ **What triggers a walk now, stated so it cannot be read as MMIO observation:**
1. a BAR0 `MMU_INVALIDATE` register write naming a PDB — RM's walker path; ALL_VA is hard-coded
   there (`rm_cannot_express_a_narrow_invalidate`), so the scope is *that address space, whole*;
2. a `MEM_OP_D MMU_TLB_INVALIDATE[_TARGETED]` method **in a Translated pushbuffer** — UVM's path
   (`clc56f.h:132-176`), which carries PDB, aperture, target VA and size, i.e. the guest tells us
   both the space and the extent. ⚠ It must be honoured at **execution** time, after the CE
   page-table writes that precede it in the same stream have retired on the host — `[w824b]`
   *retired* meaning their completion fd became ready in the loop, **never** a wait on the
   worker's stack (`THE_TRANSLATED_PLANE.md` §5) — the C latched
   at the release semaphore for this reason (`nvkvm_m2_cpt_sync_at_release`,
   `nvkvm_gpu_emul.c:596-604`). Honouring it at decode time reads tables the engine has not
   written yet;
3. an RPC that carries addresses explicitly — `GPU_PROMOTE_CTX` entries, channel alloc
   (instance block, USERD, GPFIFO), `SetPageDir`/`UPDATE_BAR_PDE` for roots.

⊘ **The one record we cannot avoid keeping, said plainly:** RM's `UNMAP_MEMORY_DMA` needs the
handle and offset of every mapping we made, so a **ledger of our own host map calls** per guest
space exists. `[fable's reading, not an owner ruling]` That is a list of RM handles, not a mirror
of the guest's tables, and rule 2 is read here as forbidding the latter. If the owner reads rule 2
as forbidding the ledger too, the only alternative is unmap-all/map-all at every invalidate,
which is `entries≈1800` RM calls per invalidate and must be measured before it is chosen.

## Page tables: promote in the invalidate window only

`[w824]` ⊘ **SUPERSEDED — kept for its reasoning; see the block directly above.**

★ The GPU never walks the guest's page tables. It walks the **host** tables that host RM builds
from our map calls. The guest's tables are interpretation data for us alone. Round-tripping them
through video memory is work done for nobody.

The protocol, and it is **self-tuning**:

1. A page table is born in video memory like anything else. The refresh treats **any unpromoted
   video-memory page as permanently dirty** — we cannot know otherwise, and saying so is honest.
2. During a refresh — and **only** during a refresh — a page the walk identifies as a page table
   is **promoted**: copied to a host buffer, which then takes authority, and every view of that
   address is re-pointed.
3. Promoted pages have working dirty tracking, so the next refresh does not re-read them.
4. On an **observable** event — the parent entry cleared, or the address space torn down — the
   page is **demoted**: copied back and the host buffer released.

⚠ **Demotion may not be inferred from disuse.** *"No longer used for paging"* is a claim about
the future; only *"not used yet"* is observable. This tree already records that shape for orphaned
promote halves. Hang demotion off an event we see.

### Why the window is safe

A correct guest does not edit an address space's page tables while it is invalidating them — it
writes, then issues the invalidate (`ogkm-580: uvm_mmu.c:800-809`, writes then `wfi_membar` then
`tlb_invalidate_all`). So the copy is safe **by construction** rather than by a lock.

⊘ Per **address space**, not globally: another thread may be writing a different space's tables
at the same moment. ⊘ And a guest that violates it corrupts **only itself** — there is no
breakout, which is the correct bar.

### The cost, and why promotion is not an optimisation

`[measured w422]` 1872 resident page-table pages, ~7.3 MiB, 784 sweeps on one address space,
1178 refreshes a boot. Uncached video-memory reads run in the low hundreds of MB/s.

`[measured 2026-09-11, bare metal, RTX 3060]` — the copy-width sweep over one 64 MiB
reservation, mapped once, filled with `splitmix64`:

| copy size | 64 B | 512 B | **4 KiB** | 64 KiB | 1 MiB | 16 MiB |
|---|---|---|---|---|---|---|
| MiB/s | 47.0 | 35.7 | **48.2** | 45.9 | 44.5 | 34.8 |

★ **Flat.** Across eight orders of magnitude of transfer size the answer never leaves the
mid-forties, which rules out per-call overhead and per-mapping locality. That is simply what the
bus gives for CPU reads of video memory. ⊘ Word-at-a-time reads give **13–15 MiB/s**, so bulk
copies are worth ~3.5x and that is the *entire* available improvement — not the hundredfold an
earlier draft of this file allowed for. The identical loop over ordinary memory: **3674 MiB/s**.

| | PCIe traffic per boot |
|---|---|
| re-read every refresh, at the measured 48 MiB/s | ~500 ms per full re-read × 1178 ⇒ **~10 minutes** |
| promote once per page | **~150 ms total** |

⇒ Promotion converts a per-sweep cost into a **per-page-once** cost. It is load-bearing for the
product, not a percentage gain — the difference between booting and not.

★ And the footprint is small enough that wasting the video memory behind a promoted page is
irrelevant: the whole hierarchy is **one part in five hundred** of what it maps (a 4 KiB leaf
table covers 2 MiB), so 12 GiB fully mapped at small pages costs 24 MiB of tables.

## ⚠ CPU VIEWS ARE BOUNDED BY BAR1; GPU VIEWS ARE NOT

`[measured 2026-09-11]` on the bench RTX 3060, from `lspci`:

| aperture | size |
|---|---|
| BAR0, registers | 16 MiB |
| **BAR1, the CPU window into video memory** | **256 MiB** |
| BAR2 | 32 MiB |

A **6144 MiB reservation succeeds** and a **256 MiB CPU mapping of it REFUSES with
`NoMemory`** — in the same process, the mapping failing while the far larger allocation
succeeds. The allocation is bounded by video memory; the CPU mapping is bounded by **BAR1**,
which is 256 MiB in total for every client on the card, not per client.

⇒ **A design that kept one persistent CPU view of the whole reservation would fail at boot.**
Slicing CPU views is forced, not optional, and the ceiling is BAR1 minus whatever the host
driver already holds. ★ This is also what PRAMIN is *for* — a small window that gets re-pointed
— and why the hardware has one at all.

⊘ **GPU virtual mappings are NOT affected.** They consume page tables, not aperture. So:

- *"the scratchpad maps the whole object"* — **holds**, it is a GPU VA mapping.
- *"the VMM keeps a CPU view of the whole object"* — **does not hold**, and never could.

★ It also strengthens promotion a third time: reading page tables from video memory needs both
a scarce aperture window **and** a 13 MiB/s bus. Promoting them into host memory removes both,
for single-digit megabytes.

## `[w824]` The pushbuffer read — not covered on 09-10

A Translated channel's entries must be read before hardware sees them
(`the_three_channel_kinds.md` §1). Where they are decides how:

| pushbuffer lives in | who puts it there | how we read it |
|---|---|---|
| **guest RAM (sysmem)** | UVM by default (`uvm_pushbuffer.c:98-117`, `pushbuffer_loc == SYS` unless the module parameter says `vid`); the native oracle's `cup2` pushbuffer was sysmem too (`native_dataplane_cup2_ga106.md:80,127`) | a CPU read of the guest's memfd at RAM speed. **No vidmem read at all.** This is the common case |
| **vidmem** (a guest that sets `pushbuffer_loc=vid`, or a kernel ring placed in FB) | the guest | ⊘ never a CPU read — 48 MiB/s. A **CE copy** on our own raw-client channel with `src = GPGA_VA_BASE + fb_phys` (arithmetic, the identity window) into a sysmem staging buffer; or, as the owner allows, `cuMemcpyDtoH` from the libcuda context that already holds GPGA |

⚠ Both vidmem routes run **behind the trap, in the worker** (§48: ms budget there, µs in the
trap) and both are a submit-and-wait. `[inferred, unmeasured]` a `cuMemcpyDtoH` of a few KiB may
be serviced by libcuda through a BAR1 CPU read rather than a CE — in which case it is bound by the
same 48 MiB/s bus figure the CE route exists to avoid. Measure before choosing it; the raw-client
CE is the route whose mechanism is known.

⊘ **Passthrough channels are never read** (`parsing_not_placement_is_what_is_forbidden`): the
guest's own CUDA pushbuffers are fetched by the host channel through the mirrored space, and the
compute class has **no physical-aperture operand to rewrite** (`grep -c PHYS clc7c0.h` → 0).

## The promote / demote protocol

`[w824]` ⊘ **SUPERSEDED by §56 rules 2/3 — see the block above "Page tables". The first paragraph
(CE, never the CPU) stands; the protocol steps do not.**

★★★★★ **The CPU never reads video memory on our path. The copy engine does.** Measured: the
processor reads video memory at **48 MiB/s**; an engine reads it at **hundreds of GB/s** and
crosses the link at **12.33 GB/s**. So every promotion copy is a **CE copy on the scratchpad
channel**, never a `memcpy`.

⇒ And the rule is **absolute, not a common case**: a child page is promoted *before* it is read,
because we learn it is a page table from its **parent**, which is already promoted. There is no
first sight that requires an aperture read. Absolutes survive refactoring; common cases do not.

### The scratchpad's address space

| range | contents |
|---|---|
| `X .. X + |GPGA|` | **the whole of GPGA**, so an address is `X + gpga_offset`. Initially all the reserved object; portions become host-memory regions as promotions land. |
| above it | the **fake range** — every promoted buffer, at `fake_base + gpga_offset`. Sparse, so no free list and no block tracking: the offset IS the key. |

⊘ Both must be mapped at once, because a promotion copy has a source in one and a destination in
the other. ★ And GPGA is *always* live and correct in this one GPU VA, which is what makes a
scrubber or a kernel CE copy pure offset arithmetic.

### Promote

1. allocate the host buffer
2. map it into the **fake range** (DMA)
3. **CE copy** vidmem → fake
4. map it over the GPGA range, so that offset now reads the fake buffer — it is authoritative
5. update every other view: PRAMIN, BAR1, BAR2, any channel, from the mapping registry
6. force dirty, so the entry read that triggered this actually re-reads

### Demote — at the END of the refresh, never during

1. unmap from the GPGA range, so the reserved object shows through again
2. **CE copy** fake → vidmem
3. update every other view
4. unmap from the fake range
5. free the host buffer

★★★ **Two conditions, both residuals over a FINISHED walk** — which is why demotion is last:

1. the range is no longer reachable as any valid PDB/PDE/PTB/PTE. Detectable **without a
   sweep**: a dirty page showing a PDB that no longer references a PTB, plus our own table
   saying no other PDB did, means the PTB is dead.
2. no older PTE still maps that range — nothing remains of earlier VA-space allocations, for the
   case where a PTE maps a table's own memory.

⊘ **PRAMIN and BAR2 referencing it do NOT block demotion.** The driver touches a range through
those only during allocation and deallocation, and that is forbidden during an invalidate. What
*does* block it is the range being mapped into an unrelated channel that may be using it.

### ★★★ Two properties that make this affordable

**Getting demotion wrong costs CHURN, not correctness.** The copy-back happens before the buffer
is released, so the bytes survive either way; a wrongly demoted page is simply promoted again
next refresh. ⇒ Conditions (1) and (2) may be **conservative approximations**, the reference
table behind (1) need not be perfect, and the safe bias is *not* to demote when unsure.

**Promotion is a FIXPOINT and needs its termination said out loud.** A newly promoted directory
reveals table addresses we did not know, so it iterates. It terminates because each round
promotes at least one new page and the page set is finite, and because the walk already refuses
cycles and dangling pointers. ⚠ An unbounded loop here hangs the invalidate the guest is waiting
on — the one place we cannot afford one.

### ⚠ THERE IS NO GLOBAL QUIESCENCE — scope the refresh to the trigger's address space

`[verified 2026-09-11, ogkm-580]` **UVM and RM serialise independently.** `uvm_mmu.c` contains
**no** RM interface call — no `rmapi`, no `nvUvmInterface`, no `rm_gpu_ops` — and does all
page-tree work under its own mutex (10 sites). RM uses the GPU group lock. Neither knows about
the other's.

| trigger | caller holds | excludes | does **NOT** exclude |
|---|---|---|---|
| RM map call | RM GPU group lock | other RM page-table work | **UVM editing any of its trees** |
| UVM kernel channel | that tree's mutex | edits to **that** tree | RM work; UVM's other trees |
| TLB invalidate | whichever subsystem issued it | that subsystem's own | the other subsystem entirely |

⊘ So RM can sit blocked on our RPC holding its lock while UVM edits trees throughout. ⇒ **A
GLOBAL refresh has no safe window.** Only the address space named by the trigger is quiesced.

★ **Scope the refresh to that address space.** Every trigger names one — the invalidate carries
its PDB, the map call names its space. What we give up is noticing changes nobody invalidated,
which is exactly right: a change nobody invalidated is one the guest has not yet asked the GPU
to honour. ⊘ The two subsystems own **disjoint** spaces (UVM manages the externally-owned ones,
RM does not walk them), so scoping leaves no gap where both could edit the same tables.

### The invalidate also carries a DEPTH, and we discard it

`tlb_invalidate_all(push, pdb_address, invalidate_depth, membar)` — UVM computes the depth from
the shallowest directory it touched (`dir->host_parent->depth` when linking, `dir->depth` when
freeing) and asserts it non-negative. ⇒ **Free information from the guest about the extent of
its own change**: re-walk from that depth, and everything above it needs neither promotion nor
re-reading.

### Root identity comes from the RPC path; root CONTENTS from the walk

A page directory base **never arrives by invalidation**. It is `SetPageDir` for a normal space,
`UPDATE_BAR_PDE` for BAR2, and for BAR1 a number **we publish**. ⇒ The root never needs the
"assume dirty" treatment unpromoted video-memory pages need — it changes only on an observable
event. ⚠ And conflating the two sources is what produced the zero-`bar1PdeBase` bug: a wrong
root makes every walk from it read the wrong memory, and **no invalidate would ever correct it.**

### ★ PRAMIN is excluded from demotion condition (2) — verified, not assumed

`[verified 2026-09-11]` `memmgrGetMemTransferType` has exactly ONE path returning
`TRANSFER_TYPE_BAR0`, guarded by `IS_SIMULATION(pGpu)` — *"significantly faster on fmodel …
because of the backdoor memory reads and writes"*. **On silicon that branch cannot run.**
Everything else goes to a processor copy, the copy engine, or GSP DMA.

⇒ The exclusion rests on the driver's control flow, not on *"it would not do that during an
invalidate"*. ⚠ Two caveats: firmware and early boot are a different regime (our own capture
sees heavy BAR0-window traffic there), and the guard is a *simulation* check rather than a
post-init one, so a future driver could widen it. Assert it rather than comment it.

⊘ **BAR2 is NOT excluded.** It is a live transfer path on real hardware, so excluding it would
rest purely on timing. It does not need an exception anyway: if the table is genuinely unused,
its BAR2 mapping goes with it.

### Exclusion, not protection

One refresh runs at a time, under a lock that **a vCPU never takes** and that may block. ⊘ It
excludes a second refresh; it protects no data. It cannot: the guest does not participate in our
locking, so a lock over guest memory is a claim we are not entitled to make.

⚠ **The unmapped window is real and the code must say so.** Steps promote-4 and demote-1 unmap
before they map, so the GPGA range is briefly absent from the scratchpad. That is safe **only**
because the sole engine work against the scratchpad is ours and we are inside the exclusive
refresh. State it at the site: it is exactly the kind of premise a later change breaks silently.

### Batching is mandatory

A CE copy completes asynchronously against a semaphore, so each promotion is a submit and a
wait. At the **1872** page-table pages measured in `w422`, per-page submission means 1872 round
trips and the round trip — not the bytes — becomes the cost, which is the same shape as the
48 MiB/s finding one level up. ⇒ One submission carrying the whole batch, then one wait. Order
within the batch is preserved; the batch boundary is not an optimisation to add later.

## What promotion actually costs

Not the copy. The **re-pointing**. A promoted page must stay reachable by the copy engine, which
writes page tables too (`ce_hal->memset_8` on the memops channel), or a copy-engine write lands
in video memory while our authority is in host RAM and we miss it silently. So every promotion
updates the guest's aperture mapping, any scratchpad mapping, and a GPU-side mapping of the host
pages — all inside the window. That is `GpgaViews` doing real work, and it is the reason it must
be **wired** rather than merely built.

## The DoS this closes by construction

| | today | under this |
|---|---|---|
| host RAM the guest can make us hold | up to the **advertised framebuffer** (12 GiB) by touching every page | page tables only, ~**7 MiB**, self-limiting via demotion |
| video memory | allocated lazily, can fail **mid-refresh** where we cannot recover | reserved at start, fixed, cannot fail later |
| more memory than entitled | possible via invented pages | impossible; DMA is already bounded by guest RAM |

⇒ What remains to bound is **countable**, not byte-valued: isolates, address spaces per process,
channels per space, tracked page-table pages. Refused by name, not by allocation failure.
⚠ Those collections are currently **uncapped** — ordinary maps that grow on a guest action. Small
per entry, unbounded in count, which is the same shape this design removes for bytes.

## Sequencing — deliberately less than the whole design first

`[w824]` ⊘ Step 1 **is done and measured** (`the_vm_lifetime_scratchpad_isolate_holds_11904_mib`:
11 904 MiB held as one object at PCI realize, `nvidia-smi` 11 909 MiB used; 6144 MiB is the
largest *contiguous, len-aligned* form measured, 09-11). Step 2 below is **replaced** by the
identity window — one FIXED map of the whole object, then `Translated` flips physical operands to
it; step 3 widens to the walkers-as-storage and the MMIO latches (§56.1). The order is now:
1 reserve (done) → 2 identity window + phys-operand rewrite → 3 mirrored space fed by the guest's
own synchronisation points → 4 delete.

1. **Reservation only.** One object, refuse to start on failure, advertised size from the
   reservation. **Keep the existing invented framebuffer as the read path**, untouched. This
   already deletes the join and its copy, ends mid-operation OOM, and gives one object to slice —
   most of the structural win — while the PCIe question stays moot.
2. **Promotion / demotion**, narrowing the invented framebuffer from *everything* to *page tables
   only*. `[w824]` ⊘ superseded — see above.
3. **Delete** the join machinery: 146 references across 10 files, mechanical once nothing calls it.

## Gates

⚠ Everything decided today rests on one five-minute boot with a measured 1-in-5 false-negative
rate. That is not enough signal for a change this size. Before the boot:

- a **host-side test** that reserves, slices, maps into a channel and reads back — seconds, no VM;
- then the raw client at `threads:8 rounds:8`;
- then the LLM, graded on **text against a same-boot CPU oracle**, never on a token count.
