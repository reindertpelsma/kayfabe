# The store is LIVE-MAPPABLE in the scratchpad — the walk relocation's premise expired six hours after it was written

**STATUS: LIVE, 2026-09-17 (w755i). Design ruling by the owner; supersedes w731a's relocation
for the walk window. NOT YET IMPLEMENTED — sequenced behind the channel-birth blocker.**

## The ruling

> Owner, 2026-09-17: *"I doubt this is a good design. A good design is live gpga mapped in
> scratchpad va so all operations can perform it. The snapshot of pt\*/pd\* was primarily so
> that the ptx can compare threaded the versions to only return delta, then updating the
> snapshot. During this progress all reads are sequential per entry and constantly bound
> checked so a malicious table cannot read outside guest vidmem."*

## ⊘⊘⊘ What w731a argued, and the premise that expired

`w731a` (`f3d06ddc`, **2026-09-15 02:23**) relocates the guest's page-table pages into a compact
arena and **rewrites the address field of directory entries** to point at the new home, because:

> *"a window that answers for the guest's tables at their own GPGA must be as long as the
> highest table page's address … `span_pages=3087533` — ~11.78 GiB up a 12 GiB framebuffer …
> An identity window is an 11.8 GiB device allocation on a 12 GiB board. Not affordable."*

⊘ **`KAYFABE_FB_STORE=device` — the framebuffer actually being the one reserved object — landed
at `96725a23`, 2026-09-15 08:17. SIX HOURS LATER.** When the relocation was designed the
framebuffer was still the sparse memfd arena: there was no single object to map, and *"no test
buffer can hold it"* was true. It stopped being true that morning and the design did not move.

## ★★★★★ And the argument conflates two different costs

*"An identity window is an 11.8 GiB device **allocation**"* is true only of a **new buffer**. The
window needs 11.8 GiB of **VA**, not of memory — 49-bit address space on Ampere, free — and the
memory is **already allocated**: it is the store. `RmBackend::map_gpu_va` maps a whole object
into the isolate's own VA space, exists today, and **nothing calls it for the store**.

⇒ the identity window costs one `NVOS46` map. There is no allocation to afford.

## What the snapshot is actually for

**Delta detection**: walk, compare against the previous version, return only what changed, update
the snapshot. That is the PTX's job and it is unaffected by where the tables are read from.
⊘ Relocation borrowed the snapshot to solve *addressability*, which is a different mechanism
wearing the same name — and is why the two got confused.

## The safety property is preserved, and it never depended on relocation

A malicious guest table cannot read outside guest vidmem because every dereference is
`win.base + gpga` **bounds-checked against `win.len` per entry**, with reads sequential per entry
(`cuda/walk/kf_walk.cu:346,357`). That is a property of the **bounds check**. A live window over
the store has exactly the same guarantee, with `win.len` = the reservation's length — and the
bound becomes *the guest's own video memory*, which is the honest statement of the invariant
rather than a proxy for it.

## ⊘⊘⊘⊘ AND THE COST IS WORSE THAN "AN EXTRA COPY" — THE DIFFERENTIAL IS FORECLOSED

`walkshadow.rs:553-560`, the relocation's **own** doc, states it:

> *"The image holds exactly the pages the **host walk visited**. ⇒ the kernel's reach is
> clipped to the host's reach, and it cannot find a subtree the host never entered."*
> `ExtraInKernel` — *"live within the visited pages, **foreclosed beyond them**"*

★★★★★ **The GPU walker exists to catch what the host walker gets WRONG. Relocation forecloses
exactly the direction in which the host is wrong BY OMISSION** — a mapping the host never
visited cannot be in the image, so the kernel cannot report it. The second walker is structurally
prevented from disagreeing in the one way that matters most.

⊘ And `absent_edges=0` — recorded on w731's boot and read as clean — is a statement about
**scope**, not agreement: it counts edges pointed at the reserved zero slot because the host
never went there. A live window has no reserved slot and no clipping, so the number stops
existing rather than reading as good.

## What the change removes, and what it unlocks

- `GmmuFmt::relocate_entry` and `walkshadow::build_image`'s rewriting go away.
- **Leaf PTEs become followable.** Today *"a leaf's target is reported, never followed"* because
  the target is not in the window. Under a live identity window it is, under the same bound.
- ★★★ **It makes the store addressable to everything in the scratchpad**, which closes a question
  raised the same day: whether `cuMemcpyAsync`/`cuMemsetAsync` could serve the emulated CE and
  scrub planes. One mapping serves the walk kernel, the emulated CE, and CUDA.

## ⚠ THE ONE MEASURABLE OBSTACLE, and the route that probably clears it

The walk kernel dereferences `win.base + gpga` as a raw device pointer. A kernel's load needs no
CUDA blessing — it needs the **VA to be mapped in the context's address space**. And libcuda
creates its **own RM client and VA space**; ours cannot `NVOS46` into it without a handle it
never gives us. That, not the window size, is the real obstacle.

★★★ **The route is to invert the ownership — allocate the store THROUGH CUDA and import it into
RM**, rather than trying to push an RM object into CUDA:

1. `cuMemCreate` the reservation (real device memory, CUDA-owned, CUDA-addressable by
   construction — the walk kernel and `cuMemcpyAsync`/`cuMemsetAsync` all just work);
2. export it to a shareable handle (`CU_MEM_HANDLE_TYPE_POSIX_FILE_DESCRIPTOR`);
3. **import that fd into our RM client**, so we still get the handle `map_store_slice` needs to
   place slices into **guest** VA spaces.

⊘ Step 3 is the one that is unproven and it is the whole design: without an RM handle the single
store cannot be mapped at guest VAs, which is the thing everything else rests on.

⚠ **PROBE IT, DO NOT REASON ABOUT IT.** A CUDA container, no guest, ~30 minutes: `cuMemCreate` →
export fd → `NV_ESC_RM_IMPORT_OBJECT_FROM_FD` → `NVOS46` a slice of it at a fixed VA. If step 3
answers, the whole design is a small change and the CUDA CE/scrub proposal lands with it. If it
refuses, the fallback is a live window over the RM-allocated store for the kernel only, which
still removes the foreclosure — and the CUDA data-plane idea needs a different bridge.

## Sequencing

Deferred behind the measured blocker (`ADOPT-WHY (6)`, channel birth). Changing the walk path
under a measurement in flight is how a boot comes back uninterpretable.
