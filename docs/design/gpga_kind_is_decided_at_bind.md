# The GPGA region kind is decided at BIND, and the VA space is what carries it

> ### STATUS — 2026-08-11 / **LIVE — OWNER DESIGN + a MEASURED inventory of what already exists**
> **Answers the question left open by `gpga_region_kinds.md` §0**: that doc measured that we are
> **not present** at the guest's allocation, so *"the kind is decided at allocation"* had no site.
> ⇒ **This doc says where the site actually is.** Owner, 2026-08-11:
>
> > *"any allocation in gpga should default to **fake fb for kernel allocs** and **real gpu mem for
> > guest userspace allocs**. is it possible to make such distinction? maybe where **vaspace** is
> > used for?"*
> > *"also remember that **kernel copy engine va spaces dont have to exist on real gpu** as for any
> > relevant action you translate in vmm anyways."*
>
> ★ **Answer: YES, and every link is already built.** What is missing is that the memory-kind
> decision does not consult them. §2 is a measured inventory; §4 is the part that is design.

---

## 1. ⊘ Why "at allocation" had no site, restated in one line

The guest runs the **stock** driver. Its RM carves video memory out of **its own heap** over the
framebuffer we advertise — **24 backing-allocating calls per CUDA run, zero reaching us**
(`gpga_region_kinds.md` §0, measured). ⇒ **We are absent at allocation. We cannot decide there.**

★★★ **But we are present at BIND** — the moment a VA is bound to a GPGA range in some VA space.
And bind is strictly better than alloc for this purpose, because **bind is the first moment at which
both facts exist at once**: *which GPGA range*, and *whose address space it is entering*.

---

## 2. ✔ MEASURED — the chain the owner is pointing at, and it is entirely built

Every row below was read from `origin/master` of `/workspace/nvkvm-rs` on 2026-08-11.

| link | where | status |
|---|---|---|
| **kernel vs. user is a DECLARED fact, not an inference** | `kayfabe-abi/src/guest_os.rs:281` — `client_kind_from_process_id` → `ClientKind::Kernel` \| `ClientKind::User { pid }`. Linux's rule is `ClientKindRule::KernelPidSentinel`, and the `process_id` comes off the guest's **own `NV01_ROOT` alloc params** | ✔ **BUILT AND IN USE** — consumed by `promote.rs`, `project.rs`, `fault.rs`, `rmrpc` |
| **an unknown rule REFUSES rather than guessing** | `ClientKindRuleUnknown`, and its doc: *"The caller owes the guest a refusal; **it must never substitute a kind**"* | ✔ the correct shape already |
| **the address table is PER-VA-SPACE** | `kayfabe-mmu/src/lib.rs:525,579` — `bind(pdb, va, …)` and `resolve(pdb, va)`. The `Pdb` is not optional; it is the key | ✔ **BUILT** |
| **VA space → owning process** | `kayfabe-core/src/gpu.rs:1944` — `Spine::by_pdb: BTreeMap<(GpuId, Pdb), ProcId>`, *"Data-plane routing (derived): `(GpuId, PDB)` → owning proc"* | ✔ **BUILT** |
| **the guest kernel's clients form their own component** | `project.rs:242` — *"the **system** component: every declared `ClientKind::Kernel`"* | ✔ **BUILT** |

⇒ ★★★ **At the instant `AddressTable::bind` is called we already know the `Pdb`, hence the `ProcId`,
hence whether the owner is the guest kernel or a guest userspace process.** The decision the taxonomy
wants is **one lookup away from a call site that already exists.**

### ⚠ One thing that does NOT do what its name suggests

`Spine::pt_page_owner(gpu, phys) -> Option<(ProcId, Pdb)>` looks like *"whose GPGA page is this"* and
**is not**. It is backed by `pt_roots` + `pt_learned` — **page-table pages only**. Its own doc says
`None` *"means **forward**, not **fault** … the overwhelming majority of copies are data."*
⇒ **Do not build the kind decision on it.** The authority is the **bind**, not a reverse lookup —
which is also what `mode2_address_table.md` requires: *forward-populated, never reverse-resolved.*

---

## 3. ★★★★★ THE SECOND RULING — kernel CE VA spaces need not exist on the real GPU

> *"kernel copy engine va spaces dont have to exist on real gpu as for any relevant action you
> translate in vmm anyways."*

This is a **subtraction**, and it composes with §2 to give the two defaults their teeth:

| owner of the PDB at bind time | region kind | host VA space |
|---|---|---|
| **`ClientKind::Kernel`** | **fake framebuffer** — our private storage for the channel we emulate | ⊘ **NONE. Do not create one.** Every relevant action is translated in the VMM |
| **`ClientKind::User { pid }`** | **real GPU memory** | ★ a real host VAS, at **identical VAs** (the passthrough ruling) |

### ⇒ Three consequences, stated so they can be checked

1. ⊘⊘ **ADJUDICATED 2026-08-11 — MY PREDICTION HERE WAS WRONG, AND SO WAS ITS PREMISE.**
   I wrote that this *"may DISSOLVE Property 2"*. Measured at `b3ecda4`: **Property 2's named
   object was already separated at `254cf38` (2026-08-10)** — the day before Property 2 was
   written — and its exploit citation is a **pre-fix** measurement quoted without its revision.
   Ruling 2 reaches only the **kernel half** of the *different* residual that survives, and its
   premise fails on two code facts: **every CE copy this tree can issue is VIRTUAL by a standing
   refusal**, so kernel CE needs *a* host VA space; and **no verb consumes the VMM translation**
   the ruling invokes. ⚠ **Applied literally today, ruling 2 turns kernel-channel forwarding OFF**
   (`FwdFault::NoVas` / `NoHostVas`) rather than rerouting it.
   ⇒ Full adjudication folded into `property_2_the_subtraction.md` **§0**. Read that, not this.
2. **It deletes work we currently do.** The doorbell materialization path calls `rm.alloc_vaspace()`
   before allocating a channel (`kayfabe-isolate/src/lib.rs:2385-2393`). Under this ruling that is
   **wrong for kernel channels** — not merely wasteful.
3. ⊘ **It does NOT touch the userspace path.** Guest userspace VA spaces must exist on the real GPU,
   at identical VAs. That is the whole passthrough thesis and this ruling narrows it to where it
   belongs rather than weakening it.

---

## 4. ⇒ WHAT TO BUILD (design, not measurement)

1. **`Kind` becomes a parameter of `AddressTable::bind`** — the compiler then names every site that
   binds without deciding. This is the tree's standing preference: make the wrong state
   *unrepresentable* rather than checked. (`gpga_region_kinds.md` §0 already records that
   `BackingBytes` is the **seed** of this type, not its casualty.)
2. **The two defaults come from `ClientKind`**, looked up via `by_pdb`. **Not** from what happens to
   be present at the address — that is `Representability`, which is derived and has **two** defaults
   pointing opposite ways (`Fabricated` on fall-through, `Untracked` → real hardware).
3. **An undeterminable owner REFUSES.** `ClientKindRuleUnknown` already establishes the precedent:
   never substitute a kind. A bind whose PDB names no proc is a **fault**, which is also what the
   address-table invariant already says (*miss = fault*).
4. **Kernel PDBs get no host VAS.** Follows from §3; it is a deletion plus a refusal, not a feature.

⚠ **What is still unwitnessed after all of this**: the *first touch* of a GPGA page that is never
bound into any VA space. `SparseFb` fabricates a zero page for any address below `fb_length`
(measured, §0), so today such a page silently becomes fiction. Under this design it stays fiction —
which is correct for the kernel case and **not obviously correct in general**. ⇒ Named here rather
than resolved.

---

## ✔ Verified by me before committing
- `guest_os.rs:281` — the `KernelPidSentinel` rule and the `ClientKindRuleUnknown` refusal, read
  verbatim.
- `kayfabe-mmu/src/lib.rs:525,579` — `bind`/`resolve` both take `Pdb` as a required key.
- `gpu.rs:1944` — `by_pdb` exists and its doc calls itself data-plane routing to the owning proc.
- `gpu.rs:2530` — `pt_page_owner` is page-table pages only; its `None` means *forward*, not *fault*.
- `kayfabe-isolate/src/lib.rs:2385-2393` — the doorbell path really does allocate a host VA space
  before the channel.

Related: `gpga_region_kinds.md` (parent — read its §0 first), `property_2_the_subtraction.md`
(§3 item 1 may dissolve it), `mode2_address_table.md`, `is_passthrough_the_only_correct_route.md`.
