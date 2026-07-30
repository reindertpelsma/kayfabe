# GPGA — the guest-physical GPU address space, lazily materialised

> Status: **designed, not built** (owner brainstorm, 2026-07-30; explicitly *not high priority*).
> This is the **full compliant path** for guest-physical GPU memory, not the happy path. It is
> recorded now because the owner's standing instruction is that the right *construct* makes the
> compliant path the natural one — and choosing that construct is cheap now and expensive later.

## 0. The governing instruction

> *"you should not only implement the happy common path but the full compliant path, and it's
> easiest if you already used the right construct."*

Everything below is one sparse-address-space abstraction. If it ends up as a fast path plus a pile
of special cases, the construct is wrong.

## 1. What GPGA is

**GPGA = guest-physical GPU address.** The address space the guest believes is its GPU's physical
memory. It is **entirely bookkeeping on our side** — no part of it need correspond to host VRAM at
the same offset, or to anything at all until touched.

★ **But where it is contiguous to the guest, we must PRESENT it contiguous.** Our backing is
chunked and lazily materialised; that chunking must be *invisible*. A guest that allocates a
contiguous GPGA range and walks it must see one range, not our allocator's seams.

## 2. The invariants (owner, verbatim in substance)

1. **Any read/write on a valid pointer is valid.** No "this address is bookkeeping so it has no
   contents" state is observable.
2. **Any range can be mmap'd to guest processes, and overlapping ranges can be mapped to multiple
   guest processes.** Aliasing is legal and expected.
3. A range **contains a mix** of fake pages, real GPU-VA-backed objects, and null pages.
4. **munmap → remap must preserve contents.** ⇒ **orphaned pages/objects — backing with no GPU VA
   it belongs to — are a REQUIREMENT, not an optimisation.**
5. **If mapped into BAR1/BAR2, map/unmap has no effect on contents.** A BAR mapping is a window,
   not ownership.
6. ★ **The scrubber is NOT a no-op.** For us it need not literally clear a page: **freeing the
   backed GPU-VA page is usually sufficient, because a newly allocated host object is already
   zero-initialised.** See §5 for the one thing to verify before relying on that.

## 3. Why null pages exist

**We cannot preallocate the range.** Most of GPGA is never touched, and a guest that dumps its
whole physical GPU space must not cause us to allocate it.

- **The null page** is a small **read-only** range of zeros — order **8 MiB** — mappable
  *anywhere* in GPGA that the guest has **read but not written**.
- Consequence, and the point of the design: **a full read sweep of GPGA allocates nothing.** Every
  unwritten page resolves to the same shared zero range.
- **On a write touch**, materialise: start at **one page**, then grow **exponentially** up to
  **16 MiB** chunks. Newly materialised backing is **orphaned or mapped to BAR** — it exists
  independently of any GPU VA, which is what makes invariant (4) hold.

## 4. Promotion — when GPGA gets real backing

When a GPGA range is mapped into a GPU VA (by a **PDB** or by **RM**), it must be backed by a real
page that genuinely occupies memory. What that page must *contain* differs by who is mapping:

**By a PDB.** Contents **must be preserved**. Two cases:
- The original was unmapped, or was null pages ⇒ **a fresh object suffices.** ★ This is the common
  case: the guest kernel running its scrubber produces exactly this state.
- Otherwise ⇒ **preserve the contents** — the guest deliberately wanted to share those pages.

**By RM (captured phys).** It depends on what real hardware would do:
- If a real GPU would present it empty ⇒ **do the same.**
- **Unless our pages are already non-empty** ⇒ then the new backing must be **initialised with
  that content**, because the guest can already observe it.

## 5. Assessment — where this is right, and the two things to check

★★ **The shape is right, and it is one abstraction.** A sparse address space with a shared
read-only zero page, write-triggered materialisation with exponential chunking, and orphan-capable
backing is the standard construct for exactly these invariants. Invariants (1)–(5) all fall out of
it rather than needing separate machinery, which is the test the owner's instruction sets.

★★★ **It shares its core construct with the §12 CE split.** Both need a **range algebra**:
partitioning an operand range into sub-ranges of differing kind (materialised / null / real-object;
fabricated / representable) and acting per sub-range. **Build it once.** If the CE split and the
GPGA allocator grow two different range types, that is the smell that the construct was missed.

**Two things to verify rather than assume:**

1. ⚠ **"A newly allocated host object is already zero-initialised."** This is very likely true —
   NVIDIA scrubs VRAM on allocation for security — but it is load-bearing for invariant (6) and it
   is currently an assumption, not a citation. **Measure it, or cite the open kernel module, before
   relying on free-as-scrub.** If it does not hold on some path, the fallback is cheap: after a
   free, resolve the range to the **null page** — which gives "reads zeros" with no allocation at
   all, and is arguably the better implementation regardless.
2. ⚠ **Overlapping mappings into multiple guest processes** (invariant 2) is the same aliasing that
   `#14` is about, and `proc_is_not_a_set_of_rm_clients` measured that two concurrent CUDA
   processes *share* one dup-DST client. Legal aliasing at the GPGA layer must not become
   *identity* aliasing at the Proc layer. Whatever keys the backing must be the range, not the
   mapper.

★ **On the scrubber, one clarification worth stating**: "freeing is sufficient" and "the scrubber
is not a no-op" are compatible, and the reconciliation is §3's null page. Free the backing, resolve
subsequent reads to the shared zero range, and the guest observes a scrubbed page having caused
neither a clear nor an allocation. That is the construct doing the work instead of a special case.

## 6. Relationship to the rest of the design

- **MISS = FAULT is unaffected.** The address table stays forward-populated; nothing here
  reverse-resolves. A GPGA page with no backing is not a *miss* — it is a **null page**, which is a
  known state with known contents.
- **§12's representability rule composes**: materialised GPGA backing is host memory a real engine
  can be pointed at, so it is *representable* and takes the fast path. Null pages are read-only
  shared zeros, which a real engine can also read. Only genuinely fabricated structures stay ours.
- **Orphan support (invariant 4) already has a precedent**: the C's teardown hardening built a host
  reaper and a GPA free-list, and the Rust carries the `#80` recycle regression from it. Orphaned
  backing is that lifetime, made first-class rather than a teardown special case.

## 7. Sizing GPGA, and what happens when the HOST GPU is out of memory

> Owner brainstorm, 2026-07-30, explicitly *"none of this seems urgent to get cup2 to pass"*.
> Agreed — nothing here blocks first compute. Recorded because two of the answers are cheap
> **only if taken before the allocator is written**.

### 7.1 Shrinking GPGA is nearly free, and it is the PRIMARY answer

**[measured]** The guest learns its VRAM size from a **single emulated register**:
`NV_USABLE_FB_SIZE_IN_MB` (`0x001183A4`), which the C answers with a compile-time constant
(`C: src/qemu/mode2_regs_ga10x.h:62` — `12288u`, "12 GiB (RTX 3060)"). Turning that constant into a
per-VM configuration value is a knob, not a project.

⇒ **A user-settable GPGA size gives per-VM VRAM limiting almost for free**, and it is the same
mechanism the industry already uses for isolation: vGPU gives each guest a fixed framebuffer and
MIG partitions memory statically. **Neither overcommits.** That is not a coincidence — see §7.2.

⚠ **One consistency trap.** The register is not necessarily the only place the size appears. The C
also models RPC **fn 65** by splicing a **captured `GspStaticConfigInfo` blob**
(`c_rust_trace_differential.md`, F-4). If a size lives in that blob too, shrinking the register
alone leaves the guest holding **two different answers**, which will not present as "wrong size" —
it will present as an inexplicable downstream failure. **Find every place the size is stated and
derive them all from one value.**

### 7.2 ★★★ The tension to name out loud: lazy materialisation IS overcommit

§3's design — most of GPGA unbacked, materialised on write touch — is exactly memory overcommit.
So these two goals are in direct conflict and cannot both be had unconditionally:

- *"Only allocate what the guest touches"* (density), and
- *"An allocation the guest's own accounting says is free never fails"* (safety).

Two coherent postures, and the mistake would be drifting between them by accident:

1. **Reserve.** GPGA size **is** a reservation against host VRAM. Admission control refuses to start
   a VM whose GPGA does not fit alongside existing ones. Wastes VRAM; **cannot OOM**; matches
   vGPU/MIG. ★ Recommended default for multi-tenant.
2. **Overcommit.** Density, and §7.3's failure path becomes load-bearing rather than theoretical.

**Recommendation:** make it a policy knob, default to **reserve** when more than one VM shares a
GPU, and treat §7.3 as required regardless — because a host process outside our control can always
consume VRAM even under posture (1).

### 7.3 When allocation fails, by where it fails

**At an RM command** — easy, and well-trodden: return the RM status a real driver already returns
(`NV_ERR_INSUFFICIENT_RESOURCES` / `NV_ERR_NO_MEMORY`), which surfaces to CUDA as
`CUDA_ERROR_OUT_OF_MEMORY`. The guest driver has handled this path since forever. **The guest's
error will not name the true cause (host-side OOM) — and that is acceptable.** The requirement is a
defined failure, not an accurate diagnosis.

★★★ **At a PDB update — the hard case, and the owner named it correctly.** Materialisation here is
triggered by the guest *writing a page-table entry*, not by a call it made. **There is no return
channel.** The guest is not expecting a failure and there is no status to fill in.

The faithful surface is a **GPU fault** — which is what real hardware raises when a page cannot be
accessed, and which the guest driver already has a handler for.

★★ **This is the SAME mechanism as `#111`'s "a bad application pointer in Mode 2 must surface as a
simulated GPU fault".** Different trigger, identical need: *we cannot materialise this access, and
must tell the guest in a language it already speaks.* **Build the fault-injection path once**, with
at least two triggers feeding it. If OOM-at-PDB and bad-pointer grow separate mechanisms, the
construct was missed — the same smell §5 flags about the range algebra.

### 7.4 The guest cannot see host GPU usage — and that is a FEATURE, not only a problem

The owner is right that Mode 1 and NVIDIA containers do not have this problem *because all GPU
stats are shared*. ★ **The flip side is worth stating: shared stats are a cross-tenant information
leak.** Fine inside one trust domain; not fine between VMs. **Mode 2's opacity is precisely what
makes it more isolating than a container** — so the fix must not be "share the host's numbers".

**What to report instead: the PARTITION's numbers, not the host's.** Total = this VM's GPGA size;
used = our own accounting for this VM. Self-consistent, leaks nothing, and is what a guest actually
wants to know. Again the vGPU model.

⇒ Under posture (1) **the problem largely dissolves**: the partition's numbers are *true*. Under
posture (2) they are true about the partition and optimistic about the host, which is the classic
balloon situation — and §7.3's fault path is the honest handling of it.

**Not urgent.** None of §7 blocks `cup2`. §7.1's consistency trap and §7.3's shared fault path are
the two items worth deciding *before* the allocator exists; the rest can follow.
