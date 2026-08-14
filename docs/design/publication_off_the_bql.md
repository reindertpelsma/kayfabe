# Publication off the BQL — the deferred map lane, the synchronous revoke floor, and `INLINE-SAFE` as a type

> **STATUS: LIVE — 2026-08-14 (w323).** Design + mechanism built and tested offline; **the
> wiring into `SharedDoorbell::ring` is NOT landed** (§9), and nothing here has met a GPU.
> Supersede in place; do not write a successor beside it.
>
> **Parents this folds into:** `blocking_and_completion_model.md` (§1's `INLINE-SAFE` gets its
> first mechanism for clauses (a)/(b) here), `budgeted_bql_disposal.md` (§6's named-and-unfixed
> defect is fixed here), `mode2_address_table.md` (§5's *"no invalidate on the compute path"* is
> what forces the commit boundary to be ours).

---

## 0. The ruling

> Owner, 2026-08-14: *"10 ms BQL lock seems already bad to me. … why do you need to publish
> under BQL? I don't see a reason. you already have other boundaries to determine when to
> update a pte write — look at the C."*

And the ordering the rationale is graded against:

> **exact GPU boundary (TLB invalidate) > trap the PTE write, as little as possible >
> deferred publish on doorbell > work under the BQL**

★ With the refinement this document establishes: **tier 1 is available only on the MAP side**
(it depends on a guest signal we cannot compel — and which, measured, never arrives on our
compute path); **REVOKE has tier 2 as its floor.**

---

## 1. ⊘⊘⊘ THE CORRECTION THAT MAKES THIS POSSIBLE — I HAD CONFLATED TWO DOORBELLS

The standing objection was *"publication cannot be deferred past the doorbell that needs it."*
**That sentence contains two different doorbells and is false for the one it sounds like.**

The real ordering constraint is:

> **publish → _our_ host ring**

⇒ **both ends are ours.** The **guest's** MMIO doorbell store is **fire-and-forget by the
GPFIFO contract**: the guest writes a token to `NV_VIRTUAL_FUNCTION_DOORBELL` and reads
nothing back. It does not poll a status, cannot observe when we act, and has no register whose
value depends on our having acted (§5.2 measures this rather than assuming it).

⇒ we may **return from the trap immediately**, publish on a worker, and ring the host when
publication is complete. The guest cannot tell the difference, **because it never waited.**

### ★★★ The C already did exactly this, and its own source says so

`C: src/qemu/nvkvm_gpu_emul.c:592-604`, verbatim:

> *"`nvkvm_m2_ce_fb_write_hook` **LATCHES an entry dirty (O(1))** when a CPU-emulated CE write
> lands on its page; `nvkvm_m2_cpt_sync_at_release` then decodes each dirtied page **DIRECTLY
> (not via a root walk)** and backs its new leaves into the persistent host GR VAS — **at the
> map push's completion-semaphore release, before the release un-gates the (already-rung,
> host-resident) GR channel into the new mapping.**"*

⇒ **the channel was ALREADY RUNG.** The C's boundaries were an **O(1) dirty latch at the
write** and a **deferred commit at the release** — never at the doorbell.

★ And the load-bearing detail from the same comment, which this design preserves: *"the guest
fills a leaf PT page THEN links it under the root a push later, so at the release a root walk
can't yet reach it (`runs=0`) but the page itself holds committed PTEs."* ⇒ **decode the
WRITTEN PAGE directly, never from the root.** A root walk at commit time will legitimately
find nothing, and reading that as "nothing to publish" is a silent miss.

★★ **We already have the latch half.** `w318`'s dirty set — `AddressTable::generation`
(bumped at `bind`/`unbind`, `kayfabe-mmu/src/lib.rs:1080`/`:1096`) and `FbStore::writes_by`
(`write_tagged`, `kayfabe-device/src/fbwin.rs:291`) — **is the C's O(1) latch, at the sink.**
What was missing is the **deferred commit point**, and that is what §3 builds.

---

## 2. What is on the BQL today — measured, with the anchor on every number

Every guest MMIO write arrives with the **QEMU BQL** held
(`shim.rs:4877`, `:6146`, `:6046`). ⇒ blocking there does not stall the ringing vCPU; it
stalls **every vCPU and QEMU's main loop** (`blocking_and_completion_model.md` §0).

| what | cost | anchor |
|---|---|---|
| one `cuLaunchKernel` | 90.9 ms, of which **86.7 ms is ONE MMIO trap** (97.5–98.9 % of SUBMIT) | `[w315, measured, GA106, 2 boots]` |
| inside that trap: `vas_publish` | **55.7 %** | `[w315]` |
| inside that trap: `pt_decode` | **25.7 %** | `[w315]` |
| ⇒ page-table + publication | **91.5 %** | `[w315]` |
| the **real host forward** (`core`) | **4.1 %** | `[w315]` |
| per-doorbell hold, post-`w318` dirty gate | **4.078 ms** | `[w318]` |
| `VAS_PUBLISH_WALL_BUDGET` — the publication drain's own ceiling, **held under the BQL** | **2000 ms** | `shim.rs:13977` |
| a pinned row costs **3 sequential cross-process round trips** (`map_guest_ram` → `describe_guest_ram` → `map_gpu_va`); 13 313 rows ⇒ **39 900 synchronous RTTs on the vCPU** | 96 % of a 3.0 s drain | `[w321, measured]` |
| retired-object disposal, pre-`w317` | one **3.70 s** BQL stall | `[w314/w317]` |

⇒ **95.6 % of the launch floor is `shape=work`**, so bare metal does not fix it
(`[w315]`) — the work has to move, not get faster.

---

## 3. The design — MAP side

### 3.1 The BQL path

```
guest MMIO store to the doorbell
  → TrapGuard::enter()                      (w323, shim_unsafe.rs — marks the thread)
  → RegPlane::write → ring_doorbell
  → PublicationQueue::offer(MapPublication::for_doorbell(token))
  → return to VM entry
```

**Total host I/O on this path: none.** A hash insert, a `VecDeque` push and a `notify_one`,
under one leaf mutex that no other trap path takes for anything else. That is `INLINE-SAFE`
(a) — it completes without the guest running; (b) — it is O(1); (c) — the lock is a leaf held
for a handful of instructions.

### 3.2 The worker

```
kayfabe-publication worker thread
  → PublicationQueue::take_blocking()
  → OffTrap::claim(...)                     ← succeeds: this thread is not in a trap
  → witness / decode / sweep the guest's page tables      (today's pt_witness/pt_decode/pt_sweep)
  → publish the VAS rows into the host VAS  (today's publish_vas_rows)
  → SharedDevice::doorbell(...)             ← the host ring, AFTER publication
  → PublicationQueue::note_completed()
```

The **order inside the worker is unchanged** from today's `SharedDoorbell::ring`, and it has
to be: *"a mapping published after the ring has been rung is a mapping published after the
engine has already faulted for it"* (`shim.rs:4970`, and the C's own *"fault-safe: a mapping
is always backed before the engine that uses it runs"*, `C: nvkvm_gpu_emul.c:582`). **What
moves is the whole block, not its internal order.**

### 3.3 ★★★ Ordering between doorbells — the problem dissolves, it is not solved

The brief asked for a per-channel queue and an overflow policy. Both are unnecessary, and the
reason is a property of the hardware contract rather than of our code:

> **The submission cursor is read at EXECUTION time, not latched at trap time.**

`SharedDevice::doorbell` → `forward_ring` (`kayfabe-rt/src/device.rs:2636`) reads the
channel's GPFIFO and `GP_PUT` out of guest memory **when it runs**. So *N* doorbells on one
token and *one* doorbell after the last `GP_PUT` advance are **the same act**.

⇒ the queue holds **at most one entry per distinct token**; re-offering a pending token is a
**coalesce**, not a push, and not a drop.

- **Per-channel order**: vacuous — there is never more than one entry per channel, and one
  worker executes them.
- **Cross-channel order**: FIFO by offer. ⊘ Not a correctness requirement — independent
  GPFIFO channels are independent by the hardware contract — but it is free and it makes a
  starvation argument trivial.
- **Overflow**: the queue's size is bounded by *live channels with work outstanding*, a
  number RM bounds and which **no amount of doorbell ringing can inflate**. A guest cannot
  grow it by ringing faster. The cap (`DEFAULT_CAP = 4096`) exists so that hitting it is a
  **diagnosis**; on refusal the caller runs the work inline — i.e. degrades to *today's
  behaviour*, which is never worse than the status quo, and `PUBQUEUE refused=N` says so.

★★★ **THE ONE NON-OBVIOUS HAZARD, and the implementation gets it right on purpose.** The
token leaves the pending set **at the take, not at completion**. Clearing on completion would
let a doorbell that arrived *while the token was executing* coalesce into a run that had
already read the cursor — a **lost wakeup**: the guest's submission would sit unexecuted until
some unrelated doorbell arrived. `pubqueue::tests::a_doorbell_arriving_after_the_take_queues_again`
is that property, and it is the test most likely to catch a future "optimisation".

⚠ **What coalescing costs, stated rather than discovered later.** The worker reads the guest's
ring **while the guest is running**, where today it reads it with the guest halted. A guest
that rewrites a GPFIFO entry between advancing `GP_PUT` and the engine consuming it now races
us. ⊘ That is **already illegal against real hardware** — the GPU DMA-reads the pushbuffer
asynchronously — so the deferral is *more* faithful, not less. What it is **not** is
identical: our decoder is not the hardware's, and a torn read must **fault** rather than be
acted on. That obligation belongs to the ring reader and is **not discharged by this rung**;
see §8's boot criteria and §9's residue.

---

## 4. ⊘⊘⊘ THE ASYMMETRY — MAP MAY DEFER, REVOKE MAY NOT

Owner ruling, 2026-08-14, folded in **above** the mechanism because the mechanism enforces it:

| direction | what a late act costs |
|---|---|
| **MAP** (invalid → valid) | the engine walks an unpublished VA ⇒ a **GPU fault**. **FAIL-SAFE**, and this tree measured the containment: a bystander process ran **2 675 519 verified iterations** through one (`gpu_fault_is_contained`). |
| **REVOKE** (valid → invalid) | a **live host-GPU translation into guest pages the guest has already released and Linux has reused**. **FAIL-DANGEROUS.** |

⇒ ★★★★★ **DEFERRING A REVOCATION IS NOT A LATENCY CHOICE, IT IS A LEAK WINDOW** whose
duration is exactly the deferral. This tree already banked the hazard as the one that
*outranks the race* (`cancellation_is_not_built_and_preempt_is_forged`: *"pinned host-GPU
translations into guest pages the guest already freed"*).

### 4.1 What the design does about it

**The asymmetry is a type, not a rule.** `kayfabe_device::pubqueue::PublicationQueue::offer`
takes a `MapPublication`. `Revocation` is a **distinct type with no route into it** — no
`From`, no `Deref`, no `as_map()`, no public field. *"Put the unmap on the deferred queue"*
**does not compile** (`crates/kayfabe-device/tests/ui/defer_a_revocation.rs`), and the three
dismantling routes a role type must close are closed and censused
(`pubqueue::tests::the_revocation_type_has_no_route_into_the_queue`, the shape
`host_class_role_wiring.rs` established).

### 4.2 ★ The honest answer: revocation stays SYNCHRONOUS, and it is a small cost

**Do not stretch the design to make everything async.** Revocation today is a
`VerbPlan::Release` chain reached through `kayfabe_fwd::dispose_on` and `Proc::drop`. It runs
inline, under the BQL, **bounded by `w317`'s budget** (`RETIRED_DRAIN_BUDGET_US = 40_000`, 1 %
of `scrubberDestruct`'s 4 s). That is **tier 2** in `blocking_and_completion_model.md`'s
vocabulary — legitimate precisely because it is bounded — and it is a *far* smaller BQL cost
than today's whole-VAS publication drain (2000 ms budget, seconds observed).

⇒ **Outcome (B) applies to the revoke direction, and it is deliberate, not residual.**

### 4.3 The ordering guarantee revocation relies on, stated

> **A revocation is effective before the guest can observe the memory as reusable, because
> the revocation completes inside the same MMIO trap in which the guest asked.**

The guest asks by freeing an RM object; the free is an ioctl our device sees as a trapped
register write / RPC; the disposal chain runs before that trap returns (subject to the w317
budget, which carries the remainder to the *next* trap — so the bound is "budget + one chunk
per trap", not "forever"). ⚠ The budget is the one soft edge: a residue carried to the next
trap is a residue still live. That is a **bounded** window measured in one trap interval, not
an unbounded one, and it is the honest statement.

⊘ **A guest omission cannot open the window.** The revocation lane is driven by **our**
teardown of **our** host objects — never by a guest signal we could be denied. Withholding
whatever would have triggered a *publish* cannot extend a *revocation*, because no guest
action is on that path at all.

### 4.3.1 ⊘⊘⊘ AND HERE THE GUARANTEE ABOVE FAILS — **A GUEST OMISSION *CAN* HOLD THE WINDOW OPEN TODAY**

`[measured 2026-08-14, git grep, whole tree]` **the revocation drain has exactly one driver,
and it is a guest MMIO write.**

- `SharedDevice::drain_retired_budgeted` — one production call site, `shim.rs:11673`, inside
  `Regs::write`.
- `pin_reclaim_gone` — `shim.rs:11642`, same function.
- `reap_retired_held` (which is what lets `Proc::drop` discharge the queue at all) —
  `shim.rs:11716`, same function.

⇒ **the budgeted drain carries its remainder to the NEXT REGISTER WRITE, and only the guest
produces register writes.** A guest that frees its host-backed objects and then simply **stops
touching MMIO** leaves the residue undrained **indefinitely** — and the residue is exactly a
live host-GPU translation into pages the guest has released.

★★★ **That is precisely the property the owner's ruling forbids** — *"a guest omission must
never be able to leave a host translation live … withholding whatever would have triggered our
unmap must not extend the window"* — and it is **true of master today**, before any deferral.
⊘ It is not introduced by this design; it is **found** by asking this design's question of the
existing code. The `w317` budget made the *per-trap* cost bounded and, in doing so, made the
*completion* of a revocation depend on the guest continuing to trap. **A bound that is
discharged only by the adversary is not a bound.**

⚠ **Not exploited, not measured.** It is stated as a mechanism with both ends cited
(`shim.rs:11642/11673/11716` and the absence of any other caller). A hostile-guest repro is the
falsifier and it does not need a GPU: drive `Regs::write` once to stage a release, then stop,
and assert `pending_release_len() > 0` forever.

★ **The fix shape, and note the irony — it is the worker this rung builds.** The drain is host
I/O, so it must not run on the BQL; it must also not depend on the guest. Those two
requirements have exactly one solution: **drive it from our own thread.** The publication
worker (§3.2) is already a host-verb-capable, off-trap execution site with an `OffTrap` it can
claim honestly. ⊘ Do **not** simply move the drain onto the deferred *publication* lane — that
lane is the MAP lane and `Revocation` is refused by it at compile time (§4.1). It wants its own
tick on the same thread, or the existing `kayfabe-completion-observer` thread (250 ms tick,
`shim.rs:10932`), which is off-trap by construction and currently read-only.

### 4.4 Failure, per direction — they differ and must not share a word

- **A failed MAP ⇒ a GPU fault.** That is correct and it is `miss = fault` / *"not found, not
  denied"*, and it is **more faithful to hardware than what we do now**: real hardware cannot
  refuse a doorbell either. ★ Surfacing publication failure as a fault is a *feature*.
- **A failed REVOKE ⇒ the translation is STILL LIVE and must not be reported as revoked.**
  The residue goes back on `pending_release` and is retried; `dispose_on` returns the
  undisposed set as a `#[must_use]` value precisely so it cannot be dropped on the floor.
- **⚠ In neither direction do we send a completion the guest could misread.** Nothing on the
  deferred lane writes a guest semaphore, advances a `finishPayload` or raises an interrupt.
  The owner's rule — *"a completion is sent only if the observed state after it is intended
  and safe"* — is satisfied here **by there being none**, which is tier 1 and is why cup3
  crossed with no completion-wait architecture at all.

### 4.5 ★★★ The instance this rung fixed — `w317`'s named-and-unfixed defect

`Proc::stage_release` extended `unmap` and `free` and **silently dropped
`orphans.guest_ram`**. Fixed (`kayfabe-core/src/gpu.rs`), with the known-positive `w317` asked
for: `tests/tests/staged_release_carries_every_orphan_kind.rs`.

⚠ **Be exact about what leaked — the obvious reading is worse than the truth and the truth is
bad enough.** Of `Orphans`' three kinds, `unmap` (the **host GPU** translation) and `free`
(the RM objects, including the `OS_DESCRIPTOR` that pins the pages) **were** staged. What was
dropped is `guest_ram` — the **isolate process's own `mmap` window** onto guest RAM. ⇒ this
did **not** leave a live host-GPU translation; it left an unprivileged host process's
CPU-visible mapping of guest pages outliving the verb, the proc, and the guest's release of
them. Same family, different aperture.

⚠ **It is a behaviour change**: `munmap`s now happen on the live verb path that did not
happen before. Graded as one; boot evidence in §8.

---

## 5. The three complications the brief named

### 5.1 Ordering — §3.3. Dissolved by coalescing; the lost-wakeup hazard named and tested.

### 5.2 Reads — ⊘ NOT A BLOCKER, and here is the evidence rather than the assertion

`RegPlane::read_inner` (`kayfabe-device/src/plane.rs:2757`) is the whole guest-visible read
surface, and every arm is local:

| arm | source |
|---|---|
| `boot_regs` | the chip table |
| `ptimer_read` | a counter |
| `rom_read` | the synthetic VBIOS |
| `bar0_window` | a latch of the guest's own last write |
| `cpu_intr` | our interrupt tree |
| `fb_read` | our emulated framebuffer, via our own page walk |
| `fsm.mmio_read_with` | the emulated GSP |

**No arm consults publication state, the host, or an isolate.** ⇒ **no guest-visible MMIO read
depends on completed publication**, which is the load-bearing premise of the owner's model and
is now checked rather than believed. Corroborating: `[w320, measured]` MMIO reads total
**20.5 ms per BOOT**, and this tree's directive is READ-NATIVE, WRITE-TRAP.

⊘ The read path is nonetheless marked with a `TrapGuard` (§7), *because* it is expected to be
clean: a gate that only watches the path you already suspect cannot refute you.

### 5.3 Error reporting — §4.4. And the one obligation this rung does NOT discharge

A torn ring read (§3.3's cost) must fault rather than be acted on. Our GPFIFO/pushbuffer
decoder's behaviour on a mid-flight rewrite is **UNMEASURED**, not "safe". It is the first
thing a follow-up lane should adversarially test, offline, before the async arm is defaulted
on.

---

## 6. ⊘ THE TWO INVALIDATES — say this plainly so nobody re-derives the wrong one

They are unrelated and the vocabulary invites conflating them.

**(1) The GUEST's TLB invalidate, as a publish trigger — DOES NOT EXIST HERE.**
`[measured, mode2_address_table.md §5 ★ CORRECTION, audit S3]` on the Mode-2 GSP-emulated
compute path: `INVALIDATE_TLB` RPC fn=200 = **0**; `MEM_OP`/`MMU_TLB_INVALIDATE` pushbuffer
method = **0**; `DMA_FILL_PTE_MEM` = **0**. This tree carries it as a standing directive:
*read-at-invalidate is FALSE on the compute path.*
⇒ **You cannot hook the guest's invalidate as a commit boundary — it never arrives.** You do
not need to: the C's release semaphore is *"the commit point that replaces the absent
invalidate"*, and §3's doorbell-driven worker is the same conclusion by a different mechanism.
⚠ This also bounds tier 1 in the owner's ordering: *"exact GPU boundary (TLB invalidate)"* is
**unavailable on our compute path**, which is why tier 2 (trap the write, latch O(1)) is the
real ceiling and why `w318`'s latch already matters.

**(2) The invalidate WE owe the HOST GPU after a host PTE changes — ★ ALREADY DISCHARGED, by
construction, and this was checked rather than assumed.**

`[measured 2026-08-14, git grep over `crates/`]` there is **no host-GPU TLB-invalidate call
anywhere in our Rust tree**, and that is **correct**, because **we never author a host PTE.**
Every host mapping change of ours is an RM ioctl:

- map: `NV_ESC_RM_MAP_MEMORY_DMA` (`kayfabe-isolate-host/src/rm.rs:2110` `raw_map_dma`)
- unmap: `NV_ESC_RM_UNMAP_MEMORY_DMA` (`rm.rs:2148` `raw_unmap_dma`)

and **RM invalidates inside them**:

- `dmaUnmapBuffer` → `vaspaceInvalidateTlb(pVAS, pGpu, PTE_DOWNGRADE)` —
  `ogkm: src/nvidia/src/kernel/gpu/mem_mgr/dma.c:899`, again at `:1010`
- the map side → `gvaspaceInvalidateTlb(pGVAS, pGpu, PTE_UPGRADE)` —
  `ogkm: .../gpu/mem_mgr/arch/maxwell/virt_mem_allocator_gm107.c:3032`
- both reach `kgmmuInvalidateTlb_HAL` — `ogkm: .../mem_mgr/gpu_vaspace.c:2125`

⇒ **(2) is not a live defect and is not a candidate explanation for the intermittent faults.**

⚠⚠ **AND THE CONSEQUENCE, which is why §4 is not merely prudent:** RM's invalidate happens
**inside the ioctl we would be deferring.** Deferring a revocation therefore defers its TLB
invalidate by exactly the same interval. The leak window is not an artefact of our
bookkeeping — it is a real interval during which the **host GMMU still holds the
translation**.

⊘ **The scope limit on (2), stated because a green answer is the dangerous kind:** it holds
*while* every host mapping change of ours is an RM ioctl. The instant anything in this tree
writes a host page table directly — a BAR2 store, a CE-issued PTE write, an `MMU_TLB_INVALIDATE`
we emit — obligation (2) becomes ours and nothing today would notice. That is a **standing
condition on a design property**, and it belongs in review of any future host-side PTE writer.

---

## 7. `INLINE-SAFE` as a TYPE — clauses (a) and (b) get their first mechanism

`blocking_and_completion_model.md` §4: *"⊘ **Clauses (a) and (b) have no mechanism at all.** …
which by this repo's own history means they will be violated by a well-meaning patch."* Three
measured instances since: `w317`'s 3.70 s disposal, `w319`'s 13 313 serialized round trips,
`w306`'s isolate call reachable only from the vCPU trap.

### 7.1 ⊘⊘ The ruling this overturns, and exactly how far

`kayfabe_core::channel_kind::TrapContract` states, correctly for what it had:

> *"⇒ **Rust cannot express *"this call is not on the vCPU thread"*.** Thread identity is not
> in any type here…"*

★ **True of a TYPE ALONE; false of a type composed with a per-thread witness.** The
composition is three facts and no two suffice:

| carried by | fact |
|---|---|
| private field, no struct literal | this token was **minted by the constructor** |
| `OffTrap::claim`'s check of `in_trap()` | the minting thread was **not inside a guest trap** |
| `!Send` + `!Sync` (`PhantomData<*mut ()>`) | it is **still on the thread that minted it** |

⇒ holding an `OffTrap` means *"the thread executing this line was off-trap when it asked, and
is the same thread."* **That is thread identity, expressed.**

### 7.2 Where it is placed, and why exactly there

**One door.** `kayfabe_isolate::Worker::execute` is the single entry to a host RM verb in the
tree (and `with_rm` is its escape hatch, gated identically — *a gate present on the planned
path and absent on the escape hatch is a gate with a documented way round it*). One signature
quantifies over every call site in the workspace, including ones written tomorrow.

`TrapGuard` is installed at `kayfabe_shim_regs_write` / `kayfabe_shim_regs_read`
(`shim_unsafe.rs`) — the **outermost** boundary, so the drains `Regs::write` runs *after* the
plane call (materialize, err-grants, the budgeted drain, the reap) are inside the mark. That
is precisely where `w317` measured 3.70 s.

### 7.3 ⊘ It obeys this crate's own prior ruling about early-minted capabilities

`BlockingSection`: *"a capability minted while lock-free must not launder a later acquisition
past the invariant."* Same hazard one axis over. Two answers, both required: `!Send` stops the
token **crossing** to a trap thread; `OffTrap::still_off_trap` **re-asserts at the verb**, so
a token held across a re-entrant dispatch on its own thread panics at the door.

### 7.4 ⚠ THE CEILING, and it is the honest half

While `OffTrap::at_a_host_verb` exists, **the gate cannot panic in production** — a trap-thread
caller gets a **counted** declared exception instead of a refusal. That is deliberate: the
current architecture legitimately runs host verbs under the BQL (that is the wall this
document removes), so a gate that panicked today would be unshippable and would be deleted.

What the gate buys is the **`VerbPlan::gated_doorbell` upgrade — omission → commission**: a
host verb can no longer land on the trap path by nobody noticing. It lands by someone naming a
site that a census counts.

**Two numbers, answering different questions:**

| number | question | instrument |
|---|---|---|
| mint sites | how many places *may* declare an inline host verb | `tests/tests/off_trap_census.rs`, currently **4 across 3 files** |
| `trapwitness::inline_exceptions()` | how many *actually did*, this boot | runtime counter, on the boot line |

⇒ **the campaign's finish line is the census set going empty**, at which point
`at_a_host_verb` is deleted and `OffTrap::claim` is the only door — and *then* the answer to
the brief's *"is there any bounded case that must survive?"* is **yes, exactly one: the
revocation direction (§4.2)**, which is why two of the four declared rows are annotated
*"expect this row to survive."*

### 7.5 The violations watched failing to compile

| row | violation | error |
|---|---|---|
| `kayfabe-isolate/tests/ui/host_verb_without_a_trap_witness.rs` | `w.execute(&plan)` — a host verb with no proof of where the caller stands | E0061 |
| `kayfabe-isolate/tests/ui/name_a_trap_witness.rs` | `OffTrap { … }` — fabricating the witness | E0451 (private fields) |
| `kayfabe-isolate/tests/ui/send_a_trap_witness_to_another_thread.rs` | minting on a worker and posting it to a vCPU | `!Send` |
| `kayfabe-device/tests/ui/defer_a_revocation.rs` | `queue.offer(Revocation…)` — deferring an unmap | E0308 |

Runtime known-positives (a type cannot see a caller that never names it):
`trapwitness::tests::a_witness_cannot_be_claimed_inside_a_guest_trap` and
`…::a_witness_minted_off_trap_does_not_launder_a_later_trap_entry`.

⚠ **THE `!Send` ROW COMPILED ON ITS FIRST WRITING, AND THE REASON IS A TRAP WORTH BANKING.**
It read `std::thread::spawn(move || { let _ = off; })`. Under edition-2021+ closure capture
rules **`let _ = x` does not capture `x` at all**, so the closure stayed `Send`, the row
compiled, and it sat in the suite looking exactly like the other three while proving nothing.
⇒ a compile-fail row's body must **use** the value. `suspect_the_instrument_first`, applied to
an instrument whose whole job is to fail.

---

## 8. ★★★ THE BOOT EVIDENCE A FOLLOW-UP LANE MUST COLLECT

No GPU bench was available for this rung (`vh`/`vh2` both busy). Everything below is
**pre-registered**, not observed.

**Arm the async lane and run `cup3` (the known-positive: `CUP3_VAL=43`, un-forgeable).**

1. **The headline, and it is one number:** `trapwitness` census line shows
   `inline_exceptions=0` for the whole boot on the armed arm, and non-zero on the control.
   ⊘ If it is non-zero on the armed arm, publication did **not** move and every other number
   below is describing the old shape.
2. **`worst_trap=<N>us`** from the same line. Pre-registered prediction: the armed arm's worst
   MMIO-trap hold falls **from ~86 700 µs to under 1 000 µs**. ⚠ Three points minimum
   (three launches), residuals printed — a two-point fit is how this campaign has been fooled
   four times in one day.
3. **`kftime` segment shape**: on the armed arm `vas_publish`, `pt_decode`, `pt_sweep` and
   `core` must **disappear from `doorbell_fwd`** and reappear on the worker's own bracket.
   A trap whose segments merely *shrink* means the work was skipped, not moved — and
   `PUBQUEUE queued=…` distinguishes those.
4. **`PUBQUEUE` census**: `refused=0` (nothing fell back inline), `coalesced > 0` (the
   coalescer is reached — if it is 0 the arm proves nothing about the bound), `high_water`
   recorded as the measured queue bound.
5. **Correctness unchanged**: `CUP3_VAL=43`, `bad=0 maxerr=0` on `cup8_iter`, `Xid=0`,
   `host_rows` within noise of the control, and **the same 40 unserviced ids** — a changed
   ledger means the arm did something other than move work.
6. **The `stage_release` behaviour change (§4.5), on its own:** `munmap` count on the live
   verb path is now **> 0** where it was 0; assert no new `Xid`, no `NV_ERR_*` on the release
   chain, and that the host ledger still balances (`teardown_reclaim`'s `HostLedger`).
   ⚠ This one needs a boot **even if the async lane is not armed** — it is a behaviour change
   on master's own path.
7. **⊘ The negative control must be BYTE-COMPARABLE**: with the lane disarmed the log must be
   identical to today's, or the two arms are not measuring one variable.

---

## 9. ⊘ WHAT IS BUILT, WHAT IS DESIGNED, AND WHAT IS WRONG IN THIS DOCUMENT'S OWN PARENTAGE

**Built and tested offline (this rung):**
- `kayfabe_util::trapwitness` — `TrapGuard`, `in_trap`, `OffTrap`, the counters and census.
- `Worker::execute` / `Worker::with_rm` gated on `&OffTrap`; the four production mint sites
  named and censused; **75 call sites** updated.
- `TrapGuard` installed at both MMIO FFI entry points.
- `kayfabe_device::pubqueue` — the coalescing lane, the map/revoke type split, the worker
  end, the cap and its refusal arm, 8 unit tests including the lost-wakeup property.
- `Proc::stage_release`'s dropped third kind, fixed, with `w317`'s specified known-positive.
- 4 compile-fail rows.

**Designed, NOT built:**
- ★ **The wiring.** `SharedDoorbell::ring` still runs publication inline. Landing the lane
  means: a `DoorbellReport` variant for *accepted-not-yet-acted*, an `Arc<SharedDoorbell>` the
  worker can hold (the port is owned as `RwLock<Box<dyn DoorbellPort>>` today), the worker
  thread at the composition root, and an arming env var with a byte-comparable disarmed arm.
  ⊘ **It is deliberately not landed in a rung that cannot boot**: the arm's whole value is a
  measured before/after, and shipping an unmeasured rewiring of the doorbell path would be the
  exact "unattributable measurement" `w317` refused to create.
- The torn-ring-read obligation (§5.3).

**★ What in the brief turned out wrong:**
- ⊘ **"nothing needs to stay under the BQL"** — mine and the coordinator's — is **false**.
  The **revocation direction must stay** (§4), and that is not a residue to be minimised: it
  is the correct floor. Outcome **(B)** for that half, **(A)** for the map half.
- ⊘ **"a release path that silently drops guest-RAM orphans leaves host-GPU translations
  live"** — the coordinator's framing of `w317`'s defect. The GPU-side kinds *were* staged;
  what leaked is the isolate's CPU mapping (§4.5). The defect is real, the aperture is
  different, and stating the wrong one would point a future reader at the wrong plane.
- ⊘ **"check whether anything in our tree issues a host TLB invalidate; if nothing does that
  is a live defect"** — nothing does, and that is **correct, not a defect** (§6.2). It would
  become one only if we ever author a host PTE ourselves.
- ⊘ **"master is RED on 3 targets / 6 tests"** — measured at `53d6375c` on the **default**
  feature set: **4 targets / 7 tests** (the extra is
  `sticky_answer::the_universe_of_answering_policies_is_derived_from_the_source`). See §10.

---

## 10. ⚠ The wall: three of the six known-red tests are the ones this design would change

`doorbell_reaches_the_completion_observer.rs` asserts that **one `RmVerb::CeCopy` is recorded
synchronously after `dev.doorbell(...)` returns**, and
`doorbell_is_forwarded_without_reading_the_ring.rs` says in its own header that it pins
**statement order** — *"`verb_op(plan → execute → commit)` **then** `forward_ring` — is two
adjacent statements in one function that any refactor may swap without a single assertion
going red."*

⇒ **These tests are part of the wall.** Their shape — *call the doorbell, immediately assert
the host verb landed* — is exactly what an asynchronous lane invalidates. They are currently
**RED on master**, which makes this easy to get wrong in the other direction: a lane that
broke them further would look like it changed nothing.

★ **The correct treatment when the lane is wired** is not to relax them. It is to give them
the **completion barrier the design already provides** — `PublicationQueue::completed()` — so
they read *"the doorbell was accepted, the worker executed, and THEN the verb was recorded"*.
That is a strictly stronger assertion than today's, because it names the instant it is
measuring instead of relying on two statements staying adjacent.

---

## 11. Traps this rung paid for, banked

- ★★★ **The `w300` unranked-lock census CAUGHT `pubqueue`'s `Mutex<Inner>` on the day it was
  written**, and was right to: it is a lock taken on the vCPU trap path whose *consumer* side
  blocks on a `Condvar`. The classification row states the discrimination — **the producer
  side may never block and does not; the consumer side blocks and is called only from the
  worker** — so if a future caller reaches `take_blocking` from a trap, that row is the thing
  that was wrong. ⇒ an instrument built two rungs ago fired on the first new lock in months.
- ⚠ **`rsync -a` preserves mtimes, and cargo's fingerprint goes stale against a freshly synced
  source.** A brand-new module read as *"cannot find `trapwitness` in `kayfabe_util`"* — a
  diagnostic indistinguishable from a real missing-module error — while `cargo check` on the
  same tree was green from cache. Cost one full cycle. The harness now `touch`es every source
  before building. ⚠ Note the shape: **the failing and the passing command disagreed, and the
  failing one was right about nothing.**
- ⚠ **A compile-fail row that compiles** (§7.5) — the instrument's own version of *a green test
  can hold a wall in place*.
- ⊘ **The "baseline" that was not one.** The first suite run was launched against a tree that
  had already been half-modified, so its 4-target red set was attributed to master. The real
  master baseline was taken from a `git archive` of `53d6375c` into a fresh target dir. ⇒ **a
  baseline must come from a tree you can name**, and the number in §9 is from that one.

⊘ **Precedent to copy, not invent:** `spawn_is_deferred_out_of_the_plane_lock.rs` and
`engine_forward_is_deferred_out_of_the_plane_lock.rs` are the same latch-then-drain shape
already in this tree, and the second states the governing rule verbatim: *"work decided under
a lock can be deferred only if nothing in the response depends on it — a signature bounds what
a function CAN return; only the call site says what is READ."*
