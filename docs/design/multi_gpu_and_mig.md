# Multi-GPU forwarding + MIG — design + the honest MIG reality-check

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
