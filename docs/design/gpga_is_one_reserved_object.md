# GPGA is ONE reserved object

**STATUS: LIVE (2026-09-10).** Owner's design, settled in conversation this date. Supersedes the
per-leaf join described in `fbwin.rs`'s module docs, which is scheduled for deletion by it.

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

## Page tables: promote in the invalidate window only

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

## The promote / demote protocol

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

1. **Reservation only.** One object, refuse to start on failure, advertised size from the
   reservation. **Keep the existing invented framebuffer as the read path**, untouched. This
   already deletes the join and its copy, ends mid-operation OOM, and gives one object to slice —
   most of the structural win — while the PCIe question stays moot.
2. **Promotion / demotion**, narrowing the invented framebuffer from *everything* to *page tables
   only*.
3. **Delete** the join machinery: 146 references across 10 files, mechanical once nothing calls it.

## Gates

⚠ Everything decided today rests on one five-minute boot with a measured 1-in-5 false-negative
rate. That is not enough signal for a change this size. Before the boot:

- a **host-side test** that reserves, slices, maps into a channel and reads back — seconds, no VM;
- then the raw client at `threads:8 rounds:8`;
- then the LLM, graded on **text against a same-boot CPU oracle**, never on a token count.
