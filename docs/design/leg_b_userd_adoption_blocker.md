# LEG B — the guest's USERD: the blocker is an ADDRESS WITH NO PRODUCER, not a missing alloc arm

> ### STATUS — 2026-08-12 / **LIVE — ANSWERED, and the answer REFUTES the rung's framing.**
> Leg B was commissioned as *"if B turns out to need a primitive that does not exist (an
> `hUserdMemory` hand-in arm in `alloc_channel_in`), build it."* That primitive is **not** what
> is missing. Read §1 before writing any code against this leg.
>
> Companion to `guest_ring_and_userd_adoption_prereg.md` (the pre-registration) and
> `guest_ring_adoption.md` §4 item 3, whose *"⊘ REFUTED — resolving the guest's declared USERD
> object"* this re-measures independently and confirms.

---

## 0. What leg B is, and why it cannot be skipped

`RESUME_HERE_2026_08_11.md` and `gr_doorbell_passthrough.md` §0.3 name **two independent**
reasons the host GR channel executes nothing:

1. **the ring is ours** — closed by legs A1 + A2 (`4a6d0ef`, `361fca8`);
2. **the cursor is ours** — `GP_PUT` lives in the USERD *we* hand RM at
   `ChannelAllocParams::h_userd_memory_0` (`rm.rs`, the single construction of the channel
   alloc params), and the only writer of that word in the tree is
   `HostRmBackend::submit_entry`.

⇒ After leg A the host channel names the guest's ring and still sees `GP_PUT == GP_GET`
forever, because the guest advances a **different** word. Leg A without leg B moves nothing,
and that is pre-registered (P7 = 0 on the `ring` arm).

★★★★★ **And the ordering is not negotiable.** `[measured 2026-08-11, R32/w233, real GA106 /
580.159.04]` **RM ZEROES all 512 bytes of a caller-supplied USERD.** Adoption at the first
doorbell therefore **wipes the cursor that caused the doorbell** — a `NV_OK`-returning hazard
that produces silence, not an error. Adoption must happen at channel **creation**.

---

## 1. ★★★ THE BLOCKER, MEASURED — and it is upstream of every alloc arm

To hand RM the guest's USERD we need a **host memory object over the guest's USERD pages**.
To mint one — by any route, `join_fb_leaf` or the guest-RAM pin — we need that object's
**address**. We do not have it, and the tree already says so in a committed test.

### 1.1 The handle is there. The address is not.

`kayfabe_core::rmgraph::AllocFacts::userd: Option<DeclaredUserd>` carries the guest's own
declaration verbatim, and it is reachable from `&Spine` exactly like `gp_fifo_ring` is:

```text
DeclaredUserd { handle: u32,   // hUserdMemory[0]
                offset: u64 }  // userdOffset[0]
```

`[measured]` on the bench's GR channel: `userd=h0x5c000014/off0x2000`
(`run_s48_4f5b357_cwait_qemu.log:234`).

The handle→address function is `RmGraph::backing_of(NodeKey)`. Its body is:

```rust
matches!(node.kind, ObjectKind::Memory)
    .then(|| node.facts.mem_phys)
    .flatten()
```

### 1.2 ⊘ `AllocFacts::mem_phys` HAS NO PRODUCER IN PRODUCTION CODE

`[measured 2026-08-12, `git grep mem_phys` over `crates/` and `tests/`, this revision]`:

| site class | count | where |
|---|---|---|
| declaration | 2 | `rmgraph.rs:383` (`AllocFacts`), `rmgraph.rs:643` (`Mapping`) |
| production **write** of `AllocFacts::mem_phys` | **0** | — |
| production **read** | 1 | `rmgraph.rs:2545`, inside `backing_of` |
| write of `Mapping::mem_phys` | 1 | `rmgraph.rs:2176`, `mem_phys: self.backing_of(mem_key).map(|base| base + m.offset)` — i.e. **derived from `backing_of`, which is the function that returns `None`** |
| writes in `tests/` and fixtures | many | `tests/src/lib.rs:489`, `tests/tests/object_model.rs`, the proptest corpora, … |

⇒ **`backing_of` returns `None` for every memory object a real guest allocates.** The field is
live only because test fixtures set it by hand.

★ **And this is not my inference — the tree asserts it.** `crates/kayfabe-rmrpc/src/lib.rs`
lists `mem_phys` under `gsp_core_bridge.md` §6's B3 ("unbuildable in this direction"), and
`tests/tests/rmrpc_bridge.rs:3298` pins it as a **test**:

> *"★ `mem_phys` is `gsp_core_bridge.md` §6's B3 row and is not buildable in this direction"*

Two independent reasons, both in `kayfabe-rmrpc`'s own docs: the address fields are `[OUT]` in
the guest→GSP direction, and `MAP_MEMORY_DMA` is a HAL stub on every GSP-client part, so
`RmEvent::MapMemoryDma` has no producer either.

⊘ And the guest's USERD alloc itself carries **no params at all** — `[measured]`
`ALLOC hClass=0x00000040 … hObject=0x5c000014 size=0 … params=-`.

### 1.3 ⇒ THE REFUTATION, STATED PLAINLY

> **A `UserdSource::Guest(handle)` arm in `alloc_channel_in` would have NO PRODUCER.**

It would compile, it would be symmetrical with `RingSource::Guest`, it would look like
progress, and nothing in the tree could ever call it — a ninth entry on `RESUME_HERE`'s list
of *"proved in isolation, never wired"*, which §5 names as this project's **structural
condition**. ⇒ **Not built.** The brief authorised building it; the brief's diagnosis of *what*
was missing is the part that is wrong.

★ Note the asymmetry with leg A, because it is the whole lesson: leg A's equivalent number —
the ring's `gpFifoOffset` — **is** on the wire (`AllocFacts::gp_fifo_ring`, decoded from the
guest's own channel alloc at a known offset), and its **address** is recoverable by walking
the guest's own page tables, because a ring is *mapped into the channel's VAS*. **USERD is
not.** Hardware reaches USERD through `hUserdMemory[0] + userdOffset[0]` — an RM **object**,
never a GPU VA — so there is no page-table walk that can find it.

---

## 2. The two routes that remain, and what each one costs

### 2.1 ⊘ REFUTED — resolve the declared handle

§1. Dead until `AllocFacts::mem_phys` has a producer, which `kayfabe-rmrpc` argues is
unbuildable in the guest→GSP direction at all.

### 2.2 ★★ THE LIVE ROUTE — the BAR1 trap, and the join it still needs

`[measured, boot `s17_e8fde62`, and the same shape in ~78 committed boots]`:

```text
BAR1[0] WRITE off=0x90000 size=4 val=0x20000000   ← a GPFIFO entry, low dword
BAR1[1] WRITE off=0x90004 size=4 val=0x2801       ← …high dword
BAR1[2] WRITE off=0xa008c size=4 val=0x1          ← GP_PUT = 1, at USERD + 0x8c
```

`0x8c` is `kayfabe_abi::submit::USERD_GP_PUT` exactly. USERD is an `NV01_MEMORY_LOCAL_USER`
(vidmem) object, so the guest CPU-maps it through **BAR1**, and BAR1 is a trapping region
whose accesses `RegPlane::bar1_phys` GMMU-walks into a framebuffer-physical address served by
our own `SparseFb`.

⇒ **The guest's USERD bytes ARE reachable, and its framebuffer address IS derivable — from the
trap, not from the graph.** What is missing is one join:

> ⊘ **Nothing joins a BAR1 offset to a CHANNEL.** That join is the guest's `NV04_MAP_MEMORY`,
> which never reaches this port. Inferring it from the observed `0x3000` USERD stride would be
> **reverse-resolution by address**, which `kayfabe-mmu`'s `gpga.rs` forbids in as many words:
> *"there is no `fn owner_of(addr)` and there never will be."*

★ That is `guest_ring_adoption.md` §4 item 3's conclusion — *"G8's supply question is a JOIN
question, not a plumbing question"* — and this rung re-derived it from the other end and
agrees.

### 2.3 ⚠ AND THE TWO ROUTES ARE NOT INTERCHANGEABLE, because RM zeroes

Route 2.2 gives the address of a **live** USERD the guest is already writing. Handing that
object to RM at channel creation makes RM zero 512 bytes of it. Whether that is safe depends
on a fact **nobody has measured**:

> ⊘ **Does the guest write `GP_PUT` before or after it allocates its first engine object?**

The host channel is born at the engine-object alloc. If the guest's `GP_PUT` store precedes
it, adoption *at creation* is **also** a wipe, and leg B has no safe moment at all under the
current birth ordering. This is registered as **unmeasured** rather than assumed
(`guest_ring_and_userd_adoption_prereg.md` §3.3 item 4).

★ It is cheap to measure and it is the next thing to do: both events are already on paths we
own — the BAR1 trap prints the `GP_PUT` store, and `ENGINE-OBJECT …` prints the birth. **One
armed boot's log ordering answers it.** ⇒ Measure the order before building either route.

---

## 3. ⊘ What this leg does NOT block

Leg A is complete and independent. A host GR channel is now born over the **guest's** ring
whenever the supply side joined its leaf; that is a real change to what RM is told, and it is
measurable on its own (`GR-RING-JOIN`, and `RingSource::Guest` reaching `alloc_channel_in`).

⇒ **Leg B's blocker does not make leg A unmeasurable.** It makes leg A **insufficient**, which
is exactly what the pre-registration says: P7 = 0 on the `ring` arm, and P7 > 0 was predicted
only for an arm that this rung has now established **cannot be built yet**.

★ ⚠ ⇒ **The pre-registration's bold prediction is therefore only half-testable, and I am
recording that rather than quietly re-grading.** `P7 > 0 on ring-and-userd` cannot be scored
this rung. The half that survives — *"P5 ≥ 1 and P7 = 0 on the `ring` arm"* — is a weaker
claim than the one I registered, and a weaker claim is what I have.
