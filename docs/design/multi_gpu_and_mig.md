# Multi-GPU forwarding + MIG — design + the honest MIG reality-check

> ### ⊘⊘⊘ NO LONGER DESIGN-ONLY — 2026-09-13 (w641–w649). **Twelve findings were MEASURED
> ### against HEAD, two were blockers, and five are fixed.** Read this block before the
> ### design below, which describes an axis the core has and the shim did not.
>
> Owner, 2026-09-13: *"Remember there is a doorbell page per gpu, each gpu has its own bar0/1/2
> we need to emulate in kayfabe, its own fake fb and gpga, and va tables, and channels, ensure
> this remains possible."* A subagent audit of the whole tree answered that, and the answer was
> not what I went looking for.
>
> **★ The headline: the two things that actually stopped a second GPU were not statics.** They
> were **process-wide KERNEL resources acquired per device**, in `kayfabe-vmm-qemu/src/slots.rs`.
> One failed loudly and misleadingly; the other failed silently and destructively the moment the
> first was fixed. ⇒ **Fix them together or a loud refusal becomes silent corruption.**
>
> | # | finding | state |
> |---|---|---|
> | 1 | `KvmSlotPlane::discover` — `discover_in_this_process` demands EXACTLY ONE `anon_inode:kvm-vm` fd and keeps its own dup, so device 1's scan finds two and is refused *"this process holds more than one KVM machine"* — **false**: one machine, two descriptors, the second one ours | ★ FIXED w641 (memoised) |
> | 2 | `SlotAllocator` minted per device, every instance `next = ceiling` ⇒ both GPUs hand out the same memslot numbers, and `KVM_SET_USER_MEMORY_REGION` on a live number is a **REPLACE**, not an error. `SLOT_BUDGET_EXHAUSTED` never fires because neither allocator misbehaves by its own accounting | ★ FIXED w641 (per-machine `SlotNumberSpace`) |
> | 3 | `Gpu::new` vs the `GpuId` roster — the shim builds one `Gpu` per device instance from **constants** (`OBJECT_GPA_WINDOW`), so two devices each believe they own the whole 64 GiB window and two procs get the same GPA arena. The core's model is *one `Gpu` with a roster carving disjoint windows*; the shim's is *one device = one `Gpu`*. **These are incompatible** | ⊘ OPEN — a design decision, not a patch. **Rank 1 of what remains** |
> | 4 | `static SHADOW_SINK` — raw pointers into each device's BAR0 shadow, resolved by **offset only**, first match wins ⇒ both devices' plane writes land in device 0's memory | ⊘ OPEN |
> | 5 | `static DROPPED` — not telemetry but a **control-flow protocol**; `take_full_rescan`'s `swap` is exactly what lets GPU 1's worker steal GPU 0's rescan latch, leaving GPU 0's page tables stale | ⊘ OPEN |
> | 6 | `gid_for_chip` derives the UUID from **ChipProfile** fields — properties of a *generation*, not a board ⇒ both GPUs declare byte-identical UUIDs and `cuDeviceGetUuid` collapses them. The escape hatch `with_gid` exists and **has no production caller** | ⊘ OPEN — blocked behind #3: fixing a UUID collision in a configuration that cannot run is fixing a symptom |
> | 7 | `MIRROR_FOR_BIRTH` — **worse than "loses a number"**: `OnceLock::set` returns `Err` for device 1 and `let _` discarded it, so device 1's drain called `note_first_channel_birth()` on **device 0's mirror** and `premap_bars()` on **device 0's BARs**. ★ The measurement was not lost, it was **taken on the wrong GPU** — an absent number reads as absent; a wrong number reads as a measurement | ★ FIXED w644 (`SharedDoorbell::bar_mirror`) |
> | 8 | `DOORBELL_TARGET_GPU` pins the gpu half of every `IsolateId`, so **both devices' isolates open host `/dev/nvidia0`**. The correct shape already exists in the same file (`shim.rs:11685`, `IsolateId::new(pid.0, gpu)`) | ⊘ OPEN — latent today (one device, roster `{ZERO}`) |
> | 9 | Identical `LockRank` on every plane — `check_acquire` panics on the **same** rank, so any thread holding device A's `PlaneState` and acquiring device B's takes a hard R3 violation | ⊘ OPEN — latent, and fails loudly rather than degrading, which is the good case |
> | 10 | `minted_join_ledger` / `supersede_ledger` keyed by framebuffer-physical addresses, which are **per-device** (each GPU's fake FB starts at its own zero) ⇒ two devices collide on every frame number | ⊘ OPEN |
> | 11 | `HostRegion.id` is a dense per-device index with no device tag; device A's `HostRegion{id:0}` is in-range for device B and resolves to a **different fd**, returning `Ok`. ⊘ The same argument the code already makes for `RAM_EXPORT_TOKEN_TAG`, never applied across device instances | ⊘ OPEN |
> | 12 | Per-device telemetry summed process-wide. ⚠ Mostly cosmetic — **except** `MIRROR_DRAINS`, which is the test oracle for a before/after delta, and `DBTABLE_SHADOW_*`, which **gated a decision** | ★ `DBTABLE_SHADOW_*` FIXED w648; the rest OPEN |
>
> ## ★★★★★ THE RULE THIS LEFT, and it is the useful part
>
> **Not *"is it a static"* but *"is the thing it NAMES process-wide?"***
> - The KVM VM descriptor **is** (one machine per QEMU process) ⇒ memoised in a static, correctly.
> - The slot-number cursor is **not** — it belongs to the machine ⇒ `SlotPlane::number_space()`.
> - The birth mirror is **not** — it belongs to the device ⇒ a port field.
>
> ⚠ My first attempt parked the slot cursor in a `static` and **reproduced the bug one level up**:
> two independent mock machines fought over one frontier and a device that fit was refused a
> window carved out of someone else's ceiling. The fix for a multi-GPU bug was a multi-GPU bug.
>
> ## ⊘ And a citation that vouched for a type nobody asked
>
> `kayfabe_mmu::refresh::WalkOutcome::Unsupported` deferred peer memory on the grounds that *"a
> peer mapping is a view of one GPU's memory in another GPU's space, which is what
> `kayfabe_device::gpgaview::ViewSpace` models"*. **It did not model it** — `Scratchpad`,
> `GuestMmio` and `Vmm` were unit variants and `Isolate` carried a bare proc id, so the type could
> tell two procs apart on one GPU and could not tell two GPUs apart anywhere. Fixed w641 (three of
> the four spaces now carry `GpuId`; `Vmm` is deliberately GPU-blind). ★ **A citation checks that
> a claim is SOURCED, never that the source says what the claim says** — this tree's capture-oracle
> lesson, pointing at our own code.
>
> ## Clean bills, worth knowing
>
> `kayfabe-isolate-host` is genuinely per-device (every registry keyed by `IsolateId`; no path,
> abstract socket, shm name or pid-derived identifier anywhere — every parent↔child channel is an
> anonymous `socketpair`/`pipe`). `kayfabe-device` has three mutable statics in 23 758 lines.
> The QEMU C glue's eight file-scope statics are all `const`. `kayfabe-core` already models the
> axis properly — `(GpuId, Pdb)` / `(GpuId, VChid)` everywhere. **The axis is built; it died at
> the shim boundary.**

> Status: **DESIGN, queued** (task #29, scheduled after the core test-hardening ladder:
> security invariants → fuzz → determinism → mutation gate). Captured here so the decision
> and its rationale live in the tree, not just in chat. Origin: design discussion 2026-07-24
> (owner raised multi-GPU usefulness + the "is a MIG slice just another `/dev/nvidiaX`?" question).

## Why

The multi-tenant product thesis is "an unprivileged host rents real GPU to untrusted guests."
A host with **N physical GPUs** renting slices is the natural scale-out of that, so the core
must forward to the *right* GPU and keep each GPU's state isolated. Today the core is
**single-GPU**: `Gpu` is a singleton owning one `GpaSpace` + procs + the `RmGraph`. The
`Device`/`Subdevice` object *kinds* exist in the graph, but nothing binds a `Device` to a
specific host GPU or routes/isolates per GPU. Multi-GPU is therefore a real, currently
**unmodeled** core dimension — not free.

## The NVIDIA boundary we build on (why this is clean)

One `hClient` can span GPUs by allocating a **`Device` per physical GPU** (selected by
`deviceInstance`); VASpaces / channels / engine objects hang off that `Device`, and a
`Subdevice` pins the specific physical GPU under it. So the physical-GPU identity **flows
from the `Device` object** — it is a declared protocol fact, resolvable in the same
order-independent way as every other edge in the `RmGraph`.

## The core work

Introduce a first-class, **routable GPU target** — a `GpuId` (newtype, in the domain-id set
alongside `Gpa`/`GpuVa`/`Pdb`) — and thread it as an axis *above* the per-`Proc` / per-`Vas`
structure:

1. **Bind subtree → target.** Derive each object subtree's `GpuId` from its owning `Device`'s
   `deviceInstance`. A `Channel`/`Vas`/`Memory` resolves to its target via its `Device`
   ancestor (a `RmGraph` projection, MISS = fault — never a guess).
2. **Route per target.** Every forwarded op (doorbell/ring, map/publish, completion arm)
   selects the isolate/backend for its **target GPU**. Per-GPU `GpaSpace` (each physical GPU
   has its own guest-physical window + arenas) and per-GPU address tables.
3. **Isolate per target.** A `Proc` on GPU0 can observe/affect nothing on GPU1 — the `#14`
   boundary lifted onto the GPU axis.

### Tests (the acceptance bar)

- An op lands on the **correct** host GPU (routing is by `Device`-derived `GpuId`, not by
  guess or by first-resolvable).
- **Cross-GPU isolation** holds: a hostile/errant `Proc` bound to GPU0 cannot reach GPU1's
  PDBs, arenas, completions, or backing.
- **`#14` extended to the GPU axis**: two GPUs presenting *identical* guest VAs and identical
  RM handles (the stock driver reuses both) must not collide — disjoint by construction, same
  discipline as the per-`Vas` / per-`Proc` separation.
- Determinism/order-independence (decision #4) holds with the GPU axis present.

## MIG — the honest reality-check

The tempting shortcut was "a MIG slice is just another `/dev/nvidiaX`, so if multi-GPU works
MIG is nearly free." **That premise is wrong on two counts, and it matters:**

1. **MIG is datacenter silicon.** It exists on A100 / A30 / H100-class GPUs. Our target — the
   GeForce **RTX 3060 (GA106)** and commodity GeForce generally — has **no MIG at all**. It is
   untestable on our bench and *off* the commodity-GeForce thesis that makes the project the
   10× over Mode-1.
2. **A MIG slice is not a device node.** The physical GPU stays `/dev/nvidia0`. MIG instances
   are **GPU-Instance / Compute-Instance (GI/CI) partitions**, addressed by MIG UUIDs, with
   access gated through **`/dev/nvidia-caps/nvidia-cap*` capability files** plus GI/CI RM
   objects (the `NVC637`-family subscription). That is a *partition-subscription + capability*
   mechanism — **not** device multiplexing. It does **not** fall out of multi-GPU for free.

   > Confidence: high on "datacenter-only" and "not a new `/dev/nvidiaX`". The exact caps
   > plumbing (`nvidia-caps`, `NVC637`, GI/CI subscription) should be confirmed against the
   > vendored references — gVisor `nvproxy` handles `nvidia-caps`, and the open kernel modules
   > (`ogkm`) are ground truth — **before** any MIG implementation.

### The synthesis (what we actually do now)

Design the multi-GPU **target abstraction to be MIG-accommodating**: a `GpuId`/target is a
*routable GPU target*, **not** hardwired to "physical device." A MIG instance is then
conceptually just **another kind of target** (a partition-target) — so MIG becomes a later
*adapter*, not a core refactor. We leave the seam in the right place and build/test **none**
of MIG now (no pretend tests on absent hardware).

### MIG — deferred milestone (named, not forgotten)

When/if the project targets datacenter cards: implement the partition-target kind =
GI/CI subscription (`NVC637`) + `nvidia-caps` capability plumbing, resolved through the same
`Device`-ancestor projection, verified against `nvproxy` + `ogkm`. Prerequisite hardware:
an A100/A30/H100-class GPU (the vast.ai GeForce bench cannot exercise it).
