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

| | PCIe traffic per boot |
|---|---|
| re-read every refresh | tens of seconds — would present as a **hang** |
| promote once per page | **~40 ms total** |

⇒ Promotion converts a per-sweep cost into a **per-page-once** cost. It is load-bearing for the
product, not a percentage gain.

★ And the footprint is small enough that wasting the video memory behind a promoted page is
irrelevant: the whole hierarchy is **one part in five hundred** of what it maps (a 4 KiB leaf
table covers 2 MiB), so 12 GiB fully mapped at small pages costs 24 MiB of tables.

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
