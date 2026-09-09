# ★★★★★ ONE ACCESSOR FOR ANY GPGA — the owner's model, with four corrections

**STATUS: LIVE**, 2026-09-09. Design note, not yet built. Written from the owner's brainstorm
plus what the tree already has.

## The model (owner, 2026-09-09)

> *"any gpga range can be cpu mapped to a vmm va, any, and munmapped. core functions. for fake
> fb partly no-op: mmap just returns the fake fb address as its just ram, munmap does nothing.
> for real fb/gpu va backed/rm object: mmap maps the MMIO address and returns that, munmap
> works, reference counted maybe. for non-allocated, a thing is created."*
>
> *"any code needing to handle gpga data — emulated channels, pte/pdb, vbios — they do not worry
> about the backing, any backing works, they just use the proper function to get an address to
> the view of the thing they need in the vmm."*

Three destinations, one shape:
| destination | fake FB | real FB / RM-backed |
|---|---|---|
| CPU / VMM VA | return the host RAM address; unmap is a no-op | map the MMIO window; unmap real, refcounted |
| guest MMIO (BAR1 VA space) | CPU-mapped, `MAP_FIXED` allowed, mappable at several places | window install |
| GPU (scratchpad / isolate VAS) | DMA (it is ordinary RAM) | ordinary RM mapping |

## ★★★ THE PART THAT IS SHARPER THAN IT LOOKS: we are the writer

> *"if the guest does a ce copy in kernel channel to copy cpu to gpu page tables, since the
> target gpga is fake fb, then in kayfabe its guest ram to fake fb which becomes a memcpy.
> there is nothing special compared to if it wrote directly. so there is no special overcomplex
> interception needed. the dirty flag should work. if the memcpy does not affect dirty flag
> since only guest writes would do, set it manually for affected pages."*

**Correct, and it removes a whole class of interception.** A CE write into fake FB is a memcpy
**we perform**. We do not need to *detect* it — we are the agent of it. Marking those pages
dirty is not a workaround; the writer is the right place to record a write.
★ The tree already attributes exactly this: `framebuffer FIRST-WRITER census: PRAMIN 21 /
BAR1 9 / BAR2 88 / EXEC 1786 / UNATTRIBUTED 0`. `EXEC` **is** engine-written FB pages, and
`UNATTRIBUTED 0` says no writer escapes us.

## What the tree ALREADY has — this is an EXTENSION, not a build
`FbStore` is already backing-agnostic for **copies**: `read(phys, buf)`, `write(phys, bytes)`,
`write_tagged(phys, bytes, by)`, `page_backing(phys, materialise)`, `install_join`,
`release_join_carrying_bytes`, `export`. Consumers already do not branch on backing to move
bytes. What is missing is (1) a **borrowed view** instead of copy-in/copy-out, and (2) the three
destinations above behind one API.

## ⊘ FOUR CORRECTIONS

### 1. Return a GUARD, never a bare address
*"mmap just returns the fake fb address … munmap does nothing"* is true and is the trap this
tree has already paid for twice: `SparseFb::release_join` **dropped the bytes** a join held
(only `release_join_carrying_bytes` preserves them), and the LLM's 16-tokens-of-garbage was a
**stale binding that kept translating** after a re-map. A bare address carries no lifetime, so
nothing stops it outliving its backing.
⇒ Always return a guard. For fake FB its `Drop` is a no-op; for MMIO it unmaps. One API, one
lifetime discipline, and the cheap case stays cheap.

### 2. Make ALIASING a recorded fact, not an accident
*"fake fb can be mapped at multiple places"* — yes, and that is precisely the fb-join aliasing
that produced the corrupt LLM output: one frame host-backed at **one** VA while the guest
aliased **17** at **two**. If the accessor permits multiple live views, it must **record** them,
so an alias is something a census prints rather than something a debugger discovers.

### 3. Copy before deciding, on any guest-writable view
The owner's own caveat, and it belongs in the type: *"when reading it malicious guest can write
bytes under you, so copy if needed."* A view of guest-writable memory must be read into a copy
before validation, or the decision is raceable. Ideally the type distinguishes *"bytes I may
decide on"* from *"bytes the guest may still be writing"* — the distinction is invisible at a
`&[u8]`.

### 4. The dirty flag now has TWO producers, and that must be stated
KVM sets it for guest CPU writes; **we** set it for engine/CE writes we perform. Both are
legitimate. But a future reader who assumes `dirty` means *"KVM saw a guest store"* will miss
ours and conclude a page was never written. Name the producer in the API.

## ⊘ AND ONE SCOPE CORRECTION ON "correct by construction"
It cannot be fully. Whether a GPGA is fake FB or real is a **runtime** property of its backing,
so no type can make the *behaviour* uniform. What CAN be correct by construction is the thing
actually worth having: **consumers never branch on backing.** That is a property of the API's
shape, and it is achievable — `FbStore` already achieves it for copies.
