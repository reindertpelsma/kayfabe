# One sparse memfd for the emulated GPGA range

**Status:** owner proposal, 2026-08-10, recorded because it existed only in conversation.
**Verdict: ACCEPT the shape, DEFER the port.** Not a rewrite — the abstraction it needs already
exists. Companion to `mode2_guest_ram_crossing.md` (the guest-RAM memfd) and
`mode2_isolate_memory_boundary.md` (the mmap authorization protocol it reuses unchanged).

---

## 1. The proposal, in the owner's own scoping

> *"For emulated framebuffers I would use memfd all along. Make **one** memfd of the entire GPGA
> range — most ranges are sparse (consume no data, still neat offsets). **Only GPGA that's emulated,
> so sysmem, is put here.** Then you can hand this sparse memfd to the isolates as well, next to the
> memfd of guest RAM, both seccomp-isolated. I would avoid making every emulated FB a memfd. Same
> mmap security. All emulated FB at the correct GPGA offsets, rest sparse. Easily mmapable in KVM
> memslots."*

★ The scoping clause is load-bearing and is **already correct**: this holds the GPGA we *fake*.
Ranges backed by real host GPU objects stay real host objects — see §4.

---

## 2. ★★★ The property that earns it: the offset IS the address, so there is no map

The guest-RAM crossing needs a **stated GPA→offset map** — the machine has several RAM blocks and a
4 GiB hole, so the mapping is neither identity nor derivable from the machine type. That is an entire
rung's worth of work, and its governing rule is *"the map must be STATED, not DERIVED."*

**A GPGA memfd makes `offset == GPGA` by construction.** There is no map to state, therefore no map
to get wrong, therefore no rung. ⇒ This does not *solve* the guest-RAM problem on the FB side; it
**dissolves** it. That is a stronger property than convenience and is the main reason to take it.

Secondary properties, all real but none decisive on their own:

- **Sparseness is free and exact.** A memfd is tmpfs; unwritten pages cost nothing, and `ftruncate`
  to the full ~12 GiB GPGA range is O(1). Sparse is what the store already is (§3) — this makes it
  the file system's job instead of a `HashMap`'s.
- **One policy, two fds.** `mode2_isolate_memory_boundary.md` §2–3 hardcodes a **fixed fd number**
  into the seccomp filter and authorizes each `mmap` against a VMM-originated `(offset, length)`.
  A second fixed fd is a second constant. **The protocol does not change at all.**
- **KVM memslots fall out.** A memslot needs a userspace VA backed by something mappable; an mmap of
  the sparse region is exactly that, and the guest-CPU view and the isolate view then come from **one
  object** — the C's hand-rolled double-mmap, but structural.

---

## 3. ★★★★★ It is NOT a rewrite — measured, because the owner's stated unknown was a number

The owner's hesitation was specific and honest: *"I am not having a picture of all sites depending on
the current sysmem fake GPGA alloc… maybe too much design rewrite that breaks everything."*
**That is a countable question, so it was counted** (`kayfabe` @ `99672fe`):

| fact | value |
|---|---|
| the abstraction | ★ **`pub trait FbStore`** — `kayfabe-device/src/fbwin.rs:232` |
| existing implementors | **two**: `RefusingFb` (`:481`), `SparseFb` (`:653`) |
| `SparseFb`'s storage | `HashMap<u64, Box<[u8; FB_PAGE]>>` (`:573`) — **already sparse, already page-granular, already offset-keyed** |
| mentions of either impl | 29, across 10 files — **4 of them tests** |

⇒ **The memfd version is a THIRD IMPLEMENTOR of a trait that already has two.** Additive by
construction. The worry is refuted by the code: the seam the port needs was built already, because
somebody needed `RefusingFb`.

★ And a detail that argues the memfd is *simpler* than what exists: `fbwin.rs:315` carries a
note — *"★ **Ascending, always.** `SparseFb` is a `HashMap`, whose iteration order…"* — i.e. the
nondeterministic order is already worked around by sorting on read. A file-backed store makes that
ordering **structural** instead of a discipline.

---

## 4. ⊘ The boundary, and the two corrections it needs

**memfd pages are host system RAM. They are never GPU vidmem.** So this cannot replace
`nvkvm_m2_back_and_map()` (`C: nvkvm_gpu_emul.c:5056`), whose entire point is that the backing is a
real host vidmem object (`nvkvm_m2_host_alloc_map_vidmem` → `CONTIGUOUS|VIDMEM` + `RM_MAP_MEMORY`).
A memfd page can only reach the GPU via `OS_DESCRIPTOR`, which presents it as **sysmem**.

⇒ Two buckets, and this is `mode2_dataplane_decision.md` §3's partition rule applied to backing store
rather than to mapping:

| GPGA range | backing |
|---|---|
| touched only by the guest CPU and our emulator (PDTs, instance blocks, fabricated structures) | **the memfd** |
| the host GPU must DMA into or out of it | **a real host RM object** — memfd cannot serve this |

### ⚠ Correction 1 — the boundary MOVES, and this is the only genuinely new design work

A range starts emulated and later needs real backing, when a channel actually runs against it. The C
measured exactly this split at copy time: **`overlay_real_write_bytes ≈ 75 MB`** (`verdict=gpga`,
real host vidmem) against **`fbpage_write_bytes ≈ 8.6 MB`** (`verdict=fbpage`, fake) — decided **per
address**, not per allocation (`mode2_userbuf_vidmem_passthrough.md`). ⇒ A **promotion path**
(emulated → real-backed) and a rule for who triggers it are owed under *any* scheme, including the
C's; the memfd neither creates nor removes that obligation. The C's answer was map-on-touch promotion.

### ⚠ Correction 2 — `OS_DESCRIPTOR` over a hole PINS it

Pinning a range that contains unwritten pages **materializes them**, silently converting sparse to
allocated. Over a ~12 GiB GPGA range that is an OOM, and **this bench has no swap, so an OOM is
instantly fatal**. ⇒ **Only ever pin ranges the VMM has explicitly materialized.** The existing
*"the VMM originates the numbers, it never validates numbers the isolate proposed"* rule
(`mode2_isolate_memory_boundary.md` §3) already gives this for free — it needs naming, not building.

---

## 5. The ruling

**Do not port now.** The trunk is the GR wall (`SET_REPORT_SEMAPHORE` → VA `0x2_0440fff0`), and this
buys nothing toward it. ⊘ Nothing here is a work item today.

**Take it when the FB backing is next touched** — which is imminent if `mode2_fb_crossing_question.md`
concludes we owe a second crossing. ★ The argument for *then* rather than *later* is that
**it is a shape, not a check** (the same argument as the isolate memory boundary): call sites written
against `FbStore` are free, but call sites written against a `HashMap` of boxed pages would have to be
found and rewritten. Right now there are **none of the latter outside `fbwin.rs` itself**, and that is
the cheapest this port will ever be.

⊘ **Kept out of scope:** any change to the real-backed path, to `OS_DESCRIPTOR` usage, or to the
guest-RAM memfd. This page is about where *fabricated* bytes live.
