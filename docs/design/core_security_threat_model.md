# Core security threat model — the pure-logic isolation invariants

**Status:** independent red-team pass (decision #18C). Companion to the boundary
example-suite (#18A, `tests/tests/security_boundary.rs`) and the C-bug regression matrix
(#18B, `c_bug_regression_matrix.md`). This document states the isolation invariants of
the pure logic core (`crates/kayfabe-core`, `-mmu`, `-fwd`, `-completion`) **formally, as
checkable properties**, defines the attacker capability model, and records what this pass
found. Each invariant maps to the proptest that searches it in
`tests/tests/security_invariants.rs`.

The thesis under attack: *an unprivileged host process can safely run an UNTRUSTED guest
against a real host GPU, multi-tenant.* A single cross-tenant leak or a wedge-the-box DoS
is fatal. This document is the adversary's checklist.

---

## 1. Scope of the core — what CAN and CANNOT live here

The core is **pure logic**: `#![forbid(unsafe_code)]`, no pointers, no host memory, no
syscalls, no OS/time/NVIDIA dependencies. It is a deterministic state machine over
guest-supplied *abstract facts* (`RmEvent`, decoded doorbell tokens, decoded pushbuffer
methods).

**In scope (this document):** *logical* isolation invariants — cross-process address
resolution, completion routing/forgery, refcount soundness, and DoS containment, all
expressed over the core's abstract state.

**Out of scope (deferred to a post-L2 audit):** memory-safety, out-of-bounds, use-after-
free, and any host-breakout surface. These are *born at L1/L2* (the mmap/isolate/trap/
VMM adapters), not in pure logic — the pure core has no pointer to get out of bounds and
no `unsafe` to be unsound. Asserting them here would be theatre. The bounded-memory type
and its `trybuild` compile-fail assertions belong to L1 and are written when L1 is
(decisions #16/#16b).

---

## 2. Attacker capability model

The adversary is a **hostile guest** driving the core's input surface. We model three
capability tiers (arch doc §4.3.5); the core must hold against all three at the *logical*
layer.

- **A1 — hostile guest userspace process.** May issue ANY sequence of `RmEvent`s
  (`Alloc`/`Dup`/`SetPageDir`/`MapMemoryDma`/`Unmap`/`Free`) in ANY order, ANY repetition,
  with ANY handle/class/VA/offset/length values, plus arbitrary doorbell tokens and
  pushbuffer bytes. It **cannot** choose a global hardware identity assigned by the guest
  kernel from physically-distinct resources — its own **PDB** (page-directory base) or a
  channel's **vChid** — because those come from resources the kernel owns. (See §5 A1.)
- **A2 — hostile guest process colliding on per-namespace identities.** May present handle
  values and guest VAs that deliberately *collide* with another process's (the #14
  identical-handle/identical-VA shape). This is the primary isolation stressor: identical
  per-namespace identities across processes must never interfere.
- **A3 — compromised guest kernel.** May additionally forge global hardware identities
  (PDB/vChid). Standard VM isolation still contains it to its own VM (we add no escape);
  within the VM it already owns every process, so the *cross-tenant* claim is unaffected.
  Its worst logical reach against the core is a **contained loud refusal**, never a device
  wedge or another tenant's corruption (§5 A3 records the residual: which claimant a
  hardware-identity collision refuses is deterministic-but-order-dependent).

The boundary objects the core reasons about (arch doc §4.3.1a):

| Object | Role | NOT |
| --- | --- | --- |
| **Client** (`hClient`) | handle namespace + access rights | **not a process key** (values reused across processes; N per process) |
| **VASpace / PDB** | ★ THE memory boundary (GMMU keys page tables by PDB) | not client-keyed (many clients share one VAS) |
| **Channel / vChid** | ★ THE execution boundary (doorbell demux) | not CPU-state-derived (no CR3 read anywhere) |
| **Proc** | grouping label for isolate + GPA-arena + lifecycle only | **not** where address/exec ops key (those key on PDB/vChid) |

`Proc` membership is a **pure projection** of the RM graph's client-ownership tree +
`DUP_OBJECT` edges — never inferred from timing.

### 2.1 ★★ The two-layer trust model for guest-shared memory

Everything above says *who* the adversary is. This says **what a page of memory the guest can
write is worth to us**, and it is the cleanest statement of the trust boundary this project
has produced. It is normative for every shared table, ring, descriptor and page-table view in
the system.

> **Layer 1 (trap + lock) gives SECURITY against a guest that ignores the protocol.
> Layer 2 (NVIDIA's own synchronisation points — the handshakes the real guest kernel uses,
> e.g. around PDB updates) gives CORRECTNESS for a guest that follows it.
> Shared tables are NEVER authoritative for an immediate decision.**

**Neither layer is a weaker version of the other**, and that is the whole reason both are
named:

- **Layer 2 alone is not security.** A cooperating guest reaching a protocol sync point tells
  us its data is stable; a malicious one simply does not reach it, or reaches it and keeps
  writing. Worse, a hostile guest can update a shared table **after our validation passed and
  before we act** — the classic TOCTOU window, and the one that turns "we checked it" into
  "we checked a value that no longer exists". Layer 1's job is to make that window **not
  exist**: while the region is trapped/locked, there is no interval in which the guest can
  move the bytes we decided on. ★ **And a guest can *fake* Layer 2** — emit the handshake, the
  invalidate, the doorbell, at any moment it likes, including moments the real driver never
  would. That is precisely why Layer 1 is not optional and cannot be replaced by "we follow
  the protocol carefully".
- **Layer 1 alone is not correctness.** A trap tells us the bytes cannot change while we look;
  it does not tell us the bytes are *finished*. A cooperating guest mid-way through publishing
  a page-directory update has written something internally inconsistent and entirely stable,
  and reading it under a perfect lock yields a perfectly-locked wrong answer. **Only the
  protocol's own synchronisation points say when the data is meant to be read** — which is why
  we take the guest's own edges (the TLB invalidate, the CE release semaphore, the GSP RPC,
  the fn-47 idle handshake) rather than inventing a moment of our own.

**The consequence, and it is the sentence that decides designs:** a shared table is evidence,
never authority. Nothing may key an immediate decision on a value read out of guest-writable
memory at an arbitrary instant. The core's existing shape is this rule already made
structural — the address table is **forward-populated** from declared events and never
reverse-resolved out of guest memory, and **MISS = FAULT** (§3 I1, `kayfabe-mmu`); the
pushbuffer is parsed from a **copy** and only at the seams where we are the mediator; a
completion fires only from a genuinely-armed source (§3 I2). Where guest memory *is* read, it
is read at a Layer-2 edge and treated as a *declaration to be validated*, not as truth.

**This is described, not proposed — Layer 2 already runs.** The C artifact implements the
Layer-2 handshakes for Mode-2: its address table is populated **only from guest-declared
edges**, never by walking guest memory whenever we feel like it
(`C: docs/design/mode2_address_table.md`).

★ **And the C's own measurement sharpens what "the protocol's sync point" means, in a way
worth carrying:** it is *whichever* edge the protocol actually commits at, not a favourite
one. On the kernel/UVM/RM paths that edge is the TLB invalidate (both transports carry the PDB
and the membar bits). On the **GSP-emulated compute path it is not**: round 6 of the #14
investigation measured **zero** occurrences of both invalidate transports *and* of
`DMA_FILL_PTE_MEM` there, and the commit point that replaces the absent invalidate is the **CE
release semaphore** at which the observed CE page-table write is latched and decoded
(`C: docs/design/mode2_address_table.md` §5, audit S3). So a design that had encoded "read at
the invalidate" as *the* Layer-2 rule would have had no sync point at all on the path that
matters most — the general rule survives, one of its instances did not. The rewrite carries
both edges as first-class facts (`TlbInvalidate` and the CE-write capture,
`execution_plane.md`).

**What the C does *not* have is Layer 1.** Trap-and-lock over guest-shared memory is named in
the rewrite architecture as a capability with an explicit *"ASSUMPTION — verify"* attached
(`C: docs/design/mode2_rust_rewrite_architecture.md`, decision #6: userfaultfd / memslot
revoke-restore, never host `mprotect`), and it is **built** at L1/L2 here
(`l1_os_shell.md` §6.8/§6.8.1). That asymmetry is exactly why the two layers are written down
together in one place rather than each in its own milestone's doc: the working system to date
has been running on Layer 2 alone, which is correct for a cooperating guest and is not a
security property.

**Scope note.** §1 defers *memory-safety* to a post-L2 audit and that deferral stands: this is
a **trust model**, not an out-of-bounds analysis. It is stated in this document because it is
the rule that decides what the core is allowed to believe about its own inputs, and the core's
inputs are guest-writable memory. Its bulk-read corollary — *copy once, validate the copy,
never re-read; a single value we decide on is always atomic* — is `l1_os_shell.md` §4.2.2, and
is the same principle applied one layer down.

---

## 3. The isolation invariants, as checkable properties

Each invariant is stated so a single violating trace is a counterexample. `∀ hostile
sequence S` means "over any sequence the attacker model above can produce."

### I1 — Cross-process address isolation
> **∀ S, ∀ two processes p ≠ q, ∀ guest VA v:** if `resolve(pdb(p), v)` succeeds it
> yields a backing owned by `p`, never one owned by `q`. Identical guest VAs in distinct
> PDBs are disjoint by construction; a miss is a loud fault, never a content-pick.

Mechanised: `i1_no_proc_va_resolves_to_another_procs_backing` — a K-process world
(identical VAs, distinct PDBs, distinct backings), arbitrary interleave + junk, checked
against an **injective PDB→phys oracle**. Structural basis: the address table is
per-`Vas`, PDB-keyed, forward-populate-only, MISS=FAULT (`kayfabe-mmu`); arenas are
per-`Proc`, disjoint by construction (`gpa.rs`).

### I2 — Completion integrity / forgery
> **∀ S:** a completion fires only from a genuinely-armed source in the *owning* process,
> at most once, at/after its target. A guest-forged or backwards fence/sema value cannot
> forge a completion, cannot cross-signal another armed source, and cannot violate the
> #12 backwards-jump guard (a step > `MAX_FENCE_JUMP` is a loud refusal, state unchanged).
> A user-visible completion cannot be forged for another process (the forge path is typed
> to the system proc).

Mechanised: `i2_fence_matches_reference_model_never_forges_a_completion` (differential vs
an independent reference fence model) and `i2_completion_never_cross_signals_another_
armed_key`. Structural basis: `FenceArms`/`CompletionQueue` are per-`Proc`; the forge
entry `signal_golden_capture` is typed `Traffic::System`; fence routing is by-PDB
(single-owner).

### I3 — Refcount soundness
> **∀ S over deep parent-trees with cross-client dups and interior/root frees:** no
> free-while-referenced, no double-free, no permanent leak. A resource is live ⟺ it has a
> live reference (a handle or a live mapping); a `DUP_OBJECT` alias keeps a resource alive
> across the origin handle's free; freeing every handle drains the graph to empty; a free
> of a truly-unknown handle is a loud `FreeUnknown`, never a panic or a silent leak.

Mechanised: `i3_deep_tree_refcount_is_sound_no_leak_no_double_free` (extends fuzz A4,
which oracles only self-parented roots, to deep trees + interior frees + dups). Structural
basis: `Resource { refs, map_refs }`, liveness ⟺ `refs` non-empty ∨ `map_refs > 0`.

### I4 — DoS containment
> **∀ S:** the device is ALWAYS projectable (no global wedge); a benign bystander's
> routing and address resolution are never corrupted by the storm; the graph stays usable
> (a fresh benign op still succeeds); every hostile event earns only its OWN loud refusal
> (`CapacityExceeded`/`Projection`/local fault), never OOM, never a panic, never another
> process's failure.

Mechanised: `i4_hostile_flood_is_contained_never_wedges_or_breaks_a_bystander` (bystander-
first, then a generated flood/collision stream that may squat the bystander's exact
PDB/vChid). Generalises #18A's atomic-apply (`Gpu::apply` snapshot/rollback) and the
capacity caps (`MAX_LIVE_HANDLES`/`MAX_LIVE_MAPPINGS`/`MAX_PARKED`/
`MAX_OUTSTANDING_COMPLETIONS`/`MAX_ARMED_FENCES`) over generated sequences.

**★ I4's honest gap — a COMPLEXITY DoS that the caps do not cover (open, measured,
deliberately deferred).** I4 as stated is about *containment*: no wedge, no OOM, no
bystander corruption, every hostile event earning only its own refusal. It says nothing
about **cost**, and cost is where the remaining boundary-2 exposure is:

> **The control plane is O(live objects) per event, so N events cost O(N²).** With
> `MAX_LIVE_HANDLES = 2^18` a guest can drive that curve deliberately — and **PyTorch startup
> reaches it benignly**, allocating thousands of RM objects. Measured (debug build,
> alloc+map against a growing graph): 1 000 events **0.85 s**; 8 000 events **54.8 s**.
> Deleting the per-event `RmGraph` clone saves ~24% and leaves the curve **still quadratic** —
> the clone is not the cause and cannot be the fix.

Same species as the parked-map linear scan already hardened, and as the O(n² log n)
condemned-list carry-forward that *was* fixed (union-find, 55 s → 3.8 s). Two candidate fixes
are named and neither is a small change: incremental derivation (which is a redesign of
decision #27's "derived state is a pure function of the graph, never accreted", on which the
whole determinism differential rests), or an undo journal in the most safety-critical file in
the repo. Full analysis, measurements, and why doing either blind in a security round would be
the wrong trade: `l1_concurrency.md` §12.23.

**State this as an open item rather than a closed one.** Every other I4 clause is
property-proven; this one is measured and outstanding, and the caps bound *memory*, not
*time*.

---

## 4. Confused-deputy: made structural (Phase 3)

**Property:** *resolving a guest handle to the WRONG `ObjectKind` is impossible to do
silently.* Every site that turns a guest-supplied handle into a *typed* object routes
through the ONE typed-resolution primitive:

```rust
RmGraph::origin_of_kind(key, want) -> Option<&RmNode>   // discriminant-checked
```

Audit of every resolution site (grep `origin_of`/`resolve*`/`backing_of`/`node`):

| Site | Turns handle into | Type-checked via |
| --- | --- | --- |
| `project::resolve_declared_handle` | whatever `want` names | `origin_of_kind(_, want)` |
| `project::resolve_channel_vas` (own `hVASpace`) | a VASpace | via `resolve_declared_handle` |
| `project::resolve_channel_vas` (CtxShare hop) | a CtxShare, then a VASpace | via `resolve_declared_handle` |
| `project::resolve_channel_vas` (parent hop) | a TSG, then a VASpace | via `resolve_declared_handle` |
| `project` engine refinement | a Channel (any engine) | via `resolve_declared_handle` |
| `RmGraph::backing_of` | a Memory → phys | `matches!(_, Memory)` (Memory-only) |
| `RmGraph::apply_map` (backing) | a Memory → phys | now via `backing_of` (see §5 F2) |
| `RmGraph::is_client_root` | a Client (one-hop, no alias) | `matches!(_, Client)` |

Mechanised: `p3_origin_of_kind_rejects_every_cross_kind_pairing` (the full cross-kind
matrix) and `p3_channel_vas_resolution_type_checks_every_hop`.

★★★ **§12.44 — the type check was never the whole question, and the other half was
missing.** Every row above resolves a handle *in a namespace*, and until §12.44 that
namespace was the bare `HClient` value carried on the resource's `NodeKey`. RM recycles
that value by design, and a resource can outlive its namespace's root (a foreign
`DUP_OBJECT` refcounts it), so a **ghost's declared handle facts were resolved against the
handle table of whoever holds the number now** — a correctly *typed* resolution into an
unrelated tenant's namespace. `project::resolve_declared_handle` now scopes every one of
these sites to the resource's own `ClientKey` DECLARATION, and a superseded declaration
resolves to nothing (MISS ⇒ DEFER here, FAULT at use). Mechanised:
`rmgraph_order_independence::a_ghost_channels_declared_hvaspace_never_binds_the_next_tenant_of_its_namespace`
and `::a_ghost_engine_object_never_retypes_the_next_tenants_channel`.

**Residual (named follow-up):** the check is *centralised runtime*, not *compile-time*.
A phantom-typed handle (`Handle<VaSpace>`) that makes an untyped resolution
*unrepresentable* — with a `trybuild` compile-fail case — is deferred: it would ripple
`NodeKey` typing through the whole graph and is out of proportion to land safely in this
pass. The runtime primitive is the single enforcement point until then.

---

## 5. Findings

### Confirmed contained (no bug) — the invariants HELD
I1, I2, I3, I4 are now **property-proven** over generated hostile sequences with the
mechanisations in §3, several against reference-model oracles. The pre-existing example
suites (#18A `b1_*`/`b4_*`/`b5_*`, sim_14, fuzz A1–A4) remain green. No cross-tenant leak
was found.

### F1 — Parked-`SET_PAGE_DIRECTORY` wedge **(REAL BUG, FIXED)**
Found by the I4 property. A hostile process parks a `SET_PAGE_DIRECTORY` on a handle it
does not yet own (aimed at a victim's PDB), allocates its OWN VASpace with its own PDB,
then `DUP_OBJECT`s that VASpace onto the parked handle. The stale parked declaration now
resolves (via the dup alias) onto the attacker's live VASpace. On the *next unrelated*
`Alloc`, `resolve_pending_pdbs` **overwrote** the VASpace's real PDB with the stale one,
forging a projection `PdbCollision`; because the parked fact survives the surrounding
`Gpu::apply` rollback, the collision **re-fired on every subsequent `Alloc`** — a
device-wide control-plane WEDGE (every other process's allocations refused). This violates
I4 ("never another process's failure").

**Fix** (`resolve_pending_pdbs`): drain the parked fact unconditionally, but only APPLY it
to a resource whose PDB is not already set — a *direct* `SET_PAGE_DIRECTORY` is
authoritative and always wins; a *parked* (older) one never overwrites it. This also makes
PDB resolution order-independent. Regression:
`p4_parked_setpagedir_via_dup_alias_cannot_wedge_the_device` (verified to fail before the
fix).

### F2 — Parked-`MAP_MEMORY_DMA` (unbacked) wedge **(REAL BUG, FIXED — F1's twin)**
Same root cause, map plane. A hostile process parks a `MAP_MEMORY_DMA` naming an unowned
handle as its `memory`, against its own PDB-bound VASpace; allocates an UNBACKED memory
object; then `DUP_OBJECT`s the unbacked memory onto the parked handle. Because `Dup` does
not retry parked maps, the poisoned map lingers until a benign `Alloc` triggers
`resolve_pending_maps`, which installs an unbacked mapping; `Gpu::sync_rpc_mappings` then
faults `UnbackedMapping`, rolling back the benign op — and re-firing forever. WEDGE.

**Fix** (`apply_map`, `replay` flag): a *replayed* parked map that resolves to an unbacked
backing is dropped (it can never populate — backing is alloc-time), so the VA simply never
forward-populates → a loud MISS=FAULT at *use*, contained to the toucher. A *direct*
unbacked map still installs and faults loudly (its own op earns the contained
`UnbackedMapping` refusal, pinned by `object_model::unbacked_mapping_is_a_loud_fault`).
Regression: `p4_parked_unbacked_map_via_dup_alias_cannot_wedge_the_device` (verified to
fail before the fix).

> **Root-cause class (for future work):** a *parked* fact that resolves during an
> unrelated apply can produce a Gpu-level fault (`PdbCollision`/`UnbackedMapping`) whose
> rollback restores the parked fact, so it re-fires — turning a contained refusal into a
> persistent wedge. F1/F2 close the two reachable instances at their resolution sites. A
> systemic guard (prune a parked fact whose resolution faults, rather than rely on
> per-site non-harm) is a candidate hardening if new parked-fact kinds are added.

### F3 — Confused-deputy: two memory→phys resolvers disagreed **(HARDENED)**
`apply_map` read `facts.mem_phys` off whatever object the `memory` handle named, while its
sibling `backing_of` type-checks `ObjectKind::Memory`. A hostile `MAP_MEMORY_DMA` naming a
NON-memory object (e.g. a VASpace) carrying an attacker-set `mem_phys` was silently
accepted as a mapping's backing — the "two resolvers disagree" class the *one resolution
path* directive (matrix row 4) forbids. `host_va`-gating neutralised *execution* (an RPC-
only binding never rings), so this was not an exploitable cross-tenant leak on its own,
but it is exactly the silent-accept the design forbids. **Fix:** `apply_map` now resolves
backing through the ONE typed `backing_of`, so a non-Memory `memory` yields no backing → a
loud `UnbackedMapping`, never a silent bind. Regression:
`p4_map_naming_a_non_memory_object_is_a_loud_unbacked_fault_not_a_silent_bind`.

### A1/A3 — First-declarer-wins on hardware-identity collisions **(assumption, not a bug)**
Two VASpaces with one PDB (or two channels with one vChid) is a hardware impossibility;
the core refuses the collision **loudly and atomically** (`PdbCollision`/`VchidCollision`
→ `Gpu::apply` rollback), never a silent wrong-resolve, never a device wedge, never a
third party affected (§3 I4; `b1_hw_identity_squat_is_contained_and_third_party_safe`).
*Which* of two colliding claimants is refused is **order-dependent (first-declarer-wins)**.
Under the A1 model this is unreachable (userspace cannot pick its PDB/vChid); it becomes
reachable only under A3 (a compromised guest kernel forging a hardware identity), which
already owns its whole VM. This is therefore a documented **assumption** — real hardware
guarantees unique PDB/vChid, kernel-assigned — not an isolation break: the cross-tenant
(cross-VM) boundary is untouched, and even intra-VM the effect is a contained loud refusal,
not a leak or a wedge (F1/F2, which *were* wedges reachable at the A1/A2 level, are fixed).

---

## 6. What this pass proved

- I1–I4 property-proven over generated hostile sequences (§3), several with differential
  oracles; the confused-deputy surface collapsed to one centrally-enforced typed primitive
  (§4).
- Two **real control-plane wedge bugs (F1, F2)** — reachable by a hostile guest at the
  A1/A2 level — found by the I4 property and fixed in the core, each with a verified
  regression.
- One confused-deputy inconsistency (F3) hardened onto the single resolution path.
- The residuals are named honestly: compile-time handle typing (§4) and a systemic
  parked-fact guard (§5 F2 note) are deferred follow-ups; the hardware-identity
  first-declarer-wins property (§5 A1/A3) is a documented assumption, not a break.
