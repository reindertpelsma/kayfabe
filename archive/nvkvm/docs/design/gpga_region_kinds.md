# GPGA region kinds — the core design

> ### STATUS — 2026-08-11 / **LIVE, WITH A MEASURED CORRECTION AT §0 — READ §0 FIRST**
> **Owner's model, stated in conversation 2026-08-11 and written up here verbatim in substance.**
> ⊘ **This is a DESIGN STATEMENT, not a measurement.** Nothing may be retired on its authority
> until a boot says so. What *is* measured is recorded in §0, §3 and §4 below, separately marked.
>
> ⊘⊘ **§0 corrects the model's central mechanism** — *"the kind is decided when the region is
> allocated"*. **We are not present at that allocation.** The model survives as a target; the
> decision point it asks for **has nowhere to live today**.

---

## 0. ⊘⊘ MEASURED CORRECTION (2026-08-11) — THE DECISION POINT HAS NO SITE

**This corrects §1's *"decided when the region is allocated"* and §2's premise that the four
surfaces are ours. It does not retire either — it says what is missing before they can be built.**

★★★ **Three of the four surfaces named in §2 are not ours in Mode 2.** The guest runs the **stock**
kernel driver: `/dev/nvidiactl` and `/dev/nvidia-uvm` are **its own**. Its RM carves video memory
out of **its own heap**, over the framebuffer *we advertise*, and **never asks us**.

| | measured |
|---|---|
| every `hClass` our boots ever saw on the **GSP wire** | `0xc36f`, `0x402c`, `0x0070`, `0x208f`, `0xc076` |
| the guest's **own** `/dev/nvidiactl`, real hardware | **19× `0x003e`** (`NV01_MEMORY_SYSTEM`), **5× `0x0040`** (`NV01_MEMORY_LOCAL_USER`) |

⇒ **24 backing-allocating calls per CUDA run, zero of them reach us.**

★★ **And the absence is a MEASUREMENT, not a silence** — ✔ *re-verified independently before this
correction was written*: `kayfabe_abi::versions::alloc_params` has arms for Device / Tsg / CtxShare /
Channel / VaSpace / engine objects and **no arm for `0x003e` or `0x0040`**. Had either ever arrived,
it would have been **refused** and printed a `GspRmAlloc failed` line. Those lines are the census.
**Arrival could not have gone unrecorded.**

⇒ ★★★ **The taxonomy is not wrong. It is UNWITNESSED.** The fix is not *"add a decision point"* —
there is no call to hang one on. It is **manufacture a witness**, and only one candidate survives:
**advertise less framebuffer**, which followed to its end is `PDB_PROP_GPU_ZERO_FB` — *the same
property that gates the dma-buf CPU-mmap door* (`is_passthrough_the_only_correct_route.md` §4 item 4).
⚠ **That is a strategic option, not a decided one.**

### Three further measured findings from the same pass

- ⊘ **There is no "unallocated" state today.** `SparseFb` fabricates a **zero page** for any address
  below `fb_length` — the code says so outright. So §1's first row has no implementation.
- ⊘ **`BackingBytes` is NOT the thing to delete** (correcting §5). It is the tree's **only declared
  kind, with no default**, and it exists because this exact bit was derived and measured backwards.
  ⇒ **It is the SEED of the declared `Kind`, not its casualty.**
- ★★ **There are TWO derived defaults pointing OPPOSITE ways**, not one: `Fabricated` is the
  unguarded fall-through, but `Untracked` — *no row at all* — routes to **real hardware**.
- ⚠ **A fifth surface, and it is worse than a missing kind**: BAR1 is the userspace window, and the
  BAR1 directory walk reads **out of `SparseFb` itself** — so the fiction *is* the userspace path's
  address model. And the rule is **unstateable**: the trap carries `(bar, off, size, val)` with **no
  CPL, no CR3, no guest PID, no `Proc`** — kernel and userspace arrive at the same line.

⇒ Full workings: `../../nvkvm-rs/docs/design/gpga_region_kind.md` (branch `fb-memfd-join`, `b3ecda4`).

---

## 1. The model

**A GPGA region is exactly ONE of four kinds, and the kind is DECIDED WHEN THE REGION IS
ALLOCATED — not derived later from whatever happens to be present.**

| kind | backed by | created when |
|---|---|---|
| **unallocated** | nothing | the default for untouched pages; a miss is a fault |
| **fake framebuffer** | our emulated store | ★ **ONLY when the guest KERNEL builds an internal channel we emulate** — it is where *we* manage that channel's pushbuffer / USERD / ring / semaphore |
| **real GPU memory** | a real allocation (`NV_ESC_RM_MAP_MEMORY` / `NV_ESC_RM_ALLOC_MEMORY`) | ★★★ **a guest USERSPACE mapping request against an unallocated region** |
| **DMA to guest physical** | `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` | guest RAM the GPU must be able to reach |

### ★ The awkward case, and its clean answer

If fake framebuffer must ever appear at a **real GPU virtual address** — the scratchpad, for
instance — **route it through `OS_DESCRIPTOR`.** Then all three parties are simultaneously correct:

- **we** know the bytes are fabricated,
- **the guest** believes it is device memory,
- **the real GPU** believes it is host RAM it was told to DMA into.

⇒ **Nobody is lied to in a way that matters**, and no party has to be told a fact it can disprove.

---

## 2. What unprivileged guest userspace may see

**Exactly four surfaces, and the fake framebuffer is not among them:**

1. `NV_ESC_RM_MAP_MEMORY` / `NV_ESC_RM_ALLOC_MEMORY`
2. `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR`
3. the **UVM** ioctls that unify CPU VA and GPU VA — including the simulated aperture, under which
   **the guest always believes the GPU is DMAing to host memory**
4. the **one doorbell page** every userspace process maps and writes its token into — **read native,
   write trapped** so the token can be translated

★★ Every one of these is either **host memory that already has a CPU mapping** or **a single page we
trap**. ⇒ None needs a mechanism NVIDIA refuses on this hardware. **The fiction stays entirely below
the userspace boundary.**

⚠ **A guest kernel would almost never publish our fake framebuffer to an unprivileged process** — it
is internal privileged RAM and channels. If we ever observe it doing so, that observation is the
finding.

---

## 3. ✔ MEASURED — why this is not merely tidier

**The taxonomy already exists in the code, but it is DERIVED and it grew one distinction at a time,
each added when a specific confusion bit.**

- `Representability` (`kayfabe-fwd`) is **computed from what happens to be present** at an address —
  `HostBacked` / `Fabricated` / `Untracked`. ⇒ **`Fabricated` is simply what you get when nothing
  decided a kind.**
- Its own doc for the host-backed case reads: *"⚠ **AND the object must be the range's ONLY
  memory**… A host object that merely *exists* at the address is not enough: `PublishVidmem` puts one
  at a VA whose bytes the guest goes on reading and writing through the emulated framebuffer."*
- `BackingBytes::{SoleBacking, ShadowsGuestMemory}` (`kayfabe-mmu`) was added **2026-08-11** as a
  **runtime check** for exactly that state.

⇒ ★★★ **In this model that state is unrepresentable.** A region is one kind. There is no *"real
object at an address the guest also reaches through the fiction"*, because a region that is real is
not fake. **The check becomes unnecessary rather than merely passing.**

★ This is the same distinction the tree discovered empirically, stated **once**, **declaratively**,
at **allocation time**, instead of re-derived at every use.

---

## 4. ✔ MEASURED — what it predicts about the standing wall

The eight walling doorbells are **guest userspace** (`proc 2`). Their **queue is backed by the fake
framebuffer** (`V:0x1024000`) while their command buffers are in host memory (`S:`).

⊘ **Two readings were proposed and BOTH were wrong** — the orchestrator's `(a) we advertise it that
way` and `(b) the driver always does this`. **The answer is neither:**

> ★★★ **Nothing decided that ring should be fabricated. `Fabricated` is the default for any touched
> GPGA page.**

⇒ Under this model, **a userspace mapping request against an unallocated region becomes real GPU
memory**, and there is no fiction on that path at all. ⚠ **`[NOT MEASURED]` — this predicts the fix;
it does not demonstrate it.** The deciding experiment is what the code does today when guest
userspace asks to map an unallocated GPGA region.

---

## 5. ⇒ What this changes

- **Region kind becomes declared state**, consulted rather than inferred. Every site that currently
  calls `representability_of` instead reads the kind.
- **`BackingBytes` becomes redundant** — kept only as a transitional assertion.
- **The scratchpad stops needing a special mechanism**; it is fake framebuffer routed through
  `OS_DESCRIPTOR`.
- ⊘ **The emulated framebuffer stops being a shared surface** and becomes what it was always meant
  to be: **private storage for channels we emulate on the guest kernel's behalf.**

Related: `mode2_gpga_memfd.md` (the owner's sparse-memfd sketch — ⚠ its motivation is narrowed by
this doc: sharing the fiction matters only for the kernel-channel case),
`property_2_the_subtraction.md`, `is_passthrough_the_only_correct_route.md`.
